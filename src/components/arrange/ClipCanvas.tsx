// Draws the content of a clip (MIDI notes or audio waveform) on a canvas.

import { useEffect, useRef } from "react";
import type { AssetInfo, Clip } from "../../lib/types";

const MAX_CANVAS = 8000;

export function ClipCanvas({
  clip,
  width,
  height,
  pxPerBeat,
  bpm,
  asset,
  color,
}: {
  clip: Clip;
  width: number;
  height: number;
  pxPerBeat: number;
  bpm: number;
  asset?: AssetInfo;
  color: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const w = Math.max(1, Math.min(MAX_CANVAS, Math.round(width)));
  const h = Math.max(1, Math.round(height));
  useEffect(() => {
    const cv = ref.current;
    if (!cv) return;
    const dpr = window.devicePixelRatio || 1;
    cv.width = w * dpr;
    cv.height = h * dpr;
    const ctx = cv.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const sx = w / Math.max(1, width); // scale if capped
    if (clip.kind === "midi") drawMidi(ctx, clip, w, h, pxPerBeat * sx, color);
    else if (asset) drawWave(ctx, clip, w, h, pxPerBeat * sx, bpm, asset, color);
  }, [clip, w, h, pxPerBeat, bpm, asset, color, width]);
  return <canvas ref={ref} className="clip-canvas" style={{ width: Math.round(width), height: h }} />;
}

function drawMidi(ctx: CanvasRenderingContext2D, clip: Clip, w: number, h: number, ppb: number, color: string) {
  if (!clip.notes.length) return;
  let lo = 127,
    hi = 0;
  for (const n of clip.notes) {
    lo = Math.min(lo, n.pitch);
    hi = Math.max(hi, n.pitch);
  }
  lo -= 1;
  hi += 1;
  const range = Math.max(8, hi - lo + 1);
  const rowH = Math.max(1.5, Math.min(6, (h - 4) / range));
  const total = rowH * range;
  const top = (h - total) / 2;
  ctx.fillStyle = color;
  const reps: number[] = [];
  if (clip.loopLength > 0) {
    const first = Math.floor(clip.offset / clip.loopLength);
    const last = Math.ceil((clip.offset + clip.length) / clip.loopLength);
    for (let k = first; k <= last; k++) reps.push(k * clip.loopLength);
  } else reps.push(0);
  for (const base of reps) {
    for (const n of clip.notes) {
      if (clip.loopLength > 0 && n.start >= clip.loopLength) continue;
      const t = base + n.start - clip.offset;
      if (t + n.length < 0 || t >= clip.length) continue;
      const x = Math.max(0, t) * ppb;
      const x2 = Math.min(clip.length, t + n.length) * ppb;
      const y = top + (hi - n.pitch) * rowH;
      ctx.globalAlpha = 0.55 + (n.velocity / 127) * 0.45;
      ctx.fillRect(x, y, Math.max(1.5, x2 - x - 1), Math.max(1, rowH - 0.5));
    }
    if (clip.loopLength > 0 && base > 0) {
      const lx = (base - clip.offset) * ppb;
      if (lx > 0 && lx < w) {
        ctx.globalAlpha = 0.35;
        ctx.fillRect(lx, 0, 1, h);
      }
    }
  }
  ctx.globalAlpha = 1;
}

function drawWave(ctx: CanvasRenderingContext2D, clip: Clip, w: number, h: number, ppb: number, bpm: number, asset: AssetInfo, color: string) {
  const peaks = asset.peaks;
  const pps = asset.peaksPerSecond;
  const secPerPx = 60 / bpm / ppb;
  const mid = h / 2;
  const gain = Math.pow(10, clip.gain / 20);
  ctx.fillStyle = color;
  ctx.globalAlpha = 0.9;
  for (let x = 0; x < w; x++) {
    const t0 = clip.offset + x * secPerPx;
    const t1 = t0 + secPerPx;
    let i0 = Math.floor(t0 * pps);
    let i1 = Math.max(i0 + 1, Math.ceil(t1 * pps));
    if (i0 >= peaks.length) break;
    i1 = Math.min(i1, peaks.length);
    i0 = Math.max(0, i0);
    let m = 0;
    for (let i = i0; i < i1; i++) if (peaks[i] > m) m = peaks[i];
    // fades
    const beat = x / ppb;
    let g = gain;
    if (clip.fadeIn > 0 && beat < clip.fadeIn) g *= beat / clip.fadeIn;
    if (clip.fadeOut > 0 && clip.length - beat < clip.fadeOut) g *= Math.max(0, (clip.length - beat) / clip.fadeOut);
    const a = Math.min(1, m * g) * (mid - 1);
    ctx.fillRect(x, mid - a, 1, Math.max(1, a * 2));
  }
  ctx.globalAlpha = 1;
}
