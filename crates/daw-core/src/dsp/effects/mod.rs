//! Built-in effects. Each effect lives in its own file and exposes a static
//! `SPEC` plus a struct implementing [`Effect`]; this module lists them and
//! holds a few helpers they share.

use super::util::{Noise, lfo_value};
use super::{DeviceSpec, Effect};

pub mod chorus;
pub mod compressor;
pub mod delay;
pub mod drive;
pub mod eq;
pub mod filter;
pub mod limiter;
pub mod phaser;
pub mod reverb;
pub mod tremolo;
pub mod width;

pub static SPECS: &[&DeviceSpec] = &[
    &eq::SPEC,
    &compressor::SPEC,
    &delay::SPEC,
    &reverb::SPEC,
    &chorus::SPEC,
    &phaser::SPEC,
    &drive::SPEC,
    &filter::SPEC,
    &limiter::SPEC,
    &width::SPEC,
    &tremolo::SPEC,
];

pub fn create(type_id: &str, sample_rate: f32) -> Option<Box<dyn Effect>> {
    let sr = if sample_rate.is_finite() && sample_rate > 1000.0 {
        sample_rate
    } else {
        48000.0
    };
    Some(match type_id {
        "eq" => Box::new(eq::ChannelEq::new(sr)),
        "compressor" => Box::new(compressor::Compressor::new(sr)),
        "delay" => Box::new(delay::Echo::new(sr)),
        "reverb" => Box::new(reverb::Space::new(sr)),
        "chorus" => Box::new(chorus::Chorus::new(sr)),
        "phaser" => Box::new(phaser::Phaser::new(sr)),
        "drive" => Box::new(drive::Drive::new(sr)),
        "filter" => Box::new(filter::AutoFilter::new(sr)),
        "limiter" => Box::new(limiter::Limiter::new(sr)),
        "width" => Box::new(width::StereoWidth::new(sr)),
        "tremolo" => Box::new(tremolo::Tremolo::new(sr)),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Dry/wet crossfade; exactly `dry` when `m == 0`.
#[inline]
pub(crate) fn mix(dry: f32, wet: f32, m: f32) -> f32 {
    dry + (wet - dry) * m
}

/// Dry/wet balance for time-based effects (chorus, phaser, echo): the dry
/// signal stays at full level up to 50 % while the wet one fades in, then
/// the dry fades out. A plain crossfade loses ~5 dB at 50 % because the wet
/// signal is decorrelated from the dry one; this keeps the level steady.
/// Exactly `dry` when `m == 0`.
#[inline]
pub(crate) fn mix_send(dry: f32, wet: f32, m: f32) -> f32 {
    dry * (2.0 - 2.0 * m).min(1.0) + wet * (2.0 * m).min(1.0)
}

/// Rounds a choice / int parameter value to an index.
#[inline]
pub(crate) fn idx(v: f32) -> usize {
    if v.is_finite() {
        v.round().max(0.0) as usize
    } else {
        0
    }
}

/// Toggle parameter value as a bool.
#[inline]
pub(crate) fn on(v: f32) -> bool {
    v >= 0.5
}

/// Cheap log2 approximation (abs. error < 0.01) for level detection.
#[inline]
pub(crate) fn fast_log2(x: f32) -> f32 {
    let x = x.max(1.0e-30);
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as f32 - 127.0;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
    // atanh series: ln(m) = 2(t + t³/3 + t⁵/5), t = (m-1)/(m+1) in [0, 1/3].
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    e + 2.885_39 * t * (1.0 + t2 * (0.333_333_3 + t2 * 0.2))
}

/// Linear gain -> dB (fast).
#[inline]
pub(crate) fn fast_gain_to_db(g: f32) -> f32 {
    6.020_6 * fast_log2(g)
}

/// dB -> linear gain (fast).
#[inline]
pub(crate) fn fast_db_to_gain(db: f32) -> f32 {
    super::util::fast_pow2(db * 0.166_096_4)
}

/// Free-running LFO with phase accumulator and sample & hold.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lfo {
    pub phase: f32,
    sh: f32,
    noise: Noise,
}

impl Lfo {
    pub fn new(seed: u32) -> Self {
        Self {
            phase: 0.0,
            sh: 0.0,
            noise: Noise::new(seed),
        }
    }

    /// Value at `phase + offset` (offset in cycles) without advancing.
    #[inline]
    pub fn at(&self, shape: usize, offset: f32) -> f32 {
        let mut p = self.phase + offset;
        if p >= 1.0 {
            p -= 1.0;
        }
        if shape == 0 {
            // Sine via the fast approximation (cheaper than sin()).
            super::util::fast_sin01(p)
        } else {
            lfo_value(shape, p, self.sh)
        }
    }

    /// Advances by `inc` cycles; draws a new S&H value on wrap.
    #[inline]
    pub fn advance(&mut self, inc: f32) {
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.sh = self.noise.next();
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.sh = 0.0;
    }
}

/// Shared checks run by every effect's unit tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::super::util::test_support::*;
    use super::super::{DeviceSpec, Effect, ParamValues};

    pub type Make = fn(f32) -> Box<dyn Effect>;

    fn make_with(make: Make, sr: f32, v: &ParamValues) -> Box<dyn Effect> {
        let mut fx = make(sr);
        fx.set_params(v);
        fx
    }

    fn impulse(frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        l[10] = 1.0;
        r[10] = 1.0;
        (l, r)
    }

    /// Spec validity, finiteness at 48k / 44.1k, presets, block-size
    /// invariance and (when `mix_id` is given) transparency at mix = 0.
    pub fn standard_checks(spec: &DeviceSpec, make: Make, mix_id: Option<&str>) {
        check_spec(spec);
        let defaults = spec.default_values();
        for sr in [48000.0f32, 44100.0] {
            let frames = sr as usize;
            // Test signal and impulse with defaults.
            let (mut l, mut r) = test_signal(sr, frames);
            run_effect(
                make_with(make, sr, &defaults).as_mut(),
                sr,
                &mut l,
                &mut r,
                512,
            );
            assert_finite(&l, spec.type_id);
            assert_finite(&r, spec.type_id);
            assert!(
                peak(&l).max(peak(&r)) < 2.0,
                "{}: default output too loud",
                spec.type_id
            );
            let (mut l, mut r) = impulse(frames);
            run_effect(
                make_with(make, sr, &defaults).as_mut(),
                sr,
                &mut l,
                &mut r,
                256,
            );
            assert_finite(&l, spec.type_id);
            assert_finite(&r, spec.type_id);

            // Every preset renders cleanly.
            for preset in spec.presets {
                let v = preset_values(spec, preset);
                let (mut l, mut r) = test_signal(sr, frames);
                run_effect(make_with(make, sr, &v).as_mut(), sr, &mut l, &mut r, 512);
                assert_finite(&l, preset.name);
                assert_finite(&r, preset.name);
                let p = peak(&l).max(peak(&r));
                assert!(p < 1.5, "{} / {}: peak {p}", spec.type_id, preset.name);
                let (mut l, mut r) = impulse(frames);
                run_effect(make_with(make, sr, &v).as_mut(), sr, &mut l, &mut r, 64);
                assert_finite(&l, preset.name);
                assert_finite(&r, preset.name);
            }

            // mix = 0 is transparent.
            if let Some(id) = mix_id {
                let v = with(spec, defaults, id, 0.0);
                let (dl, dr) = test_signal(sr, frames);
                let (mut l, mut r) = (dl.clone(), dr.clone());
                run_effect(make_with(make, sr, &v).as_mut(), sr, &mut l, &mut r, 512);
                for i in 0..frames {
                    assert!(
                        (l[i] - dl[i]).abs() < 1e-4,
                        "{}: mix 0 not transparent (L @ {i})",
                        spec.type_id
                    );
                    assert!(
                        (r[i] - dr[i]).abs() < 1e-4,
                        "{}: mix 0 not transparent (R @ {i})",
                        spec.type_id
                    );
                }
            }
        }

        // Block sizes 1, 7 and 512 give the same result.
        let sr = 48000.0;
        let frames = 12000;
        let render = |block: usize| {
            let (mut l, mut r) = test_signal(sr, frames);
            run_effect(
                make_with(make, sr, &defaults).as_mut(),
                sr,
                &mut l,
                &mut r,
                block,
            );
            (l, r)
        };
        let (rl, rr) = render(512);
        for block in [1usize, 7] {
            let (l, r) = render(block);
            for i in 0..frames {
                assert!(
                    (l[i] - rl[i]).abs() < 1e-4,
                    "{}: block {block} differs at {i}",
                    spec.type_id
                );
                assert!(
                    (r[i] - rr[i]).abs() < 1e-4,
                    "{}: block {block} differs at {i}",
                    spec.type_id
                );
            }
        }
    }

    /// Feeds a burst, changes one param mid-tail and checks the tail survives.
    pub fn tail_survives_param_change(spec: &DeviceSpec, make: Make, id: &str, value: f32) {
        let sr = 48000.0;
        let defaults = with(spec, spec.default_values(), "mix", 100.0);
        let mut fx = make_with(make, sr, &defaults);
        let (mut l, mut r) = test_signal(sr, 4800);
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 128);
        let mut l = vec![0.0; 2400];
        let mut r = vec![0.0; 2400];
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 128);
        fx.set_params(&with(spec, defaults, id, value));
        let mut l = vec![0.0; 24000];
        let mut r = vec![0.0; 24000];
        run_effect(fx.as_mut(), sr, &mut l, &mut r, 128);
        assert!(
            peak(&l).max(peak(&r)) > 1e-3,
            "{}: tail lost after changing {id}",
            spec.type_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent() {
        for spec in SPECS {
            assert_eq!(spec.kind, super::super::DeviceKind::Effect);
            assert!(
                create(spec.type_id, 48000.0).is_some(),
                "{} not creatable",
                spec.type_id
            );
        }
        assert!(create("nope", 48000.0).is_none());
    }

    #[test]
    fn fast_math_is_close() {
        for i in 1..2000 {
            let g = i as f32 / 500.0;
            assert!((fast_log2(g) - g.log2()).abs() < 0.01, "log2 {g}");
            let db = i as f32 / 20.0 - 60.0;
            assert!((fast_db_to_gain(db) / 10f32.powf(db / 20.0) - 1.0).abs() < 1e-3);
        }
    }
}
