// Bridge between the store and the Rust engine: project sync, transport
// status, live notes and recording.

import { create } from "zustand";
import { backend, listen, type AudioRecorded, type RecordedNote } from "./backend";
import { commit, findTrack, select, toast, useStore } from "./store";
import type { EngineStatus, Note, Project, Track } from "./types";
import { makeDevice } from "./catalog";
import { beatsPerBar, secondsToBeats, uid } from "./music";

interface TransportState extends EngineStatus {
  /** performance.now() of the last status, for playhead interpolation. */
  receivedAt: number;
  liveNotes: Set<number>;
  /** Notes being recorded, per track. */
  takeNotes: Record<string, Note[]>;
  takeStart: number | null;
}

export const useTransport = create<TransportState>(() => ({
  position: 0,
  playing: false,
  recording: false,
  countIn: false,
  cpu: 0,
  master: [0, 0],
  input: [0, 0],
  meters: {},
  receivedAt: performance.now(),
  liveNotes: new Set(),
  takeNotes: {},
  takeStart: null,
}));

/** Interpolated playhead position (beats). */
export function livePosition(): number {
  const t = useTransport.getState();
  if (!t.playing || t.countIn) return t.position;
  const p = useStore.getState().project;
  let pos = t.position + ((performance.now() - t.receivedAt) / 1000) * (p.bpm / 60);
  if (p.loop.enabled && t.position < p.loop.end && pos >= p.loop.end) {
    pos = p.loop.start + (pos - p.loop.end);
  }
  return pos;
}

// ------------------------------------------------------------------ sync

let syncTimer: ReturnType<typeof setTimeout> | null = null;
let lastSynced: Project | null = null;
let lastSyncedPreview: unknown = undefined;

/** Hidden track used to audition instruments from the browser (never saved). */
export const PREVIEW_TRACK_ID = "__preview__";
let previewTrack: { key: unknown; track: Track } | null = null;

function previewTrackFor(preview: { type: string; preset?: string }): Track {
  // Same object while the preview is unchanged, so the engine keeps its instrument.
  if (previewTrack?.key !== preview) {
    previewTrack = {
      key: preview,
      track: {
        id: PREVIEW_TRACK_ID,
        name: "Preview",
        kind: "instrument",
        color: "#888",
        volume: 0,
        pan: 0,
        mute: false,
        solo: false,
        armed: false,
        instrument: makeDevice(preview.type, preview.preset),
        effects: [],
        clips: [],
        height: 0,
      },
    };
  }
  return previewTrack.track;
}

/** What the engine plays: the project, plus the preview track while auditioning. */
function engineDocument(): Project {
  const { project, preview } = useStore.getState();
  if (!preview) return project;
  return { ...project, tracks: [...project.tracks, previewTrackFor(preview)] };
}

/** Sends the current project to the engine now (if it changed). */
export function flushSync(): Promise<void> {
  if (syncTimer) {
    clearTimeout(syncTimer);
    syncTimer = null;
  }
  const { project, preview } = useStore.getState();
  if (project === lastSynced && preview === lastSyncedPreview) return Promise.resolve();
  lastSynced = project;
  lastSyncedPreview = preview;
  return backend.syncProject(engineDocument()).catch((e) => console.error("sync", e));
}

function scheduleSync() {
  if (syncTimer) return;
  syncTimer = setTimeout(() => {
    syncTimer = null;
    flushSync();
  }, 16);
}

let lastTarget: string | null | undefined;
function updateLiveTarget() {
  const s = useStore.getState();
  const tracks = s.project.tracks;
  // A track bound to a MIDI channel gets its notes anyway: it never takes the keyboard.
  const armed = tracks.find((t) => t.armed && t.kind === "instrument" && !t.midiChannel);
  const sel = findTrack(s.project, s.selection.trackId);
  // Auditioning an instrument takes over the MIDI keyboard until it stops.
  const target = s.preview ? PREVIEW_TRACK_ID : (armed?.id ?? (sel?.kind === "instrument" ? sel.id : null));
  if (target !== lastTarget) {
    lastTarget = target;
    backend.setLiveTarget(target).catch(() => {});
  }
}

let inputEnabled = false;
function updateInput() {
  const want = useStore.getState().project.tracks.some((t) => t.kind === "audio" && t.armed);
  if (want !== inputEnabled) {
    inputEnabled = want;
    backend.setInputEnabled(want).catch((e) => {
      toast(`Audio input unavailable: ${e}`, "error");
      inputEnabled = false;
    });
  }
}

export function initEngineBridge() {
  useStore.subscribe((s, prev) => {
    if (s.project !== prev.project) {
      scheduleSync();
      updateInput();
    }
    if (s.preview !== prev.preview) scheduleSync();
    // Picking a track ends an audition: the keyboard goes back to the tracks.
    if (s.preview && s.selection.trackId !== prev.selection.trackId) useStore.setState({ preview: null });
    if (s.project !== prev.project || s.selection.trackId !== prev.selection.trackId || s.preview !== prev.preview) {
      // The preview track must exist in the engine before notes are routed to it.
      if (s.preview !== prev.preview) flushSync().then(updateLiveTarget);
      else updateLiveTarget();
    }
  });
  scheduleSync();
  updateLiveTarget();

  listen<EngineStatus>("engine-status", (st) => {
    const prev = useTransport.getState();
    useTransport.setState({ ...st, receivedAt: performance.now() });
    if (prev.recording && !st.recording && !takeHooks.finish?.()) finishMidiTake();
    if (!prev.recording && st.recording) {
      useTransport.setState({ takeStart: st.position, takeNotes: {} });
    }
  });
  listen<RecordedNote[]>("midi-recorded", (evts) => {
    if (!takeHooks.capture?.(evts)) onRecordedNotes(evts);
  });
  listen<AudioRecorded>("audio-recorded", onAudioRecorded);
  listen<[number, number, boolean, number?, boolean?]>("midi-input", ([pitch, velocity, on, , taken]) => {
    // Notes intercepted for the UI (Loop page pads, transpose keys) are not played notes.
    if (taken) return;
    const notes = new Set(useTransport.getState().liveNotes);
    if (on) notes.add(pitch);
    else notes.delete(pitch);
    useTransport.setState({ liveNotes: notes });
    notifyLive(pitch, velocity, on);
  });
}

/** Lets the Loop page take over recording: `capture` / `finish` return true when they handled it. */
export const takeHooks: {
  record: (() => boolean) | null;
  capture: ((evts: RecordedNote[]) => boolean) | null;
  finish: (() => boolean) | null;
} = { record: null, capture: null, finish: null };

// ------------------------------------------------------------------ live note listeners

type LiveListener = (pitch: number, velocity: number, on: boolean) => void;
const liveListeners = new Set<LiveListener>();

/** Every note played live (MIDI keyboard, musical typing). Velocity 1..127. */
export function onLiveNote(cb: LiveListener): () => void {
  liveListeners.add(cb);
  return () => liveListeners.delete(cb);
}

function notifyLive(pitch: number, velocity: number, on: boolean) {
  for (const cb of liveListeners) cb(pitch, velocity, on);
}

// ------------------------------------------------------------------ transport actions

export function play() {
  backend.play();
}

export function stop() {
  const t = useTransport.getState();
  backend.stop();
  if (!t.playing && !t.recording) {
    // Stop while stopped: rewind to start (or loop start).
    const p = useStore.getState().project;
    seek(p.loop.enabled && t.position > p.loop.start ? p.loop.start : 0);
  }
}

export function togglePlay() {
  const t = useTransport.getState();
  if (t.playing) backend.stop();
  else backend.play();
}

export function seek(beat: number) {
  const b = Math.max(0, beat);
  useTransport.setState({ position: b, receivedAt: performance.now() });
  backend.seek(b);
}

export async function record() {
  if (takeHooks.record?.()) return;
  const t = useTransport.getState();
  if (t.recording || t.countIn) {
    backend.stop();
    return;
  }
  const s = useStore.getState();
  let armed = s.project.tracks.filter((tr) => tr.armed);
  if (armed.length === 0) {
    const sel = findTrack(s.project, s.selection.trackId);
    if (!sel) {
      toast("Add a track to record on.", "error");
      return;
    }
    commit((p) => {
      const tr = p.tracks.find((x) => x.id === sel.id);
      if (tr) tr.armed = true;
    });
    armed = [sel];
  }
  const audio = armed.find((tr) => tr.kind === "audio");
  await flushSync();
  updateLiveTarget();
  backend.record(audio?.id ?? null, audio?.name ?? null).catch((e) => toast(`Cannot record: ${e}`, "error"));
}

/** Plays a note. With `trackId` the note goes to that track only and is never recorded
 * (editor previews); without, it goes to the live target like a MIDI keyboard. */
export function liveNote(pitch: number, velocity: number, on: boolean, trackId: string | null = null) {
  backend.liveNote(pitch, velocity, on, trackId);
  const notes = new Set(useTransport.getState().liveNotes);
  if (on) notes.add(pitch);
  else notes.delete(pitch);
  useTransport.setState({ liveNotes: notes });
  // Previews aimed at a specific track (editor keys) are not "played" notes.
  if (trackId === null) notifyLive(pitch, Math.max(1, Math.round(velocity * 127)), on);
}

// ------------------------------------------------------------------ recording

const pendingOn = new Map<string, { beat: number; velocity: number }>();

function onRecordedNotes(evts: RecordedNote[]) {
  const take = { ...useTransport.getState().takeNotes };
  for (const e of evts) {
    const key = `${e.trackId}:${e.pitch}`;
    if (e.on) {
      pendingOn.set(key, { beat: e.beat, velocity: e.velocity });
      const list = [...(take[e.trackId] ?? [])];
      list.push({ id: uid("n"), pitch: e.pitch, start: e.beat, length: 0.05, velocity: Math.max(1, Math.round(e.velocity * 127)) });
      take[e.trackId] = list;
    } else {
      const on = pendingOn.get(key);
      pendingOn.delete(key);
      if (!on) continue;
      const list = [...(take[e.trackId] ?? [])];
      for (let i = list.length - 1; i >= 0; i--) {
        if (list[i].pitch === e.pitch && Math.abs(list[i].start - on.beat) < 1e-6) {
          list[i] = { ...list[i], length: Math.max(0.05, e.beat - on.beat) };
          break;
        }
      }
      take[e.trackId] = list;
    }
  }
  useTransport.setState({ takeNotes: take });
}

function finishMidiTake() {
  const { position } = useTransport.getState();
  // Notes still held when recording stopped end at the stop position.
  const takeNotes = { ...useTransport.getState().takeNotes };
  for (const [key, on] of pendingOn) {
    const [trackId, pitchStr] = [key.slice(0, key.lastIndexOf(":")), key.slice(key.lastIndexOf(":") + 1)];
    const pitch = +pitchStr;
    takeNotes[trackId] = (takeNotes[trackId] ?? []).map((n) =>
      n.pitch === pitch && Math.abs(n.start - on.beat) < 1e-6 ? { ...n, length: Math.max(0.05, position - on.beat) } : n,
    );
  }
  pendingOn.clear();
  useTransport.setState({ takeNotes: {}, takeStart: null });
  const p = useStore.getState().project;
  const bpb = beatsPerBar(p.timeSigNum, p.timeSigDen);
  const entries = Object.entries(takeNotes).filter(([, n]) => n.length > 0);
  if (entries.length === 0) return;
  commit((d) => {
    for (const [trackId, notes] of entries) {
      const tr = d.tracks.find((t) => t.id === trackId);
      if (!tr) continue;
      const first = Math.min(...notes.map((n) => n.start));
      const last = Math.max(...notes.map((n) => n.start + n.length), position);
      const start = Math.floor(first / bpb) * bpb;
      const end = Math.max(start + bpb, Math.ceil(last / bpb) * bpb);
      tr.clips.push({
        id: uid("clip"),
        kind: "midi",
        name: `${tr.name} take`,
        start,
        length: end - start,
        muted: false,
        offset: 0,
        loopLength: 0,
        notes: notes.map((n) => ({ ...n, start: n.start - start })),
        assetId: null,
        gain: 0,
        fadeIn: 0,
        fadeOut: 0,
      });
    }
  });
  toast("MIDI take recorded", "success");
}

function onAudioRecorded(r: AudioRecorded) {
  useStore.setState((s) => ({ assets: { ...s.assets, [r.asset.id]: r.asset } }));
  const p = useStore.getState().project;
  const clipId = uid("clip");
  commit((d) => {
    const tr = d.tracks.find((t) => t.id === r.trackId);
    if (!tr) return;
    if (!d.assets.some((a) => a.id === r.asset.id)) {
      d.assets.push({ id: r.asset.id, path: r.asset.path, name: r.asset.name, duration: r.asset.duration });
    }
    const start = Math.max(0, r.startBeat);
    tr.clips.push({
      id: clipId,
      kind: "audio",
      name: tr.name,
      start,
      length: secondsToBeats(r.asset.duration, p.bpm) - (start - r.startBeat),
      muted: false,
      offset: Math.max(0, ((start - r.startBeat) * 60) / p.bpm),
      loopLength: 0,
      notes: [],
      assetId: r.asset.id,
      gain: 0,
      fadeIn: 0,
      fadeOut: 0,
    });
  });
  select({ clipIds: [clipId] });
  toast("Audio take recorded", "success");
}
