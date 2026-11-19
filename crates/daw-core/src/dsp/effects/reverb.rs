//! "Space": an 8-line feedback delay network reverb.
//!
//! Signal flow: pre-delay -> 4 series allpass diffusers per channel -> FDN
//! (8 delay lines mixed through a normalized Hadamard matrix, each with a
//! damping low-pass and an RT60-derived gain) -> low cut -> width -> mix.
//! Two lines are slowly modulated to soften metallic ringing. Size changes
//! glide the line lengths so tails are never cut.

use super::fast_db_to_gain;
use crate::dsp::util::{DelayLine, OnePole, Smoother, fast_sin01};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};

const P_SIZE: usize = 0;
const P_DECAY: usize = 1;
const P_DAMPING: usize = 2;
const P_PREDELAY: usize = 3;
const P_WIDTH: usize = 4;
const P_LOWCUT: usize = 5;
const P_MIX: usize = 6;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "reverb",
    name: "Space",
    category: "Reverb",
    description: "Lush algorithmic reverb, from small rooms to endless washes.",
    params: &[
        ParamSpec::lin("size", "Size", 0.0, 100.0, 50.0, "%", "Room"),
        ParamSpec::log("decay", "Decay", 0.2, 20.0, 2.0, "s", "Room"),
        ParamSpec::lin("damping", "Damping", 0.0, 100.0, 50.0, "%", "Room"),
        ParamSpec::lin("predelay", "Pre-Delay", 0.0, 250.0, 10.0, "ms", "Room"),
        ParamSpec::lin("width", "Width", 0.0, 100.0, 100.0, "%", "Output"),
        ParamSpec::log("lowcut", "Low Cut", 20.0, 1000.0, 100.0, "Hz", "Output"),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 25.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Small Room",
            values: &[
                ("size", 18.0),
                ("decay", 0.6),
                ("damping", 60.0),
                ("predelay", 3.0),
                ("mix", 20.0),
            ],
        },
        Preset {
            name: "Vocal Plate",
            values: &[
                ("size", 45.0),
                ("decay", 1.8),
                ("damping", 25.0),
                ("predelay", 25.0),
                ("lowcut", 200.0),
                ("mix", 22.0),
            ],
        },
        Preset {
            name: "Big Hall",
            values: &[
                ("size", 85.0),
                ("decay", 4.5),
                ("damping", 45.0),
                ("predelay", 30.0),
                ("mix", 28.0),
            ],
        },
        Preset {
            name: "Endless",
            values: &[
                ("size", 100.0),
                ("decay", 20.0),
                ("damping", 35.0),
                ("predelay", 60.0),
                ("lowcut", 150.0),
                ("mix", 35.0),
            ],
        },
    ],
    note_names: &[],
};

const N: usize = 8;
/// FDN line lengths in ms at size = 100% (mutually prime-ish).
const LINE_MS: [f32; N] = [37.1, 41.9, 47.3, 53.9, 59.3, 67.7, 73.1, 83.3];
/// Allpass diffuser lengths in ms (per channel, slightly different L/R).
const AP_MS: [[f32; 4]; 2] = [[4.77, 3.59, 12.73, 9.31], [4.53, 3.83, 12.29, 9.87]];
const AP_GAIN: f32 = 0.62;
const MAX_PREDELAY_MS: f32 = 250.0;
/// Modulation depth (samples at 48 kHz) of the modulated lines.
const MOD_DEPTH_MS: f32 = 0.35;
/// Recompute line gains every `CONTROL` samples while size/decay glide.
const CONTROL: u32 = 32;
/// Makeup gain of the wet path (≈ +5 dB): wet ≈ dry level at the default decay.
const WET_MAKEUP: f32 = 1.8;

/// Schroeder allpass with a fixed integer delay.
struct Allpass {
    line: DelayLine,
    delay: usize,
}

impl Allpass {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let d = self.line.tap(self.delay - 1);
        let v = x + AP_GAIN * d;
        self.line.push(v);
        d - AP_GAIN * v
    }
}

/// Size (0..1) -> line length scale.
#[inline]
fn scale(size: f32) -> f32 {
    0.25 + 0.75 * size
}

pub struct Space {
    sr: f32,
    primed: bool,
    pre_l: DelayLine,
    pre_r: DelayLine,
    ap: [[Allpass; 4]; 2],
    lines: [DelayLine; N],
    damp: [OnePole; N],
    gains: [f32; N],
    /// Current (smoothed) line lengths in samples.
    lengths: [f32; N],
    size: Smoother,
    decay: Smoother,
    predelay: Smoother,
    width: Smoother,
    mix: Smoother,
    hp_l: OnePole,
    hp_r: OnePole,
    /// Output gain of the wet signal (makeup + partial decay compensation).
    wet_gain: f32,
    mod_phase: f32,
    mod_inc: f32,
    counter: u32,
}

impl Space {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let ms = |m: f32| (m * 0.001 * sr).ceil() as usize;
        let mk_ap = |m: f32| Allpass {
            line: DelayLine::new(ms(m) + 2),
            delay: ms(m).max(1),
        };
        let max_line = ms(LINE_MS[N - 1]) + ms(MOD_DEPTH_MS * 2.0) + 8;
        let mut s = Self {
            sr,
            primed: false,
            pre_l: DelayLine::new(ms(MAX_PREDELAY_MS) + 4),
            pre_r: DelayLine::new(ms(MAX_PREDELAY_MS) + 4),
            ap: AP_MS.map(|ch| ch.map(mk_ap)),
            lines: std::array::from_fn(|_| DelayLine::new(max_line)),
            damp: [OnePole::default(); N],
            gains: [0.0; N],
            lengths: [0.0; N],
            size: Smoother::new(0.5, 400.0, sr),
            decay: Smoother::new(2.0, 100.0, sr),
            predelay: Smoother::new(0.0, 200.0, sr),
            width: Smoother::new(1.0, 30.0, sr),
            mix: Smoother::new(0.25, 20.0, sr),
            hp_l: OnePole::default(),
            hp_r: OnePole::default(),
            wet_gain: 1.0,
            mod_phase: 0.0,
            mod_inc: 0.43 / sr,
            counter: 0,
        };
        s.set_params(&SPEC.default_values());
        s.primed = false;
        s
    }

    /// Updates line lengths and feedback gains from the smoothed size/decay.
    fn update_lines(&mut self) {
        let sc = scale(self.size.value);
        let rt60 = self.decay.value.max(0.05);
        // The FDN output sits ~5 dB under its input at 2 s and its level grows with
        // the decay time: add makeup gain and compensate about half of that growth
        // so short rooms stay audible and long washes don't explode.
        self.wet_gain = WET_MAKEUP * (2.0 / rt60).powf(0.14);
        for ((ms, len), gain) in LINE_MS
            .iter()
            .zip(self.lengths.iter_mut())
            .zip(self.gains.iter_mut())
        {
            *len = ms * 0.001 * self.sr * sc;
            // Gain for -60 dB after rt60 seconds: 10^(-3 * len / (rt60 * sr)).
            *gain = fast_db_to_gain(-60.0 * *len / (rt60 * self.sr));
        }
    }
}

/// (dry, wet) gains of an equal-power crossfade; exact at 0 and 1.
#[inline]
fn equal_power(m: f32) -> (f32, f32) {
    if m <= 0.0 {
        return (1.0, 0.0);
    }
    if m >= 1.0 {
        return (0.0, 1.0);
    }
    let a = m * std::f32::consts::FRAC_PI_2;
    (a.cos(), a.sin())
}

/// In-place normalized 8-point fast Walsh–Hadamard transform (orthogonal).
#[inline]
fn hadamard8(x: &mut [f32; N]) {
    let mut h = 1;
    while h < N {
        let mut i = 0;
        while i < N {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
            i += h * 2;
        }
        h *= 2;
    }
    let norm = 1.0 / (N as f32).sqrt();
    for v in x.iter_mut() {
        *v *= norm;
    }
}

impl Effect for Space {
    fn set_params(&mut self, v: &ParamValues) {
        let size = (v[P_SIZE] * 0.01).clamp(0.0, 1.0);
        let decay = v[P_DECAY].clamp(0.05, 30.0);
        let pre = (v[P_PREDELAY].clamp(0.0, MAX_PREDELAY_MS) * 0.001 * self.sr).max(1.0);
        let width = (v[P_WIDTH] * 0.01).clamp(0.0, 1.0);
        let m = (v[P_MIX] * 0.01).clamp(0.0, 1.0);
        // Damping 0..100% maps the in-loop low-pass from ~18 kHz to ~1.2 kHz.
        let damp_hz = 18000.0 * (1.2f32 / 18.0).powf((v[P_DAMPING] * 0.01).clamp(0.0, 1.0));
        for d in self.damp.iter_mut() {
            d.set(damp_hz, self.sr);
        }
        self.hp_l.set(v[P_LOWCUT], self.sr);
        self.hp_r.set(v[P_LOWCUT], self.sr);
        if self.primed {
            self.size.set(size);
            self.decay.set(decay);
            self.predelay.set(pre);
            self.width.set(width);
            self.mix.set(m);
        } else {
            self.size.snap(size);
            self.decay.snap(decay);
            self.predelay.snap(pre);
            self.width.snap(width);
            self.mix.snap(m);
            self.primed = true;
            self.update_lines();
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let mod_depth = MOD_DEPTH_MS * 0.001 * self.sr;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            if self.counter == 0 {
                for _ in 0..CONTROL {
                    self.size.next();
                    self.decay.next();
                }
                self.update_lines();
            }
            self.counter = (self.counter + 1) % CONTROL;

            let (a, b) = (*xl, *xr);
            // Pre-delay.
            let pd = self.predelay.next();
            self.pre_l.push(a);
            self.pre_r.push(b);
            let mut il = self.pre_l.read_linear(pd - 1.0);
            let mut ir = self.pre_r.read_linear(pd - 1.0);
            // Input diffusion.
            for ap in self.ap[0].iter_mut() {
                il = ap.process(il);
            }
            for ap in self.ap[1].iter_mut() {
                ir = ap.process(ir);
            }

            // Read the FDN lines (two of them slowly modulated, in quadrature).
            self.mod_phase += self.mod_inc;
            if self.mod_phase >= 1.0 {
                self.mod_phase -= 1.0;
            }
            let m1 = fast_sin01(self.mod_phase) * mod_depth;
            let m2 = fast_sin01(self.mod_phase + 0.25) * mod_depth;
            let mut x: [f32; N] = std::array::from_fn(|i| {
                let modulation = match i {
                    2 => m1 + mod_depth,
                    5 => m2 + mod_depth,
                    _ => 0.0,
                };
                self.lines[i].read_linear(self.lengths[i] + modulation)
            });
            // Outputs: alternating taps for decorrelated L/R.
            let wet_l = (x[0] - x[2] + x[4] - x[6] + x[1]) * 0.3;
            let wet_r = (x[1] - x[3] + x[5] - x[7] + x[6]) * 0.3;
            // Damping + decay gain, then the lossless mixing matrix.
            for ((v, damp), g) in x.iter_mut().zip(self.damp.iter_mut()).zip(self.gains) {
                *v = damp.lp(*v) * g;
            }
            hadamard8(&mut x);
            for (i, (line, v)) in self.lines.iter_mut().zip(x).enumerate() {
                let inp = if i % 2 == 0 { il } else { ir };
                line.push(v + inp * 0.5);
            }

            let wet_l = self.hp_l.hp(wet_l) * self.wet_gain;
            let wet_r = self.hp_r.hp(wet_r) * self.wet_gain;
            // Width via mid/side.
            let w = self.width.next();
            let mid = (wet_l + wet_r) * 0.5;
            let side = (wet_l - wet_r) * 0.5 * w;
            // Equal-power dry/wet law: at 25 % the dry signal only loses 0.7 dB
            // while the reverb is clearly present (a linear crossfade buried it).
            let m = self.mix.next();
            let (dry_g, wet_g) = equal_power(m);
            *xl = a * dry_g + (mid + side) * wet_g;
            *xr = b * dry_g + (mid - side) * wet_g;
        }
    }

    fn reset(&mut self) {
        self.pre_l.clear();
        self.pre_r.clear();
        for ch in self.ap.iter_mut() {
            for ap in ch.iter_mut() {
                ap.line.clear();
            }
        }
        for line in self.lines.iter_mut() {
            line.clear();
        }
        for d in self.damp.iter_mut() {
            d.reset();
        }
        self.hp_l.reset();
        self.hp_r.reset();
    }
}

#[cfg(test)]
mod tests {

    /// Regression: the wet signal used to sit ~15 dB under the dry one at the
    /// default mix, so the effect seemed to do nothing.
    #[test]
    fn reverb_is_clearly_audible_at_default_settings() {
        use crate::dsp::{Effect, ProcessCtx};
        let sr = 48_000.0;
        let render = |mix: f32| {
            let mut fx = super::Space::new(sr);
            let mut v = super::SPEC.default_values();
            v[super::P_MIX] = mix;
            fx.set_params(&v);
            let mut seed = 7u32;
            let mut lp = 0f32;
            let input: Vec<f32> = (0..96_000)
                .map(|_| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    lp += 0.25 * (((seed >> 9) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0 - lp);
                    lp
                })
                .collect();
            let (mut l, mut r) = (input.clone(), input.clone());
            let ctx = ProcessCtx { sample_rate: sr, bpm: 120.0, playing: true, beat: 0.0 };
            for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
                fx.process(&ctx, cl, cr);
            }
            (input, l)
        };
        let rms = |a: &[f32]| (a.iter().map(|x| x * x).sum::<f32>() / a.len() as f32).sqrt();
        let db = |x: f32| 20.0 * x.log10();
        // Fully wet: about the input level at the default decay.
        let (input, wet) = render(100.0);
        let ratio = db(rms(&wet[48_000..]) / rms(&input[48_000..]));
        assert!((-4.0..4.0).contains(&ratio), "wet/dry at 100 %: {ratio} dB");
        // Default mix: what the reverb adds is well above inaudible.
        let (input, out) = render(super::SPEC.params[super::P_MIX].default);
        let added: Vec<f32> = out.iter().zip(&input).map(|(o, i)| o - i).collect();
        let added_db = db(rms(&added[48_000..]) / rms(&input[48_000..]));
        assert!(added_db > -12.0, "reverb contribution at default mix: {added_db} dB");
    }

    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Space::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }

    #[test]
    fn tail_survives() {
        tail_survives_param_change(&SPEC, make, "size", 90.0);
        tail_survives_param_change(&SPEC, make, "damping", 90.0);
    }

    #[test]
    fn tail_decays_to_silence() {
        let sr = 48000.0;
        let v = with(
            &SPEC,
            with(&SPEC, SPEC.default_values(), "decay", 1.0),
            "mix",
            100.0,
        );
        let mut fx = make(sr);
        fx.set_params(&v);
        let (mut l, mut r) = test_signal(sr, 4800);
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
        let mut l = vec![0.0; 48000 * 3];
        let mut r = vec![0.0; 48000 * 3];
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
        assert!(peak(&l[..4800]) > 0.01, "no tail");
        assert!(peak(&l[48000 * 2..]) < 1e-3, "tail does not decay");
    }
}
