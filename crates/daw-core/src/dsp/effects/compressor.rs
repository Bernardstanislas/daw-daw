//! "Compressor": feed-forward, stereo-linked peak compressor with soft knee,
//! makeup gain and parallel (dry/wet) mix.

use super::{fast_db_to_gain, fast_gain_to_db, mix};
use crate::dsp::util::Smoother;
use crate::dsp::{
    DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx, db_to_gain,
};

const P_THRESHOLD: usize = 0;
const P_RATIO: usize = 1;
const P_ATTACK: usize = 2;
const P_RELEASE: usize = 3;
const P_KNEE: usize = 4;
const P_MAKEUP: usize = 5;
const P_MIX: usize = 6;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "compressor",
    name: "Compressor",
    category: "Dynamics",
    description: "Stereo-linked compressor with soft knee and parallel mix.",
    params: &[
        ParamSpec::lin(
            "threshold",
            "Threshold",
            -60.0,
            0.0,
            -18.0,
            "dB",
            "Dynamics",
        ),
        ParamSpec::log("ratio", "Ratio", 1.0, 20.0, 4.0, ":1", "Dynamics"),
        ParamSpec::log("attack", "Attack", 0.1, 100.0, 10.0, "ms", "Timing"),
        ParamSpec::log("release", "Release", 10.0, 1000.0, 120.0, "ms", "Timing"),
        ParamSpec::lin("knee", "Knee", 0.0, 24.0, 6.0, "dB", "Dynamics"),
        ParamSpec::lin("makeup", "Makeup", 0.0, 24.0, 0.0, "dB", "Output"),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 100.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Gentle Glue",
            values: &[
                ("threshold", -14.0),
                ("ratio", 2.0),
                ("attack", 30.0),
                ("release", 200.0),
                ("knee", 10.0),
                ("makeup", 2.0),
            ],
        },
        Preset {
            name: "Vocal Leveler",
            values: &[
                ("threshold", -22.0),
                ("ratio", 4.0),
                ("attack", 5.0),
                ("release", 90.0),
                ("knee", 6.0),
                ("makeup", 6.0),
            ],
        },
        Preset {
            name: "Drum Smash",
            values: &[
                ("threshold", -30.0),
                ("ratio", 12.0),
                ("attack", 1.0),
                ("release", 60.0),
                ("knee", 2.0),
                ("makeup", 10.0),
                ("mix", 45.0),
            ],
        },
        Preset {
            name: "Bass Tamer",
            values: &[
                ("threshold", -20.0),
                ("ratio", 6.0),
                ("attack", 15.0),
                ("release", 150.0),
                ("knee", 4.0),
                ("makeup", 5.0),
            ],
        },
    ],
    note_names: &[],
};

pub struct Compressor {
    sr: f32,
    primed: bool,
    threshold: f32,
    slope: f32,
    knee: f32,
    att_coef: f32,
    rel_coef: f32,
    makeup: Smoother,
    mix: Smoother,
    /// Detector envelope (linear peak).
    env: f32,
}

fn coef(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sr).max(1.0)).exp()
}

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut c = Self {
            sr,
            primed: false,
            threshold: -18.0,
            slope: 0.75,
            knee: 6.0,
            att_coef: 0.0,
            rel_coef: 0.0,
            makeup: Smoother::new(1.0, 20.0, sr),
            mix: Smoother::new(1.0, 20.0, sr),
            env: 0.0,
        };
        c.set_params(&SPEC.default_values());
        c.primed = false;
        c
    }

    /// Gain reduction in dB (<= 0) for a detector level in dB.
    #[inline]
    fn gain_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold;
        let half = self.knee * 0.5;
        if over <= -half {
            0.0
        } else if over >= half || self.knee <= 0.0 {
            -self.slope * over
        } else {
            let x = over + half;
            -self.slope * x * x / (2.0 * self.knee)
        }
    }
}

impl Effect for Compressor {
    fn set_params(&mut self, v: &ParamValues) {
        self.threshold = v[P_THRESHOLD];
        self.slope = 1.0 - 1.0 / v[P_RATIO].max(1.0);
        self.knee = v[P_KNEE].max(0.0);
        self.att_coef = coef(v[P_ATTACK], self.sr);
        self.rel_coef = coef(v[P_RELEASE], self.sr);
        let makeup = db_to_gain(v[P_MAKEUP]);
        let m = (v[P_MIX] * 0.01).clamp(0.0, 1.0);
        if self.primed {
            self.makeup.set(makeup);
            self.mix.set(m);
        } else {
            self.makeup.snap(makeup);
            self.mix.snap(m);
            self.primed = true;
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let (a, b) = (*xl, *xr);
            let x = a.abs().max(b.abs());
            let c = if x > self.env {
                self.att_coef
            } else {
                self.rel_coef
            };
            self.env = x + (self.env - x) * c;
            if self.env < 1.0e-9 {
                self.env = 0.0;
            }
            let gr = self.gain_db(fast_gain_to_db(self.env));
            let g = fast_db_to_gain(gr) * self.makeup.next();
            let m = self.mix.next();
            *xl = mix(a, a * g, m);
            *xr = mix(b, b * g, m);
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Compressor::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }

    #[test]
    fn reduces_loud_signal() {
        let sr = 48000.0;
        let v = with(
            &SPEC,
            with(&SPEC, SPEC.default_values(), "threshold", -20.0),
            "ratio",
            10.0,
        );
        let mut fx = make(sr);
        fx.set_params(&v);
        let mut l: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();
        let mut r = l.clone();
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
        // 0.9 ≈ -1 dB, 19 dB over → ~ -17 dB reduction → ≈ 0.14 peak.
        let p = peak(&l[24000..]);
        assert!(p < 0.25 && p > 0.05, "peak {p}");
    }
}
