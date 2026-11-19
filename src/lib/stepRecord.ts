// Step input: notes played on a MIDI keyboard (or with musical typing) are
// written one step at a time at a cursor in the open clip. Keys held together
// make a chord; the cursor moves one step forward when every key is released.

import { create } from "zustand";
import { commit, draftClip, findClip, useStore } from "./store";
import { livePosition, onLiveNote } from "./engine";
import { beatsPerBar, uid } from "./music";

export const STEP_LENGTHS: { label: string; beats: number }[] = [
  { label: "1/32", beats: 0.125 },
  { label: "1/16", beats: 0.25 },
  { label: "1/8", beats: 0.5 },
  { label: "1/4", beats: 1 },
  { label: "1/2", beats: 2 },
  { label: "1 bar", beats: 4 },
  { label: "1/8 triplet", beats: 1 / 3 },
  { label: "1/16 triplet", beats: 1 / 6 },
];

interface StepRecordState {
  active: boolean;
  clipId: string | null;
  /** Insert position, in clip content beats (same space as note.start). */
  cursor: number;
  /** Step length (beats). */
  step: number;
  /** Note length as a fraction of the step (legato = 1). */
  gate: number;
}

export const useStepRecord = create<StepRecordState>(() => ({
  active: false,
  clipId: null,
  cursor: 0,
  step: Number(localStorage.getItem("dawdaw:stepInput")) || 0.25,
  gate: 0.9,
}));

const held = new Set<number>();
/** Start of the chord being entered (all its notes share it). */
let chordAt: number | null = null;

function clipNow() {
  const { clipId } = useStepRecord.getState();
  return findClip(useStore.getState().project, clipId)?.clip;
}

/** Starts step input on a clip, at the playhead if it is inside the clip, else at its start. */
export function startStepRecord(clipId: string) {
  const found = findClip(useStore.getState().project, clipId);
  if (!found) return;
  const { clip } = found;
  const { step } = useStepRecord.getState();
  const rel = livePosition() - clip.start;
  const inside = rel >= 0 && rel < clip.length;
  const cursor = inside ? Math.floor((rel + clip.offset) / step + 1e-6) * step : Math.max(0, clip.offset);
  held.clear();
  chordAt = null;
  useStepRecord.setState({ active: true, clipId, cursor });
}

export function stopStepRecord() {
  held.clear();
  chordAt = null;
  useStepRecord.setState({ active: false });
}

export function setStepLength(beats: number) {
  localStorage.setItem("dawdaw:stepInput", String(beats));
  useStepRecord.setState({ step: beats });
}

/** Grows the clip (and its loop) so that `end` (content beats) fits inside. */
function ensureFits(clipId: string, end: number) {
  const clip = clipNow();
  if (!clip) return;
  const p = useStore.getState().project;
  const bar = beatsPerBar(p.timeSigNum, p.timeSigDen);
  const visibleEnd = clip.offset + clip.length;
  const loopEnd = clip.loopLength > 0 ? clip.loopLength : Infinity;
  if (end <= visibleEnd + 1e-6 && end <= loopEnd + 1e-6) return;
  commit((d) => {
    const c = draftClip(d, clipId);
    if (!c) return;
    const target = Math.ceil((end - 1e-6) / bar) * bar;
    if (c.loopLength > 0 && end > c.loopLength + 1e-6) c.loopLength = target;
    if (end > c.offset + c.length + 1e-6) c.length = Math.max(c.length, target - c.offset);
  });
}

/** Moves the cursor one step forward (a rest). */
export function stepRest() {
  const s = useStepRecord.getState();
  if (!s.active || !s.clipId) return;
  const next = s.cursor + s.step;
  ensureFits(s.clipId, next);
  useStepRecord.setState({ cursor: next });
}

/** Moves the cursor one step back. */
export function stepBack() {
  const s = useStepRecord.getState();
  if (!s.active) return;
  useStepRecord.setState({ cursor: Math.max(0, s.cursor - s.step) });
}

/** Deletes the notes of the previous step and moves back onto it. */
export function stepDeleteBack() {
  const s = useStepRecord.getState();
  if (!s.active || !s.clipId) return;
  const from = Math.max(0, s.cursor - s.step);
  const to = s.cursor;
  commit((d) => {
    const c = draftClip(d, s.clipId!);
    if (c) c.notes = c.notes.filter((n) => !(n.start >= from - 1e-6 && n.start < to - 1e-6));
  });
  useStepRecord.setState({ cursor: from });
}

function onNote(pitch: number, velocity: number, on: boolean) {
  const s = useStepRecord.getState();
  if (!s.active || !s.clipId) return;
  if (on) {
    if (held.size === 0) chordAt = s.cursor;
    held.add(pitch);
    const start = chordAt ?? s.cursor;
    ensureFits(s.clipId, start + s.step);
    commit((d) => {
      const c = draftClip(d, s.clipId!);
      if (!c) return;
      // Replaying a pitch on the same step replaces it.
      c.notes = c.notes.filter((n) => !(n.pitch === pitch && Math.abs(n.start - start) < 1e-6));
      c.notes.push({ id: uid("n"), pitch, start, length: s.step * s.gate, velocity: Math.max(1, Math.min(127, velocity)) });
    });
  } else if (held.delete(pitch) && held.size === 0 && chordAt !== null) {
    useStepRecord.setState({ cursor: chordAt + s.step });
    chordAt = null;
  }
}

/** Wires step input to live notes and stops it when its clip goes away. */
export function initStepRecord() {
  onLiveNote(onNote);
  useStore.subscribe((st, prev) => {
    const s = useStepRecord.getState();
    if (!s.active) return;
    if (st.editorClipId !== prev.editorClipId || !findClip(st.project, s.clipId)) stopStepRecord();
  });
}

function isEditable(t: EventTarget | null) {
  const el = t as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
}

/** While step input is on: → rest, ← back, ⌫ delete previous step, Esc stop. */
export function initStepRecordKeys() {
  window.addEventListener(
    "keydown",
    (e) => {
      if (!useStepRecord.getState().active || isEditable(e.target) || e.metaKey || e.ctrlKey) return;
      const actions: Record<string, () => void> = {
        ArrowRight: stepRest,
        ArrowLeft: stepBack,
        Backspace: stepDeleteBack,
        Escape: stopStepRecord,
      };
      const act = actions[e.key];
      if (!act) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      act();
    },
    true,
  );
}
