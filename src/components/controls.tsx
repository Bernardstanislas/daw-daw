// Reusable controls: Knob, Fader, Meter, DragNumber.

import { useEffect, useRef, useState } from "react";
import { useTransport } from "../lib/engine";
import { clamp, dbToFader, faderToDb, formatDb, capturePointer } from "../lib/music";
import type { ParamSpec } from "../lib/types";

// ---------------------------------------------------------------- value mapping

export function toNorm(v: number, min: number, max: number, log: boolean): number {
  if (log && min > 0) return Math.log(v / min) / Math.log(max / min);
  return (v - min) / (max - min || 1);
}

export function fromNorm(n: number, min: number, max: number, log: boolean): number {
  const x = clamp(n, 0, 1);
  if (log && min > 0) return min * Math.pow(max / min, x);
  return min + x * (max - min);
}

export function formatParam(spec: ParamSpec, v: number): string {
  if (spec.kind === "choice") return spec.choices[Math.round(v)] ?? String(v);
  if (spec.kind === "toggle") return v >= 0.5 ? "On" : "Off";
  if (spec.kind === "int") return `${Math.round(v)}${spec.unit ? " " + spec.unit : ""}`;
  const u = spec.unit;
  if (u === "Hz" && v >= 1000) return `${(v / 1000).toFixed(v >= 10000 ? 1 : 2)} kHz`;
  if (u === "ms" && v >= 1000) return `${(v / 1000).toFixed(2)} s`;
  const abs = Math.abs(v);
  const digits = abs >= 100 ? 0 : abs >= 10 ? 1 : 2;
  return `${v.toFixed(digits)}${u ? (u === "%" ? "%" : " " + u) : ""}`;
}

// ---------------------------------------------------------------- drag helper

/** Vertical drag on an element; calls onDelta(dy normalized, fine) */
function useVerticalDrag(
  onStart: () => void,
  onMove: (deltaPx: number, fine: boolean) => void,
  onEnd: () => void,
) {
  return (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const startY = e.clientY;
    onStart();
    const move = (ev: PointerEvent) => onMove(startY - ev.clientY, ev.shiftKey || ev.altKey);
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      onEnd();
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };
}

// ---------------------------------------------------------------- Knob

interface KnobProps {
  value: number;
  min: number;
  max: number;
  defaultValue?: number;
  log?: boolean;
  step?: number;
  size?: number;
  label?: string;
  display?: string;
  color?: string;
  bipolar?: boolean;
  onBegin?: () => void;
  onChange: (v: number) => void;
  onEnd?: () => void;
  title?: string;
}

export function Knob({
  value,
  min,
  max,
  defaultValue,
  log = false,
  step,
  size = 34,
  label,
  display,
  color = "var(--accent)",
  bipolar,
  onBegin,
  onChange,
  onEnd,
  title,
}: KnobProps) {
  const [active, setActive] = useState(false);
  const startNorm = useRef(0);
  const norm = clamp(toNorm(value, min, max, log), 0, 1);
  const quantize = (v: number) => (step ? Math.round(v / step) * step : v);

  // Mouse wheel turns the knob and must NOT scroll the surrounding panel.
  // React registers wheel listeners as passive (preventDefault is ignored),
  // so a native non-passive listener is used. Consecutive ticks form one undo step.
  const svgRef = useRef<HTMLDivElement>(null);
  const latest = useRef({ norm, min, max, log, step, onBegin, onChange, onEnd });
  latest.current = { norm, min, max, log, step, onBegin, onChange, onEnd };
  useEffect(() => {
    const el = svgRef.current;
    if (!el) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let cur = 0;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const k = latest.current;
      if (timer === null) {
        k.onBegin?.();
        cur = k.norm;
      } else clearTimeout(timer);
      const delta = Math.abs(e.deltaY) > Math.abs(e.deltaX) ? e.deltaY : -e.deltaX;
      // Trackpads send many small deltas, mice a few big ones.
      cur = clamp(cur - Math.sign(delta) * Math.min(Math.abs(delta), 40) * (e.shiftKey ? 0.0002 : 0.001), 0, 1);
      let v = fromNorm(cur, k.min, k.max, k.log);
      if (k.step) v = Math.round(v / k.step) * k.step;
      k.onChange(clamp(v, k.min, k.max));
      timer = setTimeout(() => {
        timer = null;
        latest.current.onEnd?.();
      }, 250);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      if (timer !== null) {
        clearTimeout(timer);
        latest.current.onEnd?.();
      }
    };
  }, []);
  const isBipolar = bipolar ?? (min < 0 && max > 0);

  const onPointerDown = useVerticalDrag(
    () => {
      startNorm.current = norm;
      setActive(true);
      onBegin?.();
    },
    (dy, fine) => {
      const n = startNorm.current + dy / (fine ? 800 : 160);
      onChange(clamp(quantize(fromNorm(n, min, max, log)), min, max));
    },
    () => {
      setActive(false);
      onEnd?.();
    },
  );

  // Analog knob: a machined, knurled cap that really rotates, a cream pointer,
  // and a printed scale whose ticks light up from the origin to the value.
  const a0 = -135,
    a1 = 135;
  const angle = a0 + norm * (a1 - a0);
  const c = size / 2;
  const ticks = size < 26 ? 7 : size < 36 ? 11 : 15;
  const zeroNorm = isBipolar ? clamp(toNorm(0, min, max, log), 0, 1) : 0;
  const lo = Math.min(zeroNorm, norm);
  const hi = Math.max(zeroNorm, norm);
  const rOut = c - 0.5;
  const rIn = c - (size < 26 ? 2 : 3);
  const cap = size - (size < 26 ? 6 : 10);
  const tickEls = Array.from({ length: ticks }, (_, i) => {
    const t = i / (ticks - 1);
    const deg = a0 + t * (a1 - a0);
    const rad = ((deg - 90) * Math.PI) / 180;
    const lit = t >= lo - 1e-6 && t <= hi + 1e-6 && (hi - lo > 0.001 || Math.abs(t - zeroNorm) < 1e-6);
    return (
      <line
        key={i}
        x1={c + rIn * Math.cos(rad)}
        y1={c + rIn * Math.sin(rad)}
        x2={c + rOut * Math.cos(rad)}
        y2={c + rOut * Math.sin(rad)}
        className={lit ? "knob-tick lit" : "knob-tick"}
        style={lit && color !== "var(--accent)" ? { stroke: color } : undefined}
      />
    );
  });

  return (
    <div className={`knob ${active ? "active" : ""}`} title={title ?? label}>
      <div
        ref={svgRef}
        className="knob-dial"
        style={{ width: size, height: size }}
        onPointerDown={onPointerDown}
        onDoubleClick={() => {
          if (defaultValue === undefined) return;
          onBegin?.();
          onChange(defaultValue);
          onEnd?.();
        }}
      >
        <svg width={size} height={size} className="knob-scale" aria-hidden>
          {tickEls}
        </svg>
        <div className={`knob-cap ${size < 26 ? "small" : ""}`} style={{ width: cap, height: cap, transform: `translate(-50%, -50%) rotate(${angle}deg)` }} />
      </div>
      {label !== undefined && <div className="knob-label">{label}</div>}
      {display !== undefined && <div className={`knob-value ${active ? "lit" : ""}`}>{display}</div>}
    </div>
  );
}

// ---------------------------------------------------------------- Fader

interface FaderProps {
  db: number;
  height?: number;
  onBegin?: () => void;
  onChange: (db: number) => void;
  onEnd?: () => void;
}

const FADER_MARKS = [6, 0, -6, -12, -24, -48];

/** Console-style fader: printed dB scale, a slot and a grooved cap. */
export function Fader({ db, height = 160, onBegin, onChange, onEnd }: FaderProps) {
  const start = useRef(0);
  const pos = dbToFader(db);
  const onPointerDown = useVerticalDrag(
    () => {
      start.current = pos;
      onBegin?.();
    },
    (dy, fine) => {
      const x = clamp(start.current + dy / (fine ? height * 6 : height), 0, 1);
      const v = faderToDb(x);
      onChange(Math.round(v * 10) / 10);
    },
    () => onEnd?.(),
  );
  return (
    <div
      className="fader"
      style={{ height }}
      onPointerDown={onPointerDown}
      onDoubleClick={() => {
        onBegin?.();
        onChange(0);
        onEnd?.();
      }}
      title={`${formatDb(db)} dB — double-click: 0 dB`}
    >
      <div className="fader-scale" aria-hidden>
        {FADER_MARKS.map((m) => (
          <div key={m} className={`fader-mark ${m === 0 ? "unity" : ""}`} style={{ bottom: `${dbToFader(m) * 100}%` }}>
            <span>{m > 0 ? `+${m}` : m}</span>
          </div>
        ))}
        <div className="fader-mark" style={{ bottom: 0 }}>
          <span>∞</span>
        </div>
      </div>
      <div className="fader-slot" />
      <div className="fader-cap" style={{ bottom: `calc(${pos * 100}% - 12px)` }} />
    </div>
  );
}

// ---------------------------------------------------------------- Meter

function levelToHeight(v: number): number {
  // -60 dB .. +6 dB mapped to 0..1
  if (v <= 0.001) return 0;
  const db = 20 * Math.log10(v);
  return clamp((db + 60) / 66, 0, 1);
}

/** Stereo peak meter fed by the engine status (DOM updated without re-render). */
export function Meter({ source, vertical = true, thickness = 4 }: { source: string; vertical?: boolean; thickness?: number }) {
  const lRef = useRef<HTMLDivElement>(null);
  const rRef = useRef<HTMLDivElement>(null);
  const state = useRef({ l: 0, r: 0, pl: 0, pr: 0 });
  useEffect(() => {
    const read = (s: ReturnType<typeof useTransport.getState>): [number, number] => {
      if (source === "master") return s.master;
      if (source === "input") return s.input;
      return s.meters[source] ?? [0, 0];
    };
    let raf = 0;
    let target: [number, number] = [0, 0];
    const unsub = useTransport.subscribe((s) => {
      target = read(s);
    });
    const tick = () => {
      const st = state.current;
      st.l = Math.max(target[0], st.l * 0.86);
      st.r = Math.max(target[1], st.r * 0.86);
      const hl = levelToHeight(st.l);
      const hr = levelToHeight(st.r);
      const prop = vertical ? "height" : "width";
      if (lRef.current) {
        lRef.current.style[prop] = `${hl * 100}%`;
        lRef.current.classList.toggle("clip", st.l >= 1);
      }
      if (rRef.current) {
        rRef.current.style[prop] = `${hr * 100}%`;
        rRef.current.classList.toggle("clip", st.r >= 1);
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => {
      unsub();
      cancelAnimationFrame(raf);
    };
  }, [source, vertical]);
  return (
    <div className={`meter ${vertical ? "vertical" : "horizontal"}`} style={{ ["--meter-th" as string]: `${thickness}px` }}>
      <div className="meter-ch">
        <div className="meter-fill" ref={lRef} />
      </div>
      <div className="meter-ch">
        <div className="meter-fill" ref={rRef} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- DragNumber

interface DragNumberProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  digits?: number;
  pxPerStep?: number;
  onChange: (v: number) => void;
  onBegin?: () => void;
  onEnd?: () => void;
  className?: string;
  suffix?: string;
  title?: string;
}

/** A number you can drag vertically or double-click to type. */
export function DragNumber({ value, min, max, step = 1, digits = 0, pxPerStep = 4, onChange, onBegin, onEnd, className, suffix, title }: DragNumberProps) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const startVal = useRef(value);
  const onPointerDown = useVerticalDrag(
    () => {
      startVal.current = value;
      onBegin?.();
    },
    (dy, fine) => {
      const steps = Math.round(dy / (fine ? pxPerStep * 4 : pxPerStep));
      const fineStep = fine && digits > 0 ? step / 10 : step;
      onChange(clamp(+(startVal.current + steps * fineStep).toFixed(Math.max(digits, 2)), min, max));
    },
    () => onEnd?.(),
  );
  if (editing) {
    return (
      <input
        className={`drag-number-input ${className ?? ""}`}
        autoFocus
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => setEditing(false)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") {
            const v = parseFloat(text.replace(",", "."));
            if (Number.isFinite(v)) {
              onBegin?.();
              onChange(clamp(v, min, max));
              onEnd?.();
            }
            setEditing(false);
          } else if (e.key === "Escape") setEditing(false);
        }}
      />
    );
  }
  return (
    <span
      className={`drag-number ${className ?? ""}`}
      onPointerDown={onPointerDown}
      onDoubleClick={() => {
        setText(value.toFixed(digits));
        setEditing(true);
      }}
      title={title ?? "Drag or double-click to edit"}
    >
      <span className="dn-value">{value.toFixed(digits)}</span>
      {suffix && <span className="dn-unit">{suffix.trim()}</span>}
    </span>
  );
}

// ---------------------------------------------------------------- VU meter

/** VU scale marks (VU units; 0 VU = -18 dBFS). */
const VU_MARKS = [-20, -10, -7, -5, -3, -2, -1, 0, 1, 2, 3];
const VU_MIN = -20;
const VU_MAX = 3;
const VU_SWEEP = 48; // degrees each side of center

/** dB → needle angle (the printed scale is not linear in dB, like a real meter). */
function vuAngle(vu: number): number {
  const v = Math.max(VU_MIN, Math.min(VU_MAX + 1, vu));
  // Map through amplitude so the scale bunches up at the low end.
  const a = Math.pow(10, v / 20);
  const aMin = Math.pow(10, VU_MIN / 20);
  const aMax = Math.pow(10, VU_MAX / 20);
  const t = (a - aMin) / (aMax - aMin);
  return -VU_SWEEP + t * 2 * VU_SWEEP;
}

/**
 * Backlit analog VU meter with a real needle and VU ballistics (~300 ms).
 * `channel`: which side of the stereo pair it shows (0 left, 1 right, -1 the louder one).
 */
export function VuMeter({ source = "master", channel = -1, width = 124, label = "VU" }: { source?: string; channel?: 0 | 1 | -1; width?: number; label?: string }) {
  const needle = useRef<SVGGElement>(null);
  const height = Math.round(width * 0.52);
  const cx = width / 2;
  const cy = height * 1.18;
  const r = height * 0.98;
  useEffect(() => {
    let raf = 0;
    let level = 0;
    let vu = VU_MIN;
    let last = performance.now();
    const unsub = useTransport.subscribe((s) => {
      const lr = source === "master" ? s.master : (s.meters[source] ?? [0, 0]);
      level = channel === -1 ? Math.max(lr[0], lr[1]) : lr[channel];
    });
    const tick = (now: number) => {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const target = level > 1e-5 ? 20 * Math.log10(level) + 18 - 3 : VU_MIN - 2; // peak → rough average
      vu += (target - vu) * (1 - Math.exp(-dt / 0.3));
      if (needle.current) needle.current.style.transform = `rotate(${vuAngle(vu)}deg)`;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => {
      unsub();
      cancelAnimationFrame(raf);
    };
  }, [source, channel]);

  const polar = (deg: number, rr: number) => {
    const rad = ((deg - 90) * Math.PI) / 180;
    return [cx + rr * Math.cos(rad), cy + rr * Math.sin(rad)];
  };
  const arc = (from: number, to: number, rr: number) => {
    const [x0, y0] = polar(from, rr);
    const [x1, y1] = polar(to, rr);
    return `M ${x0} ${y0} A ${rr} ${rr} 0 0 1 ${x1} ${y1}`;
  };
  return (
    <div className="vu" style={{ width, height }} title="Output level (VU)">
      <svg width={width} height={height} aria-hidden>
        <path d={arc(vuAngle(VU_MIN), vuAngle(0), r * 0.82)} className="vu-scale" />
        <path d={arc(vuAngle(0), vuAngle(VU_MAX), r * 0.82)} className="vu-scale red" />
        {VU_MARKS.map((m) => {
          const a = vuAngle(m);
          const [x0, y0] = polar(a, r * 0.82);
          const [x1, y1] = polar(a, r * (m % 5 === 0 || m >= 0 ? 0.92 : 0.88));
          const [tx, ty] = polar(a, r * 0.99);
          const major = [-20, -10, -5, 0, 3].includes(m);
          return (
            <g key={m} className={m > 0 ? "red" : ""}>
              <line x1={x0} y1={y0} x2={x1} y2={y1} className="vu-tick" />
              {major && width >= 100 && (
                <text x={tx} y={ty} className="vu-num">
                  {m > 0 ? `+${m}` : Math.abs(m)}
                </text>
              )}
            </g>
          );
        })}
        <text x={cx} y={height * 0.78} className="vu-label">
          {label}
        </text>
        <g ref={needle} className="vu-needle" style={{ transformOrigin: `${cx}px ${cy}px` }}>
          <line x1={cx} y1={cy} x2={cx} y2={cy - r * 0.95} />
        </g>
      </svg>
      <div className="vu-glass" />
    </div>
  );
}

// ---------------------------------------------------------------- horizontal fader

/**
 * Hardware horizontal fader (slot, grooved cap, printed ticks). `value` in
 * [min, max]; `log` for zoom-like ranges. Wheel and drag supported.
 */
export function HFader({
  value,
  min,
  max,
  log = false,
  width = 120,
  onChange,
  title,
  label,
}: {
  value: number;
  min: number;
  max: number;
  log?: boolean;
  width?: number;
  onChange: (v: number) => void;
  title?: string;
  label?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const norm = clamp(toNorm(value, min, max, log), 0, 1);
  const CAP = 14;
  const latest = useRef({ norm, min, max, log, onChange });
  latest.current = { norm, min, max, log, onChange };
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const k = latest.current;
      const d = Math.abs(e.deltaY) > Math.abs(e.deltaX) ? -e.deltaY : e.deltaX;
      k.onChange(fromNorm(clamp(k.norm + Math.sign(d) * Math.min(Math.abs(d), 40) * 0.0015, 0, 1), k.min, k.max, k.log));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);
  const setFrom = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect();
    const n = clamp((clientX - r.left - CAP / 2) / (r.width - CAP), 0, 1);
    onChange(fromNorm(n, min, max, log));
  };
  return (
    <div className="hfader-wrap" title={title}>
      {label && <span className="engraved">{label}</span>}
      <div
        ref={ref}
        className="hfader"
        style={{ width }}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          const el = e.currentTarget as HTMLElement;
          capturePointer(el, e.pointerId);
          setFrom(e.clientX);
          const move = (ev: PointerEvent) => setFrom(ev.clientX);
          const up = () => {
            el.removeEventListener("pointermove", move);
            el.removeEventListener("pointerup", up);
          };
          el.addEventListener("pointermove", move);
          el.addEventListener("pointerup", up);
        }}
      >
        <div className="hfader-ticks" aria-hidden>
          {Array.from({ length: 9 }, (_, i) => (
            <span key={i} className={i % 4 === 0 ? "major" : ""} style={{ left: `calc(${CAP / 2}px + (100% - ${CAP}px) * ${i / 8})` }} />
          ))}
        </div>
        <div className="hfader-slot" />
        <div className="hfader-cap" style={{ left: `calc((100% - ${CAP}px) * ${norm})` }} />
      </div>
    </div>
  );
}
