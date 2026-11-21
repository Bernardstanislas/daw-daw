//! "Grand": a modelled acoustic piano (no samples).
//!
//! Each note is a bank of up to 32 inharmonic string partials
//! (`f_n = n·f0·√(1 + B·n²)`, B growing towards the treble), every partial
//! carried by two damped complex resonators standing for the unison
//! strings: one decays fast (the "prompt" sound), the other slowly (the
//! "aftersound"), slightly detuned against each other. That gives the
//! characteristic double decay and the slow beating of a real piano.
//!
//! The hammer sets the spectrum: harder (louder) strikes excite more
//! upper partials, the strike point cuts a comb notch around the 7th–8th
//! partial, and a short filtered-noise thump adds the felt-on-string
//! attack. Dampers stop the note on release (except in the top octaves,
//! which have none, as on a real grand) unless the sustain pedal is down.
//! Tuning is stretched like a concert tuner would do.

// Parallel per-partial arrays read clearest with index loops.
#![allow(clippy::needless_range_loop)]

use super::util::{Noise, Smoother, Svf, decay_coef, flush, pan_gains};
use super::*;
use std::f32::consts::PI;

const MODELS: &[&str] = &["Concert Grand", "Studio Upright", "Bright Pop", "Felt", "Honky-Tonk"];

const P_MODEL: usize = 0;
const P_TONE: usize = 1;
const P_HARD: usize = 2;
const P_DECAY: usize = 3;
const P_RELEASE: usize = 4;
const P_DETUNE: usize = 5;
const P_STRETCH: usize = 6;
const P_HAMMER: usize = 7;
const P_PEDAL: usize = 8;
const P_WIDTH: usize = 9;
const P_VEL: usize = 10;
const P_VOLUME: usize = 11;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "grand",
    name: "Grand",
    category: "Keys",
    description: "Modelled acoustic piano: inharmonic strings, hammer, dampers, sustain pedal and five piano models.",
    params: &[
        ParamSpec::choice("model", "Model", MODELS, 0, "Piano"),
        ParamSpec::lin("tone", "Tone", 0.0, 100.0, 50.0, "%", "Piano"),
        ParamSpec::lin("hardness", "Hammer Hardness", 0.0, 100.0, 55.0, "%", "Piano"),
        ParamSpec::lin("decay", "Sustain", 0.0, 100.0, 50.0, "%", "Piano"),
        ParamSpec::lin("release", "Damper", 0.0, 100.0, 50.0, "%", "Piano"),
        ParamSpec::lin("detune", "Unison", 0.0, 100.0, 50.0, "%", "Strings"),
        ParamSpec::lin("stretch", "Stretch", 0.0, 100.0, 50.0, "%", "Strings"),
        ParamSpec::lin("hammer", "Hammer Noise", 0.0, 100.0, 40.0, "%", "Strings"),
        ParamSpec::toggle("pedal", "Sustain Pedal", false, "Strings"),
        ParamSpec::lin("width", "Width", 0.0, 100.0, 60.0, "%", "Output"),
        ParamSpec::lin("vel_sens", "Dynamics", 0.0, 100.0, 75.0, "%", "Output"),
        ParamSpec::lin("volume", "Volume", -24.0, 6.0, 0.0, "dB", "Output"),
    ],
    presets: &[
        Preset { name: "Concert Grand", values: &[] },
        Preset {
            name: "Ballad Grand",
            values: &[("tone", 38.0), ("hardness", 40.0), ("decay", 65.0), ("pedal", 1.0), ("width", 75.0)],
        },
        Preset {
            name: "Studio Upright",
            values: &[("model", 1.0), ("tone", 55.0), ("hammer", 50.0), ("width", 45.0)],
        },
        Preset {
            name: "Pop Piano",
            values: &[("model", 2.0), ("tone", 70.0), ("hardness", 75.0), ("vel_sens", 55.0)],
        },
        Preset {
            name: "Felt Piano",
            values: &[("model", 3.0), ("tone", 35.0), ("hammer", 65.0), ("release", 70.0), ("width", 50.0)],
        },
        Preset {
            name: "Ambient Felt",
            values: &[("model", 3.0), ("tone", 25.0), ("decay", 85.0), ("pedal", 1.0), ("hammer", 45.0), ("width", 90.0)],
        },
        Preset {
            name: "Honky-Tonk Saloon",
            values: &[("model", 4.0), ("tone", 62.0), ("detune", 70.0), ("hammer", 50.0)],
        },
    ],
    note_names: &[],
};

/// Per-model character.
struct Model {
    /// Inharmonicity multiplier.
    b: f32,
    decay: f32,
    /// Brightness offset added to the tone (in "hardness" units).
    bright: f32,
    /// Unison detune at 50 % (cents).
    detune: f32,
    noise: f32,
    /// Extra low-pass on the partial spectrum (felt dampening), 1 = none.
    felt: f32,
}

const MODEL_DATA: [Model; 5] = [
    Model { b: 1.0, decay: 1.0, bright: 0.0, detune: 0.7, noise: 1.0, felt: 1.0 },
    Model { b: 1.7, decay: 0.6, bright: -0.08, detune: 1.3, noise: 1.3, felt: 1.0 },
    Model { b: 1.0, decay: 0.85, bright: 0.3, detune: 0.9, noise: 0.9, felt: 1.0 },
    Model { b: 0.8, decay: 0.65, bright: -0.45, detune: 0.5, noise: 1.8, felt: 0.35 },
    Model { b: 1.3, decay: 0.7, bright: 0.15, detune: 11.0, noise: 1.2, felt: 1.0 },
];

const NP: usize = 32;
const MAX_POLY: usize = 24;
const NUM_SLOTS: usize = 30;
const CHUNK: usize = 32;
const SILENCE: f32 = 2.0e-5;
const VOICE_GAIN: f32 = 0.3;
/// Notes at or above this pitch have no damper.
const NO_DAMPER: u8 = 89;

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    held: bool,
    /// Kept ringing by the pedal after its key was released.
    pedalled: bool,
    pitch: u8,
    age: u64,
    n: usize,
    // Two resonators per partial: (re, im) state and rotation (cr, ci).
    ar: [f32; NP],
    ai: [f32; NP],
    acr: [f32; NP],
    aci: [f32; NP],
    br: [f32; NP],
    bi: [f32; NP],
    bcr: [f32; NP],
    bci: [f32; NP],
    /// Output envelope (dampers / voice stealing).
    env: f32,
    env_coef: f32,
    damp_coef: f32,
    gl: f32,
    gr: f32,
    // Hammer thump.
    noise: Noise,
    nf: Svf,
    nenv: f32,
    ncoef: f32,
    nlvl: f32,
}

impl Voice {
    fn idle() -> Self {
        Self {
            active: false,
            held: false,
            pedalled: false,
            pitch: 0,
            age: 0,
            n: 0,
            ar: [0.0; NP],
            ai: [0.0; NP],
            acr: [0.0; NP],
            aci: [0.0; NP],
            br: [0.0; NP],
            bi: [0.0; NP],
            bcr: [0.0; NP],
            bci: [0.0; NP],
            env: 0.0,
            env_coef: 1.0,
            damp_coef: 0.0,
            gl: 1.0,
            gr: 1.0,
            noise: Noise::new(1),
            nf: Svf::default(),
            nenv: 0.0,
            ncoef: 0.0,
            nlvl: 0.0,
        }
    }

    fn render(&mut self, bl: &mut [f32], br: &mut [f32], mono: &mut [f32; CHUNK]) {
        let len = bl.len();
        let m = &mut mono[..len];
        m.fill(0.0);
        for p in 0..self.n {
            let (mut ar, mut ai, acr, aci) = (self.ar[p], self.ai[p], self.acr[p], self.aci[p]);
            let (mut brr, mut bi, bcr, bci) = (self.br[p], self.bi[p], self.bcr[p], self.bci[p]);
            for s in m.iter_mut() {
                let nar = ar * acr - ai * aci;
                ai = ar * aci + ai * acr;
                ar = nar;
                let nbr = brr * bcr - bi * bci;
                bi = brr * bci + bi * bcr;
                brr = nbr;
                *s += ai + bi;
            }
            self.ar[p] = flush(ar);
            self.ai[p] = flush(ai);
            self.br[p] = flush(brr);
            self.bi[p] = flush(bi);
        }
        for k in 0..len {
            let mut x = m[k];
            if self.nenv > SILENCE {
                x += self.nf.process(self.noise.next()).1 * self.nenv * self.nlvl;
                self.nenv *= self.ncoef;
            }
            let out = x * self.env;
            self.env *= self.env_coef;
            bl[k] += out * self.gl;
            br[k] += out * self.gr;
        }
        // Drop partials that have died out (upper ones die first).
        while self.n > 0 {
            let p = self.n - 1;
            let a = self.ar[p].abs() + self.ai[p].abs() + self.br[p].abs() + self.bi[p].abs();
            if a < SILENCE * 0.25 {
                self.n -= 1;
            } else {
                break;
            }
        }
        if (self.n == 0 || self.env < SILENCE) && self.nenv <= SILENCE {
            self.active = false;
        }
    }

    fn damp(&mut self) {
        self.held = false;
        self.pedalled = false;
        if self.pitch < NO_DAMPER {
            self.env_coef = self.env_coef.min(self.damp_coef);
        }
    }

    fn kill(&mut self, coef: f32) {
        self.held = false;
        self.pedalled = false;
        self.env_coef = self.env_coef.min(coef);
    }
}

pub struct Grand {
    sr: f32,
    params: ParamValues,
    voices: [Voice; NUM_SLOTS],
    counter: u64,
    seed: u32,
    pedal: bool,
    kill_coef: f32,
    mono: [f32; CHUNK],
    buf_l: [f32; CHUNK],
    buf_r: [f32; CHUNK],
    volume: Smoother,
}

impl Grand {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut g = Self {
            sr,
            params: SPEC.default_values(),
            voices: [Voice::idle(); NUM_SLOTS],
            counter: 0,
            seed: 0x1234_5678,
            pedal: false,
            kill_coef: decay_coef(0.04, sr),
            mono: [0.0; CHUNK],
            buf_l: [0.0; CHUNK],
            buf_r: [0.0; CHUNK],
            volume: Smoother::new(1.0, 30.0, sr),
        };
        let v = g.params;
        g.set_params(&v);
        g.volume.snap(g.volume.target);
        g
    }

    fn alloc(&mut self) -> usize {
        let kc = self.kill_coef;
        let sounding = self.voices.iter().filter(|v| v.active && v.env_coef > kc).count();
        if sounding >= MAX_POLY {
            // Fade the oldest note, released ones first.
            if let Some(v) = self
                .voices
                .iter_mut()
                .filter(|v| v.active && v.env_coef > kc)
                .min_by_key(|v| (v.held, v.age))
            {
                v.kill(kc);
            }
        }
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        let mut best = 0;
        for i in 1..NUM_SLOTS {
            if self.voices[i].env < self.voices[best].env {
                best = i;
            }
        }
        best
    }

    fn make_voice(&mut self, pitch: u8, vel: f32) -> Voice {
        let sr = self.sr;
        let p = &self.params;
        let model = &MODEL_DATA[(p[P_MODEL].round() as usize).min(MODEL_DATA.len() - 1)];
        let note = pitch as f32;
        let vs = p[P_VEL] / 100.0;
        let vel_gain = 1.0 - vs + vs * vel.powf(1.7);

        // Concert-style stretch: flat bass, sharp treble (±~25 cents).
        let rel = (note - 64.0) / 44.0;
        let stretch_ct = p[P_STRETCH] / 50.0 * 22.0 * rel * rel * rel;
        let f0 = midi_to_hz(note + stretch_ct / 100.0);

        let b = model.b * 0.00038 * 2f32.powf((note - 60.0) / 19.0);
        // Hammer: louder → harder felt → brighter spectrum.
        let hard = (p[P_HARD] / 100.0 * 0.6 + vel.powf(0.8) * 0.5 + (p[P_TONE] / 100.0 - 0.5) * 0.8 + model.bright)
            .clamp(0.0, 1.2);
        let slope = 2.5 - 1.6 * hard;
        let lp_hz = (900.0 + 9000.0 * hard * hard) * model.felt + f0 * 2.0;
        let strike = 1.0 / 7.3;

        // Decay of the fundamental (aftersound), seconds to -60 dB.
        let decay_mult = 2f32.powf((p[P_DECAY] - 50.0) / 50.0 * 1.3) * model.decay;
        let t0 = (14.0 * 2f32.powf(-(note - 60.0) / 15.0)).clamp(0.5, 28.0) * decay_mult;
        let det_ct = model.detune * p[P_DETUNE] / 50.0;

        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut v = Voice::idle();
        v.active = true;
        v.held = true;
        v.pitch = pitch;
        let max_f = (sr * 0.45).min(16000.0);
        let mut amps = [0.0f32; NP];
        let mut freqs = [0.0f32; NP];
        let mut sum = 0.0;
        let mut n = 0;
        for k in 0..NP {
            let h = (k + 1) as f32;
            let f = h * f0 * (1.0 + b * h * h).sqrt();
            if f >= max_f {
                break;
            }
            let comb = (PI * h * strike).sin().abs().max(0.08);
            let lp = 1.0 / (1.0 + (f / lp_hz).powi(2));
            let a = comb * lp / h.powf(slope);
            amps[k] = a;
            freqs[k] = f;
            sum += a;
            n = k + 1;
        }
        let norm = if sum > 0.0 { 1.0 / sum.powf(0.8) } else { 0.0 };
        v.n = n;
        for k in 0..n {
            let f = freqs[k];
            let h = (k + 1) as f32;
            // Upper partials lose energy faster.
            let t = t0 / (1.0 + f / 1800.0 + 0.04 * (h - 1.0));
            let ta = (t * 0.22).max(0.05);
            let tb = t;
            // Prompt (fast) and aftersound (slow) strings, detuned apart.
            let fa = f * 2f32.powf(-det_ct / 2400.0);
            let fb = f * 2f32.powf(det_ct / 2400.0);
            let ra = decay_coef(ta, sr);
            let rb = decay_coef(tb, sr);
            let (wa, wb) = (2.0 * PI * fa / sr, 2.0 * PI * fb / sr);
            let a = amps[k] * norm * vel_gain;
            v.ar[k] = a * 0.62;
            v.ai[k] = 0.0;
            v.acr[k] = ra * wa.cos();
            v.aci[k] = ra * wa.sin();
            v.br[k] = a * 0.38;
            v.bi[k] = 0.0;
            v.bcr[k] = rb * wb.cos();
            v.bci[k] = rb * wb.sin();
        }
        v.env = 1.0;
        v.env_coef = 1.0;
        // Damper: bass strings take longer to stop.
        let damp_t = (0.35 * 2f32.powf(-(note - 48.0) / 24.0)).clamp(0.06, 0.6)
            * 2f32.powf((p[P_RELEASE] - 50.0) / 50.0 * 1.5);
        v.damp_coef = decay_coef(damp_t, sr);
        // Player's perspective: bass on the left.
        let pan = ((note - 64.0) / 36.0).clamp(-1.0, 1.0) * p[P_WIDTH] / 100.0;
        let (gl, gr) = pan_gains(pan);
        v.gl = gl * VOICE_GAIN;
        v.gr = gr * VOICE_GAIN;
        v.noise = Noise::new(self.seed | 1);
        v.nf.set((f0 * 3.0).clamp(250.0, 4500.0), 0.35, sr);
        v.nenv = 1.0;
        v.ncoef = decay_coef(0.025 + 0.02 * (1.0 - hard.min(1.0)), sr);
        v.nlvl = p[P_HAMMER] / 100.0 * model.noise * 0.35 * vel.powf(1.5) * vel_gain.sqrt();
        v
    }
}

impl Instrument for Grand {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let vel = velocity.clamp(0.0, 1.0);
        if vel <= 0.0 {
            return;
        }
        // Re-striking a string: the previous strike fades under the new one.
        let kc = self.kill_coef;
        for v in self.voices.iter_mut().filter(|v| v.active && v.pitch == pitch) {
            v.kill(kc);
        }
        let i = self.alloc();
        self.counter += 1;
        let mut v = self.make_voice(pitch, vel);
        v.age = self.counter;
        self.voices[i] = v;
    }

    fn note_off(&mut self, pitch: u8) {
        let pedal = self.pedal;
        for v in self.voices.iter_mut().filter(|v| v.active && v.held && v.pitch == pitch) {
            if pedal {
                v.held = false;
                v.pedalled = true;
            } else {
                v.damp();
            }
        }
    }

    fn all_notes_off(&mut self, hard: bool) {
        let kc = self.kill_coef;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            if hard {
                v.kill(kc);
            } else {
                v.damp();
            }
        }
    }

    fn set_params(&mut self, values: &ParamValues) {
        self.params = *values;
        let pedal = values[P_PEDAL] >= 0.5;
        if self.pedal && !pedal {
            for v in self.voices.iter_mut().filter(|v| v.active && v.pedalled) {
                v.damp();
            }
        }
        self.pedal = pedal;
        self.volume.set(db_to_gain(values[P_VOLUME]));
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let mut i = 0;
        while i < n {
            let len = CHUNK.min(n - i);
            let (bl, br) = (&mut self.buf_l[..len], &mut self.buf_r[..len]);
            bl.fill(0.0);
            br.fill(0.0);
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.render(bl, br, &mut self.mono);
            }
            for k in 0..len {
                let g = self.volume.next();
                l[i + k] += bl[k] * g;
                r[i + k] += br[k] * g;
            }
            i += len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::util::test_support::*;

    #[test]
    fn spec_is_valid() {
        check_spec(&SPEC);
    }

    #[test]
    fn every_model_and_range_renders_sanely() {
        for &sr in &[44100.0, 48000.0] {
            for preset in SPEC.presets {
                for &(note, vel) in &[(21u8, 1.0f32), (48, 0.3), (60, 0.8), (84, 1.0), (108, 0.6)] {
                    let mut g = Grand::new(sr);
                    g.set_params(&preset_values(&SPEC, preset));
                    g.note_on(note, vel);
                    let (l, r) = render(&mut g, sr, (sr * 0.3) as usize, 128);
                    assert_finite(&l, preset.name);
                    assert_finite(&r, preset.name);
                    let pk = peak(&l).max(peak(&r));
                    assert!(pk > 0.003 && pk < 1.5, "{} note {note}: peak {pk}", preset.name);
                }
            }
        }
    }

    #[test]
    fn dampers_stop_the_note_unless_pedal() {
        let sr = 48000.0;
        let mut g = Grand::new(sr);
        g.note_on(48, 0.9);
        let _ = render(&mut g, sr, 4800, 256);
        g.note_off(48);
        let _ = render(&mut g, sr, 48000, 256);
        assert!(!g.voices.iter().any(|v| v.active), "damped note still ringing");

        let mut g = Grand::new(sr);
        g.set_params(&with(&SPEC, SPEC.default_values(), "pedal", 1.0));
        g.note_on(48, 0.9);
        g.note_off(48);
        let _ = render(&mut g, sr, 48000, 256);
        let (l, _) = render(&mut g, sr, 2048, 256);
        assert!(peak(&l) > 1.0e-3, "pedal should keep the string ringing");
        // Lifting the pedal damps it.
        g.set_params(&SPEC.default_values());
        let _ = render(&mut g, sr, 48000, 256);
        assert!(!g.voices.iter().any(|v| v.active));
    }

    #[test]
    fn louder_is_brighter() {
        // Zero crossings rise with velocity (more upper partials).
        let sr = 48000.0;
        let zc = |vel: f32| {
            let mut g = Grand::new(sr);
            g.set_params(&with(&SPEC, SPEC.default_values(), "hammer", 0.0));
            g.note_on(48, vel);
            let (l, _) = render(&mut g, sr, 9600, 256);
            l.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
        };
        assert!(zc(1.0) > zc(0.2));
    }

    #[test]
    fn polyphony_is_bounded() {
        let sr = 48000.0;
        let mut g = Grand::new(sr);
        g.set_params(&with(&SPEC, SPEC.default_values(), "pedal", 1.0));
        for k in 0..60u8 {
            g.note_on(30 + k, 0.8);
            g.note_off(30 + k);
            let _ = render(&mut g, sr, 300, 128);
        }
        let (l, _) = render(&mut g, sr, 4800, 128);
        assert_finite(&l, "flood");
        assert!(peak(&l) < 2.0);
    }
}
