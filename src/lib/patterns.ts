// Built-in MIDI loops ("patterns") offered in the browser. All original.

import type { Note } from "./types";
import { uid } from "./music";

export interface Pattern {
  id: string;
  name: string;
  category: "Drums" | "Bass" | "Chords" | "Melody";
  /** Length in beats. */
  length: number;
  instrument: { type: string; preset?: string };
  notes: Omit<Note, "id">[];
}

const KICK = 36,
  RIM = 37,
  SNARE = 38,
  CLAP = 39,
  CH = 42,
  OH = 46,
  LTOM = 41,
  MTOM = 45,
  HTOM = 48,
  CRASH = 49,
  SHAKER = 54;

/** Builds notes from a step string: 'x' = accent, 'o' = normal, '.' = rest. */
function steps(pitch: number, pattern: string, stepLen = 0.25, swing = 0): Omit<Note, "id">[] {
  const out: Omit<Note, "id">[] = [];
  const s = pattern.replace(/\s+/g, "");
  for (let i = 0; i < s.length; i++) {
    const ch = s[i];
    if (ch === ".") continue;
    const sw = i % 2 === 1 ? swing * stepLen : 0;
    out.push({ pitch, start: i * stepLen + sw, length: stepLen * 0.9, velocity: ch === "x" ? 112 : ch === "o" ? 84 : 60 });
  }
  return out;
}

/** Chord helper: notes (MIDI) held from `start` for `len` beats. */
function chord(pitches: number[], start: number, len: number, vel = 90): Omit<Note, "id">[] {
  return pitches.map((pitch) => ({ pitch, start, length: len, velocity: vel }));
}

function seq(pitches: (number | null)[], stepLen: number, len = stepLen * 0.9, vel = 96): Omit<Note, "id">[] {
  const out: Omit<Note, "id">[] = [];
  pitches.forEach((p, i) => {
    if (p !== null) out.push({ pitch: p, start: i * stepLen, length: len, velocity: vel });
  });
  return out;
}

const kit = { type: "kit" };

export const PATTERNS: Pattern[] = [
  // ------------------------------------------------------------ drums
  {
    id: "drum-house",
    name: "Floor Stomp",
    category: "Drums",
    length: 4,
    instrument: kit,
    notes: [
      ...steps(KICK, "x...x...x...x..."),
      ...steps(CLAP, "....x.......x..."),
      ...steps(CH, "..o...o...o...o."),
      ...steps(OH, "..............o."),
      ...steps(SHAKER, ".o.o.o.o.o.o.o.o"),
    ],
  },
  {
    id: "drum-boombap",
    name: "Dusty Head Nod",
    category: "Drums",
    length: 8,
    instrument: { type: "kit", preset: "Dusty Crate" },
    notes: [
      ...steps(KICK, "x.....o...x.....x.....o..o......", 0.25),
      ...steps(SNARE, "....x.......x.......x.......x...", 0.25),
      ...steps(CH, "o.o.o.o.o.o.o.o.o.o.o.o.o.o.o.o.", 0.25, 0.12),
    ],
  },
  {
    id: "drum-halftime",
    name: "Halftime Rattle",
    category: "Drums",
    length: 8,
    instrument: kit,
    notes: [
      ...steps(KICK, "x.......o.x.....x.....x...o.....", 0.25),
      ...steps(SNARE, "........x...............x.......", 0.25),
      ...steps(CH, "o.o.o.o.o.o.o.o.o.o.o.oooo.o.o.o", 0.25),
    ],
  },
  {
    id: "drum-rock",
    name: "Garage Backbeat",
    category: "Drums",
    length: 4,
    instrument: { type: "samplekit", preset: "TightKit" },
    notes: [
      ...steps(KICK, "x.....x.x......."),
      ...steps(SNARE, "....x.......x..."),
      ...steps(CH, "o.o.o.o.o.o.o.o."),
    ],
  },
  {
    id: "drum-break",
    name: "Broken Pavement",
    category: "Drums",
    length: 4,
    instrument: { type: "kit", preset: "Punch Card" },
    notes: [
      ...steps(KICK, "x.x.......x..o.."),
      ...steps(SNARE, "....x..o.o..x..o"),
      ...steps(CH, "oooooooooooooooo"),
    ],
  },
  {
    id: "drum-disco",
    name: "Mirrorball",
    category: "Drums",
    length: 4,
    instrument: kit,
    notes: [
      ...steps(KICK, "x...x...x...x..."),
      ...steps(SNARE, "....x.......x..."),
      ...steps(OH, "..o...o...o...o."),
      ...steps(CH, "o...o...o...o..."),
    ],
  },
  {
    id: "drum-latin",
    name: "Island Skip",
    category: "Drums",
    length: 4,
    instrument: kit,
    notes: [
      ...steps(KICK, "x...x...x...x..."),
      ...steps(RIM, "...o..o....o..o."),
      ...steps(CH, "o.o.o.o.o.o.o.o."),
    ],
  },
  {
    id: "drum-fill",
    name: "Tom Tumble (fill)",
    category: "Drums",
    length: 4,
    instrument: kit,
    notes: [
      ...steps(SNARE, "o.o.x..........."),
      ...steps(HTOM, "....x.o........."),
      ...steps(MTOM, "........x.o....."),
      ...steps(LTOM, "............x.o."),
      ...steps(KICK, "x..............."),
      { pitch: CRASH, start: 3.75, length: 0.25, velocity: 100 },
    ],
  },

  // ------------------------------------------------------------ bass
  {
    id: "bass-eighths",
    name: "Pumping Eighths",
    category: "Bass",
    length: 16,
    instrument: { type: "pulse", preset: "Lead Weight Bass" },
    notes: [
      ...seq(Array(8).fill(36), 0.5, 0.4),
      ...seq(Array(8).fill(43), 0.5, 0.4).map((n) => ({ ...n, start: n.start + 4 })),
      ...seq(Array(8).fill(45), 0.5, 0.4).map((n) => ({ ...n, start: n.start + 8 })),
      ...seq(Array(8).fill(41), 0.5, 0.4).map((n) => ({ ...n, start: n.start + 12 })),
    ],
  },
  {
    id: "bass-octave",
    name: "Octave Hopper",
    category: "Bass",
    length: 4,
    instrument: { type: "pulse", preset: "Lead Weight Bass" },
    notes: seq([33, 45, 33, 45, 33, 45, 33, 45], 0.5, 0.3),
  },
  {
    id: "bass-funk",
    name: "Rubber Band",
    category: "Bass",
    length: 4,
    instrument: { type: "pulse", preset: "Squelch Machine" },
    notes: seq([36, null, null, 36, null, 39, 41, null, 36, null, 48, null, 46, null, 43, 39], 0.25, 0.2, 100),
  },
  {
    id: "bass-sub",
    name: "Deep Sub Glide",
    category: "Bass",
    length: 8,
    instrument: { type: "pulse", preset: "Basement Sub" },
    notes: [
      { pitch: 33, start: 0, length: 2.75, velocity: 100 },
      { pitch: 33, start: 3, length: 0.5, velocity: 90 },
      { pitch: 29, start: 4, length: 2.75, velocity: 100 },
      { pitch: 31, start: 7, length: 1, velocity: 90 },
    ],
  },

  // ------------------------------------------------------------ chords
  {
    id: "chords-pop",
    name: "Sunny Four (I–V–vi–IV)",
    category: "Chords",
    length: 16,
    instrument: { type: "sampler", preset: "Grand Piano" },
    notes: [
      ...chord([48, 60, 64, 67], 0, 3.9),
      ...chord([43, 59, 62, 67], 4, 3.9),
      ...chord([45, 60, 64, 69], 8, 3.9),
      ...chord([41, 60, 65, 69], 12, 3.9),
    ],
  },
  {
    id: "chords-minor",
    name: "Night Drive (i–VI–III–VII)",
    category: "Chords",
    length: 16,
    instrument: { type: "pulse", preset: "Amber Pad" },
    notes: [
      ...chord([57, 60, 64], 0, 3.95, 80),
      ...chord([53, 57, 60], 4, 3.95, 80),
      ...chord([55, 60, 64], 8, 3.95, 80),
      ...chord([55, 59, 62], 12, 3.95, 80),
    ],
  },
  {
    id: "chords-jazz",
    name: "Smoky Turnaround (ii–V–I)",
    category: "Chords",
    length: 16,
    instrument: { type: "keys" },
    notes: [
      ...chord([50, 60, 65, 69], 0, 3.8, 80),
      ...chord([43, 59, 65, 69], 4, 3.8, 80),
      ...chord([48, 59, 64, 67], 8, 7.8, 76),
    ],
  },
  {
    id: "chords-stabs",
    name: "Offbeat Stabs",
    category: "Chords",
    length: 4,
    instrument: { type: "pulse", preset: "Bold Fanfare" },
    notes: [0.5, 1.5, 2.5, 3.5].flatMap((t) => chord([57, 60, 64], t, 0.3, 100)),
  },

  // ------------------------------------------------------------ melody
  {
    id: "mel-arp",
    name: "Rising Arp",
    category: "Melody",
    length: 4,
    instrument: { type: "pulse", preset: "Glass Pluck" },
    notes: seq([57, 60, 64, 69, 72, 69, 64, 60, 57, 60, 64, 69, 72, 76, 72, 69], 0.25, 0.2, 90),
  },
  {
    id: "mel-hook",
    name: "Little Hook",
    category: "Melody",
    length: 8,
    instrument: { type: "pulse", preset: "Laser Lemon" },
    notes: [
      { pitch: 72, start: 0, length: 0.5, velocity: 100 },
      { pitch: 74, start: 0.5, length: 0.5, velocity: 90 },
      { pitch: 76, start: 1, length: 1, velocity: 100 },
      { pitch: 74, start: 2.5, length: 0.5, velocity: 90 },
      { pitch: 72, start: 3, length: 1, velocity: 96 },
      { pitch: 69, start: 4, length: 0.5, velocity: 96 },
      { pitch: 72, start: 4.5, length: 0.5, velocity: 90 },
      { pitch: 67, start: 5, length: 2.5, velocity: 100 },
    ],
  },
];

export function patternNotes(p: Pattern): Note[] {
  return p.notes.map((n) => ({ ...n, id: uid("n") }));
}

export function findPattern(id: string) {
  return PATTERNS.find((p) => p.id === id);
}
