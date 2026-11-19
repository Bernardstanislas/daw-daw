//! Pulse Synth: a polyphonic virtual-analog subtractive synthesizer.
//!
//! Signal path per voice:
//!   2 PolyBLEP oscillators (x1–4 unison, stereo spread) + sub square + noise
//!   → drive → resonant TPT state-variable filter (LP24 / LP12 / BP / HP)
//!   → amp envelope → pan.
//!
//! Pitch, filter cutoff and LFO are evaluated at control rate (every
//! `CONTROL` samples, with the filter coefficient linearly interpolated in
//! between); envelopes and oscillators run per sample. The control counter is
//! global and persists across `process` calls, so the output does not depend
//! on how the host splits blocks.

use super::util::{
    Adsr, LFO_SHAPES, Noise, SYNC_NAMES, Smoother, Svf, fast_pow2, fast_sin01, fast_tan, lfo_value,
    poly_blep, semis_to_ratio, sync_seconds,
};
use super::*;
use std::f32::consts::PI;

const MAX_VOICES: usize = 16;
/// Extra slots so a stolen voice can fade out while the new note starts.
const VOICE_SLOTS: usize = MAX_VOICES + 4;
const MAX_UNISON: usize = 4;
/// Control-rate period in samples.
const CONTROL: usize = 16;
/// Per-voice gain before the master volume (keeps one note around -9 dBFS).
const VOICE_GAIN: f32 = 0.5;

const WAVES: &[&str] = &["Saw", "Square", "Triangle", "Sine"];
const FILTER_MODES: &[&str] = &["LP 24", "LP 12", "Band", "High"];

// Parameter indices (position in `SPEC.params`).
const P_OSC1_WAVE: usize = 0;
const P_OSC1_OCT: usize = 1;
const P_OSC1_SEMI: usize = 2;
const P_OSC1_FINE: usize = 3;
const P_OSC1_PW: usize = 4;
const P_OSC1_LEVEL: usize = 5;
const P_OSC2_WAVE: usize = 6;
const P_OSC2_OCT: usize = 7;
const P_OSC2_SEMI: usize = 8;
const P_OSC2_FINE: usize = 9;
const P_OSC2_PW: usize = 10;
const P_OSC2_LEVEL: usize = 11;
const P_SUB: usize = 12;
const P_NOISE: usize = 13;
const P_UNISON: usize = 14;
const P_UNI_DETUNE: usize = 15;
const P_UNI_SPREAD: usize = 16;
const P_FLT_MODE: usize = 17;
const P_CUTOFF: usize = 18;
const P_RES: usize = 19;
const P_KEYTRACK: usize = 20;
const P_ENV_AMT: usize = 21;
const P_VEL_CUT: usize = 22;
const P_DRIVE: usize = 23;
const P_FENV_A: usize = 24;
const P_FENV_D: usize = 25;
const P_FENV_S: usize = 26;
const P_FENV_R: usize = 27;
const P_AENV_A: usize = 28;
const P_AENV_D: usize = 29;
const P_AENV_S: usize = 30;
const P_AENV_R: usize = 31;
const P_VEL_SENS: usize = 32;
const P_LFO_SHAPE: usize = 33;
const P_LFO_SYNC: usize = 34;
const P_LFO_RATE: usize = 35;
const P_LFO_RATE_SYNC: usize = 36;
const P_LFO_PITCH: usize = 37;
const P_LFO_CUTOFF: usize = 38;
const P_LFO_AMP: usize = 39;
const P_GLIDE: usize = 40;
const P_MONO: usize = 41;
const P_VOLUME: usize = 42;

static PARAMS: &[ParamSpec] = &[
    // Osc 1
    ParamSpec::choice("osc1_wave", "Wave", WAVES, 0, "Osc 1"),
    ParamSpec::int("osc1_oct", "Octave", -2.0, 2.0, 0.0, "Osc 1"),
    ParamSpec::int("osc1_semi", "Semi", -12.0, 12.0, 0.0, "Osc 1"),
    ParamSpec::lin("osc1_fine", "Fine", -50.0, 50.0, 0.0, "ct", "Osc 1"),
    ParamSpec::lin("osc1_pw", "Pulse Width", 5.0, 95.0, 50.0, "%", "Osc 1"),
    ParamSpec::lin("osc1_level", "Level", 0.0, 100.0, 80.0, "%", "Osc 1"),
    // Osc 2
    ParamSpec::choice("osc2_wave", "Wave", WAVES, 0, "Osc 2"),
    ParamSpec::int("osc2_oct", "Octave", -2.0, 2.0, 0.0, "Osc 2"),
    ParamSpec::int("osc2_semi", "Semi", -12.0, 12.0, 0.0, "Osc 2"),
    ParamSpec::lin("osc2_fine", "Fine", -50.0, 50.0, 7.0, "ct", "Osc 2"),
    ParamSpec::lin("osc2_pw", "Pulse Width", 5.0, 95.0, 50.0, "%", "Osc 2"),
    ParamSpec::lin("osc2_level", "Level", 0.0, 100.0, 55.0, "%", "Osc 2"),
    // Mixer
    ParamSpec::lin("sub_level", "Sub", 0.0, 100.0, 20.0, "%", "Mixer"),
    ParamSpec::lin("noise_level", "Noise", 0.0, 100.0, 0.0, "%", "Mixer"),
    ParamSpec::int("unison", "Unison", 1.0, 4.0, 1.0, "Mixer"),
    ParamSpec::lin("uni_detune", "Detune", 0.0, 100.0, 12.0, "ct", "Mixer"),
    ParamSpec::lin("uni_spread", "Stereo", 0.0, 100.0, 60.0, "%", "Mixer"),
    // Filter
    ParamSpec::choice("flt_mode", "Mode", FILTER_MODES, 0, "Filter"),
    ParamSpec::log("cutoff", "Cutoff", 20.0, 20000.0, 1800.0, "Hz", "Filter"),
    ParamSpec::lin("resonance", "Resonance", 0.0, 100.0, 15.0, "%", "Filter"),
    ParamSpec::lin("key_track", "Key Track", 0.0, 100.0, 50.0, "%", "Filter"),
    ParamSpec::lin(
        "env_amount",
        "Env Amount",
        -100.0,
        100.0,
        25.0,
        "%",
        "Filter",
    ),
    ParamSpec::lin(
        "vel_cutoff",
        "Vel > Cutoff",
        0.0,
        100.0,
        30.0,
        "%",
        "Filter",
    ),
    ParamSpec::lin("drive", "Drive", 0.0, 100.0, 0.0, "%", "Filter"),
    // Filter envelope
    ParamSpec::log(
        "fenv_attack",
        "Attack",
        0.001,
        10.0,
        0.005,
        "s",
        "Filter Env",
    ),
    ParamSpec::log("fenv_decay", "Decay", 0.005, 10.0, 0.6, "s", "Filter Env"),
    ParamSpec::lin(
        "fenv_sustain",
        "Sustain",
        0.0,
        100.0,
        40.0,
        "%",
        "Filter Env",
    ),
    ParamSpec::log(
        "fenv_release",
        "Release",
        0.005,
        10.0,
        0.5,
        "s",
        "Filter Env",
    ),
    // Amp envelope
    ParamSpec::log("aenv_attack", "Attack", 0.001, 10.0, 0.01, "s", "Amp Env"),
    ParamSpec::log("aenv_decay", "Decay", 0.005, 10.0, 0.8, "s", "Amp Env"),
    ParamSpec::lin("aenv_sustain", "Sustain", 0.0, 100.0, 75.0, "%", "Amp Env"),
    ParamSpec::log("aenv_release", "Release", 0.005, 10.0, 0.45, "s", "Amp Env"),
    ParamSpec::lin("vel_sens", "Velocity", 0.0, 100.0, 60.0, "%", "Amp Env"),
    // LFO
    ParamSpec::choice("lfo_shape", "Shape", LFO_SHAPES, 0, "LFO"),
    ParamSpec::toggle("lfo_sync", "Sync", false, "LFO"),
    ParamSpec::log("lfo_rate", "Rate", 0.05, 30.0, 4.0, "Hz", "LFO"),
    ParamSpec::choice("lfo_rate_sync", "Sync Rate", SYNC_NAMES, 8, "LFO"),
    ParamSpec::lin("lfo_pitch", "> Pitch", 0.0, 12.0, 0.0, "st", "LFO"),
    ParamSpec::lin("lfo_cutoff", "> Cutoff", 0.0, 100.0, 0.0, "%", "LFO"),
    ParamSpec::lin("lfo_amp", "> Amp", 0.0, 100.0, 0.0, "%", "LFO"),
    // Global
    ParamSpec::log("glide", "Glide", 1.0, 3000.0, 1.0, "ms", "Global"),
    ParamSpec::toggle("mono", "Mono Legato", false, "Global"),
    ParamSpec::lin("volume", "Volume", -36.0, 6.0, -6.0, "dB", "Global"),
];

static PRESETS: &[Preset] = &[
    Preset {
        name: "Sunset Poly",
        values: &[],
    },
    Preset {
        name: "Lead Weight Bass",
        values: &[
            ("osc1_oct", -1.0),
            ("osc2_wave", 1.0),
            ("osc2_oct", -1.0),
            ("osc2_fine", 9.0),
            ("osc2_level", 60.0),
            ("sub_level", 60.0),
            ("cutoff", 260.0),
            ("resonance", 22.0),
            ("key_track", 30.0),
            ("env_amount", 45.0),
            ("fenv_attack", 0.001),
            ("fenv_decay", 0.35),
            ("fenv_sustain", 20.0),
            ("aenv_attack", 0.002),
            ("aenv_decay", 0.4),
            ("aenv_sustain", 90.0),
            ("aenv_release", 0.12),
            ("drive", 40.0),
            ("glide", 30.0),
            ("mono", 1.0),
            ("volume", 0.0),
        ],
    },
    Preset {
        name: "Basement Sub",
        values: &[
            ("osc1_wave", 3.0),
            ("osc1_oct", -1.0),
            ("osc1_level", 90.0),
            ("osc2_wave", 2.0),
            ("osc2_oct", -1.0),
            ("osc2_fine", 0.0),
            ("osc2_level", 30.0),
            ("sub_level", 35.0),
            ("flt_mode", 1.0),
            ("cutoff", 420.0),
            ("resonance", 0.0),
            ("env_amount", 0.0),
            ("vel_cutoff", 0.0),
            ("aenv_attack", 0.003),
            ("aenv_sustain", 100.0),
            ("aenv_release", 0.1),
            ("vel_sens", 30.0),
            ("mono", 1.0),
        ],
    },
    Preset {
        name: "Squelch Machine",
        values: &[
            ("osc1_oct", -1.0),
            ("osc1_level", 90.0),
            ("osc2_level", 0.0),
            ("sub_level", 0.0),
            ("cutoff", 260.0),
            ("resonance", 82.0),
            ("key_track", 40.0),
            ("env_amount", 70.0),
            ("vel_cutoff", 60.0),
            ("drive", 55.0),
            ("fenv_attack", 0.001),
            ("fenv_decay", 0.22),
            ("fenv_sustain", 0.0),
            ("fenv_release", 0.1),
            ("aenv_attack", 0.001),
            ("aenv_decay", 0.3),
            ("aenv_sustain", 80.0),
            ("aenv_release", 0.06),
            ("glide", 70.0),
            ("mono", 1.0),
            ("volume", -2.0),
        ],
    },
    Preset {
        name: "Glass Pluck",
        values: &[
            ("osc2_wave", 1.0),
            ("osc2_oct", 1.0),
            ("osc2_level", 40.0),
            ("sub_level", 0.0),
            ("cutoff", 900.0),
            ("resonance", 25.0),
            ("env_amount", 60.0),
            ("fenv_attack", 0.001),
            ("fenv_decay", 0.25),
            ("fenv_sustain", 0.0),
            ("aenv_attack", 0.001),
            ("aenv_decay", 0.55),
            ("aenv_sustain", 0.0),
            ("aenv_release", 0.3),
            ("unison", 2.0),
            ("uni_detune", 8.0),
        ],
    },
    Preset {
        name: "Laser Lemon",
        values: &[
            ("osc2_fine", -12.0),
            ("osc2_level", 70.0),
            ("sub_level", 0.0),
            ("unison", 2.0),
            ("uni_detune", 15.0),
            ("cutoff", 3500.0),
            ("resonance", 25.0),
            ("env_amount", 30.0),
            ("fenv_decay", 0.4),
            ("aenv_attack", 0.003),
            ("aenv_sustain", 90.0),
            ("aenv_release", 0.2),
            ("lfo_rate", 5.5),
            ("lfo_pitch", 0.15),
            ("glide", 40.0),
            ("mono", 1.0),
        ],
    },
    Preset {
        name: "Hollow Whistle",
        values: &[
            ("osc1_wave", 2.0),
            ("osc2_wave", 3.0),
            ("osc2_oct", 1.0),
            ("osc2_fine", 0.0),
            ("osc2_level", 30.0),
            ("sub_level", 0.0),
            ("noise_level", 4.0),
            ("flt_mode", 1.0),
            ("cutoff", 2500.0),
            ("resonance", 5.0),
            ("env_amount", 10.0),
            ("aenv_attack", 0.04),
            ("aenv_sustain", 85.0),
            ("aenv_release", 0.3),
            ("lfo_rate", 5.0),
            ("lfo_pitch", 0.12),
            ("glide", 60.0),
            ("mono", 1.0),
            ("volume", -3.0),
        ],
    },
    Preset {
        name: "Amber Pad",
        values: &[
            ("osc2_fine", -9.0),
            ("osc2_level", 70.0),
            ("sub_level", 10.0),
            ("unison", 3.0),
            ("uni_detune", 18.0),
            ("uni_spread", 75.0),
            ("cutoff", 1200.0),
            ("resonance", 10.0),
            ("env_amount", 20.0),
            ("vel_cutoff", 15.0),
            ("fenv_attack", 1.2),
            ("fenv_decay", 2.0),
            ("fenv_sustain", 60.0),
            ("fenv_release", 1.5),
            ("aenv_attack", 0.8),
            ("aenv_decay", 1.0),
            ("aenv_sustain", 80.0),
            ("aenv_release", 1.8),
            ("vel_sens", 30.0),
            ("lfo_rate", 0.3),
            ("lfo_cutoff", 12.0),
        ],
    },
    Preset {
        name: "Silk Section",
        values: &[
            ("osc2_fine", 11.0),
            ("osc2_level", 80.0),
            ("sub_level", 0.0),
            ("unison", 4.0),
            ("uni_detune", 12.0),
            ("uni_spread", 85.0),
            ("flt_mode", 1.0),
            ("cutoff", 3200.0),
            ("resonance", 5.0),
            ("env_amount", 0.0),
            ("aenv_attack", 0.35),
            ("aenv_decay", 1.0),
            ("aenv_sustain", 90.0),
            ("aenv_release", 0.9),
            ("vel_sens", 40.0),
            ("lfo_rate", 5.2),
            ("lfo_pitch", 0.08),
            ("volume", -8.0),
        ],
    },
    Preset {
        name: "Bold Fanfare",
        values: &[
            ("osc1_fine", -8.0),
            ("osc2_fine", 8.0),
            ("osc2_level", 80.0),
            ("sub_level", 15.0),
            ("cutoff", 700.0),
            ("resonance", 12.0),
            ("env_amount", 55.0),
            ("vel_cutoff", 50.0),
            ("fenv_attack", 0.06),
            ("fenv_decay", 0.4),
            ("fenv_sustain", 45.0),
            ("fenv_release", 0.25),
            ("aenv_attack", 0.02),
            ("aenv_decay", 0.4),
            ("aenv_sustain", 70.0),
            ("aenv_release", 0.2),
            ("vel_sens", 70.0),
        ],
    },
    Preset {
        name: "Ice Chime",
        values: &[
            ("osc1_wave", 3.0),
            ("osc2_wave", 3.0),
            ("osc2_oct", 2.0),
            ("osc2_semi", 3.0),
            ("osc2_fine", -14.0),
            ("osc2_level", 45.0),
            ("sub_level", 0.0),
            ("flt_mode", 1.0),
            ("cutoff", 8000.0),
            ("resonance", 0.0),
            ("env_amount", 0.0),
            ("aenv_attack", 0.001),
            ("aenv_decay", 2.5),
            ("aenv_sustain", 0.0),
            ("aenv_release", 2.0),
            ("vel_sens", 70.0),
            ("unison", 2.0),
            ("uni_detune", 4.0),
        ],
    },
    Preset {
        name: "Pocket Arp",
        values: &[
            ("osc1_wave", 1.0),
            ("osc1_pw", 30.0),
            ("osc2_oct", 1.0),
            ("osc2_level", 35.0),
            ("sub_level", 0.0),
            ("cutoff", 1500.0),
            ("resonance", 35.0),
            ("env_amount", 45.0),
            ("fenv_attack", 0.001),
            ("fenv_decay", 0.18),
            ("fenv_sustain", 10.0),
            ("aenv_attack", 0.001),
            ("aenv_decay", 0.25),
            ("aenv_sustain", 20.0),
            ("aenv_release", 0.15),
        ],
    },
    Preset {
        name: "Bit Hero",
        values: &[
            ("osc1_wave", 1.0),
            ("osc1_pw", 25.0),
            ("osc2_level", 0.0),
            ("sub_level", 0.0),
            ("flt_mode", 1.0),
            ("cutoff", 20000.0),
            ("resonance", 0.0),
            ("key_track", 0.0),
            ("env_amount", 0.0),
            ("vel_cutoff", 0.0),
            ("aenv_attack", 0.001),
            ("aenv_decay", 0.1),
            ("aenv_sustain", 85.0),
            ("aenv_release", 0.05),
            ("vel_sens", 0.0),
            ("lfo_shape", 3.0),
            ("lfo_rate", 7.0),
            ("lfo_pitch", 0.0),
            ("mono", 1.0),
            ("volume", -9.0),
        ],
    },
    Preset {
        name: "Wub Engine",
        values: &[
            ("osc1_oct", -1.0),
            ("osc2_wave", 1.0),
            ("osc2_oct", -1.0),
            ("osc2_level", 70.0),
            ("sub_level", 50.0),
            ("cutoff", 200.0),
            ("resonance", 45.0),
            ("env_amount", 0.0),
            ("drive", 50.0),
            ("aenv_attack", 0.003),
            ("aenv_sustain", 100.0),
            ("aenv_release", 0.1),
            ("lfo_shape", 1.0),
            ("lfo_sync", 1.0),
            ("lfo_rate_sync", 5.0),
            ("lfo_cutoff", 70.0),
            ("mono", 1.0),
            ("volume", 0.0),
        ],
    },
];

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "pulse",
    name: "Pulse Synth",
    category: "Synth",
    description: "Polyphonic virtual-analog synth: two oscillators with unison, sub and noise, a resonant multimode filter, two envelopes and an LFO.",
    params: PARAMS,
    presets: PRESETS,
    note_names: &[],
};

/// Settings derived from the parameter values (computed in `set_params`).
#[derive(Clone, Copy)]
struct Settings {
    wave: [u8; 2],
    /// Per-oscillator pitch offset in semitones (octave + semi + fine).
    offset: [f32; 2],
    pw: [f32; 2],
    level: [f32; 2],
    sub: f32,
    noise: f32,
    unison: usize,
    detune: f32,
    /// Per-unison-voice (left, right) gains, including 1/sqrt(n) loudness
    /// compensation.
    uni_gain: [(f32, f32); MAX_UNISON],
    /// Per-unison-voice detune factor in [-1, 1].
    uni_pos: [f32; MAX_UNISON],
    stereo: bool,
    mode: u8,
    cutoff: f32,
    res: f32,
    key_track: f32,
    env_amt: f32,
    vel_cut: f32,
    drive_gain: f32,
    drive_comp: f32,
    vel_sens: f32,
    lfo_shape: usize,
    lfo_sync: bool,
    lfo_rate: f32,
    lfo_rate_sync: usize,
    lfo_pitch: f32,
    lfo_cutoff: f32,
    lfo_amp: f32,
    /// Glide one-pole coefficient per control step (0 = no glide).
    glide_coef: f32,
    mono: bool,
}

#[derive(Clone)]
struct Voice {
    active: bool,
    /// Gate held (note not yet released).
    held: bool,
    note: u8,
    velocity: f32,
    vel_gain: f32,
    age: u64,
    /// Current (gliding) and target pitch in semitones.
    pitch: f32,
    target: f32,
    phase: [[f32; MAX_UNISON]; 2],
    inc: [[f32; MAX_UNISON]; 2],
    sub_phase: f32,
    sub_inc: f32,
    noise: Noise,
    filt: [[Svf; 2]; 2],
    g: f32,
    g_step: f32,
    amp_env: Adsr,
    filt_env: Adsr,
}

impl Voice {
    fn new(sr: f32) -> Self {
        Self {
            active: false,
            held: false,
            note: 0,
            velocity: 0.0,
            vel_gain: 0.0,
            age: 0,
            pitch: 60.0,
            target: 60.0,
            phase: [[0.0; MAX_UNISON]; 2],
            inc: [[0.0; MAX_UNISON]; 2],
            sub_phase: 0.0,
            sub_inc: 0.0,
            noise: Noise::new(1),
            filt: [[Svf::default(); 2]; 2],
            g: 0.1,
            g_step: 0.0,
            amp_env: Adsr::new(sr),
            filt_env: Adsr::new(sr),
        }
    }

    /// Recomputes oscillator increments and the filter coefficient target.
    /// `interp` = spread the filter change over the next control period.
    fn control(&mut self, s: &Settings, sr: f32, lfo: f32, interp: bool) {
        // Glide toward the target pitch.
        if s.glide_coef > 0.0 {
            self.pitch = self.target + (self.pitch - self.target) * s.glide_coef;
            if (self.pitch - self.target).abs() < 0.001 {
                self.pitch = self.target;
            }
        } else {
            self.pitch = self.target;
        }
        let pitch = self.pitch + lfo * s.lfo_pitch;
        let inv_sr = 1.0 / sr;
        for o in 0..2 {
            let base = midi_hz_fast(pitch + s.offset[o]) * inv_sr;
            for u in 0..s.unison {
                let ratio = semis_to_ratio(s.uni_pos[u] * s.detune);
                self.inc[o][u] = (base * ratio).min(0.45);
            }
        }
        self.sub_inc = (midi_hz_fast(pitch + s.offset[0] - 12.0) * inv_sr).min(0.45);

        // Cutoff modulation, in octaves.
        let oct = s.key_track * (self.note as f32 - 60.0) / 12.0
            + s.env_amt * self.filt_env.level
            + s.vel_cut * (self.velocity - 1.0)
            + s.lfo_cutoff * lfo;
        let fc = (s.cutoff * fast_pow2(oct)).clamp(16.0, sr * 0.45);
        let g = fast_tan(PI * fc / sr);
        if interp {
            self.g_step = (g - self.g) * (1.0 / CONTROL as f32);
        } else {
            self.g = g;
            self.g_step = 0.0;
        }
    }

    /// Renders `n` samples (n <= CONTROL), adding into `out_l`/`out_r`.
    /// `gain` holds per-sample master gain (volume * amp LFO).
    fn render(&mut self, s: &Settings, gain: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let n = gain.len();
        let res_lp24_first = s.res * 0.25;
        for i in 0..n {
            // Oscillators (unison voices panned across the stereo field).
            let mut sl = 0.0;
            let mut sum_r = 0.0;
            for u in 0..s.unison {
                let mut x = 0.0;
                for o in 0..2 {
                    if s.level[o] > 0.0 {
                        let t = self.phase[o][u];
                        let dt = self.inc[o][u];
                        x += s.level[o] * osc(s.wave[o], t, dt, s.pw[o]);
                        let mut t = t + dt;
                        if t >= 1.0 {
                            t -= 1.0;
                        }
                        self.phase[o][u] = t;
                    }
                }
                let (gl, gr) = s.uni_gain[u];
                sl += x * gl;
                sum_r += x * gr;
            }
            // Sub oscillator and noise are centered.
            let mut center = 0.0;
            if s.sub > 0.0 {
                center += s.sub * square(self.sub_phase, self.sub_inc, 0.5);
            }
            let mut t = self.sub_phase + self.sub_inc;
            if t >= 1.0 {
                t -= 1.0;
            }
            self.sub_phase = t;
            if s.noise > 0.0 {
                center += s.noise * self.noise.next();
            }
            sl += center;
            sum_r += center;

            // Drive into the filter.
            if s.drive_gain > 1.0 {
                sl = drive(sl * s.drive_gain) * s.drive_comp;
                sum_r = drive(sum_r * s.drive_gain) * s.drive_comp;
            }

            // Filter.
            self.g += self.g_step;
            let g = self.g;
            let channels = if s.stereo { 2 } else { 1 };
            let mut y = [sl, sum_r];
            for (c, yc) in y.iter_mut().enumerate().take(channels) {
                let f = &mut self.filt[c];
                *yc = match s.mode {
                    0 => {
                        f[0].set_g(g, res_lp24_first);
                        f[1].set_g(g, s.res);
                        let a = f[0].process(*yc).0;
                        f[1].process(a).0
                    }
                    1 => {
                        f[0].set_g(g, s.res);
                        f[0].process(*yc).0
                    }
                    2 => {
                        f[0].set_g(g, s.res);
                        // Normalize the band-pass so its peak is ~unity.
                        f[0].process(*yc).1 * (2.0 - 1.96 * s.res).max(0.35)
                    }
                    _ => {
                        f[0].set_g(g, s.res);
                        f[0].process(*yc).2
                    }
                };
            }
            if !s.stereo {
                y[1] = y[0];
            }

            self.filt_env.next();
            let amp = self.amp_env.next() * self.vel_gain * gain[i];
            out_l[i] += y[0] * amp;
            out_r[i] += y[1] * amp;
        }
        if !self.amp_env.is_active() {
            self.active = false;
            self.held = false;
        }
    }
}

#[inline]
fn midi_hz_fast(pitch: f32) -> f32 {
    440.0 * fast_pow2((pitch - 69.0) * (1.0 / 12.0))
}

/// Band-limited pulse wave with DC removed.
#[inline]
fn square(t: f32, dt: f32, pw: f32) -> f32 {
    let mut y = if t < pw { 1.0 } else { -1.0 };
    y += poly_blep(t, dt);
    let mut t2 = t - pw;
    if t2 < 0.0 {
        t2 += 1.0;
    }
    y -= poly_blep(t2, dt);
    y - (2.0 * pw - 1.0)
}

#[inline]
fn osc(wave: u8, t: f32, dt: f32, pw: f32) -> f32 {
    match wave {
        0 => 2.0 * t - 1.0 - poly_blep(t, dt),
        1 => square(t, dt, pw),
        2 => 1.0 - 4.0 * (t - 0.5).abs(),
        _ => fast_sin01(t),
    }
}

/// Asymmetric-free soft saturation for the filter drive.
#[inline]
fn drive(x: f32) -> f32 {
    super::util::soft_clip(x)
}

pub struct PulseSynth {
    sr: f32,
    voices: Vec<Voice>,
    s: Settings,
    volume: Smoother,
    /// Samples left in the current control period.
    ctrl_left: usize,
    lfo_phase: f32,
    lfo_sh: f32,
    lfo_val: f32,
    lfo_noise: Noise,
    /// Amp-LFO gain at the start of the control period and its slope.
    amp_lfo: f32,
    amp_lfo_step: f32,
    bpm: f32,
    age: u64,
    seed: u32,
    last_pitch: Option<f32>,
    /// Held notes in mono mode (most recent last).
    mono_stack: [u8; 16],
    mono_len: usize,
    gain_buf: [f32; CONTROL],
}

impl PulseSynth {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut synth = Self {
            sr,
            voices: (0..VOICE_SLOTS).map(|_| Voice::new(sr)).collect(),
            s: Settings {
                wave: [0; 2],
                offset: [0.0; 2],
                pw: [0.5; 2],
                level: [0.0; 2],
                sub: 0.0,
                noise: 0.0,
                unison: 1,
                detune: 0.0,
                uni_gain: [(1.0, 1.0); MAX_UNISON],
                uni_pos: [0.0; MAX_UNISON],
                stereo: false,
                mode: 0,
                cutoff: 1000.0,
                res: 0.0,
                key_track: 0.0,
                env_amt: 0.0,
                vel_cut: 0.0,
                drive_gain: 1.0,
                drive_comp: 1.0,
                vel_sens: 0.0,
                lfo_shape: 0,
                lfo_sync: false,
                lfo_rate: 1.0,
                lfo_rate_sync: 8,
                lfo_pitch: 0.0,
                lfo_cutoff: 0.0,
                lfo_amp: 0.0,
                glide_coef: 0.0,
                mono: false,
            },
            volume: Smoother::new(0.5, 20.0, sr),
            ctrl_left: 0,
            lfo_phase: 0.0,
            lfo_sh: 0.0,
            lfo_val: 0.0,
            lfo_noise: Noise::new(0x5eed),
            amp_lfo: 1.0,
            amp_lfo_step: 0.0,
            bpm: 120.0,
            age: 0,
            seed: 1,
            last_pitch: None,
            mono_stack: [0; 16],
            mono_len: 0,
            gain_buf: [0.0; CONTROL],
        };
        synth.set_params(&SPEC.default_values());
        synth.volume.snap(synth.volume.target);
        synth
    }

    fn start_voice(&mut self, idx: usize, pitch: u8, velocity: f32, retrigger_from_zero: bool) {
        self.age += 1;
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let s = self.s;
        let glide_from = if s.glide_coef > 0.0 {
            self.last_pitch
        } else {
            None
        };
        let v = &mut self.voices[idx];
        let was_active = v.active;
        v.active = true;
        v.held = true;
        v.note = pitch;
        v.velocity = velocity;
        v.vel_gain = 1.0 - s.vel_sens * (1.0 - velocity);
        v.age = self.age;
        v.target = pitch as f32;
        v.pitch = glide_from.unwrap_or(pitch as f32);
        if retrigger_from_zero || !was_active {
            // Fresh voice: reset filter state and pick deterministic
            // pseudo-random oscillator phases (analog-style free phase).
            v.noise = Noise::new(self.seed);
            for o in 0..2 {
                for u in 0..MAX_UNISON {
                    v.phase[o][u] = v.noise.next() * 0.5 + 0.5;
                }
            }
            v.sub_phase = 0.0;
            for f in v.filt.iter_mut().flatten() {
                f.reset();
            }
            v.amp_env.trigger();
            v.filt_env.trigger();
        } else {
            v.amp_env.gate_on();
            v.filt_env.gate_on();
        }
        v.control(&s, self.sr, self.lfo_val, false);
        self.last_pitch = Some(pitch as f32);
    }

    /// Chooses a voice slot for a new poly note, stealing if needed.
    fn allocate(&mut self) -> usize {
        let sounding = self
            .voices
            .iter()
            .filter(|v| v.active && v.amp_env.stage() != super::util::AdsrStage::Kill)
            .count();
        if sounding >= MAX_VOICES {
            // Steal: prefer the quietest released voice, else the oldest.
            let victim = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.active && v.amp_env.stage() != super::util::AdsrStage::Kill)
                .min_by(|(_, a), (_, b)| {
                    let ka = (
                        a.held,
                        if a.held {
                            a.age as f32
                        } else {
                            a.amp_env.level
                        },
                    );
                    let kb = (
                        b.held,
                        if b.held {
                            b.age as f32
                        } else {
                            b.amp_env.level
                        },
                    );
                    ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i);
            if let Some(i) = victim {
                self.voices[i].amp_env.kill();
                self.voices[i].held = false;
            }
        }
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // All slots busy (many fading voices): take the quietest outright.
        self.voices
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.amp_env
                    .level
                    .partial_cmp(&b.amp_env.level)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn mono_push(&mut self, pitch: u8) {
        self.mono_remove(pitch);
        if self.mono_len == self.mono_stack.len() {
            self.mono_stack.copy_within(1.., 0);
            self.mono_len -= 1;
        }
        self.mono_stack[self.mono_len] = pitch;
        self.mono_len += 1;
    }

    fn mono_remove(&mut self, pitch: u8) {
        if let Some(i) = self.mono_stack[..self.mono_len]
            .iter()
            .position(|&p| p == pitch)
        {
            self.mono_stack.copy_within(i + 1..self.mono_len, i);
            self.mono_len -= 1;
        }
    }

    fn control_update(&mut self) {
        let s = self.s;
        // Advance the LFO by one control period.
        let rate = if s.lfo_sync {
            1.0 / sync_seconds(s.lfo_rate_sync, self.bpm)
        } else {
            s.lfo_rate
        };
        self.lfo_phase += rate * CONTROL as f32 / self.sr;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= self.lfo_phase.floor();
            self.lfo_sh = self.lfo_noise.next();
        }
        self.lfo_val = lfo_value(s.lfo_shape, self.lfo_phase, self.lfo_sh);
        let amp = 1.0 - s.lfo_amp * (0.5 - 0.5 * self.lfo_val);
        self.amp_lfo_step = (amp - self.amp_lfo) / CONTROL as f32;

        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.control(&s, self.sr, self.lfo_val, true);
        }
    }
}

impl Instrument for PulseSynth {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let velocity = velocity.clamp(0.0, 1.0);
        if self.s.mono {
            self.mono_push(pitch);
            let v = &self.voices[0];
            if v.active && v.held {
                // Legato: move the pitch without retriggering envelopes.
                let s = self.s;
                let v = &mut self.voices[0];
                v.note = pitch;
                v.target = pitch as f32;
                if s.glide_coef <= 0.0 {
                    v.pitch = v.target;
                }
                v.control(&s, self.sr, self.lfo_val, false);
                self.last_pitch = Some(pitch as f32);
            } else {
                let from_zero = !self.voices[0].active;
                self.start_voice(0, pitch, velocity, from_zero);
            }
            return;
        }
        // Re-striking a held/releasing note reuses its voice (no click).
        if let Some(i) = self.voices.iter().position(|v| {
            v.active && v.note == pitch && v.amp_env.stage() != super::util::AdsrStage::Kill
        }) {
            self.start_voice(i, pitch, velocity, false);
            return;
        }
        let i = self.allocate();
        self.start_voice(i, pitch, velocity, true);
    }

    fn note_off(&mut self, pitch: u8) {
        if self.s.mono {
            self.mono_remove(pitch);
            let v = &self.voices[0];
            if !(v.active && v.held && v.note == pitch) {
                return;
            }
            if self.mono_len > 0 {
                // Return (legato) to the most recent still-held note.
                let back = self.mono_stack[self.mono_len - 1];
                let s = self.s;
                let v = &mut self.voices[0];
                v.note = back;
                v.target = back as f32;
                if s.glide_coef <= 0.0 {
                    v.pitch = v.target;
                }
                v.control(&s, self.sr, self.lfo_val, false);
                self.last_pitch = Some(back as f32);
            } else {
                let v = &mut self.voices[0];
                v.held = false;
                v.amp_env.gate_off();
                v.filt_env.gate_off();
            }
            return;
        }
        for v in self
            .voices
            .iter_mut()
            .filter(|v| v.active && v.held && v.note == pitch)
        {
            v.held = false;
            v.amp_env.gate_off();
            v.filt_env.gate_off();
        }
    }

    fn all_notes_off(&mut self, hard: bool) {
        self.mono_len = 0;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.held = false;
            if hard {
                v.amp_env.kill();
            } else {
                v.amp_env.gate_off();
            }
            v.filt_env.gate_off();
        }
    }

    fn set_params(&mut self, p: &ParamValues) {
        let sr = self.sr;
        let s = &mut self.s;
        let waves = [p[P_OSC1_WAVE], p[P_OSC2_WAVE]];
        let octs = [p[P_OSC1_OCT], p[P_OSC2_OCT]];
        let semis = [p[P_OSC1_SEMI], p[P_OSC2_SEMI]];
        let fines = [p[P_OSC1_FINE], p[P_OSC2_FINE]];
        let pws = [p[P_OSC1_PW], p[P_OSC2_PW]];
        let levels = [p[P_OSC1_LEVEL], p[P_OSC2_LEVEL]];
        for o in 0..2 {
            s.wave[o] = waves[o].round().clamp(0.0, 3.0) as u8;
            s.offset[o] = octs[o].round() * 12.0 + semis[o].round() + fines[o] * 0.01;
            s.pw[o] = (pws[o] * 0.01).clamp(0.05, 0.95);
            s.level[o] = levels[o] * 0.01;
        }
        s.sub = p[P_SUB] * 0.01;
        s.noise = p[P_NOISE] * 0.01 * 0.7;
        s.unison = (p[P_UNISON].round() as usize).clamp(1, MAX_UNISON);
        s.detune = p[P_UNI_DETUNE] * 0.01;
        let spread = p[P_UNI_SPREAD] * 0.01;
        let n = s.unison;
        let norm = 1.0 / (n as f32).sqrt();
        for u in 0..MAX_UNISON {
            let pos = if n > 1 {
                2.0 * u as f32 / (n - 1) as f32 - 1.0
            } else {
                0.0
            };
            s.uni_pos[u] = pos;
            // Equal-power pan normalized to unity at center.
            let a = (pos * spread + 1.0) * 0.25 * PI;
            let (gl, gr) = (
                a.cos() * std::f32::consts::SQRT_2,
                a.sin() * std::f32::consts::SQRT_2,
            );
            s.uni_gain[u] = (gl * norm, gr * norm);
        }
        s.stereo = n > 1 && spread > 0.0;
        s.mode = p[P_FLT_MODE].round().clamp(0.0, 3.0) as u8;
        s.cutoff = p[P_CUTOFF];
        s.res = (p[P_RES] * 0.01).clamp(0.0, 1.0);
        s.key_track = p[P_KEYTRACK] * 0.01;
        s.env_amt = p[P_ENV_AMT] * 0.01 * 6.0;
        s.vel_cut = p[P_VEL_CUT] * 0.01 * 3.0;
        let drive = p[P_DRIVE] * 0.01;
        s.drive_gain = 1.0 + drive * 5.0;
        s.drive_comp = 1.0 / (1.0 + drive * 1.5);
        s.vel_sens = p[P_VEL_SENS] * 0.01;
        s.lfo_shape = p[P_LFO_SHAPE].round().max(0.0) as usize;
        s.lfo_sync = p[P_LFO_SYNC] >= 0.5;
        s.lfo_rate = p[P_LFO_RATE];
        s.lfo_rate_sync = p[P_LFO_RATE_SYNC].round().max(0.0) as usize;
        s.lfo_pitch = p[P_LFO_PITCH];
        s.lfo_cutoff = p[P_LFO_CUTOFF] * 0.01 * 3.0;
        s.lfo_amp = p[P_LFO_AMP] * 0.01;
        let glide_ms = p[P_GLIDE];
        s.glide_coef = if glide_ms <= 1.5 {
            0.0
        } else {
            // Time to cover ~99% of the interval.
            (-(CONTROL as f32) * 4.6 / (glide_ms * 0.001 * sr)).exp()
        };
        let mono = p[P_MONO] >= 0.5;
        if mono != s.mono {
            s.mono = mono;
            self.mono_len = 0;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.held = false;
                v.amp_env.gate_off();
                v.filt_env.gate_off();
            }
        }
        let (fa, fd, fs, fr) = (p[P_FENV_A], p[P_FENV_D], p[P_FENV_S] * 0.01, p[P_FENV_R]);
        let (aa, ad, as_, ar) = (p[P_AENV_A], p[P_AENV_D], p[P_AENV_S] * 0.01, p[P_AENV_R]);
        for v in self.voices.iter_mut() {
            v.filt_env.set(fa, fd, fs, fr, sr);
            v.amp_env.set(aa, ad, as_, ar, sr);
        }
        self.volume.set(db_to_gain(p[P_VOLUME]) * VOICE_GAIN);
    }

    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let frames = l.len().min(r.len());
        self.bpm = ctx.bpm;
        let mut start = 0;
        while start < frames {
            if self.ctrl_left == 0 {
                self.control_update();
                self.ctrl_left = CONTROL;
            }
            let n = self.ctrl_left.min(frames - start);
            // Per-sample master gain (smoothed volume x amp LFO).
            for g in self.gain_buf[..n].iter_mut() {
                self.amp_lfo += self.amp_lfo_step;
                *g = self.volume.next() * self.amp_lfo;
            }
            let s = self.s;
            let gain = &self.gain_buf[..n];
            let (ol, or) = (&mut l[start..start + n], &mut r[start..start + n]);
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.render(&s, gain, ol, or);
            }
            self.ctrl_left -= n;
            start += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::util::test_support::*;
    use super::*;

    fn synth_with(values: &ParamValues, sr: f32) -> PulseSynth {
        let mut s = PulseSynth::new(sr);
        s.set_params(values);
        s.volume.snap(s.volume.target);
        s
    }

    fn active_voices(s: &PulseSynth) -> usize {
        s.voices.iter().filter(|v| v.active).count()
    }

    #[test]
    fn spec_is_valid() {
        check_spec(&SPEC);
        assert_eq!(SPEC.params.len(), P_VOLUME + 1);
        assert!(SPEC.presets.len() >= 12);
        // Index constants must match the spec order.
        assert_eq!(SPEC.index_of("cutoff"), Some(P_CUTOFF));
        assert_eq!(SPEC.index_of("lfo_amp"), Some(P_LFO_AMP));
        assert_eq!(SPEC.index_of("aenv_release"), Some(P_AENV_R));
        assert_eq!(SPEC.index_of("drive"), Some(P_DRIVE));
    }

    #[test]
    fn single_note_level_and_release() {
        for &sr in &[48000.0, 44100.0] {
            let mut s = synth_with(&SPEC.default_values(), sr);
            s.note_on(60, 1.0);
            let (l, r) = render(&mut s, sr, sr as usize, 256);
            assert_finite(&l, "l");
            assert_finite(&r, "r");
            let p = peak(&l).max(peak(&r));
            assert!((0.05..=1.0).contains(&p), "peak {p} at {sr}");
            s.note_off(60);
            let (l, _) = render(&mut s, sr, sr as usize * 2, 256);
            let tail = peak(&l[l.len() - 1000..]);
            assert!(tail < 1e-4, "tail {tail}");
            assert_eq!(active_voices(&s), 0, "voice not freed");
        }
    }

    #[test]
    fn output_has_no_dc() {
        let mut s = synth_with(&SPEC.default_values(), 48000.0);
        s.note_on(48, 0.8);
        let (l, _) = render(&mut s, 48000.0, 48000, 512);
        let mean = l[4800..].iter().sum::<f32>() / (l.len() - 4800) as f32;
        assert!(mean.abs() < 0.01, "dc {mean}");
    }

    #[test]
    fn all_presets_render_cleanly() {
        for preset in SPEC.presets {
            let v = preset_values(&SPEC, preset);
            for &sr in &[48000.0, 44100.0] {
                let mut s = synth_with(&v, sr);
                // A chord, then a single low note.
                for &n in &[48, 55, 60, 64] {
                    s.note_on(n, 0.9);
                }
                let (l, r) = render(&mut s, sr, 24000, 512);
                for &n in &[48, 55, 60, 64] {
                    s.note_off(n);
                }
                let (l2, r2) = render(&mut s, sr, 12000, 512);
                for b in [&l, &r, &l2, &r2] {
                    assert_finite(b, preset.name);
                    assert!(peak(b) < 1.5, "{}: peak {}", preset.name, peak(b));
                }
                assert!(peak(&l) > 0.02, "{}: silent", preset.name);

                let mut s = synth_with(&v, sr);
                s.note_on(36, 1.0);
                let (l, _) = render(&mut s, sr, 24000, 512);
                let p = peak(&l);
                assert!(
                    (0.05..=1.0).contains(&p),
                    "{}: single note peak {p}",
                    preset.name
                );
            }
        }
    }

    #[test]
    fn block_size_independence() {
        // Exercise unison, LFO, glide and filter envelope.
        let mut v = preset_values(&SPEC, &PRESETS[5]);
        v = with(&SPEC, v, "noise_level", 20.0);
        v = with(&SPEC, v, "lfo_cutoff", 40.0);
        v = with(&SPEC, v, "lfo_amp", 30.0);
        let reference = {
            let mut s = synth_with(&v, 48000.0);
            s.note_on(60, 0.9);
            let (a, _) = render(&mut s, 48000.0, 3000, 512);
            s.note_on(67, 0.7);
            let (b, _) = render(&mut s, 48000.0, 3000, 512);
            [a, b].concat()
        };
        for &bs in &[1, 7, 512] {
            let mut s = synth_with(&v, 48000.0);
            s.note_on(60, 0.9);
            let (a, _) = render(&mut s, 48000.0, 3000, bs);
            s.note_on(67, 0.7);
            let (b, _) = render(&mut s, 48000.0, 3000, bs);
            let out = [a, b].concat();
            let diff = out
                .iter()
                .zip(&reference)
                .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
            assert!(diff < 1e-5, "block {bs}: diff {diff}");
        }
    }

    #[test]
    fn voice_stealing_limits_polyphony() {
        let mut s = synth_with(&SPEC.default_values(), 48000.0);
        for n in 0..40u8 {
            s.note_on(30 + n, 0.8);
            let (l, _) = render(&mut s, 48000.0, 64, 64);
            assert_finite(&l, "steal");
            let sounding = s
                .voices
                .iter()
                .filter(|v| v.active && v.amp_env.stage() != super::super::util::AdsrStage::Kill)
                .count();
            assert!(sounding <= MAX_VOICES);
        }
        // 16 simultaneous saws can legitimately get loud; just stay sane.
        let (l, _) = render(&mut s, 48000.0, 4800, 512);
        assert_finite(&l, "steal tail");
        assert!(peak(&l) < 4.0);
    }

    #[test]
    fn hard_all_notes_off_is_fast() {
        let mut s = synth_with(&SPEC.default_values(), 48000.0);
        for &n in &[48, 52, 55] {
            s.note_on(n, 1.0);
        }
        render(&mut s, 48000.0, 4800, 512);
        s.all_notes_off(true);
        let (l, _) = render(&mut s, 48000.0, 960, 64);
        assert!(peak(&l[480..]) < 1e-3);
        assert_eq!(active_voices(&s), 0);
    }

    #[test]
    fn soft_all_notes_off_releases() {
        let mut s = synth_with(&SPEC.default_values(), 48000.0);
        s.note_on(60, 1.0);
        render(&mut s, 48000.0, 4800, 512);
        s.all_notes_off(false);
        let (l, _) = render(&mut s, 48000.0, 480, 64);
        assert!(peak(&l) > 0.01, "should still ring during release");
        render(&mut s, 48000.0, 96000, 512);
        assert_eq!(active_voices(&s), 0);
    }

    #[test]
    fn set_params_does_not_reset_voices() {
        let mut s = synth_with(&SPEC.default_values(), 48000.0);
        s.note_on(60, 1.0);
        render(&mut s, 48000.0, 4800, 512);
        let mut v = SPEC.default_values();
        for step in 0..20 {
            v[P_CUTOFF] = 300.0 + step as f32 * 400.0;
            v[P_RES] = step as f32 * 5.0;
            s.set_params(&v);
            let (l, _) = render(&mut s, 48000.0, 1440, 512);
            assert_finite(&l, "sweep");
            assert!(peak(&l) > 0.01, "voice died at step {step}");
        }
        assert_eq!(active_voices(&s), 1);
    }

    #[test]
    fn mono_legato_and_glide() {
        let v = with(
            &SPEC,
            with(&SPEC, SPEC.default_values(), "mono", 1.0),
            "glide",
            100.0,
        );
        let mut s = synth_with(&v, 48000.0);
        s.note_on(60, 1.0);
        render(&mut s, 48000.0, 4800, 512);
        s.note_on(72, 1.0);
        assert_eq!(active_voices(&s), 1);
        // Envelope is not retriggered and pitch glides.
        render(&mut s, 48000.0, 480, 512);
        let p = s.voices[0].pitch;
        assert!(p > 60.5 && p < 71.5, "pitch should be gliding: {p}");
        render(&mut s, 48000.0, 9600, 512);
        assert!((s.voices[0].pitch - 72.0).abs() < 0.01);
        // Releasing the top note returns to the held one.
        s.note_off(72);
        assert_eq!(s.voices[0].target, 60.0);
        assert!(s.voices[0].held);
        s.note_off(60);
        assert!(!s.voices[0].held);
    }

    #[test]
    fn extreme_settings_are_stable() {
        let mut v = SPEC.default_values();
        for p in [P_RES, P_DRIVE, P_NOISE, P_SUB] {
            v[p] = 100.0;
        }
        v[P_ENV_AMT] = 100.0;
        v[P_LFO_CUTOFF] = 100.0;
        v[P_LFO_PITCH] = 12.0;
        v[P_LFO_RATE] = 30.0;
        v[P_UNISON] = 4.0;
        v[P_UNI_DETUNE] = 100.0;
        for mode in 0..4 {
            for &cut in &[20.0, 20000.0] {
                v[P_FLT_MODE] = mode as f32;
                v[P_CUTOFF] = cut;
                let mut s = synth_with(&v, 44100.0);
                for &n in &[0u8, 60, 127] {
                    s.note_on(n, 1.0);
                }
                let (l, r) = render(&mut s, 44100.0, 22050, 512);
                assert_finite(&l, "extreme l");
                assert_finite(&r, "extreme r");
                assert!(peak(&l) < 4.0, "mode {mode} cut {cut}: {}", peak(&l));
            }
        }
    }

    /// Rough CPU check (run with `cargo test --release -- --ignored`).
    #[test]
    #[ignore]
    fn bench_16_voices() {
        let mut v = SPEC.default_values();
        v[P_UNISON] = 2.0;
        let mut s = synth_with(&v, 48000.0);
        for n in 0..16u8 {
            s.note_on(40 + n * 2, 0.8);
        }
        let t = std::time::Instant::now();
        let (l, _) = render(&mut s, 48000.0, 48000 * 5, 512);
        let el = t.elapsed().as_secs_f64();
        println!("16 voices x2 unison: {:.2}% of a core", el / 5.0 * 100.0);
        assert!(peak(&l) > 0.0);
    }
}
