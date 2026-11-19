//! "Echo": stereo / ping-pong delay with tempo sync, filtered feedback and a
//! smoothed read head (time changes glide like tape instead of crackling).

use super::{idx, mix, on};
use crate::dsp::util::{DelayLine, OnePole, SYNC_NAMES, Smoother, soft_clip, sync_seconds};
use crate::dsp::{DeviceKind, DeviceSpec, Effect, ParamSpec, ParamValues, Preset, ProcessCtx};

const P_SYNC: usize = 0;
const P_TIME_SYNC: usize = 1;
const P_TIME_MS: usize = 2;
const P_FEEDBACK: usize = 3;
const P_PINGPONG: usize = 4;
const P_LOWCUT: usize = 5;
const P_HIGHCUT: usize = 6;
const P_MIX: usize = 7;

/// Longest delay time in seconds.
const MAX_SECONDS: f32 = 2.0;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Effect,
    type_id: "delay",
    name: "Echo",
    category: "Delay",
    description: "Tempo-synced stereo / ping-pong echo with filtered feedback.",
    params: &[
        ParamSpec::toggle("sync", "Sync", true, "Time"),
        ParamSpec::choice("time_sync", "Note", SYNC_NAMES, 6, "Time"),
        ParamSpec::log("time_ms", "Time", 1.0, 2000.0, 350.0, "ms", "Time"),
        ParamSpec::lin("feedback", "Feedback", 0.0, 95.0, 35.0, "%", "Feedback"),
        ParamSpec::toggle("pingpong", "Ping-Pong", false, "Feedback"),
        ParamSpec::log("lowcut", "Low Cut", 20.0, 2000.0, 120.0, "Hz", "Feedback"),
        ParamSpec::log(
            "highcut", "High Cut", 500.0, 20000.0, 6000.0, "Hz", "Feedback",
        ),
        ParamSpec::lin("mix", "Mix", 0.0, 100.0, 30.0, "%", "Output"),
    ],
    presets: &[
        Preset {
            name: "Slapback",
            values: &[
                ("sync", 0.0),
                ("time_ms", 95.0),
                ("feedback", 10.0),
                ("highcut", 4500.0),
                ("mix", 30.0),
            ],
        },
        Preset {
            name: "Dotted Bounce",
            values: &[
                ("sync", 1.0),
                ("time_sync", 6.0),
                ("feedback", 45.0),
                ("pingpong", 1.0),
                ("mix", 28.0),
            ],
        },
        Preset {
            name: "Worn Tape",
            values: &[
                ("sync", 1.0),
                ("time_sync", 8.0),
                ("feedback", 60.0),
                ("lowcut", 300.0),
                ("highcut", 2200.0),
                ("mix", 35.0),
            ],
        },
        Preset {
            name: "Canyon",
            values: &[
                ("sync", 0.0),
                ("time_ms", 900.0),
                ("feedback", 70.0),
                ("pingpong", 1.0),
                ("highcut", 5000.0),
                ("mix", 30.0),
            ],
        },
    ],
    note_names: &[],
};

pub struct Echo {
    sr: f32,
    primed: bool,
    sync: bool,
    sync_idx: usize,
    time_ms: f32,
    pingpong: bool,
    /// Last tempo seen by `process` (used to prime the time smoother).
    bpm: f32,
    line_l: DelayLine,
    line_r: DelayLine,
    /// Delay time in samples, smoothed (tape-style glide).
    time: Smoother,
    feedback: Smoother,
    mix: Smoother,
    hp_l: OnePole,
    hp_r: OnePole,
    lp_l: OnePole,
    lp_r: OnePole,
}

impl Echo {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let max = (MAX_SECONDS * sr) as usize + 8;
        let mut e = Self {
            sr,
            primed: false,
            sync: true,
            sync_idx: 6,
            time_ms: 350.0,
            pingpong: false,
            bpm: 120.0,
            line_l: DelayLine::new(max),
            line_r: DelayLine::new(max),
            time: Smoother::new(0.35 * sr, 120.0, sr),
            feedback: Smoother::new(0.35, 20.0, sr),
            mix: Smoother::new(0.3, 20.0, sr),
            hp_l: OnePole::default(),
            hp_r: OnePole::default(),
            lp_l: OnePole::default(),
            lp_r: OnePole::default(),
        };
        e.set_params(&SPEC.default_values());
        e.primed = false;
        e
    }

    fn target_samples(&self, bpm: f32) -> f32 {
        let secs = if self.sync {
            sync_seconds(self.sync_idx, bpm)
        } else {
            self.time_ms * 0.001
        };
        (secs.min(MAX_SECONDS) * self.sr).clamp(1.0, self.line_l.max_delay() - 2.0)
    }
}

impl Effect for Echo {
    fn set_params(&mut self, v: &ParamValues) {
        self.sync = on(v[P_SYNC]);
        self.sync_idx = idx(v[P_TIME_SYNC]).min(SYNC_NAMES.len() - 1);
        self.time_ms = v[P_TIME_MS];
        self.pingpong = on(v[P_PINGPONG]);
        let sr = self.sr;
        self.hp_l.set(v[P_LOWCUT], sr);
        self.hp_r.set(v[P_LOWCUT], sr);
        self.lp_l.set(v[P_HIGHCUT], sr);
        self.lp_r.set(v[P_HIGHCUT], sr);
        let fb = (v[P_FEEDBACK] * 0.01).clamp(0.0, 0.95);
        let m = (v[P_MIX] * 0.01).clamp(0.0, 1.0);
        if self.primed {
            self.feedback.set(fb);
            self.mix.set(m);
        } else {
            self.feedback.snap(fb);
            self.mix.snap(m);
            self.time.snap(self.target_samples(self.bpm));
            self.primed = true;
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        self.bpm = ctx.bpm;
        self.time.set(self.target_samples(ctx.bpm));
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let (a, b) = (*xl, *xr);
            let d = self.time.next();
            let fb = self.feedback.next();
            let m = self.mix.next();
            let out_l = self.line_l.read(d);
            let out_r = self.line_r.read(d);
            // Feedback path: band-limit, then gently saturate to keep it tame.
            let fl = soft_clip(self.lp_l.lp(self.hp_l.hp(out_l)) * fb);
            let fr = soft_clip(self.lp_r.lp(self.hp_r.hp(out_r)) * fb);
            if self.pingpong {
                // Mono input enters the left line; the lines feed each other.
                self.line_l.push((a + b) * 0.5 + fr);
                self.line_r.push(fl);
            } else {
                self.line_l.push(a + fl);
                self.line_r.push(b + fr);
            }
            *xl = mix(a, out_l, m);
            *xr = mix(b, out_r, m);
        }
    }

    fn reset(&mut self) {
        self.line_l.clear();
        self.line_r.clear();
        for f in [
            &mut self.hp_l,
            &mut self.hp_r,
            &mut self.lp_l,
            &mut self.lp_r,
        ] {
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
        Box::new(Echo::new(sr))
    }

    #[test]
    fn standard() {
        standard_checks(&SPEC, make, Some("mix"));
    }

    #[test]
    fn tail_survives() {
        tail_survives_param_change(&SPEC, make, "highcut", 2000.0);
        tail_survives_param_change(&SPEC, make, "time_sync", 8.0);
    }

    #[test]
    fn echo_arrives_on_time() {
        let sr = 48000.0;
        let mut v = with(&SPEC, SPEC.default_values(), "sync", 0.0);
        v = with(&SPEC, v, "time_ms", 100.0);
        v = with(&SPEC, v, "mix", 100.0);
        v = with(&SPEC, v, "lowcut", 20.0);
        v = with(&SPEC, v, "highcut", 20000.0);
        let mut fx = make(sr);
        fx.set_params(&v);
        let mut l = vec![0.0; 9600];
        let mut r = vec![0.0; 9600];
        l[0] = 1.0;
        r[0] = 1.0;
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 512);
        let pos = l
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert!((pos as i32 - 4800).abs() <= 2, "echo at {pos}");
    }
}
