//! "Chorus": two modulated delay taps per channel, LFOs offset by 90° between
//! channels for a wide stereo image, with optional feedback (flanger-ish).

use super::{Lfo, mix_send};
use crate::dsp::util::{DelayLine, Smoother};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};

const P_RATE: usize = 0;
const P_DEPTH: usize = 1;
const P_DELAY: usize = 2;
const P_FEEDBACK: usize = 3;
const P_MIX: usize = 4;

/// Maximum modulation excursion (ms) at depth 100%.
const MAX_DEPTH_MS: f32 = 6.0;
const MAX_DELAY_MS: f32 = 30.0;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "chorus",
    name: "Chorus",
    category: "Modulation",
    description: "Stereo chorus with feedback, from subtle shimmer to seasick flange.",
    params: &[
        ParamSpec::log("rate", "Rate", 0.05, 5.0, 0.6, "Hz", "Modulation"),
        ParamSpec::lin("depth", "Depth", 0.0, 100.0, 50.0, "%", "Modulation"),
        ParamSpec::log("delay", "Delay", 1.0, 30.0, 8.0, "ms", "Modulation"),
        ParamSpec::lin("feedback", "Feedback", -90.0, 90.0, 0.0, "%", "Modulation"),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 50.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Silk Doubler",
            values: &[
                ("rate", 0.35),
                ("depth", 30.0),
                ("delay", 12.0),
                ("mix", 40.0),
            ],
        },
        Preset {
            name: "Eighties Shimmer",
            values: &[
                ("rate", 0.9),
                ("depth", 60.0),
                ("delay", 7.0),
                ("mix", 50.0),
            ],
        },
        Preset {
            name: "Jet Wash",
            values: &[
                ("rate", 0.15),
                ("depth", 70.0),
                ("delay", 2.0),
                ("feedback", 70.0),
                ("mix", 50.0),
            ],
        },
        Preset {
            name: "Wobble Tape",
            values: &[
                ("rate", 3.5),
                ("depth", 25.0),
                ("delay", 5.0),
                ("mix", 35.0),
            ],
        },
    ],
    note_names: &[],
};

pub struct Chorus {
    sr: f32,
    primed: bool,
    line_l: DelayLine,
    line_r: DelayLine,
    lfo: Lfo,
    rate: Smoother,
    depth: Smoother,
    delay: Smoother,
    feedback: Smoother,
    mix: Smoother,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let max = ((MAX_DELAY_MS + MAX_DEPTH_MS) * 0.001 * sr) as usize + 8;
        let mut c = Self {
            sr,
            primed: false,
            line_l: DelayLine::new(max),
            line_r: DelayLine::new(max),
            lfo: Lfo::new(7),
            rate: Smoother::new(0.6, 50.0, sr),
            depth: Smoother::new(0.5, 50.0, sr),
            delay: Smoother::new(8.0, 100.0, sr),
            feedback: Smoother::new(0.0, 30.0, sr),
            mix: Smoother::new(0.5, 20.0, sr),
        };
        c.set_params(&SPEC.default_values());
        c.primed = false;
        c
    }
}

impl Effect for Chorus {
    fn set_params(&mut self, v: &ParamValues) {
        let t = [
            v[P_RATE],
            (v[P_DEPTH] * 0.01).clamp(0.0, 1.0),
            v[P_DELAY].clamp(1.0, MAX_DELAY_MS),
            (v[P_FEEDBACK] * 0.01).clamp(-0.9, 0.9),
            (v[P_MIX] * 0.01).clamp(0.0, 1.0),
        ];
        let ss = [
            &mut self.rate,
            &mut self.depth,
            &mut self.delay,
            &mut self.feedback,
            &mut self.mix,
        ];
        for (s, t) in ss.into_iter().zip(t) {
            if self.primed { s.set(t) } else { s.snap(t) }
        }
        self.primed = true;
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let ms = 0.001 * self.sr;
        let inv_sr = 1.0 / self.sr;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let (a, b) = (*xl, *xr);
            let rate = self.rate.next();
            let depth = self.depth.next() * MAX_DEPTH_MS * ms;
            let center = self.delay.next() * ms;
            let fb = self.feedback.next();
            let m = self.mix.next();
            // Excursion never goes below ~0.5 ms.
            let depth = depth.min(center - 0.5 * ms).max(0.0);
            let tap =
                |line: &DelayLine, off: f32, lfo: &Lfo| line.read(center + depth * lfo.at(0, off));
            let wl = 0.5 * (tap(&self.line_l, 0.0, &self.lfo) + tap(&self.line_l, 0.5, &self.lfo));
            let wr =
                0.5 * (tap(&self.line_r, 0.25, &self.lfo) + tap(&self.line_r, 0.75, &self.lfo));
            self.lfo.advance(rate * inv_sr);
            self.line_l.push(a + wl * fb);
            self.line_r.push(b + wr * fb);
            *xl = mix_send(a, wl, m);
            *xr = mix_send(b, wr, m);
        }
    }

    fn reset(&mut self) {
        self.line_l.clear();
        self.line_r.clear();
        self.lfo.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Chorus::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }
}
