//! Central list of built-in devices. Add a device here to make it available
//! to the engine and the UI (the UI reads `all_specs()` through `get_catalog`).

use std::sync::OnceLock;

use super::{DeviceSpec, Effect, Instrument};
use crate::model::DeviceState;

/// Devices provided by the host application (e.g. Audio Unit / VST plugins
/// on desktop). Built-in devices always take precedence.
pub trait ExternalDevices: Send + Sync {
    /// True if this provider creates devices of that type id.
    fn handles(&self, type_id: &str) -> bool;
    /// `offline` = for an export render: must not replace the live instance.
    fn create_instrument(&self, device: &DeviceState, sample_rate: f32, offline: bool) -> Option<Box<dyn Instrument>>;
    fn create_effect(&self, device: &DeviceState, sample_rate: f32, offline: bool) -> Option<Box<dyn Effect>>;
}

static EXTERNAL: OnceLock<Box<dyn ExternalDevices>> = OnceLock::new();

/// Installs the external device provider (once, at startup).
pub fn set_external(provider: Box<dyn ExternalDevices>) {
    let _ = EXTERNAL.set(provider);
}

pub fn external() -> Option<&'static dyn ExternalDevices> {
    EXTERNAL.get().map(|b| b.as_ref())
}

/// Built-in or external device type.
pub fn is_known(type_id: &str) -> bool {
    spec(type_id).is_some() || external().is_some_and(|e| e.handles(type_id))
}

pub fn all_specs() -> Vec<&'static DeviceSpec> {
    let mut v: Vec<&'static DeviceSpec> = vec![
        &super::synth::SPEC,
        &super::poly::SPEC,
        &super::bassline::SPEC,
        &super::sampler::SPEC,
        &super::drums::SPEC,
        &super::sampler::KIT_SPEC,
        &super::keys::SPEC,
        &super::grand::SPEC,
        &super::rhythm::SPEC,
    ];
    v.extend(super::effects::SPECS.iter().copied());
    v
}

pub fn spec(type_id: &str) -> Option<&'static DeviceSpec> {
    all_specs().into_iter().find(|s| s.type_id == type_id)
}

pub fn create_instrument(type_id: &str, sample_rate: f32) -> Option<Box<dyn Instrument>> {
    match type_id {
        "pulse" => Some(Box::new(super::synth::PulseSynth::new(sample_rate))),
        "kit" => Some(Box::new(super::drums::DrumKit::new(sample_rate))),
        "keys" => Some(Box::new(super::keys::Keys::new(sample_rate))),
        "poly" => Some(Box::new(super::poly::Poly80::new(sample_rate))),
        "grand" => Some(Box::new(super::grand::Grand::new(sample_rate))),
        "bassline" => Some(Box::new(super::bassline::Bassline::new(sample_rate))),
        "rhythm" => Some(Box::new(super::rhythm::Rhythm::new(sample_rate))),
        "sampler" => Some(Box::new(super::sampler::Sampler::new(sample_rate))),
        "samplekit" => Some(Box::new(super::sampler::SampleKit::new(sample_rate))),
        _ => None,
    }
}

pub fn create_effect(type_id: &str, sample_rate: f32) -> Option<Box<dyn Effect>> {
    super::effects::create(type_id, sample_rate)
}
