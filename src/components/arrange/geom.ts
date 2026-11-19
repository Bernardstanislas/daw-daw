// Shared geometry of the arrangement view (updated by <Arrange/> on render).

import { snapBeats, beatsPerBar } from "../../lib/music";
import { useStore } from "../../lib/store";

export const HEADER_W = 236;
export const RULER_H = 30;
export const ARRANGER_H = 24;
export const TOP_H = RULER_H + ARRANGER_H;

export const geom = {
  el: null as HTMLDivElement | null,
  ppb: 28,
  /** Cumulative top (px, relative to the first lane) of each track. */
  tops: [] as number[],
  heights: [] as number[],
};

export function beatAtClientX(clientX: number): number {
  const el = geom.el;
  if (!el) return 0;
  const r = el.getBoundingClientRect();
  return (clientX - r.left + el.scrollLeft - HEADER_W) / geom.ppb;
}

/** Index of the track under clientY (may be >= track count = empty area, -1 = above). */
export function trackIndexAtClientY(clientY: number): number {
  const el = geom.el;
  if (!el) return -1;
  const r = el.getBoundingClientRect();
  const y = clientY - r.top + el.scrollTop - TOP_H;
  if (y < 0) return -1;
  for (let i = 0; i < geom.tops.length; i++) {
    if (y < geom.tops[i] + geom.heights[i]) return i;
  }
  return geom.tops.length;
}

/** Arrangement grid (beats), coarsened when zoomed out so that cells stay >= 6 px. */
export function arrangeGrid(): number {
  const { ui, project } = useStore.getState();
  const bpb = beatsPerBar(project.timeSigNum, project.timeSigDen);
  let g = snapBeats(ui.snap, bpb);
  if (!g) return 0;
  while (g * geom.ppb < 6) g *= 2;
  return g;
}

export function snapBeat(b: number, mode: "round" | "floor" = "round", bypass = false): number {
  const g = bypass ? 0 : arrangeGrid();
  if (!g) return b;
  const f = mode === "round" ? Math.round : Math.floor;
  return f(b / g + 1e-9) * g;
}
