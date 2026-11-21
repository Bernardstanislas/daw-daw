//! The subset of the project document the audio engine cares about.
//! The frontend owns the full document (UI fields, colors, sections...);
//! unknown fields are ignored here. Positions are in beats (quarter notes).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Project {
    pub bpm: f64,
    pub time_sig_num: u32,
    pub time_sig_den: u32,
    #[serde(rename = "loop")]
    pub loop_region: LoopRegion,
    pub metronome: Metronome,
    pub tracks: Vec<Track>,
    pub master: Master,
    pub assets: Vec<AssetRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LoopRegion {
    pub enabled: bool,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Metronome {
    pub enabled: bool,
    pub volume: f32,
    pub count_in_bars: u32,
}

impl Default for Metronome {
    fn default() -> Self {
        Self { enabled: false, volume: 0.7, count_in_bars: 1 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Master {
    /// dB
    pub volume: f32,
    pub effects: Vec<DeviceState>,
}

impl Default for Master {
    fn default() -> Self {
        Self { volume: 0.0, effects: vec![] }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    #[default]
    Instrument,
    Audio,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    /// dB
    pub volume: f32,
    /// -1..1
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    /// MIDI channel (1..16) whose notes always play this track, whatever is selected.
    pub midi_channel: Option<u8>,
    pub instrument: Option<DeviceState>,
    pub effects: Vec<DeviceState>,
    pub clips: Vec<Clip>,
    pub automation: Automation,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Automation {
    /// dB
    pub volume: Vec<AutoPoint>,
    /// -1..1
    pub pan: Vec<AutoPoint>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoPoint {
    pub beat: f64,
    pub value: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceState {
    pub id: String,
    #[serde(rename = "type")]
    pub type_id: String,
    pub enabled: bool,
    pub params: HashMap<String, f32>,
    /// Opaque saved state of an external plugin (base64), restored when it is created.
    pub state: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ClipKind {
    #[default]
    Midi,
    Audio,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Clip {
    pub id: String,
    pub kind: ClipKind,
    /// Timeline start (beats).
    pub start: f64,
    /// Timeline length (beats).
    pub length: f64,
    pub muted: bool,
    /// MIDI: content offset in beats (content position at clip start).
    /// Audio: offset into the source file, in seconds.
    pub offset: f64,
    /// MIDI: if > 0, content repeats every `loop_length` beats.
    pub loop_length: f64,
    pub notes: Vec<Note>,
    pub asset_id: Option<String>,
    /// dB
    pub gain: f32,
    /// beats
    pub fade_in: f64,
    /// beats
    pub fade_out: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Note {
    pub pitch: u8,
    /// Relative to the clip content start (beats).
    pub start: f64,
    pub length: f64,
    /// 1..127
    pub velocity: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AssetRef {
    pub id: String,
    pub path: String,
    pub name: String,
}

impl Project {
    pub fn beats_per_bar(&self) -> f64 {
        let num = self.time_sig_num.max(1) as f64;
        let den = self.time_sig_den.max(1) as f64;
        num * 4.0 / den
    }

    /// End of the last clip, in beats.
    pub fn content_end(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.length)
            .fold(0.0, f64::max)
    }
}
