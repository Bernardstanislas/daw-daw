//! "Channel EQ": low cut, low shelf, mid peak, high shelf, high cut and
//! output gain, built from RBJ biquads. Parameters are smoothed and the
//! coefficients recomputed at control rate while they move.

use super::on;
use crate::dsp::util::{Biquad, BiquadType, Smoother};
use crate::dsp::{
    DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx, db_to_gain,
};

const P_LOWCUT_ON: usize = 0;
const P_LOWCUT: usize = 1;
const P_LOW_FREQ: usize = 2;
const P_LOW_GAIN: usize = 3;
const P_MID_FREQ: usize = 4;
const P_MID_GAIN: usize = 5;
const P_MID_Q: usize = 6;
const P_HIGH_FREQ: usize = 7;
const P_HIGH_GAIN: usize = 8;
const P_HIGHCUT_ON: usize = 9;
const P_HIGHCUT: usize = 10;
const P_OUTPUT: usize = 11;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "eq",
    name: "Channel EQ",
    category: "EQ",
    description: "Five-band channel equalizer: low/high cut, two shelves and a sweepable mid.",
    params: &[
        ParamSpec::toggle("lowcut_on", "Low Cut", false, "Low Cut"),
        ParamSpec::log(
            "lowcut",
            "Low Cut Freq",
            20.0,
            1000.0,
            80.0,
            "Hz",
            "Low Cut",
        ),
        ParamSpec::log("low_freq", "Low Freq", 20.0, 1000.0, 120.0, "Hz", "Low"),
        ParamSpec::lin("low_gain", "Low Gain", -18.0, 18.0, 0.0, "dB", "Low"),
        ParamSpec::log("mid_freq", "Mid Freq", 100.0, 10000.0, 1000.0, "Hz", "Mid"),
        ParamSpec::lin("mid_gain", "Mid Gain", -18.0, 18.0, 0.0, "dB", "Mid"),
        ParamSpec::log("mid_q", "Mid Q", 0.1, 10.0, 0.9, "", "Mid"),
        ParamSpec::log(
            "high_freq",
            "High Freq",
            1000.0,
            20000.0,
            6000.0,
            "Hz",
            "High",
        ),
        ParamSpec::lin("high_gain", "High Gain", -18.0, 18.0, 0.0, "dB", "High"),
        ParamSpec::toggle("highcut_on", "High Cut", false, "High Cut"),
        ParamSpec::log(
            "highcut",
            "High Cut Freq",
            1000.0,
            20000.0,
            16000.0,
            "Hz",
            "High Cut",
        ),
        ParamSpec::lin("output", "Output", -24.0, 24.0, 0.0, "dB", "Output"),
    ],
    presets: &[
        Preset {
            name: "Clean Up",
            values: &[
                ("lowcut_on", 1.0),
                ("lowcut", 90.0),
                ("mid_freq", 350.0),
                ("mid_gain", -3.0),
                ("mid_q", 1.2),
            ],
        },
        Preset {
            name: "Air Lift",
            values: &[
                ("high_freq", 9000.0),
                ("high_gain", 4.5),
                ("mid_freq", 2500.0),
                ("mid_gain", 1.5),
                ("output", -2.0),
            ],
        },
        Preset {
            name: "Warm Blanket",
            values: &[
                ("low_freq", 180.0),
                ("low_gain", 3.0),
                ("high_gain", -4.0),
                ("high_freq", 5000.0),
                ("highcut_on", 1.0),
                ("highcut", 11000.0),
                ("output", -2.0),
            ],
        },
        Preset {
            name: "Telephone",
            values: &[
                ("lowcut_on", 1.0),
                ("lowcut", 450.0),
                ("highcut_on", 1.0),
                ("highcut", 3200.0),
                ("mid_freq", 1500.0),
                ("mid_gain", 6.0),
                ("mid_q", 1.5),
                ("output", -3.0),
            ],
        },
    ],
    note_names: &[],
};

/// Number of smoothed values: lowcut, low f/g, mid f/g/q, high f/g, highcut.
const N_SMOOTH: usize = 9;
/// Coefficients are recomputed every `CONTROL` samples while smoothing.
const CONTROL: u32 = 16;

pub struct ChannelEq {
    sr: f32,
    primed: bool,
    /// Smoothed (in log domain for frequencies) parameter values.
    smooth: [Smoother; N_SMOOTH],
    lowcut_en: Smoother,
    highcut_en: Smoother,
    output: Smoother,
    /// Filters: [lowcut, low shelf, mid, high shelf, highcut] per channel.
    fl: [Biquad; 5],
    fr: [Biquad; 5],
    counter: u32,
    dirty: bool,
}

impl ChannelEq {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut eq = Self {
            sr,
            primed: false,
            smooth: [Smoother::new(0.0, 30.0, sr); N_SMOOTH],
            lowcut_en: Smoother::new(0.0, 15.0, sr),
            highcut_en: Smoother::new(0.0, 15.0, sr),
            output: Smoother::new(1.0, 20.0, sr),
            fl: [Biquad::default(); 5],
            fr: [Biquad::default(); 5],
            counter: 0,
            dirty: true,
        };
        eq.set_params(&SPEC.default_values());
        eq.primed = false;
        eq
    }

    fn update_coefs(&mut self) {
        let s = |i: usize| self.smooth[i].value;
        let sr = self.sr;
        let bands = [
            (BiquadType::HighPass, s(0).exp(), 0.707, 0.0),
            (BiquadType::LowShelf, s(1).exp(), 0.707, s(2)),
            (BiquadType::Peak, s(3).exp(), s(5).exp(), s(4)),
            (BiquadType::HighShelf, s(6).exp(), 0.707, s(7)),
            (BiquadType::LowPass, s(8).exp(), 0.707, 0.0),
        ];
        for (i, (kind, f, q, g)) in bands.into_iter().enumerate() {
            self.fl[i].set(kind, f, q, g, sr);
            self.fr[i].set(kind, f, q, g, sr);
        }
    }
}

impl Effect for ChannelEq {
    fn set_params(&mut self, v: &ParamValues) {
        let targets = [
            v[P_LOWCUT].max(1.0).ln(),
            v[P_LOW_FREQ].max(1.0).ln(),
            v[P_LOW_GAIN],
            v[P_MID_FREQ].max(1.0).ln(),
            v[P_MID_GAIN],
            v[P_MID_Q].max(0.01).ln(),
            v[P_HIGH_FREQ].max(1.0).ln(),
            v[P_HIGH_GAIN],
            v[P_HIGHCUT].max(1.0).ln(),
        ];
        let lc = if on(v[P_LOWCUT_ON]) { 1.0 } else { 0.0 };
        let hc = if on(v[P_HIGHCUT_ON]) { 1.0 } else { 0.0 };
        let out = db_to_gain(v[P_OUTPUT]);
        if self.primed {
            for (s, t) in self.smooth.iter_mut().zip(targets) {
                s.set(t);
            }
            self.lowcut_en.set(lc);
            self.highcut_en.set(hc);
            self.output.set(out);
        } else {
            for (s, t) in self.smooth.iter_mut().zip(targets) {
                s.snap(t);
            }
            self.lowcut_en.snap(lc);
            self.highcut_en.snap(hc);
            self.output.snap(out);
            self.primed = true;
            self.update_coefs();
        }
        self.dirty = true;
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            if self.dirty {
                if self.counter == 0 {
                    let mut moving = false;
                    for s in self.smooth.iter_mut() {
                        // Advance the smoother by a whole control period.
                        for _ in 0..CONTROL {
                            s.next();
                        }
                        if (s.value - s.target).abs() > 1e-4 {
                            moving = true;
                        } else {
                            s.snap(s.target);
                        }
                    }
                    self.update_coefs();
                    self.dirty = moving;
                }
                self.counter = (self.counter + 1) % CONTROL;
            }
            let lc = self.lowcut_en.next();
            let hc = self.highcut_en.next();
            let g = self.output.next();
            let mut a = *xl;
            let mut b = *xr;
            // Cut filters always run so enabling them is click-free.
            let (cl, cr) = (self.fl[0].process(a), self.fr[0].process(b));
            a += (cl - a) * lc;
            b += (cr - b) * lc;
            for i in 1..4 {
                a = self.fl[i].process(a);
                b = self.fr[i].process(b);
            }
            let (cl, cr) = (self.fl[4].process(a), self.fr[4].process(b));
            a += (cl - a) * hc;
            b += (cr - b) * hc;
            *xl = a * g;
            *xr = b * g;
        }
    }

    fn reset(&mut self) {
        for f in self.fl.iter_mut().chain(self.fr.iter_mut()) {
            f.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(ChannelEq::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, None);
    }

    #[test]
    fn flat_eq_is_transparent() {
        let sr = 48000.0;
        let (dl, dr) = test_signal(sr, 24000);
        let (mut l, mut r) = (dl.clone(), dr.clone());
        let mut fx = make(sr);
        fx.set_params(&SPEC.default_values());
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
        for i in 0..l.len() {
            assert!(
                (l[i] - dl[i]).abs() < 1e-4 && (r[i] - dr[i]).abs() < 1e-4,
                "at {i}"
            );
        }
    }

    #[test]
    fn low_cut_removes_bass() {
        let sr = 48000.0;
        let v = with(
            &SPEC,
            with(&SPEC, SPEC.default_values(), "lowcut_on", 1.0),
            "lowcut",
            500.0,
        );
        let mut fx = make(sr);
        fx.set_params(&v);
        let mut l: Vec<f32> = (0..48000)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 50.0 / sr).sin())
            .collect();
        let mut r = l.clone();
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
        assert!(peak(&l[24000..]) < 0.05);
    }
}
