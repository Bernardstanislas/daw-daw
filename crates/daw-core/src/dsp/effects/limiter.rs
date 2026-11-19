//! "Limiter": lookahead brickwall limiter.
//!
//! For every input sample we compute the gain needed to keep it under the
//! ceiling, take the minimum over a window of `L` samples (the lookahead),
//! apply release smoothing (instant attack, exponential release) and finally
//! a moving average of length `L`. The audio is delayed by `L - 1` samples,
//! so the averaged gain has fully ramped down by the time a peak comes out
//! (see the proof sketch in `process`). A final clamp guarantees the ceiling.

use crate::dsp::util::Smoother;
use crate::dsp::{
    DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx, db_to_gain,
};

const P_INPUT: usize = 0;
const P_CEILING: usize = 1;
const P_RELEASE: usize = 2;

const LOOKAHEAD_MS: f32 = 3.0;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "limiter",
    name: "Limiter",
    category: "Dynamics",
    description: "Transparent lookahead brickwall limiter (3 ms latency).",
    params: &[
        ParamSpec::lin("input", "Input Gain", -12.0, 24.0, 0.0, "dB", "Limiter"),
        ParamSpec::lin("ceiling", "Ceiling", -24.0, 0.0, -0.3, "dB", "Limiter"),
        ParamSpec::log("release", "Release", 10.0, 1000.0, 100.0, "ms", "Limiter"),
    ],
    presets: &[
        Preset {
            name: "Safety Net",
            values: &[("input", 0.0), ("ceiling", -0.3), ("release", 150.0)],
        },
        Preset {
            name: "Loud and Proud",
            values: &[("input", 8.0), ("ceiling", -0.5), ("release", 60.0)],
        },
        Preset {
            name: "Gentle Master",
            values: &[("input", 3.0), ("ceiling", -1.0), ("release", 300.0)],
        },
    ],
    note_names: &[],
};

pub struct Limiter {
    sr: f32,
    primed: bool,
    /// Lookahead length in samples.
    len: usize,
    /// Delayed audio (ring buffers of `len` samples).
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    /// Required gain per sample over the window (ring buffer).
    req: Vec<f32>,
    /// Released gain values for the moving average (ring buffer).
    avg_buf: Vec<f32>,
    avg_sum: f64,
    pos: usize,
    /// Current window minimum and the age of that minimum.
    win_min: f32,
    win_age: usize,
    released: f32,
    rel_coef: f32,
    input: Smoother,
    ceiling: Smoother,
}

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let len = ((LOOKAHEAD_MS * 0.001 * sr) as usize).max(2);
        let mut lim = Self {
            sr,
            primed: false,
            len,
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            req: vec![1.0; len],
            avg_buf: vec![1.0; len],
            avg_sum: len as f64,
            pos: 0,
            win_min: 1.0,
            win_age: 0,
            released: 1.0,
            rel_coef: 0.0,
            input: Smoother::new(1.0, 20.0, sr),
            ceiling: Smoother::new(1.0, 20.0, sr),
        };
        lim.set_params(&SPEC.default_values());
        lim.primed = false;
        lim
    }

    /// Latency introduced by the lookahead, in samples.
    pub fn latency(&self) -> usize {
        self.len - 1
    }
}

impl Effect for Limiter {
    fn set_params(&mut self, v: &ParamValues) {
        self.rel_coef = (-1.0 / (v[P_RELEASE].max(1.0) * 0.001 * self.sr)).exp();
        let input = db_to_gain(v[P_INPUT]);
        let ceiling = db_to_gain(v[P_CEILING].min(0.0));
        if self.primed {
            self.input.set(input);
            self.ceiling.set(ceiling);
        } else {
            self.input.snap(input);
            self.ceiling.snap(ceiling);
            self.primed = true;
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = self.len;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let gin = self.input.next();
            let ceil = self.ceiling.next();
            let a = *xl * gin;
            let b = *xr * gin;
            let p = a.abs().max(b.abs());
            let need = if p > ceil { ceil / p } else { 1.0 };

            // Sliding-window minimum (rescan only when the minimum expires).
            self.req[self.pos] = need;
            self.win_age += 1;
            if need <= self.win_min {
                self.win_min = need;
                self.win_age = 0;
            } else if self.win_age >= n {
                let mut m = 1.0f32;
                let mut age = 0;
                for k in 0..n {
                    // k = 0 is the newest sample.
                    let v = self.req[(self.pos + n - k) % n];
                    if v < m {
                        m = v;
                        age = k;
                    }
                }
                self.win_min = m;
                self.win_age = age;
            }

            // Instant attack, exponential release (never above the hold).
            self.released = if self.win_min < self.released {
                self.win_min
            } else {
                self.win_min + (self.released - self.win_min) * self.rel_coef
            };

            // Moving average over `n` samples: every value it averages is a
            // window-min that covers the sample now leaving the delay line,
            // so the result is <= the gain that sample needs.
            let old = self.avg_buf[self.pos];
            self.avg_buf[self.pos] = self.released;
            self.avg_sum += self.released as f64 - old as f64;
            let g = (self.avg_sum / n as f64) as f32;

            // Delay line: read the oldest sample (n - 1 behind), write the new one.
            let rd = (self.pos + 1) % n;
            let (dl, dr) = (self.buf_l[rd], self.buf_r[rd]);
            self.buf_l[self.pos] = a;
            self.buf_r[self.pos] = b;
            self.pos = rd;
            if rd == 0 {
                // Once per lookahead window, recompute the running sum to
                // cancel accumulated float drift.
                self.avg_sum = self.avg_buf.iter().map(|&x| x as f64).sum();
            }

            *xl = (dl * g).clamp(-ceil, ceil);
            *xr = (dr * g).clamp(-ceil, ceil);
        }
    }

    fn reset(&mut self) {
        self.buf_l.iter_mut().for_each(|x| *x = 0.0);
        self.buf_r.iter_mut().for_each(|x| *x = 0.0);
        self.req.iter_mut().for_each(|x| *x = 1.0);
        self.avg_buf.iter_mut().for_each(|x| *x = 1.0);
        self.avg_sum = self.len as f64;
        self.win_min = 1.0;
        self.win_age = 0;
        self.released = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::dsp::util::test_support::*;

    fn make(sr: f32) -> Box<dyn Effect> {
        Box::new(Limiter::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, None);
    }

    #[test]
    fn respects_ceiling() {
        for sr in [44100.0f32, 48000.0] {
            for (input, ceiling) in [(24.0, -0.3), (12.0, -6.0), (0.0, -12.0)] {
                let v = with(
                    &SPEC,
                    with(&SPEC, SPEC.default_values(), "input", input),
                    "ceiling",
                    ceiling,
                );
                let mut fx = make(sr);
                fx.set_params(&v);
                let (mut l, mut r) = test_signal(sr, sr as usize);
                // Add some sharp transients.
                for i in (1000..l.len()).step_by(7919) {
                    l[i] = 1.0;
                    r[i] = -1.0;
                }
                run_effect(fx.as_mut(), sr, &mut l, &mut r, 333);
                let lim = db_to_gain(ceiling) * 1.0001;
                assert!(peak(&l).max(peak(&r)) <= lim, "ceiling {ceiling} exceeded");
            }
        }
    }

    #[test]
    fn quiet_signal_passes_delayed() {
        let sr = 48000.0;
        let mut fx = Limiter::new(sr);
        fx.set_params(&SPEC.default_values());
        let lat = fx.latency();
        let src: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin() * 0.25).collect();
        let (mut l, mut r) = (src.clone(), src.clone());
        run_effect(&mut fx, sr, &mut l, &mut r, 512);
        for i in lat..src.len() {
            assert!((l[i] - src[i - lat]).abs() < 1e-5, "at {i}");
        }
    }
}
