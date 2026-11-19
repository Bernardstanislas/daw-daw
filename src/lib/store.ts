// Application store: the project document, undo/redo history, selection and
// UI state. Project updates are immutable (immer) so history snapshots are cheap.

import { create } from "zustand";
import { produce, type Draft } from "immer";
import type { AssetInfo, Clip, Project, Track } from "./types";
import type { SnapValue } from "./music";

export type BottomPanel = "editor" | null;

/** Pages: one full-window workspace per stage of the work, all on the same project and transport. */
export type Page = "arrange" | "sound" | "mix";
export const PAGES: { id: Page; label: string; hint: string }[] = [
  { id: "arrange", label: "Arrange", hint: "Timeline, clips and piano roll" },
  { id: "sound", label: "Sound", hint: "Instrument and effect chain of one track" },
  { id: "mix", label: "Mix", hint: "Console: levels, pan and inserts" },
];

export interface Selection {
  trackId: string | null;
  clipIds: string[];
  noteIds: string[];
  sectionId: string | null;
}

export interface UiState {
  page: Page;
  pxPerBeat: number;
  snap: SnapValue;
  browser: boolean;
  bottom: BottomPanel;
  bottomHeight: number;
  follow: boolean;
  typing: boolean;
  editorPxPerBeat: number;
  editorRowHeight: number;
  /** MIDI editor view; "auto" = steps for drum kits, piano roll otherwise. */
  editorMode: "auto" | "piano" | "steps";
  browserTab: "instruments" | "effects" | "loops" | "files";
  browserWidth: number;
}

export interface ToastMsg {
  id: number;
  text: string;
  kind: "info" | "error" | "success";
}

interface AppStore {
  project: Project;
  projectPath: string | null;
  savedProject: Project | null;
  past: Project[];
  future: Project[];
  gestureBase: Project | null;
  selection: Selection;
  editorClipId: string | null;
  ui: UiState;
  assets: Record<string, AssetInfo>;
  toasts: ToastMsg[];
  dialog: null | "settings" | "export" | "help" | "new";
  exportProgress: number | null;
  /** Instrument auditioned from the browser (played live, not part of the project). */
  preview: { type: string; preset?: string } | null;
}

export const HISTORY_LIMIT = 200;

/** Track colors: vintage gear and tape-label hues, soft enough to look at all night. */
export const TRACK_COLORS = [
  "#d4805f", // brick
  "#d9a35a", // amber
  "#c9b66a", // mustard
  "#93ad7a", // olive
  "#6fa89c", // teal
  "#7c9cc4", // slate
  "#a08cbf", // lilac
  "#c4848f", // dusty rose
  "#a7a196", // sand
  "#8e9aa6", // steel
];

export function emptyProject(): Project {
  return {
    version: 1,
    name: "Untitled",
    bpm: 120,
    timeSigNum: 4,
    timeSigDen: 4,
    key: "C",
    loop: { enabled: false, start: 0, end: 16 },
    metronome: { enabled: false, volume: 0.7, countInBars: 1 },
    tracks: [],
    master: { volume: 0, effects: [] },
    assets: [],
    sections: [],
  };
}

// Panel sizes survive restarts.
const UI_KEY = "dawdaw:ui";
const PERSISTED_UI = ["browserWidth", "bottomHeight", "browser"] as const;

function savedUi(): Partial<UiState> {
  try {
    const raw = JSON.parse(localStorage.getItem(UI_KEY) ?? "{}");
    const out: Record<string, unknown> = {};
    for (const k of PERSISTED_UI) if (k in raw) out[k] = raw[k];
    return out as Partial<UiState>;
  } catch {
    return {};
  }
}

export const useStore = create<AppStore>(() => ({
  project: emptyProject(),
  projectPath: null,
  savedProject: null,
  past: [],
  future: [],
  gestureBase: null,
  selection: { trackId: null, clipIds: [], noteIds: [], sectionId: null },
  editorClipId: null,
  ui: {
    page: "arrange",
    pxPerBeat: 28,
    snap: "1/4",
    browser: true,
    bottom: "editor",
    bottomHeight: 300,
    follow: true,
    typing: false,
    editorPxPerBeat: 80,
    editorRowHeight: 14,
    editorMode: "auto",
    browserTab: "instruments",
    browserWidth: 250,
    ...savedUi(),
  },
  assets: {},
  toasts: [],
  dialog: null,
  exportProgress: null,
  preview: null,
}));

// ---------------------------------------------------------------- history

/** Applies a change to the project and records it in the undo history. */
export function commit(recipe: (p: Draft<Project>) => void) {
  const s = useStore.getState();
  const next = produce(s.project, recipe);
  if (next === s.project) return;
  useStore.setState({
    project: next,
    past: [...s.past, s.project].slice(-HISTORY_LIMIT),
    future: [],
  });
}

/** Applies a change without touching history (e.g. transient UI fields). */
export function silent(recipe: (p: Draft<Project>) => void) {
  const s = useStore.getState();
  const next = produce(s.project, recipe);
  if (next !== s.project) useStore.setState({ project: next });
}

/** Starts a continuous edit (drag): later `gesture()` calls are relative to the base. */
export function beginGesture() {
  useStore.setState({ gestureBase: useStore.getState().project });
}

/** Re-applies `recipe` on top of the gesture base. */
export function gesture(recipe: (p: Draft<Project>) => void) {
  const s = useStore.getState();
  const base = s.gestureBase ?? s.project;
  useStore.setState({ project: produce(base, recipe) });
}

export function endGesture() {
  const s = useStore.getState();
  const base = s.gestureBase;
  if (base && base !== s.project) {
    useStore.setState({ past: [...s.past, base].slice(-HISTORY_LIMIT), future: [], gestureBase: null });
  } else {
    useStore.setState({ gestureBase: null });
  }
}

export function cancelGesture() {
  const s = useStore.getState();
  if (s.gestureBase) useStore.setState({ project: s.gestureBase, gestureBase: null });
}

export function undo() {
  const s = useStore.getState();
  const prev = s.past[s.past.length - 1];
  if (!prev) return;
  useStore.setState({ project: prev, past: s.past.slice(0, -1), future: [s.project, ...s.future] });
  sanitizeSelection();
}

export function redo() {
  const s = useStore.getState();
  const next = s.future[0];
  if (!next) return;
  useStore.setState({ project: next, past: [...s.past, s.project], future: s.future.slice(1) });
  sanitizeSelection();
}

function sanitizeSelection() {
  const s = useStore.getState();
  const tracks = s.project.tracks;
  const clipIds = new Set(tracks.flatMap((t) => t.clips.map((c) => c.id)));
  const trackId = tracks.some((t) => t.id === s.selection.trackId) ? s.selection.trackId : (tracks[0]?.id ?? null);
  useStore.setState({
    selection: {
      ...s.selection,
      trackId,
      clipIds: s.selection.clipIds.filter((id) => clipIds.has(id)),
    },
    editorClipId: s.editorClipId && clipIds.has(s.editorClipId) ? s.editorClipId : null,
  });
}

// ---------------------------------------------------------------- UI helpers

export function setUi(patch: Partial<UiState>) {
  useStore.setState((s) => ({ ui: { ...s.ui, ...patch } }));
}

/** Opens a track on the Sound page (its instrument and effect chain). */
export function openInSound(trackId: string) {
  useStore.setState((s) => ({ selection: { ...s.selection, trackId }, ui: { ...s.ui, page: "sound" } }));
}

export function select(patch: Partial<Selection>) {
  useStore.setState((s) => ({ selection: { ...s.selection, ...patch } }));
}

let toastId = 0;
export function toast(text: string, kind: ToastMsg["kind"] = "info", ms = 3200) {
  const id = ++toastId;
  useStore.setState((s) => ({ toasts: [...s.toasts, { id, text, kind }] }));
  setTimeout(() => useStore.setState((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), ms);
}

// ---------------------------------------------------------------- lookups

export function findTrack(p: Project, id: string | null | undefined): Track | undefined {
  return id ? p.tracks.find((t) => t.id === id) : undefined;
}

export function findClip(p: Project, clipId: string | null | undefined): { track: Track; clip: Clip } | undefined {
  if (!clipId) return undefined;
  for (const track of p.tracks) {
    const clip = track.clips.find((c) => c.id === clipId);
    if (clip) return { track, clip };
  }
  return undefined;
}

export function draftClip(p: Draft<Project>, clipId: string): Draft<Clip> | undefined {
  for (const t of p.tracks) {
    const c = t.clips.find((c) => c.id === clipId);
    if (c) return c;
  }
  return undefined;
}

export function draftTrack(p: Draft<Project>, id: string): Draft<Track> | undefined {
  return p.tracks.find((t) => t.id === id);
}

export function isDirty(): boolean {
  const s = useStore.getState();
  return s.savedProject !== s.project && (s.savedProject !== null || s.project.tracks.length > 0);
}

useStore.subscribe((s, prev) => {
  if (s.ui === prev.ui) return;
  const out: Record<string, unknown> = {};
  for (const k of PERSISTED_UI) out[k] = s.ui[k];
  try {
    localStorage.setItem(UI_KEY, JSON.stringify(out));
  } catch {
    /* storage full / unavailable */
  }
});
