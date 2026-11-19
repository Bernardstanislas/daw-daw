//! "Stereo Width": mid/side width, balance, gain, polarity flips and mono.

use super::on;
use crate::dsp::util::Smoother;
use crate::dsp::{
    DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx, db_to_gain,
};

const P_WIDTH: usize = 0;
const P_GAIN: usize = 1;
const P_PAN: usize = 2;
const P_INVERT_L: usize = 3;
const P_INVERT_R: usize = 4;
const P_MONO: usize = 5;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "width",
    name: "Stereo Width",
    category: "Utility",
    description: "Stereo width, balance, gain, polarity and mono utility.",
    params: &[
        ParamSpec::lin("width", "Width", 0.0, 200.0, 100.0, "%", "Stereo"),
        ParamSpec::lin("gain", "Gain", -24.0, 24.0, 0.0, "dB", "Output"),
        ParamSpec::lin("pan", "Pan", -100.0, 100.0, 0.0, "%", "Output"),
        ParamSpec::toggle("invert_l", "Invert L", false, "Polarity"),
        ParamSpec::toggle("invert_r", "Invert R", false, "Polarity"),
        ParamSpec::toggle("mono", "Mono", false, "Stereo"),
    ],
    presets: &[
        Preset {
            name: "Mono Check",
            values: &[("mono", 1.0)],
        },
        Preset {
            name: "Extra Wide",
            values: &[("width", 160.0), ("gain", -1.5)],
        },
        Preset {
            name: "Narrow Focus",
            values: &[("width", 50.0)],
        },
    ],
    note_names: &[],
};

pub struct StereoWidth {
    primed: bool,
    width: Smoother,
    gain_l: Smoother,
    gain_r: Smoother,
}

impl StereoWidth {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut w = Self {
            primed: false,
            width: Smoother::new(1.0, 20.0, sr),
            gain_l: Smoother::new(1.0, 20.0, sr),
            gain_r: Smoother::new(1.0, 20.0, sr),
        };
        w.set_params(&SPEC.default_values());
        w.primed = false;
        w
    }
}

impl Effect for StereoWidth {
    fn set_params(&mut self, v: &ParamValues) {
        let width = if on(v[P_MONO]) {
            0.0
        } else {
            (v[P_WIDTH] * 0.01).clamp(0.0, 2.0)
        };
        let gain = db_to_gain(v[P_GAIN]);
        // Balance law: unity at center, the opposite side fades out linearly.
        let pan = (v[P_PAN] * 0.01).clamp(-1.0, 1.0);
        let bal_l = (1.0 - pan).min(1.0);
        let bal_r = (1.0 + pan).min(1.0);
        let pol_l = if on(v[P_INVERT_L]) { -1.0 } else { 1.0 };
        let pol_r = if on(v[P_INVERT_R]) { -1.0 } else { 1.0 };
        let t = [width, gain * bal_l * pol_l, gain * bal_r * pol_r];
        let ss = [&mut self.width, &mut self.gain_l, &mut self.gain_r];
        for (s, t) in ss.into_iter().zip(t) {
            if self.primed { s.set(t) } else { s.snap(t) }
        }
        self.primed = true;
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let w = self.width.next();
            let gl = self.gain_l.next();
            let gr = self.gain_r.next();
            let mid = (*xl + *xr) * 0.5;
            let side = (*xl - *xr) * 0.5 * w;
            *xl = (mid + side) * gl;
            *xr = (mid - side) * gr;
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(StereoWidth::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, None);
    }

    #[test]
    fn defaults_are_transparent_and_mono_sums() {
        let sr = 48000.0;
        let (dl, dr) = test_signal(sr, 4800);
        let (mut l, mut r) = (dl.clone(), dr.clone());
        run_effect(make(sr).as_mut(), sr, &mut l, &mut r, 64);
        for i in 0..l.len() {
            assert!((l[i] - dl[i]).abs() < 1e-5 && (r[i] - dr[i]).abs() < 1e-5);
        }
        let mut fx = make(sr);
        fx.set_params(&with(&SPEC, SPEC.default_values(), "mono", 1.0));
        let (mut l, mut r) = (dl.clone(), dr.clone());
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 64);
        for i in 0..l.len() {
            assert!((l[i] - r[i]).abs() < 1e-6);
        }
    }
}
