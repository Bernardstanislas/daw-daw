//! "Drum Kit": a fully synthesized drum machine (no samples).
//!
//! Every hit is a small one-shot voice built from a handful of primitives
//! (sine with pitch envelope, metallic square clusters, filtered noise,
//! multi-burst noise) whose coefficients are computed once at note-on. A
//! `kit` choice selects one of four characters which changes the base
//! tunings / decays of every piece and the bus colouring (bitcrush, low-pass,
//! saturation). Notes follow the General MIDI drum map.

// Parallel per-partial arrays read clearest with index loops.
#![allow(clippy::needless_range_loop)]

use super::util::{
    Noise, OnePole, Smoother, Svf, decay_coef, fast_sin01, pan_gains, poly_blep, soft_clip,
};
use super::*;

const KITS: &[&str] = &["Boom", "Punch", "Dusty", "Tiny"];

const P_KIT: usize = 0;
const P_KICK_TUNE: usize = 1;
const P_KICK_DECAY: usize = 2;
const P_KICK_PUNCH: usize = 3;
const P_SNARE_TUNE: usize = 4;
const P_SNARE_TONE: usize = 5;
const P_SNARE_DECAY: usize = 6;
const P_HAT_TONE: usize = 7;
const P_HAT_DECAY: usize = 8;
const P_TOM_TUNE: usize = 9;
const P_TOM_DECAY: usize = 10;
const P_CYM_DECAY: usize = 11;
const P_PERC_TUNE: usize = 12;
const P_DRIVE: usize = 13;
const P_LEVEL: usize = 14;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "kit",
    name: "Drum Kit",
    category: "Drums",
    description: "Synthesized drum machine with four kit characters, GM drum mapping.",
    params: &[
        ParamSpec::choice("kit", "Kit", KITS, 1, "Kit"),
        ParamSpec::lin("kick_tune", "Tune", -12.0, 12.0, 0.0, "st", "Kick"),
        ParamSpec::lin("kick_decay", "Decay", 0.0, 100.0, 50.0, "%", "Kick"),
        ParamSpec::lin("kick_punch", "Punch", 0.0, 100.0, 50.0, "%", "Kick"),
        ParamSpec::lin("snare_tune", "Tune", -12.0, 12.0, 0.0, "st", "Snare"),
        ParamSpec::lin("snare_tone", "Snappy", 0.0, 100.0, 50.0, "%", "Snare"),
        ParamSpec::lin("snare_decay", "Decay", 0.0, 100.0, 50.0, "%", "Snare"),
        ParamSpec::lin("hat_tone", "Tone", 0.0, 100.0, 50.0, "%", "Hats"),
        ParamSpec::lin("hat_decay", "Decay", 0.0, 100.0, 50.0, "%", "Hats"),
        ParamSpec::lin("tom_tune", "Tune", -12.0, 12.0, 0.0, "st", "Toms"),
        ParamSpec::lin("tom_decay", "Decay", 0.0, 100.0, 50.0, "%", "Toms"),
        ParamSpec::lin("cym_decay", "Decay", 0.0, 100.0, 50.0, "%", "Cymbals"),
        ParamSpec::lin("perc_tune", "Tune", -12.0, 12.0, 0.0, "st", "Perc"),
        ParamSpec::lin("drive", "Drive", 0.0, 100.0, 0.0, "%", "Master"),
        ParamSpec::lin("level", "Level", -24.0, 6.0, 0.0, "dB", "Master"),
    ],
    presets: &[
        Preset {
            name: "Boom Room",
            values: &[("kit", 0.0)],
        },
        Preset {
            name: "Punch Card",
            values: &[("kit", 1.0)],
        },
        Preset {
            name: "Dusty Crate",
            values: &[("kit", 2.0), ("drive", 25.0)],
        },
        Preset {
            name: "Tiny Arcade",
            values: &[("kit", 3.0)],
        },
        Preset {
            name: "Deep Thump",
            values: &[
                ("kit", 0.0),
                ("kick_tune", -3.0),
                ("kick_decay", 80.0),
                ("kick_punch", 30.0),
                ("drive", 35.0),
            ],
        },
        Preset {
            name: "Tight Box",
            values: &[
                ("kit", 1.0),
                ("kick_decay", 25.0),
                ("kick_punch", 80.0),
                ("snare_tune", 3.0),
                ("snare_decay", 30.0),
                ("hat_decay", 30.0),
                ("tom_decay", 30.0),
                ("cym_decay", 30.0),
            ],
        },
        Preset {
            name: "Tape Basement",
            values: &[
                ("kit", 2.0),
                ("kick_tune", -2.0),
                ("snare_tone", 70.0),
                ("hat_tone", 30.0),
                ("cym_decay", 70.0),
                ("drive", 50.0),
                ("level", -2.0),
            ],
        },
    ],
    note_names: &[
        (36, "Kick"),
        (37, "Rim"),
        (38, "Snare"),
        (39, "Clap"),
        (41, "Low Tom"),
        (42, "Closed Hat"),
        (44, "Pedal Hat"),
        (45, "Mid Tom"),
        (46, "Open Hat"),
        (48, "High Tom"),
        (49, "Crash"),
        (51, "Ride"),
        (54, "Tambourine"),
        (56, "Cowbell"),
        (75, "Clave"),
    ],
};

/// Per-kit character: base tunings (Hz), decay times (seconds to -60 dB)
/// and bus colouring.
struct Style {
    kick_hz: f32,
    kick_decay: f32,
    /// Pitch-envelope depth (ratio above the base pitch at the very start).
    kick_bend: f32,
    kick_bend_time: f32,
    kick_click: f32,
    snare_hz: f32,
    snare_body: f32,
    snare_noise: f32,
    snare_fc: f32,
    clap_fc: f32,
    clap_tail: f32,
    hat_base: f32,
    hat_fc: f32,
    hat_noise: f32,
    hat_decay: f32,
    tom_scale: f32,
    tom_decay: f32,
    tom_bend: f32,
    cym_decay: f32,
    perc_scale: f32,
    /// Triangle oscillators instead of sines (chip flavour).
    chip: bool,
    /// Bus bitcrusher: bit depth (0 = off) and target sample rate.
    crush_bits: f32,
    crush_rate: f32,
    /// Bus low-pass (0 = off).
    lp_hz: f32,
    /// Extra bus saturation.
    sat: f32,
}

static STYLES: [Style; 4] = [
    // Boom: deep, long, 808-flavoured.
    Style {
        kick_hz: 45.0,
        kick_decay: 1.1,
        kick_bend: 2.2,
        kick_bend_time: 0.22,
        kick_click: 0.35,
        snare_hz: 172.0,
        snare_body: 0.2,
        snare_noise: 0.28,
        snare_fc: 1700.0,
        clap_fc: 1100.0,
        clap_tail: 0.3,
        hat_base: 330.0,
        hat_fc: 8500.0,
        hat_noise: 0.3,
        hat_decay: 1.1,
        tom_scale: 0.85,
        tom_decay: 0.8,
        tom_bend: 0.45,
        cym_decay: 1.3,
        perc_scale: 1.0,
        chip: false,
        crush_bits: 0.0,
        crush_rate: 0.0,
        lp_hz: 0.0,
        sat: 0.0,
    },
    // Punch: tight and snappy.
    Style {
        kick_hz: 56.0,
        kick_decay: 0.42,
        kick_bend: 4.0,
        kick_bend_time: 0.09,
        kick_click: 0.7,
        snare_hz: 198.0,
        snare_body: 0.14,
        snare_noise: 0.22,
        snare_fc: 2400.0,
        clap_fc: 1300.0,
        clap_tail: 0.2,
        hat_base: 410.0,
        hat_fc: 10000.0,
        hat_noise: 0.45,
        hat_decay: 0.9,
        tom_scale: 1.0,
        tom_decay: 0.5,
        tom_bend: 0.7,
        cym_decay: 1.0,
        perc_scale: 1.0,
        chip: false,
        crush_bits: 0.0,
        crush_rate: 0.0,
        lp_hz: 0.0,
        sat: 0.0,
    },
    // Dusty: lo-fi, crunchy, darker.
    Style {
        kick_hz: 52.0,
        kick_decay: 0.55,
        kick_bend: 2.8,
        kick_bend_time: 0.13,
        kick_click: 0.45,
        snare_hz: 186.0,
        snare_body: 0.16,
        snare_noise: 0.25,
        snare_fc: 1500.0,
        clap_fc: 1000.0,
        clap_tail: 0.22,
        hat_base: 360.0,
        hat_fc: 7000.0,
        hat_noise: 0.6,
        hat_decay: 1.0,
        tom_scale: 0.95,
        tom_decay: 0.55,
        tom_bend: 0.6,
        cym_decay: 0.85,
        perc_scale: 0.95,
        chip: false,
        crush_bits: 10.0,
        crush_rate: 16000.0,
        lp_hz: 7500.0,
        sat: 0.6,
    },
    // Tiny: chip / electro blips.
    Style {
        kick_hz: 68.0,
        kick_decay: 0.25,
        kick_bend: 7.0,
        kick_bend_time: 0.05,
        kick_click: 0.25,
        snare_hz: 255.0,
        snare_body: 0.09,
        snare_noise: 0.14,
        snare_fc: 3500.0,
        clap_fc: 1500.0,
        clap_tail: 0.12,
        hat_base: 620.0,
        hat_fc: 11000.0,
        hat_noise: 0.8,
        hat_decay: 0.7,
        tom_scale: 1.3,
        tom_decay: 0.3,
        tom_bend: 1.4,
        cym_decay: 0.6,
        perc_scale: 1.2,
        chip: true,
        crush_bits: 6.0,
        crush_rate: 22050.0,
        lp_hz: 0.0,
        sat: 0.2,
    },
];

// Invented inharmonic frequency ratios for the metallic square clusters.
const HAT_RATIOS: [f32; 6] = [1.0, 1.4471, 1.6170, 1.9265, 2.5028, 2.6637];
const CRASH_RATIOS: [f32; 6] = [1.0, 1.3932, 1.7206, 2.0731, 2.6489, 3.1279];
const RIDE_RATIOS: [f32; 6] = [1.0, 1.1892, 1.5104, 2.2449, 2.7311, 3.3019];
const TAMB_RATIOS: [f32; 6] = [2.0, 2.33, 2.71, 3.1, 3.55, 4.02];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    Kick,
    Rim,
    Snare,
    Clap,
    ClosedHat,
    PedalHat,
    OpenHat,
    LowTom,
    MidTom,
    HighTom,
    Crash,
    Ride,
    Cowbell,
    Tamb,
    Clave,
}

/// GM drum map (plus a few close neighbours so imported MIDI files play).
fn piece_for(pitch: u8) -> Option<Piece> {
    Some(match pitch {
        35 | 36 => Piece::Kick,
        37 => Piece::Rim,
        38 | 40 => Piece::Snare,
        39 => Piece::Clap,
        42 => Piece::ClosedHat,
        44 => Piece::PedalHat,
        46 => Piece::OpenHat,
        41 | 43 => Piece::LowTom,
        45 | 47 => Piece::MidTom,
        48 | 50 => Piece::HighTom,
        49 | 52 | 55 | 57 => Piece::Crash,
        51 | 53 | 59 => Piece::Ride,
        56 => Piece::Cowbell,
        54 | 69 | 70 => Piece::Tamb,
        75..=77 => Piece::Clave,
        _ => return None,
    })
}

const NUM_VOICES: usize = 24;
const CHUNK: usize = 64;
/// Envelope level below which a voice is considered finished (-80 dB).
const SILENCE: f32 = 1.0e-4;

#[inline]
fn wrap(p: f32) -> f32 {
    if p >= 1.0 { p - 1.0 } else { p }
}

/// Sine, or a triangle starting at zero phase for the chip kit.
#[inline]
fn osc(ph: f32, chip: bool) -> f32 {
    if chip {
        4.0 * (wrap(ph + 0.75) - 0.5).abs() - 1.0
    } else {
        fast_sin01(ph)
    }
}

/// Band-limited square (PolyBLEP).
#[inline]
fn square(ph: f32, inc: f32) -> f32 {
    let naive = if ph < 0.5 { 1.0 } else { -1.0 };
    naive + poly_blep(ph, inc) - poly_blep(wrap(ph + 0.5), inc)
}

/// Decay-knob mapping: 50 % = nominal, 0 % ≈ x0.33, 100 % ≈ x3.
fn decay_mult(pct: f32) -> f32 {
    2f32.powf((pct - 50.0) / 50.0 * 1.6)
}

fn semis(st: f32) -> f32 {
    2f32.powf(st / 12.0)
}

/// One drum hit. Fields are generic; each piece uses a subset.
#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    piece: Piece,
    age: u64,
    gl: f32,
    gr: f32,
    chip: bool,
    ph: [f32; 6],
    inc: [f32; 6],
    /// Pitch envelope: ratio = 1 + pamt * penv.
    penv: f32,
    pcoef: f32,
    pamt: f32,
    /// Main amplitude envelope.
    amp: f32,
    acoef: f32,
    /// Secondary envelope (noise layer, slow stage, attack splash...).
    amp2: f32,
    acoef2: f32,
    /// Click / transient envelope.
    click: f32,
    ccoef: f32,
    lvl_a: f32,
    lvl_b: f32,
    lvl_c: f32,
    noise: Noise,
    f1: Svf,
    f2: Svf,
    /// Clap bursts.
    t: u32,
    burst_len: u32,
    bursts: u32,
    tail_coef: f32,
}

impl Voice {
    fn idle() -> Voice {
        Voice {
            active: false,
            piece: Piece::Kick,
            age: 0,
            gl: 0.0,
            gr: 0.0,
            chip: false,
            ph: [0.0; 6],
            inc: [0.0; 6],
            penv: 0.0,
            pcoef: 0.0,
            pamt: 0.0,
            amp: 0.0,
            acoef: 0.0,
            amp2: 0.0,
            acoef2: 0.0,
            click: 0.0,
            ccoef: 0.0,
            lvl_a: 0.0,
            lvl_b: 0.0,
            lvl_c: 0.0,
            noise: Noise::new(1),
            f1: Svf::default(),
            f2: Svf::default(),
            t: 0,
            burst_len: 1,
            bursts: 0,
            tail_coef: 0.0,
        }
    }

    #[inline]
    fn loudness(&self) -> f32 {
        (self.amp + self.amp2) * (self.gl + self.gr)
    }

    /// Fast fade-out (choke / hard stop).
    fn fade(&mut self, coef: f32) {
        self.acoef = self.acoef.min(coef);
        self.acoef2 = self.acoef2.min(coef);
        self.ccoef = self.ccoef.min(coef);
        self.tail_coef = self.tail_coef.min(coef);
        self.bursts = 0;
    }

    #[inline]
    fn metal(&mut self) -> f32 {
        let mut m = 0.0;
        for i in 0..6 {
            let p = wrap(self.ph[i] + self.inc[i]);
            self.ph[i] = p;
            m += if p < 0.5 { 1.0 } else { -1.0 };
        }
        let x = m * (1.0 / 6.0) * self.lvl_a + self.noise.next() * self.lvl_b;
        let b = self.f1.process(x).1;
        let h = self.f2.process(b).2;
        let env = self.amp + self.lvl_c * self.amp2;
        self.amp *= self.acoef;
        self.amp2 *= self.acoef2;
        h * env
    }

    /// Renders one mono sample.
    #[inline]
    fn tick(&mut self) -> f32 {
        match self.piece {
            Piece::Kick => {
                let pm = 1.0 + self.pamt * self.penv;
                self.penv *= self.pcoef;
                self.ph[0] = wrap(self.ph[0] + self.inc[0] * pm);
                let body = osc(self.ph[0], self.chip) * self.amp;
                self.amp *= self.acoef;
                let c = self.f1.process(self.noise.next()).2 * self.click;
                self.click *= self.ccoef;
                body * self.lvl_a + c * self.lvl_c
            }
            Piece::Snare => {
                let pm = 1.0 + self.pamt * self.penv;
                self.penv *= self.pcoef;
                self.ph[0] = wrap(self.ph[0] + self.inc[0] * pm);
                self.ph[1] = wrap(self.ph[1] + self.inc[1] * pm);
                let body =
                    (osc(self.ph[0], self.chip) + 0.55 * osc(self.ph[1], self.chip)) * self.amp;
                self.amp *= self.acoef;
                let h = self.f1.process(self.noise.next()).2;
                let n = self.f2.process(h).0 * self.amp2;
                self.amp2 *= self.acoef2;
                body * self.lvl_a + n * self.lvl_b
            }
            Piece::Rim => {
                self.ph[0] = wrap(self.ph[0] + self.inc[0]);
                self.ph[1] = wrap(self.ph[1] + self.inc[1]);
                let tone = (0.6 * osc(self.ph[0], self.chip) + 0.4 * osc(self.ph[1], self.chip))
                    * self.amp;
                self.amp *= self.acoef;
                let n = self.f1.process(self.noise.next()).1 * self.amp2;
                self.amp2 *= self.acoef2;
                tone + n * self.lvl_b
            }
            Piece::Clap => {
                self.t += 1;
                if self.bursts > 0 && self.t.is_multiple_of(self.burst_len) {
                    self.bursts -= 1;
                    self.amp = 1.0;
                    if self.bursts == 0 {
                        self.acoef = self.tail_coef;
                    }
                }
                let n = self.f1.process(self.noise.next()).1 * self.amp;
                self.amp *= self.acoef;
                n
            }
            Piece::ClosedHat
            | Piece::PedalHat
            | Piece::OpenHat
            | Piece::Crash
            | Piece::Ride
            | Piece::Tamb => self.metal(),
            Piece::LowTom | Piece::MidTom | Piece::HighTom => {
                let pm = 1.0 + self.pamt * self.penv;
                self.penv *= self.pcoef;
                self.ph[0] = wrap(self.ph[0] + self.inc[0] * pm);
                let body = osc(self.ph[0], self.chip) * self.amp;
                self.amp *= self.acoef;
                let n = self.f1.process(self.noise.next()).0 * self.amp2;
                self.amp2 *= self.acoef2;
                body + n * self.lvl_b
            }
            Piece::Cowbell => {
                let (i0, i1) = (self.inc[0], self.inc[1]);
                self.ph[0] = wrap(self.ph[0] + i0);
                self.ph[1] = wrap(self.ph[1] + i1);
                let x = 0.5 * (square(self.ph[0], i0) + square(self.ph[1], i1));
                let b = self.f1.process(x).1;
                let env = 0.65 * self.amp + 0.35 * self.amp2;
                self.amp *= self.acoef;
                self.amp2 *= self.acoef2;
                b * env
            }
            Piece::Clave => {
                self.ph[0] = wrap(self.ph[0] + self.inc[0]);
                let s = osc(self.ph[0], self.chip) * self.amp;
                self.amp *= self.acoef;
                s
            }
        }
    }

    fn render(&mut self, bl: &mut [f32], br: &mut [f32]) {
        for (l, r) in bl.iter_mut().zip(br.iter_mut()) {
            let s = self.tick();
            *l += s * self.gl;
            *r += s * self.gr;
            if self.amp < SILENCE && self.amp2 < SILENCE && self.click < SILENCE && self.bursts == 0
            {
                self.active = false;
                break;
            }
        }
    }
}

pub struct DrumKit {
    sr: f32,
    params: ParamValues,
    voices: [Voice; NUM_VOICES],
    counter: u64,
    kit: usize,
    buf_l: [f32; CHUNK],
    buf_r: [f32; CHUNK],
    drive_gain: Smoother,
    drive_comp: Smoother,
    level: Smoother,
    // Bus colouring state.
    crush_q: f32,
    crush_hold: u32,
    crush_cnt: u32,
    held: (f32, f32),
    lp_l: OnePole,
    lp_r: OnePole,
    sat_gain: f32,
    sat_comp: f32,
    kill_coef: f32,
    choke_coef: f32,
}

impl DrumKit {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut k = Self {
            sr,
            params: SPEC.default_values(),
            voices: [Voice::idle(); NUM_VOICES],
            counter: 0,
            kit: usize::MAX,
            buf_l: [0.0; CHUNK],
            buf_r: [0.0; CHUNK],
            drive_gain: Smoother::new(1.0, 30.0, sr),
            drive_comp: Smoother::new(1.0, 30.0, sr),
            level: Smoother::new(1.0, 30.0, sr),
            crush_q: 0.0,
            crush_hold: 1,
            crush_cnt: 0,
            held: (0.0, 0.0),
            lp_l: OnePole::default(),
            lp_r: OnePole::default(),
            sat_gain: 1.0,
            sat_comp: 1.0,
            kill_coef: decay_coef(0.004, sr),
            choke_coef: decay_coef(0.012, sr),
        };
        let v = k.params;
        k.set_params(&v);
        k.drive_gain.snap(k.drive_gain.target);
        k.drive_comp.snap(k.drive_comp.target);
        k.level.snap(k.level.target);
        k
    }

    fn style(&self) -> &'static Style {
        &STYLES[self.kit.min(STYLES.len() - 1)]
    }

    fn alloc(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // Steal the quietest hit.
        let mut best = 0;
        let mut best_l = f32::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            let l = v.loudness();
            if l < best_l {
                best_l = l;
                best = i;
            }
        }
        best
    }

    fn make_voice(&self, piece: Piece, vel: f32, seed: u32) -> Voice {
        let sr = self.sr;
        let p = &self.params;
        let st = self.style();
        let mut v = Voice::idle();
        v.active = true;
        v.piece = piece;
        v.age = self.counter;
        v.chip = st.chip;
        v.noise = Noise::new(seed);
        // Velocity: level and a touch of brightness.
        let vg = vel.powf(1.3);
        let bright = 0.75 + 0.25 * vel;
        let nyq = sr * 0.45;
        let (gain, pan) = match piece {
            Piece::Kick => {
                let punch = p[P_KICK_PUNCH] / 100.0;
                let f0 = st.kick_hz * semis(p[P_KICK_TUNE]);
                v.inc[0] = f0 / sr;
                v.pamt = st.kick_bend * (0.6 + 0.4 * vel) * (0.6 + 0.8 * punch);
                v.penv = 1.0;
                v.pcoef = decay_coef(st.kick_bend_time, sr);
                v.amp = 1.0;
                v.acoef = decay_coef(st.kick_decay * decay_mult(p[P_KICK_DECAY]), sr);
                v.click = 1.0;
                v.ccoef = decay_coef(0.012, sr);
                v.lvl_a = 1.0;
                v.lvl_c = st.kick_click * punch * (0.4 + 0.6 * vel) * 1.6;
                v.f1.set(2500.0, 0.2, sr);
                (0.42, 0.0)
            }
            Piece::Snare => {
                let tune = semis(p[P_SNARE_TUNE]);
                let f = st.snare_hz * tune;
                let dm = decay_mult(p[P_SNARE_DECAY]);
                let tone = p[P_SNARE_TONE] / 100.0;
                v.inc[0] = f / sr;
                v.inc[1] = f * 1.53 / sr;
                v.pamt = 0.35;
                v.penv = 1.0;
                v.pcoef = decay_coef(0.05, sr);
                v.amp = 1.0;
                v.acoef = decay_coef(st.snare_body * dm, sr);
                v.amp2 = 1.0;
                v.acoef2 = decay_coef(st.snare_noise * dm, sr);
                v.lvl_a = 1.0 - 0.7 * tone;
                v.lvl_b = (0.4 + 1.6 * tone) * 1.4;
                v.f1.set((st.snare_fc * tune.sqrt() * bright).min(nyq), 0.1, sr);
                v.f2.set((9000.0 * bright).min(nyq), 0.0, sr);
                (0.3, -0.05)
            }
            Piece::Rim => {
                let tune = semis(p[P_PERC_TUNE]) * st.perc_scale;
                v.inc[0] = (1780.0 * tune).min(nyq) / sr;
                v.inc[1] = 510.0 * tune / sr;
                v.amp = 1.0;
                v.acoef = decay_coef(0.045, sr);
                v.amp2 = 1.0;
                v.acoef2 = decay_coef(0.012, sr);
                v.lvl_b = 1.5;
                v.f1.set((3200.0 * tune * bright).min(nyq), 0.4, sr);
                (0.4, -0.12)
            }
            Piece::Clap => {
                let dm = decay_mult(p[P_SNARE_DECAY]);
                v.f1.set(
                    (st.clap_fc * semis(p[P_SNARE_TUNE] * 0.5) * (0.9 + 0.2 * vel)).min(nyq),
                    0.55,
                    sr,
                );
                v.amp = 1.0;
                v.acoef = decay_coef(0.025, sr);
                v.burst_len = ((0.0095 * sr) as u32).max(1);
                v.bursts = 3;
                v.tail_coef = decay_coef(st.clap_tail * dm, sr);
                (1.1, 0.06)
            }
            Piece::ClosedHat | Piece::PedalHat | Piece::OpenHat => {
                let tone = 2f32.powf((p[P_HAT_TONE] - 50.0) / 50.0);
                for i in 0..6 {
                    v.inc[i] = (st.hat_base * HAT_RATIOS[i]).min(nyq) / sr;
                    v.ph[i] = 0.5 + 0.5 * v.noise.next();
                }
                let fc = (st.hat_fc * tone * bright).min(sr * 0.42);
                v.f1.set(fc, 0.35, sr);
                v.f2.set(fc * 0.7, 0.1, sr);
                v.lvl_a = 1.0 - st.hat_noise;
                v.lvl_b = st.hat_noise;
                let (base, g) = match piece {
                    Piece::ClosedHat => (0.09, 1.0),
                    Piece::PedalHat => (0.12, 0.85),
                    _ => (0.75, 0.9),
                };
                v.amp = 1.0;
                v.acoef = decay_coef(base * st.hat_decay * decay_mult(p[P_HAT_DECAY]), sr);
                (1.5 * g, 0.3)
            }
            Piece::Crash | Piece::Ride | Piece::Tamb => {
                let (ratios, base, fc, hp, noise, decay, splash, pan, g) = match piece {
                    Piece::Crash => (
                        &CRASH_RATIOS,
                        st.hat_base * 1.21,
                        7000.0,
                        3500.0,
                        0.55,
                        1.8,
                        0.8,
                        -0.35,
                        1.0,
                    ),
                    Piece::Ride => (
                        &RIDE_RATIOS,
                        st.hat_base * 0.83,
                        4800.0,
                        2500.0,
                        0.25,
                        2.6,
                        0.7,
                        0.35,
                        0.9,
                    ),
                    _ => (
                        &TAMB_RATIOS,
                        1500.0 * st.perc_scale,
                        7500.0,
                        5000.0,
                        0.5,
                        0.25,
                        1.0,
                        -0.25,
                        0.85,
                    ),
                };
                for i in 0..6 {
                    v.inc[i] = (base * ratios[i]).min(nyq) / sr;
                    v.ph[i] = 0.5 + 0.5 * v.noise.next();
                }
                v.f1.set((fc * bright).min(sr * 0.42), 0.3, sr);
                v.f2.set(hp, 0.1, sr);
                v.lvl_a = 1.0 - noise;
                v.lvl_b = noise;
                v.lvl_c = splash;
                let dm = if piece == Piece::Tamb {
                    1.0
                } else {
                    st.cym_decay * decay_mult(p[P_CYM_DECAY])
                };
                v.amp = 1.0;
                v.acoef = decay_coef(decay * dm, sr);
                v.amp2 = 1.0;
                v.acoef2 = decay_coef(0.12, sr);
                (g, pan)
            }
            Piece::LowTom | Piece::MidTom | Piece::HighTom => {
                let (hz, dscale, pan) = match piece {
                    Piece::LowTom => (92.0, 1.2, -0.35),
                    Piece::MidTom => (128.0, 1.0, -0.05),
                    _ => (172.0, 0.85, 0.3),
                };
                v.inc[0] = hz * st.tom_scale * semis(p[P_TOM_TUNE]) / sr;
                v.pamt = st.tom_bend * (0.6 + 0.4 * vel);
                v.penv = 1.0;
                v.pcoef = decay_coef(0.25, sr);
                v.amp = 1.0;
                v.acoef = decay_coef(st.tom_decay * dscale * decay_mult(p[P_TOM_DECAY]), sr);
                v.amp2 = 1.0;
                v.acoef2 = decay_coef(0.03, sr);
                v.lvl_b = 0.4;
                v.f1.set((3000.0 * bright).min(nyq), 0.0, sr);
                (0.45, pan)
            }
            Piece::Cowbell => {
                let tune = semis(p[P_PERC_TUNE]) * st.perc_scale;
                v.inc[0] = 562.0 * tune / sr;
                v.inc[1] = 838.0 * tune / sr;
                v.f1.set((1000.0 * tune).min(nyq), 0.55, sr);
                v.amp = 1.0;
                v.acoef = decay_coef(0.07, sr);
                v.amp2 = 1.0;
                v.acoef2 = decay_coef(0.45, sr);
                (0.55, 0.15)
            }
            Piece::Clave => {
                let tune = semis(p[P_PERC_TUNE]) * st.perc_scale;
                v.inc[0] = (2450.0 * tune).min(nyq) / sr;
                v.amp = 1.0;
                v.acoef = decay_coef(0.09, sr);
                (0.4, 0.2)
            }
        };
        let (pl, pr) = pan_gains(pan);
        v.gl = gain * vg * pl;
        v.gr = gain * vg * pr;
        v
    }

    #[cfg(test)]
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

impl Instrument for DrumKit {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let Some(piece) = piece_for(pitch) else {
            return;
        };
        let vel = velocity.clamp(0.0, 1.0);
        if vel <= 0.0 {
            return;
        }
        if matches!(piece, Piece::ClosedHat | Piece::PedalHat) {
            let c = self.choke_coef;
            for v in self
                .voices
                .iter_mut()
                .filter(|v| v.active && v.piece == Piece::OpenHat)
            {
                v.fade(c);
            }
        }
        self.counter += 1;
        let seed = (self.counter as u32).wrapping_mul(0x9E37_79B9)
            ^ (pitch as u32).wrapping_mul(0x85EB_CA6B);
        let idx = self.alloc();
        self.voices[idx] = self.make_voice(piece, vel, seed);
    }

    /// Drum hits are one-shots: note-off is ignored.
    fn note_off(&mut self, _pitch: u8) {}

    fn all_notes_off(&mut self, hard: bool) {
        if hard {
            let c = self.kill_coef;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.fade(c);
            }
        }
    }

    fn set_params(&mut self, values: &ParamValues) {
        self.params = *values;
        let kit = (values[P_KIT].round().max(0.0) as usize).min(STYLES.len() - 1);
        if kit != self.kit {
            self.kit = kit;
            let st = &STYLES[kit];
            if st.crush_bits > 0.0 {
                self.crush_q = 2f32.powf(st.crush_bits - 1.0);
                self.crush_hold = ((self.sr / st.crush_rate).round() as u32).max(1);
            } else {
                self.crush_q = 0.0;
                self.crush_hold = 1;
            }
            if st.lp_hz > 0.0 {
                self.lp_l.set(st.lp_hz, self.sr);
                self.lp_r.set(st.lp_hz, self.sr);
            }
            self.sat_gain = 1.0 + st.sat;
            self.sat_comp = 1.0 / (1.0 + 0.5 * st.sat);
        }
        let dg = 1.0 + 7.0 * (values[P_DRIVE] / 100.0);
        self.drive_gain.set(dg);
        self.drive_comp.set(dg.powf(-0.6));
        self.level.set(db_to_gain(values[P_LEVEL]));
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let st = self.style();
        let mut start = 0;
        while start < n {
            let len = (n - start).min(CHUNK);
            let bl = &mut self.buf_l[..len];
            let br = &mut self.buf_r[..len];
            bl.fill(0.0);
            br.fill(0.0);
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.render(bl, br);
            }
            let (ol, or) = (&mut l[start..start + len], &mut r[start..start + len]);
            for i in 0..len {
                let dg = self.drive_gain.next();
                let dc = self.drive_comp.next();
                let lv = self.level.next();
                let mut yl = soft_clip(self.buf_l[i] * dg) * dc;
                let mut yr = soft_clip(self.buf_r[i] * dg) * dc;
                if self.crush_q > 0.0 {
                    if self.crush_cnt == 0 {
                        let q = self.crush_q;
                        self.held = ((yl * q).round() / q, (yr * q).round() / q);
                    }
                    self.crush_cnt += 1;
                    if self.crush_cnt >= self.crush_hold {
                        self.crush_cnt = 0;
                    }
                    (yl, yr) = self.held;
                }
                if st.lp_hz > 0.0 {
                    yl = self.lp_l.lp(yl);
                    yr = self.lp_r.lp(yr);
                }
                if st.sat > 0.0 {
                    yl = soft_clip(yl * self.sat_gain) * self.sat_comp;
                    yr = soft_clip(yr * self.sat_gain) * self.sat_comp;
                }
                ol[i] += yl * lv;
                or[i] += yr * lv;
            }
            start += len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::util::test_support::*;
    use super::*;

    const ALL: [u8; 15] = [36, 37, 38, 39, 41, 42, 44, 45, 46, 48, 49, 51, 54, 56, 75];

    fn kit_with(values: ParamValues, sr: f32) -> DrumKit {
        let mut k = DrumKit::new(sr);
        k.set_params(&values);
        k
    }

    #[test]
    fn spec_is_valid() {
        check_spec(&SPEC);
        for (n, _) in SPEC.note_names {
            assert!(piece_for(*n).is_some());
        }
    }

    #[test]
    fn every_piece_every_kit_renders_sanely() {
        for &sr in &[48000.0, 44100.0] {
            for kit in 0..KITS.len() {
                let vals = with(&SPEC, SPEC.default_values(), "kit", kit as f32);
                for &note in &ALL {
                    let mut k = kit_with(vals, sr);
                    k.note_on(note, 1.0);
                    let (l, r) = render(&mut k, sr, sr as usize, 512);
                    assert_finite(&l, "drums l");
                    assert_finite(&r, "drums r");
                    let pk = peak(&l).max(peak(&r));
                    assert!(
                        (0.05..=1.0).contains(&pk),
                        "kit {kit} note {note} sr {sr}: peak {pk}"
                    );
                    // Let the tail finish: voices must free themselves.
                    let (l2, _) = render(&mut k, sr, (sr * 9.0) as usize, 512);
                    assert_eq!(k.active_voices(), 0, "kit {kit} note {note} still active");
                    assert!(
                        peak(&l2[l2.len() - 4800..]) < 1e-4,
                        "kit {kit} note {note} not silent"
                    );
                }
            }
        }
    }

    #[test]
    fn max_decay_tails_still_end() {
        let mut v = SPEC.default_values();
        for id in [
            "kick_decay",
            "snare_decay",
            "hat_decay",
            "tom_decay",
            "cym_decay",
        ] {
            v = with(&SPEC, v, id, 100.0);
        }
        v = with(&SPEC, v, "kit", 0.0);
        let mut k = kit_with(v, 48000.0);
        for &n in &ALL {
            k.note_on(n, 1.0);
        }
        let (l, _) = render(&mut k, 48000.0, 48000 * 16, 512);
        assert_finite(&l, "long tails");
        assert_eq!(k.active_voices(), 0);
    }

    #[test]
    fn presets_render_without_clipping() {
        for preset in SPEC.presets {
            let mut k = kit_with(preset_values(&SPEC, preset), 48000.0);
            for &n in &[36, 38, 42, 49, 45] {
                k.note_on(n, 1.0);
            }
            let (l, r) = render(&mut k, 48000.0, 48000, 256);
            assert_finite(&l, preset.name);
            assert_finite(&r, preset.name);
            let pk = peak(&l).max(peak(&r));
            assert!(pk < 1.5 && pk > 0.05, "{}: peak {pk}", preset.name);
        }
    }

    /// Renders a little pattern with notes landing between block boundaries.
    fn pattern(block: usize) -> Vec<f32> {
        let sr = 48000.0;
        let mut k = DrumKit::new(sr);
        let mut out = Vec::new();
        for (i, &n) in [36u8, 42, 38, 46, 42, 39, 49, 56].iter().enumerate() {
            k.note_on(n, 0.5 + 0.06 * i as f32);
            let (l, r) = render(&mut k, sr, 3001, block);
            out.extend(l);
            out.extend(r);
        }
        out
    }

    #[test]
    fn block_size_does_not_change_output() {
        let a = pattern(1);
        let b = pattern(7);
        let c = pattern(512);
        for i in 0..a.len() {
            assert!(
                (a[i] - b[i]).abs() < 1e-5 && (a[i] - c[i]).abs() < 1e-5,
                "mismatch at {i}"
            );
        }
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let mut k = DrumKit::new(48000.0);
        k.note_on(46, 1.0);
        render(&mut k, 48000.0, 2000, 512);
        assert!(
            k.voices
                .iter()
                .any(|v| v.active && v.piece == Piece::OpenHat)
        );
        k.note_on(42, 1.0);
        render(&mut k, 48000.0, 2400, 512);
        assert!(
            !k.voices
                .iter()
                .any(|v| v.active && v.piece == Piece::OpenHat)
        );
    }

    #[test]
    fn hard_all_notes_off_silences_quickly() {
        let mut k = DrumKit::new(48000.0);
        k.note_on(49, 1.0);
        k.note_on(36, 1.0);
        render(&mut k, 48000.0, 1000, 512);
        k.all_notes_off(true);
        render(&mut k, 48000.0, 480, 64); // 10 ms
        assert_eq!(k.active_voices(), 0);
        let (l, _) = render(&mut k, 48000.0, 512, 512);
        assert!(peak(&l) < 1e-6);
    }

    #[test]
    fn set_params_keeps_ringing_hits() {
        let mut k = DrumKit::new(48000.0);
        k.note_on(49, 1.0);
        render(&mut k, 48000.0, 4800, 512);
        let v = with(&SPEC, SPEC.default_values(), "drive", 60.0);
        k.set_params(&with(&SPEC, v, "cym_decay", 0.0));
        assert_eq!(k.active_voices(), 1);
        let (l, _) = render(&mut k, 48000.0, 480, 512);
        assert!(peak(&l) > 1e-3);
    }

    #[test]
    fn voice_stealing_is_safe() {
        let mut k = DrumKit::new(44100.0);
        for i in 0..100 {
            k.note_on(ALL[i % ALL.len()], 1.0);
            let (l, _) = render(&mut k, 44100.0, 37, 37);
            assert_finite(&l, "stealing");
        }
        assert!(k.active_voices() <= NUM_VOICES);
    }
}
