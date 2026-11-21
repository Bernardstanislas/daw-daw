//! The real-time audio engine. `Engine` lives on the audio thread and is fed
//! by `Controller` (control thread) through a lock-free command queue.

pub mod commands;
pub mod controller;
pub mod metronome;
pub mod shared;
pub mod spectrum;
pub mod track;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use rtrb::{Consumer, Producer};

use crate::assets::AudioBuffer;
use crate::dsp::util::Smoother;
use crate::dsp::{ProcessCtx, db_to_gain};
use commands::{Command, FxTarget, Garbage, Handle, RecEvent};
use metronome::Metronome;
use shared::Shared;
use track::{FxSlot, TrackRt};

/// Internal processing block size (frames). Larger device buffers are split.
pub const BLOCK: usize = 256;
const MAX_EVENTS: usize = 512;

#[derive(Clone, Copy)]
struct SeqEvent {
    frame: usize,
    /// Insertion order: events on the same frame keep the order they were scheduled in.
    seq: u32,
    pitch: u8,
    velocity: f32,
    on: bool,
}

#[derive(Clone, Copy)]
struct LiveEvent {
    /// None = the current live target track.
    handle: Option<Handle>,
    pitch: u8,
    velocity: f32,
    on: bool,
    /// Captured into recordings (false for editor previews).
    record: bool,
}

const MAX_SEGS: usize = 8;

struct Transport {
    playing: bool,
    pos: f64,
    bpm: f64,
    beats_per_bar: f64,
    loop_enabled: bool,
    loop_start: f64,
    loop_end: f64,
    /// Timeline position at which the count-in ends (recording starts).
    count_in_until: Option<f64>,
    record_requested: bool,
    recording: bool,
    count_in_bars: u32,
}

struct Preview {
    buffer: Arc<AudioBuffer>,
    pos: f64,
}

pub struct Engine {
    sr: f32,
    commands: Consumer<Command>,
    garbage: Producer<Garbage>,
    rec_events: Producer<RecEvent>,
    shared: Arc<Shared>,
    transport: Transport,
    tracks: Vec<Box<TrackRt>>,
    master_fx: Box<Vec<FxSlot>>,
    master_gain: Smoother,
    master_volume_db: f32,
    metronome: Metronome,
    live_target: Option<Handle>,
    live_pending: Vec<LiveEvent>,
    /// Fade the next rendered audio in (after a seek / start) to avoid clicks.
    declick: bool,
    preview: Option<Preview>,
    events: Vec<SeqEvent>,
    bus_l: Vec<f32>,
    bus_r: Vec<f32>,
    trk_l: Vec<f32>,
    trk_r: Vec<f32>,
}

impl Engine {
    pub fn new(
        sr: f32,
        commands: Consumer<Command>,
        garbage: Producer<Garbage>,
        rec_events: Producer<RecEvent>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            sr,
            commands,
            garbage,
            rec_events,
            shared,
            transport: Transport {
                playing: false,
                pos: 0.0,
                bpm: 120.0,
                beats_per_bar: 4.0,
                loop_enabled: false,
                loop_start: 0.0,
                loop_end: 16.0,
                count_in_until: None,
                record_requested: false,
                recording: false,
                count_in_bars: 1,
            },
            tracks: Vec::with_capacity(shared::MAX_TRACKS),
            master_fx: Box::new(Vec::new()),
            master_gain: Smoother::new(1.0, 20.0, sr),
            master_volume_db: 0.0,
            metronome: Metronome::new(sr),
            live_target: None,
            live_pending: Vec::with_capacity(256),
            declick: true,
            preview: None,
            events: Vec::with_capacity(MAX_EVENTS),
            bus_l: vec![0.0; BLOCK],
            bus_r: vec![0.0; BLOCK],
            trk_l: vec![0.0; BLOCK],
            trk_r: vec![0.0; BLOCK],
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    pub fn position(&self) -> f64 {
        self.transport.pos
    }

    pub fn set_position(&mut self, pos: f64) {
        self.transport.pos = pos;
        self.shared.position.store(pos);
    }

    fn trash(&mut self, g: Garbage) {
        // If the queue is full we have no choice but to drop here.
        let _ = self.garbage.push(g);
    }

    fn track_mut(&mut self, h: Handle) -> Option<&mut TrackRt> {
        self.tracks.iter_mut().find(|t| t.handle == h).map(|b| &mut **b)
    }

    fn fx_slots(&mut self, target: FxTarget) -> Option<&mut Box<Vec<FxSlot>>> {
        match target {
            FxTarget::Master => Some(&mut self.master_fx),
            FxTarget::Track(h) => self.track_mut(h).map(|t| &mut t.effects),
        }
    }

    fn release_all_sequenced(&mut self) {
        for t in self.tracks.iter_mut() {
            t.release_sequenced();
        }
    }

    fn stop_transport(&mut self) {
        let was_playing = self.transport.playing;
        if let Some(ci) = self.transport.count_in_until {
            self.transport.pos = ci;
            self.shared.position.store(ci);
        }
        self.transport.playing = false;
        self.transport.count_in_until = None;
        self.transport.record_requested = false;
        self.transport.recording = false;
        self.shared.recording.store(false, Ordering::Release);
        self.shared.count_in.store(false, Ordering::Relaxed);
        self.shared.playing.store(false, Ordering::Relaxed);
        self.release_all_sequenced();
        if !was_playing {
            // Second stop = panic-ish: silence lingering voices.
            for t in self.tracks.iter_mut() {
                if let Some(i) = t.instrument.as_mut() {
                    i.all_notes_off(false);
                }
            }
        }
    }

    fn handle_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::Play => {
                    if !self.transport.playing {
                        self.declick = true;
                        self.transport.playing = true;
                        self.shared.playing.store(true, Ordering::Relaxed);
                    }
                }
                Command::Stop => self.stop_transport(),
                Command::Record => {
                    let start = self.transport.pos;
                    self.transport.record_requested = true;
                    let count = self.transport.count_in_bars as f64 * self.transport.beats_per_bar;
                    if count > 0.0 && !self.transport.playing {
                        self.transport.count_in_until = Some(start);
                        self.transport.pos = start - count;
                        self.shared.count_in.store(true, Ordering::Relaxed);
                    } else {
                        self.begin_recording(start);
                    }
                    self.declick = true;
                    self.transport.playing = true;
                    self.shared.playing.store(true, Ordering::Relaxed);
                }
                Command::Seek(p) => {
                    self.release_all_sequenced();
                    self.transport.pos = p.max(0.0);
                    // A seek ends any count-in / take (the take stays anchored where it started).
                    self.transport.count_in_until = None;
                    self.transport.record_requested = false;
                    self.transport.recording = false;
                    self.shared.recording.store(false, Ordering::Release);
                    self.shared.count_in.store(false, Ordering::Relaxed);
                    self.shared.position.store(self.transport.pos);
                    self.declick = true;
                }
                Command::Reset { empty_fx } => {
                    while let Some(t) = self.tracks.pop() {
                        self.trash(Garbage::Track(t));
                    }
                    let old = std::mem::replace(&mut self.master_fx, empty_fx);
                    self.trash(Garbage::Effects(old));
                    self.live_target = None;
                }
                Command::SetTempo(bpm) => self.transport.bpm = bpm.clamp(20.0, 400.0),
                Command::SetBeatsPerBar(b) => self.transport.beats_per_bar = b.max(0.25),
                Command::SetLoop { enabled, start, end } => {
                    // Loops shorter than a sixteenth are ignored (keeps segments per block bounded).
                    self.transport.loop_enabled = enabled && end > start + 0.25;
                    self.transport.loop_start = start;
                    self.transport.loop_end = end;
                }
                Command::SetMetronome { enabled, volume, count_in_bars } => {
                    self.metronome.enabled = enabled;
                    self.metronome.volume = volume;
                    self.transport.count_in_bars = count_in_bars;
                }
                Command::AddTrack(t) => {
                    if self.tracks.len() < self.tracks.capacity() {
                        self.tracks.push(t);
                    } else {
                        self.trash(Garbage::Track(t));
                    }
                }
                Command::RemoveTrack(h) => {
                    if let Some(i) = self.tracks.iter().position(|t| t.handle == h) {
                        let t = self.tracks.swap_remove(i);
                        self.trash(Garbage::Track(t));
                        if self.live_target == Some(h) {
                            self.live_target = None;
                        }
                    }
                }
                Command::SetTrackMix { handle, volume_db, pan, mute, solo, armed } => {
                    if let Some(t) = self.track_mut(handle) {
                        t.volume_db = volume_db;
                        t.pan = pan;
                        t.mute = mute;
                        t.solo = solo;
                        t.armed = armed;
                    }
                }
                Command::SetInstrument { handle, instrument } => {
                    let old = self.track_mut(handle).map(|t| {
                        t.active_len = 0;
                        std::mem::replace(&mut t.instrument, instrument)
                    });
                    if let Some(Some(old)) = old {
                        self.trash(Garbage::Instrument(old));
                    }
                }
                Command::SetInstrumentParams { handle, values } => {
                    if let Some(t) = self.track_mut(handle)
                        && let Some(i) = t.instrument.as_mut() {
                            i.set_params(&values);
                        }
                    self.trash(Garbage::Params(values));
                }
                Command::SetEffects { target, slots } => {
                    let old = self.fx_slots(target).map(|s| std::mem::replace(s, slots));
                    if let Some(old) = old { self.trash(Garbage::Effects(old)) }
                }
                Command::SetEffectParams { target, index, enabled, values } => {
                    if let Some(slots) = self.fx_slots(target)
                        && let Some(slot) = slots.get_mut(index) {
                            if slot.enabled != enabled {
                                slot.effect.reset();
                            }
                            slot.enabled = enabled;
                            slot.effect.set_params(&values);
                        }
                    self.trash(Garbage::Params(values));
                }
                Command::SetNotes { handle, notes } => {
                    let old = self.track_mut(handle).map(|t| std::mem::replace(&mut t.notes, notes));
                    if let Some(old) = old {
                        self.trash(Garbage::Notes(old));
                    }
                }
                Command::SetAudioClips { handle, clips } => {
                    let old = self.track_mut(handle).map(|t| std::mem::replace(&mut t.audio_clips, clips));
                    if let Some(old) = old {
                        self.trash(Garbage::AudioClips(old));
                    }
                }
                Command::SetAutomation { handle, volume, pan } => {
                    let old = self.track_mut(handle).map(|t| {
                        (std::mem::replace(&mut t.auto_volume, volume), std::mem::replace(&mut t.auto_pan, pan))
                    });
                    if let Some((v, p)) = old {
                        self.trash(Garbage::Envelope(v));
                        self.trash(Garbage::Envelope(p));
                    }
                }
                Command::SetMasterVolume(db) => self.master_volume_db = db,
                Command::SetLiveTarget(h) => {
                    if self.live_target != h {
                        if let Some(old) = self.live_target
                            && let Some(t) = self.track_mut(old)
                                && let Some(i) = t.instrument.as_mut() {
                                    i.all_notes_off(false);
                                }
                        self.live_target = h;
                    }
                }
                Command::LiveNote { handle, pitch, velocity, on, record } => {
                    if self.live_pending.len() < self.live_pending.capacity() {
                        self.live_pending.push(LiveEvent { handle, pitch, velocity, on, record });
                    }
                }
                Command::AllNotesOff => {
                    for t in self.tracks.iter_mut() {
                        t.active_len = 0;
                        if let Some(i) = t.instrument.as_mut() {
                            i.all_notes_off(true);
                        }
                        for s in t.effects.iter_mut() {
                            s.effect.reset();
                        }
                    }
                    for s in self.master_fx.iter_mut() {
                        s.effect.reset();
                    }
                    if let Some(p) = self.preview.take() {
                        self.trash(Garbage::Buffer(p.buffer));
                    }
                }
                Command::Preview(b) => {
                    if let Some(old) = self.preview.take() {
                        self.trash(Garbage::Buffer(old.buffer));
                    }
                    self.preview = b.map(|buffer| Preview { buffer, pos: 0.0 });
                }
            }
        }
    }

    fn begin_recording(&mut self, at: f64) {
        self.transport.count_in_until = None;
        self.transport.recording = true;
        self.shared.count_in.store(false, Ordering::Relaxed);
        self.shared.record_start.store(at);
        self.shared.recording.store(true, Ordering::Release);
    }

    /// Renders interleaved output. `channels` is the device channel count.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        // No monotonic clock in wasm32-unknown-unknown: the browser host reports load itself.
        #[cfg(not(target_arch = "wasm32"))]
        let t0 = Instant::now();
        self.handle_commands();
        let channels = channels.max(1);
        let total = out.len() / channels;
        let mut done = 0;
        while done < total {
            let n = (total - done).min(BLOCK);
            self.render_block(n);
            for i in 0..n {
                let base = (done + i) * channels;
                let l = self.bus_l[i].clamp(-1.0, 1.0);
                let r = self.bus_r[i].clamp(-1.0, 1.0);
                if channels == 1 {
                    out[base] = 0.5 * (l + r);
                } else {
                    out[base] = l;
                    out[base + 1] = r;
                    for c in 2..channels {
                        out[base + c] = 0.0;
                    }
                }
            }
            done += n;
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.report_load(t0.elapsed().as_secs_f32(), total);
    }

    /// Feeds the DSP-load meter: `elapsed` seconds spent rendering `frames` frames.
    pub fn report_load(&mut self, elapsed: f32, frames: usize) {
        let budget = frames as f32 / self.sr;
        if budget > 0.0 {
            let load = elapsed / budget;
            let prev = self.shared.cpu.load();
            self.shared.cpu.store(prev * 0.9 + load * 0.1);
        }
    }

    /// Renders `n <= BLOCK` frames into bus_l / bus_r.
    pub fn render_block(&mut self, n: usize) {
        let bpm = self.transport.bpm;
        let beats_per_frame = bpm / 60.0 / self.sr as f64;
        let frames_per_beat = 1.0 / beats_per_frame;
        self.bus_l[..n].fill(0.0);
        self.bus_r[..n].fill(0.0);

        // --- Timeline segments for this block (split at loop end / count-in end).
        let mut segs: [(usize, usize, f64); MAX_SEGS] = [(0, 0, 0.0); MAX_SEGS];
        // Upper bound (beats) for note-ons of each segment: the loop end for the segment
        // that wraps, so a note exactly at loop_end is not started (and stuck) before the jump.
        let mut seg_limit = [f64::INFINITY; MAX_SEGS];
        // Segment ends with a jump back to the loop start.
        let mut seg_wraps = [false; MAX_SEGS];
        let mut nseg = 0;
        let mut seq_active = [false; MAX_SEGS];
        if self.transport.playing {
            let mut f = 0usize;
            let mut pos = self.transport.pos;
            while f < n && nseg < MAX_SEGS {
                let remaining = n - f;
                let mut len = remaining;
                let mut boundary: Option<f64> = None;
                if let Some(ci) = self.transport.count_in_until
                    && pos < ci {
                        let fr = ((ci - pos) * frames_per_beat).ceil() as usize;
                        if fr <= len {
                            len = fr.max(1);
                            boundary = Some(ci);
                        }
                    }
                // No loop while recording: takes are linear.
                let looping = self.transport.loop_enabled
                    && !self.transport.recording
                    && self.transport.count_in_until.is_none()
                    && pos < self.transport.loop_end;
                if looping {
                    let fr = ((self.transport.loop_end - pos) * frames_per_beat).ceil() as usize;
                    if fr <= len {
                        len = fr.max(1);
                        boundary = Some(-1.0);
                    }
                }
                segs[nseg] = (f, len, pos);
                seq_active[nseg] = self.transport.count_in_until.map(|ci| pos >= ci).unwrap_or(true);
                if boundary == Some(-1.0) {
                    seg_limit[nseg] = self.transport.loop_end;
                    seg_wraps[nseg] = true;
                }
                nseg += 1;
                f += len;
                pos += len as f64 * beats_per_frame;
                match boundary {
                    Some(b) if b < 0.0 => {
                        pos = self.transport.loop_start;
                    }
                    Some(ci) => {
                        pos = ci;
                        if self.transport.record_requested {
                            self.begin_recording(ci);
                        } else {
                            self.transport.count_in_until = None;
                            self.shared.count_in.store(false, Ordering::Relaxed);
                        }
                    }
                    None => {}
                }
            }
            // Safety net: never leave a count-in hanging past its end.
            if let Some(ci) = self.transport.count_in_until
                && pos >= ci
            {
                if self.transport.record_requested {
                    self.begin_recording(ci);
                } else {
                    self.transport.count_in_until = None;
                    self.shared.count_in.store(false, Ordering::Relaxed);
                }
            }
            // Any frames not covered (pathological tiny loops) just advance time.
            if f < n {
                pos += (n - f) as f64 * beats_per_frame;
            }
            // If the count-in was still running at the start, mark its segments inactive.
            if let Some(ci) = self.transport.count_in_until {
                for (i, s) in segs.iter().enumerate().take(nseg) {
                    if s.2 < ci {
                        seq_active[i] = false;
                    }
                }
            }
            self.transport.pos = pos;
        }

        let ctx = ProcessCtx {
            sample_rate: self.sr,
            bpm: bpm as f32,
            playing: self.transport.playing,
            beat: if nseg > 0 { segs[0].2 } else { self.transport.pos },
        };
        let any_solo = self.tracks.iter().any(|t| t.solo);
        let declick = std::mem::replace(&mut self.declick, false) && nseg > 0;
        let recording = self.transport.recording;
        let live_target = self.live_target;

        // Live (keyboard / MIDI) events are applied at frame 0 of the block.
        let mut live = std::mem::take(&mut self.live_pending);

        for ti in 0..self.tracks.len() {
            let t = &mut self.tracks[ti];
            let tl = &mut self.trk_l[..n];
            let tr = &mut self.trk_r[..n];
            tl.fill(0.0);
            tr.fill(0.0);

            // Collect sequencer events.
            self.events.clear();
            let mut seq = 0u32;
            let events = &mut self.events;
            let mut push = |frame: usize, pitch: u8, velocity: f32, on: bool| {
                if events.len() < MAX_EVENTS {
                    events.push(SeqEvent { frame, seq, pitch, velocity, on });
                    seq += 1;
                }
            };
            if t.instrument.is_some() {
                for s in 0..nseg {
                    if !seq_active[s] {
                        continue;
                    }
                    let (f0, len, b0) = segs[s];
                    let b1 = b0 + len as f64 * beats_per_frame;
                    // Note offs for active notes ending in this segment.
                    let mut k = 0;
                    while k < t.active_len {
                        let (p, end) = t.active[k];
                        if end < b1 {
                            let fr = f0 + (((end - b0).max(0.0) * frames_per_beat) as usize).min(len - 1);
                            push(fr, p, 0.0, false);
                            t.active_len -= 1;
                            t.active[k] = t.active[t.active_len];
                        } else {
                            k += 1;
                        }
                    }
                    // Note ons.
                    let notes = &t.notes;
                    let limit = b1.min(seg_limit[s]);
                    let mut idx = notes.partition_point(|x| x.start < b0);
                    while idx < notes.len() && notes[idx].start < limit {
                        let nt = notes[idx];
                        idx += 1;
                        let ends_here = nt.end < b1;
                        if !ends_here && t.active_len >= track::MAX_ACTIVE {
                            continue; // could never be released: skip rather than stick
                        }
                        let fr = f0 + (((nt.start - b0) * frames_per_beat) as usize).min(len - 1);
                        // Same pitch still sounding (overlapping notes): end it first.
                        if let Some(k) = (0..t.active_len).find(|&k| t.active[k].0 == nt.pitch) {
                            push(fr, nt.pitch, 0.0, false);
                            t.active_len -= 1;
                            t.active[k] = t.active[t.active_len];
                        }
                        push(fr, nt.pitch, nt.velocity, true);
                        if ends_here {
                            let fe = f0 + (((nt.end - b0) * frames_per_beat) as usize).min(len - 1);
                            push(fe.max(fr), nt.pitch, 0.0, false);
                        } else {
                            t.active[t.active_len] = (nt.pitch, nt.end);
                            t.active_len += 1;
                        }
                    }
                    // Loop wrap: release what is still sounding at the end of the segment.
                    if seg_wraps[s] {
                        let fr = f0 + len - 1;
                        for k in 0..t.active_len {
                            push(fr, t.active[k].0, 0.0, false);
                        }
                        t.active_len = 0;
                    }
                }
            }
            for ev in live.iter() {
                if ev.handle.or(live_target) != Some(t.handle) {
                    continue;
                }
                push(0, ev.pitch, ev.velocity, ev.on);
                if ev.record && recording && t.armed {
                        let beat = ctx.beat;
                        let _ = self.rec_events.push(RecEvent {
                            handle: t.handle,
                            pitch: ev.pitch,
                            velocity: ev.velocity,
                            on: ev.on,
                            beat: beat.max(0.0),
                        });
                }
            }

            // Audio clips (rendered first so the de-click ramps only touch audio).
            let ramp = ((0.003 * self.sr) as usize).max(1);
            for s in 0..nseg {
                if !seq_active[s] {
                    continue;
                }
                let (f0, len, b0) = segs[s];
                let b1 = b0 + len as f64 * beats_per_frame;
                let (sl, sr_) = (&mut tl[f0..f0 + len], &mut tr[f0..f0 + len]);
                t.render_audio_clips(b0, b1, bpm, self.sr, sl, sr_);
                let fade_in = (s == 0 && declick) || (s > 0 && seg_wraps[s - 1]);
                if fade_in {
                    for i in 0..ramp.min(len) {
                        let g = i as f32 / ramp as f32;
                        sl[i] *= g;
                        sr_[i] *= g;
                    }
                }
                if seg_wraps[s] {
                    let r = ramp.min(len);
                    for i in 0..r {
                        let g = i as f32 / r as f32;
                        sl[len - 1 - i] *= g;
                        sr_[len - 1 - i] *= g;
                    }
                }
            }

            // Render the instrument, split at event frames.
            if let Some(inst) = t.instrument.as_mut() {
                // In-place (non-allocating) sort; note-offs before note-ons on the same frame.
                self.events.sort_unstable_by_key(|e| (e.frame, e.seq));
                let mut cursor = 0usize;
                for ev in self.events.iter() {
                    let fr = ev.frame.min(n);
                    if fr > cursor {
                        let c = ProcessCtx { beat: ctx.beat + cursor as f64 * beats_per_frame, ..ctx };
                        inst.process(&c, &mut tl[cursor..fr], &mut tr[cursor..fr]);
                        cursor = fr;
                    }
                    if ev.on {
                        inst.note_on(ev.pitch, ev.velocity);
                    } else {
                        inst.note_off(ev.pitch);
                    }
                }
                if cursor < n {
                    let c = ProcessCtx { beat: ctx.beat + cursor as f64 * beats_per_frame, ..ctx };
                    inst.process(&c, &mut tl[cursor..n], &mut tr[cursor..n]);
                }
            }

            t.run_effects(&ctx, tl, tr);

            let (gl, gr) = t.target_gains_at(any_solo, Some(ctx.beat.max(0.0)));
            t.gain_l.set(gl);
            t.gain_r.set(gr);
            let mut pl = 0f32;
            let mut pr = 0f32;
            for i in 0..n {
                let l = tl[i] * t.gain_l.next();
                let r = tr[i] * t.gain_r.next();
                tl[i] = l;
                tr[i] = r;
                pl = pl.max(l.abs());
                pr = pr.max(r.abs());
                self.bus_l[i] += l;
                self.bus_r[i] += r;
            }
            if (t.handle as usize) < self.shared.meters.len() {
                self.shared.meters[t.handle as usize].feed(pl, pr);
            }
            if self.shared.scope.listens_to(t.handle) {
                // Post-fader, like the meter (the loop above left the faded signal in tl/tr).
                self.shared.scope.write(&tl[..n], &tr[..n]);
            }
        }
        live.clear();
        self.live_pending = live;

        // Master chain.
        {
            let (bl, br) = (&mut self.bus_l[..n], &mut self.bus_r[..n]);
            for slot in self.master_fx.iter_mut() {
                if slot.enabled {
                    slot.effect.process(&ctx, bl, br);
                }
            }
            self.master_gain.set(db_to_gain(self.master_volume_db));
            for i in 0..n {
                let g = self.master_gain.next();
                bl[i] *= g;
                br[i] *= g;
            }
            if self.shared.scope.listens_to(shared::SCOPE_MASTER) {
                self.shared.scope.write(bl, br);
            }
        }

        // Metronome & preview are not affected by the master chain.
        if self.transport.playing {
            for s in 0..nseg {
                let (f0, len, b0) = segs[s];
                let force = !seq_active[s];
                self.metronome.render(
                    b0,
                    beats_per_frame,
                    self.transport.beats_per_bar,
                    force,
                    &mut self.bus_l[f0..f0 + len],
                    &mut self.bus_r[f0..f0 + len],
                );
            }
        }
        self.metronome.render_tail(&mut self.bus_l[..n], &mut self.bus_r[..n]);

        if let Some(p) = self.preview.as_mut() {
            let step = p.buffer.sample_rate as f64 / self.sr as f64;
            let frames = p.buffer.frames() as f64;
            let stereo = p.buffer.channels.len() > 1;
            for i in 0..n {
                if p.pos >= frames {
                    break;
                }
                let l = p.buffer.read(0, p.pos) * 0.8;
                let r = if stereo { p.buffer.read(1, p.pos) * 0.8 } else { l };
                self.bus_l[i] += l;
                self.bus_r[i] += r;
                p.pos += step;
            }
            if p.pos >= frames {
                let done = self.preview.take().unwrap();
                self.trash(Garbage::Buffer(done.buffer));
            }
        }

        let mut ml = 0f32;
        let mut mr = 0f32;
        for i in 0..n {
            ml = ml.max(self.bus_l[i].abs());
            mr = mr.max(self.bus_r[i].abs());
        }
        self.shared.master.feed(ml, mr);
        self.shared.position.store(self.transport.pos);
    }
}
