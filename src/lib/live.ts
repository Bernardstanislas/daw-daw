// Loop page: live looping on the project's own tracks. The project loop is the
// sequence; each track keeps its loop in one "live" MIDI clip. Recording keeps
// cycling the loop (engine overdub) and every pass played on a track is a layer
// that can be undone. KeyLab pads (bank A) mute tracks; a button lets the keys
// set the transpose of the selected track.

import { create } from "zustand";
import type { Draft } from "immer";
import type { Clip, LiveAction, Project, Track } from "./types";
import { HISTORY_LIMIT, findTrack, select, silent, commit, toast, useStore } from "./store";
import { backend, listen, type RecordedNote } from "./backend";
import { flushSync, seek, takeHooks, useTransport } from "./engine";
import { liveButtons } from "./midiMap";
import { getSpec } from "./catalog";
import { beatsPerBar, uid } from "./music";

export const LOOP_BARS = [1, 2, 4, 8];
export const PAD_COUNT = 8;

interface Layer {
  id: string;
  noteIds: string[];
}

interface LiveState {
  /** Quantize grid of recorded note starts (beats); 0 = off. */
  quantize: number;
  /** On record: add to the loop, or replace what the selected track had. */
  mode: "overdub" | "replace";
  /** Transpose takes effect now, or at the next pass of the loop. */
  apply: "now" | "next";
  /** Transposes waiting for the next pass: track id → semitones. */
  pending: Record<string, number>;
  /** The keys set the transpose of the selected track instead of playing. */
  keysTranspose: boolean;
  /** Layers recorded this session, per track (last = most recent). */
  layers: Record<string, Layer[]>;
  /** Passes of the loop since playback started. */
  pass: number;
  /** Pad learn: slot waiting for a pad, or null. */
  padLearn: number | null;
  /** Bank A pads: [channel, note] per slot (slot i mutes track i). */
  pads: ([number, number] | null)[];
}

const PADS_KEY = "dawdaw:livePads";
const PREFS_KEY = "dawdaw:live";

function loadPads(): ([number, number] | null)[] {
  try {
    const v = JSON.parse(localStorage.getItem(PADS_KEY) ?? "null");
    if (Array.isArray(v)) return Array.from({ length: PAD_COUNT }, (_, i) => (Array.isArray(v[i]) ? (v[i] as [number, number]) : null));
  } catch {
    /* unavailable */
  }
  return Array(PAD_COUNT).fill(null);
}

function loadPrefs(): Partial<LiveState> {
  try {
    const v = JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}");
    const out: Partial<LiveState> = {};
    if (typeof v.quantize === "number") out.quantize = v.quantize;
    if (v.mode === "overdub" || v.mode === "replace") out.mode = v.mode;
    if (v.apply === "now" || v.apply === "next") out.apply = v.apply;
    return out;
  } catch {
    return {};
  }
}

export const useLive = create<LiveState>(() => ({
  quantize: 0.25,
  mode: "overdub",
  apply: "next",
  pending: {},
  keysTranspose: false,
  layers: {},
  pass: 0,
  padLearn: null,
  pads: loadPads(),
  ...loadPrefs(),
}));

useLive.subscribe((s, prev) => {
  try {
    if (s.pads !== prev.pads) localStorage.setItem(PADS_KEY, JSON.stringify(s.pads));
    if (s.quantize !== prev.quantize || s.mode !== prev.mode || s.apply !== prev.apply) {
      localStorage.setItem(PREFS_KEY, JSON.stringify({ quantize: s.quantize, mode: s.mode, apply: s.apply }));
    }
  } catch {
    /* storage unavailable */
  }
});

export function setLive(patch: Partial<LiveState>) {
  useLive.setState(patch);
}

// ---------------------------------------------------------------- loop region & clips

export function loopInfo(p: Project) {
  const bpb = beatsPerBar(p.timeSigNum, p.timeSigDen);
  const len = Math.max(bpb, p.loop.end - p.loop.start);
  return { bpb, start: p.loop.start, len, bars: Math.round(len / bpb) };
}

/** The clip that holds a track's loop. */
export function liveClip(t: Track): Clip | undefined {
  return t.clips.find((c) => c.live && c.kind === "midi");
}

/** Drum kits keep their pitches: transposing them would change the drums. */
export function isDrumTrack(t: Track): boolean {
  return !!t.instrument && (getSpec(t.instrument.type)?.noteNames.length ?? 0) > 0;
}

/** The live clip of a track, created if needed and laid exactly over the loop (repeats unrolled). */
function draftLiveClip(d: Draft<Project>, trackId: string): Draft<Clip> | undefined {
  const t = d.tracks.find((x) => x.id === trackId);
  if (!t || t.kind !== "instrument") return undefined;
  const { start, len } = loopInfo(d as Project);
  let c = t.clips.find((x) => x.live && x.kind === "midi");
  if (!c) {
    t.clips.push({
      id: uid("clip"),
      kind: "midi",
      name: `${t.name} loop`,
      start,
      length: len,
      muted: false,
      offset: 0,
      loopLength: 0,
      notes: [],
      assetId: null,
      gain: 0,
      fadeIn: 0,
      fadeOut: 0,
      live: true,
    });
    c = t.clips[t.clips.length - 1];
  }
  if (c.loopLength > 0) {
    // Content repeated by a longer loop: write it out so new notes can go anywhere.
    const l = c.loopLength;
    const base = c.notes.filter((n) => n.start >= 0 && n.start < l);
    const notes = [];
    for (let k = 0; k * l < c.length + c.offset; k++) {
      for (const n of base) {
        const s = n.start + k * l;
        if (s >= c.offset && s < c.offset + c.length) notes.push({ ...n, id: k === 0 ? n.id : uid("n"), start: s });
      }
    }
    c.notes = notes;
    c.loopLength = 0;
  }
  if (c.offset !== 0) {
    c.notes = c.notes.map((n) => ({ ...n, start: n.start - c!.offset })).filter((n) => n.start >= 0);
    c.offset = 0;
  }
  c.start = start;
  c.length = len;
  c.muted = false;
  return c;
}

/** Turns the project loop on for the Loop page (a loop that is too short becomes 4 bars). */
function ensureLoop() {
  const p = useStore.getState().project;
  const { bpb } = loopInfo(p);
  const start = Math.floor(p.loop.start / bpb + 1e-6) * bpb;
  const bars = Math.round((p.loop.end - start) / bpb);
  const end = start + (bars >= 1 ? bars : 4) * bpb;
  if (p.loop.enabled && p.loop.overdub && p.loop.start === start && p.loop.end === end) return;
  silent((d) => {
    d.loop.enabled = true;
    d.loop.overdub = true;
    d.loop.start = start;
    d.loop.end = end;
  });
}

/** Changes the loop length; the existing loops repeat to fill a longer one. */
export function setBars(bars: number) {
  const p = useStore.getState().project;
  const { bpb, start, len: oldLen } = loopInfo(p);
  const len = bars * bpb;
  if (Math.abs(len - oldLen) < 1e-6) return;
  commit((d) => {
    d.loop.enabled = true;
    d.loop.end = start + len;
    for (const t of d.tracks) {
      const c = t.clips.find((x) => x.live && x.kind === "midi");
      if (!c) continue;
      if (len > oldLen && c.loopLength === 0 && c.notes.length) c.loopLength = oldLen;
      if (c.loopLength >= len) c.loopLength = 0;
      c.start = start;
      c.length = len;
    }
  });
  useLive.setState({ layers: {} });
}

export function cycleBars() {
  const { bars } = loopInfo(useStore.getState().project);
  const i = LOOP_BARS.indexOf(bars);
  setBars(LOOP_BARS[(i + 1) % LOOP_BARS.length]);
}

// ---------------------------------------------------------------- recording

interface Take {
  /** Project before the take: one undo step for the whole take. */
  base: Project;
  /** Arm flags to restore after the take. */
  armed: Record<string, boolean>;
  /** Note-ons waiting for their note-off: "track:pitch" → beat, velocity, layer. */
  held: Map<string, { beat: number; velocity: number; pass: number }>;
  no: number;
}

let take: Take | null = null;
let takeCount = 0;

function isLoopPage() {
  return useStore.getState().ui.page === "loop";
}

/** Rec on the Loop page: start a take on the loop, or end it and keep playing. */
export async function liveRecord() {
  const tr = useTransport.getState();
  if (tr.recording || tr.countIn) {
    if (tr.countIn) backend.stop();
    else backend.punchOut();
    return;
  }
  const s = useStore.getState();
  const sel = findTrack(s.project, s.selection.trackId);
  if (!sel || sel.kind !== "instrument") {
    toast("Select an instrument track to record on.", "error");
    return;
  }
  const base = s.project;
  ensureLoop();
  const armed: Record<string, boolean> = {};
  silent((d) => {
    for (const t of d.tracks) {
      armed[t.id] = t.armed;
      // The selected track takes the keys; tracks bound to a MIDI channel (pads → drums) record too.
      t.armed = t.kind === "instrument" && (t.id === sel.id || !!t.midiChannel);
    }
    if (useLive.getState().mode === "replace") {
      const c = draftLiveClip(d, sel.id);
      if (c) c.notes = [];
    }
  });
  if (useLive.getState().mode === "replace") useLive.setState((l) => ({ layers: { ...l.layers, [sel.id]: [] } }));
  take = { base, armed, held: new Map(), no: ++takeCount };
  const p = useStore.getState().project;
  const pos = useTransport.getState().position;
  if (pos < p.loop.start - 1e-6 || pos >= p.loop.end) seek(p.loop.start);
  await flushSync();
  backend.record(null, null).catch((e) => {
    take = null;
    toast(`Cannot record: ${e}`, "error");
  });
}

function addNote(trackId: string, pitch: number, on: { beat: number; velocity: number; pass: number }, offBeat: number) {
  const p = useStore.getState().project;
  const { start: ls, len } = loopInfo(p);
  const rel = (b: number) => ((((b - ls) % len) + len) % len);
  const q = useLive.getState().quantize;
  let start = rel(on.beat);
  let length = rel(offBeat) - start;
  if (length <= 0.001) length += len;
  if (q > 0) {
    start = Math.round(start / q) * q;
    if (start >= len - 1e-6) start = 0;
  }
  length = Math.min(len, Math.max(0.05, length));
  const id = uid("n");
  let added = false;
  silent((d) => {
    const c = draftLiveClip(d, trackId);
    // Replaying a note that is already there (same pitch, same start) adds nothing.
    if (c && !c.notes.some((n) => n.pitch === pitch && Math.abs(n.start - start) < 1e-6)) {
      added = true;
      c.notes.push({ id, pitch, start, length, velocity: Math.max(1, Math.round(on.velocity * 127)) });
    }
  });
  return added ? id : null;
}

function capture(evts: RecordedNote[]): boolean {
  if (!take) return false;
  for (const e of evts) {
    const key = `${e.trackId}:${e.pitch}`;
    if (e.on) {
      // Recorded notes can arrive before the status that shows the wrap: a beat well
      // behind the last known position already belongs to the next pass.
      const wrapped = e.beat < useTransport.getState().position - 0.25;
      take.held.set(key, { beat: e.beat, velocity: e.velocity, pass: useLive.getState().pass + (wrapped ? 1 : 0) });
      continue;
    }
    const on = take.held.get(key);
    take.held.delete(key);
    if (on) recordNote(e.trackId, e.pitch, on, e.beat);
  }
  return true;
}

function recordNote(trackId: string, pitch: number, on: { beat: number; velocity: number; pass: number }, offBeat: number) {
  const id = addNote(trackId, pitch, on, offBeat);
  if (!id) return;
  const layerId = `${take!.no}:${on.pass}`;
  useLive.setState((l) => {
    const list = [...(l.layers[trackId] ?? [])];
    const last = list[list.length - 1];
    if (last?.id === layerId) list[list.length - 1] = { ...last, noteIds: [...last.noteIds, id] };
    else list.push({ id: layerId, noteIds: [id] });
    return { layers: { ...l.layers, [trackId]: list } };
  });
}

function finish(): boolean {
  if (!take) return false;
  const t = take;
  take = null;
  const pos = useTransport.getState().position;
  // Keys still held when the take ended stop where it ended.
  for (const [key, on] of t.held) {
    const i = key.lastIndexOf(":");
    recordNote(key.slice(0, i), +key.slice(i + 1), on, pos);
  }
  silent((d) => {
    for (const tr of d.tracks) if (tr.id in t.armed) tr.armed = t.armed[tr.id];
  });
  const s = useStore.getState();
  if (s.project !== t.base) useStore.setState({ past: [...s.past, t.base].slice(-HISTORY_LIMIT), future: [] });
  return true;
}

export function isTaking() {
  return take !== null;
}

// ---------------------------------------------------------------- editing

function selectedTrack(): Track | undefined {
  const s = useStore.getState();
  return findTrack(s.project, s.selection.trackId);
}

/** Removes the last layer recorded on the selected track. */
export function undoLayer() {
  const t = selectedTrack();
  if (!t) return;
  const list = useLive.getState().layers[t.id] ?? [];
  const last = list[list.length - 1];
  if (!last) {
    toast(`No layer to undo on ${t.name}`);
    return;
  }
  const ids = new Set(last.noteIds);
  commit((d) => {
    const c = d.tracks.find((x) => x.id === t.id)?.clips.find((x) => x.live);
    if (c) c.notes = c.notes.filter((n) => !ids.has(n.id));
  });
  useLive.setState((l) => ({ layers: { ...l.layers, [t.id]: list.slice(0, -1) } }));
}

/** Empties the loop of the selected track. */
export function clearTrack() {
  const t = selectedTrack();
  if (!t) return;
  commit((d) => {
    const c = d.tracks.find((x) => x.id === t.id)?.clips.find((x) => x.live);
    if (c) {
      c.notes = [];
      c.transpose = 0;
    }
  });
  useLive.setState((l) => ({ layers: { ...l.layers, [t.id]: [] } }));
}

export function toggleMute(trackId: string) {
  commit((d) => {
    const t = d.tracks.find((x) => x.id === trackId);
    if (t) t.mute = !t.mute;
  });
}

function stepTrack(delta: number) {
  const s = useStore.getState();
  const list = s.project.tracks.filter((t) => t.kind === "instrument");
  if (!list.length) return;
  const i = list.findIndex((t) => t.id === s.selection.trackId);
  const next = list[(i + delta + list.length) % list.length];
  select({ trackId: next.id });
}

// ---------------------------------------------------------------- transpose (per track)

export const TRANSPOSE_RANGE = 24;

/** Semitones the loop of a track is shifted by. */
export function trackTranspose(t: Track): number {
  return liveClip(t)?.transpose ?? 0;
}

function applyTranspose(trackId: string, target: number) {
  commit((d) => {
    const c = d.tracks.find((x) => x.id === trackId)?.clips.find((x) => x.live);
    if (!c) return;
    const delta = target - (c.transpose ?? 0);
    if (!delta) return;
    for (const n of c.notes) n.pitch = Math.max(0, Math.min(127, n.pitch + delta));
    c.transpose = target;
  });
}

/** Transposes the loop of one track (at the next pass when playing and set so). */
export function setTranspose(trackId: string, semis: number) {
  const t = findTrack(useStore.getState().project, trackId);
  if (!t) return;
  if (isDrumTrack(t)) {
    toast(`${t.name}: drum kits are not transposed`);
    return;
  }
  if (!liveClip(t)) {
    toast(`${t.name} has no loop to transpose yet`);
    return;
  }
  const target = Math.max(-TRANSPOSE_RANGE, Math.min(TRANSPOSE_RANGE, Math.round(semis)));
  if (useLive.getState().apply === "next" && useTransport.getState().playing) {
    useLive.setState((l) => ({ pending: { ...l.pending, [trackId]: target } }));
  } else {
    useLive.setState((l) => {
      const { [trackId]: _, ...pending } = l.pending;
      return { pending };
    });
    applyTranspose(trackId, target);
  }
}

export function nudgeTranspose(delta: number) {
  const t = selectedTrack();
  if (!t) return;
  const cur = useLive.getState().pending[t.id] ?? trackTranspose(t);
  setTranspose(t.id, cur + delta);
}

function applyPending() {
  const pending = useLive.getState().pending;
  if (!Object.keys(pending).length) return;
  useLive.setState({ pending: {} });
  for (const [id, v] of Object.entries(pending)) applyTranspose(id, v);
}

// ---------------------------------------------------------------- hardware

/** Transpose button: a tap latches "keys set transpose", a long press holds it. */
let transposePressedAt = 0;
let transposeLatchedByPress = false;

function onLiveButton(action: LiveAction, value: number, prev: number) {
  const pressed = value >= 64 && prev < 64;
  const released = value < 64 && prev >= 64;
  if (action === "transpose") {
    if (pressed) {
      const on = !useLive.getState().keysTranspose;
      transposeLatchedByPress = on;
      transposePressedAt = performance.now();
      useLive.setState({ keysTranspose: on });
    } else if (released && transposeLatchedByPress && performance.now() - transposePressedAt > 400) {
      useLive.setState({ keysTranspose: false });
    }
    return;
  }
  if (!pressed) return;
  if (action === "undoLayer") undoLayer();
  else if (action === "length") cycleBars();
  else if (action === "prevTrack") stepTrack(-1);
  else if (action === "nextTrack") stepTrack(1);
}

export function startPadLearn() {
  useLive.setState({ padLearn: 0 });
}

export function stopPadLearn() {
  useLive.setState({ padLearn: null });
}

export function clearPads() {
  useLive.setState({ pads: Array(PAD_COUNT).fill(null), padLearn: null });
}

function onMidiNote(pitch: number, on: boolean, channel: number, taken: boolean) {
  if (!on || !isLoopPage()) return;
  const live = useLive.getState();
  if (live.padLearn !== null) {
    const slot = live.padLearn;
    const pads = live.pads.map((p) => (p && p[0] === channel && p[1] === pitch ? null : p));
    pads[slot] = [channel, pitch];
    const next = slot + 1 < PAD_COUNT ? slot + 1 : null;
    useLive.setState({ pads, padLearn: next });
    if (next === null) toast("Pads learned: bank A mutes tracks 1–8", "success");
    return;
  }
  if (!taken) return;
  const idx = live.pads.findIndex((p) => p && p[0] === channel && p[1] === pitch);
  if (idx >= 0) {
    const t = useStore.getState().project.tracks[idx];
    if (t) toggleMute(t.id);
    return;
  }
  if (live.keysTranspose) {
    const t = selectedTrack();
    if (t) setTranspose(t.id, pitch - 60);
  }
}

/** Pads and transpose keys are kept from the instruments while the Loop page is shown. */
let lastIntercept = "";
function updateIntercept() {
  const live = useLive.getState();
  const loop = isLoopPage();
  const notes = loop ? (live.pads.filter(Boolean) as [number, number][]) : [];
  const all = loop && (live.keysTranspose || live.padLearn !== null);
  const key = JSON.stringify([notes, all]);
  if (key === lastIntercept) return;
  lastIntercept = key;
  backend.setMidiIntercept(notes, all).catch(() => {});
}

export function initLive() {
  takeHooks.record = () => {
    if (!isLoopPage()) return false;
    liveRecord();
    return true;
  };
  takeHooks.capture = capture;
  takeHooks.finish = finish;
  liveButtons.handler = onLiveButton;

  listen<[number, number, boolean, number, boolean?]>("midi-input", ([pitch, , on, ch, taken]) => onMidiNote(pitch, on, ch ?? 1, !!taken));

  useStore.subscribe((s, prev) => {
    if (s.ui.page !== prev.ui.page) {
      if (s.ui.page === "loop") ensureLoop();
      else if (prev.ui.page === "loop") {
        useLive.setState({ keysTranspose: false, padLearn: null });
        // Back to linear takes elsewhere.
        if (s.project.loop.overdub && !take) silent((d) => void (d.loop.overdub = false));
      }
      updateIntercept();
    }
  });
  useLive.subscribe((s, prev) => {
    if (s.pads !== prev.pads || s.keysTranspose !== prev.keysTranspose || s.padLearn !== prev.padLearn) updateIntercept();
  });

  // Passes of the loop: layers and transposes waiting for the next pass.
  useTransport.subscribe((t, prev) => {
    if (t.playing && !prev.playing) useLive.setState({ pass: 0 });
    if (!t.playing && prev.playing) applyPending();
    if (!t.playing || t.countIn) return;
    const p = useStore.getState().project;
    if (!p.loop.enabled) return;
    if (t.position < prev.position - 0.25) {
      useLive.setState((l) => ({ pass: l.pass + 1 }));
      applyPending();
    } else if (p.loop.end - t.position < 0.08 && p.loop.end - prev.position >= 0.08) {
      // Just before the wrap, so the first notes of the next pass already sound transposed.
      applyPending();
    }
  });
  if (isLoopPage()) ensureLoop();
  updateIntercept();
}
