//! "Drive": waveshaping distortion (soft, hard, tube, fuzz) and a bitcrusher,
//! followed by a low-pass tone control, output gain and dry/wet mix.

use super::{fast_db_to_gain, idx, mix};
use crate::dsp::util::{DcBlocker, Smoother, Svf, fast_tan, soft_clip};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};
use std::f32::consts::PI;

const P_DRIVE: usize = 0;
const P_TYPE: usize = 1;
const P_TONE: usize = 2;
const P_BITS: usize = 3;
const P_DOWNSAMPLE: usize = 4;
const P_OUTPUT: usize = 5;
const P_MIX: usize = 6;

const T_HARD: usize = 1;
const T_TUBE: usize = 2;
const T_FUZZ: usize = 3;
const T_CRUSH: usize = 4;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "drive",
    name: "Drive",
    category: "Distortion",
    description: "Saturation, clipping, fuzz and bitcrushing with a tone control.",
    params: &[
        ParamSpec::lin("drive", "Drive", 0.0, 48.0, 12.0, "dB", "Drive"),
        ParamSpec::choice(
            "type",
            "Type",
            &["Soft", "Hard", "Tube", "Fuzz", "Bitcrush"],
            0,
            "Drive",
        ),
        ParamSpec::log("tone", "Tone", 500.0, 20000.0, 9000.0, "Hz", "Drive"),
        ParamSpec::int("bits", "Bits", 1.0, 16.0, 8.0, "Bitcrush"),
        ParamSpec::lin("downsample", "Downsample", 1.0, 50.0, 1.0, "x", "Bitcrush"),
        ParamSpec::lin("output", "Output", -24.0, 12.0, -6.0, "dB", "Output"),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 100.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Warm Saturation",
            values: &[
                ("drive", 8.0),
                ("type", 2.0),
                ("tone", 12000.0),
                ("output", -2.0),
            ],
        },
        Preset {
            name: "Crunch Box",
            values: &[
                ("drive", 22.0),
                ("type", 1.0),
                ("tone", 5000.0),
                ("output", -8.0),
            ],
        },
        Preset {
            name: "Buzzsaw Fuzz",
            values: &[
                ("drive", 30.0),
                ("type", 3.0),
                ("tone", 3500.0),
                ("output", -9.0),
            ],
        },
        Preset {
            name: "Eight-Bit Grit",
            values: &[
                ("drive", 3.0),
                ("type", 4.0),
                ("bits", 6.0),
                ("downsample", 6.0),
                ("tone", 12000.0),
                ("output", -3.0),
            ],
        },
    ],
    note_names: &[],
};

#[inline]
fn shape(kind: usize, x: f32) -> f32 {
    match kind {
        T_HARD => x.clamp(-1.0, 1.0),
        T_TUBE => {
            // Asymmetric: softer, later clipping on the negative side (even harmonics).
            if x >= 0.0 {
                soft_clip(x)
            } else {
                1.3 * x / (1.3 - x)
            }
        }
        T_FUZZ => {
            let y = soft_clip(3.0 * x);
            (y + 0.35 * y.abs()).clamp(-1.0, 1.0) * 0.85
        }
        _ => soft_clip(x), // Soft
    }
}

pub struct Drive {
    sr: f32,
    primed: bool,
    kind: usize,
    bits_step: f32,
    hold_inc: f32,
    drive: Smoother,
    tone: Smoother,
    output: Smoother,
    mix: Smoother,
    tone_l: Svf,
    tone_r: Svf,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    /// Sample-rate reduction state.
    hold_phase: f32,
    held: [f32; 2],
    counter: u32,
}

impl Drive {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut d = Self {
            sr,
            primed: false,
            kind: 0,
            bits_step: 1.0 / 128.0,
            hold_inc: 1.0,
            drive: Smoother::new(1.0, 20.0, sr),
            tone: Smoother::new(9000f32.log2(), 30.0, sr),
            output: Smoother::new(1.0, 20.0, sr),
            mix: Smoother::new(1.0, 20.0, sr),
            tone_l: Svf::default(),
            tone_r: Svf::default(),
            dc_l: DcBlocker::new(sr),
            dc_r: DcBlocker::new(sr),
            hold_phase: 1.0,
            held: [0.0; 2],
            counter: 0,
        };
        d.set_params(&SPEC.default_values());
        d.primed = false;
        d
    }

    fn update_tone(&mut self) {
        let fc = super::super::util::fast_pow2(self.tone.value).clamp(20.0, self.sr * 0.45);
        let g = fast_tan(PI * fc / self.sr);
        self.tone_l.set_g(g, 0.3);
        self.tone_r.set_g(g, 0.3);
    }
}

impl Effect for Drive {
    fn set_params(&mut self, v: &ParamValues) {
        self.kind = idx(v[P_TYPE]).min(T_CRUSH);
        let bits = idx(v[P_BITS]).clamp(1, 16) as i32;
        // Quantization step for a [-1, 1] signal with `bits` bits.
        self.bits_step = 2.0 / (2f32.powi(bits) - 1.0).max(1.0);
        self.hold_inc = 1.0 / v[P_DOWNSAMPLE].max(1.0);
        let t = [
            fast_db_to_gain(v[P_DRIVE]),
            v[P_TONE].max(20.0).log2(),
            fast_db_to_gain(v[P_OUTPUT]),
            (v[P_MIX] * 0.01).clamp(0.0, 1.0),
        ];
        let ss = [
            &mut self.drive,
            &mut self.tone,
            &mut self.output,
            &mut self.mix,
        ];
        for (s, t) in ss.into_iter().zip(t) {
            if self.primed { s.set(t) } else { s.snap(t) }
        }
        if !self.primed {
            self.update_tone();
        }
        self.primed = true;
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            // Tone coefficients at control rate (every 16 samples).
            if self.counter == 0 {
                for _ in 0..16 {
                    self.tone.next();
                }
                self.update_tone();
            }
            self.counter = (self.counter + 1) % 16;

            let (a, b) = (*xl, *xr);
            let g = self.drive.next();
            let out = self.output.next();
            let m = self.mix.next();
            let (mut yl, mut yr);
            if self.kind == T_CRUSH {
                // Sample & hold at a reduced rate, then quantize.
                self.hold_phase += self.hold_inc;
                if self.hold_phase >= 1.0 {
                    self.hold_phase -= 1.0;
                    self.held = [(a * g).clamp(-1.0, 1.0), (b * g).clamp(-1.0, 1.0)];
                }
                let q = self.bits_step;
                yl = (self.held[0] / q).round() * q;
                yr = (self.held[1] / q).round() * q;
            } else {
                yl = shape(self.kind, a * g);
                yr = shape(self.kind, b * g);
            }
            yl = self.dc_l.process(self.tone_l.process(yl).0);
            yr = self.dc_r.process(self.tone_r.process(yr).0);
            *xl = mix(a, yl * out, m);
            *xr = mix(b, yr * out, m);
        }
    }

    fn reset(&mut self) {
        self.tone_l.reset();
        self.tone_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
        self.held = [0.0; 2];
        self.hold_phase = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Drive::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }

    #[test]
    fn every_type_is_bounded() {
        let sr = 48000.0;
        for t in 0..5 {
            let mut v = with(&SPEC, SPEC.default_values(), "type", t as f32);
            v = with(&SPEC, v, "drive", 48.0);
            v = with(&SPEC, v, "output", 0.0);
            let mut fx = make(sr);
            fx.set_params(&v);
            let (mut l, mut r) = test_signal(sr, 24000);
            run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
            assert_finite(&l, "drive");
            assert!(peak(&l).max(peak(&r)) < 1.6, "type {t}");
        }
    }
}
