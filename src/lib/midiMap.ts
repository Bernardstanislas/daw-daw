// Hardware controllers: MIDI learn (a knob, fader or button of the keyboard drives
// any setting of the project) and per-track MIDI channels (pads → drums, keys → synth).
// Notes are routed by the engine itself; controllers (CC) are applied here.

import { create } from "zustand";
import { produce, type Draft } from "immer";
import type { LiveAction, MidiMapping, MidiTarget, Project } from "./types";
import { HISTORY_LIMIT, commit, findTrack, toast, useStore } from "./store";
import { listen } from "./backend";
import { getSpec, deviceName } from "./catalog";
import { clamp, faderToDb, uid } from "./music";
import { fromNorm } from "../components/controls";
import { play, record, stop } from "./engine";

export interface MidiEvent {
  type: "note" | "cc";
  channel: number;
  /** Note number or CC number. */
  number: number;
  value: number;
}

interface LearnState {
  /** MIDI learn mode: click a control, then move a knob of the keyboard. */
  active: boolean;
  /** Control waiting for a CC. */
  armed: MidiTarget | null;
  /** Track waiting for a note to take its channel. */
  channelLearn: string | null;
  /** Last message received, for the activity monitor. */
  last: MidiEvent | null;
}

export const useMidiLearn = create<LearnState>(() => ({ active: false, armed: null, channelLearn: null, last: null }));

export function setLearnMode(active: boolean) {
  useMidiLearn.setState({ active, armed: null });
}

// ---------------------------------------------------------------- targets

export function targetKey(t: MidiTarget): string {
  switch (t.kind) {
    case "param":
      return `param:${t.deviceId}:${t.paramId}`;
    case "track":
      return `track:${t.trackId}:${t.prop}`;
    case "transport":
      return `transport:${t.action}`;
    case "live":
      return `live:${t.action}`;
    default:
      return t.kind;
  }
}

export function parseTargetKey(k: string): MidiTarget | null {
  const [kind, a, b] = k.split(":");
  if (kind === "param" && a && b) return { kind, deviceId: a, paramId: b };
  if (kind === "track" && a && b) return { kind, trackId: a, prop: b as "volume" };
  if (kind === "transport" && a) return { kind, action: a as "play" };
  if (kind === "master" || kind === "tempo") return { kind };
  if (kind === "live" && a && a in LIVE_NAMES) return { kind, action: a as LiveAction };
  return null;
}

function findDevice(p: Project, deviceId: string) {
  for (const t of p.tracks) {
    if (t.instrument?.id === deviceId) return { track: t.name, device: t.instrument };
    const fx = t.effects.find((d) => d.id === deviceId);
    if (fx) return { track: t.name, device: fx };
  }
  const fx = p.master.effects.find((d) => d.id === deviceId);
  return fx ? { track: "Master", device: fx } : undefined;
}

function draftDevice(p: Draft<Project>, deviceId: string) {
  for (const t of p.tracks) {
    if (t.instrument?.id === deviceId) return t.instrument;
    const fx = t.effects.find((d) => d.id === deviceId);
    if (fx) return fx;
  }
  return p.master.effects.find((d) => d.id === deviceId);
}

export const LIVE_NAMES: Record<LiveAction, string> = {
  undoLayer: "Undo layer",
  length: "Loop length",
  prevTrack: "Previous track",
  nextTrack: "Next track",
  transpose: "Keys set transpose",
};

/** Loop page buttons: the handler gets every value (press and release). Set by live.ts. */
export const liveButtons: { handler: ((action: LiveAction, value: number, prev: number) => void) | null } = { handler: null };

const PROP_NAMES = { volume: "Volume", pan: "Pan", mute: "Mute", solo: "Solo", armed: "Arm" };

/** Human name of a target, or null when it no longer exists. */
export function targetLabel(p: Project, t: MidiTarget): string | null {
  switch (t.kind) {
    case "param": {
      const f = findDevice(p, t.deviceId);
      if (!f) return null;
      const spec = getSpec(f.device.type)?.params.find((x) => x.id === t.paramId);
      return `${f.track} · ${deviceName(f.device)} · ${spec?.name ?? t.paramId}`;
    }
    case "track": {
      const tr = findTrack(p, t.trackId);
      return tr ? `${tr.name} · ${PROP_NAMES[t.prop]}` : null;
    }
    case "master":
      return "Master · Volume";
    case "tempo":
      return "Tempo";
    case "transport":
      return `Transport · ${t.action[0].toUpperCase()}${t.action.slice(1)}`;
    case "live":
      return `Loop · ${LIVE_NAMES[t.action]}`;
  }
}

export function ccLabel(m: { channel: number; cc: number }): string {
  return `CC${m.cc}${m.channel !== 1 ? ` ch${m.channel}` : ""}`;
}

// ---------------------------------------------------------------- applying CC values

/** Continuous moves are grouped into one undo step per gesture. */
let gestureBase: Project | null = null;
let gesturePast = 0;
let gestureTimer: ReturnType<typeof setTimeout> | null = null;

function applyContinuous(recipe: (p: Draft<Project>) => void) {
  const s = useStore.getState();
  if (!gestureBase) {
    gestureBase = s.project;
    gesturePast = s.past.length;
  }
  const next = produce(s.project, recipe);
  if (next !== s.project) useStore.setState({ project: next });
  if (gestureTimer) clearTimeout(gestureTimer);
  gestureTimer = setTimeout(() => {
    const st = useStore.getState();
    // Only if nothing else touched the history meanwhile.
    if (gestureBase && gestureBase !== st.project && st.past.length === gesturePast) {
      useStore.setState({ past: [...st.past, gestureBase].slice(-HISTORY_LIMIT), future: [] });
    }
    gestureBase = null;
    gestureTimer = null;
  }, 600);
}

/** Last value per mapping: buttons act on press (value crossing 64 upwards). */
const lastValue = new Map<string, number>();

function applyMapping(m: MidiMapping, value: number) {
  const prev = lastValue.get(m.id) ?? 0;
  lastValue.set(m.id, value);
  const pressed = value >= 64 && prev < 64;
  const norm = value / 127;
  const t = m.target;
  switch (t.kind) {
    case "param": {
      const f = findDevice(useStore.getState().project, t.deviceId);
      const spec = f && getSpec(f.device.type)?.params.find((x) => x.id === t.paramId);
      if (!spec) return;
      if (spec.kind === "toggle") {
        if (pressed) commit((p) => void (draftDevice(p, t.deviceId)!.params[t.paramId] = (f.device.params[t.paramId] ?? spec.default) >= 0.5 ? 0 : 1));
        return;
      }
      let v: number;
      if (spec.kind === "choice") v = Math.round(norm * Math.max(0, spec.choices.length - 1));
      else {
        v = fromNorm(norm, spec.min, spec.max, spec.kind === "log");
        if (spec.kind === "int") v = Math.round(v);
      }
      applyContinuous((p) => {
        const d = draftDevice(p, t.deviceId);
        if (d) d.params[t.paramId] = v;
      });
      return;
    }
    case "track": {
      if (t.prop === "volume" || t.prop === "pan") {
        const v = t.prop === "volume" ? Math.round(faderToDb(norm) * 10) / 10 : Math.round(clamp((value - 64) / 63, -1, 1) * 100) / 100;
        applyContinuous((p) => {
          const tr = p.tracks.find((x) => x.id === t.trackId);
          if (tr) tr[t.prop as "volume" | "pan"] = v;
        });
      } else if (pressed) {
        commit((p) => {
          const tr = p.tracks.find((x) => x.id === t.trackId);
          if (tr) tr[t.prop as "mute" | "solo" | "armed"] = !tr[t.prop as "mute"];
        });
      }
      return;
    }
    case "master": {
      const v = Math.round(faderToDb(norm) * 10) / 10;
      applyContinuous((p) => void (p.master.volume = v));
      return;
    }
    case "tempo": {
      const v = Math.round(40 + norm * 200);
      applyContinuous((p) => void (p.bpm = v));
      return;
    }
    case "transport": {
      if (!pressed) return;
      if (t.action === "play") play();
      else if (t.action === "stop") stop();
      else if (t.action === "record") record();
      else if (t.action === "loop") commit((p) => void (p.loop.enabled = !p.loop.enabled));
      else if (t.action === "metronome") commit((p) => void (p.metronome.enabled = !p.metronome.enabled));
      return;
    }
    case "live":
      liveButtons.handler?.(t.action, value, prev);
      return;
  }
}

// ---------------------------------------------------------------- mapping edits

export function addMapping(channel: number, cc: number, target: MidiTarget) {
  const key = targetKey(target);
  commit((p) => {
    // One control per CC, one CC per control.
    const kept = (p.midiMap ?? []).filter((m) => !(m.channel === channel && m.cc === cc) && targetKey(m.target) !== key);
    p.midiMap = [...kept, { id: uid("map"), channel, cc, target }];
  });
}

export function removeMapping(id: string) {
  commit((p) => {
    p.midiMap = (p.midiMap ?? []).filter((m) => m.id !== id);
  });
}

export function removeMappingFor(target: MidiTarget) {
  const key = targetKey(target);
  const m = (useStore.getState().project.midiMap ?? []).find((x) => targetKey(x.target) === key);
  if (m) removeMapping(m.id);
}

/** Sets the channel a track listens to; a channel belongs to one track at a time. */
export function setTrackChannel(trackId: string, channel: number | null) {
  commit((p) => {
    for (const t of p.tracks) {
      if (t.id === trackId) t.midiChannel = channel;
      else if (channel !== null && t.midiChannel === channel) t.midiChannel = null;
    }
  });
}

// ---------------------------------------------------------------- input

export function onCC(channel: number, cc: number, value: number) {
  const learn = useMidiLearn.getState();
  useMidiLearn.setState({ last: { type: "cc", channel, number: cc, value } });
  if (learn.active && learn.armed) {
    const target = learn.armed;
    useMidiLearn.setState({ armed: null });
    addMapping(channel, cc, target);
    toast(`${ccLabel({ channel, cc })} → ${targetLabel(useStore.getState().project, target) ?? "control"}`, "success");
    return;
  }
  for (const m of useStore.getState().project.midiMap ?? []) {
    if (m.channel === channel && m.cc === cc) applyMapping(m, value);
  }
}

function onNote(pitch: number, velocity: number, on: boolean, channel: number) {
  useMidiLearn.setState({ last: { type: "note", channel, number: pitch, value: velocity } });
  const trackId = useMidiLearn.getState().channelLearn;
  if (on && trackId) {
    useMidiLearn.setState({ channelLearn: null });
    setTrackChannel(trackId, channel);
    toast(`${findTrack(useStore.getState().project, trackId)?.name ?? "Track"} now plays MIDI channel ${channel}`, "success");
  }
}

export function initMidiControl() {
  listen<[number, number, number]>("midi-cc", ([ch, cc, v]) => onCC(ch, cc, v));
  listen<[number, number, boolean, number, boolean?]>("midi-input", ([pitch, vel, on, ch, taken]) => {
    if (!taken) onNote(pitch, vel, on, ch ?? 1);
  });

  // Learn mode: clicking a learnable control arms it instead of using it;
  // right-click removes its mapping. Escape leaves the mode.
  const pick = (e: Event) => {
    if (!useMidiLearn.getState().active) return;
    const el = (e.target as HTMLElement | null)?.closest<HTMLElement>("[data-learn]");
    if (!el) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.type !== "pointerdown" && e.type !== "contextmenu") return;
    const target = parseTargetKey(el.dataset.learn!);
    if (!target) return;
    if (e.type === "contextmenu") removeMappingFor(target);
    else useMidiLearn.setState({ armed: target });
  };
  for (const type of ["pointerdown", "mousedown", "click", "dblclick", "contextmenu", "wheel"]) {
    window.addEventListener(type, pick, { capture: true, passive: false });
  }
  window.addEventListener(
    "keydown",
    (e) => {
      const s = useMidiLearn.getState();
      if (e.key === "Escape" && (s.active || s.channelLearn)) {
        e.stopPropagation();
        useMidiLearn.setState({ active: false, armed: null, channelLearn: null });
      }
    },
    { capture: true },
  );
}

// ---------------------------------------------------------------- control props

/** Props for a control that MIDI learn can map (spread on its root element). */
export function useLearn(target: MidiTarget | undefined): Record<string, string | undefined> {
  const key = target ? targetKey(target) : undefined;
  const mapping = useStore((s) => (key ? s.project.midiMap?.find((m) => targetKey(m.target) === key) : undefined));
  const armed = useMidiLearn((s) => (key && s.armed ? targetKey(s.armed) === key : false));
  if (!key) return {};
  return {
    "data-learn": key,
    "data-learn-cc": mapping ? ccLabel(mapping) : undefined,
    "data-learn-armed": armed ? "1" : undefined,
  };
}
