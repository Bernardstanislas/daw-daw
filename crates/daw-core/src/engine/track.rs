//! Audio-thread representation of a track.

use std::sync::Arc;

use super::commands::Handle;
use crate::assets::AudioBuffer;
use crate::dsp::util::{Smoother, pan_gains};
use crate::dsp::{Effect, Instrument, ProcessCtx, db_to_gain};

/// A note with absolute timeline positions (beats), clip-truncated and loop-expanded.
#[derive(Debug, Clone, Copy)]
pub struct NoteRt {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub velocity: f32,
}

#[derive(Debug, Clone)]
pub struct AudioClipRt {
    pub start: f64,
    pub end: f64,
    /// Offset into the source, seconds.
    pub offset_sec: f64,
    pub gain: f32,
    pub fade_in: f64,
    pub fade_out: f64,
    pub buffer: Arc<AudioBuffer>,
}

pub struct FxSlot {
    pub enabled: bool,
    pub effect: Box<dyn Effect>,
}

pub const MAX_ACTIVE: usize = 128;

/// Breakpoint envelope (beat, value), sorted by beat.
pub type Envelope = Box<Vec<(f64, f32)>>;

/// Linear interpolation of an envelope; None when empty.
#[inline]
pub fn envelope_at(env: &[(f64, f32)], beat: f64) -> Option<f32> {
    let first = env.first()?;
    if beat <= first.0 {
        return Some(first.1);
    }
    let i = env.partition_point(|p| p.0 <= beat);
    if i >= env.len() {
        return Some(env[env.len() - 1].1);
    }
    let (a, b) = (env[i - 1], env[i]);
    let t = ((beat - a.0) / (b.0 - a.0).max(1e-9)) as f32;
    Some(a.1 + (b.1 - a.1) * t)
}

pub struct TrackRt {
    pub handle: Handle,
    pub instrument: Option<Box<dyn Instrument>>,
    pub effects: Box<Vec<FxSlot>>,
    pub notes: Box<Vec<NoteRt>>,
    pub audio_clips: Box<Vec<AudioClipRt>>,
    pub auto_volume: Envelope,
    pub auto_pan: Envelope,
    pub volume_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub gain_l: Smoother,
    pub gain_r: Smoother,
    /// Notes started by the sequencer and still sounding: (pitch, end beat).
    pub active: [(u8, f64); MAX_ACTIVE],
    pub active_len: usize,
}

impl TrackRt {
    pub fn new(handle: Handle, sample_rate: f32) -> Self {
        Self {
            handle,
            instrument: None,
            effects: Box::new(Vec::with_capacity(16)),
            notes: Box::new(Vec::new()),
            audio_clips: Box::new(Vec::new()),
            auto_volume: Box::new(Vec::new()),
            auto_pan: Box::new(Vec::new()),
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            gain_l: Smoother::new(1.0, 15.0, sample_rate),
            gain_r: Smoother::new(1.0, 15.0, sample_rate),
            active: [(0, 0.0); MAX_ACTIVE],
            active_len: 0,
        }
    }

    /// Target L/R gains given the mixer state and whether any track is soloed.
    pub fn target_gains(&self, any_solo: bool) -> (f32, f32) {
        self.target_gains_at(any_solo, None)
    }

    /// Gains at a song position (automation applied when `beat` is given).
    pub fn target_gains_at(&self, any_solo: bool, beat: Option<f64>) -> (f32, f32) {
        if self.mute || (any_solo && !self.solo) {
            return (0.0, 0.0);
        }
        let vol = beat.and_then(|b| envelope_at(&self.auto_volume, b)).unwrap_or(self.volume_db);
        let pan = beat.and_then(|b| envelope_at(&self.auto_pan, b)).unwrap_or(self.pan);
        let g = db_to_gain(vol);
        let (pl, pr) = pan_gains(pan);
        (g * pl, g * pr)
    }

    /// Releases every note started by the sequencer.
    pub fn release_sequenced(&mut self) {
        if let Some(inst) = self.instrument.as_mut() {
            for i in 0..self.active_len {
                inst.note_off(self.active[i].0);
            }
        }
        self.active_len = 0;
    }

    /// Adds audio clip output for the beat range [b0, b1) mapped to `l/r[0..n]`.
    pub fn render_audio_clips(&self, b0: f64, b1: f64, bpm: f64, sr: f32, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        if n == 0 || self.audio_clips.is_empty() {
            return;
        }
        let sec_per_beat = 60.0 / bpm;
        let frames_per_beat = sec_per_beat * sr as f64;
        let edge = (0.003 / sec_per_beat).max(1e-9); // 3 ms anti-click fades, in beats
        for clip in self.audio_clips.iter() {
            if clip.end <= b0 || clip.start >= b1 {
                continue;
            }
            let buf = &clip.buffer;
            let f_start = (((clip.start - b0) * frames_per_beat).ceil().max(0.0) as usize).min(n);
            let f_end = (((clip.end - b0) * frames_per_beat).ceil().max(0.0) as usize).min(n);
            let stereo = buf.channels.len() > 1;
            let len_beats = clip.end - clip.start;
            for f in f_start..f_end {
                let beat = b0 + f as f64 / frames_per_beat;
                let rel = beat - clip.start;
                let src = (clip.offset_sec + rel * sec_per_beat) * buf.sample_rate as f64;
                if src < 0.0 {
                    continue;
                }
                let mut g = clip.gain;
                let fi = clip.fade_in.max(edge);
                if rel < fi {
                    g *= (rel / fi) as f32;
                }
                let fo = clip.fade_out.max(edge);
                let to_end = len_beats - rel;
                if to_end < fo {
                    g *= (to_end / fo).max(0.0) as f32;
                }
                let sl = buf.read(0, src);
                let sr_ = if stereo { buf.read(1, src) } else { sl };
                l[f] += sl * g;
                r[f] += sr_ * g;
            }
        }
    }

    pub fn run_effects(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        for slot in self.effects.iter_mut() {
            if slot.enabled {
                slot.effect.process(ctx, l, r);
            }
        }
    }
}
