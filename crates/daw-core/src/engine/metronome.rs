//! Click track: short pitched blips on each beat, accented on the downbeat.

use std::f32::consts::TAU;

pub struct Metronome {
    pub enabled: bool,
    pub volume: f32,
    sr: f32,
    phase: f32,
    freq: f32,
    env: f32,
    decay: f32,
    /// Frames already rendered in the current block (so the tail pass skips them).
    rendered_until: usize,
}

impl Metronome {
    pub fn new(sr: f32) -> Self {
        Self {
            enabled: false,
            volume: 0.7,
            sr,
            phase: 0.0,
            freq: 1000.0,
            env: 0.0,
            decay: (-1.0 / (0.02 * sr)).exp(),
            rendered_until: 0,
        }
    }

    fn trigger(&mut self, accent: bool) {
        self.freq = if accent { 1760.0 } else { 1320.0 };
        self.env = if accent { 1.0 } else { 0.6 };
        self.phase = 0.0;
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.env < 1e-4 {
            self.env = 0.0;
            return;
        }
        let inc = TAU * self.freq / self.sr;
        let g = self.volume * 0.5;
        for i in 0..l.len() {
            let s = self.phase.sin() * self.env * g;
            self.phase += inc;
            if self.phase > TAU {
                self.phase -= TAU;
            }
            self.env *= self.decay;
            l[i] += s;
            r[i] += s;
        }
    }

    /// Triggers clicks for the beats inside this segment (offset `f0` in the
    /// block) and renders the click sound up to the end of the segment.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        b0: f64,
        beats_per_frame: f64,
        beats_per_bar: f64,
        force: bool,
        l: &mut [f32],
        r: &mut [f32],
    ) {
        let n = l.len();
        let mut cursor = 0usize;
        if self.enabled || force {
            let b1 = b0 + n as f64 * beats_per_frame;
            let mut beat = (b0 - 1e-9).ceil();
            while beat < b1 {
                let frame = (((beat - b0) / beats_per_frame).max(0.0) as usize).min(n);
                self.run(&mut l[cursor..frame], &mut r[cursor..frame]);
                cursor = frame;
                let rel = beat.rem_euclid(beats_per_bar.max(1.0));
                self.trigger(rel < 1e-6 || (beats_per_bar - rel) < 1e-6);
                beat += 1.0;
            }
        }
        self.run(&mut l[cursor..], &mut r[cursor..]);
        self.rendered_until = usize::MAX;
    }

    /// Renders the decaying click when the transport did not call `render`
    /// for this block (stopped).
    pub fn render_tail(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.rendered_until == usize::MAX {
            self.rendered_until = 0;
            return;
        }
        self.run(l, r);
    }
}
