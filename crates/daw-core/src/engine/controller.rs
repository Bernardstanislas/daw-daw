//! Control-thread side of the engine: diffs project documents coming from the
//! UI and turns them into audio-thread commands. All allocation (instrument
//! and effect construction, note list building) happens here.

use std::collections::HashMap;
use std::sync::Arc;

use rtrb::{Consumer, Producer, RingBuffer};

use super::Engine;
use super::commands::{Command, FxTarget, Garbage, Handle, RecEvent};
use super::shared::{MAX_TRACKS, Shared};
use super::track::{AudioClipRt, FxSlot, NoteRt, TrackRt};
use crate::assets::AssetCache;
use crate::dsp::{db_to_gain, registry};
use crate::model::{AutoPoint, Clip, ClipKind, DeviceState, Project, Track};

fn envelope(points: &[AutoPoint]) -> Box<Vec<(f64, f32)>> {
    let mut v: Vec<(f64, f32)> = points.iter().filter(|p| p.value.is_finite()).map(|p| (p.beat.max(0.0), p.value)).collect();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Box::new(v)
}

#[derive(Clone, PartialEq)]
struct FxMirror {
    id: String,
    type_id: String,
    enabled: bool,
    params: HashMap<String, f32>,
}

struct TrackMirror {
    handle: Handle,
    instrument: Option<(String, HashMap<String, f32>)>,
    fx: Vec<FxMirror>,
    clips: Vec<Clip>,
    /// Asset ids that were available when the clips were last built.
    loaded_assets: usize,
    mix: (f32, f32, bool, bool, bool),
    automation: crate::model::Automation,
}

pub struct Controller {
    producer: Producer<Command>,
    garbage: Consumer<Garbage>,
    rec_events: Consumer<RecEvent>,
    pub shared: Arc<Shared>,
    pub sample_rate: f32,
    pub assets: AssetCache,
    project: Project,
    mirrors: HashMap<String, TrackMirror>,
    handle_ids: HashMap<Handle, String>,
    free: Vec<Handle>,
    master_fx: Vec<FxMirror>,
    live_target: Option<String>,
    synced_once: bool,
    /// A command was dropped (engine not draining): resync everything when possible.
    overflow: bool,
    /// Offline render (export): external plugins get private instances.
    pub offline: bool,
}

fn make_rings(sr: f32, shared: Arc<Shared>) -> (Producer<Command>, Consumer<Garbage>, Consumer<RecEvent>, Engine) {
    let (cmd_p, cmd_c) = RingBuffer::<Command>::new(4096);
    let (gar_p, gar_c) = RingBuffer::<Garbage>::new(4096);
    let (rec_p, rec_c) = RingBuffer::<RecEvent>::new(4096);
    let engine = Engine::new(sr, cmd_c, gar_p, rec_p, shared);
    (cmd_p, gar_c, rec_c, engine)
}

impl Controller {
    pub fn new(sample_rate: f32) -> (Self, Engine) {
        let shared = Arc::new(Shared::new(sample_rate));
        let (producer, garbage, rec_events, engine) = make_rings(sample_rate, shared.clone());
        let c = Self {
            producer,
            garbage,
            rec_events,
            shared,
            sample_rate,
            assets: AssetCache::default(),
            project: Project::default(),
            mirrors: HashMap::new(),
            handle_ids: HashMap::new(),
            free: (0..MAX_TRACKS as Handle).rev().collect(),
            master_fx: Vec::new(),
            live_target: None,
            synced_once: false,
            overflow: false,
            offline: false,
        };
        (c, engine)
    }

    /// Creates a fresh engine (e.g. after a sample-rate change) and re-sends
    /// the whole project to it.
    pub fn rebuild(&mut self, sample_rate: f32) -> Engine {
        use std::sync::atomic::Ordering;
        let pos = self.shared.position.load();
        let was_playing = self.shared.playing.load(Ordering::Relaxed);
        // The new engine starts stopped and not recording: make the shared state agree.
        self.shared.recording.store(false, Ordering::Release);
        self.shared.count_in.store(false, Ordering::Relaxed);
        self.shared.playing.store(false, Ordering::Relaxed);
        self.overflow = false;
        self.shared.sample_rate.store(sample_rate);
        let (producer, garbage, rec_events, mut engine) = make_rings(sample_rate, self.shared.clone());
        self.producer = producer;
        self.garbage = garbage;
        self.rec_events = rec_events;
        self.sample_rate = sample_rate;
        self.mirrors.clear();
        self.handle_ids.clear();
        self.free = (0..MAX_TRACKS as Handle).rev().collect();
        self.master_fx.clear();
        self.synced_once = false;
        let project = std::mem::take(&mut self.project);
        self.sync(&project);
        let lt = self.live_target.take();
        self.set_live_target(lt);
        engine.set_position(pos);
        if was_playing {
            self.send(Command::Play);
        }
        engine
    }

    /// Non-blocking. If the queue is full (audio callback not running), the
    /// command is dropped and the next `sync` rebuilds the engine state.
    pub fn send(&mut self, cmd: Command) {
        if self.overflow {
            return;
        }
        if self.producer.push(cmd).is_err() {
            if !self.overflow {
                eprintln!("[engine] command queue full (no audio callback?), will resync");
            }
            self.overflow = true;
        }
    }

    pub fn handle_of(&self, track_id: &str) -> Option<Handle> {
        self.mirrors.get(track_id).map(|m| m.handle)
    }

    /// Track listening to MIDI channel `channel` (1..16), if any.
    pub fn midi_route(&self, channel: u8) -> Option<Handle> {
        self.project
            .tracks
            .iter()
            .find(|t| t.midi_channel == Some(channel) && t.kind == crate::model::TrackKind::Instrument)
            .and_then(|t| self.handle_of(&t.id))
    }

    /// A note from a MIDI controller: goes to the track listening to its channel,
    /// else to the live target. Recordable either way.
    pub fn midi_note(&mut self, channel: u8, pitch: u8, velocity: f32, on: bool) {
        let handle = self.midi_route(channel);
        self.send(Command::LiveNote { handle, pitch: pitch.min(127), velocity: velocity.clamp(0.0, 1.0), on, record: true });
    }

    pub fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage.pop() {
            drop(g);
        }
    }

    pub fn drain_recorded(&mut self) -> Vec<(String, RecEvent)> {
        let mut v = Vec::new();
        while let Ok(e) = self.rec_events.pop() {
            if let Some(id) = self.handle_ids.get(&e.handle) {
                v.push((id.clone(), e));
            }
        }
        v
    }

    pub fn track_meters(&self) -> Vec<(String, f32, f32)> {
        self.handle_ids
            .iter()
            .map(|(h, id)| {
                let (l, r) = self.shared.meters[*h as usize].take();
                (id.clone(), l, r)
            })
            .collect()
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn set_live_target(&mut self, track_id: Option<String>) {
        let h = track_id.as_ref().and_then(|id| self.mirrors.get(id)).map(|m| m.handle);
        self.live_target = track_id;
        self.send(Command::SetLiveTarget(h));
    }

    fn build_effects(&self, devices: &[DeviceState]) -> (Vec<FxSlot>, Vec<FxMirror>) {
        let mut slots = Vec::with_capacity(devices.len().max(8));
        let mut mirrors = Vec::new();
        for d in devices {
            let fx = match registry::spec(&d.type_id) {
                Some(spec) => registry::create_effect(&d.type_id, self.sample_rate).map(|mut fx| {
                    fx.set_params(&spec.values_from_map(&d.params));
                    fx
                }),
                None => registry::external()
                    .filter(|e| e.handles(&d.type_id))
                    .and_then(|e| e.create_effect(d, self.sample_rate, self.offline)),
            };
            let Some(fx) = fx else { continue };
            slots.push(FxSlot { enabled: d.enabled, effect: fx });
            mirrors.push(FxMirror {
                id: d.id.clone(),
                type_id: d.type_id.clone(),
                enabled: d.enabled,
                params: d.params.clone(),
            });
        }
        (slots, mirrors)
    }

    fn known_effects(devices: &[DeviceState]) -> Vec<&DeviceState> {
        devices.iter().filter(|d| registry::is_known(&d.type_id)).collect()
    }

    /// Diffs an effect chain; returns the new mirror list if it changed.
    fn sync_effects(&mut self, target: FxTarget, old: &[FxMirror], devices: &[DeviceState]) -> Option<Vec<FxMirror>> {
        let known = Self::known_effects(devices);
        let same_structure = old.len() == known.len()
            && old.iter().zip(known.iter()).all(|(m, d)| m.id == d.id && m.type_id == d.type_id);
        if !same_structure {
            let (slots, mirrors) = self.build_effects(devices);
            self.send(Command::SetEffects { target, slots: Box::new(slots) });
            return Some(mirrors);
        }
        let mut changed = false;
        let mut new = old.to_vec();
        for (i, (m, d)) in old.iter().zip(known.iter()).enumerate() {
            if m.enabled != d.enabled || m.params != d.params {
                // External plugins keep their own parameters: only "enabled" travels.
                let values = registry::spec(&d.type_id)
                    .map(|spec| spec.values_from_map(&d.params))
                    .unwrap_or([0.0; crate::dsp::MAX_PARAMS]);
                self.send(Command::SetEffectParams { target, index: i, enabled: d.enabled, values: Box::new(values) });
                new[i].enabled = d.enabled;
                new[i].params = d.params.clone();
                changed = true;
            }
        }
        if changed { Some(new) } else { None }
    }

    pub fn build_notes(clips: &[Clip]) -> Vec<NoteRt> {
        let mut out = Vec::new();
        for c in clips.iter().filter(|c| c.kind == ClipKind::Midi && !c.muted && c.length > 0.0) {
            let clip_end = c.start + c.length;
            let loop_len = c.loop_length;
            let mut push = |content_pos: f64, n: &crate::model::Note| {
                let t = c.start + content_pos - c.offset;
                if t >= c.start - 1e-9 && t < clip_end {
                    let mut end = (t + n.length.max(0.01)).min(clip_end);
                    if loop_len > 0.0 {
                        // A note never rings into the next pass of the loop.
                        end = end.min(t - n.start + loop_len);
                    }
                    out.push(NoteRt {
                        start: t.max(c.start),
                        end,
                        pitch: n.pitch.min(127),
                        velocity: (n.velocity / 127.0).clamp(0.0, 1.0),
                    });
                }
            };
            if c.loop_length > 0.0 {
                let l = c.loop_length;
                let first = (c.offset / l).floor() as i64;
                let last = ((c.offset + c.length) / l).ceil() as i64;
                for k in first..=last {
                    for n in c.notes.iter().filter(|n| n.start < l && n.start >= 0.0) {
                        push(n.start + k as f64 * l, n);
                    }
                }
            } else {
                for n in &c.notes {
                    push(n.start, n);
                }
            }
        }
        out.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    fn build_audio_clips(&self, clips: &[Clip]) -> Vec<AudioClipRt> {
        clips
            .iter()
            .filter(|c| c.kind == ClipKind::Audio && !c.muted && c.length > 0.0)
            .filter_map(|c| {
                let buffer = self.assets.get(c.asset_id.as_deref()?)?;
                Some(AudioClipRt {
                    start: c.start,
                    end: c.start + c.length,
                    offset_sec: c.offset,
                    gain: db_to_gain(c.gain),
                    fade_in: c.fade_in,
                    fade_out: c.fade_out,
                    buffer,
                })
            })
            .collect()
    }

    fn loaded_count(&self, clips: &[Clip]) -> usize {
        clips
            .iter()
            .filter(|c| c.kind == ClipKind::Audio)
            .filter(|c| c.asset_id.as_deref().map(|a| self.assets.buffers.contains_key(a)).unwrap_or(false))
            .count()
    }

    fn create_instrument(&self, d: &DeviceState) -> Option<Box<dyn crate::dsp::Instrument>> {
        let Some(spec) = registry::spec(&d.type_id) else {
            return registry::external()
                .filter(|e| e.handles(&d.type_id))
                .and_then(|e| e.create_instrument(d, self.sample_rate, self.offline));
        };
        let mut inst = registry::create_instrument(&d.type_id, self.sample_rate)?;
        inst.set_params(&spec.values_from_map(&d.params));
        Some(inst)
    }

    fn add_track(&mut self, t: &Track) {
        let Some(handle) = self.free.pop() else {
            eprintln!("[engine] too many tracks");
            return;
        };
        let mut rt = TrackRt::new(handle, self.sample_rate);
        rt.instrument = t.instrument.as_ref().and_then(|d| self.create_instrument(d));
        let (slots, fx) = self.build_effects(&t.effects);
        *rt.effects = slots;
        *rt.notes = Self::build_notes(&t.clips);
        *rt.audio_clips = self.build_audio_clips(&t.clips);
        rt.volume_db = t.volume;
        rt.pan = t.pan;
        rt.mute = t.mute;
        rt.solo = t.solo;
        rt.armed = t.armed;
        rt.auto_volume = envelope(&t.automation.volume);
        rt.auto_pan = envelope(&t.automation.pan);
        let (gl, gr) = rt.target_gains(false);
        rt.gain_l.snap(gl);
        rt.gain_r.snap(gr);
        self.send(Command::AddTrack(Box::new(rt)));
        self.mirrors.insert(
            t.id.clone(),
            TrackMirror {
                handle,
                instrument: t.instrument.as_ref().map(|d| (d.type_id.clone(), d.params.clone())),
                fx,
                clips: t.clips.clone(),
                loaded_assets: self.loaded_count(&t.clips),
                mix: (t.volume, t.pan, t.mute, t.solo, t.armed),
                automation: t.automation.clone(),
            },
        );
        self.handle_ids.insert(handle, t.id.clone());
        if self.live_target.as_deref() == Some(t.id.as_str()) {
            self.send(Command::SetLiveTarget(Some(handle)));
        }
    }

    fn update_track(&mut self, t: &Track) {
        let Some(mut m) = self.mirrors.remove(&t.id) else { return };
        let h = m.handle;

        let mix = (t.volume, t.pan, t.mute, t.solo, t.armed);
        if mix != m.mix {
            self.send(Command::SetTrackMix {
                handle: h,
                volume_db: t.volume,
                pan: t.pan,
                mute: t.mute,
                solo: t.solo,
                armed: t.armed,
            });
            m.mix = mix;
        }

        if t.automation != m.automation {
            self.send(Command::SetAutomation {
                handle: h,
                volume: envelope(&t.automation.volume),
                pan: envelope(&t.automation.pan),
            });
            m.automation = t.automation.clone();
        }

        let new_inst = t.instrument.as_ref().map(|d| (d.type_id.clone(), d.params.clone()));
        if new_inst != m.instrument {
            let same_type = matches!((&new_inst, &m.instrument), (Some(a), Some(b)) if a.0 == b.0);
            if same_type {
                let d = t.instrument.as_ref().unwrap();
                if let Some(spec) = registry::spec(&d.type_id) {
                    self.send(Command::SetInstrumentParams {
                        handle: h,
                        values: Box::new(spec.values_from_map(&d.params)),
                    });
                }
            } else {
                let inst = t.instrument.as_ref().and_then(|d| self.create_instrument(d));
                self.send(Command::SetInstrument { handle: h, instrument: inst });
            }
            m.instrument = new_inst;
        }

        if let Some(fx) = self.sync_effects(FxTarget::Track(h), &m.fx.clone(), &t.effects) {
            m.fx = fx;
        }

        let loaded = self.loaded_count(&t.clips);
        if t.clips != m.clips || loaded != m.loaded_assets {
            let notes = Self::build_notes(&t.clips);
            self.send(Command::SetNotes { handle: h, notes: Box::new(notes) });
            let audio = self.build_audio_clips(&t.clips);
            self.send(Command::SetAudioClips { handle: h, clips: Box::new(audio) });
            m.clips = t.clips.clone();
            m.loaded_assets = loaded;
        }
        self.mirrors.insert(t.id.clone(), m);
    }

    /// Applies a new project document to the engine (only what changed).
    pub fn sync(&mut self, p: &Project) {
        self.collect_garbage();
        if self.overflow {
            // Wait until the engine drained most of the queue, then start over.
            if self.producer.slots() < 3000 {
                self.project = p.clone();
                return;
            }
            self.overflow = false;
            self.mirrors.clear();
            self.handle_ids.clear();
            self.free = (0..MAX_TRACKS as Handle).rev().collect();
            self.master_fx.clear();
            self.synced_once = false;
            self.send(Command::Reset { empty_fx: Box::new(Vec::with_capacity(16)) });
            let lt = self.live_target.clone();
            self.project = Project::default();
            self.sync(p);
            self.set_live_target(lt);
            return;
        }
        let first = !self.synced_once;
        let old = std::mem::take(&mut self.project);

        if first || p.bpm != old.bpm {
            self.send(Command::SetTempo(if p.bpm > 0.0 { p.bpm } else { 120.0 }));
        }
        if first || p.time_sig_num != old.time_sig_num || p.time_sig_den != old.time_sig_den {
            self.send(Command::SetBeatsPerBar(p.beats_per_bar()));
        }
        if first || p.loop_region != old.loop_region {
            self.send(Command::SetLoop {
                enabled: p.loop_region.enabled,
                start: p.loop_region.start,
                end: p.loop_region.end,
            });
        }
        if first || p.metronome != old.metronome {
            self.send(Command::SetMetronome {
                enabled: p.metronome.enabled,
                volume: p.metronome.volume,
                count_in_bars: p.metronome.count_in_bars,
            });
        }
        if first || p.master.volume != old.master.volume {
            self.send(Command::SetMasterVolume(p.master.volume));
        }
        let master_old = self.master_fx.clone();
        if let Some(fx) = self.sync_effects(FxTarget::Master, &master_old, &p.master.effects) {
            self.master_fx = fx;
        }

        // Removed tracks.
        let ids: std::collections::HashSet<&str> = p.tracks.iter().map(|t| t.id.as_str()).collect();
        let removed: Vec<String> = self.mirrors.keys().filter(|k| !ids.contains(k.as_str())).cloned().collect();
        for id in removed {
            if let Some(m) = self.mirrors.remove(&id) {
                self.send(Command::RemoveTrack(m.handle));
                self.handle_ids.remove(&m.handle);
                self.free.push(m.handle);
            }
        }
        for t in &p.tracks {
            if self.mirrors.contains_key(&t.id) {
                self.update_track(t);
            } else {
                self.add_track(t);
            }
        }
        self.project = p.clone();
        self.synced_once = true;
    }
}
