// Spectrum analyzer drawn as a filled, glowing curve: the backdrop of the Sound page.
// The engine sends dBFS per log-spaced band (20 Hz – 20 kHz); smoothing and the
// display tilt happen here, on a canvas redrawn every frame without React.

import { useEffect, useRef } from "react";
import { backend } from "../lib/backend";
import { useTransport } from "../lib/engine";

const F_LO = 20;
const F_HI = 20_000;
/** dB/octave around 1 kHz, so that a full mix reads roughly flat (as in most analyzers). */
const TILT = 4.5;
const DB_TOP = 6;
const DB_BOTTOM = -84;
/** Seconds for the curve to rise / fall by ~63% towards the new value. */
const RISE = 0.02;
const FALL = 0.28;
const GRID = [50, 100, 200, 500, 1000, 2000, 5000, 10000];
const LABELS: Record<number, string> = { 100: "100", 1000: "1k", 10000: "10k" };

const xOfFreq = (f: number, w: number) => (Math.log(f / F_LO) / Math.log(F_HI / F_LO)) * w;

/** Live spectrum of `source` (a track id or "master"), filling its positioned parent. */
export function Spectrum({ source }: { source: string }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    backend.setScope(source).catch(() => {});
    return () => void backend.setScope(null).catch(() => {});
  }, [source]);

  useEffect(() => {
    const canvas = ref.current;
    const g = canvas?.getContext("2d");
    if (!canvas || !g) return;
    let w = 0;
    let h = 0;
    /** Nothing to animate: the last frame stays on the canvas. */
    let idle = false;
    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      w = canvas.clientWidth;
      h = canvas.clientHeight;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      idle = false; // resizing clears the canvas
    };
    const ro = new ResizeObserver(resize);
    ro.observe(canvas);
    resize();

    let shown: Float32Array | null = null;
    let last = performance.now();
    let raf = 0;

    const tick = (now: number) => {
      raf = requestAnimationFrame(tick);
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const raw = useTransport.getState().spectrum;
      const n = raw?.length ?? shown?.length ?? 0;
      if (n < 2) return;
      if (!shown || shown.length !== n) shown = new Float32Array(n).fill(DB_BOTTOM);

      // Ballistics, in dB, after the display tilt.
      const up = 1 - Math.exp(-dt / RISE);
      const down = 1 - Math.exp(-dt / FALL);
      let alive = false;
      for (let i = 0; i < n; i++) {
        const f = F_LO * Math.pow(F_HI / F_LO, (i + 0.5) / n);
        const target = raw ? Math.max(DB_BOTTOM, raw[i] + TILT * Math.log2(f / 1000)) : DB_BOTTOM;
        const cur = shown[i];
        shown[i] = cur + (target - cur) * (target > cur ? up : down);
        if (shown[i] > DB_BOTTOM + 0.5) alive = true;
      }
      if (!alive && idle) return;
      idle = !alive;
      draw(shown);
    };

    const draw = (bands: Float32Array) => {
      g.clearRect(0, 0, w, h);
      if (w <= 0 || h <= 0) return;

      // Faint frequency grid.
      g.lineWidth = 1;
      g.font = "10px ui-monospace, SFMono-Regular, Menlo, monospace";
      g.textAlign = "center";
      for (const f of GRID) {
        const x = Math.round(xOfFreq(f, w)) + 0.5;
        g.strokeStyle = LABELS[f] ? "rgba(255, 186, 140, 0.07)" : "rgba(255, 186, 140, 0.035)";
        g.beginPath();
        g.moveTo(x, 0);
        g.lineTo(x, h);
        g.stroke();
        if (LABELS[f]) {
          g.fillStyle = "rgba(255, 186, 140, 0.22)";
          g.fillText(LABELS[f], x, h - 6);
        }
      }

      // Light spatial smoothing, then a curve through the band centres.
      const n = bands.length;
      const yOf = (db: number) => h - ((Math.min(DB_TOP, db) - DB_BOTTOM) / (DB_TOP - DB_BOTTOM)) * h;
      const pts: [number, number][] = [];
      for (let i = 0; i < n; i++) {
        const a = bands[Math.max(0, i - 1)];
        const b = bands[Math.min(n - 1, i + 1)];
        pts.push([((i + 0.5) / n) * w, yOf(0.25 * a + 0.5 * bands[i] + 0.25 * b)]);
      }
      const path = new Path2D();
      path.moveTo(0, pts[0][1]);
      for (let i = 0; i < n - 1; i++) {
        const [x0, y0] = pts[i];
        const [x1, y1] = pts[i + 1];
        path.quadraticCurveTo(x0, y0, (x0 + x1) / 2, (y0 + y1) / 2);
      }
      path.lineTo(w, pts[n - 1][1]);

      const fill = new Path2D(path);
      fill.lineTo(w, h);
      fill.lineTo(0, h);
      fill.closePath();
      const grad = g.createLinearGradient(0, 0, 0, h);
      grad.addColorStop(0, "rgba(255, 178, 128, 0.55)");
      grad.addColorStop(0.55, "rgba(236, 132, 82, 0.30)");
      grad.addColorStop(1, "rgba(200, 96, 56, 0.10)");
      g.fillStyle = grad;
      g.fill(fill);

      g.lineWidth = 1.5;
      g.lineJoin = "round";
      g.strokeStyle = "rgba(255, 200, 160, 0.75)";
      g.shadowColor = "rgba(255, 150, 90, 0.6)";
      g.shadowBlur = 8;
      g.stroke(path);
      g.shadowBlur = 0;
    };

    raf = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, []);

  return <canvas ref={ref} className="spectrum-backdrop" aria-hidden />;
}
