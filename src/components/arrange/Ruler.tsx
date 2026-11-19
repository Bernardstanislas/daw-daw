// Timeline ruler: bar numbers, click/drag to seek, upper strip to draw / move the loop.

import { memo } from "react";
import { beginGesture, commit, endGesture, gesture, useStore } from "../../lib/store";
import { seek } from "../../lib/engine";
import { beatsPerBar, capturePointer } from "../../lib/music";
import { beatAtClientX, snapBeat } from "./geom";

export const Ruler = memo(function Ruler({ width, ppb }: { width: number; ppb: number }) {
  const project = useStore((s) => s.project);
  const bpb = beatsPerBar(project.timeSigNum, project.timeSigDen);
  const barPx = bpb * ppb;
  // Label every N bars so labels don't overlap.
  let every = 1;
  while (barPx * every < 34) every *= 2;
  const bars = Math.ceil(width / barPx) + 1;
  const loop = project.loop;

  const marks: React.ReactNode[] = [];
  for (let b = 0; b < bars; b += 1) {
    const x = b * barPx;
    const major = b % every === 0;
    marks.push(
      <div key={b} className={`ruler-mark ${major ? "major" : ""}`} style={{ left: x }}>
        {major && <span>{b + 1}</span>}
      </div>,
    );
  }

  const onLoopStripDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const b0 = beatAtClientX(e.clientX);
    const inLoop = b0 >= loop.start && b0 <= loop.end;
    const nearStart = Math.abs((b0 - loop.start) * ppb) < 6;
    const nearEnd = Math.abs((b0 - loop.end) * ppb) < 6;
    const mode = nearStart ? "start" : nearEnd ? "end" : inLoop && e.detail < 2 ? "move" : "draw";
    beginGesture();
    const anchor = snapBeat(b0, "floor");
    const move = (ev: PointerEvent) => {
      const b = beatAtClientX(ev.clientX);
      gesture((p) => {
        if (mode === "draw") {
          const s = snapBeat(Math.min(anchor, b), "round");
          const en = snapBeat(Math.max(anchor, b), "round");
          if (en - s > 0.01) {
            p.loop.start = Math.max(0, s);
            p.loop.end = en;
            p.loop.enabled = true;
          }
        } else if (mode === "move") {
          const len = loop.end - loop.start;
          const s = Math.max(0, snapBeat(loop.start + (b - b0), "round"));
          p.loop.start = s;
          p.loop.end = s + len;
        } else if (mode === "start") {
          p.loop.start = Math.max(0, Math.min(loop.end - 0.25, snapBeat(b)));
        } else {
          p.loop.end = Math.max(loop.start + 0.25, snapBeat(b));
        }
      });
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      endGesture();
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const onSeekDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    seek(snapBeat(beatAtClientX(e.clientX), "round", e.metaKey || e.ctrlKey));
    const move = (ev: PointerEvent) => seek(snapBeat(beatAtClientX(ev.clientX), "round", ev.metaKey || ev.ctrlKey));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  return (
    <div className="ruler" style={{ width }}>
      <div className="ruler-loop-strip" onPointerDown={onLoopStripDown} title="Drag to set the loop region · double-click: toggle loop" onDoubleClick={() => commit((p) => void (p.loop.enabled = !p.loop.enabled))}>
        <div
          className={`loop-region ${loop.enabled ? "on" : ""}`}
          style={{ left: loop.start * ppb, width: Math.max(2, (loop.end - loop.start) * ppb) }}
        />
      </div>
      <div className="ruler-marks" onPointerDown={onSeekDown} title="Click or drag to move the playhead">
        {marks}
      </div>
    </div>
  );
});
