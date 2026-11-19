// Step sequencer view of a MIDI clip: one row per drum piece (or note), one
// column per step. Toggling a cell adds or removes a note in the clip, so the
// piano roll and the step view always show the same data.

import { useEffect, useMemo, useRef, useState } from "react";
import type { Clip, Note, Track } from "../../lib/types";
import { beginGesture, commit, draftClip, endGesture, gesture, setUi } from "../../lib/store";
import { livePosition, liveNote } from "../../lib/engine";
import { noteNamesFor } from "../../lib/catalog";
import { beatsPerBar, noteName, uid } from "../../lib/music";
import { openContextMenu } from "../Menu";
import { useStore } from "../../lib/store";
import { StepInputControls } from "./StepInput";
import { useStepRecord } from "../../lib/stepRecord";

const STEP_SIZES: { label: string; beats: number }[] = [
  { label: "1/8", beats: 0.5 },
  { label: "1/16", beats: 0.25 },
  { label: "1/32", beats: 0.125 },
  { label: "1/8 triplet", beats: 1 / 3 },
  { label: "1/16 triplet", beats: 1 / 6 },
];
const MAX_STEPS = 64;
/** Velocities cycled by Shift+click: normal → accent → soft. */
const VELOCITIES = [100, 127, 60];

function cellNotes(notes: Note[], pitch: number, start: number, step: number): Note[] {
  return notes.filter((n) => n.pitch === pitch && n.start >= start - 1e-6 && n.start < start + step * 0.5);
}

type Op = { pitch: number; step: number; action: "add" | "remove"; velocity?: number };

export function StepSequencer({ clip, track, modeSwitch }: { clip: Clip; track: Track; modeSwitch: React.ReactNode }) {
  const project = useStore((s) => s.project);
  const bpb = beatsPerBar(project.timeSigNum, project.timeSigDen);
  const [stepBeats, setStepBeats] = useState(() => Number(localStorage.getItem("dawdaw:stepSize")) || 0.25);
  const [octave, setOctave] = useState(3);
  useEffect(() => localStorage.setItem("dawdaw:stepSize", String(stepBeats)), [stepBeats]);

  const names = useMemo(() => noteNamesFor(track.instrument?.type), [track.instrument?.type]);
  const isDrum = names.size > 0;
  const patternLen = clip.loopLength > 0 ? clip.loopLength : clip.length;
  const steps = Math.max(1, Math.min(MAX_STEPS, Math.round(patternLen / stepBeats)));

  // Rows: kit pieces (kick first) or one octave of notes, plus any pitch already used.
  const rows = useMemo(() => {
    const used = new Set(clip.notes.filter((n) => n.start < patternLen).map((n) => n.pitch));
    const base = isDrum ? [...names.keys()] : Array.from({ length: 13 }, (_, i) => (octave + 1) * 12 + 12 - i);
    for (const p of used) if (!base.includes(p)) base.push(p);
    return isDrum ? base.sort((a, b) => a - b) : base.sort((a, b) => b - a);
  }, [clip.notes, names, isDrum, octave, patternLen]);

  // ---- editing (a drag paints several cells in one undo step)
  const painting = useRef<{ mode: "add" | "remove"; ops: Op[]; seen: Set<string> } | null>(null);

  const applyOps = (ops: Op[]) =>
    gesture((d) => {
      const c = draftClip(d, clip.id);
      if (!c) return;
      for (const op of ops) {
        const start = op.step * stepBeats;
        const hits = cellNotes(c.notes as Note[], op.pitch, start, stepBeats);
        if (op.action === "remove") {
          const ids = new Set(hits.map((n) => n.id));
          c.notes = c.notes.filter((n) => !ids.has(n.id));
        } else if (hits.length === 0) {
          c.notes.push({ id: uid("n"), pitch: op.pitch, start, length: stepBeats * 0.9, velocity: op.velocity ?? 100 });
        } else if (op.velocity !== undefined) {
          for (const n of c.notes) if (hits.some((h) => h.id === n.id)) n.velocity = op.velocity;
        }
      }
    });

  const onCellDown = (e: React.PointerEvent, pitch: number, step: number) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const hits = cellNotes(clip.notes, pitch, step * stepBeats, stepBeats);
    beginGesture();
    if (e.shiftKey && hits.length) {
      // Accent cycle on an existing step.
      const i = VELOCITIES.indexOf(hits[0].velocity);
      applyOps([{ pitch, step, action: "add", velocity: VELOCITIES[(i + 1) % VELOCITIES.length] }]);
      endGesture();
      return;
    }
    const mode = hits.length ? "remove" : "add";
    painting.current = { mode, ops: [{ pitch, step, action: mode }], seen: new Set([`${pitch}:${step}`]) };
    applyOps(painting.current.ops);
    if (mode === "add") {
      liveNote(pitch, 0.8, true, track.id);
      setTimeout(() => liveNote(pitch, 0, false, track.id), 150);
    }
    const up = () => {
      window.removeEventListener("pointerup", up);
      painting.current = null;
      endGesture();
    };
    window.addEventListener("pointerup", up);
  };

  // Painting follows the pointer over cells (works with mouse, trackpad and touch).
  const onGridMove = (e: React.PointerEvent) => {
    if (!painting.current) return;
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    if (!el || !el.dataset.step) return;
    onCellEnter(Number(el.dataset.pitch), Number(el.dataset.step));
  };

  const onCellEnter = (pitch: number, step: number) => {
    const p = painting.current;
    if (!p) return;
    const key = `${pitch}:${step}`;
    if (p.seen.has(key)) return;
    p.seen.add(key);
    p.ops.push({ pitch, step, action: p.mode });
    applyOps(p.ops);
  };

  const fillRow = (pitch: number, every: number) =>
    commit((d) => {
      const c = draftClip(d, clip.id);
      if (!c) return;
      c.notes = c.notes.filter((n) => !(n.pitch === pitch && n.start < patternLen));
      if (every > 0)
        for (let s = 0; s < steps; s += every)
          c.notes.push({ id: uid("n"), pitch, start: s * stepBeats, length: stepBeats * 0.9, velocity: s % (every * 4) === 0 ? 120 : 100 });
    });

  const setLength = (beats: number) =>
    commit((d) => {
      const c = draftClip(d, clip.id);
      if (!c) return;
      c.loopLength = beats;
      if (c.length < beats) c.length = beats;
    });

  // ---- playhead column
  const headRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const el = headRef.current;
      if (el) {
        const rel = livePosition() - clip.start;
        const inClip = rel >= 0 && rel < clip.length;
        const content = (((rel + clip.offset) % patternLen) + patternLen) % patternLen;
        const col = Math.floor(content / stepBeats + 1e-6);
        el.style.display = inClip && col < steps ? "block" : "none";
        el.style.setProperty("--col", String(col));
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [clip.start, clip.length, clip.offset, patternLen, stepBeats, steps]);

  const stepsPerBeat = Math.max(1, Math.round(1 / stepBeats));
  const stepRec = useStepRecord();
  const cursorStep =
    stepRec.active && stepRec.clipId === clip.id ? Math.floor((stepRec.cursor % patternLen) / stepBeats + 1e-6) : null;
  const lengths = [bpb, bpb * 2, bpb * 4];

  return (
    <div className="steps">
      <div className="editor-toolbar">
        <span className="editor-title" style={{ ["--track-color" as string]: track.color }}>
          <span className="swatch" /> {track.name} › {clip.name}
        </span>
        {modeSwitch}
        <StepInputControls clipId={clip.id} />
        <label className="mini-field">
          Step
          <select value={stepBeats} onChange={(e) => setStepBeats(+e.target.value)}>
            {STEP_SIZES.map((s) => (
              <option key={s.label} value={s.beats}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
        <div className="seg" role="group" aria-label="Pattern length">
          {lengths.map((l) => (
            <button key={l} className={Math.abs(patternLen - l) < 1e-6 ? "on" : ""} onClick={() => setLength(l)}>
              {l / bpb} bar{l / bpb > 1 ? "s" : ""}
            </button>
          ))}
        </div>
        {!isDrum && (
          <div className="seg" role="group" aria-label="Octave">
            <button onClick={() => setOctave((o) => Math.max(0, o - 1))}>−</button>
            <span className="seg-label">C{octave}</span>
            <button onClick={() => setOctave((o) => Math.min(7, o + 1))}>+</button>
          </div>
        )}
        <div className="spacer" />
        <span className="muted-text">
          {steps} steps · Shift-click: accent
        </span>
      </div>
      <div className="steps-body">
        <div className="steps-grid" style={{ ["--steps" as string]: steps }} onPointerMove={onGridMove}>
          {rows.map((pitch) => {
            const label = names.get(pitch) ?? noteName(pitch);
            return (
              <div className="steps-row" key={pitch}>
                <button
                  className="steps-row-head"
                  onPointerDown={() => liveNote(pitch, 0.8, true, track.id)}
                  onPointerUp={() => liveNote(pitch, 0, false, track.id)}
                  onPointerLeave={(e) => e.buttons && liveNote(pitch, 0, false, track.id)}
                  onContextMenu={(e) =>
                    openContextMenu(e, [
                      { label: "Every step", onClick: () => fillRow(pitch, 1) },
                      { label: "Every 2 steps", onClick: () => fillRow(pitch, 2) },
                      { label: `Every beat`, onClick: () => fillRow(pitch, stepsPerBeat) },
                      { label: "Every 2 beats", onClick: () => fillRow(pitch, stepsPerBeat * 2) },
                      { separator: true },
                      { label: "Clear row", onClick: () => fillRow(pitch, 0) },
                    ])
                  }
                  title={`${label} — click to hear, right-click to fill`}
                >
                  {label}
                </button>
                <div className="steps-cells">
                  {Array.from({ length: steps }, (_, s) => {
                    const hit = cellNotes(clip.notes, pitch, s * stepBeats, stepBeats)[0];
                    const beatStart = s % stepsPerBeat === 0;
                    const atCursor = cursorStep === s;
                    return (
                      <div
                        key={s}
                        className={`step ${hit ? "on" : ""} ${beatStart ? "beat" : ""} ${Math.floor(s / stepsPerBeat) % 2 ? "alt" : ""} ${atCursor ? "cursor" : ""} ${hit && hit.velocity >= 120 ? "accent" : ""} ${hit && hit.velocity < 80 ? "soft" : ""}`}
                        data-pitch={pitch}
                        data-step={s}
                        onPointerDown={(e) => onCellDown(e, pitch, s)}
                      />
                    );
                  })}
                </div>
              </div>
            );
          })}
          <div className="steps-head" ref={headRef} />
        </div>
      </div>
      <div className="editor-hint">
        Click or drag to paint steps · Shift-click cycles accent / soft · right-click a row name to fill it · the pattern loops over the clip
      </div>
    </div>
  );
}

/** Piano roll / Steps switch shown in both editor toolbars. */
export function EditorModeSwitch({ mode }: { mode: "piano" | "steps" }) {
  return (
    <div className="seg" role="group" aria-label="Editor view">
      <button className={mode === "piano" ? "on" : ""} onClick={() => setUi({ editorMode: "piano" })}>
        Piano roll
      </button>
      <button className={mode === "steps" ? "on" : ""} onClick={() => setUi({ editorMode: "steps" })}>
        Steps
      </button>
    </div>
  );
}
