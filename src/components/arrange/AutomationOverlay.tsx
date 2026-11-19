// Automation envelope drawn over a track lane (volume or pan).
// Click: add a point · drag a point: move · double/right-click a point: delete.

import { memo, useState } from "react";
import type { AutoParam, AutoPoint, Track } from "../../lib/types";
import { beginGesture, commit, draftTrack, endGesture, gesture } from "../../lib/store";
import { capturePointer, clamp, formatDb } from "../../lib/music";
import { beatAtClientX, snapBeat } from "./geom";

const RANGE: Record<AutoParam, [number, number]> = { volume: [-60, 6], pan: [-1, 1] };
const PAD = 6;

function toY(param: AutoParam, v: number, h: number) {
  const [lo, hi] = RANGE[param];
  return PAD + (1 - (v - lo) / (hi - lo)) * (h - PAD * 2);
}
function fromY(param: AutoParam, y: number, h: number) {
  const [lo, hi] = RANGE[param];
  const v = lo + (1 - (y - PAD) / (h - PAD * 2)) * (hi - lo);
  return clamp(param === "volume" ? Math.round(v * 10) / 10 : Math.round(v * 100) / 100, lo, hi);
}
export function formatAuto(param: AutoParam, v: number) {
  if (param === "volume") return `${formatDb(v)} dB`;
  return v === 0 ? "C" : v < 0 ? `L${Math.round(-v * 100)}` : `R${Math.round(v * 100)}`;
}

function sorted(points: AutoPoint[]) {
  return [...points].sort((a, b) => a.beat - b.beat);
}

export const AutomationOverlay = memo(function AutomationOverlay({ track, param, width, ppb }: { track: Track; param: AutoParam; width: number; ppb: number }) {
  const h = track.height - 1;
  const points = sorted(track.automation?.[param] ?? []);
  const staticValue = param === "volume" ? track.volume : track.pan;
  const [hover, setHover] = useState<number | null>(null);

  const setPoints = (fn: (pts: AutoPoint[]) => AutoPoint[], mode: "commit" | "gesture") => {
    const recipe = (p: Parameters<Parameters<typeof commit>[0]>[0]) => {
      const t = draftTrack(p, track.id);
      if (!t) return;
      t.automation = t.automation ?? {};
      t.automation[param] = sorted(fn(sorted(track.automation?.[param] ?? [])));
    };
    if (mode === "commit") commit(recipe);
    else gesture(recipe);
  };

  const yOf = (clientY: number, el: Element) => clientY - el.getBoundingClientRect().top;

  const dragPoint = (e: React.PointerEvent, index: number) => {
    e.stopPropagation();
    if (e.button !== 0) return;
    const svg = (e.currentTarget as SVGElement).ownerSVGElement!;
    capturePointer(e.currentTarget as Element, e.pointerId);
    const target = e.currentTarget as Element;
    beginGesture();
    const move = (ev: PointerEvent) => {
      const beat = Math.max(0, snapBeat(beatAtClientX(ev.clientX), "round", ev.metaKey || ev.ctrlKey));
      const value = fromY(param, yOf(ev.clientY, svg), h);
      setHover(index);
      setPoints((pts) => pts.map((p, i) => (i === index ? { beat, value } : p)), "gesture");
    };
    const up = () => {
      target.removeEventListener("pointermove", move as EventListener);
      target.removeEventListener("pointerup", up);
      endGesture();
      setHover(null);
    };
    target.addEventListener("pointermove", move as EventListener);
    target.addEventListener("pointerup", up);
  };

  const onBackgroundDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const svg = e.currentTarget as SVGSVGElement;
    const beat = Math.max(0, snapBeat(beatAtClientX(e.clientX), "round", e.metaKey || e.ctrlKey));
    const value = fromY(param, yOf(e.clientY, svg), h);
    const base = points.length ? points : [];
    // First point: also pin the current static value at beat 0 so the curve starts sensibly.
    const add: AutoPoint[] = base.length === 0 && beat > 0 ? [{ beat: 0, value: staticValue }, { beat, value }] : [{ beat, value }];
    commit((p) => {
      const t = draftTrack(p, track.id);
      if (!t) return;
      t.automation = t.automation ?? {};
      t.automation[param] = sorted([...(t.automation[param] ?? []), ...add]);
    });
  };

  const remove = (index: number) => setPoints((pts) => pts.filter((_, i) => i !== index), "commit");

  const line =
    points.length === 0
      ? `M0 ${toY(param, staticValue, h)} H${width}`
      : `M0 ${toY(param, points[0].value, h)} ` + points.map((p) => `L${p.beat * ppb} ${toY(param, p.value, h)}`).join(" ") + ` H${width}`;

  return (
    <svg className="automation" width={width} height={h} onPointerDown={onBackgroundDown} onContextMenu={(e) => e.stopPropagation()}>
      <rect x={0} y={0} width={width} height={h} className="automation-bg" />
      {param === "pan" && <line x1={0} x2={width} y1={toY(param, 0, h)} y2={toY(param, 0, h)} className="automation-center" />}
      <path d={line} className={`automation-line ${points.length ? "" : "inactive"}`} />
      {points.map((p, i) => (
        <g key={i}>
          <circle
            cx={p.beat * ppb}
            cy={toY(param, p.value, h)}
            r={hover === i ? 6 : 4.5}
            className="automation-point"
            onPointerDown={(e) => dragPoint(e, i)}
            onPointerEnter={() => setHover(i)}
            onPointerLeave={() => setHover((x) => (x === i ? null : x))}
            onDoubleClick={(e) => {
              e.stopPropagation();
              remove(i);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              remove(i);
            }}
          />
          {hover === i && (
            <text x={p.beat * ppb + 8} y={Math.max(12, toY(param, p.value, h) - 8)} className="automation-label">
              {formatAuto(param, p.value)}
            </text>
          )}
        </g>
      ))}
      <text x={8} y={h - 8} className="automation-title">
        {param === "volume" ? "Volume" : "Pan"} automation {points.length === 0 ? "— click to add points" : ""}
      </text>
    </svg>
  );
});

/** Linear interpolation of an envelope at `beat` (used for UI display). */
export function automationValueAt(points: AutoPoint[] | undefined, beat: number): number | null {
  if (!points || points.length === 0) return null;
  const pts = sorted(points);
  if (beat <= pts[0].beat) return pts[0].value;
  for (let i = 1; i < pts.length; i++) {
    if (beat < pts[i].beat) {
      const a = pts[i - 1],
        b = pts[i];
      return a.value + ((beat - a.beat) / (b.beat - a.beat || 1)) * (b.value - a.value);
    }
  }
  return pts[pts.length - 1].value;
}
