//! "Tremolo": LFO amplitude modulation, free or tempo-synced, with an
//! auto-pan mode (channels modulated in opposition).

use super::{Lfo, idx, on};
use crate::dsp::util::{LFO_SHAPES, SYNC_NAMES, Smoother, sync_seconds};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};

const P_RATE: usize = 0;
const P_SYNC: usize = 1;
const P_RATE_SYNC: usize = 2;
const P_DEPTH: usize = 3;
const P_SHAPE: usize = 4;
const P_STEREO: usize = 5;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "tremolo",
    name: "Tremolo",
    category: "Modulation",
    description: "Amplitude tremolo and auto-pan, free running or tempo-synced.",
    params: &[
        ParamSpec::log("rate", "Rate", 0.1, 20.0, 4.0, "Hz", "Modulation"),
        ParamSpec::toggle("sync", "Sync", false, "Modulation"),
        ParamSpec::choice("rate_sync", "Note", SYNC_NAMES, 5, "Modulation"),
        ParamSpec::lin("depth", "Depth", 0.0, 100.0, 50.0, "%", "Modulation"),
        ParamSpec::choice("shape", "Shape", LFO_SHAPES, 0, "Modulation"),
        ParamSpec::toggle("stereo", "Auto-Pan", false, "Modulation"),
    ],
    presets: &[
        Preset {
            name: "Amp Shimmer",
            values: &[("rate", 5.5), ("depth", 45.0), ("shape", 1.0)],
        },
        Preset {
            name: "Slow Pan",
            values: &[("rate", 0.3), ("depth", 80.0), ("stereo", 1.0)],
        },
        Preset {
            name: "Helicopter Chop",
            values: &[
                ("sync", 1.0),
                ("rate_sync", 2.0),
                ("depth", 100.0),
                ("shape", 3.0),
            ],
        },
    ],
    note_names: &[],
};

pub struct Tremolo {
    sr: f32,
    primed: bool,
    rate: f32,
    sync: bool,
    sync_idx: usize,
    shape: usize,
    stereo: Smoother,
    depth: Smoother,
    /// De-clicks the LFO (square, S&H) with a ~2 ms one-pole.
    lfo_smooth: Smoother,
    lfo: Lfo,
}

impl Tremolo {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut t = Self {
            sr,
            primed: false,
            rate: 4.0,
            sync: false,
            sync_idx: 5,
            shape: 0,
            stereo: Smoother::new(0.0, 30.0, sr),
            depth: Smoother::new(0.5, 30.0, sr),
            lfo_smooth: Smoother::new(0.0, 2.0, sr),
            lfo: Lfo::new(31),
        };
        t.set_params(&SPEC.default_values());
        t.primed = false;
        t
    }
}

impl Effect for Tremolo {
    fn set_params(&mut self, v: &ParamValues) {
        self.rate = v[P_RATE];
        self.sync = on(v[P_SYNC]);
        self.sync_idx = idx(v[P_RATE_SYNC]).min(SYNC_NAMES.len() - 1);
        self.shape = idx(v[P_SHAPE]).min(LFO_SHAPES.len() - 1);
        let depth = (v[P_DEPTH] * 0.01).clamp(0.0, 1.0);
        let stereo = if on(v[P_STEREO]) { 1.0 } else { 0.0 };
        if self.primed {
            self.depth.set(depth);
            self.stereo.set(stereo);
        } else {
            self.depth.snap(depth);
            self.stereo.snap(stereo);
            self.primed = true;
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let hz = if self.sync {
            1.0 / sync_seconds(self.sync_idx, ctx.bpm)
        } else {
            self.rate
        };
        let inc = hz / self.sr;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            self.lfo_smooth.set(self.lfo.at(self.shape, 0.0));
            let lfo = self.lfo_smooth.next();
            self.lfo.advance(inc);
            let depth = self.depth.next();
            let st = self.stereo.next();
            // Unipolar modulator in [0, 1]; right channel inverts in auto-pan.
            let ml = 0.5 + 0.5 * lfo;
            let mr = ml + (1.0 - 2.0 * ml) * st;
            *xl *= 1.0 - depth * (1.0 - ml);
            *xr *= 1.0 - depth * (1.0 - mr);
        }
    }

    fn reset(&mut self) {
        self.lfo.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Tremolo::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, None);
    }

    #[test]
    fn zero_depth_is_transparent() {
        let sr = 48000.0;
        let mut fx = make(sr);
        fx.set_params(&with(&SPEC, SPEC.default_values(), "depth", 0.0));
        let (dl, dr) = test_signal(sr, 4800);
        let (mut l, mut r) = (dl.clone(), dr.clone());
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 64);
        for i in 0..l.len() {
            assert!((l[i] - dl[i]).abs() < 1e-6 && (r[i] - dr[i]).abs() < 1e-6);
        }
    }
}
