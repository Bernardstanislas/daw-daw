//! "Poly 80": a classic early-80s analog polysynth.
//!
//! Modelled after the architecture of the iconic DCO polys of that era:
//! one oscillator per voice with mixable saw, pulse (with LFO PWM) and a
//! square sub-octave, plus a detunable second saw for fatter patches,
//! a gentle high-pass, a resonant 24 dB/oct transistor ladder, a single
//! ADSR shared by filter and amp (or a plain gate on the amp), one global
//! triangle LFO with delayed onset, and the unmistakable stereo bucket-
//! brigade chorus (modes I, II and I+II).
//!
//! "Unison" stacks every voice on the last played note, detuned and spread
//! across the stereo field, for big mono leads and basses.

use super::util::{
    Adsr, DelayLine, Ladder, Noise, OnePole, Smoother, fast_pow2, flush, pan_gains, poly_blep,
};
use super::*;

const VOICES: usize = 8;
/// Control-rate period (filter cutoff, pitch, LFO).
const CONTROL: usize = 16;
const VOICE_GAIN: f32 = 0.32;

const HPF_STEPS: &[&str] = &["Off", "1", "2", "3"];
const HPF_HZ: [f32; 4] = [0.0, 110.0, 240.0, 500.0];
const AMP_MODES: &[&str] = &["Env", "Gate"];
const CHORUS: &[&str] = &["Off", "I", "II", "I+II"];
const MODES: &[&str] = &["Poly", "Unison"];

const P_SAW: usize = 0;
const P_PULSE: usize = 1;
const P_PW: usize = 2;
const P_PWM: usize = 3;
const P_SUB: usize = 4;
const P_NOISE: usize = 5;
const P_OSC2: usize = 6;
const P_OSC2_OCT: usize = 7;
const P_DETUNE: usize = 8;
const P_HPF: usize = 9;
const P_CUTOFF: usize = 10;
const P_RES: usize = 11;
const P_ENV_AMT: usize = 12;
const P_LFO_FLT: usize = 13;
const P_KEY_TRK: usize = 14;
const P_VEL_FLT: usize = 15;
const P_ATTACK: usize = 16;
const P_DECAY: usize = 17;
const P_SUSTAIN: usize = 18;
const P_RELEASE: usize = 19;
const P_AMP_MODE: usize = 20;
const P_LFO_RATE: usize = 21;
const P_LFO_DELAY: usize = 22;
const P_LFO_PITCH: usize = 23;
const P_CHORUS: usize = 24;
const P_MODE: usize = 25;
const P_UNI_DETUNE: usize = 26;
const P_GLIDE: usize = 27;
const P_VEL_AMP: usize = 28;
const P_VOLUME: usize = 29;

pub static SPEC: DeviceSpec = DeviceSpec {
    kind: DeviceKind::Instrument,
    type_id: "poly",
    name: "Poly 80",
    category: "Synth",
    description: "Classic analog polysynth: saw/pulse/sub DCO, ladder filter, one ADSR and the legendary stereo chorus.",
    params: &[
        ParamSpec::lin("saw", "Saw", 0.0, 100.0, 100.0, "%", "Oscillator"),
        ParamSpec::lin("pulse", "Pulse", 0.0, 100.0, 0.0, "%", "Oscillator"),
        ParamSpec::lin("pw", "Width", 5.0, 50.0, 50.0, "%", "Oscillator"),
        ParamSpec::lin("pwm", "PWM", 0.0, 100.0, 0.0, "%", "Oscillator"),
        ParamSpec::lin("sub", "Sub", 0.0, 100.0, 0.0, "%", "Oscillator"),
        ParamSpec::lin("noise", "Noise", 0.0, 100.0, 0.0, "%", "Oscillator"),
        ParamSpec::lin("osc2", "Saw 2", 0.0, 100.0, 0.0, "%", "Oscillator"),
        ParamSpec::int("osc2_oct", "Saw 2 Oct", -1.0, 1.0, 0.0, "Oscillator"),
        ParamSpec::lin("detune", "Detune", 0.0, 50.0, 8.0, "ct", "Oscillator"),
        ParamSpec::choice("hpf", "HPF", HPF_STEPS, 0, "Filter"),
        ParamSpec::log("cutoff", "Cutoff", 30.0, 18000.0, 2400.0, "Hz", "Filter"),
        ParamSpec::lin("res", "Resonance", 0.0, 100.0, 15.0, "%", "Filter"),
        ParamSpec::lin("env_amt", "Env", -100.0, 100.0, 35.0, "%", "Filter"),
        ParamSpec::lin("lfo_flt", "LFO", 0.0, 100.0, 0.0, "%", "Filter"),
        ParamSpec::lin("key_trk", "Key", 0.0, 100.0, 50.0, "%", "Filter"),
        ParamSpec::lin("vel_flt", "Velocity", 0.0, 100.0, 25.0, "%", "Filter"),
        ParamSpec::log("attack", "Attack", 0.001, 5.0, 0.003, "s", "Envelope"),
        ParamSpec::log("decay", "Decay", 0.005, 10.0, 0.6, "s", "Envelope"),
        ParamSpec::lin("sustain", "Sustain", 0.0, 100.0, 60.0, "%", "Envelope"),
        ParamSpec::log("release", "Release", 0.005, 10.0, 0.35, "s", "Envelope"),
        ParamSpec::choice("amp_mode", "Amp", AMP_MODES, 0, "Envelope"),
        ParamSpec::log("lfo_rate", "Rate", 0.1, 20.0, 5.0, "Hz", "LFO"),
        ParamSpec::lin("lfo_delay", "Delay", 0.0, 3.0, 0.0, "s", "LFO"),
        ParamSpec::lin("lfo_pitch", "Vibrato", 0.0, 100.0, 0.0, "%", "LFO"),
        ParamSpec::choice("chorus", "Chorus", CHORUS, 1, "Chorus"),
        ParamSpec::choice("mode", "Mode", MODES, 0, "Voice"),
        ParamSpec::lin("uni_detune", "Spread", 0.0, 100.0, 35.0, "%", "Voice"),
        ParamSpec::log("glide", "Glide", 0.001, 2.0, 0.001, "s", "Voice"),
        ParamSpec::lin("vel_amp", "Velocity", 0.0, 100.0, 40.0, "%", "Voice"),
        ParamSpec::lin("volume", "Volume", -24.0, 6.0, 0.0, "dB", "Voice"),
    ],
    presets: &[
        Preset {
            name: "Strings 80",
            values: &[
                ("saw", 100.0),
                ("pwm", 0.0),
                ("hpf", 1.0),
                ("cutoff", 4200.0),
                ("res", 5.0),
                ("env_amt", 10.0),
                ("attack", 0.45),
                ("decay", 1.5),
                ("sustain", 85.0),
                ("release", 1.2),
                ("chorus", 2.0),
                ("lfo_pitch", 6.0),
                ("lfo_rate", 5.5),
                ("lfo_delay", 0.6),
                ("vel_amp", 20.0),
            ],
        },
        Preset {
            name: "Brass Stab",
            values: &[
                ("saw", 100.0),
                ("osc2", 80.0),
                ("detune", 9.0),
                ("cutoff", 600.0),
                ("res", 10.0),
                ("env_amt", 62.0),
                ("attack", 0.04),
                ("decay", 0.45),
                ("sustain", 55.0),
                ("release", 0.25),
                ("chorus", 1.0),
                ("vel_flt", 45.0),
            ],
        },
        Preset {
            name: "Hoover Pad",
            values: &[
                ("saw", 70.0),
                ("pulse", 80.0),
                ("pw", 30.0),
                ("pwm", 70.0),
                ("lfo_rate", 0.6),
                ("cutoff", 1800.0),
                ("res", 20.0),
                ("env_amt", 15.0),
                ("attack", 0.9),
                ("decay", 2.0),
                ("sustain", 80.0),
                ("release", 2.5),
                ("chorus", 3.0),
            ],
        },
        Preset {
            name: "Square Organ",
            values: &[
                ("saw", 0.0),
                ("pulse", 100.0),
                ("pw", 50.0),
                ("sub", 70.0),
                ("cutoff", 5000.0),
                ("res", 0.0),
                ("env_amt", 0.0),
                ("amp_mode", 1.0),
                ("release", 0.05),
                ("chorus", 1.0),
                ("vel_amp", 0.0),
                ("volume", -6.0),
            ],
        },
        Preset {
            name: "Resonant Pluck",
            values: &[
                ("saw", 100.0),
                ("pulse", 40.0),
                ("pw", 20.0),
                ("cutoff", 220.0),
                ("res", 45.0),
                ("env_amt", 70.0),
                ("attack", 0.001),
                ("decay", 0.28),
                ("sustain", 0.0),
                ("release", 0.3),
                ("chorus", 1.0),
                ("key_trk", 80.0),
            ],
        },
        Preset {
            name: "Analog Bass",
            values: &[
                ("saw", 100.0),
                ("sub", 85.0),
                ("cutoff", 180.0),
                ("res", 30.0),
                ("env_amt", 55.0),
                ("decay", 0.35),
                ("sustain", 20.0),
                ("release", 0.08),
                ("chorus", 0.0),
                ("key_trk", 30.0),
            ],
        },
        Preset {
            name: "Unison Lead",
            values: &[
                ("saw", 100.0),
                ("osc2", 60.0),
                ("detune", 6.0),
                ("mode", 1.0),
                ("uni_detune", 45.0),
                ("cutoff", 2600.0),
                ("res", 25.0),
                ("env_amt", 30.0),
                ("decay", 0.8),
                ("sustain", 70.0),
                ("glide", 0.06),
                ("lfo_pitch", 10.0),
                ("lfo_delay", 0.4),
                ("chorus", 0.0),
                ("volume", -4.0),
            ],
        },
        Preset {
            name: "Dream Keys",
            values: &[
                ("saw", 40.0),
                ("pulse", 70.0),
                ("pw", 35.0),
                ("pwm", 25.0),
                ("lfo_rate", 3.2),
                ("cutoff", 900.0),
                ("res", 12.0),
                ("env_amt", 45.0),
                ("attack", 0.002),
                ("decay", 1.8),
                ("sustain", 25.0),
                ("release", 0.9),
                ("chorus", 2.0),
            ],
        },
    ],
    note_names: &[],
};

/// Parameter values decoded once per `set_params`.
#[derive(Debug, Clone, Copy)]
struct Settings {
    saw: f32,
    pulse: f32,
    pw: f32,
    pwm: f32,
    sub: f32,
    noise: f32,
    osc2: f32,
    osc2_ratio: f32,
    detune_ratio: f32,
    cutoff_oct: f32,
    res: f32,
    env_oct: f32,
    lfo_flt_oct: f32,
    key_trk: f32,
    vel_flt_oct: f32,
    gate_amp: bool,
    lfo_inc: f32,
    lfo_delay: f32,
    vib_semis: f32,
    unison: bool,
    uni_cents: f32,
    glide_coef: f32,
    vel_amp: f32,
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    held: bool,
    note: u8,
    age: u64,
    vel: f32,
    /// Current / target pitch (MIDI semitones, fractional while gliding).
    pitch: f32,
    target: f32,
    /// Fixed per-voice offset (unison spread, analog drift) in semitones.
    offset: f32,
    gl: f32,
    gr: f32,
    ph: f32,
    sub_ph: f32,
    ph2: f32,
    inc: f32,
    inc2: f32,
    env: Adsr,
    gate: Adsr,
    lfo_env: f32,
    lfo_env_inc: f32,
    filter: Ladder,
    hpf: OnePole,
    noise: Noise,
}

impl Voice {
    fn new(sr: f32, seed: u32) -> Self {
        Self {
            active: false,
            held: false,
            note: 0,
            age: 0,
            vel: 1.0,
            pitch: 60.0,
            target: 60.0,
            offset: 0.0,
            gl: 1.0,
            gr: 1.0,
            ph: 0.0,
            sub_ph: 0.0,
            ph2: 0.0,
            inc: 0.0,
            inc2: 0.0,
            env: Adsr::new(sr),
            gate: Adsr::new(sr),
            lfo_env: 0.0,
            lfo_env_inc: 1.0,
            filter: Ladder::default(),
            hpf: OnePole::default(),
            noise: Noise::new(seed),
        }
    }
}

pub struct Poly80 {
    sr: f32,
    params: ParamValues,
    s: Settings,
    voices: [Voice; VOICES],
    counter: u64,
    clock: usize,
    lfo_ph: f32,
    lfo: f32,
    /// Held notes in unison mode (last = sounding).
    stack: [u8; 16],
    stack_len: usize,
    hpf_hz: f32,
    // Chorus.
    chorus_mode: usize,
    ch_delay: DelayLine,
    ch_ph: f32,
    ch_wet: Smoother,
    ch_tone: OnePole,
    volume: Smoother,
}

impl Poly80 {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut voices = [Voice::new(sr, 1); VOICES];
        for (i, v) in voices.iter_mut().enumerate() {
            v.noise = Noise::new(0x9E37_79B9 ^ (i as u32 * 7919 + 13));
        }
        let mut ch_tone = OnePole::default();
        ch_tone.set(7000.0, sr);
        let mut p = Self {
            sr,
            params: SPEC.default_values(),
            s: decode(&SPEC.default_values(), sr),
            voices,
            counter: 0,
            clock: 0,
            lfo_ph: 0.0,
            lfo: 0.0,
            stack: [0; 16],
            stack_len: 0,
            hpf_hz: 0.0,
            chorus_mode: 1,
            ch_delay: DelayLine::new((0.02 * sr) as usize + 4),
            ch_ph: 0.0,
            ch_wet: Smoother::new(1.0, 40.0, sr),
            ch_tone,
            volume: Smoother::new(1.0, 30.0, sr),
        };
        let v = p.params;
        p.set_params(&v);
        p.ch_wet.snap(p.ch_wet.target);
        p.volume.snap(p.volume.target);
        p
    }

    fn apply_env_settings(&mut self) {
        let p = &self.params;
        let (a, d, s, r) = (p[P_ATTACK], p[P_DECAY], p[P_SUSTAIN] / 100.0, p[P_RELEASE]);
        let sr = self.sr;
        for v in self.voices.iter_mut() {
            v.env.set(a, d, s, r, sr);
            v.gate.set(0.002, 0.01, 1.0, r.min(0.03), sr);
        }
    }

    fn allocate(&mut self, pitch: u8) -> usize {
        // Same note again: reuse its voice.
        if let Some(i) = self.voices.iter().position(|v| v.active && v.note == pitch) {
            return i;
        }
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        // Steal: oldest released voice first, then the oldest held one.
        let mut best = 0;
        for i in 1..VOICES {
            let (a, b) = (&self.voices[i], &self.voices[best]);
            if (a.held, a.age) < (b.held, b.age) {
                best = i;
            }
        }
        best
    }

    fn start(&mut self, i: usize, pitch: u8, vel: f32, legato: bool) {
        self.counter += 1;
        let s = self.s;
        let sr = self.sr;
        let v = &mut self.voices[i];
        let was_active = v.active;
        v.active = true;
        v.held = true;
        v.note = pitch;
        v.age = self.counter;
        v.target = pitch as f32;
        if !was_active || s.glide_coef <= 0.0 {
            v.pitch = v.target;
        }
        if legato {
            return;
        }
        v.vel = vel;
        v.lfo_env = if s.lfo_delay <= 0.0 { 1.0 } else { 0.0 };
        v.lfo_env_inc = if s.lfo_delay <= 0.0 { 1.0 } else { CONTROL as f32 / (s.lfo_delay * sr) };
        if was_active {
            v.env.gate_on();
            v.gate.gate_on();
        } else {
            v.env.trigger();
            v.gate.trigger();
            v.filter.reset();
            // DCO behaviour: oscillators restart on a fresh voice.
            v.ph = 0.0;
            v.sub_ph = 0.0;
            v.ph2 = 0.37;
        }
    }

    fn release(&mut self, i: usize) {
        let v = &mut self.voices[i];
        v.held = false;
        v.env.gate_off();
        v.gate.gate_off();
    }

    /// Unison: (re)assigns every voice to `pitch` with its spread offset.
    fn unison_play(&mut self, pitch: u8, vel: f32, legato: bool) {
        let spread = self.s.uni_cents / 100.0;
        for i in 0..VOICES {
            let x = i as f32 / (VOICES - 1) as f32 * 2.0 - 1.0; // -1..1
            self.voices[i].offset = x * spread;
            let (gl, gr) = pan_gains(x * 0.8);
            // Eight stacked voices: keep the sum near a single poly note.
            self.voices[i].gl = gl * 0.4;
            self.voices[i].gr = gr * 0.4;
            self.start(i, pitch, vel, legato);
        }
    }

    fn control(&mut self) {
        let s = self.s;
        let sr = self.sr;
        // Global triangle LFO.
        self.lfo_ph += s.lfo_inc * CONTROL as f32;
        if self.lfo_ph >= 1.0 {
            self.lfo_ph -= 1.0;
        }
        self.lfo = 1.0 - 4.0 * (self.lfo_ph - 0.5).abs();
        let lfo = self.lfo;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            if s.glide_coef > 0.0 {
                v.pitch = v.target + (v.pitch - v.target) * s.glide_coef;
            } else {
                v.pitch = v.target;
            }
            v.lfo_env = (v.lfo_env + v.lfo_env_inc).min(1.0);
            let vlfo = lfo * v.lfo_env;
            let note = v.pitch + v.offset + vlfo * s.vib_semis;
            let f = 440.0 * fast_pow2((note - 69.0) / 12.0);
            v.inc = (f / sr).min(0.45);
            v.inc2 = (f * s.osc2_ratio * s.detune_ratio / sr).min(0.45);
            let env = v.env.level;
            let oct = s.cutoff_oct
                + env * s.env_oct
                + vlfo * s.lfo_flt_oct
                + s.key_trk * (v.pitch - 60.0) / 12.0
                + v.vel * s.vel_flt_oct;
            let fc = 1000.0 * fast_pow2(oct.clamp(-6.0, 4.5));
            v.filter.set(fc, s.res, sr);
        }
    }

    fn render_voices(&mut self, l: &mut [f32], r: &mut [f32]) {
        let s = self.s;
        let hpf_on = self.hpf_hz > 0.0;
        let pwm0 = self.lfo;
        let lfo_step = {
            // Linear extrapolation of the LFO inside the control period keeps
            // PWM smooth without a per-sample LFO for every voice.
            let slope = if self.lfo_ph < 0.5 { 4.0 } else { -4.0 };
            slope * s.lfo_inc
        };
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let amp_vel = 1.0 - s.vel_amp + s.vel_amp * v.vel;
            let mut pwm_lfo = pwm0;
            for (ol, or) in l.iter_mut().zip(r.iter_mut()) {
                let dt = v.inc;
                // Main DCO (saw + pulse share the phase), sub one octave down.
                v.ph += dt;
                if v.ph >= 1.0 {
                    v.ph -= 1.0;
                }
                v.sub_ph += dt * 0.5;
                if v.sub_ph >= 1.0 {
                    v.sub_ph -= 1.0;
                }
                let t = v.ph;
                let mut x = 0.0;
                if s.saw > 0.0 {
                    x += (2.0 * t - 1.0 - poly_blep(t, dt)) * s.saw;
                }
                if s.pulse > 0.0 {
                    let pw = (s.pw + s.pwm * (pwm_lfo * 0.5 + 0.5) * (0.95 - s.pw)).clamp(0.05, 0.95);
                    let mut t2 = t - pw;
                    if t2 < 0.0 {
                        t2 += 1.0;
                    }
                    let sq = (if t < pw { 1.0 } else { -1.0 }) + poly_blep(t, dt) - poly_blep(t2, dt);
                    x += sq * s.pulse;
                }
                if s.sub > 0.0 {
                    let sd = dt * 0.5;
                    let st = v.sub_ph;
                    let mut st2 = st + 0.5;
                    if st2 >= 1.0 {
                        st2 -= 1.0;
                    }
                    let sq = (if st < 0.5 { 1.0 } else { -1.0 }) + poly_blep(st, sd) - poly_blep(st2, sd);
                    x += sq * s.sub;
                }
                if s.osc2 > 0.0 {
                    let d2 = v.inc2;
                    v.ph2 += d2;
                    if v.ph2 >= 1.0 {
                        v.ph2 -= 1.0;
                    }
                    x += (2.0 * v.ph2 - 1.0 - poly_blep(v.ph2, d2)) * s.osc2;
                }
                if s.noise > 0.0 {
                    x += v.noise.next() * s.noise;
                }
                pwm_lfo = (pwm_lfo + lfo_step).clamp(-1.0, 1.0);
                if hpf_on {
                    x = v.hpf.hp(x);
                }
                let y = v.filter.process(x * 0.45, 4, 0.0);
                let fenv = v.env.next();
                let g = if s.gate_amp { v.gate.next() } else { fenv };
                let out = y * g * amp_vel * VOICE_GAIN;
                *ol += out * v.gl;
                *or += out * v.gr;
            }
            let amp_done = if s.gate_amp { !v.gate.is_active() } else { !v.env.is_active() };
            if amp_done {
                v.active = false;
                v.held = false;
            }
        }
    }

    fn chorus(&mut self, l: &mut [f32], r: &mut [f32]) {
        // (rate Hz, centre ms, depth ms) per mode.
        let (rate, centre, depth) = match self.chorus_mode {
            1 => (0.5, 3.6, 1.7),
            2 => (0.83, 3.6, 2.2),
            3 => (8.0, 3.2, 0.35),
            _ => (0.5, 3.6, 0.0),
        };
        let inc = rate / self.sr;
        let ms = self.sr * 0.001;
        for (ol, or) in l.iter_mut().zip(r.iter_mut()) {
            let wet = self.ch_wet.next();
            let mono = (*ol + *or) * 0.5;
            self.ch_delay.push(self.ch_tone.lp(mono));
            if wet < 1.0e-4 {
                continue;
            }
            self.ch_ph += inc;
            if self.ch_ph >= 1.0 {
                self.ch_ph -= 1.0;
            }
            let tri = 1.0 - 4.0 * (self.ch_ph - 0.5).abs();
            let da = (centre + depth * tri) * ms;
            let db = (centre - depth * tri) * ms;
            let wa = self.ch_delay.read_linear(da);
            let wb = self.ch_delay.read_linear(db);
            *ol = flush(*ol + wa * wet * 0.85);
            *or = flush(*or + wb * wet * 0.85);
        }
    }
}

fn decode(p: &ParamValues, sr: f32) -> Settings {
    let pct = |i: usize| p[i] / 100.0;
    let glide = p[P_GLIDE];
    Settings {
        saw: pct(P_SAW),
        pulse: pct(P_PULSE),
        pw: pct(P_PW),
        pwm: pct(P_PWM),
        sub: pct(P_SUB),
        noise: pct(P_NOISE) * 0.8,
        osc2: pct(P_OSC2),
        osc2_ratio: 2f32.powf(p[P_OSC2_OCT].round()),
        detune_ratio: 2f32.powf(p[P_DETUNE] / 1200.0),
        cutoff_oct: (p[P_CUTOFF].max(1.0) / 1000.0).log2(),
        res: pct(P_RES) * 0.97,
        env_oct: pct(P_ENV_AMT) * 7.0,
        lfo_flt_oct: pct(P_LFO_FLT) * 2.5,
        key_trk: pct(P_KEY_TRK),
        vel_flt_oct: pct(P_VEL_FLT) * 2.5,
        gate_amp: p[P_AMP_MODE].round() >= 1.0,
        lfo_inc: p[P_LFO_RATE] / sr,
        lfo_delay: p[P_LFO_DELAY],
        vib_semis: pct(P_LFO_PITCH) * pct(P_LFO_PITCH) * 2.0,
        unison: p[P_MODE].round() >= 1.0,
        uni_cents: pct(P_UNI_DETUNE) * 30.0,
        glide_coef: if glide <= 0.0015 { 0.0 } else { (-(CONTROL as f32) * 4.6 / (glide * sr)).exp() },
        vel_amp: pct(P_VEL_AMP),
    }
}

impl Instrument for Poly80 {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let vel = velocity.clamp(0.0, 1.0);
        if self.s.unison {
            let legato = self.stack_len > 0 && self.voices[0].held;
            if let Some(pos) = self.stack[..self.stack_len].iter().position(|&n| n == pitch) {
                self.stack.copy_within(pos + 1..self.stack_len, pos);
                self.stack_len -= 1;
            }
            if self.stack_len == self.stack.len() {
                self.stack.copy_within(1.., 0);
                self.stack_len -= 1;
            }
            self.stack[self.stack_len] = pitch;
            self.stack_len += 1;
            self.unison_play(pitch, vel, legato);
            return;
        }
        let i = self.allocate(pitch);
        // Poly: a little analog drift per voice, centred pan with slight spread.
        let drift = [0.0, 0.03, -0.025, 0.018, -0.035, 0.022, -0.012, 0.03][i];
        let (gl, gr) = pan_gains(((i as f32 / (VOICES - 1) as f32) * 2.0 - 1.0) * 0.15);
        let v = &mut self.voices[i];
        v.offset = drift;
        v.gl = gl;
        v.gr = gr;
        self.start(i, pitch, vel, false);
    }

    fn note_off(&mut self, pitch: u8) {
        if self.s.unison {
            if let Some(pos) = self.stack[..self.stack_len].iter().position(|&n| n == pitch) {
                self.stack.copy_within(pos + 1..self.stack_len, pos);
                self.stack_len -= 1;
            }
            if self.voices[0].note != pitch {
                return;
            }
            if self.stack_len > 0 {
                let prev = self.stack[self.stack_len - 1];
                let vel = self.voices[0].vel;
                self.unison_play(prev, vel, true);
            } else {
                for i in 0..VOICES {
                    self.release(i);
                }
            }
            return;
        }
        for i in 0..VOICES {
            if self.voices[i].active && self.voices[i].held && self.voices[i].note == pitch {
                self.release(i);
            }
        }
    }

    fn all_notes_off(&mut self, hard: bool) {
        self.stack_len = 0;
        for v in self.voices.iter_mut() {
            v.held = false;
            if hard {
                v.env.kill();
                v.gate.kill();
            } else {
                v.env.gate_off();
                v.gate.gate_off();
            }
        }
    }

    fn set_params(&mut self, values: &ParamValues) {
        let was_unison = self.s.unison;
        self.params = *values;
        self.s = decode(values, self.sr);
        if was_unison != self.s.unison {
            self.all_notes_off(false);
        }
        self.apply_env_settings();
        let hz = HPF_HZ[(values[P_HPF].round() as usize).min(3)];
        self.hpf_hz = hz;
        if hz > 0.0 {
            for v in self.voices.iter_mut() {
                v.hpf.set(hz, self.sr);
            }
        }
        self.chorus_mode = (values[P_CHORUS].round() as usize).min(3);
        self.ch_wet.set(if self.chorus_mode == 0 { 0.0 } else { 1.0 });
        self.volume.set(db_to_gain(values[P_VOLUME]));
    }

    fn process(&mut self, _ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        let n = l.len().min(r.len());
        let mut tl = [0.0f32; CONTROL];
        let mut tr = [0.0f32; CONTROL];
        let mut i = 0;
        while i < n {
            if self.clock == 0 {
                self.control();
            }
            let len = (CONTROL - self.clock).min(n - i);
            let (bl, br) = (&mut tl[..len], &mut tr[..len]);
            bl.fill(0.0);
            br.fill(0.0);
            self.render_voices(bl, br);
            self.chorus(bl, br);
            for k in 0..len {
                let g = self.volume.next();
                l[i + k] += bl[k] * g;
                r[i + k] += br[k] * g;
            }
            self.clock = (self.clock + len) % CONTROL;
            i += len;
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
    }

    #[test]
    fn every_preset_plays_and_releases() {
        for &sr in &[44100.0, 48000.0] {
            for preset in SPEC.presets {
                let mut s = Poly80::new(sr);
                s.set_params(&preset_values(&SPEC, preset));
                s.note_on(48, 0.9);
                s.note_on(55, 0.7);
                s.note_on(64, 0.8);
                let (l, r) = render(&mut s, sr, (sr * 0.5) as usize, 128);
                assert_finite(&l, preset.name);
                assert_finite(&r, preset.name);
                let p = peak(&l).max(peak(&r));
                assert!(p > 0.02 && p < 2.0, "{}: peak {p}", preset.name);
                for n in [48, 55, 64] {
                    s.note_off(n);
                }
                let _ = render(&mut s, sr, (sr * 11.0) as usize, 512);
                let (l, _) = render(&mut s, sr, 512, 512);
                assert!(peak(&l) < 1.0e-3, "{}: tail does not die", preset.name);
            }
        }
    }

    #[test]
    fn block_size_does_not_change_output() {
        let mk = || {
            let mut s = Poly80::new(48000.0);
            s.note_on(60, 1.0);
            s
        };
        let (a, _) = render(&mut mk(), 48000.0, 4000, 64);
        let (b, _) = render(&mut mk(), 48000.0, 4000, 37);
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-5);
        }
    }

    #[test]
    fn unison_follows_last_note() {
        let mut s = Poly80::new(48000.0);
        s.set_params(&preset_values(&SPEC, &SPEC.presets[6]));
        s.note_on(60, 1.0);
        s.note_on(67, 1.0);
        assert!(s.voices.iter().all(|v| v.note == 67 && v.held));
        s.note_off(67);
        assert!(s.voices.iter().all(|v| v.note == 60 && v.held));
        s.note_off(60);
        assert!(s.voices.iter().all(|v| !v.held));
    }
}
