//! Lock-free state shared between the audio thread and the rest of the app.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

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
        };
        s.sample_rate.store(sample_rate);
        s
    }
}
