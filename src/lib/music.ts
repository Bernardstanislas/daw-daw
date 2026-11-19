// Musical helpers: time formatting, snapping, note names.

export const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export function noteName(pitch: number): string {
  return `${NOTE_NAMES[((pitch % 12) + 12) % 12]}${Math.floor(pitch / 12) - 1}`;
}

export function isBlackKey(pitch: number): boolean {
  return [1, 3, 6, 8, 10].includes(((pitch % 12) + 12) % 12);
}

export function beatsPerBar(num: number, den: number): number {
  return (num * 4) / den;
}

/** "bar.beat.tick" (1-based bar and beat, 960 ticks per beat). */
export function formatBBT(beats: number, num: number, den: number): string {
  const bpb = beatsPerBar(num, den);
  const beatLen = 4 / den;
  const b = Math.max(0, beats);
  const bar = Math.floor(b / bpb);
  const inBar = b - bar * bpb;
  const beat = Math.floor(inBar / beatLen + 1e-9);
  const ticks = Math.floor(((inBar - beat * beatLen) / beatLen) * 960);
  return `${bar + 1}.${beat + 1}.${String(ticks).padStart(3, "0")}`;
}

export function formatTime(beats: number, bpm: number): string {
  const sec = Math.max(0, (beats * 60) / bpm);
  const m = Math.floor(sec / 60);
  const s = sec - m * 60;
  return `${m}:${s.toFixed(3).padStart(6, "0")}`;
}

export function beatsToSeconds(beats: number, bpm: number) {
  return (beats * 60) / bpm;
}

export function secondsToBeats(sec: number, bpm: number) {
  return (sec * bpm) / 60;
}

export type SnapValue = "bar" | "1/2" | "1/4" | "1/8" | "1/16" | "1/32" | "1/8t" | "1/16t" | "off";

export const SNAP_OPTIONS: SnapValue[] = ["bar", "1/2", "1/4", "1/8", "1/16", "1/32", "1/8t", "1/16t", "off"];

/** Grid size in beats. */
export function snapBeats(snap: SnapValue, bpb: number): number {
  switch (snap) {
    case "bar":
      return bpb;
    case "1/2":
      return 2;
    case "1/4":
      return 1;
    case "1/8":
      return 0.5;
    case "1/16":
      return 0.25;
    case "1/32":
      return 0.125;
    case "1/8t":
      return 1 / 3;
    case "1/16t":
      return 1 / 6;
    case "off":
      return 0;
  }
}

export function snapTo(beat: number, grid: number, mode: "round" | "floor" = "round"): number {
  if (!grid) return beat;
  const f = mode === "round" ? Math.round : Math.floor;
  return f(beat / grid + 1e-9) * grid;
}

export function dbToGain(db: number) {
  return db <= -96 ? 0 : Math.pow(10, db / 20);
}

export function gainToDb(g: number) {
  return g <= 0.0000158 ? -96 : 20 * Math.log10(g);
}

/** Fader position (0..1) ↔ dB: cubic gain law, 0 dB at 0.8, +5.8 dB at 1. */
export function faderToDb(x: number): number {
  if (x <= 0.001) return -96;
  return Math.max(-96, 60 * Math.log10(x / 0.8));
}

export function dbToFader(db: number): number {
  if (db <= -96) return 0;
  return clamp(0.8 * Math.pow(10, db / 60), 0, 1);
}

export function formatDb(db: number): string {
  if (db <= -96) return "-∞";
  return `${db > 0 ? "+" : ""}${db.toFixed(1)}`;
}

export function clamp(v: number, lo: number, hi: number) {
  return Math.max(lo, Math.min(hi, v));
}

let idCounter = 0;
export function uid(prefix = "id"): string {
  idCounter = (idCounter + 1) % 1e6;
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}${idCounter.toString(36)}`;
}

/** setPointerCapture that never throws (synthetic events, removed nodes). */
export function capturePointer(el: Element, pointerId: number) {
  try {
    el.setPointerCapture(pointerId);
  } catch {
    /* ignore */
  }
}
