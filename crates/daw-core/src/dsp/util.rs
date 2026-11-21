//! Small reusable DSP helpers.

use std::f32::consts::PI;

/// One-pole parameter smoother (avoids zipper noise on knob moves).
#[derive(Debug, Clone, Copy)]
pub struct Smoother {
    pub value: f32,
    pub target: f32,
    coef: f32,
}

impl Smoother {
    pub fn new(value: f32, time_ms: f32, sr: f32) -> Self {
        let mut s = Self { value, target: value, coef: 0.0 };
        s.set_time(time_ms, sr);
        s
    }
    pub fn set_time(&mut self, time_ms: f32, sr: f32) {
        let samples = (time_ms * 0.001 * sr).max(1.0);
        self.coef = (-1.0 / samples).exp();
    }
    #[inline]
    pub fn set(&mut self, target: f32) {
        self.target = target;
    }
    #[inline]
    pub fn snap(&mut self, v: f32) {
        self.value = v;
        self.target = v;
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.value = self.target + (self.value - self.target) * self.coef;
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BiquadType {
    LowPass,
    HighPass,
    BandPass,
    Peak,
    LowShelf,
    HighShelf,
    Notch,
}

/// RBJ cookbook biquad, transposed direct form II.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    pub fn set(&mut self, kind: BiquadType, freq: f32, q: f32, gain_db: f32, sr: f32) {
        let freq = freq.clamp(10.0, sr * 0.49);
        let q = q.max(0.05);
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sr;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadType::LowPass => {
                let b1 = 1.0 - cs;
                (b1 / 2.0, b1, b1 / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
            }
            BiquadType::HighPass => {
                let b1 = -(1.0 + cs);
                ((1.0 + cs) / 2.0, b1, (1.0 + cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
            }
            BiquadType::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            BiquadType::Notch => (1.0, -2.0 * cs, 1.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            BiquadType::Peak => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            BiquadType::LowShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cs + sq),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - sq),
                    (a + 1.0) + (a - 1.0) * cs + sq,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - sq,
                )
            }
            BiquadType::HighShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - sq),
                    (a + 1.0) - (a - 1.0) * cs + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - sq,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Fast xorshift noise generator, audio-thread safe.
#[derive(Debug, Clone, Copy)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    /// Uniform in [-1, 1).
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Equal-power pan law. `pan` in [-1, 1].
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (p.cos() * std::f32::consts::SQRT_2, p.sin() * std::f32::consts::SQRT_2)
}

/// Gentle soft clipper (tanh-like, cheap).
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x > 3.0 {
        1.0
    } else if x < -3.0 {
        -1.0
    } else {
        x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
    }
}

// ---------------------------------------------------------------------------
// Helpers shared by the built-in instruments and effects.
// ---------------------------------------------------------------------------

/// Flushes values that are so small they could become denormals.
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-15 { 0.0 } else { x }
}

/// Cheap 2^x approximation (max rel. error ~1e-4), good enough for pitch /
/// cutoff modulation in the audio loop. Valid for roughly |x| < 30.
#[inline]
pub fn fast_pow2(x: f32) -> f32 {
    let x = x.clamp(-30.0, 30.0);
    let xi = x.round();
    let f = x - xi;
    // Taylor polynomial of 2^f on [-0.5, 0.5] (rel. error < 5e-5).
    let p = 1.0 + f * (std::f32::consts::LN_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * 0.009_618_1)));
    p * f32::from_bits((((xi as i32) + 127) as u32) << 23)
}

/// Pitch ratio for a value in semitones (fast).
#[inline]
pub fn semis_to_ratio(semis: f32) -> f32 {
    fast_pow2(semis * (1.0 / 12.0))
}

/// Cheap tan() for the TPT filter prewarp, accurate on [0, ~1.5].
#[inline]
pub fn fast_tan(x: f32) -> f32 {
    // Padé (7/6) approximant: x(945 - 105x² + x⁴) / (945 - 420x² + 15x⁴).
    let x2 = x * x;
    x * (1.0 + x2 * (-0.111_111_1 + x2 * 0.001_058_2)) / (1.0 + x2 * (-0.444_444_4 + x2 * 0.015_873_0))
}

/// Mode of the state-variable filter output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvfMode {
    LowPass,
    BandPass,
    HighPass,
}

/// Zero-delay-feedback (TPT) state-variable filter (A. Simper's design).
/// Stable for any cutoff below Nyquist and any resonance in [0, 1).
#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    /// `res` in [0, 1]: 0 = no resonance (Q 0.5), 1 = near self-oscillation.
    #[inline]
    pub fn set(&mut self, cutoff: f32, res: f32, sr: f32) {
        let fc = cutoff.clamp(10.0, sr * 0.45);
        let g = fast_tan(PI * fc / sr);
        self.set_g(g, res);
    }

    /// Same as `set` with a precomputed `g = tan(pi * fc / sr)`.
    #[inline]
    pub fn set_g(&mut self, g: f32, res: f32) {
        let k = 2.0 - 1.96 * res.clamp(0.0, 1.0);
        self.k = k;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Processes one sample, returning (low, band, high).
    #[inline]
    pub fn process(&mut self, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, v1, x - self.k * v1 - v2)
    }

    #[inline]
    pub fn process_mode(&mut self, x: f32, mode: SvfMode) -> f32 {
        let (lp, bp, hp) = self.process(x);
        match mode {
            SvfMode::LowPass => lp,
            SvfMode::BandPass => bp * self.k.max(0.3),
            SvfMode::HighPass => hp,
        }
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// One-pole low-pass / high-pass (6 dB/oct), cheap tone control.
#[derive(Debug, Clone, Copy, Default)]
pub struct OnePole {
    a: f32,
    z: f32,
}

impl OnePole {
    pub fn set(&mut self, cutoff: f32, sr: f32) {
        let fc = cutoff.clamp(1.0, sr * 0.49);
        self.a = 1.0 - (-2.0 * PI * fc / sr).exp();
    }
    #[inline]
    pub fn lp(&mut self, x: f32) -> f32 {
        self.z = flush(self.z + self.a * (x - self.z));
        self.z
    }
    #[inline]
    pub fn hp(&mut self, x: f32) -> f32 {
        x - self.lp(x)
    }
    pub fn reset(&mut self) {
        self.z = 0.0;
    }
}

/// ADSR envelope: linear attack, exponential decay and release.
/// Times are in seconds, sustain in [0, 1].
#[derive(Debug, Clone, Copy)]
pub struct Adsr {
    stage: AdsrStage,
    pub level: f32,
    attack_inc: f32,
    decay_coef: f32,
    release_coef: f32,
    sustain: f32,
    /// Fast fade used by `kill()`.
    kill_coef: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdsrStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
    Kill,
}

/// One-pole coefficient so that a value decays by ~60 dB... (to 1/e^4.6) in `secs`.
#[inline]
pub fn decay_coef(secs: f32, sr: f32) -> f32 {
    let n = (secs * sr).max(1.0);
    (-6.9 / n).exp()
}

impl Adsr {
    pub fn new(sr: f32) -> Self {
        let mut e = Self {
            stage: AdsrStage::Idle,
            level: 0.0,
            attack_inc: 1.0,
            decay_coef: 0.0,
            release_coef: 0.0,
            sustain: 1.0,
            kill_coef: 0.0,
        };
        e.set(0.005, 0.2, 0.7, 0.3, sr);
        e.kill_coef = decay_coef(0.003, sr);
        e
    }

    /// Release and decay times are the time to fall ~60 dB.
    pub fn set(&mut self, attack: f32, decay: f32, sustain: f32, release: f32, sr: f32) {
        self.attack_inc = 1.0 / (attack * sr).max(1.0);
        self.decay_coef = decay_coef(decay, sr);
        self.release_coef = decay_coef(release, sr);
        self.sustain = sustain.clamp(0.0, 1.0);
    }

    pub fn gate_on(&mut self) {
        self.stage = AdsrStage::Attack;
    }
    /// Restart from zero (used for fresh voices).
    pub fn trigger(&mut self) {
        self.level = 0.0;
        self.stage = AdsrStage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.stage != AdsrStage::Idle && self.stage != AdsrStage::Kill {
            self.stage = AdsrStage::Release;
        }
    }
    /// Quick ~3 ms fade to silence.
    pub fn kill(&mut self) {
        if self.stage != AdsrStage::Idle {
            self.stage = AdsrStage::Kill;
        }
    }
    pub fn reset(&mut self) {
        self.stage = AdsrStage::Idle;
        self.level = 0.0;
    }
    #[inline]
    pub fn stage(&self) -> AdsrStage {
        self.stage
    }
    #[inline]
    pub fn is_active(&self) -> bool {
        self.stage != AdsrStage::Idle
    }
    #[inline]
    pub fn is_releasing(&self) -> bool {
        matches!(self.stage, AdsrStage::Release | AdsrStage::Kill)
    }

    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            AdsrStage::Idle => {}
            AdsrStage::Attack => {
                self.level += self.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = AdsrStage::Decay;
                }
            }
            AdsrStage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if self.level - self.sustain < 1.0e-4 {
                    self.level = self.sustain;
                    self.stage = AdsrStage::Sustain;
                }
            }
            AdsrStage::Sustain => {
                // Follow sustain changes smoothly.
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if self.level < 1.0e-4 && self.sustain <= 0.0 {
                    self.reset();
                }
            }
            AdsrStage::Release => {
                self.level *= self.release_coef;
                if self.level < 1.0e-4 {
                    self.reset();
                }
            }
            AdsrStage::Kill => {
                self.level *= self.kill_coef;
                if self.level < 1.0e-4 {
                    self.reset();
                }
            }
        }
        self.level
    }
}

/// LFO waveforms shared by devices.
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "S&H"];

/// Evaluates an LFO shape at `phase` in [0, 1). Output in [-1, 1].
/// `sh` is the current sample-and-hold value (updated by the caller on wrap).
#[inline]
pub fn lfo_value(shape: usize, phase: f32, sh: f32) -> f32 {
    match shape {
        0 => (phase * 2.0 * PI).sin(),
        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
        2 => 2.0 * phase - 1.0,
        3 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => sh,
    }
}

/// Tempo-sync note values (shared by delay, LFOs, tremolo...).
pub const SYNC_NAMES: &[&str] = &[
    "1/32", "1/16T", "1/16", "1/16D", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D", "1/2T", "1/2", "1/2D", "1/1",
    "2/1", "4/1",
];
/// Length in beats (quarter notes) of each `SYNC_NAMES` entry.
pub const SYNC_BEATS: &[f32] = &[
    0.125,
    1.0 / 6.0,
    0.25,
    0.375,
    1.0 / 3.0,
    0.5,
    0.75,
    2.0 / 3.0,
    1.0,
    1.5,
    4.0 / 3.0,
    2.0,
    3.0,
    4.0,
    8.0,
    16.0,
];

/// Duration in seconds of the sync note value `idx` at `bpm`.
#[inline]
pub fn sync_seconds(idx: usize, bpm: f32) -> f32 {
    let beats = SYNC_BEATS[idx.min(SYNC_BEATS.len() - 1)];
    beats * 60.0 / bpm.clamp(20.0, 999.0)
}

/// Fixed-capacity delay line with fractional (Hermite) reads.
#[derive(Debug, Clone)]
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    /// Allocates room for `max_samples` of delay (plus interpolation guard).
    pub fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4) + 4], pos: 0 }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    /// Max usable fractional delay in samples.
    #[inline]
    pub fn max_delay(&self) -> f32 {
        (self.buf.len() - 3) as f32
    }
    #[inline]
    pub fn push(&mut self, x: f32) {
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.pos = 0;
        }
        self.buf[self.pos] = flush(x);
    }
    /// Integer delay read (0 = most recently pushed sample).
    #[inline]
    pub fn tap(&self, delay: usize) -> f32 {
        let n = self.buf.len();
        let d = delay.min(n - 1);
        self.buf[(self.pos + n - d) % n]
    }
    /// Fractional delay read with 4-point Hermite interpolation.
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, self.max_delay());
        let di = d as usize;
        let f = d - di as f32;
        let xm1 = self.tap(di - 1);
        let x0 = self.tap(di);
        let x1 = self.tap(di + 1);
        let x2 = self.tap(di + 2);
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * f + c2) * f + c1) * f + x0
    }
    /// Linear interpolated read (cheaper).
    #[inline]
    pub fn read_linear(&self, delay: f32) -> f32 {
        let d = delay.clamp(0.0, self.max_delay());
        let di = d as usize;
        let f = d - di as f32;
        let a = self.tap(di);
        let b = self.tap(di + 1);
        a + (b - a) * f
    }
    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }
}

/// DC blocker (first-order high-pass around 10–20 Hz).
#[derive(Debug, Clone, Copy)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
    r: f32,
}

impl DcBlocker {
    pub fn new(sr: f32) -> Self {
        Self { x1: 0.0, y1: 0.0, r: 1.0 - (2.0 * PI * 12.0 / sr) }
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        y
    }
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

/// Polynomial band-limited step correction (PolyBLEP).
/// `t` = phase in [0, 1), `dt` = phase increment per sample.
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Cheap sine approximation for phase in [0, 1) (parabolic, refined).
/// Max error ~0.001. Use where `sin()` per sample is too costly.
#[inline]
pub fn fast_sin01(phase: f32) -> f32 {
    // Map to [-pi, pi] domain as x in [-1, 1).
    let x = 2.0 * (phase - phase.floor()) - 1.0;
    // sin(pi * x) with sign flipped since x = phase*2 - 1 shifts by pi.
    let y = 4.0 * x * (1.0 - x.abs());
    -(0.225 * (y * y.abs() - y) + y)
}

/// Zero-delay-feedback transistor-ladder low-pass (V. Zavalishin's TPT
/// form) with a soft-clipped input stage. `poles` = 4 gives the classic
/// 24 dB/oct ladder, 3 the rounder 18 dB/oct "diode box" response used by
/// acid bass machines. Resonance in [0, 1] self-oscillates near 1.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ladder {
    s: [f32; 4],
    g: f32,
    k: f32,
}

impl Ladder {
    #[inline]
    pub fn set(&mut self, cutoff: f32, res: f32, sr: f32) {
        let fc = cutoff.clamp(10.0, sr * 0.45);
        let g = fast_tan(PI * fc / sr);
        self.g = g / (1.0 + g);
        self.k = res.clamp(0.0, 1.0) * 3.9;
    }

    /// Processes one sample. `drive` > 0 boosts the input into the clipper.
    #[inline]
    pub fn process(&mut self, x: f32, poles: usize, drive: f32) -> f32 {
        let gg = self.g;
        let b = 1.0 - gg; // = 1 / (1 + g): weight of each stage's state
        let n = poles.clamp(3, 4);
        // Instantaneous response of the cascade: y_n = G^n·u + Σ.
        let mut gn = 1.0;
        let mut sigma = 0.0;
        for i in 0..n {
            sigma = sigma * gg + self.s[i] * b;
            gn *= gg;
        }
        // Resonance gain compensation keeps the bass from thinning out.
        let comp = 1.0 + self.k * 0.45;
        let xin = soft_clip(x * (1.0 + drive) * comp) ;
        let u = soft_clip((xin - self.k * sigma) / (1.0 + self.k * gn));
        let mut y = u;
        for i in 0..n {
            let v = (y - self.s[i]) * gg;
            let out = v + self.s[i];
            self.s[i] = flush(out + v);
            y = out;
        }
        y
    }

    pub fn reset(&mut self) {
        self.s = [0.0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximations_are_accurate() {
        for i in 0..1000 {
            let x = i as f32 / 1000.0 * 1.45;
            assert!((fast_tan(x) - x.tan()).abs() / x.tan().max(1e-3) < 2e-3, "tan {x}");
            let p = i as f32 / 1000.0;
            assert!((fast_sin01(p) - (p * 2.0 * PI).sin()).abs() < 2e-3, "sin {p}");
            let e = i as f32 / 50.0 - 10.0;
            assert!((fast_pow2(e) / e.exp2() - 1.0).abs() < 2e-4, "pow2 {e}");
        }
    }

    #[test]
    fn svf_is_stable_at_extremes() {
        let mut f = Svf::default();
        let mut n = Noise::new(3);
        for &fc in &[10.0, 1000.0, 20000.0, 30000.0] {
            f.set(fc, 1.0, 44100.0);
            for _ in 0..48000 {
                let (l, b, h) = f.process(n.next());
                assert!(l.is_finite() && b.is_finite() && h.is_finite());
                assert!(l.abs() < 1000.0);
            }
        }
    }

    #[test]
    fn delay_line_reads_back() {
        let mut d = DelayLine::new(100);
        for i in 0..50 {
            d.push(i as f32);
        }
        assert_eq!(d.tap(0), 49.0);
        assert_eq!(d.tap(10), 39.0);
        assert!((d.read(10.5) - 38.5).abs() < 1e-4);
    }
}

/// Helpers shared by the device unit tests.
#[cfg(test)]
pub mod test_support {
    use crate::dsp::{DeviceSpec, Effect, Instrument, MAX_PARAMS, ParamKind, ParamValues, Preset, ProcessCtx};

    pub fn ctx(sample_rate: f32) -> ProcessCtx {
        ProcessCtx { sample_rate, bpm: 120.0, playing: true, beat: 0.0 }
    }

    /// Structural checks every device spec must pass.
    pub fn check_spec(spec: &DeviceSpec) {
        assert!(spec.params.len() <= MAX_PARAMS, "{}: too many params", spec.type_id);
        for (i, p) in spec.params.iter().enumerate() {
            assert!(p.min <= p.default && p.default <= p.max, "{}: default of {} out of range", spec.type_id, p.id);
            assert!(p.min < p.max, "{}: empty range for {}", spec.type_id, p.id);
            if matches!(p.kind, ParamKind::Log) {
                assert!(p.min > 0.0, "{}: log param {} must be > 0", spec.type_id, p.id);
            }
            if matches!(p.kind, ParamKind::Choice) {
                assert!(!p.choices.is_empty());
            }
            assert!(
                spec.params[..i].iter().all(|q| q.id != p.id),
                "{}: duplicate param id {}",
                spec.type_id,
                p.id
            );
        }
        for preset in spec.presets {
            for (id, v) in preset.values {
                let p = spec.params.iter().find(|p| p.id == *id);
                let p = p.unwrap_or_else(|| panic!("{}: preset {} uses unknown id {}", spec.type_id, preset.name, id));
                assert!(
                    *v >= p.min && *v <= p.max,
                    "{}: preset {} value {} = {} out of range",
                    spec.type_id,
                    preset.name,
                    id,
                    v
                );
            }
        }
    }

    pub fn preset_values(spec: &DeviceSpec, preset: &Preset) -> ParamValues {
        let mut v = spec.default_values();
        for (id, x) in preset.values {
            if let Some(i) = spec.index_of(id) {
                v[i] = *x;
            }
        }
        v
    }

    /// Value array with one param overridden.
    pub fn with(spec: &DeviceSpec, mut v: ParamValues, id: &str, x: f32) -> ParamValues {
        let i = spec.index_of(id).unwrap_or_else(|| panic!("unknown param {id}"));
        v[i] = x;
        v
    }

    /// Renders `frames` frames of an instrument in blocks of `block` frames.
    pub fn render(inst: &mut dyn Instrument, sr: f32, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0; frames];
        let mut r = vec![0.0; frames];
        let c = ctx(sr);
        let mut i = 0;
        while i < frames {
            let n = block.min(frames - i);
            inst.process(&c, &mut l[i..i + n], &mut r[i..i + n]);
            i += n;
        }
        (l, r)
    }

    /// Processes the buffers through an effect in blocks of `block` frames.
    pub fn run_effect(fx: &mut dyn Effect, sr: f32, l: &mut [f32], r: &mut [f32], block: usize) {
        let c = ctx(sr);
        let mut i = 0;
        while i < l.len() {
            let n = block.min(l.len() - i);
            fx.process(&c, &mut l[i..i + n], &mut r[i..i + n]);
            i += n;
        }
    }

    pub fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    pub fn assert_finite(buf: &[f32], what: &str) {
        assert!(buf.iter().all(|x| x.is_finite()), "{what}: non-finite sample");
    }

    /// Deterministic test signal: noise + a sine, roughly -6 dBFS.
    pub fn test_signal(sr: f32, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut n = super::Noise::new(12345);
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        for i in 0..frames {
            let s = (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / sr).sin() * 0.3;
            l.push(s + n.next() * 0.2);
            r.push(s * 0.8 + n.next() * 0.2);
        }
        (l, r)
    }
}
