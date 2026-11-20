// Piano roll for MIDI clips: draw / select / move / resize notes, velocity lane,
// drum-row folding, quantize & transpose.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Clip, Note, Track } from "../../lib/types";
import { beginGesture, cancelGesture, commit, draftClip, endGesture, gesture, select, setUi, useStore } from "../../lib/store";
import { livePosition, liveNote, useTransport } from "../../lib/engine";
import { clamp, isBlackKey, noteName, snapBeats, SNAP_OPTIONS, type SnapValue, uid, beatsPerBar, capturePointer } from "../../lib/music";
import { noteNamesFor } from "../../lib/catalog";
import { quantizeNotes, transposeNotes } from "../../lib/actions";
import { IconPencil, IconPointer } from "../icons";
import { StepInputControls } from "./StepInput";
import { auditionProps } from "./audition";
import { HFader } from "../controls";
import { useStepRecord } from "../../lib/stepRecord";

const KEYS_W = 72;
const VEL_H = 72;

type Tool = "draw" | "select";

let previewTrack: string | null = null;
function previewNote(pitch: number, vel = 100) {
  const t = previewTrack;
  liveNote(pitch, vel / 127, true, t);
  setTimeout(() => liveNote(pitch, 0, false, t), 180);
}

export function PianoRoll({ clip, track, modeSwitch }: { clip: Clip; track: Track; modeSwitch?: React.ReactNode }) {
  const ui = useStore((s) => s.ui);
  const selectedNotes = useStore((s) => s.selection.noteIds);
  const project = useStore((s) => s.project);
  const eppb = ui.editorPxPerBeat;
  previewTrack = track.id;
  const rowH = ui.editorRowHeight;
  const [tool, setTool] = useState<Tool>(() => (localStorage.getItem("dawdaw:prTool") as Tool) || "draw");
  const [gridName, setGridName] = useState<SnapValue>(() => (localStorage.getItem("dawdaw:prGrid") as SnapValue) || "1/16");
  const names = useMemo(() => noteNamesFor(track.instrument?.type), [track.instrument?.type]);
  const isDrum = names.size > 0;
  const [fold, setFold] = useState(isDrum);
  useEffect(() => setFold(isDrum), [isDrum, clip.id]);
  const [marquee, setMarquee] = useState<null | { x0: number; y0: number; x1: number; y1: number }>(null);
  const lastLen = useRef(0.25);
  const scrollRef = useRef<HTMLDivElement>(null);
  const velRef = useRef<HTMLDivElement>(null);
  const bpb = beatsPerBar(project.timeSigNum, project.timeSigDen);

  useEffect(() => localStorage.setItem("dawdaw:prTool", tool), [tool]);
  useEffect(() => localStorage.setItem("dawdaw:prGrid", gridName), [gridName]);

  const grid = snapBeats(gridName, bpb);
  const snap = (b: number, mode: "round" | "floor" = "round", bypass = false) => {
    if (!grid || bypass) return b;
    return (mode === "round" ? Math.round : Math.floor)(b / grid + 1e-9) * grid;
  };

  // Rows (top to bottom)
  const rows = useMemo(() => {
    if (fold) {
      const set = new Set<number>([...names.keys(), ...clip.notes.map((n) => n.pitch)]);
      if (!isDrum) for (const n of clip.notes) set.add(n.pitch);
      const arr = [...set].sort((a, b) => b - a);
      return arr.length ? arr : [60];
    }
    return Array.from({ length: 128 }, (_, i) => 127 - i);
  }, [fold, names, clip.notes, isDrum]);
  const rowOf = useMemo(() => new Map(rows.map((p, i) => [p, i])), [rows]);

  const loopLen = clip.loopLength > 0 ? clip.loopLength : 0;
  const editLen = loopLen || clip.length;
  const width = Math.max((clip.length + bpb * 2) * eppb, 400);
  const height = rows.length * rowH;

  // Initial vertical scroll: center on the notes (or C4).
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const pitches = clip.notes.map((n) => n.pitch);
    const center = pitches.length ? (Math.max(...pitches) + Math.min(...pitches)) / 2 : 60;
    const r = rowOf.get(Math.round(center)) ?? rows.findIndex((p) => p <= center);
    el.scrollTop = Math.max(0, r * rowH - el.clientHeight / 2);
    el.scrollLeft = 0;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clip.id, fold]);

  // Sync velocity lane horizontal scroll
  const onScroll = () => {
    if (velRef.current && scrollRef.current) velRef.current.scrollLeft = scrollRef.current.scrollLeft;
  };

  // Playhead
  const phRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const x = (livePosition() - clip.start) * eppb;
      if (phRef.current) {
        phRef.current.style.transform = `translateX(${x}px)`;
        phRef.current.style.display = x >= 0 && x <= width ? "block" : "none";
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [clip.start, eppb, width]);

  // Ctrl/Cmd + wheel zoom
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        setUi({ editorPxPerBeat: clamp(useStore.getState().ui.editorPxPerBeat * Math.pow(1.0018, -e.deltaY), 10, 600) });
      } else if (e.altKey) {
        e.preventDefault();
        setUi({ editorRowHeight: clamp(useStore.getState().ui.editorRowHeight + (e.deltaY < 0 ? 1 : -1), 8, 28) });
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // ---- coordinates
  const toContent = (clientX: number, clientY: number) => {
    const el = scrollRef.current!;
    const r = el.getBoundingClientRect();
    const x = clientX - r.left + el.scrollLeft;
    const y = clientY - r.top + el.scrollTop;
    const beat = x / eppb + clip.offset; // content beat
    const row = Math.floor(y / rowH);
    return { x, y, beat, row, pitch: rows[clamp(row, 0, rows.length - 1)] };
  };

  const noteX = (n: Note) => (n.start - clip.offset) * eppb;

  const selSet = new Set(selectedNotes);
  // Notes currently held (MIDI keyboard, musical typing, clicks on the keys).
  const liveNotes = useTransport((s) => s.liveNotes);
  const stepRec = useStepRecord();
  const stepCursor = stepRec.active && stepRec.clipId === clip.id ? stepRec.cursor : null;
  const stepLen = stepRec.step;

  // ---- note interactions
  const startNoteDrag = (e: React.PointerEvent, note: Note, forcedMode?: "resize") => {
    e.stopPropagation();
    if (e.button === 2) return;
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const mode = forcedMode ?? (e.clientX > rect.right - Math.min(7, rect.width / 3) ? "resize" : "move");
    let ids = selectedNotes;
    if (e.shiftKey) {
      ids = ids.includes(note.id) ? ids.filter((x) => x !== note.id) : [...ids, note.id];
      select({ noteIds: ids });
      if (!ids.includes(note.id)) return;
    } else if (!ids.includes(note.id)) {
      ids = [note.id];
      select({ noteIds: ids });
    }
    const copy = e.altKey && mode === "move";
    const origin = clip.notes.filter((n) => ids.includes(n.id));
    const newIds = origin.map(() => uid("n"));
    const p0 = toContent(e.clientX, e.clientY);
    const row0 = rowOf.get(note.pitch) ?? 0;
    let started = false;
    let lastPitch = note.pitch;
    if (mode === "move") previewNote(note.pitch, note.velocity);
    capturePointer(el, e.pointerId);
    const move = (ev: PointerEvent) => {
      const p = toContent(ev.clientX, ev.clientY);
      if (!started) {
        if (Math.abs(p.x - p0.x) < 3 && Math.abs(p.y - p0.y) < 3) return;
        started = true;
        beginGesture();
      }
      const bypass = ev.metaKey || ev.ctrlKey;
      if (mode === "move") {
        // Clamp the shared delta so a group keeps its shape at beat 0.
        const minStart = Math.min(...origin.map((o) => o.start));
        const dBeat = Math.max(-minStart, snap(note.start + (p.beat - p0.beat), "round", bypass) - note.start);
        const dRow = clamp(p.row, 0, rows.length - 1) - row0;
        const newPitch = rows[clamp(row0 + dRow, 0, rows.length - 1)];
        if (newPitch !== lastPitch) {
          lastPitch = newPitch;
          previewNote(newPitch, note.velocity);
        }
        gesture((d) => {
          const c = draftClip(d, clip.id);
          if (!c) return;
          origin.forEach((o, i) => {
            const r = rowOf.get(o.pitch) ?? 0;
            const pitch = rows[clamp(r + dRow, 0, rows.length - 1)];
            const start = o.start + dBeat;
            if (copy) c.notes.push({ ...o, id: newIds[i], start, pitch });
            else {
              const n = c.notes.find((x) => x.id === o.id);
              if (n) {
                n.start = start;
                n.pitch = pitch;
              }
            }
          });
        });
      } else {
        const dLen = p.beat - p0.beat;
        gesture((d) => {
          const c = draftClip(d, clip.id);
          if (!c) return;
          for (const o of origin) {
            const n = c.notes.find((x) => x.id === o.id);
            if (!n) continue;
            const end = snap(o.start + o.length + dLen, "round", bypass);
            n.length = Math.max(grid && !bypass ? grid : 0.03, end - o.start);
            lastLen.current = n.length;
          }
        });
      }
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", cancel);
      if (started) {
        endGesture();
        if (copy) select({ noteIds: newIds });
        else if (mode === "resize") lastLen.current = useStore.getState().project.tracks.flatMap((t) => t.clips).find((c) => c.id === clip.id)?.notes.find((n) => n.id === note.id)?.length ?? lastLen.current;
      } else if (mode === "move") {
        lastLen.current = note.length;
      }
    };
    const cancel = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", cancel);
      if (started) cancelGesture();
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", cancel);
  };

  const deleteNotes = (ids: string[]) => {
    const set = new Set(ids);
    commit((d) => {
      const c = draftClip(d, clip.id);
      if (c) c.notes = c.notes.filter((n) => !set.has(n.id));
    });
    select({ noteIds: [] });
  };

  const onGridDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const el = e.currentTarget as HTMLElement;
    const p0 = toContent(e.clientX, e.clientY);
    capturePointer(el, e.pointerId);
    if (tool === "select" || e.shiftKey) {
      const base = e.shiftKey ? selectedNotes : [];
      if (!e.shiftKey) select({ noteIds: [] });
      const move = (ev: PointerEvent) => {
        const p = toContent(ev.clientX, ev.clientY);
        setMarquee({ x0: p0.x, y0: p0.y, x1: p.x, y1: p.y });
        const [b0, b1] = [Math.min(p0.beat, p.beat), Math.max(p0.beat, p.beat)];
        const [r0, r1] = [Math.min(p0.row, p.row), Math.max(p0.row, p.row)];
        const ids = clip.notes
          .filter((n) => {
            const r = rowOf.get(n.pitch);
            return r !== undefined && r >= r0 && r <= r1 && n.start < b1 && n.start + n.length > b0;
          })
          .map((n) => n.id);
        select({ noteIds: [...new Set([...base, ...ids])] });
      };
      const up = () => {
        el.removeEventListener("pointermove", move);
        el.removeEventListener("pointerup", up);
        setMarquee(null);
      };
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", up);
      return;
    }
    // Draw a note, drag to set its length.
    const start = Math.max(Math.min(0, clip.offset), snap(p0.beat, "floor", e.metaKey || e.ctrlKey));
    const id = uid("n");
    const len = lastLen.current || grid || 0.25;
    const vel = 100;
    beginGesture();
    gesture((d) => {
      draftClip(d, clip.id)?.notes.push({ id, pitch: p0.pitch, start, length: len, velocity: vel });
    });
    select({ noteIds: [id] });
    previewNote(p0.pitch, vel);
    const move = (ev: PointerEvent) => {
      const p = toContent(ev.clientX, ev.clientY);
      const end = snap(p.beat, "round", ev.metaKey || ev.ctrlKey);
      const l = end - start > 0.01 ? Math.max(grid || 0.03, end - start) : len;
      gesture((d) => {
        draftClip(d, clip.id)?.notes.push({ id, pitch: p0.pitch, start, length: l, velocity: vel });
      });
      lastLen.current = l;
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      endGesture();
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  // ---- keyboard shortcuts while the editor has focus
  const onKeyDown = (e: React.KeyboardEvent) => {
    const ids = selectedNotes;
    const meta = e.metaKey || e.ctrlKey;
    if (e.key === "Delete" || e.key === "Backspace") {
      // Never fall through to the arrangement (would delete the whole clip).
      e.preventDefault();
      e.stopPropagation();
      if (ids.length) deleteNotes(ids);
    } else if (meta && e.key.toLowerCase() === "a") {
      e.preventDefault();
      e.stopPropagation();
      select({ noteIds: clip.notes.map((n) => n.id) });
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && ids.length) {
      e.preventDefault();
      e.stopPropagation();
      transposeNotes(clip.id, (e.key === "ArrowUp" ? 1 : -1) * (e.shiftKey ? 12 : 1), ids);
    } else if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && ids.length) {
      e.preventDefault();
      e.stopPropagation();
      const d = (e.key === "ArrowRight" ? 1 : -1) * (grid || 0.25);
      const set = new Set(ids);
      commit((p) => {
        const c = draftClip(p, clip.id);
        if (c) for (const n of c.notes) if (set.has(n.id)) n.start = Math.max(0, n.start + d);
      });
    } else if (meta && e.key.toLowerCase() === "d" && ids.length) {
      e.preventDefault();
      e.stopPropagation();
      const set = new Set(ids);
      const src = clip.notes.filter((n) => set.has(n.id));
      const span = Math.max(...src.map((n) => n.start + n.length)) - Math.min(...src.map((n) => n.start));
      const shift = grid ? Math.ceil(span / grid - 1e-9) * grid : span;
      const copies = src.map((n) => ({ ...n, id: uid("n"), start: n.start + shift }));
      commit((p) => {
        draftClip(p, clip.id)?.notes.push(...copies);
      });
      select({ noteIds: copies.map((n) => n.id) });
    } else if (e.key.toLowerCase() === "q" && !meta) {
      e.preventDefault();
      e.stopPropagation();
      quantizeNotes(clip.id, grid || 0.25, ids);
    }
  };

  // ---- render helpers
  const visibleNotes = clip.notes;
  const ghostOffsets: number[] = [];
  if (loopLen > 0) for (let k = loopLen; k - clip.offset < clip.length; k += loopLen) ghostOffsets.push(k);

  const velocityDrag = (e: React.PointerEvent, note: Note) => {
    e.stopPropagation();
    const el = velRef.current!;
    const ids = selSet.has(note.id) ? selectedNotes : [note.id];
    const r = el.getBoundingClientRect();
    beginGesture();
    const apply = (clientY: number) => {
      const v = clamp(Math.round((1 - (clientY - r.top - 4) / (VEL_H - 8)) * 127), 1, 127);
      const delta = v - note.velocity;
      gesture((d) => {
        const c = draftClip(d, clip.id);
        if (!c) return;
        for (const n of c.notes) {
          if (!ids.includes(n.id)) continue;
          const o = clip.notes.find((x) => x.id === n.id)!;
          n.velocity = n.id === note.id ? v : clamp(o.velocity + delta, 1, 127);
        }
      });
    };
    apply(e.clientY);
    capturePointer(e.currentTarget as HTMLElement, e.pointerId);
    const target = e.currentTarget as HTMLElement;
    const move = (ev: PointerEvent) => apply(ev.clientY);
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      endGesture();
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  };

  const gridBg = {
    backgroundImage: `linear-gradient(to right, var(--grid-bar) 1px, transparent 1px), linear-gradient(to right, var(--grid-beat) 1px, transparent 1px)${grid && grid * eppb >= 6 ? ", linear-gradient(to right, var(--grid-sub) 1px, transparent 1px)" : ""}`,
    backgroundSize: `${bpb * eppb}px 100%, ${eppb}px 100%${grid && grid * eppb >= 6 ? `, ${grid * eppb}px 100%` : ""}`,
    backgroundPosition: `${-clip.offset * eppb}px 0`,
  };

  return (
    <div className="piano-roll" tabIndex={0} onKeyDown={onKeyDown}>
      <div className="editor-toolbar">
        <span className="editor-title" style={{ ["--track-color" as string]: track.color }}>
          <span className="swatch" /> {track.name} › {clip.name}
        </span>
        {modeSwitch}
        <StepInputControls clipId={clip.id} />
        <div className="btn-group">
          <button className={`icon-btn ${tool === "select" ? "on" : ""}`} onClick={() => setTool("select")} title="Select tool (drag = marquee)">
            <span className={`lamp ${tool === "select" ? "lit" : ""}`} />
            <IconPointer />
          </button>
          <button className={`icon-btn ${tool === "draw" ? "on" : ""}`} onClick={() => setTool("draw")} title="Draw tool (click = add note, ⇧-drag = marquee)">
            <span className={`lamp ${tool === "draw" ? "lit" : ""}`} />
            <IconPencil />
          </button>
        </div>
        <label className="mini-field" title="Editor grid">
          Grid
          <select value={gridName} onChange={(e) => setGridName(e.target.value as SnapValue)}>
            {SNAP_OPTIONS.map((s) => (
              <option key={s} value={s}>
                {s === "off" ? "Off" : s === "bar" ? "Bar" : s}
              </option>
            ))}
          </select>
        </label>
        <button className="btn small" onClick={() => quantizeNotes(clip.id, grid || 0.25, selectedNotes)} title="Quantize selected notes (all if none) to the grid (Q)">
          Quantize
        </button>
        <div className="btn-group">
          <button className="btn small" onClick={() => transposeNotes(clip.id, -12, selectedNotes)} title="Octave down">
            −8va
          </button>
          <button className="btn small" onClick={() => transposeNotes(clip.id, -1, selectedNotes)} title="Semitone down (↓)">
            −1
          </button>
          <button className="btn small" onClick={() => transposeNotes(clip.id, 1, selectedNotes)} title="Semitone up (↑)">
            +1
          </button>
          <button className="btn small" onClick={() => transposeNotes(clip.id, 12, selectedNotes)} title="Octave up">
            +8va
          </button>
        </div>
        <button className={`toggle-btn small ${fold ? "on" : ""}`} onClick={() => setFold(!fold)} title="Show only used / named rows">
          Fold
        </button>
        <div className="spacer" />
        <span className="muted-text">
          {selectedNotes.length ? `${selectedNotes.length} selected · ` : ""}
          {clip.notes.length} notes{loopLen ? ` · loop ${loopLen} beats` : ""}
        </span>
        <HFader value={eppb} min={10} max={400} log width={110} label="zoom" onChange={(v) => setUi({ editorPxPerBeat: v })} title="Zoom (⌘ + wheel) · ⌥ + wheel: row height" />
      </div>
      <div className="pr-body">
        <div className="pr-main">
          <div className="pr-keys-scroll" style={{ width: KEYS_W }}>
            <div className="pr-keys" style={{ height, transform: `translateY(0)` }} id="pr-keys-inner">
              {rows.map((p) => {
                const label = names.get(p);
                const black = !fold && isBlackKey(p);
                return (
                  <div
                    key={p}
                    className={`pr-key ${black ? "black" : "white"} ${p % 12 === 0 ? "c" : ""} ${label ? "named" : ""} ${liveNotes.has(p) ? "live" : ""}`}
                    style={{ height: rowH }}
                    {...auditionProps(p, track.id)}
                    title={`${noteName(p)}${label ? " — " + label : ""}`}
                  >
                    {label ? <span>{label}</span> : p % 12 === 0 || fold ? <span>{noteName(p)}</span> : null}
                  </div>
                );
              })}
            </div>
          </div>
          <div
            className="pr-scroll"
            ref={scrollRef}
            onScroll={(e) => {
              onScroll();
              const keys = document.getElementById("pr-keys-inner");
              if (keys) keys.style.transform = `translateY(${-(e.currentTarget as HTMLElement).scrollTop}px)`;
            }}
          >
            <div className="pr-grid" style={{ width, height, ...gridBg }} onPointerDown={onGridDown}>
              {rows.map((p, i) =>
                !fold && isBlackKey(p) ? <div key={p} className="pr-row-black" style={{ top: i * rowH, height: rowH }} /> : fold && i % 2 ? <div key={p} className="pr-row-black" style={{ top: i * rowH, height: rowH }} /> : null,
              )}
              {rows.map((p, i) => (liveNotes.has(p) ? <div key={`live-${p}`} className="pr-row-live" style={{ top: i * rowH, height: rowH }} /> : null))}
              <div className="pr-outside" style={{ left: clip.length * eppb, width: Math.max(0, width - clip.length * eppb) }} />
              {loopLen > 0 && <div className="pr-loop-end" style={{ left: (loopLen - clip.offset) * eppb }} title="Loop end" />}
              {ghostOffsets.map((g) =>
                visibleNotes
                  .filter((n) => n.start < loopLen)
                  .map((n) => {
                    const r = rowOf.get(n.pitch);
                    if (r === undefined) return null;
                    const x = noteX(n) + g * eppb;
                    if (x >= clip.length * eppb) return null;
                    return <div key={`${g}-${n.id}`} className="pr-note ghost" style={{ left: x, top: r * rowH, width: Math.max(3, n.length * eppb - 1), height: rowH - 1 }} />;
                  }),
              )}
              {visibleNotes.map((n) => {
                const r = rowOf.get(n.pitch);
                if (r === undefined) return null;
                const outside = loopLen > 0 && n.start >= loopLen;
                return (
                  <div
                    key={n.id}
                    className={`pr-note ${selSet.has(n.id) ? "selected" : ""} ${outside ? "outside" : ""}`}
                    style={{
                      left: noteX(n),
                      top: r * rowH,
                      width: Math.max(4, n.length * eppb - 1),
                      height: rowH - 1,
                      ["--vel" as string]: n.velocity / 127,
                      ["--note-color" as string]: track.color,
                    }}
                    onPointerDown={(e) => startNoteDrag(e, n)}
                    onDoubleClick={(e) => {
                      e.stopPropagation();
                      deleteNotes([n.id]);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      deleteNotes(selSet.has(n.id) ? selectedNotes : [n.id]);
                    }}
                    title={`${names.get(n.pitch) ?? noteName(n.pitch)} · vel ${n.velocity}`}
                  >
                    {rowH >= 12 && n.length * eppb > 26 && <span>{names.get(n.pitch) ?? noteName(n.pitch)}</span>}
                    <div className="pr-note-resize" onPointerDown={(e) => startNoteDrag(e, n, "resize")} />
                  </div>
                );
              })}
              {marquee && (
                <div
                  className="marquee"
                  style={{
                    left: Math.min(marquee.x0, marquee.x1),
                    top: Math.min(marquee.y0, marquee.y1),
                    width: Math.abs(marquee.x1 - marquee.x0),
                    height: Math.abs(marquee.y1 - marquee.y0),
                  }}
                />
              )}
              <div className="pr-playhead" ref={phRef} style={{ height }} />
              {stepCursor !== null && (
                <div className="step-cursor" style={{ left: (stepCursor - clip.offset) * eppb, width: Math.max(2, stepLen * eppb), height }} />
              )}
            </div>
          </div>
        </div>
        <div className="pr-vel">
          <div className="pr-vel-label" style={{ width: KEYS_W }}>
            Velocity
          </div>
          <div className="pr-vel-scroll" ref={velRef}>
            <div className="pr-vel-inner" style={{ width, height: VEL_H, backgroundImage: "linear-gradient(to right, var(--grid-bar) 1px, transparent 1px)", backgroundSize: `${bpb * eppb}px 100%`, backgroundPosition: `${-clip.offset * eppb}px 0` }}>
              {visibleNotes.map((n) => (
                <div
                  key={n.id}
                  className={`pr-vel-bar ${selSet.has(n.id) ? "selected" : ""}`}
                  style={{ left: noteX(n), height: `${(n.velocity / 127) * (VEL_H - 8)}px`, ["--note-color" as string]: track.color }}
                  onPointerDown={(e) => velocityDrag(e, n)}
                  title={`Velocity ${n.velocity}`}
                />
              ))}
            </div>
          </div>
        </div>
      </div>
      <div className="editor-hint">
        {tool === "draw" ? "Click: add note · drag: length" : "Drag: select"} · drag note: move (⌥ copy) · edge: resize · double/right-click: delete · ↑↓ transpose · Q quantize · ⌘D duplicate · ⌘ bypasses grid · edit range: {editLen} beats
      </div>
    </div>
  );
}
