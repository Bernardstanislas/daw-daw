// Project document. The frontend owns it; the Rust engine reads the audio
// relevant subset (see src-tauri/src/model.rs). Positions are in beats.

export type TrackKind = "instrument" | "audio";
export type ClipKind = "midi" | "audio";

export interface Note {
  id: string;
  pitch: number;
  /** Relative to clip content start (beats). */
  start: number;
  length: number;
  /** 1..127 */
  velocity: number;
}

export interface Clip {
  id: string;
  kind: ClipKind;
  name: string;
  start: number;
  length: number;
  muted: boolean;
  /** MIDI: content offset (beats). Audio: source offset (seconds). */
  offset: number;
  /** MIDI only: >0 → content repeats every loopLength beats. */
  loopLength: number;
  notes: Note[];
  assetId: string | null;
  /** dB */
  gain: number;
  fadeIn: number;
  fadeOut: number;
  color?: string;
}

export interface DeviceState {
  id: string;
  type: string;
  enabled: boolean;
  params: Record<string, number>;
  preset?: string;
  /** Display name for third-party plugins (no built-in spec). */
  label?: string;
  /** Saved state of a third-party plugin (base64), restored on load. */
  state?: string;
}

/** An installed third-party plugin (Audio Unit on macOS). */
export interface PluginInfo {
  /** Device type id, e.g. "au:…". */
  id: string;
  name: string;
  vendor: string;
  kind: "instrument" | "effect";
  format: string;
}

export interface AutoPoint {
  beat: number;
  /** volume: dB (-60..+6) · pan: -1..1 */
  value: number;
}

export type AutoParam = "volume" | "pan";

export interface Track {
  id: string;
  name: string;
  kind: TrackKind;
  color: string;
  volume: number;
  pan: number;
  mute: boolean;
  solo: boolean;
  armed: boolean;
  instrument: DeviceState | null;
  effects: DeviceState[];
  clips: Clip[];
  height: number;
  /** Automation envelopes; an empty list means "use the static value". */
  automation?: Partial<Record<AutoParam, AutoPoint[]>>;
  /** UI: which envelope is shown/edited over the lane. */
  showAutomation?: AutoParam | null;
}

export interface Section {
  id: string;
  name: string;
  start: number;
  length: number;
  color: string;
}

export interface AssetRef {
  id: string;
  path: string;
  name: string;
  duration: number;
}

export interface Project {
  version: number;
  name: string;
  bpm: number;
  timeSigNum: number;
  timeSigDen: number;
  key: string;
  loop: { enabled: boolean; start: number; end: number };
  metronome: { enabled: boolean; volume: number; countInBars: number };
  tracks: Track[];
  master: { volume: number; effects: DeviceState[] };
  assets: AssetRef[];
  sections: Section[];
}

// ---- Device catalog (from Rust `get_catalog`)

export type ParamKind = "linear" | "log" | "int" | "toggle" | "choice";

export interface ParamSpec {
  id: string;
  name: string;
  min: number;
  max: number;
  default: number;
  unit: string;
  kind: ParamKind;
  choices: string[];
  group: string;
}

export interface Preset {
  name: string;
  values: [string, number][];
}

export interface DeviceSpec {
  kind: "instrument" | "effect";
  typeId: string;
  name: string;
  category: string;
  description: string;
  params: ParamSpec[];
  presets: Preset[];
  noteNames: [number, string][];
}

export interface AssetInfo {
  id: string;
  path: string;
  name: string;
  duration: number;
  channels: number;
  sampleRate: number;
  peaks: number[];
  peaksPerSecond: number;
}

export interface EngineStatus {
  position: number;
  playing: boolean;
  recording: boolean;
  countIn: boolean;
  cpu: number;
  master: [number, number];
  input: [number, number];
  meters: Record<string, [number, number]>;
}
