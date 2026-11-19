//! "Auto Filter": resonant state-variable filter whose cutoff is modulated
//! by an LFO (free or tempo-synced) and an envelope follower.

use super::{Lfo, idx, mix, on};
use crate::dsp::util::{
    LFO_SHAPES, SYNC_NAMES, Smoother, Svf, SvfMode, fast_pow2, fast_tan, flush, sync_seconds,
};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};
use std::f32::consts::PI;

const P_MODE: usize = 0;
const P_CUTOFF: usize = 1;
const P_RES: usize = 2;
const P_SYNC: usize = 3;
const P_RATE: usize = 4;
const P_RATE_SYNC: usize = 5;
const P_SHAPE: usize = 6;
const P_DEPTH: usize = 7;
const P_ENV: usize = 8;
const P_MIX: usize = 9;

/// Modulation range in octaves at 100% depth / envelope amount.
const MOD_OCTAVES: f32 = 4.0;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "filter",
    name: "Auto Filter",
    category: "Filter",
    description: "Resonant filter swept by a tempo-syncable LFO and an envelope follower.",
    params: &[
        ParamSpec::choice(
            "mode",
            "Mode",
            &["Low Pass", "Band Pass", "High Pass"],
            0,
            "Filter",
        ),
        ParamSpec::log("cutoff", "Cutoff", 20.0, 20000.0, 1200.0, "Hz", "Filter"),
        ParamSpec::lin("resonance", "Resonance", 0.0, 100.0, 40.0, "%", "Filter"),
        ParamSpec::toggle("sync", "Sync", false, "LFO"),
        ParamSpec::log("rate", "Rate", 0.05, 20.0, 1.0, "Hz", "LFO"),
        ParamSpec::choice("rate_sync", "Note", SYNC_NAMES, 8, "LFO"),
        ParamSpec::choice("shape", "Shape", LFO_SHAPES, 0, "LFO"),
        ParamSpec::lin("depth", "LFO Depth", 0.0, 100.0, 40.0, "%", "LFO"),
        ParamSpec::lin("env", "Envelope", -100.0, 100.0, 0.0, "%", "Envelope"),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 100.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Slow Breather",
            values: &[
                ("cutoff", 900.0),
                ("resonance", 35.0),
                ("rate", 0.2),
                ("depth", 60.0),
            ],
        },
        Preset {
            name: "Sixteenth Gate",
            values: &[
                ("cutoff", 600.0),
                ("resonance", 55.0),
                ("sync", 1.0),
                ("rate_sync", 2.0),
                ("shape", 3.0),
                ("depth", 70.0),
            ],
        },
        Preset {
            name: "Funky Wah",
            values: &[
                ("mode", 1.0),
                ("cutoff", 450.0),
                ("resonance", 70.0),
                ("depth", 0.0),
                ("env", 80.0),
            ],
        },
        Preset {
            name: "Random Steps",
            values: &[
                ("cutoff", 1500.0),
                ("resonance", 50.0),
                ("sync", 1.0),
                ("rate_sync", 5.0),
                ("shape", 4.0),
                ("depth", 55.0),
            ],
        },
    ],
    note_names: &[],
};

pub struct AutoFilter {
    sr: f32,
    primed: bool,
    mode: SvfMode,
    sync: bool,
    sync_idx: usize,
    shape: usize,
    rate: f32,
    svf_l: Svf,
    svf_r: Svf,
    lfo: Lfo,
    cutoff: Smoother,
    res: Smoother,
    depth: Smoother,
    env_amt: Smoother,
    mix: Smoother,
    /// Smooths the LFO output (de-clicks square / S&H steps).
    lfo_smooth: Smoother,
    env: f32,
    env_att: f32,
    env_rel: f32,
}

impl AutoFilter {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let mut f = Self {
            sr,
            primed: false,
            mode: SvfMode::LowPass,
            sync: false,
            sync_idx: 8,
            shape: 0,
            rate: 1.0,
            svf_l: Svf::default(),
            svf_r: Svf::default(),
            lfo: Lfo::new(23),
            cutoff: Smoother::new(1200f32.log2(), 30.0, sr),
            res: Smoother::new(0.4, 30.0, sr),
            depth: Smoother::new(0.4, 30.0, sr),
            env_amt: Smoother::new(0.0, 30.0, sr),
            mix: Smoother::new(1.0, 20.0, sr),
            lfo_smooth: Smoother::new(0.0, 2.0, sr),
            env: 0.0,
            env_att: (-1.0 / (0.005 * sr)).exp(),
            env_rel: (-1.0 / (0.15 * sr)).exp(),
        };
        f.set_params(&SPEC.default_values());
        f.primed = false;
        f
    }
}

impl Effect for AutoFilter {
    fn set_params(&mut self, v: &ParamValues) {
        self.mode = match idx(v[P_MODE]) {
            1 => SvfMode::BandPass,
            2 => SvfMode::HighPass,
            _ => SvfMode::LowPass,
        };
        self.sync = on(v[P_SYNC]);
        self.sync_idx = idx(v[P_RATE_SYNC]).min(SYNC_NAMES.len() - 1);
        self.shape = idx(v[P_SHAPE]).min(LFO_SHAPES.len() - 1);
        self.rate = v[P_RATE];
        let t = [
            v[P_CUTOFF].max(20.0).log2(),
            // Keep a hair below self-oscillation.
            (v[P_RES] * 0.01).clamp(0.0, 1.0) * 0.97,
            (v[P_DEPTH] * 0.01).clamp(0.0, 1.0) * MOD_OCTAVES,
            (v[P_ENV] * 0.01).clamp(-1.0, 1.0) * MOD_OCTAVES,
            (v[P_MIX] * 0.01).clamp(0.0, 1.0),
        ];
        let ss = [
            &mut self.cutoff,
            &mut self.res,
            &mut self.depth,
            &mut self.env_amt,
            &mut self.mix,
        ];
        for (s, t) in ss.into_iter().zip(t) {
            if self.primed { s.set(t) } else { s.snap(t) }
        }
        self.primed = true;
    }

    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let hz = if self.sync {
            1.0 / sync_seconds(self.sync_idx, ctx.bpm)
        } else {
            self.rate
        };
        let inc = hz / self.sr;
        let max_log = (self.sr * 0.45).log2();
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let (a, b) = (*xl, *xr);
            // Envelope follower on the stereo peak.
            let x = a.abs().max(b.abs());
            let c = if x > self.env {
                self.env_att
            } else {
                self.env_rel
            };
            self.env = flush(x + (self.env - x) * c);

            self.lfo_smooth.set(self.lfo.at(self.shape, 0.0));
            let lfo = self.lfo_smooth.next();
            self.lfo.advance(inc);

            let cutoff = self.cutoff.next();
            let res = self.res.next();
            let depth = self.depth.next();
            let env_amt = self.env_amt.next();
            let m = self.mix.next();
            // Envelope saturates around 0.5 peak so typical levels use the full range.
            let env = (self.env * 2.0).min(1.0);
            let log_fc = (cutoff + depth * lfo + env_amt * env).clamp(4.33, max_log);
            let g = fast_tan(PI * fast_pow2(log_fc) / self.sr);
            self.svf_l.set_g(g, res);
            self.svf_r.set_g(g, res);
            let wl = self.svf_l.process_mode(a, self.mode);
            let wr = self.svf_r.process_mode(b, self.mode);
            *xl = mix(a, wl, m);
            *xr = mix(b, wr, m);
        }
    }

    fn reset(&mut self) {
        self.svf_l.reset();
        self.svf_r.reset();
        self.lfo.reset();
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(AutoFilter::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }
}
