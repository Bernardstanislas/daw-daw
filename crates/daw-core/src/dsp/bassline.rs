//! "Bassline": a monophonic bass synth in the spirit of the famous little
//! silver acid box, extended for modern basses.
//!
//! One saw / square oscillator (+ square sub-octave) into a resonant
//! ladder filter (rounder 18 dB "acid" response or full 24 dB), a decaying
//! filter envelope, accent and slide:
//! - **Slide**: a note played while another is still held glides to the new
//!   pitch without retriggering the envelopes (tie notes in the piano roll).
//! - **Accent**: notes with velocity ≥ ~0.75 are accented: louder, shorter
//!   and deeper filter sweep, plus the "accent sweep" capacitor that charges
//!   over consecutive accents and pushes the cutoff further up — the
//!   characteristic acid "wow".
//!
//! A post-filter drive stage covers the classic distorted acid sound.

use super::util::{Ladder, Smoother, decay_coef, fast_pow2, flush, poly_blep, soft_clip};
use super::*;

const WAVES: &[&str] = &["Saw", "Square"];
const FILTERS: &[&str] = &["Acid 18 dB", "Ladder 24 dB"];
const CONTROL: usize = 8;
const ACCENT_VEL: f32 = 0.75;

const P_WAVE: usize = 0;
const P_OCTAVE: usize = 1;
const P_TUNE: usize = 2;
const P_SUB: usize = 3;
const P_FILTER: usize = 4;
const P_CUTOFF: usize = 5;
const P_RES: usize = 6;
const P_ENV_MOD: usize = 7;
const P_DECAY: usize = 8;
const P_ACCENT: usize = 9;
const P_AMP_DECAY: usize = 10;
const P_SLIDE: usize = 11;
const P_DRIVE: usize = 12;
const P_VOLUME: usize = 13;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "bassline",
    name: "Bassline",
    category: "Bass",
    description: "Mono acid bass: saw/square, squelchy resonant filter, accent (velocity ≥ 96) and slide (overlapping notes).",
    params: &[
        ParamSpec::choice("wave", "Wave", WAVES, 0, "Oscillator"),
        ParamSpec::int("octave", "Octave", -2.0, 1.0, 0.0, "Oscillator"),
        ParamSpec::lin("tune", "Tune", -12.0, 12.0, 0.0, "st", "Oscillator"),
        ParamSpec::lin("sub", "Sub", 0.0, 100.0, 0.0, "%", "Oscillator"),
        ParamSpec::choice("filter", "Filter", FILTERS, 0, "Filter"),
        ParamSpec::log("cutoff", "Cutoff", 40.0, 8000.0, 420.0, "Hz", "Filter"),
        ParamSpec::lin("res", "Resonance", 0.0, 100.0, 65.0, "%", "Filter"),
        ParamSpec::lin("env_mod", "Env Mod", 0.0, 100.0, 55.0, "%", "Filter"),
        ParamSpec::log("decay", "Decay", 0.05, 2.5, 0.35, "s", "Filter"),
        ParamSpec::lin("accent", "Accent", 0.0, 100.0, 60.0, "%", "Filter"),
        ParamSpec::log("amp_decay", "Amp Decay", 0.05, 8.0, 2.0, "s", "Amp"),
        ParamSpec::log("slide", "Slide", 0.01, 0.5, 0.06, "s", "Amp"),
        ParamSpec::lin("drive", "Drive", 0.0, 100.0, 15.0, "%", "Amp"),
        ParamSpec::lin("volume", "Volume", -24.0, 6.0, 0.0, "dB", "Amp"),
    ],
    presets: &[
        Preset { name: "Acid Classic", values: &[] },
        Preset {
            name: "Squelch Lead",
            values: &[("cutoff", 300.0), ("res", 88.0), ("env_mod", 80.0), ("decay", 0.5), ("accent", 85.0), ("drive", 45.0)],
        },
        Preset {
            name: "Distorted Acid",
            values: &[("cutoff", 520.0), ("res", 75.0), ("env_mod", 70.0), ("decay", 0.25), ("accent", 75.0), ("drive", 90.0), ("volume", -5.0)],
        },
        Preset {
            name: "Square Acid",
            values: &[("wave", 1.0), ("cutoff", 380.0), ("res", 70.0), ("env_mod", 60.0), ("decay", 0.3)],
        },
        Preset {
            name: "Rubber Bass",
            values: &[("filter", 1.0), ("sub", 50.0), ("cutoff", 220.0), ("res", 35.0), ("env_mod", 45.0), ("decay", 0.22), ("accent", 30.0), ("drive", 10.0)],
        },
        Preset {
            name: "Deep Sub",
            values: &[("wave", 1.0), ("filter", 1.0), ("octave", -1.0), ("sub", 100.0), ("cutoff", 140.0), ("res", 10.0), ("env_mod", 10.0), ("accent", 15.0), ("drive", 20.0)],
        },
        Preset {
            name: "Funk Pluck",
            values: &[("filter", 1.0), ("sub", 30.0), ("cutoff", 160.0), ("res", 45.0), ("env_mod", 75.0), ("decay", 0.14), ("amp_decay", 0.45), ("slide", 0.03)],
        },
    ],
    note_names: &[],
};

pub struct Bassline {
    sr: f32,
    params: ParamValues,
    clock: usize,
    // Note stack (last-note priority).
    stack: [u8; 16],
    stack_len: usize,
    gate: bool,
    // Oscillator.
    pitch: f32,
    target: f32,
    slide_coef: f32,
    ph: f32,
    sub_ph: f32,
    inc: f32,
    // Envelopes.
    fenv: f32,
    fenv_coef: f32,
    amp: f32,
    amp_target: f32,
    amp_attack: f32,
    amp_decay: f32,
    amp_release: f32,
    accented: bool,
    acc_cap: f32,
    acc_charge: f32,
    acc_coef: f32,
    filter: Ladder,
    volume: Smoother,
}

impl Bassline {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut b = Self {
            sr,
            params: SPEC.default_values(),
            clock: 0,
            stack: [0; 16],
            stack_len: 0,
            gate: false,
            pitch: 36.0,
            target: 36.0,
            slide_coef: 0.0,
            ph: 0.0,
            sub_ph: 0.0,
            inc: 0.0,
            fenv: 0.0,
            fenv_coef: 0.0,
            amp: 0.0,
            amp_target: 0.0,
            amp_attack: 1.0 - (-1.0 / (0.002 * sr)).exp(),
            amp_decay: 1.0,
            amp_release: decay_coef(0.012, sr),
            accented: false,
            acc_cap: 0.0,
            acc_charge: 1.0 - (-1.0 / (0.012 * sr)).exp(),
            acc_coef: decay_coef(0.9, sr),
            filter: Ladder::default(),
            volume: Smoother::new(1.0, 30.0, sr),
        };
        let v = b.params;
        b.set_params(&v);
        b.volume.snap(b.volume.target);
        b
    }

    fn base_note(&self, pitch: u8) -> f32 {
        pitch as f32 + 12.0 * self.params[P_OCTAVE].round() + self.params[P_TUNE]
    }

    fn trigger(&mut self, pitch: u8, vel: f32, slide: bool) {
        self.target = self.base_note(pitch);
        if slide {
            return;
        }
        self.pitch = self.target;
        self.accented = vel >= ACCENT_VEL;
        let p = &self.params;
        let decay = if self.accented { p[P_DECAY].min(0.2) } else { p[P_DECAY] };
        self.fenv = 1.0;
        self.fenv_coef = decay_coef(decay, self.sr);
        let accent = p[P_ACCENT] / 100.0;
        let level = 0.55 + 0.25 * vel;
        self.amp_target = if self.accented { level * (1.0 + 0.6 * accent) } else { level };
        if !self.gate {
            self.filter.reset();
        }
        self.gate = true;
    }

    fn control(&mut self) {
        let p = &self.params;
        let sr = self.sr;
        self.pitch = self.target + (self.pitch - self.target) * self.slide_coef;
        let f = 440.0 * fast_pow2((self.pitch - 69.0) / 12.0);
        self.inc = (f / sr).min(0.45);
        let res = p[P_RES] / 100.0;
        let env_mod = p[P_ENV_MOD] / 100.0;
        let accent = p[P_ACCENT] / 100.0;
        let mut oct = (p[P_CUTOFF] / 1000.0).log2() + self.fenv * env_mod * 4.5;
        if self.accented {
            oct += self.fenv * accent * 1.5;
        }
        oct += self.acc_cap * accent * (1.2 + res * 1.6);
        let fc = 1000.0 * fast_pow2(oct.clamp(-6.0, 4.3));
        self.filter.set(fc, res * 0.96, sr);
    }
}

impl Instrument for Bassline {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let vel = velocity.clamp(0.0, 1.0);
        let slide = self.gate && self.stack_len > 0;
        if let Some(pos) = self.stack[..self.stack_len].iter().position(|&n| n == pitch) {
            self.stack.copy_within(pos + 1..self.stack_len, pos);
            self.stack_len -= 1;
        }
        if self.stack_len == self.stack.len() {
            self.stack.copy_within(1.., 0);
            self.stack_len -= 1;
        }
        self.stack[self.stack_len] = pitch;
        self.stack_len += 1;
        self.trigger(pitch, vel, slide);
    }

    fn note_off(&mut self, pitch: u8) {
        let Some(pos) = self.stack[..self.stack_len].iter().position(|&n| n == pitch) else {
            return;
        };
        let was_top = pos + 1 == self.stack_len;
        self.stack.copy_within(pos + 1..self.stack_len, pos);
        self.stack_len -= 1;
        if !was_top {
            return;
        }
        if self.stack_len > 0 {
            // Fall back to the previous held note, sliding.
            let prev = self.stack[self.stack_len - 1];
            self.target = self.base_note(prev);
        } else {
            self.gate = false;
        }
    }

    fn all_notes_off(&mut self, hard: bool) {
        self.stack_len = 0;
        self.gate = false;
        if hard {
            self.amp = 0.0;
            self.filter.reset();
        }
    }

    fn set_params(&mut self, values: &ParamValues) {
        self.params = *values;
        let sr = self.sr;
        self.slide_coef = (-(CONTROL as f32) * 4.6 / (values[P_SLIDE] * sr)).exp();
        self.amp_decay = decay_coef(values[P_AMP_DECAY], sr);
        self.volume.set(db_to_gain(values[P_VOLUME]));
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let p = self.params;
        let square = p[P_WAVE].round() >= 1.0;
        let sub = p[P_SUB] / 100.0;
        let poles = if p[P_FILTER].round() >= 1.0 { 4 } else { 3 };
        let drive = p[P_DRIVE] / 100.0;
        let pre = 1.0 + drive * drive * 9.0;
        let post = 1.0 / (1.0 + drive * 0.6);
        for i in 0..n {
            if self.clock == 0 {
                self.control();
            }
            self.clock = (self.clock + 1) % CONTROL;
            // Envelopes.
            self.fenv *= self.fenv_coef;
            if self.gate {
                // Fast attack chasing a slowly decaying target.
                self.amp_target *= self.amp_decay;
                self.amp += (self.amp_target - self.amp) * self.amp_attack;
            } else {
                self.amp = flush(self.amp * self.amp_release);
            }
            // Accent sweep capacitor: charges on accented notes, leaks slowly.
            if self.gate && self.accented && self.fenv > 0.3 {
                self.acc_cap += (1.0 - self.acc_cap) * self.acc_charge * 0.35;
            } else {
                self.acc_cap = flush(self.acc_cap * self.acc_coef);
            }
            if self.amp < 1.0e-5 && !self.gate {
                self.amp = 0.0;
                continue;
            }
            // Oscillators.
            let dt = self.inc;
            self.ph += dt;
            if self.ph >= 1.0 {
                self.ph -= 1.0;
            }
            self.sub_ph += dt * 0.5;
            if self.sub_ph >= 1.0 {
                self.sub_ph -= 1.0;
            }
            let t = self.ph;
            let mut x = if square {
                let mut t2 = t + 0.5;
                if t2 >= 1.0 {
                    t2 -= 1.0;
                }
                (if t < 0.5 { 1.0 } else { -1.0 }) + poly_blep(t, dt) - poly_blep(t2, dt)
            } else {
                2.0 * t - 1.0 - poly_blep(t, dt)
            };
            if sub > 0.0 {
                let sd = dt * 0.5;
                let st = self.sub_ph;
                let mut st2 = st + 0.5;
                if st2 >= 1.0 {
                    st2 -= 1.0;
                }
                x += ((if st < 0.5 { 1.0 } else { -1.0 }) + poly_blep(st, sd) - poly_blep(st2, sd)) * sub;
            }
            let y = self.filter.process(x * 0.5, poles, 0.0);
            let out = soft_clip(y * pre * 1.4) * post * self.amp * 0.8;
            let g = self.volume.next();
            l[i] += out * g;
            r[i] += out * g;
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
    fn every_preset_plays_and_stops() {
        for &sr in &[44100.0, 48000.0] {
            for preset in SPEC.presets {
                let mut b = Bassline::new(sr);
                b.set_params(&preset_values(&SPEC, preset));
                b.note_on(36, 1.0);
                let (l, r) = render(&mut b, sr, (sr * 0.3) as usize, 128);
                assert_finite(&l, preset.name);
                assert_finite(&r, preset.name);
                let pk = peak(&l);
                assert!(pk > 0.02 && pk < 1.5, "{}: peak {pk}", preset.name);
                b.note_off(36);
                let _ = render(&mut b, sr, 4800, 128);
                let (l, _) = render(&mut b, sr, 512, 128);
                assert!(peak(&l) < 1.0e-3, "{}: does not stop", preset.name);
            }
        }
    }

    #[test]
    fn overlapping_notes_slide_without_retrigger() {
        let sr = 48000.0;
        let mut b = Bassline::new(sr);
        b.note_on(36, 0.5);
        let _ = render(&mut b, sr, 4800, 64);
        let fenv = b.fenv;
        b.note_on(48, 0.5);
        assert_eq!(b.fenv, fenv, "slide must not retrigger the filter envelope");
        let _ = render(&mut b, sr, 2400, 64);
        assert!((b.pitch - 48.0).abs() < 0.5);
        // Releasing the slid-to note returns to the held one.
        b.note_off(48);
        let _ = render(&mut b, sr, 4800, 64);
        assert!((b.pitch - 36.0).abs() < 0.1);
        assert!(b.gate);
    }

    #[test]
    fn accent_opens_the_filter_more() {
        let sr = 48000.0;
        let energy = |vel: f32| {
            let mut b = Bassline::new(sr);
            b.note_on(36, vel);
            let (l, _) = render(&mut b, sr, 4800, 64);
            l.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
        };
        assert!(energy(1.0) > energy(0.5) * 1.2);
    }

    #[test]
    fn block_size_does_not_change_output() {
        let mk = || {
            let mut b = Bassline::new(48000.0);
            b.note_on(40, 1.0);
            b
        };
        let (a, _) = render(&mut mk(), 48000.0, 3000, 64);
        let (c, _) = render(&mut mk(), 48000.0, 3000, 29);
        for (x, y) in a.iter().zip(c.iter()) {
            assert!((x - y).abs() < 1e-6);
        }
    }
}
