//! DSP building blocks: instrument / effect traits, parameter specs and the
//! device registry. Every built-in device declares its parameters with a
//! static `DeviceSpec`; the UI renders knobs generically from that spec, so
//! adding a new device only requires a spec + an implementation + a line in
//! `registry.rs`.

pub mod bassline;
pub mod drums;
pub mod effects;
pub mod grand;
pub mod keys;
pub mod poly;
pub mod registry;
pub mod rhythm;
pub mod sample_data;
pub mod sampler;
pub mod synth;
pub mod util;

use serde::Serialize;

/// Maximum number of parameters per device. Parameter values travel to the
/// audio thread as a fixed-size array, so no allocation happens there.
pub const MAX_PARAMS: usize = 48;
pub type ParamValues = [f32; MAX_PARAMS];

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ParamKind {
    /// Linear continuous value.
    Linear,
    /// Logarithmic continuous value (frequencies, times).
    Log,
    /// Integer value.
    Int,
    /// On / off.
    Toggle,
    /// Index into `choices`.
    Choice,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    pub kind: ParamKind,
    pub choices: &'static [&'static str],
    /// Optional UI grouping ("Osc 1", "Filter"...).
    pub group: &'static str,
}

impl ParamSpec {
    pub const fn lin(
        id: &'static str,
        name: &'static str,
        min: f32,
        max: f32,
        default: f32,
        unit: &'static str,
        group: &'static str,
    ) -> Self {
        Self { id, name, min, max, default, unit, kind: ParamKind::Linear, choices: &[], group }
    }
    pub const fn log(
        id: &'static str,
        name: &'static str,
        min: f32,
        max: f32,
        default: f32,
        unit: &'static str,
        group: &'static str,
    ) -> Self {
        Self { id, name, min, max, default, unit, kind: ParamKind::Log, choices: &[], group }
    }
    pub const fn int(
        id: &'static str,
        name: &'static str,
        min: f32,
        max: f32,
        default: f32,
        group: &'static str,
    ) -> Self {
        Self { id, name, min, max, default, unit: "", kind: ParamKind::Int, choices: &[], group }
    }
    pub const fn toggle(id: &'static str, name: &'static str, default: bool, group: &'static str) -> Self {
        Self {
            id,
            name,
            min: 0.0,
            max: 1.0,
            default: if default { 1.0 } else { 0.0 },
            unit: "",
            kind: ParamKind::Toggle,
            choices: &[],
            group,
        }
    }
    pub const fn choice(
        id: &'static str,
        name: &'static str,
        choices: &'static [&'static str],
        default: usize,
        group: &'static str,
    ) -> Self {
        Self {
            id,
            name,
            min: 0.0,
            max: (choices.len() - 1) as f32,
            default: default as f32,
            unit: "",
            kind: ParamKind::Choice,
            choices,
            group,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: &'static str,
    /// Values overriding the defaults; missing params keep their default.
    pub values: &'static [(&'static str, f32)],
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Instrument,
    Effect,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSpec {
    pub kind: DeviceKind,
    pub type_id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub params: &'static [ParamSpec],
    pub presets: &'static [Preset],
    /// For instruments: note names to show in the piano roll instead of
    /// pitches (drum kits). Empty = regular keyboard.
    pub note_names: &'static [(u8, &'static str)],
}

impl DeviceSpec {
    pub fn default_values(&self) -> ParamValues {
        let mut v = [0.0; MAX_PARAMS];
        for (i, p) in self.params.iter().enumerate().take(MAX_PARAMS) {
            v[i] = p.default;
        }
        v
    }

    /// Builds the value array from a name → value map, clamping to range.
    pub fn values_from_map(&self, map: &std::collections::HashMap<String, f32>) -> ParamValues {
        let mut v = self.default_values();
        for (i, p) in self.params.iter().enumerate().take(MAX_PARAMS) {
            if let Some(x) = map.get(p.id)
                && x.is_finite() {
                    v[i] = x.clamp(p.min, p.max);
                }
        }
        v
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }
}

/// Information available to devices while processing a block.
#[derive(Debug, Clone, Copy)]
pub struct ProcessCtx {
    pub sample_rate: f32,
    pub bpm: f32,
    pub playing: bool,
    /// Song position (in beats) at the start of the block.
    pub beat: f64,
}

pub trait Instrument: Send {
    fn note_on(&mut self, pitch: u8, velocity: f32);
    fn note_off(&mut self, pitch: u8);
    /// Releases every voice. `hard` = cut immediately (no release tail).
    fn all_notes_off(&mut self, hard: bool);
    fn set_params(&mut self, values: &ParamValues);
    /// Renders and ADDS the instrument output to `l`/`r`.
    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]);
}

pub trait Effect: Send {
    fn set_params(&mut self, values: &ParamValues);
    /// Processes the buffers in place.
    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]);
    /// Clears internal state (delay lines, envelopes).
    fn reset(&mut self);
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -96.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    if g <= 0.0000158 { -96.0 } else { 20.0 * g.log10() }
}

#[inline]
pub fn midi_to_hz(pitch: f32) -> f32 {
    440.0 * 2f32.powf((pitch - 69.0) / 12.0)
}
