//! "Keys": electric piano / organ / soft piano instrument.
//!
//! Three engines share one 16-voice pool:
//! - **Tine EP**: two-operator phase modulation (1:1) whose index depends on
//!   velocity ("bark") and decays quickly, plus a short inharmonic "tine"
//!   partial for the attack bell.
//! - **Drawbar Organ**: nine additive sine partials with drawbar-style
//!   levels (live while notes are held), key click, and a rotary-ish
//!   Doppler/amplitude modulation on the output.
//! - **Soft Piano**: ten slightly inharmonic, slightly detuned decaying
//!   partials (higher ones decay faster) plus a felt hammer thump.
//!
//! A tremolo / auto-pan and output volume are common to all modes.

// Parallel per-partial arrays read clearest with index loops.
#![allow(clippy::needless_range_loop)]

use super::util::{DelayLine, Noise, Smoother, Svf, decay_coef, fast_sin01, flush, pan_gains};
use super::*;

const MODES: &[&str] = &["Tine EP", "Drawbar Organ", "Soft Piano"];
const ROTARY: &[&str] = &["Off", "Slow", "Fast"];

const P_MODE: usize = 0;
const P_TONE: usize = 1;
const P_DECAY: usize = 2;
const P_RELEASE: usize = 3;
const P_VEL: usize = 4;
const P_OCTAVE: usize = 5;
const P_TREM_RATE: usize = 6;
const P_TREM_DEPTH: usize = 7;
const P_TREM_PAN: usize = 8;
const P_VOLUME: usize = 9;
const P_BARK: usize = 10;
const P_BELL: usize = 11;
/// First of the nine drawbar params.
const P_BAR: usize = 12;
const P_CLICK: usize = 21;
const P_ROTARY: usize = 22;
const P_HAMMER: usize = 23;
const P_DETUNE: usize = 24;

const MODE_EP: u8 = 0;
const MODE_ORGAN: u8 = 1;
const MODE_PIANO: u8 = 2;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "keys",
    name: "Keys",
    category: "Keys",
    description: "Electric piano, drawbar organ and soft piano in one instrument.",
    params: &[
        ParamSpec::choice("mode", "Mode", MODES, 0, "Global"),
        ParamSpec::lin("tone", "Tone", 0.0, 100.0, 50.0, "%", "Global"),
        ParamSpec::lin("decay", "Decay", 0.0, 100.0, 50.0, "%", "Global"),
        ParamSpec::log("release", "Release", 0.01, 3.0, 0.3, "s", "Global"),
        ParamSpec::lin("vel_sens", "Velocity", 0.0, 100.0, 70.0, "%", "Global"),
        ParamSpec::int("octave", "Octave", -2.0, 2.0, 0.0, "Global"),
        ParamSpec::log("trem_rate", "Rate", 0.5, 12.0, 4.5, "Hz", "Tremolo"),
        ParamSpec::lin("trem_depth", "Depth", 0.0, 100.0, 0.0, "%", "Tremolo"),
        ParamSpec::toggle("trem_pan", "Auto-Pan", false, "Tremolo"),
        ParamSpec::lin("volume", "Volume", -24.0, 6.0, 0.0, "dB", "Global"),
        ParamSpec::lin("bark", "Bark", 0.0, 100.0, 50.0, "%", "Tine EP"),
        ParamSpec::lin("bell", "Bell", 0.0, 100.0, 30.0, "%", "Tine EP"),
        ParamSpec::int("bar1", "16'", 0.0, 8.0, 8.0, "Organ"),
        ParamSpec::int("bar2", "5 1/3'", 0.0, 8.0, 8.0, "Organ"),
        ParamSpec::int("bar3", "8'", 0.0, 8.0, 8.0, "Organ"),
        ParamSpec::int("bar4", "4'", 0.0, 8.0, 0.0, "Organ"),
        ParamSpec::int("bar5", "2 2/3'", 0.0, 8.0, 0.0, "Organ"),
        ParamSpec::int("bar6", "2'", 0.0, 8.0, 0.0, "Organ"),
        ParamSpec::int("bar7", "1 3/5'", 0.0, 8.0, 0.0, "Organ"),
        ParamSpec::int("bar8", "1 1/3'", 0.0, 8.0, 0.0, "Organ"),
        ParamSpec::int("bar9", "1'", 0.0, 8.0, 0.0, "Organ"),
        ParamSpec::lin("click", "Key Click", 0.0, 100.0, 40.0, "%", "Organ"),
        ParamSpec::choice("rotary", "Rotary", ROTARY, 1, "Organ"),
        ParamSpec::lin("hammer", "Hammer", 0.0, 100.0, 40.0, "%", "Piano"),
        ParamSpec::lin("detune", "Detune", 0.0, 100.0, 30.0, "%", "Piano"),
    ],
    presets: &[
        Preset {
            name: "Velvet Tines",
            values: &[
                ("mode", 0.0),
                ("tone", 40.0),
                ("bark", 35.0),
                ("bell", 25.0),
                ("trem_rate", 3.8),
                ("trem_depth", 35.0),
                ("trem_pan", 1.0),
            ],
        },
        Preset {
            name: "Bright Bark EP",
            values: &[
                ("mode", 0.0),
                ("tone", 75.0),
                ("bark", 90.0),
                ("bell", 45.0),
                ("vel_sens", 85.0),
            ],
        },
        Preset {
            name: "Gospel Drawbars",
            values: &[
                ("mode", 1.0),
                ("bar1", 8.0),
                ("bar2", 8.0),
                ("bar3", 8.0),
                ("bar4", 8.0),
                ("bar5", 0.0),
                ("bar6", 0.0),
                ("bar7", 0.0),
                ("bar8", 0.0),
                ("bar9", 8.0),
                ("click", 55.0),
                ("rotary", 2.0),
                ("vel_sens", 0.0),
                ("release", 0.04),
                ("volume", -3.0),
            ],
        },
        Preset {
            name: "Jazz Organ",
            values: &[
                ("mode", 1.0),
                ("bar1", 8.0),
                ("bar2", 8.0),
                ("bar3", 8.0),
                ("click", 70.0),
                ("rotary", 1.0),
                ("vel_sens", 0.0),
                ("release", 0.03),
            ],
        },
        Preset {
            name: "Chapel Flutes",
            values: &[
                ("mode", 1.0),
                ("bar1", 0.0),
                ("bar2", 0.0),
                ("bar3", 8.0),
                ("bar4", 6.0),
                ("bar5", 0.0),
                ("bar6", 4.0),
                ("bar7", 0.0),
                ("bar8", 0.0),
                ("bar9", 0.0),
                ("click", 5.0),
                ("rotary", 0.0),
                ("vel_sens", 0.0),
                ("release", 0.4),
                ("tone", 35.0),
            ],
        },
        Preset {
            name: "Felt Upright",
            values: &[
                ("mode", 2.0),
                ("tone", 25.0),
                ("hammer", 55.0),
                ("detune", 40.0),
                ("release", 0.35),
            ],
        },
        Preset {
            name: "Glass Grand",
            values: &[
                ("mode", 2.0),
                ("tone", 80.0),
                ("hammer", 30.0),
                ("detune", 20.0),
                ("decay", 70.0),
            ],
        },
        Preset {
            name: "Music Box",
            values: &[
                ("mode", 0.0),
                ("octave", 2.0),
                ("tone", 30.0),
                ("bark", 10.0),
                ("bell", 90.0),
                ("decay", 15.0),
                ("release", 0.6),
                ("vel_sens", 40.0),
            ],
        },
    ],
    note_names: &[],
};

/// Drawbar harmonic ratios (sub-octave, sub-fifth, fundamental, ...).
const BAR_RATIOS: [f32; 9] = [0.5, 1.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0];
/// Partials per voice (piano uses all ten, organ nine, EP three).
const NP: usize = 10;
const MAX_POLY: usize = 16;
/// A few spare slots so stolen voices can fade out instead of clicking.
const NUM_SLOTS: usize = 20;
const CHUNK: usize = 32;
const SILENCE: f32 = 1.0e-4;

/// Decay-knob mapping: 50 % = nominal, 0 % ≈ x0.35, 100 % ≈ x2.8.
fn decay_mult(pct: f32) -> f32 {
    2f32.powf((pct - 50.0) / 50.0 * 1.5)
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    gate: bool,
    pitch: u8,
    mode: u8,
    age: u64,
    /// Main envelope: linear attack, `hold_coef` decay while held,
    /// `rel_coef` after release.
    env: f32,
    att_inc: f32,
    attacking: bool,
    hold_coef: f32,
    rel_coef: f32,
    gl: f32,
    gr: f32,
    ph: [f32; NP],
    inc: [f32; NP],
    amp: [f32; NP],
    coef: [f32; NP],
    /// EP modulation index (in cycles) and its decay towards `idx_floor`.
    idx: f32,
    idx_floor: f32,
    idx_coef: f32,
    /// Click / hammer noise transient.
    noise: Noise,
    nf: Svf,
    nenv: f32,
    ncoef: f32,
    nlvl: f32,
}

impl Voice {
    fn idle() -> Self {
        Self {
            active: false,
            gate: false,
            pitch: 0,
            mode: 0,
            age: 0,
            env: 0.0,
            att_inc: 1.0,
            attacking: false,
            hold_coef: 1.0,
            rel_coef: 0.0,
            gl: 0.0,
            gr: 0.0,
            ph: [0.0; NP],
            inc: [0.0; NP],
            amp: [0.0; NP],
            coef: [1.0; NP],
            idx: 0.0,
            idx_floor: 0.0,
            idx_coef: 1.0,
            noise: Noise::new(1),
            nf: Svf::default(),
            nenv: 0.0,
            ncoef: 0.0,
            nlvl: 0.0,
        }
    }

    #[inline]
    fn advance_env(&mut self) -> f32 {
        if self.attacking {
            self.env += self.att_inc;
            if self.env >= 1.0 {
                self.env = 1.0;
                self.attacking = false;
            }
        } else if self.gate {
            self.env *= self.hold_coef;
        } else {
            self.env *= self.rel_coef;
        }
        self.env
    }

    #[inline]
    fn transient(&mut self) -> f32 {
        if self.nenv < SILENCE {
            return 0.0;
        }
        let n = self.nf.process(self.noise.next()).1 * self.nenv * self.nlvl;
        self.nenv *= self.ncoef;
        n
    }

    /// Renders into the chunk buffers. `bars` = live organ drawbar gains.
    fn render(&mut self, bl: &mut [f32], br: &mut [f32], bars: &[f32; 9]) {
        for (l, r) in bl.iter_mut().zip(br.iter_mut()) {
            let env = self.advance_env();
            let s = match self.mode {
                MODE_EP => {
                    self.ph[0] = wrap(self.ph[0] + self.inc[0]);
                    self.ph[1] = wrap(self.ph[1] + self.inc[1]);
                    self.ph[2] = wrap(self.ph[2] + self.inc[2]);
                    let m = fast_sin01(self.ph[1]) * self.idx;
                    self.idx = self.idx_floor + (self.idx - self.idx_floor) * self.idx_coef;
                    let c = fast_sin01(self.ph[0] + m);
                    let b = fast_sin01(self.ph[2]) * self.amp[2];
                    self.amp[2] *= self.coef[2];
                    (c * self.amp[0] + b) * env
                }
                MODE_ORGAN => {
                    let mut sum = 0.0;
                    for i in 0..9 {
                        self.ph[i] = wrap(self.ph[i] + self.inc[i]);
                        sum += fast_sin01(self.ph[i]) * bars[i] * self.amp[i];
                    }
                    sum * env + self.transient()
                }
                _ => {
                    let mut sum = 0.0;
                    for i in 0..NP {
                        self.ph[i] = wrap(self.ph[i] + self.inc[i]);
                        sum += fast_sin01(self.ph[i]) * self.amp[i];
                        self.amp[i] = flush(self.amp[i] * self.coef[i]);
                    }
                    sum * env + self.transient() * env.min(1.0)
                }
            };
            *l += s * self.gl;
            *r += s * self.gr;
        }
        // Voice end: envelope (or, for the piano/EP, the partials) gone.
        let partials_done = match self.mode {
            MODE_PIANO => self.amp[0] < SILENCE && self.amp[1] < SILENCE,
            _ => false,
        };
        if !self.attacking && (self.env < SILENCE || partials_done) && self.nenv < SILENCE {
            self.active = false;
        }
    }
}

#[inline]
fn wrap(p: f32) -> f32 {
    if p >= 1.0 { p - 1.0 } else { p }
}

pub struct Keys {
    sr: f32,
    params: ParamValues,
    voices: [Voice; NUM_SLOTS],
    counter: u64,
    buf_l: [f32; CHUNK],
    buf_r: [f32; CHUNK],
    /// Absolute sample counter: control-rate updates happen on fixed
    /// boundaries so the output doesn't depend on block sizes.
    clock: u64,
    bar_target: [f32; 9],
    bar_gain: [f32; 9],
    bar_coef: f32,
    kill_coef: f32,
    // Tremolo / auto-pan.
    trem_phase: f32,
    trem_inc: f32,
    trem_depth: Smoother,
    trem_pan: bool,
    // Rotary.
    rot_delay: DelayLine,
    rot_phase: f32,
    rot_speed: Smoother,
    rot_wet: Smoother,
    volume: Smoother,
}

impl Keys {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut k = Self {
            sr,
            params: SPEC.default_values(),
            voices: [Voice::idle(); NUM_SLOTS],
            counter: 0,
            buf_l: [0.0; CHUNK],
            buf_r: [0.0; CHUNK],
            clock: 0,
            bar_target: [0.0; 9],
            bar_gain: [0.0; 9],
            // Drawbar moves glide over ~20 ms (updated every CHUNK samples).
            bar_coef: (-(CHUNK as f32) / (0.02 * sr)).exp(),
            kill_coef: decay_coef(0.004, sr),
            trem_phase: 0.0,
            trem_inc: 0.0,
            trem_depth: Smoother::new(0.0, 30.0, sr),
            trem_pan: false,
            rot_delay: DelayLine::new((0.012 * sr) as usize),
            rot_phase: 0.0,
            rot_speed: Smoother::new(0.8, 900.0, sr),
            rot_wet: Smoother::new(0.0, 60.0, sr),
            volume: Smoother::new(1.0, 30.0, sr),
        };
        let v = k.params;
        k.set_params(&v);
        k.bar_gain = k.bar_target;
        k.trem_depth.snap(k.trem_depth.target);
        k.rot_speed.snap(k.rot_speed.target);
        k.rot_wet.snap(k.rot_wet.target);
        k.volume.snap(k.volume.target);
        k
    }

    fn mode(&self) -> u8 {
        (self.params[P_MODE].round().clamp(0.0, 2.0)) as u8
    }

    fn alloc(&mut self) -> usize {
        // Enforce the polyphony limit by fading the oldest sounding note.
        let sounding = self
            .voices
            .iter()
            .filter(|v| v.active && v.rel_coef != self.kill_coef)
            .count();
        if sounding >= MAX_POLY {
            let kc = self.kill_coef;
            let oldest = self
                .voices
                .iter_mut()
                .filter(|v| v.active && v.rel_coef != kc)
                .min_by_key(|v| (v.gate, v.age));
            if let Some(v) = oldest {
                v.gate = false;
                v.attacking = false;
                v.rel_coef = kc;
                v.ncoef = kc;
            }
        }
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // All slots busy (many fades in flight): hard-steal the quietest.
        let mut best = 0;
        for i in 1..NUM_SLOTS {
            if self.voices[i].env < self.voices[best].env {
                best = i;
            }
        }
        best
    }

    fn make_voice(&self, pitch: u8, vel: f32, seed: u32) -> Voice {
        let sr = self.sr;
        let p = &self.params;
        let mode = self.mode();
        let tone = p[P_TONE] / 100.0;
        let dm = decay_mult(p[P_DECAY]);
        let vs = p[P_VEL] / 100.0;
        let vel_gain = 1.0 - vs + vs * vel.powf(1.4);
        let note = pitch as f32 + 12.0 * p[P_OCTAVE].round();
        let f = midi_to_hz(note).min(sr * 0.45);
        let nyq = sr * 0.45;
        // Higher notes decay faster.
        let key_decay = 2f32.powf(-(note - 60.0) / 30.0);

        let mut v = Voice::idle();
        v.active = true;
        v.gate = true;
        v.pitch = pitch;
        v.mode = mode;
        v.age = self.counter;
        v.attacking = true;
        v.noise = Noise::new(seed);
        v.rel_coef = decay_coef(p[P_RELEASE], sr);
        let mut seedn = Noise::new(seed ^ 0x5bd1_e995);

        let (gain, pan) = match mode {
            MODE_EP => {
                v.att_inc = 1.0 / (0.0015 * sr);
                v.hold_coef = decay_coef((4.5 * key_decay * dm).clamp(0.2, 12.0), sr);
                v.inc[0] = f / sr;
                v.inc[1] = f / sr;
                // Tine partial: inharmonic, short.
                let fb = f * 7.03;
                v.inc[2] = if fb < nyq { fb / sr } else { 0.0 };
                let bell = p[P_BELL] / 100.0;
                v.amp[0] = 1.0;
                v.amp[2] = if fb < nyq {
                    bell * 0.25 * (0.4 + 0.6 * vel)
                } else {
                    0.0
                };
                v.coef[2] = decay_coef(0.25 * key_decay.sqrt(), sr);
                // Modulation index (cycles): brighter with tone, velocity
                // and bark; reduced up the keyboard to limit aliasing.
                let bark = p[P_BARK] / 100.0;
                let key_scale = (1.0 - (note - 60.0) / 60.0).clamp(0.15, 1.3);
                let base = 0.08 + 0.25 * tone;
                v.idx = (base + bark * 0.55 * vel * vel) * key_scale;
                v.idx_floor = base * 0.25 * key_scale;
                v.idx_coef = decay_coef((0.9 * key_decay).clamp(0.1, 2.0), sr);
                (
                    0.3 * vel_gain,
                    ((note - 64.0) / 64.0 * 0.3).clamp(-0.4, 0.4),
                )
            }
            MODE_ORGAN => {
                v.att_inc = 1.0 / (0.004 * sr);
                v.hold_coef = 1.0;
                for i in 0..9 {
                    let fi = f * BAR_RATIOS[i];
                    if fi < nyq {
                        v.inc[i] = fi / sr;
                        v.amp[i] = 1.0;
                    }
                    v.ph[i] = 0.5 + 0.5 * seedn.next();
                }
                let click = p[P_CLICK] / 100.0;
                v.nf.set((2500.0 + 1500.0 * tone).min(nyq), 0.3, sr);
                v.nenv = 1.0;
                v.ncoef = decay_coef(0.008, sr);
                v.nlvl = click * 1.2;
                (vel_gain, 0.0)
            }
            _ => {
                v.att_inc = 1.0 / (0.001 * sr);
                v.hold_coef = 1.0;
                let detune = p[P_DETUNE] / 100.0;
                // Stiff-string inharmonicity, stronger in the low register.
                let b = 0.0002 * key_decay;
                let bright = (0.6 * tone + 0.4 * vel).clamp(0.0, 1.0);
                let rolloff = 2.4 - 1.4 * bright;
                let base_decay = (7.0 * key_decay * dm).clamp(0.3, 20.0);
                let mut sum = 0.0;
                for i in 0..8 {
                    let n = (i + 1) as f32;
                    let cents = detune * 2.5 * if i % 2 == 0 { 1.0 } else { -1.0 };
                    let fi = f * n * (1.0 + b * n * n).sqrt() * (1.0 + cents / 1731.0);
                    if fi < nyq {
                        v.inc[i] = fi / sr;
                        v.amp[i] = n.powf(-rolloff);
                        sum += v.amp[i];
                    }
                    v.coef[i] = decay_coef(base_decay / (1.0 + 0.6 * (n - 1.0)), sr);
                }
                // Second "strings" for the two lowest partials, detuned the
                // other way: slow beating gives the chorused piano bloom.
                for (j, src) in [(8usize, 0usize), (9, 1)] {
                    let cents = detune * 4.0 + 0.3;
                    v.inc[j] = v.inc[src] * (1.0 - cents / 1731.0);
                    v.amp[j] = v.amp[src] * 0.5;
                    v.coef[j] = v.coef[src];
                    v.ph[j] = 0.5 + 0.5 * seedn.next();
                    sum += v.amp[j];
                }
                for i in 0..NP {
                    v.amp[i] /= sum.max(1e-3);
                }
                let hammer = p[P_HAMMER] / 100.0;
                v.nf.set((800.0 + 3000.0 * vel * (0.5 + tone)).min(nyq), 0.2, sr);
                v.nenv = 1.0;
                v.ncoef = decay_coef(0.03, sr);
                v.nlvl = hammer * 0.9 * vel;
                (
                    0.55 * vel_gain,
                    ((note - 64.0) / 64.0 * 0.35).clamp(-0.45, 0.45),
                )
            }
        };
        let (pl, pr) = pan_gains(pan);
        v.gl = gain * pl;
        v.gr = gain * pr;
        v
    }

    #[cfg(test)]
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

impl Instrument for Keys {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let vel = velocity.clamp(0.0, 1.0);
        if vel <= 0.0 {
            self.note_off(pitch);
            return;
        }
        // Re-striking a held key releases the previous strike.
        let rel = decay_coef(self.params[P_RELEASE].min(0.3), self.sr);
        for v in self
            .voices
            .iter_mut()
            .filter(|v| v.active && v.gate && v.pitch == pitch)
        {
            v.gate = false;
            v.rel_coef = v.rel_coef.min(rel);
        }
        self.counter += 1;
        let seed = (self.counter as u32).wrapping_mul(0x9E37_79B9)
            ^ (pitch as u32).wrapping_mul(0x85EB_CA6B);
        let idx = self.alloc();
        self.voices[idx] = self.make_voice(pitch, vel, seed);
    }

    fn note_off(&mut self, pitch: u8) {
        let rel = decay_coef(self.params[P_RELEASE], self.sr);
        for v in self
            .voices
            .iter_mut()
            .filter(|v| v.active && v.gate && v.pitch == pitch)
        {
            v.gate = false;
            v.rel_coef = rel;
            // Organ: a softer click on key release.
            if v.mode == MODE_ORGAN && v.nlvl > 0.0 {
                v.nenv = 0.4;
            }
        }
    }

    fn all_notes_off(&mut self, hard: bool) {
        let rel = if hard {
            self.kill_coef
        } else {
            decay_coef(self.params[P_RELEASE], self.sr)
        };
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.gate = false;
            if hard {
                v.attacking = false;
                v.ncoef = rel;
                v.coef.iter_mut().for_each(|c| *c = c.min(rel));
                v.rel_coef = rel;
            } else if v.rel_coef != self.kill_coef {
                v.rel_coef = rel;
            }
        }
    }

    fn set_params(&mut self, values: &ParamValues) {
        self.params = *values;
        let tone = values[P_TONE] / 100.0;
        // Drawbar levels: ~3 dB per step; tone tilts the upper partials.
        let tilt = 0.6 + 0.6 * tone;
        let mut gains = [0.0f32; 9];
        let mut sum = 0.0;
        for i in 0..9 {
            let lvl = values[P_BAR + i].round().clamp(0.0, 8.0);
            let g = if lvl <= 0.0 {
                0.0
            } else {
                10f32.powf((lvl - 8.0) * 3.0 / 20.0)
            };
            gains[i] = g * tilt.powf(BAR_RATIOS[i].log2().max(0.0));
            sum += gains[i];
        }
        let norm = 0.3 / sum.max(1.0).powf(0.7);
        for i in 0..9 {
            self.bar_target[i] = gains[i] * norm;
        }
        self.trem_inc = values[P_TREM_RATE] / self.sr;
        self.trem_depth.set(values[P_TREM_DEPTH] / 100.0);
        self.trem_pan = values[P_TREM_PAN] >= 0.5;
        let rot = values[P_ROTARY].round() as i32;
        let organ = self.mode() == MODE_ORGAN;
        self.rot_wet.set(if organ && rot > 0 { 1.0 } else { 0.0 });
        self.rot_speed.set(if rot >= 2 { 6.4 } else { 0.8 });
        self.volume.set(db_to_gain(values[P_VOLUME]));
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let sr = self.sr;
        let rot_base = 0.003 * sr;
        let rot_depth = 0.001 * sr;
        let mut start = 0;
        while start < n {
            // Chunks end on absolute CHUNK boundaries (block-size independent).
            let to_boundary = CHUNK - (self.clock % CHUNK as u64) as usize;
            let len = (n - start).min(to_boundary);
            if self.clock.is_multiple_of(CHUNK as u64) {
                for i in 0..9 {
                    self.bar_gain[i] = self.bar_target[i]
                        + (self.bar_gain[i] - self.bar_target[i]) * self.bar_coef;
                }
            }
            let bl = &mut self.buf_l[..len];
            let br = &mut self.buf_r[..len];
            bl.fill(0.0);
            br.fill(0.0);
            let bars = self.bar_gain;
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.render(bl, br, &bars);
            }
            for i in 0..len {
                let mut yl = self.buf_l[i];
                let mut yr = self.buf_r[i];
                // Rotary: two Doppler taps on a mono feed, opposite phases.
                let w = self.rot_wet.next();
                self.rot_phase = wrap(self.rot_phase + self.rot_speed.next() / sr);
                let m = 0.5 * (yl + yr);
                self.rot_delay.push(m);
                if w > 1e-4 {
                    let s = fast_sin01(self.rot_phase);
                    let c = fast_sin01(self.rot_phase + 0.25);
                    let tl = self.rot_delay.read(rot_base + rot_depth * s) * (1.0 + 0.3 * c);
                    let tr = self.rot_delay.read(rot_base - rot_depth * s) * (1.0 - 0.3 * c);
                    yl += w * (0.45 * m + 0.6 * tl - yl);
                    yr += w * (0.45 * m + 0.6 * tr - yr);
                }
                // Tremolo / auto-pan.
                let d = self.trem_depth.next();
                self.trem_phase = wrap(self.trem_phase + self.trem_inc);
                if d > 1e-4 {
                    let s = fast_sin01(self.trem_phase);
                    if self.trem_pan {
                        yl *= 1.0 - d * (0.5 + 0.5 * s);
                        yr *= 1.0 - d * (0.5 - 0.5 * s);
                    } else {
                        let g = 1.0 - d * (0.5 + 0.5 * s);
                        yl *= g;
                        yr *= g;
                    }
                }
                let vol = self.volume.next();
                l[start + i] += yl * vol;
                r[start + i] += yr * vol;
            }
            self.clock += len as u64;
            start += len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::util::test_support::*;
    use super::*;

    fn keys_with(values: ParamValues, sr: f32) -> Keys {
        let mut k = Keys::new(sr);
        k.set_params(&values);
        k
    }

    #[test]
    fn spec_is_valid() {
        check_spec(&SPEC);
    }

    #[test]
    fn each_mode_note_peak_and_release() {
        for &sr in &[48000.0, 44100.0] {
            for mode in 0..MODES.len() {
                for &pitch in &[36u8, 60, 96] {
                    let mut k =
                        keys_with(with(&SPEC, SPEC.default_values(), "mode", mode as f32), sr);
                    k.note_on(pitch, 1.0);
                    let (l, r) = render(&mut k, sr, sr as usize, 512);
                    assert_finite(&l, "keys l");
                    assert_finite(&r, "keys r");
                    let pk = peak(&l).max(peak(&r));
                    assert!(
                        (0.05..=1.0).contains(&pk),
                        "mode {mode} pitch {pitch} sr {sr}: peak {pk}"
                    );
                    k.note_off(pitch);
                    let (l2, _) = render(&mut k, sr, (sr * 3.0) as usize, 512);
                    assert_eq!(k.active_voices(), 0, "mode {mode}: voice not freed");
                    assert!(
                        peak(&l2[l2.len() - 4096..]) < 1e-4,
                        "mode {mode}: not silent"
                    );
                }
            }
        }
    }

    #[test]
    fn presets_render_chords() {
        for preset in SPEC.presets {
            let mut k = keys_with(preset_values(&SPEC, preset), 48000.0);
            for &n in &[48u8, 60, 64, 67, 72] {
                k.note_on(n, 0.9);
            }
            let (l, r) = render(&mut k, 48000.0, 48000, 256);
            assert_finite(&l, preset.name);
            assert_finite(&r, preset.name);
            let pk = peak(&l).max(peak(&r));
            assert!(pk > 0.05 && pk < 1.5, "{}: peak {pk}", preset.name);
            for &n in &[48u8, 60, 64, 67, 72] {
                k.note_off(n);
            }
            render(&mut k, 48000.0, 48000 * 4, 512);
            assert_eq!(k.active_voices(), 0, "{}: voices left", preset.name);
        }
    }

    fn sequence(mode: f32, block: usize) -> Vec<f32> {
        let sr = 48000.0;
        let mut v = with(&SPEC, SPEC.default_values(), "mode", mode);
        v = with(&SPEC, v, "trem_depth", 50.0);
        let mut k = keys_with(v, sr);
        let mut out = Vec::new();
        for (i, &n) in [60u8, 64, 67, 72].iter().enumerate() {
            k.note_on(n, 0.6 + 0.1 * i as f32);
            let (l, r) = render(&mut k, sr, 2999, block);
            out.extend(l);
            out.extend(r);
        }
        k.note_off(64);
        let (l, r) = render(&mut k, sr, 4001, block);
        out.extend(l);
        out.extend(r);
        out
    }

    #[test]
    fn block_size_does_not_change_output() {
        for mode in 0..3 {
            let a = sequence(mode as f32, 1);
            let b = sequence(mode as f32, 7);
            let c = sequence(mode as f32, 512);
            for i in 0..a.len() {
                assert!(
                    (a[i] - b[i]).abs() < 1e-5 && (a[i] - c[i]).abs() < 1e-5,
                    "mode {mode}: mismatch at {i}: {} {} {}",
                    a[i],
                    b[i],
                    c[i]
                );
            }
        }
    }

    #[test]
    fn polyphony_is_bounded_and_safe() {
        let mut k = Keys::new(44100.0);
        for i in 0..40u8 {
            k.note_on(40 + i, 1.0);
            let (l, _) = render(&mut k, 44100.0, 64, 64);
            assert_finite(&l, "poly");
        }
        assert!(k.active_voices() <= NUM_SLOTS);
        render(&mut k, 44100.0, 4410, 512);
        assert!(k.voices.iter().filter(|v| v.active).count() <= MAX_POLY);
    }

    #[test]
    fn set_params_keeps_held_notes() {
        let mut k = keys_with(with(&SPEC, SPEC.default_values(), "mode", 1.0), 48000.0);
        k.note_on(60, 1.0);
        render(&mut k, 48000.0, 4800, 512);
        let mut v = with(&SPEC, SPEC.default_values(), "mode", 1.0);
        v = with(&SPEC, v, "bar5", 8.0);
        v = with(&SPEC, v, "rotary", 2.0);
        k.set_params(&v);
        let (l, _) = render(&mut k, 48000.0, 4800, 512);
        assert_eq!(k.active_voices(), 1);
        assert!(peak(&l) > 0.05);
        assert!(k.voices.iter().any(|v| v.active && v.env > 0.99));
    }

    #[test]
    fn all_notes_off_hard_and_soft() {
        for hard in [true, false] {
            let mut k = Keys::new(48000.0);
            for &n in &[60u8, 64, 67] {
                k.note_on(n, 1.0);
            }
            render(&mut k, 48000.0, 2000, 512);
            k.all_notes_off(hard);
            let frames = if hard { 480 } else { 48000 * 2 };
            render(&mut k, 48000.0, frames, 64);
            assert_eq!(k.active_voices(), 0, "hard={hard}");
        }
    }
}
