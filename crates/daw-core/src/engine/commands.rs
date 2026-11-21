//! Messages between the control thread and the audio thread.

use std::sync::Arc;

use super::track::{AudioClipRt, Envelope, FxSlot, NoteRt, TrackRt};
use crate::assets::AudioBuffer;
use crate::dsp::{Instrument, ParamValues};

pub type Handle = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxTarget {
    Track(Handle),
    Master,
}

pub enum Command {
    Play,
    Stop,
    Seek(f64),
    /// Start recording: optional count-in then play + capture.
    Record,
    SetTempo(f64),
    SetBeatsPerBar(f64),
    SetLoop { enabled: bool, start: f64, end: f64, overdub: bool },
    /// Ends the take but keeps playing (Loop page: record → play).
    PunchOut,
    SetMetronome { enabled: bool, volume: f32, count_in_bars: u32 },
    AddTrack(Box<TrackRt>),
    RemoveTrack(Handle),
    SetTrackMix { handle: Handle, volume_db: f32, pan: f32, mute: bool, solo: bool, armed: bool },
    SetInstrument { handle: Handle, instrument: Option<Box<dyn Instrument>> },
    SetInstrumentParams { handle: Handle, values: Box<ParamValues> },
    SetEffects { target: FxTarget, slots: Box<Vec<FxSlot>> },
    SetEffectParams { target: FxTarget, index: usize, enabled: bool, values: Box<ParamValues> },
    SetNotes { handle: Handle, notes: Box<Vec<NoteRt>> },
    SetAudioClips { handle: Handle, clips: Box<Vec<AudioClipRt>> },
    SetAutomation { handle: Handle, volume: Envelope, pan: Envelope },
    SetMasterVolume(f32),
    SetLiveTarget(Option<Handle>),
    /// `handle` None = current live target. `record` = may be captured into a take.
    LiveNote { handle: Option<Handle>, pitch: u8, velocity: f32, on: bool, record: bool },
    /// Drops every track and master effect (used to resync after a queue overflow).
    Reset { empty_fx: Box<Vec<FxSlot>> },
    /// Panic button: hard stop of every voice.
    AllNotesOff,
    /// Audition an audio file (browser preview). None stops it.
    Preview(Option<Arc<AudioBuffer>>),
}

/// Objects handed back to the control thread so that deallocation never
/// happens on the audio thread.
#[allow(dead_code)]
pub enum Garbage {
    Track(Box<TrackRt>),
    Instrument(Box<dyn Instrument>),
    Effects(Box<Vec<FxSlot>>),
    Notes(Box<Vec<NoteRt>>),
    AudioClips(Box<Vec<AudioClipRt>>),
    Params(Box<ParamValues>),
    Envelope(Envelope),
    Buffer(Arc<AudioBuffer>),
}

/// A live note captured while recording (sent back to the UI).
#[derive(Debug, Clone, Copy)]
pub struct RecEvent {
    pub handle: Handle,
    pub pitch: u8,
    pub velocity: f32,
    pub on: bool,
    pub beat: f64,
}
