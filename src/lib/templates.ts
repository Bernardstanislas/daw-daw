// Project templates for "New project".

import type { Clip, Project, Track } from "./types";
import { emptyProject, TRACK_COLORS } from "./store";
import { makeDevice, getSpec } from "./catalog";
import { findPattern, patternNotes } from "./patterns";
import { uid } from "./music";

function track(name: string, colorIdx: number, type: string, preset?: string): Track {
  return {
    id: uid("trk"),
    name,
    kind: "instrument",
    color: TRACK_COLORS[colorIdx % TRACK_COLORS.length],
    volume: 0,
    pan: 0,
    mute: false,
    solo: false,
    armed: false,
    instrument: makeDevice(type, preset),
    effects: [],
    clips: [],
    height: 72,
  };
}

function patternClip(patternId: string, start: number, length: number, name?: string): Clip {
  const p = findPattern(patternId)!;
  return {
    id: uid("clip"),
    kind: "midi",
    name: name ?? p.name,
    start,
    length,
    muted: false,
    offset: 0,
    loopLength: p.length,
    notes: patternNotes(p),
    assetId: null,
    gain: 0,
    fadeIn: 0,
    fadeOut: 0,
  };
}

function hasPreset(type: string, preset: string) {
  return !!getSpec(type)?.presets.some((p) => p.name === preset);
}

export interface Template {
  id: string;
  name: string;
  description: string;
  build: () => Project;
}

export const TEMPLATES: Template[] = [
  {
    id: "empty",
    name: "Empty project",
    description: "A blank canvas with one instrument track.",
    build: () => {
      const p = emptyProject();
      p.tracks = [track("Pulse Synth", 5, "pulse")];
      return p;
    },
  },
  {
    id: "beat",
    name: "Night drive (demo)",
    description: "Drums, bass, pad and arp over an Intro / Verse / Chorus arrangement.",
    build: () => {
      const p = emptyProject();
      p.name = "Night Drive";
      p.bpm = 112;
      const drums = track("Drums", 0, "kit");
      drums.clips = [
        patternClip("drum-house", 16, 28, "Beat"),
        patternClip("drum-fill", 44, 4, "Fill"),
        patternClip("drum-house", 48, 32, "Beat"),
      ];
      const bass = track("Bass", 1, "pulse", hasPreset("pulse", "Lead Weight Bass") ? "Lead Weight Bass" : undefined);
      bass.clips = [patternClip("bass-eighths", 16, 64, "Bassline")];
      bass.volume = -3;
      const pad = track("Strings", 6, "sampler", hasPreset("sampler", "Strings") ? "Strings" : undefined);
      pad.clips = [patternClip("chords-minor", 0, 80, "Chords")];
      pad.volume = -6;
      pad.effects = [makeDevice("reverb")];
      const arp = track("Arp", 4, "pulse", hasPreset("pulse", "Glass Pluck") ? "Glass Pluck" : undefined);
      arp.clips = [patternClip("mel-arp", 48, 32, "Arp")];
      arp.volume = -8;
      arp.pan = 0.25;
      arp.effects = [makeDevice("delay")];
      p.tracks = [drums, bass, pad, arp];
      p.sections = [
        { id: uid("sec"), name: "Intro", start: 0, length: 16, color: "#6f8fb3" },
        { id: uid("sec"), name: "Verse", start: 16, length: 32, color: "#b58a5c" },
        { id: uid("sec"), name: "Chorus", start: 48, length: 32, color: "#7f9e74" },
      ];
      p.loop = { enabled: false, start: 16, end: 48 };
      p.master.effects = [makeDevice("limiter")];
      return p;
    },
  },
  {
    id: "songwriter",
    name: "Songwriter",
    description: "Audio track for vocals/guitar, keys and a simple beat.",
    build: () => {
      const p = emptyProject();
      p.name = "New Song";
      p.bpm = 90;
      const voc: Track = { ...track("Vocals", 8, "pulse"), kind: "audio", instrument: null, effects: [makeDevice("eq"), makeDevice("compressor"), makeDevice("reverb")] };
      const keys = track("Piano", 6, "sampler", hasPreset("sampler", "Grand Piano") ? "Grand Piano" : undefined);
      keys.clips = [patternClip("chords-pop", 0, 32, "Chords")];
      const drums = track("Drums", 0, "samplekit", "TightKit");
      drums.clips = [patternClip("drum-boombap", 0, 32, "Beat")];
      drums.volume = -4;
      p.tracks = [voc, keys, drums];
      p.metronome.enabled = true;
      return p;
    },
  },
];
