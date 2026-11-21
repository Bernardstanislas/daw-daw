// Editing operations on the project. Each exported function is one undoable step.

import type { Draft } from "immer";
import { commit, draftTrack, findClip, findTrack, select, toast, TRACK_COLORS, useStore } from "./store";
import type { Clip, DeviceState, Note, Project, Section, Track, TrackKind } from "./types";
import { makeDevice, getSpec, getPlugin } from "./catalog";
import { beatsPerBar, secondsToBeats, uid } from "./music";
import { backend } from "./backend";

// ---------------------------------------------------------------- tracks

export function newTrack(kind: TrackKind, opts: Partial<Track> = {}): Track {
  const p = useStore.getState().project;
  const color = TRACK_COLORS[p.tracks.length % TRACK_COLORS.length];
  return {
    id: uid("trk"),
    name: kind === "audio" ? `Audio ${p.tracks.filter((t) => t.kind === "audio").length + 1}` : "Instrument",
    kind,
    color,
    volume: 0,
    pan: 0,
    mute: false,
    solo: false,
    armed: false,
    instrument: kind === "instrument" ? makeDevice("pulse") : null,
    effects: [],
    clips: [],
    height: 72,
    ...opts,
  };
}

export function addTrack(kind: TrackKind, opts: Partial<Track> = {}, index?: number): string {
  const t = newTrack(kind, opts);
  commit((p) => {
    if (index === undefined) p.tracks.push(t);
    else p.tracks.splice(index, 0, t);
  });
  select({ trackId: t.id, clipIds: [] });
  return t.id;
}

export function addInstrumentTrack(type: string, preset?: string, index?: number): string {
  const spec = getSpec(type);
  const name = preset && preset !== "Default" ? preset : (spec?.name ?? getPlugin(type)?.name ?? "Instrument");
  return addTrack("instrument", { name, instrument: makeDevice(type, preset) }, index);
}

export function updateTrack(id: string, recipe: (t: Draft<Track>) => void) {
  commit((p) => {
    const t = draftTrack(p, id);
    if (t) recipe(t);
  });
}

export function deleteTrack(id: string) {
  const s = useStore.getState();
  const idx = s.project.tracks.findIndex((t) => t.id === id);
  commit((p) => {
    p.tracks = p.tracks.filter((t) => t.id !== id);
  });
  const tracks = useStore.getState().project.tracks;
  select({ trackId: tracks[Math.min(idx, tracks.length - 1)]?.id ?? null, clipIds: [] });
}

export function duplicateTrack(id: string) {
  const src = findTrack(useStore.getState().project, id);
  if (!src) return;
  const copy: Track = {
    ...structuredClone(src),
    id: uid("trk"),
    name: `${src.name} copy`,
    armed: false,
    midiChannel: null,
  };
  copy.clips = copy.clips.map((c) => ({ ...c, id: uid("clip"), notes: c.notes.map((n) => ({ ...n, id: uid("n") })) }));
  copy.effects = copy.effects.map((e) => ({ ...e, id: uid("dev") }));
  if (copy.instrument) copy.instrument = { ...copy.instrument, id: uid("dev") };
  commit((p) => {
    const i = p.tracks.findIndex((t) => t.id === id);
    p.tracks.splice(i + 1, 0, copy);
  });
  select({ trackId: copy.id });
}

export function moveTrack(id: string, toIndex: number) {
  commit((p) => {
    const i = p.tracks.findIndex((t) => t.id === id);
    if (i < 0) return;
    const [t] = p.tracks.splice(i, 1);
    p.tracks.splice(Math.max(0, Math.min(p.tracks.length, toIndex)), 0, t);
  });
}

export function toggleTrackFlag(id: string, flag: "mute" | "solo" | "armed", exclusive = false) {
  commit((p) => {
    const t = draftTrack(p, id);
    if (!t) return;
    const v = !t[flag];
    if (exclusive) for (const o of p.tracks) o[flag] = false;
    t[flag] = v;
  });
}

// ---------------------------------------------------------------- devices

export function setInstrument(trackId: string, type: string, preset?: string) {
  updateTrack(trackId, (t) => {
    t.instrument = makeDevice(type, preset);
  });
}

export function addEffect(target: string | "master", type: string, preset?: string, index?: number) {
  const dev = makeDevice(type, preset);
  commit((p) => {
    const list = target === "master" ? p.master.effects : draftTrack(p, target)?.effects;
    if (!list) return;
    if (index === undefined) list.push(dev);
    else list.splice(index, 0, dev);
  });
  return dev.id;
}

function deviceList(p: Draft<Project>, target: string | "master"): Draft<DeviceState>[] | undefined {
  return target === "master" ? p.master.effects : draftTrack(p, target)?.effects;
}

export function removeEffect(target: string | "master", deviceId: string) {
  commit((p) => {
    const list = deviceList(p, target);
    if (!list) return;
    const i = list.findIndex((d) => d.id === deviceId);
    if (i >= 0) list.splice(i, 1);
  });
}

export function moveEffect(target: string | "master", deviceId: string, delta: number) {
  commit((p) => {
    const list = deviceList(p, target);
    if (!list) return;
    const i = list.findIndex((d) => d.id === deviceId);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= list.length) return;
    const [d] = list.splice(i, 1);
    list.splice(j, 0, d);
  });
}

/** Finds a device (instrument or effect) in a draft project. */
export function draftDevice(p: Draft<Project>, deviceId: string): Draft<DeviceState> | undefined {
  for (const d of p.master.effects) if (d.id === deviceId) return d;
  for (const t of p.tracks) {
    if (t.instrument?.id === deviceId) return t.instrument;
    for (const d of t.effects) if (d.id === deviceId) return d;
  }
  return undefined;
}

export function applyPreset(deviceId: string, presetName: string) {
  commit((p) => {
    const d = draftDevice(p, deviceId);
    if (!d) return;
    const spec = getSpec(d.type);
    if (!spec) return;
    const params: Record<string, number> = {};
    for (const ps of spec.params) params[ps.id] = ps.default;
    const preset = spec.presets.find((x) => x.name === presetName);
    if (preset) for (const [k, v] of preset.values) params[k] = v;
    d.params = params;
    d.preset = presetName;
  });
}

// ---------------------------------------------------------------- clips

export function makeMidiClip(start: number, length: number, notes: Note[] = [], name = "MIDI"): Clip {
  return {
    id: uid("clip"),
    kind: "midi",
    name,
    start,
    length,
    muted: false,
    offset: 0,
    loopLength: 0,
    notes,
    assetId: null,
    gain: 0,
    fadeIn: 0,
    fadeOut: 0,
  };
}

export function createMidiClip(trackId: string, start: number, length?: number): string | null {
  const p = useStore.getState().project;
  const t = findTrack(p, trackId);
  if (!t || t.kind !== "instrument") return null;
  const bpb = beatsPerBar(p.timeSigNum, p.timeSigDen);
  const clip = makeMidiClip(Math.max(0, start), length ?? bpb, [], t.name);
  commit((d) => {
    draftTrack(d, trackId)?.clips.push(clip);
  });
  select({ trackId, clipIds: [clip.id] });
  useStore.setState({ editorClipId: clip.id });
  return clip.id;
}

export function updateClip(clipId: string, recipe: (c: Draft<Clip>) => void) {
  commit((p) => {
    for (const t of p.tracks) {
      const c = t.clips.find((c) => c.id === clipId);
      if (c) recipe(c);
    }
  });
}

export function deleteClips(ids: string[]) {
  if (!ids.length) return;
  const set = new Set(ids);
  commit((p) => {
    for (const t of p.tracks) t.clips = t.clips.filter((c) => !set.has(c.id));
  });
  const s = useStore.getState();
  useStore.setState({
    selection: { ...s.selection, clipIds: [] },
    editorClipId: s.editorClipId && set.has(s.editorClipId) ? null : s.editorClipId,
  });
}

function cloneClip(c: Clip, start: number): Clip {
  return { ...c, id: uid("clip"), start, notes: c.notes.map((n) => ({ ...n, id: uid("n") })) };
}

/** Duplicates the clips right after the selection block. */
export function duplicateClips(ids: string[]) {
  const p = useStore.getState().project;
  const found = ids.map((id) => findClip(p, id)).filter(Boolean) as { track: Track; clip: Clip }[];
  if (!found.length) return;
  const start = Math.min(...found.map((f) => f.clip.start));
  const end = Math.max(...found.map((f) => f.clip.start + f.clip.length));
  const span = end - start;
  const newIds: string[] = [];
  commit((d) => {
    for (const f of found) {
      const c = cloneClip(f.clip, f.clip.start + span);
      newIds.push(c.id);
      draftTrack(d, f.track.id)?.clips.push(c);
    }
  });
  select({ clipIds: newIds });
}

/** Splits a clip at an absolute beat. Returns the id of the right part. */
function splitClipDraft(t: Draft<Track>, c: Draft<Clip>, at: number, bpm: number): string | null {
  if (at <= c.start + 1e-6 || at >= c.start + c.length - 1e-6) return null;
  const leftLen = at - c.start;
  const right: Clip = JSON.parse(JSON.stringify(c));
  right.id = uid("clip");
  right.start = at;
  right.length = c.length - leftLen;
  if (c.kind === "midi") {
    right.offset = c.offset + leftLen;
    right.notes = right.notes.map((n) => ({ ...n, id: uid("n") }));
  } else {
    right.offset = c.offset + (leftLen * 60) / bpm;
    right.fadeIn = 0;
  }
  c.length = leftLen;
  if (c.kind === "audio") c.fadeOut = 0;
  t.clips.push(right);
  return right.id;
}

export function splitClipsAt(ids: string[], at: number) {
  const set = new Set(ids);
  const newIds: string[] = [];
  commit((p) => {
    for (const t of p.tracks) {
      for (const c of [...t.clips]) {
        if (!set.has(c.id)) continue;
        const r = splitClipDraft(t, c, at, p.bpm);
        if (r) newIds.push(r);
      }
    }
  });
  if (newIds.length) select({ clipIds: [...ids, ...newIds] });
}

// Clipboard (in memory).
let clipboard: { trackOffset: number; clip: Clip }[] = [];
let clipboardStart = 0;

export function copyClips(ids: string[]) {
  const p = useStore.getState().project;
  const found = ids.map((id) => findClip(p, id)).filter(Boolean) as { track: Track; clip: Clip }[];
  if (!found.length) return;
  const trackIdx = (id: string) => p.tracks.findIndex((t) => t.id === id);
  const minTrack = Math.min(...found.map((f) => trackIdx(f.track.id)));
  clipboardStart = Math.min(...found.map((f) => f.clip.start));
  clipboard = found.map((f) => ({ trackOffset: trackIdx(f.track.id) - minTrack, clip: structuredClone(f.clip) }));
  toast(`${found.length} clip${found.length > 1 ? "s" : ""} copied`);
}

export function cutClips(ids: string[]) {
  copyClips(ids);
  deleteClips(ids);
}

export function pasteClips(at: number) {
  if (!clipboard.length) return;
  const s = useStore.getState();
  const p = s.project;
  const base = Math.max(0, p.tracks.findIndex((t) => t.id === s.selection.trackId));
  const newIds: string[] = [];
  commit((d) => {
    for (const item of clipboard) {
      const t = d.tracks[base + item.trackOffset];
      if (!t || (t.kind === "audio") !== (item.clip.kind === "audio")) continue;
      const c = cloneClip(item.clip, at + (item.clip.start - clipboardStart));
      newIds.push(c.id);
      t.clips.push(c);
    }
  });
  select({ clipIds: newIds });
}

export function hasClipboard() {
  return clipboard.length > 0;
}

// ---------------------------------------------------------------- notes

export function quantizeNotes(clipId: string, grid: number, noteIds?: string[], strength = 1) {
  if (!grid) return;
  const set = noteIds && noteIds.length ? new Set(noteIds) : null;
  updateClip(clipId, (c) => {
    for (const n of c.notes) {
      if (set && !set.has(n.id)) continue;
      const q = Math.round(n.start / grid) * grid;
      n.start = n.start + (q - n.start) * strength;
    }
  });
}

export function transposeNotes(clipId: string, semis: number, noteIds?: string[]) {
  const set = noteIds && noteIds.length ? new Set(noteIds) : null;
  updateClip(clipId, (c) => {
    for (const n of c.notes) {
      if (set && !set.has(n.id)) continue;
      n.pitch = Math.max(0, Math.min(127, n.pitch + semis));
    }
  });
}

// ---------------------------------------------------------------- audio import

/** Imports audio files: paths on disk (desktop) or File objects (browser). */
export async function importAudioFiles(sources: (string | File)[], trackId: string | null, at: number) {
  for (const [i, source] of sources.entries()) {
    const id = uid("asset");
    const label = typeof source === "string" ? source.split("/").pop() : source.name;
    try {
      const info =
        typeof source === "string" ? await backend.importAudio(id, source) : await (await import("./webEngine")).importFile(id, source);
      const path = info.path;
      useStore.setState((s) => ({ assets: { ...s.assets, [id]: info } }));
      const p = useStore.getState().project;
      let target = trackId && i === 0 ? findTrack(p, trackId) : undefined;
      if (target && target.kind !== "audio") target = undefined;
      const len = secondsToBeats(info.duration, p.bpm);
      const clip: Clip = {
        id: uid("clip"),
        kind: "audio",
        name: info.name,
        start: at,
        length: len,
        muted: false,
        offset: 0,
        loopLength: 0,
        notes: [],
        assetId: id,
        gain: 0,
        fadeIn: 0,
        fadeOut: 0,
      };
      if (target) {
        const tid = target.id;
        commit((d) => {
          d.assets.push({ id, path, name: info.name, duration: info.duration });
          draftTrack(d, tid)?.clips.push(clip);
        });
      } else {
        const t = newTrack("audio", { name: info.name });
        t.clips.push(clip);
        commit((d) => {
          d.assets.push({ id, path, name: info.name, duration: info.duration });
          d.tracks.push(t);
        });
        select({ trackId: t.id });
      }
      select({ clipIds: [clip.id] });
    } catch (e) {
      toast(`Cannot import ${label}: ${e}`, "error", 5000);
    }
  }
}

// ---------------------------------------------------------------- arranger sections

const SECTION_COLORS = ["#6f8fb3", "#b58a5c", "#7f9e74", "#9a83b0", "#b3a05e", "#6e9f9a", "#ad7680"];
const SECTION_NAMES = ["Intro", "Verse", "Chorus", "Verse 2", "Bridge", "Chorus 2", "Outro"];

export function addSection(start: number, length: number, name?: string): string {
  const p = useStore.getState().project;
  const sec: Section = {
    id: uid("sec"),
    name: name ?? SECTION_NAMES[p.sections.length % SECTION_NAMES.length],
    start,
    length,
    color: SECTION_COLORS[p.sections.length % SECTION_COLORS.length],
  };
  commit((d) => {
    d.sections.push(sec);
    d.sections.sort((a, b) => a.start - b.start);
  });
  select({ sectionId: sec.id });
  return sec.id;
}

export function updateSection(id: string, recipe: (s: Draft<Section>) => void) {
  commit((p) => {
    const s = p.sections.find((x) => x.id === id);
    if (s) recipe(s);
    p.sections.sort((a, b) => a.start - b.start);
  });
}

export function deleteSectionMarker(id: string) {
  commit((p) => {
    p.sections = p.sections.filter((s) => s.id !== id);
  });
}

/** Splits every clip crossing `at` (used before moving song parts). */
function splitAllAt(p: Draft<Project>, at: number) {
  for (const t of p.tracks) {
    for (const c of [...t.clips]) splitClipDraft(t, c, at, p.bpm);
  }
}

type AutoMap = NonNullable<Track["automation"]>;
const AUTO_PARAMS = ["volume", "pan"] as const;

/** Removes and returns automation points in [a, b) (per track id). */
function liftAutomation(p: Draft<Project>, a: number, b: number): Map<string, AutoMap> {
  const out = new Map<string, AutoMap>();
  for (const t of p.tracks) {
    if (!t.automation) continue;
    const lifted: AutoMap = {};
    for (const k of AUTO_PARAMS) {
      const pts = t.automation[k];
      if (!pts?.length) continue;
      lifted[k] = pts.filter((x) => x.beat >= a - 1e-6 && x.beat < b - 1e-6).map((x) => ({ ...x }));
      t.automation[k] = pts.filter((x) => !(x.beat >= a - 1e-6 && x.beat < b - 1e-6));
    }
    out.set(t.id, lifted);
  }
  return out;
}

function dropAutomation(p: Draft<Project>, lifted: Map<string, AutoMap>, shift: number) {
  for (const t of p.tracks) {
    const l = lifted.get(t.id);
    if (!l) continue;
    t.automation = t.automation ?? {};
    for (const k of AUTO_PARAMS) {
      if (!l[k]?.length) continue;
      t.automation[k] = [...(t.automation[k] ?? []), ...l[k]!.map((x) => ({ beat: x.beat + shift, value: x.value }))].sort((x, y) => x.beat - y.beat);
    }
  }
}

/** Shifts everything at/after `from` by `delta` beats (clips, automation, sections). */
function shiftAfter(p: Draft<Project>, from: number, delta: number, skipSection?: string) {
  for (const t of p.tracks) for (const c of t.clips) if (c.start >= from - 1e-6) c.start += delta;
  for (const t of p.tracks)
    for (const k of AUTO_PARAMS) for (const pt of t.automation?.[k] ?? []) if (pt.beat >= from - 1e-6) pt.beat += delta;
  for (const s of p.sections) if (s.id !== skipSection && s.start >= from - 1e-6) s.start += delta;
}

/** Duplicates a section and its content right after it; later content moves right. */
export function duplicateSection(id: string) {
  const sec = useStore.getState().project.sections.find((s) => s.id === id);
  if (!sec) return;
  const end = sec.start + sec.length;
  commit((p) => {
    splitAllAt(p, sec.start);
    splitAllAt(p, end);
    const inside: { t: Draft<Track>; c: Clip }[] = [];
    for (const t of p.tracks)
      for (const c of t.clips)
        if (c.start >= sec.start - 1e-6 && c.start < end - 1e-6) inside.push({ t, c: JSON.parse(JSON.stringify(c)) });
    const autoCopy = liftAutomation(p, sec.start, end);
    dropAutomation(p, autoCopy, 0); // put the originals back
    shiftAfter(p, end, sec.length);
    dropAutomation(p, autoCopy, sec.length);
    for (const { t, c } of inside) t.clips.push(cloneClip(c, c.start + sec.length));
    p.sections.push({ ...sec, id: uid("sec"), start: end, name: `${sec.name}'` });
    p.sections.sort((a, b) => a.start - b.start);
  });
}

/** Removes a section and its content; later content moves left. */
export function deleteSection(id: string) {
  const sec = useStore.getState().project.sections.find((s) => s.id === id);
  if (!sec) return;
  const end = sec.start + sec.length;
  commit((p) => {
    splitAllAt(p, sec.start);
    splitAllAt(p, end);
    for (const t of p.tracks) t.clips = t.clips.filter((c) => !(c.start >= sec.start - 1e-6 && c.start < end - 1e-6));
    liftAutomation(p, sec.start, end);
    p.sections = p.sections.filter((s) => s.id !== id);
    shiftAfter(p, end, -sec.length);
  });
  select({ sectionId: null });
}

/** Moves a section (with its content) so that it finally starts at `to`, rippling the song
 * (other content closes the gap, then makes room at the destination). */
export function moveSection(id: string, to: number) {
  const sec = useStore.getState().project.sections.find((s) => s.id === id);
  if (!sec) return;
  const len = sec.length;
  const end = sec.start + len;
  if (Math.abs(to - sec.start) < 1e-6) return;
  commit((p) => {
    splitAllAt(p, sec.start);
    splitAllAt(p, end);
    // 1. lift the section out
    const lifted: { t: Draft<Track>; c: Clip }[] = [];
    for (const t of p.tracks) {
      const keep: Draft<Clip>[] = [];
      for (const c of t.clips) {
        if (c.start >= sec.start - 1e-6 && c.start < end - 1e-6) lifted.push({ t, c: JSON.parse(JSON.stringify(c)) });
        else keep.push(c);
      }
      t.clips = keep;
    }
    const autoLifted = liftAutomation(p, sec.start, end);
    shiftAfter(p, end, -len, id);
    // 2. insert at destination (expressed in the final timeline = collapsed timeline)
    const insertAt = Math.max(0, to);
    splitAllAt(p, insertAt);
    shiftAfter(p, insertAt, len, id);
    for (const { t, c } of lifted) t.clips.push({ ...c, start: c.start - sec.start + insertAt });
    dropAutomation(p, autoLifted, insertAt - sec.start);
    const s = p.sections.find((x) => x.id === id);
    if (s) s.start = insertAt;
    p.sections.sort((a, b) => a.start - b.start);
  });
}

export function selectSectionContent(id: string) {
  const p = useStore.getState().project;
  const sec = p.sections.find((s) => s.id === id);
  if (!sec) return;
  const end = sec.start + sec.length;
  const ids = p.tracks.flatMap((t) => t.clips.filter((c) => c.start < end && c.start + c.length > sec.start).map((c) => c.id));
  select({ clipIds: ids, sectionId: id });
}

// ---------------------------------------------------------------- project

export function setProjectField<K extends keyof Project>(key: K, value: Project[K]) {
  commit((p) => {
    (p as Project)[key] = value;
  });
}
