//! Spectrum analyzer for the UI: Hann-windowed FFT of the scope ring buffer,
//! folded into log-spaced bands (20 Hz – 20 kHz) in dBFS.

use super::shared::{SCOPE_LEN, Scope};

/// Number of bands sent to the UI.
pub const BANDS: usize = 160;
const F_LO: f32 = 20.0;
const F_HI: f32 = 20_000.0;
const FLOOR_DB: f32 = -120.0;

pub struct Analyzer {
    window: Vec<f32>,
    samples: Box<[f32; SCOPE_LEN]>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Twiddle factors for the radix-2 FFT.
    cos: Vec<f32>,
    sin: Vec<f32>,
    mag: Vec<f32>,
}

impl Default for Analyzer {
    fn default() -> Self {
        let n = SCOPE_LEN;
        let window = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
        let cos = (0..n / 2).map(|k| (std::f32::consts::TAU * k as f32 / n as f32).cos()).collect();
        let sin = (0..n / 2).map(|k| -(std::f32::consts::TAU * k as f32 / n as f32).sin()).collect();
        Self { window, samples: Box::new([0.0; SCOPE_LEN]), re: vec![0.0; n], im: vec![0.0; n], cos, sin, mag: vec![0.0; n / 2 + 1] }
    }
}

impl Analyzer {
    /// Band levels (dBFS, a full-scale sine reads ~0 dB) of the latest scope contents.
    pub fn analyze(&mut self, scope: &Scope, sample_rate: f32) -> Vec<f32> {
        scope.read(&mut self.samples);
        let n = SCOPE_LEN;
        for i in 0..n {
            self.re[i] = self.samples[i] * self.window[i];
            self.im[i] = 0.0;
        }
        self.fft();
        // Hann window: a full-scale sine peaks at n/4.
        let norm = 4.0 / n as f32;
        for k in 0..=n / 2 {
            self.mag[k] = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() * norm;
        }
        self.bands(sample_rate)
    }

    fn bands(&self, sample_rate: f32) -> Vec<f32> {
        let bin_hz = sample_rate / SCOPE_LEN as f32;
        let last = self.mag.len() - 1;
        let ratio = (F_HI / F_LO).ln();
        let edge = |b: usize| F_LO * (ratio * b as f32 / BANDS as f32).exp() / bin_hz;
        (0..BANDS)
            .map(|b| {
                let (lo, hi) = (edge(b), edge(b + 1));
                let v = if hi - lo < 1.0 {
                    // Narrower than a bin (low end): interpolate at the band centre.
                    let c = ((lo * hi).sqrt()).min(last as f32);
                    let i = c.floor() as usize;
                    let f = c - i as f32;
                    self.mag[i] * (1.0 - f) + self.mag[(i + 1).min(last)] * f
                } else {
                    let (i0, i1) = (lo.ceil() as usize, (hi.floor() as usize).min(last));
                    (i0..=i1.max(i0).min(last)).map(|i| self.mag[i]).fold(0.0, f32::max)
                };
                if v > 0.0 { (20.0 * v.log10()).max(FLOOR_DB) } else { FLOOR_DB }
            })
            .collect()
    }

    /// In-place iterative radix-2 FFT of `re`/`im`.
    fn fft(&mut self) {
        let n = self.re.len();
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                self.re.swap(i, j);
                self.im.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = self.re[b] * wr - self.im[b] * wi;
                    let ti = self.re[b] * wi + self.im[b] * wr;
                    self.re[b] = self.re[a] - tr;
                    self.im[b] = self.im[a] - ti;
                    self.re[a] += tr;
                    self.im[a] += ti;
                }
            }
            size *= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peaks_in_its_band_at_zero_db() {
        let sr = 48_000.0;
        let scope = Scope::default();
        let f = 1_000.0;
        let l: Vec<f32> = (0..SCOPE_LEN).map(|i| (std::f32::consts::TAU * f * i as f32 / sr).sin()).collect();
        scope.write(&l, &l);
        let bands = Analyzer::default().analyze(&scope, sr);
        let (peak, db) = bands.iter().enumerate().fold((0, f32::MIN), |m, (i, &v)| if v > m.1 { (i, v) } else { m });
        let centre = F_LO * ((F_HI / F_LO).ln() * (peak as f32 + 0.5) / BANDS as f32).exp();
        assert!((centre / f).log2().abs() < 0.1, "peak at {centre} Hz");
        assert!(db > -2.0 && db < 1.0, "peak level {db} dB");
        assert!(bands[0] < -60.0, "20 Hz stays low: {}", bands[0]);
    }
}
