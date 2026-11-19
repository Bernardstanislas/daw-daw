//! Sample-based instruments built on the embedded sample set (`sample_data.rs`):
//! - `Sampler`: chromatic multi-sound instrument (one looped sample per sound).
//! - `SampleKit`: one-shot drum kit (TightKit), GM mapped.

use std::sync::{Arc, OnceLock};

use super::sample_data::{DRUM_NOTE_NAMES, DRUMS, INSTRUMENT_NAMES, INSTRUMENT_PRESETS, INSTRUMENTS};
use super::util::{Adsr, AdsrStage};
use super::*;

/// Native rate of every embedded sample.
const SAMPLE_RATE: f64 = 48_000.0;

/// Decoded sample data, shared by every instance (decoded once, off the audio thread).
struct Bank {
    instruments: Vec<Arc<[f32]>>,
    drums: Vec<Arc<[f32]>>,
}

fn decode(wav: &[u8]) -> Arc<[f32]> {
    let reader = hound::WavReader::new(std::io::Cursor::new(wav)).expect("embedded WAV");
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;
    let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader.into_samples::<i32>().map(|s| s.unwrap_or(0) as f32 * scale).collect(),
        hound::SampleFormat::Float => reader.into_samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
    };
    // Mono mix-down (the set is mono already).
    raw.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect::<Vec<_>>().into()
}

fn bank() -> &'static Bank {
    static BANK: OnceLock<Bank> = OnceLock::new();
    BANK.get_or_init(|| Bank {
        instruments: INSTRUMENTS.iter().map(|s| decode(s.wav)).collect(),
        drums: DRUMS.iter().map(|s| decode(s.wav)).collect(),
    })
}

#[inline]
fn read(data: &[f32], pos: f64) -> f32 {
    let i = pos as usize;
    if i + 1 >= data.len() {
        return data.get(i).copied().unwrap_or(0.0);
    }
    let t = (pos - i as f64) as f32;
    data[i] + (data[i + 1] - data[i]) * t
}

// ===================================================================== Sampler

const P_SOUND: usize = 0;
const P_ATTACK: usize = 1;
const P_DECAY: usize = 2;
const P_SUSTAIN: usize = 3;
const P_RELEASE: usize = 4;
const P_TUNE: usize = 5;
const P_FINE: usize = 6;
const P_CUTOFF: usize = 7;
const P_VEL: usize = 8;
const P_LOOP: usize = 9;
const P_VOLUME: usize = 10;

const DEFAULT_SOUND: usize = 21; // Grand Piano

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "sampler",
    name: "Sampler",
    category: "Sampler",
    description: "Sampled instruments: pianos, strings, winds, guitars, organs and voices.",
    params: &[
        ParamSpec::choice("sound", "Sound", INSTRUMENT_NAMES, DEFAULT_SOUND, "Sound"),
        ParamSpec::log("attack", "Attack", 0.001, 5.0, 0.005, "s", "Envelope"),
        ParamSpec::log("decay", "Decay", 0.01, 10.0, 1.2, "s", "Envelope"),
        ParamSpec::lin("sustain", "Sustain", 0.0, 1.0, 0.55, "", "Envelope"),
        ParamSpec::log("release", "Release", 0.01, 10.0, 0.7, "s", "Envelope"),
        ParamSpec::int("tune", "Transpose", -24.0, 24.0, 0.0, "Pitch"),
        ParamSpec::lin("fine", "Fine", -100.0, 100.0, 0.0, "ct", "Pitch"),
        ParamSpec::log("cutoff", "Tone", 200.0, 20_000.0, 20_000.0, "Hz", "Tone"),
        ParamSpec::lin("vel_sens", "Velocity", 0.0, 100.0, 70.0, "%", "Tone"),
        ParamSpec::toggle("loop", "Sustain loop", true, "Tone"),
        ParamSpec::lin("volume", "Volume", -24.0, 12.0, 0.0, "dB", "Tone"),
    ],
    presets: INSTRUMENT_PRESETS,
    note_names: &[],
};

const VOICES: usize = 16;

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    note: u8,
    sound: usize,
    pos: f64,
    rate: f64,
    gain: f32,
    env: Adsr,
    age: u64,
}

pub struct Sampler {
    sr: f32,
    bank: &'static Bank,
    voices: [Voice; VOICES],
    sound: usize,
    adsr: [f32; 4],
    transpose: f32,
    vel_sens: f32,
    looping: bool,
    gain: Smoother,
    lp_coef: Smoother,
    lp_state: f32,
    clock: u64,
}

use super::util::Smoother;

impl Sampler {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut s = Self {
            sr,
            bank: bank(),
            voices: [Voice { active: false, note: 0, sound: 0, pos: 0.0, rate: 1.0, gain: 0.0, env: Adsr::new(sr), age: 0 }; VOICES],
            sound: DEFAULT_SOUND,
            adsr: [0.005, 1.2, 0.55, 0.7],
            transpose: 0.0,
            vel_sens: 0.7,
            looping: true,
            gain: Smoother::new(1.0, 20.0, sr),
            lp_coef: Smoother::new(1.0, 20.0, sr),
            lp_state: 0.0,
            clock: 0,
        };
        s.set_params(&SPEC.default_values());
        s.gain.snap(s.gain.target);
        s.lp_coef.snap(s.lp_coef.target);
        s
    }

    fn rate_for(&self, note: u8, sound: usize) -> f64 {
        let root = INSTRUMENTS[sound].root_midi;
        let semis = note as f32 - root + self.transpose;
        (2f64).powf(semis as f64 / 12.0) * SAMPLE_RATE / self.sr as f64
    }
}

impl Instrument for Sampler {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        self.clock += 1;
        // Retrigger the same note on its own voice; else a free voice; else steal the oldest.
        let idx = self
            .voices
            .iter()
            .position(|v| v.active && v.note == pitch && !v.env.is_releasing())
            .or_else(|| self.voices.iter().position(|v| !v.active))
            .unwrap_or_else(|| {
                (0..VOICES)
                    .min_by_key(|&i| (!self.voices[i].env.is_releasing() as u8, self.voices[i].age))
                    .unwrap_or(0)
            });
        let sound = self.sound.min(INSTRUMENTS.len() - 1);
        let rate = self.rate_for(pitch, sound);
        let vel = velocity.clamp(0.0, 1.0);
        let gain = (1.0 - self.vel_sens) + self.vel_sens * vel * vel;
        let [a, d, s, r] = self.adsr;
        let v = &mut self.voices[idx];
        v.active = true;
        v.note = pitch;
        v.sound = sound;
        v.pos = 0.0;
        v.rate = rate;
        v.gain = gain * INSTRUMENTS[sound].gain;
        v.env.set(a, d, s, r, self.sr);
        v.env.trigger();
        v.age = self.clock;
    }

    fn note_off(&mut self, pitch: u8) {
        for v in self.voices.iter_mut() {
            if v.active && v.note == pitch && !v.env.is_releasing() {
                v.env.gate_off();
            }
        }
    }

    fn all_notes_off(&mut self, hard: bool) {
        for v in self.voices.iter_mut() {
            if hard {
                v.env.kill();
            } else {
                v.env.gate_off();
            }
        }
    }

    fn set_params(&mut self, p: &ParamValues) {
        self.sound = (p[P_SOUND].round().max(0.0) as usize).min(INSTRUMENTS.len() - 1);
        self.adsr = [p[P_ATTACK], p[P_DECAY], p[P_SUSTAIN], p[P_RELEASE]];
        self.transpose = p[P_TUNE].round() + p[P_FINE] / 100.0;
        self.vel_sens = (p[P_VEL] / 100.0).clamp(0.0, 1.0);
        self.looping = p[P_LOOP] >= 0.5;
        self.gain.set(db_to_gain(p[P_VOLUME]) * 0.42);
        let fc = p[P_CUTOFF].clamp(20.0, self.sr * 0.45);
        self.lp_coef.set(1.0 - (-2.0 * std::f32::consts::PI * fc / self.sr).exp());
        // Live voices follow envelope edits (release time matters for held notes).
        let [a, d, s, r] = self.adsr;
        let sr = self.sr;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.env.set(a, d, s, r, sr);
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        if !self.voices.iter().any(|v| v.active) && self.lp_state.abs() < 1e-7 {
            // Keep smoothers moving so a later note starts at the right level.
            for _ in 0..n {
                self.gain.next();
                self.lp_coef.next();
            }
            return;
        }
        let looping = self.looping;
        for i in 0..n {
            let mut mix = 0f32;
            for v in self.voices.iter_mut() {
                if !v.active {
                    continue;
                }
                let info = &INSTRUMENTS[v.sound];
                let data = &self.bank.instruments[v.sound];
                let len = data.len();
                let (ls, le) = (info.loop_start.min(len), info.loop_end.min(len));
                let span = le.saturating_sub(ls);
                let loops = looping && span > 64;
                let xf = info.crossfade.min(ls).min(span / 2) as f64;
                let mut s = read(data, v.pos);
                if loops && xf > 1.0 && v.pos >= le as f64 - xf {
                    // Crossfade the loop end into the matching point before the loop start.
                    let t = ((v.pos - (le as f64 - xf)) / xf) as f32;
                    s = s * (1.0 - t) + read(data, v.pos - span as f64) * t;
                }
                let e = v.env.next();
                mix += s * e * v.gain;
                v.pos += v.rate;
                if loops {
                    if v.pos >= le as f64 {
                        v.pos -= span as f64;
                    }
                } else if v.pos >= (len - 1) as f64 {
                    v.active = false;
                    v.env.reset();
                    continue;
                }
                if v.env.stage() == AdsrStage::Idle {
                    v.active = false;
                }
            }
            let a = self.lp_coef.next();
            self.lp_state += a * (mix - self.lp_state);
            if self.lp_state.abs() < 1e-20 {
                self.lp_state = 0.0;
            }
            let out = self.lp_state * self.gain.next();
            l[i] += out;
            r[i] += out;
        }
    }
}

// ===================================================================== Sample kit

const K_TUNE: usize = 0;
const K_DECAY: usize = 1;
const K_CHOKE: usize = 2;
const K_DRIVE: usize = 3;
const K_LEVEL: usize = 4;

pub static KIT_SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "samplekit",
    name: "Sample Kit",
    category: "Drums",
    description: "TightKit: a sampled drum kit (kick, snare, clap, hats, tom, conga, shaker).",
    params: &[
        ParamSpec::lin("tune", "Tune", -12.0, 12.0, 0.34, "st", "Kit"),
        ParamSpec::lin("decay", "Decay", 5.0, 100.0, 100.0, "%", "Kit"),
        ParamSpec::toggle("choke", "Hat choke", true, "Kit"),
        ParamSpec::lin("drive", "Drive", 0.0, 100.0, 0.0, "%", "Kit"),
        ParamSpec::lin("level", "Level", -24.0, 12.0, 0.0, "dB", "Kit"),
    ],
    presets: &[
        Preset { name: "TightKit", values: &[] },
        Preset { name: "TightKit Short", values: &[("decay", 35.0)] },
        Preset { name: "TightKit Low", values: &[("tune", -3.0), ("drive", 25.0)] },
        Preset { name: "TightKit Crunch", values: &[("drive", 70.0), ("level", -4.0)] },
    ],
    note_names: DRUM_NOTE_NAMES,
};

/// GM notes that have no sample of their own fall back to a close piece.
fn kit_note(pitch: u8) -> u8 {
    match pitch {
        35 => 36,
        37 | 40 => 38,
        44 => 42,
        43 | 45 | 47 => 41,
        50 | 60..=64 => 48,
        49 | 51 | 52 | 55 | 57 | 59 => 46,
        54 | 56 | 58 | 69 | 75 | 82 => 70,
        p => p,
    }
}

const KIT_VOICES: usize = 24;

#[derive(Clone, Copy)]
struct Hit {
    active: bool,
    drum: usize,
    pos: f64,
    gain: f32,
    /// Per-sample multiplier of `fade` (1.0 = no decay shortening).
    fade_coef: f32,
    fade: f32,
    choked: bool,
}

pub struct SampleKit {
    sr: f32,
    bank: &'static Bank,
    hits: [Hit; KIT_VOICES],
    rate: f64,
    decay: f32,
    choke: bool,
    drive: f32,
    level: Smoother,
    choke_coef: f32,
}

impl SampleKit {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut k = Self {
            sr,
            bank: bank(),
            hits: [Hit { active: false, drum: 0, pos: 0.0, gain: 0.0, fade_coef: 1.0, fade: 1.0, choked: false }; KIT_VOICES],
            rate: 1.0,
            decay: 1.0,
            choke: true,
            drive: 0.0,
            level: Smoother::new(1.0, 20.0, sr),
            choke_coef: super::util::decay_coef(0.02, sr),
        };
        k.set_params(&KIT_SPEC.default_values());
        k.level.snap(k.level.target);
        k
    }
}

impl Instrument for SampleKit {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let note = kit_note(pitch);
        let Some(drum) = DRUMS.iter().position(|d| d.note == note) else { return };
        if self.choke && (note == 42) {
            if let Some(open) = DRUMS.iter().position(|d| d.note == 46) {
                for h in self.hits.iter_mut().filter(|h| h.active && h.drum == open) {
                    h.choked = true;
                }
            }
        }
        let idx = self.hits.iter().position(|h| !h.active).unwrap_or_else(|| {
            // Steal the quietest hit.
            (0..KIT_VOICES)
                .min_by(|&a, &b| self.hits[a].fade.partial_cmp(&self.hits[b].fade).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(0)
        });
        let len = self.bank.drums[drum].len() as f32;
        // decay < 100 %: fade to -60 dB over that fraction of the sample.
        let fade_coef = if self.decay >= 0.999 { 1.0 } else { super::util::decay_coef((len / 48_000.0) * self.decay, self.sr) };
        let vel = velocity.clamp(0.0, 1.0);
        self.hits[idx] = Hit {
            active: true,
            drum,
            pos: 0.0,
            gain: 0.3 + 0.7 * vel,
            fade_coef,
            fade: 1.0,
            choked: false,
        };
    }

    fn note_off(&mut self, _pitch: u8) {}

    fn all_notes_off(&mut self, hard: bool) {
        if hard {
            for h in self.hits.iter_mut() {
                h.choked = true;
            }
        }
    }

    fn set_params(&mut self, p: &ParamValues) {
        self.rate = (2f64).powf(p[K_TUNE] as f64 / 12.0) * SAMPLE_RATE / self.sr as f64;
        self.decay = (p[K_DECAY] / 100.0).clamp(0.05, 1.0);
        self.choke = p[K_CHOKE] >= 0.5;
        self.drive = (p[K_DRIVE] / 100.0).clamp(0.0, 1.0);
        self.level.set(db_to_gain(p[K_LEVEL]) * 0.6);
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let rate = self.rate;
        let drive = 1.0 + self.drive * 6.0;
        let comp = 1.0 / (1.0 + self.drive * 1.5);
        for i in 0..l.len() {
            let mut mix = 0f32;
            for h in self.hits.iter_mut() {
                if !h.active {
                    continue;
                }
                let data = &self.bank.drums[h.drum];
                if h.pos >= (data.len() - 1) as f64 || h.fade < 1e-4 {
                    h.active = false;
                    continue;
                }
                mix += read(data, h.pos) * h.gain * h.fade;
                h.pos += rate;
                h.fade *= if h.choked { self.choke_coef } else { h.fade_coef };
            }
            let g = self.level.next();
            let out = if self.drive > 0.0 { super::util::soft_clip(mix * drive) * comp } else { mix } * g;
            l[i] += out;
            r[i] += out;
        }
    }
}

// ===================================================================== tests

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(sr: f32) -> ProcessCtx {
        ProcessCtx { sample_rate: sr, bpm: 120.0, playing: true, beat: 0.0 }
    }

    fn render(inst: &mut dyn Instrument, sr: f32, frames: usize) -> Vec<f32> {
        let mut l = vec![0f32; frames];
        let mut r = vec![0f32; frames];
        for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            inst.process(&ctx(sr), cl, cr);
        }
        l
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    /// Autocorrelation pitch estimate (first strong peak in 80..2000 Hz).
    fn estimate_hz(x: &[f32], sr: f32) -> f32 {
        let n = x.len().min(8192);
        let x = &x[..n];
        let lag_min = (sr / 2000.0) as usize;
        let lag_max = (sr / 80.0) as usize;
        let w = n - lag_max;
        let acf: Vec<f32> = (0..=lag_max).map(|t| (0..w).map(|i| x[i] * x[i + t]).sum()).collect();
        let best = (lag_min..lag_max).map(|t| acf[t]).fold(f32::MIN, f32::max);
        let mut lag = lag_min;
        for t in lag_min + 1..lag_max {
            if acf[t] > 0.85 * best && acf[t] >= acf[t - 1] && acf[t] >= acf[t + 1] {
                lag = t;
                break;
            }
        }
        sr / lag as f32
    }

    #[test]
    fn every_sound_plays_and_is_finite() {
        for (i, s) in INSTRUMENTS.iter().enumerate() {
            let mut smp = Sampler::new(48_000.0);
            let mut p = SPEC.default_values();
            p[P_SOUND] = i as f32;
            smp.set_params(&p);
            smp.note_on(60, 0.9);
            let out = render(&mut smp, 48_000.0, 24_000);
            assert!(out.iter().all(|v| v.is_finite()), "{}", s.name);
            let pk = peak(&out);
            assert!(pk > 0.02 && pk < 1.2, "{}: peak {pk}", s.name);
        }
    }

    #[test]
    fn root_note_plays_at_native_rate() {
        let mut smp = Sampler::new(48_000.0);
        for (i, s) in INSTRUMENTS.iter().enumerate() {
            smp.sound = i;
            let r = smp.rate_for(s.root_midi.round() as u8, i);
            assert!((r - 1.0).abs() < 0.03, "{}: rate {r}", s.name);
        }
        // Octave up doubles the rate; at 44.1 kHz the rate compensates.
        let mut smp = Sampler::new(44_100.0);
        smp.sound = DEFAULT_SOUND;
        let root = INSTRUMENTS[DEFAULT_SOUND].root_midi.round() as u8;
        let ratio = smp.rate_for(root + 12, DEFAULT_SOUND) / smp.rate_for(root, DEFAULT_SOUND);
        assert!((ratio - 2.0).abs() < 1e-9);
        assert!(smp.rate_for(root, DEFAULT_SOUND) > 1.05);
    }

    #[test]
    fn pitch_follows_the_keyboard() {
        // Tine EP is a clean tone: A4 (440 Hz) should come out near 440 Hz.
        let tine = INSTRUMENTS.iter().position(|s| s.id == "tine-ep").unwrap();
        let mut smp = Sampler::new(48_000.0);
        let mut p = SPEC.default_values();
        p[P_SOUND] = tine as f32;
                smp.set_params(&p);
        smp.note_on(69, 1.0);
        let out = render(&mut smp, 48_000.0, 48_000);
        let hz = estimate_hz(&out[9_600..], 48_000.0);
        assert!((hz - 440.0).abs() < 12.0, "measured {hz} Hz");
    }

    #[test]
    fn held_note_sustains_through_the_loop_and_releases() {
        let strings = INSTRUMENTS.iter().position(|s| s.id == "strings").unwrap();
        let mut smp = Sampler::new(48_000.0);
        let mut p = SPEC.default_values();
        p[P_SOUND] = strings as f32;
        p[P_ATTACK] = 0.01;
        p[P_SUSTAIN] = 1.0;
        p[P_RELEASE] = 0.2;
        smp.set_params(&p);
        smp.note_on(60, 1.0);
        // The strings sample is 1.4 s long: 4 s later the loop must still sound.
        let out = render(&mut smp, 48_000.0, 4 * 48_000);
        assert!(peak(&out[3 * 48_000..]) > 0.02, "loop keeps sounding");
        smp.note_off(60);
        let tail = render(&mut smp, 48_000.0, 48_000);
        assert!(peak(&tail[36_000..]) < 1e-3, "released");
        assert!(smp.voices.iter().all(|v| !v.active));
    }

    #[test]
    fn kit_maps_gm_notes_and_fallbacks() {
        let mut kit = SampleKit::new(48_000.0);
        for note in [36u8, 35, 38, 40, 42, 44, 46, 49, 54, 39, 41, 45, 48, 70] {
            kit.note_on(note, 1.0);
            let out = render(&mut kit, 48_000.0, 4_800);
            assert!(peak(&out) > 0.01, "note {note} sounds");
            kit.all_notes_off(true);
            render(&mut kit, 48_000.0, 48_000 * 2);
        }
        kit.note_on(30, 1.0);
        assert!(kit.hits.iter().all(|h| !h.active), "unmapped note is ignored");
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let mut kit = SampleKit::new(48_000.0);
        kit.note_on(46, 1.0);
        render(&mut kit, 48_000.0, 4_800);
        kit.note_on(42, 1.0);
        render(&mut kit, 48_000.0, 24_000);
        let open = DRUMS.iter().position(|d| d.note == 46).unwrap();
        assert!(kit.hits.iter().all(|h| !(h.active && h.drum == open)));
    }

    #[test]
    fn spec_is_consistent() {
        for spec in [&SPEC, &KIT_SPEC] {
            assert!(spec.params.len() <= MAX_PARAMS);
            for p in spec.params {
                assert!(p.default >= p.min && p.default <= p.max, "{}", p.id);
            }
            for pr in spec.presets {
                for (id, v) in pr.values {
                    let ps = spec.params.iter().find(|p| p.id == *id).unwrap_or_else(|| panic!("{} uses {id}", pr.name));
                    assert!(*v >= ps.min && *v <= ps.max, "{}: {id}={v}", pr.name);
                }
            }
        }
        assert_eq!(INSTRUMENT_NAMES.len(), INSTRUMENTS.len());
    }
}
