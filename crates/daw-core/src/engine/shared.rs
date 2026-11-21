//! Lock-free state shared between the audio thread and the rest of the app.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub const MAX_TRACKS: usize = 256;

#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    #[inline]
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    #[inline]
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
    /// Stores max(current, v).
    #[inline]
    pub fn store_max(&self, v: f32) {
        let mut cur = self.0.load(Ordering::Relaxed);
        while v > f32::from_bits(cur) {
            match self.0.compare_exchange_weak(cur, v.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(c) => cur = c,
            }
        }
    }
    #[inline]
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

#[derive(Default)]
pub struct AtomicF64(AtomicU64);

impl AtomicF64 {
    #[inline]
    pub fn load(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
    #[inline]
    pub fn store(&self, v: f64) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct Meter {
    pub l: AtomicF32,
    pub r: AtomicF32,
}

impl Meter {
    #[inline]
    pub fn feed(&self, l: f32, r: f32) {
        self.l.store_max(l);
        self.r.store_max(r);
    }
    pub fn take(&self) -> (f32, f32) {
        (self.l.take(), self.r.take())
    }
}

/// Samples kept for the spectrum analyzer (a power of two).
pub const SCOPE_LEN: usize = 4096;
/// `Scope::source` values that are not track handles.
pub const SCOPE_OFF: u32 = u32::MAX;
pub const SCOPE_MASTER: u32 = u32::MAX - 1;

/// Ring buffer of the last mono samples of one source (a track or the master),
/// read by the control thread to draw a spectrum.
pub struct Scope {
    /// Track handle, `SCOPE_MASTER` or `SCOPE_OFF`.
    pub source: AtomicU32,
    /// Total samples written (the next write goes to `pos % SCOPE_LEN`).
    pub pos: AtomicUsize,
    pub buf: Vec<AtomicF32>,
}

impl Default for Scope {
    fn default() -> Self {
        Self { source: AtomicU32::new(SCOPE_OFF), pos: AtomicUsize::new(0), buf: (0..SCOPE_LEN).map(|_| AtomicF32::default()).collect() }
    }
}

impl Scope {
    #[inline]
    pub fn listens_to(&self, source: u32) -> bool {
        self.source.load(Ordering::Relaxed) == source
    }
    /// Appends `l`/`r` (mixed to mono). Audio thread only.
    #[inline]
    pub fn write(&self, l: &[f32], r: &[f32]) {
        let mut p = self.pos.load(Ordering::Relaxed);
        for (a, b) in l.iter().zip(r) {
            self.buf[p & (SCOPE_LEN - 1)].store(0.5 * (a + b));
            p = p.wrapping_add(1);
        }
        self.pos.store(p, Ordering::Release);
    }
    /// Copies the last `SCOPE_LEN` samples, oldest first.
    pub fn read(&self, out: &mut [f32; SCOPE_LEN]) {
        let p = self.pos.load(Ordering::Acquire);
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.buf[p.wrapping_add(i) & (SCOPE_LEN - 1)].load();
        }
    }
    pub fn clear(&self) {
        for v in &self.buf {
            v.store(0.0);
        }
    }
}

pub struct Shared {
    /// Song position in beats.
    pub position: AtomicF64,
    pub playing: AtomicBool,
    /// Count-in in progress (metronome only, timeline not running yet).
    pub count_in: AtomicBool,
    /// Actually capturing (after count-in).
    pub recording: AtomicBool,
    pub record_start: AtomicF64,
    pub sample_rate: AtomicF32,
    /// DSP load of the last callbacks (0..1+).
    pub cpu: AtomicF32,
    pub meters: Vec<Meter>,
    pub master: Meter,
    pub input: Meter,
    pub scope: Scope,
}

impl Shared {
    pub fn new(sample_rate: f32) -> Self {
        let s = Self {
            position: AtomicF64::default(),
            playing: AtomicBool::new(false),
            count_in: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            record_start: AtomicF64::default(),
            sample_rate: AtomicF32::default(),
            cpu: AtomicF32::default(),
            meters: (0..MAX_TRACKS).map(|_| Meter::default()).collect(),
            master: Meter::default(),
            input: Meter::default(),
            scope: Scope::default(),
        };
        s.sample_rate.store(sample_rate);
        s
    }
}
