//! "Phaser": cascade of first-order allpass stages swept by an LFO (in
//! octaves around a center frequency), with feedback. The right channel's
//! LFO is offset by 90° for a stereo sweep.

use super::{Lfo, idx, mix};
use crate::dsp::util::{Smoother, fast_pow2, fast_tan, flush, soft_clip};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};
use std::f32::consts::PI;

const P_RATE: usize = 0;
const P_DEPTH: usize = 1;
const P_STAGES: usize = 2;
const P_FEEDBACK: usize = 3;
const P_CENTER: usize = 4;
const P_MIX: usize = 5;

const MAX_STAGES: usize = 8;
/// Sweep range (octaves, each side of the center) at depth 100%.
const SWEEP_OCTAVES: f32 = 2.5;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "phaser",
    name: "Phaser",
    category: "Modulation",
    description: "Swirling 4/6/8-stage stereo phaser with feedback.",
    params: &[
        ParamSpec::log("rate", "Rate", 0.02, 10.0, 0.4, "Hz", "Modulation"),
        ParamSpec::lin("depth", "Depth", 0.0, 100.0, 70.0, "%", "Modulation"),
        ParamSpec::choice("stages", "Stages", &["4", "6", "8"], 1, "Modulation"),
        ParamSpec::lin("feedback", "Feedback", -90.0, 90.0, 40.0, "%", "Modulation"),
        ParamSpec::log("center", "Center", 100.0, 4000.0, 700.0, "Hz", "Modulation"),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 50.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Slow Swirl",
            values: &[
                ("rate", 0.15),
                ("depth", 80.0),
                ("stages", 1.0),
                ("feedback", 30.0),
            ],
        },
        Preset {
            name: "Liquid Keys",
            values: &[
                ("rate", 0.6),
                ("depth", 60.0),
                ("stages", 0.0),
                ("feedback", 55.0),
                ("center", 900.0),
            ],
        },
        Preset {
            name: "Laser Sweep",
            values: &[
                ("rate", 2.5),
                ("depth", 90.0),
                ("stages", 2.0),
                ("feedback", 80.0),
                ("center", 1200.0),
            ],
        },
    ],
    note_names: &[],
};

/// First-order allpass, transposed direct form.
#[derive(Debug, Clone, Copy, Default)]
struct Allpass1 {
    z: f32,
}

impl Allpass1 {
    #[inline]
    fn process(&mut self, x: f32, a: f32) -> f32 {
        let y = a * x + self.z;
        self.z = x - a * y;
        y
    }
}

pub struct Phaser {
    sr: f32,
    primed: bool,
    stages: usize,
    ap: [[Allpass1; MAX_STAGES]; 2],
    fb_state: [f32; 2],
    lfo: Lfo,
    rate: Smoother,
    depth: Smoother,
    feedback: Smoother,
    center: Smoother,
    mix: Smoother,
}

impl Phaser {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut p = Self {
            sr,
            primed: false,
            stages: 6,
            ap: [[Allpass1::default(); MAX_STAGES]; 2],
            fb_state: [0.0; 2],
            lfo: Lfo::new(11),
            rate: Smoother::new(0.4, 50.0, sr),
            depth: Smoother::new(0.7, 50.0, sr),
            feedback: Smoother::new(0.4, 30.0, sr),
            center: Smoother::new(700f32.log2(), 50.0, sr),
            mix: Smoother::new(0.5, 20.0, sr),
        };
        p.set_params(&SPEC.default_values());
        p.primed = false;
        p
    }

    /// Allpass coefficient for a break frequency given in log2(Hz).
    #[inline]
    fn coef(&self, log2_hz: f32) -> f32 {
        let fc = fast_pow2(log2_hz).clamp(20.0, self.sr * 0.45);
        let t = fast_tan(PI * fc / self.sr);
        (t - 1.0) / (t + 1.0)
    }
}

impl Effect for Phaser {
    fn set_params(&mut self, v: &ParamValues) {
        self.stages = [4, 6, 8][idx(v[P_STAGES]).min(2)];
        let t = [
            v[P_RATE],
            (v[P_DEPTH] * 0.01).clamp(0.0, 1.0),
            (v[P_FEEDBACK] * 0.01).clamp(-0.9, 0.9),
            v[P_CENTER].max(20.0).log2(),
            (v[P_MIX] * 0.01).clamp(0.0, 1.0),
        ];
        let ss = [
            &mut self.rate,
            &mut self.depth,
            &mut self.feedback,
            &mut self.center,
            &mut self.mix,
        ];
        for (s, t) in ss.into_iter().zip(t) {
            if self.primed { s.set(t) } else { s.snap(t) }
        }
        self.primed = true;
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let inv_sr = 1.0 / self.sr;
        let n = self.stages;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let rate = self.rate.next();
            let depth = self.depth.next() * SWEEP_OCTAVES;
            let fb = self.feedback.next();
            let center = self.center.next();
            let m = self.mix.next();
            let a_l = self.coef(center + depth * self.lfo.at(1, 0.0));
            let a_r = self.coef(center + depth * self.lfo.at(1, 0.25));
            self.lfo.advance(rate * inv_sr);

            // Wet = allpass chain only: at mix 50% the dry/wet sum produces
            // the classic phaser notches.
            for (ch, (x, a)) in [(xl, a_l), (xr, a_r)].into_iter().enumerate() {
                let dry = *x;
                let mut y = dry + soft_clip(self.fb_state[ch] * fb);
                for stage in self.ap[ch][..n].iter_mut() {
                    y = stage.process(y, a);
                }
                self.fb_state[ch] = flush(y);
                *x = mix(dry, y, m);
            }
        }
    }

    fn reset(&mut self) {
        self.ap = [[Allpass1::default(); MAX_STAGES]; 2];
        self.fb_state = [0.0; 2];
        self.lfo.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Phaser::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }
}
