//! "Rhythm": an analog-style drum machine in the tradition of the two
//! legendary 80s rhythm composers (the boomy "eight" and the punchy
//! "nine"), fully synthesized.
//!
//! Every hit is a one-shot voice mixing a few circuit-like primitives whose
//! coefficients are computed at note-on:
//! - resonant sine "bridged-T" tones with a pitch sweep (kick, toms, congas,
//!   snare body, rim, clave),
//! - six detuned square oscillators through band-pass + high-pass filters
//!   (hats, cymbals, ride, cowbell),
//! - filtered noise with optional multi-burst triggering (snare, clap,
//!   maracas).
//!
//! Each section has its own tune / decay / tone / level. The open hat is
//! choked by the closed and pedal hats. GM drum note mapping.

// Parallel per-oscillator arrays read clearest with index loops.
#![allow(clippy::needless_range_loop)]

use super::util::{Noise, OnePole, Smoother, Svf, SvfMode, decay_coef, fast_sin01, pan_gains, soft_clip};
use super::*;

const MACHINES: &[&str] = &["Eight (boom)", "Nine (punch)"];

const P_MACHINE: usize = 0;
const P_KICK_TUNE: usize = 1;
const P_KICK_DECAY: usize = 2;
const P_KICK_TONE: usize = 3;
const P_KICK_LEVEL: usize = 4;
const P_SN_TUNE: usize = 5;
const P_SN_SNAPPY: usize = 6;
const P_SN_TONE: usize = 7;
const P_SN_DECAY: usize = 8;
const P_SN_LEVEL: usize = 9;
const P_CLAP_DECAY: usize = 10;
const P_CLAP_TONE: usize = 11;
const P_CLAP_LEVEL: usize = 12;
const P_HAT_TONE: usize = 13;
const P_CH_DECAY: usize = 14;
const P_OH_DECAY: usize = 15;
const P_HAT_LEVEL: usize = 16;
const P_CYM_TONE: usize = 17;
const P_CYM_DECAY: usize = 18;
const P_CYM_LEVEL: usize = 19;
const P_TOM_TUNE: usize = 20;
const P_TOM_DECAY: usize = 21;
const P_TOM_LEVEL: usize = 22;
const P_PERC_TUNE: usize = 23;
const P_PERC_DECAY: usize = 24;
const P_PERC_LEVEL: usize = 25;
const P_DRIVE: usize = 26;
const P_ACCENT: usize = 27;
const P_LEVEL: usize = 28;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "rhythm",
    name: "Rhythm",
    category: "Drums",
    description: "Analog-style drum machine (classic 'eight' boom and 'nine' punch), per-voice tune, decay, tone and level.",
    params: &[
        ParamSpec::choice("machine", "Machine", MACHINES, 0, "Machine"),
        ParamSpec::lin("kick_tune", "Tune", -12.0, 12.0, 0.0, "st", "Kick"),
        ParamSpec::lin("kick_decay", "Decay", 0.0, 100.0, 50.0, "%", "Kick"),
        ParamSpec::lin("kick_tone", "Attack", 0.0, 100.0, 40.0, "%", "Kick"),
        ParamSpec::lin("kick_level", "Level", -24.0, 6.0, 0.0, "dB", "Kick"),
        ParamSpec::lin("sn_tune", "Tune", -12.0, 12.0, 0.0, "st", "Snare"),
        ParamSpec::lin("sn_snappy", "Snappy", 0.0, 100.0, 55.0, "%", "Snare"),
        ParamSpec::lin("sn_tone", "Tone", 0.0, 100.0, 50.0, "%", "Snare"),
        ParamSpec::lin("sn_decay", "Decay", 0.0, 100.0, 50.0, "%", "Snare"),
        ParamSpec::lin("sn_level", "Level", -24.0, 6.0, 0.0, "dB", "Snare"),
        ParamSpec::lin("clap_decay", "Decay", 0.0, 100.0, 50.0, "%", "Clap"),
        ParamSpec::lin("clap_tone", "Tone", 0.0, 100.0, 50.0, "%", "Clap"),
        ParamSpec::lin("clap_level", "Level", -24.0, 6.0, 0.0, "dB", "Clap"),
        ParamSpec::lin("hat_tone", "Tone", 0.0, 100.0, 50.0, "%", "Hats"),
        ParamSpec::lin("ch_decay", "Closed", 0.0, 100.0, 40.0, "%", "Hats"),
        ParamSpec::lin("oh_decay", "Open", 0.0, 100.0, 50.0, "%", "Hats"),
        ParamSpec::lin("hat_level", "Level", -24.0, 6.0, -2.0, "dB", "Hats"),
        ParamSpec::lin("cym_tone", "Tone", 0.0, 100.0, 50.0, "%", "Cymbal"),
        ParamSpec::lin("cym_decay", "Decay", 0.0, 100.0, 50.0, "%", "Cymbal"),
        ParamSpec::lin("cym_level", "Level", -24.0, 6.0, -4.0, "dB", "Cymbal"),
        ParamSpec::lin("tom_tune", "Tune", -12.0, 12.0, 0.0, "st", "Toms"),
        ParamSpec::lin("tom_decay", "Decay", 0.0, 100.0, 50.0, "%", "Toms"),
        ParamSpec::lin("tom_level", "Level", -24.0, 6.0, -2.0, "dB", "Toms"),
        ParamSpec::lin("perc_tune", "Tune", -12.0, 12.0, 0.0, "st", "Perc"),
        ParamSpec::lin("perc_decay", "Decay", 0.0, 100.0, 50.0, "%", "Perc"),
        ParamSpec::lin("perc_level", "Level", -24.0, 6.0, -3.0, "dB", "Perc"),
        ParamSpec::lin("drive", "Drive", 0.0, 100.0, 10.0, "%", "Master"),
        ParamSpec::lin("accent", "Accent", 0.0, 100.0, 60.0, "%", "Master"),
        ParamSpec::lin("level", "Level", -24.0, 6.0, 0.0, "dB", "Master"),
    ],
    presets: &[
        Preset { name: "Eight Classic", values: &[] },
        Preset {
            name: "Eight Long Boom",
            values: &[("kick_decay", 90.0), ("kick_tune", -3.0), ("kick_tone", 20.0), ("drive", 25.0), ("sn_snappy", 45.0)],
        },
        Preset {
            name: "Eight Electro",
            values: &[("kick_decay", 70.0), ("sn_snappy", 75.0), ("sn_tone", 70.0), ("ch_decay", 25.0), ("perc_level", 0.0), ("drive", 20.0)],
        },
        Preset { name: "Nine Classic", values: &[("machine", 1.0)] },
        Preset {
            name: "Nine Techno",
            values: &[
                ("machine", 1.0),
                ("kick_decay", 60.0),
                ("kick_tone", 70.0),
                ("kick_level", 2.0),
                ("drive", 45.0),
                ("oh_decay", 35.0),
                ("clap_decay", 60.0),
            ],
        },
        Preset {
            name: "Nine House",
            values: &[("machine", 1.0), ("kick_decay", 45.0), ("sn_tone", 35.0), ("clap_level", 2.0), ("hat_tone", 65.0), ("drive", 20.0)],
        },
        Preset {
            name: "Lo-Fi Box",
            values: &[("hat_tone", 20.0), ("cym_tone", 20.0), ("sn_tone", 30.0), ("clap_tone", 25.0), ("drive", 60.0), ("level", -4.0)],
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
        (49, "Cymbal"),
        (51, "Ride"),
        (56, "Cowbell"),
        (62, "Hi Conga Mute"),
        (63, "Hi Conga"),
        (64, "Low Conga"),
        (70, "Maracas"),
        (75, "Clave"),
    ],
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    Kick,
    Snare,
    Clap,
    Rim,
    ClosedHat,
    PedalHat,
    OpenHat,
    Cymbal,
    Ride,
    LowTom,
    MidTom,
    HighTom,
    Cowbell,
    CongaMute,
    CongaHi,
    CongaLow,
    Maracas,
    Clave,
}

fn piece_for(note: u8) -> Option<Piece> {
    Some(match note {
        35 | 36 => Piece::Kick,
        37 => Piece::Rim,
        38 | 40 => Piece::Snare,
        39 => Piece::Clap,
        41 | 43 => Piece::LowTom,
        42 => Piece::ClosedHat,
        44 => Piece::PedalHat,
        45 | 47 => Piece::MidTom,
        46 => Piece::OpenHat,
        48 | 50 => Piece::HighTom,
        49 | 52 | 55 | 57 => Piece::Cymbal,
        51 | 53 | 59 => Piece::Ride,
        56 => Piece::Cowbell,
        62 => Piece::CongaMute,
        63 => Piece::CongaHi,
        64 => Piece::CongaLow,
        54 | 69 | 70 => Piece::Maracas,
        75..=77 => Piece::Clave,
        _ => return None,
    })
}

/// Metallic square frequencies of the classic six-oscillator cymbal circuit.
const METAL_HZ: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
const MAX_VOICES: usize = 24;
const SILENCE: f32 = 1.0e-4;

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    piece: Piece,
    age: u64,
    gain: f32,
    gl: f32,
    gr: f32,
    sat: f32,
    // Two resonant tones with a shared pitch sweep.
    t_ph: [f32; 2],
    t_inc: [f32; 2],
    t_amp: [f32; 2],
    t_coef: [f32; 2],
    bend: f32,
    bend_coef: f32,
    // Click transient.
    click: f32,
    click_coef: f32,
    // Noise.
    noise: Noise,
    nf: Svf,
    nmode: SvfMode,
    namp: f32,
    ncoef: f32,
    nlvl: f32,
    // Clap bursts.
    bursts: u8,
    burst_timer: u32,
    burst_period: u32,
    burst_coef: f32,
    burst_env: f32,
    // Metal (six squares).
    m_ph: [f32; 6],
    m_inc: [f32; 6],
    m_count: usize,
    mf: Svf,
    mhp: OnePole,
    m_fast: f32,
    m_fast_coef: f32,
    m_slow: f32,
    m_slow_coef: f32,
}

impl Voice {
    fn idle() -> Self {
        Self {
            active: false,
            piece: Piece::Kick,
            age: 0,
            gain: 0.0,
            gl: 1.0,
            gr: 1.0,
            sat: 0.0,
            t_ph: [0.0; 2],
            t_inc: [0.0; 2],
            t_amp: [0.0; 2],
            t_coef: [0.0; 2],
            bend: 0.0,
            bend_coef: 0.0,
            click: 0.0,
            click_coef: 0.0,
            noise: Noise::new(1),
            nf: Svf::default(),
            nmode: SvfMode::BandPass,
            namp: 0.0,
            ncoef: 0.0,
            nlvl: 0.0,
            bursts: 0,
            burst_timer: 0,
            burst_period: 1,
            burst_coef: 0.0,
            burst_env: 0.0,
            m_ph: [0.0; 6],
            m_inc: [0.0; 6],
            m_count: 0,
            mf: Svf::default(),
            mhp: OnePole::default(),
            m_fast: 0.0,
            m_fast_coef: 0.0,
            m_slow: 0.0,
            m_slow_coef: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        let mut x = 0.0;
        // Tones.
        let b = 1.0 + self.bend;
        self.bend *= self.bend_coef;
        for k in 0..2 {
            if self.t_amp[k] > SILENCE {
                self.t_ph[k] += self.t_inc[k] * b;
                if self.t_ph[k] >= 1.0 {
                    self.t_ph[k] -= self.t_ph[k].floor();
                }
                x += fast_sin01(self.t_ph[k]) * self.t_amp[k];
                self.t_amp[k] *= self.t_coef[k];
            }
        }
        if self.click.abs() > SILENCE {
            x += self.click;
            self.click *= -self.click_coef; // alternating: bright, DC-free
        }
        // Noise (with clap re-triggering).
        if self.bursts > 0 {
            if self.burst_timer == 0 {
                self.burst_env = 1.0;
                self.bursts -= 1;
                if self.bursts == 0 {
                    // Last burst: the reverberant tail takes over.
                    self.namp = 0.7;
                }
                self.burst_timer = self.burst_period;
            } else {
                self.burst_timer -= 1;
            }
        }
        if self.namp > SILENCE || self.burst_env > SILENCE {
            let n = self.nf.process_mode(self.noise.next(), self.nmode);
            x += n * (self.namp + self.burst_env) * self.nlvl;
            self.namp *= self.ncoef;
            self.burst_env *= self.burst_coef;
        }
        // Metal.
        if self.m_count > 0 && (self.m_fast > SILENCE || self.m_slow > SILENCE) {
            let mut m = 0.0;
            for k in 0..self.m_count {
                self.m_ph[k] += self.m_inc[k];
                if self.m_ph[k] >= 1.0 {
                    self.m_ph[k] -= 1.0;
                }
                m += if self.m_ph[k] < 0.5 { 1.0 } else { -1.0 };
            }
            let m = self.mhp.hp(self.mf.process(m * 0.25).1);
            x += m * (self.m_fast + self.m_slow);
            self.m_fast *= self.m_fast_coef;
            self.m_slow *= self.m_slow_coef;
        }
        if self.sat > 0.0 {
            x = soft_clip(x * (1.0 + self.sat)) / (1.0 + self.sat * 0.5);
        }
        x * self.gain
    }

    fn done(&self) -> bool {
        self.t_amp[0] < SILENCE
            && self.t_amp[1] < SILENCE
            && self.click.abs() < SILENCE
            && self.namp < SILENCE
            && self.burst_env < SILENCE
            && self.bursts == 0
            && (self.m_count == 0 || (self.m_fast < SILENCE && self.m_slow < SILENCE))
    }
}

fn decay_mult(pct: f32) -> f32 {
    2f32.powf((pct - 50.0) / 50.0 * 1.5)
}

pub struct Rhythm {
    sr: f32,
    params: ParamValues,
    voices: [Voice; MAX_VOICES],
    counter: u64,
    seed: u32,
    drive: Smoother,
    level: Smoother,
}

impl Rhythm {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut r = Self {
            sr,
            params: SPEC.default_values(),
            voices: [Voice::idle(); MAX_VOICES],
            counter: 0,
            seed: 0xC0FF_EE11,
            drive: Smoother::new(0.0, 30.0, sr),
            level: Smoother::new(1.0, 30.0, sr),
        };
        let v = r.params;
        r.set_params(&v);
        r.drive.snap(r.drive.target);
        r.level.snap(r.level.target);
        r
    }

    fn alloc(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        let mut best = 0;
        for i in 1..MAX_VOICES {
            if self.voices[i].age < self.voices[best].age {
                best = i;
            }
        }
        best
    }

    fn make(&mut self, piece: Piece, vel: f32) -> Voice {
        let sr = self.sr;
        let p = &self.params;
        let nine = p[P_MACHINE].round() >= 1.0;
        let pct = |i: usize| p[i] / 100.0;
        let ratio = |i: usize| 2f32.powf(p[i] / 12.0);
        let acc = pct(P_ACCENT);
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut v = Voice::idle();
        v.active = true;
        v.piece = piece;
        v.noise = Noise::new(self.seed | 1);
        let vel_gain = (1.0 - acc) + acc * vel.powf(1.3);
        let level_db: f32;
        let mut pan = 0.0;
        let tone = |v: &mut Voice, k: usize, hz: f32, amp: f32, secs: f32| {
            v.t_inc[k] = (hz / sr).min(0.45);
            v.t_amp[k] = amp;
            v.t_coef[k] = decay_coef(secs, sr);
        };
        let metal = |v: &mut Voice, scale: f32, count: usize, bp: f32, res: f32, hp: f32| {
            v.m_count = count;
            for k in 0..count {
                v.m_inc[k] = (METAL_HZ[k] * scale / sr).min(0.45);
                v.m_ph[k] = (k as f32 * 0.137) % 1.0;
            }
            v.mf.set(bp, res, sr);
            v.mhp.set(hp, sr);
        };
        match piece {
            Piece::Kick => {
                let tune = ratio(P_KICK_TUNE);
                let dm = decay_mult(p[P_KICK_DECAY]);
                let click = pct(P_KICK_TONE);
                if nine {
                    tone(&mut v, 0, 52.0 * tune, 1.0, 0.45 * dm);
                    v.bend = 2.8;
                    v.bend_coef = decay_coef(0.09, sr);
                    v.click = 0.5 * click;
                    v.click_coef = decay_coef(0.002, sr);
                    v.namp = 0.6 * click;
                    v.ncoef = decay_coef(0.012, sr);
                    v.nlvl = 1.0;
                    v.nf.set(3500.0, 0.2, sr);
                    v.nmode = SvfMode::LowPass;
                    v.sat = 1.2;
                } else {
                    tone(&mut v, 0, 49.0 * tune, 1.0, 1.1 * dm);
                    v.bend = 0.7;
                    v.bend_coef = decay_coef(0.04, sr);
                    v.click = 0.35 * click;
                    v.click_coef = decay_coef(0.0015, sr);
                    v.sat = 0.3;
                }
                level_db = p[P_KICK_LEVEL];
            }
            Piece::Snare => {
                let tune = ratio(P_SN_TUNE);
                let dm = decay_mult(p[P_SN_DECAY]);
                let t = pct(P_SN_TONE);
                let snappy = pct(P_SN_SNAPPY);
                if nine {
                    tone(&mut v, 0, 195.0 * tune, 0.75 * (1.2 - t * 0.4), 0.14 * dm);
                    tone(&mut v, 1, 340.0 * tune, 0.45, 0.09 * dm);
                    v.bend = 0.45;
                    v.bend_coef = decay_coef(0.03, sr);
                    v.nf.set(3000.0 + 6000.0 * t, 0.1, sr);
                    v.nmode = SvfMode::LowPass;
                    v.namp = 1.0;
                    v.ncoef = decay_coef(0.28 * dm, sr);
                    v.nlvl = 0.9 * snappy;
                } else {
                    tone(&mut v, 0, 182.0 * tune, 0.75 * (1.2 - t * 0.4), 0.17 * dm);
                    tone(&mut v, 1, 328.0 * tune, 0.5 * (0.6 + t * 0.6), 0.1 * dm);
                    v.bend = 0.12;
                    v.bend_coef = decay_coef(0.01, sr);
                    v.nf.set(1500.0 + 4000.0 * t, 0.15, sr);
                    v.nmode = SvfMode::HighPass;
                    v.namp = 1.0;
                    v.ncoef = decay_coef(0.2 * dm, sr);
                    v.nlvl = 0.8 * snappy;
                }
                v.sat = 0.4;
                level_db = p[P_SN_LEVEL];
            }
            Piece::Clap => {
                let dm = decay_mult(p[P_CLAP_DECAY]);
                let t = pct(P_CLAP_TONE);
                v.nf.set(if nine { 1300.0 } else { 1050.0 } * (0.6 + t * 0.9), 0.45, sr);
                v.nmode = SvfMode::BandPass;
                v.bursts = 3;
                v.burst_timer = 0;
                v.burst_period = (0.0105 * sr) as u32;
                v.burst_coef = decay_coef(0.009, sr);
                // The tail starts at the last burst.
                v.namp = 0.0;
                v.ncoef = decay_coef(0.3 * dm, sr);
                v.nlvl = 2.6;
                v.sat = 0.5;
                level_db = p[P_CLAP_LEVEL];
                pan = 0.05;
            }
            Piece::Rim => {
                let tune = ratio(P_PERC_TUNE);
                tone(&mut v, 0, 480.0 * tune, 0.6, 0.03);
                tone(&mut v, 1, 1720.0 * tune, 0.5, 0.02);
                v.click = 0.4;
                v.click_coef = decay_coef(0.001, sr);
                v.sat = 1.5;
                level_db = p[P_PERC_LEVEL];
                pan = 0.1;
            }
            Piece::ClosedHat | Piece::PedalHat | Piece::OpenHat => {
                let t = pct(P_HAT_TONE);
                let scale = if nine { 1.45 } else { 1.0 };
                metal(&mut v, scale, 6, 7000.0 + 4000.0 * t, 0.35, 5500.0 + 2500.0 * t);
                let secs = match piece {
                    Piece::ClosedHat => 0.03 + 0.12 * pct(P_CH_DECAY),
                    Piece::PedalHat => 0.06 + 0.1 * pct(P_CH_DECAY),
                    _ => 0.18 + 1.1 * pct(P_OH_DECAY),
                };
                v.m_fast = 0.0;
                v.m_slow = 1.6;
                v.m_slow_coef = decay_coef(secs, sr);
                if nine {
                    // Brighter, hissier hats.
                    v.nf.set(9000.0, 0.1, sr);
                    v.nmode = SvfMode::HighPass;
                    v.namp = 1.0;
                    v.ncoef = decay_coef(secs * 0.8, sr);
                    v.nlvl = 0.35;
                }
                level_db = p[P_HAT_LEVEL];
                pan = -0.25;
            }
            Piece::Cymbal | Piece::Ride => {
                let t = pct(P_CYM_TONE);
                let dm = decay_mult(p[P_CYM_DECAY]);
                let ride = piece == Piece::Ride;
                let (scale, bp, secs) = if ride { (1.9, 5200.0, 1.4) } else { (1.15, 4200.0, 2.2) };
                metal(&mut v, if nine { scale * 1.2 } else { scale }, 6, bp * (0.7 + t * 0.8), 0.3, 3000.0 + 3000.0 * t);
                v.m_fast = 1.4;
                v.m_fast_coef = decay_coef(0.12, sr);
                v.m_slow = if ride { 0.7 } else { 1.0 };
                v.m_slow_coef = decay_coef(secs * dm, sr);
                v.nf.set(8000.0, 0.1, sr);
                v.nmode = SvfMode::HighPass;
                v.namp = 1.0;
                v.ncoef = decay_coef(secs * dm * 0.6, sr);
                v.nlvl = if ride { 0.1 } else { 0.25 };
                level_db = p[P_CYM_LEVEL];
                pan = if ride { 0.35 } else { 0.3 };
            }
            Piece::LowTom | Piece::MidTom | Piece::HighTom => {
                let tune = ratio(P_TOM_TUNE);
                let dm = decay_mult(p[P_TOM_DECAY]);
                let (hz, p0) = match piece {
                    Piece::LowTom => (if nine { 95.0 } else { 88.0 }, -0.3),
                    Piece::MidTom => (if nine { 135.0 } else { 126.0 }, 0.0),
                    _ => (if nine { 190.0 } else { 175.0 }, 0.3),
                };
                tone(&mut v, 0, hz * tune, 1.0, (if nine { 0.45 } else { 0.6 }) * dm);
                v.bend = if nine { 0.6 } else { 0.25 };
                v.bend_coef = decay_coef(0.12, sr);
                v.nf.set(hz * 6.0, 0.2, sr);
                v.nmode = SvfMode::LowPass;
                v.namp = 1.0;
                v.ncoef = decay_coef(0.05, sr);
                v.nlvl = if nine { 0.35 } else { 0.12 };
                v.sat = 0.3;
                level_db = p[P_TOM_LEVEL];
                pan = p0;
            }
            Piece::Cowbell => {
                let tune = ratio(P_PERC_TUNE);
                let dm = decay_mult(p[P_PERC_DECAY]);
                v.m_count = 2;
                v.m_inc[0] = 540.0 * tune / sr;
                v.m_inc[1] = 800.0 * tune / sr;
                v.mf.set(2640.0 * tune, 0.75, sr);
                v.mhp.set(400.0, sr);
                v.m_fast = 2.2;
                v.m_fast_coef = decay_coef(0.04, sr);
                v.m_slow = 1.1;
                v.m_slow_coef = decay_coef(0.35 * dm, sr);
                level_db = p[P_PERC_LEVEL];
                pan = 0.2;
            }
            Piece::CongaMute | Piece::CongaHi | Piece::CongaLow => {
                let tune = ratio(P_PERC_TUNE);
                let dm = decay_mult(p[P_PERC_DECAY]);
                let (hz, secs, p0) = match piece {
                    Piece::CongaMute => (330.0, 0.08, 0.4),
                    Piece::CongaHi => (330.0, 0.22, 0.4),
                    _ => (220.0, 0.3, -0.4),
                };
                tone(&mut v, 0, hz * tune, 0.9, secs * dm);
                v.bend = 0.08;
                v.bend_coef = decay_coef(0.02, sr);
                v.click = 0.2;
                v.click_coef = decay_coef(0.001, sr);
                level_db = p[P_PERC_LEVEL];
                pan = p0;
            }
            Piece::Maracas => {
                let dm = decay_mult(p[P_PERC_DECAY]);
                v.nf.set(7000.0, 0.2, sr);
                v.nmode = SvfMode::HighPass;
                v.namp = 1.0;
                v.ncoef = decay_coef(0.05 * dm, sr);
                v.nlvl = 0.6;
                level_db = p[P_PERC_LEVEL];
                pan = -0.35;
            }
            Piece::Clave => {
                let tune = ratio(P_PERC_TUNE);
                let dm = decay_mult(p[P_PERC_DECAY]);
                tone(&mut v, 0, 2450.0 * tune, 0.8, 0.05 * dm);
                level_db = p[P_PERC_LEVEL];
                pan = 0.45;
            }
        }
        v.gain = db_to_gain(level_db) * vel_gain * 0.6;
        let (gl, gr) = pan_gains(pan);
        v.gl = gl;
        v.gr = gr;
        v
    }
}

impl Instrument for Rhythm {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let Some(piece) = piece_for(pitch) else { return };
        let vel = velocity.clamp(0.0, 1.0);
        if vel <= 0.0 {
            return;
        }
        // Closed / pedal hat chokes the open hat; a new hit replaces the
        // previous one of the same piece (like the one-voice-per-instrument
        // hardware), with a tiny fade to avoid clicks.
        let fade = decay_coef(0.004, self.sr);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let choke = v.piece == piece
                || (matches!(piece, Piece::ClosedHat | Piece::PedalHat) && v.piece == Piece::OpenHat);
            if choke {
                for k in 0..2 {
                    v.t_coef[k] = v.t_coef[k].min(fade);
                }
                v.click = 0.0;
                v.bursts = 0;
                v.ncoef = v.ncoef.min(fade);
                v.burst_coef = v.burst_coef.min(fade);
                v.m_fast_coef = v.m_fast_coef.min(fade);
                v.m_slow_coef = v.m_slow_coef.min(fade);
                // Keep its age so it is reused first.
                v.age = 0;
            }
        }
        let i = self.alloc();
        self.counter += 1;
        let mut v = self.make(piece, vel);
        v.age = self.counter;
        self.voices[i] = v;
    }

    fn note_off(&mut self, _pitch: u8) {}

    fn all_notes_off(&mut self, hard: bool) {
        if hard {
            for v in self.voices.iter_mut() {
                v.active = false;
            }
        }
    }

    fn set_params(&mut self, values: &ParamValues) {
        self.params = *values;
        self.drive.set(values[P_DRIVE] / 100.0);
        self.level.set(db_to_gain(values[P_LEVEL]));
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        for i in 0..n {
            let mut sl = 0.0;
            let mut sr = 0.0;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                let x = v.tick();
                sl += x * v.gl;
                sr += x * v.gr;
            }
            let d = self.drive.next();
            let g = self.level.next();
            if d > 0.001 {
                let pre = 1.0 + d * d * 6.0;
                let post = 1.0 / (1.0 + d * 1.5);
                sl = soft_clip(sl * pre) * post;
                sr = soft_clip(sr * pre) * post;
            }
            l[i] += sl * g;
            r[i] += sr * g;
        }
        for v in self.voices.iter_mut().filter(|v| v.active) {
            if v.done() {
                v.active = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::util::test_support::*;

    #[test]
    fn spec_is_valid() {
        check_spec(&SPEC);
        for (n, _) in SPEC.note_names {
            assert!(piece_for(*n).is_some(), "note {n} unmapped");
        }
    }

    #[test]
    fn every_piece_every_preset_renders_and_ends() {
        for &sr in &[44100.0, 48000.0] {
            for preset in SPEC.presets {
                for (note, name) in SPEC.note_names {
                    let mut d = Rhythm::new(sr);
                    d.set_params(&preset_values(&SPEC, preset));
                    d.note_on(*note, 1.0);
                    let (l, r) = render(&mut d, sr, (sr * 0.25) as usize, 128);
                    assert_finite(&l, name);
                    assert_finite(&r, name);
                    let pk = peak(&l).max(peak(&r));
                    assert!(pk > 0.01 && pk < 1.6, "{} / {name}: peak {pk}", preset.name);
                    let _ = render(&mut d, sr, (sr * 8.0) as usize, 512);
                    assert!(!d.voices.iter().any(|v| v.active), "{} / {name}: never ends", preset.name);
                }
            }
        }
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let sr = 48000.0;
        let mut d = Rhythm::new(sr);
        d.note_on(46, 1.0);
        let _ = render(&mut d, sr, 2000, 128);
        d.note_on(42, 1.0);
        let _ = render(&mut d, sr, 4800, 128);
        assert!(!d.voices.iter().any(|v| v.active && v.piece == Piece::OpenHat));
    }

    #[test]
    fn unmapped_notes_are_silent() {
        let mut d = Rhythm::new(48000.0);
        d.note_on(100, 1.0);
        let (l, _) = render(&mut d, 48000.0, 512, 128);
        assert_eq!(peak(&l), 0.0);
    }
}
