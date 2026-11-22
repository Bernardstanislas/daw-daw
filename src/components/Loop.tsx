// Loop page: play live on a loop. The project loop is the sequence and every
// track keeps its loop in one clip, so what is played here is also in Arrange.
// Rec starts a take that keeps cycling (overdub); each pass is a layer.

import { useEffect, useState } from "react";
import { select, useStore } from "../lib/store";
import { livePosition, useTransport } from "../lib/engine";
import { useLearn } from "../lib/midiMap";
import { deviceName } from "../lib/catalog";
import { noteName } from "../lib/music";
import type { LiveAction, Track } from "../lib/types";
import {
  LOOP_BARS,
  PAD_COUNT,
  TRANSPOSE_RANGE,
  clearPads,
  clearTrack,
  isDrumTrack,
  liveClip,
  loopInfo,
  nudgeTranspose,
  setBars,
  setLive,
  setTranspose,
  startPadLearn,
  stopPadLearn,
  toggleMute,
  trackTranspose,
  undoLayer,
  useLive,
} from "../lib/live";

const NO_LAYERS: never[] = [];

const QUANTIZE = [
  { label: "Off", beats: 0 },
  { label: "1/8", beats: 0.5 },
  { label: "1/16", beats: 0.25 },
];

function semis(v: number) {
  return `${v > 0 ? "+" : v < 0 ? "−" : "±"}${Math.abs(v)} st`;
}

/** Playhead inside the loop, refreshed on each animation frame (only re-renders on a new 16th). */
function useLoopPosition() {
  const [step, setStep] = useState(0);
  const playing = useTransport((s) => s.playing);
  const position = useTransport((s) => s.position);
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      setStep(Math.floor(livePosition() * 4));
      raf = requestAnimationFrame(tick);
    };
    if (playing) raf = requestAnimationFrame(tick);
    else setStep(Math.floor(position * 4));
    return () => cancelAnimationFrame(raf);
  }, [playing, position]);
  return step / 4;
}

/** Lamps of the loop ring: always 16, a big one every quarter of the loop. */
const RING_LAMPS = 16;
// Radius: the middle of the band between the dark disc (~72px) and the edge (~97px) of the ring.
const RING_RADIUS = 84;
const RING = Array.from({ length: RING_LAMPS }, (_, i) => {
  const a = (i / RING_LAMPS) * Math.PI * 2 - Math.PI / 2;
  return { left: `calc(50% + ${Math.cos(a) * RING_RADIUS}px)`, top: `calc(50% + ${Math.sin(a) * RING_RADIUS}px)` };
});

function LoopRing({ pos }: { pos: number }) {
  const project = useStore((s) => s.project);
  const recording = useTransport((s) => s.recording);
  const playing = useTransport((s) => s.playing);
  const pass = useLive((s) => s.pass);
  const { start, len } = loopInfo(project);
  const rel = Math.min(len - 1e-6, Math.max(0, pos - start));
  // A fixed ring, like on hardware: the loop length changes what a lamp is worth, never where the lamps are.
  const now = Math.floor((rel / len) * RING_LAMPS);
  // The position is on the top bar's display; the lamps show it in the loop, the middle counts the passes.
  const passes = playing ? String(Math.min(99, pass + 1)) : "--";
  return (
    <div className={`loop-ring ${recording ? "rec" : ""}`}>
      {RING.map(({ left, top }, i) => (
        <span key={i} className={`lamp ${i % 4 === 0 ? "bar" : ""} ${i < now ? "lit" : ""} ${i === now ? "now" : ""}`} style={{ left, top }} />
      ))}
      <div className="loop-ring-mid">
        <div className="lcd-digits">
          <span className="ghost">88</span>
          <span>{passes.padStart(2, "!")}</span>
        </div>
        <div className="lcd-label">pass</div>
      </div>
    </div>
  );
}

function Readout() {
  const recording = useTransport((s) => s.recording);
  const countIn = useTransport((s) => s.countIn);
  const playing = useTransport((s) => s.playing);
  const track = useStore((s) => s.project.tracks.find((t) => t.id === s.selection.trackId));
  const layers = useLive((s) => (track ? (s.layers[track.id]?.length ?? 0) : 0));
  const state = countIn ? "COUNT-IN" : recording ? "OVERDUB" : playing ? "PLAY" : "STOPPED";
  return (
    <div className={`loop-readout ${recording || countIn ? "rec" : ""}`}>
      <b>{state}</b>
      <span>{track ? `${track.name}${layers ? ` · L${layers}` : ""}` : "no track"}</span>
    </div>
  );
}

function Learnable({ action, className = "btn small", children, onClick, title }: { action: LiveAction; className?: string; children: React.ReactNode; onClick: () => void; title?: string }) {
  const learn = useLearn({ kind: "live", action });
  return (
    <button className={className} onClick={onClick} title={title} {...learn}>
      {children}
    </button>
  );
}

function Side({ pos }: { pos: number }) {
  const project = useStore((s) => s.project);
  const { bars } = loopInfo(project);
  const quantize = useLive((s) => s.quantize);
  const mode = useLive((s) => s.mode);
  const learnLength = useLearn({ kind: "live", action: "length" });
  return (
    <aside className="loop-side">
      <div className="section-title">
        <span>Loop</span>
      </div>
      <LoopRing pos={pos} />
      <Readout />
      <div className="loop-field">
        <span className="engraved">Length</span>
        <div className="seg" role="group" aria-label="Loop length in bars" title="Loop length (bars)" {...learnLength}>
          {LOOP_BARS.map((b) => (
            <button key={b} className={bars === b ? "on" : ""} onClick={() => setBars(b)}>
              {b}
            </button>
          ))}
        </div>
      </div>
      <div className="loop-field">
        <span className="engraved">Quantize</span>
        <div className="seg" role="group" aria-label="Quantize">
          {QUANTIZE.map((q) => (
            <button key={q.label} className={quantize === q.beats ? "on" : ""} onClick={() => setLive({ quantize: q.beats })}>
              {q.label}
            </button>
          ))}
        </div>
      </div>
      <div className="loop-field">
        <span className="engraved">On record</span>
        <div className="seg" role="group" aria-label="On record">
          <button className={mode === "overdub" ? "on" : ""} onClick={() => setLive({ mode: "overdub" })} title="Add to what the track already plays">
            Overdub
          </button>
          <button className={mode === "replace" ? "on" : ""} onClick={() => setLive({ mode: "replace" })} title="Empty the selected track's loop when recording starts">
            Replace
          </button>
        </div>
      </div>
    </aside>
  );
}

function TrackCard({ track, index, frac, loop }: { track: Track; index: number; frac: number; loop: { start: number; end: number } }) {
  const selected = useStore((s) => s.selection.trackId === track.id);
  const recording = useTransport((s) => s.recording);
  const playing = useTransport((s) => s.playing);
  const layers = useLive((s) => s.layers[track.id]?.length ?? 0);
  const pending = useLive((s) => s.pending[track.id]);
  const learnMute = useLearn({ kind: "track", trackId: track.id, prop: "mute" });
  const clip = liveClip(track);
  const notes = clip?.notes.length ?? 0;
  const audio = track.kind !== "instrument";
  const taking = recording && selected && !audio;
  // Clips from Arrange under the loop play too.
  const arrange = !notes && track.clips.some((c) => !c.live && !c.muted && c.start < loop.end && c.start + c.length > loop.start);
  const state = audio ? "audio" : taking ? "overdub" : track.mute ? "muted" : notes ? (playing ? "playing" : "ready") : arrange ? "arrange" : "empty";
  const drum = isDrumTrack(track);
  const tr = trackTranspose(track);
  const sub = [track.instrument ? deviceName(track.instrument) : audio ? "Audio track" : "No instrument", !audio && notes ? (drum ? "fixed" : pending !== undefined ? `${semis(tr)} → ${semis(pending)}` : semis(tr)) : null]
    .filter(Boolean)
    .join(" · ");
  return (
    <div
      className={`loop-card ${state} ${selected ? "sel" : ""}`}
      style={{ "--c": track.color } as React.CSSProperties}
      onClick={() => select({ trackId: track.id })}
      title={audio ? "Audio tracks are not looped here" : "Click: play and record this track with the keys"}
    >
      <div className="loop-card-top">
        <span className="tape">{track.name}</span>
        <span className="loop-card-num">{index < PAD_COUNT ? index + 1 : ""}</span>
      </div>
      <div className="loop-card-sub">{sub}</div>
      <div className="loop-card-foot">
        <span className={`lamp ${state === "playing" || state === "overdub" || (state === "arrange" && playing) ? "lit" : ""} ${state === "overdub" ? "rec" : ""}`} style={{ "--c": track.color } as React.CSSProperties} />
        <span>{state.toUpperCase()}</span>
        <span className="loop-card-layers">
          {Array.from({ length: Math.min(layers, 6) }, (_, i) => (
            <span key={i} className="lamp lit" />
          ))}
        </span>
        {!audio && (
          <button
            className={`loop-mute ${track.mute ? "on" : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              toggleMute(track.id);
            }}
            title="Mute (pads of bank A)"
            {...learnMute}
          >
            M
          </button>
        )}
      </div>
      {notes > 0 && (
        <div className="loop-card-slot">
          <i style={{ width: `${track.mute ? 100 : frac * 100}%` }} />
        </div>
      )}
    </div>
  );
}

function LayersRow() {
  const track = useStore((s) => s.project.tracks.find((t) => t.id === s.selection.trackId));
  const layers = useLive((s) => (track ? s.layers[track.id] : undefined)) ?? NO_LAYERS;
  const recording = useTransport((s) => s.recording);
  if (!track) return null;
  return (
    <>
      <div className="section-title">
        <span>{track.name} · layers</span>
      </div>
      <div className="loop-layers">
        {layers.length === 0 && <span className="engraved">{liveClip(track)?.notes.length ? "Recorded before this session: no layer to undo" : "Nothing recorded yet: select a track, press Rec and play"}</span>}
        {layers.map((l, i) => (
          <span key={l.id} className="loop-chip">
            <span className={`lamp lit ${recording && i === layers.length - 1 ? "rec" : ""}`} style={{ "--c": track.color } as React.CSSProperties} />
            <b>L{i + 1}</b>
            <span>
              {l.noteIds.length} note{l.noteIds.length > 1 ? "s" : ""}
            </span>
          </span>
        ))}
        <span className="spacer" />
        <Learnable action="undoLayer" onClick={undoLayer} title="Remove the last layer of this track">
          ↶ Undo layer
        </Learnable>
        <button className="btn small" onClick={clearTrack} title="Empty this track's loop">
          Clear track
        </button>
      </div>
    </>
  );
}

/** One octave and a fifth, from C3 (−12) to C5 (+12) around middle C. */
const MINI_KEYS = Array.from({ length: 25 }, (_, i) => 48 + i);
const BLACK = new Set([1, 3, 6, 8, 10]);

function TransposeRow() {
  const track = useStore((s) => s.project.tracks.find((t) => t.id === s.selection.trackId));
  const apply = useLive((s) => s.apply);
  const keys = useLive((s) => s.keysTranspose);
  const pending = useLive((s) => (track ? s.pending[track.id] : undefined));
  if (!track) return null;
  const drum = isDrumTrack(track);
  const current = trackTranspose(track);
  const shown = pending ?? current;
  const whites = MINI_KEYS.filter((k) => !BLACK.has(k % 12));
  const disabled = drum || track.kind !== "instrument";
  return (
    <>
      <div className="section-title">
        <span>{track.name} · transpose</span>
      </div>
      <div className={`loop-transpose ${disabled ? "disabled" : ""}`}>
        <div className="mini-keys" title="Click a key: transpose this track by its distance from middle C">
          {whites.map((k) => (
            <span key={k} className={`wk ${k - 60 === shown ? "down" : ""} ${k === 60 ? "c4" : ""}`} onClick={() => !disabled && setTranspose(track.id, k - 60)} />
          ))}
          {MINI_KEYS.filter((k) => BLACK.has(k % 12)).map((k) => {
            const before = whites.filter((w) => w < k).length;
            return <span key={k} className={`bk ${k - 60 === shown ? "down" : ""}`} style={{ left: before * 18 - 6 }} onClick={() => !disabled && setTranspose(track.id, k - 60)} />;
          })}
        </div>
        <div className="loop-readout small">
          <b>{drum ? "fixed" : semis(shown)}</b>
          <span>{drum ? "drum kit" : pending !== undefined ? "next loop" : noteName(60 + shown).replace(/-?\d+$/, "")}</span>
        </div>
        <button className="btn small" disabled={disabled || shown <= -TRANSPOSE_RANGE} onClick={() => nudgeTranspose(-1)} title="Down a semitone">
          −
        </button>
        <button className="btn small" disabled={disabled || shown >= TRANSPOSE_RANGE} onClick={() => nudgeTranspose(1)} title="Up a semitone">
          +
        </button>
        <span className="engraved">Apply</span>
        <div className="seg" role="group" aria-label="Apply transpose">
          <button className={apply === "now" ? "on" : ""} onClick={() => setLive({ apply: "now" })}>
            Now
          </button>
          <button className={apply === "next" ? "on" : ""} onClick={() => setLive({ apply: "next" })} title="At the start of the next pass, to stay in time">
            Next loop
          </button>
        </div>
        <span className="spacer" />
        <Learnable
          action="transpose"
          className={`btn small loop-toggle ${keys ? "on" : ""}`}
          onClick={() => setLive({ keysTranspose: !keys })}
          title="While on, a key of the keyboard sets the transpose of the selected track (distance from middle C) instead of playing. Map it to a keyboard button: tap = latch, hold = momentary."
        >
          <span className={`lamp ${keys ? "lit" : ""}`} />
          Keys set transpose
        </Learnable>
      </div>
    </>
  );
}

function PadRow() {
  const tracks = useStore((s) => s.project.tracks);
  const pads = useLive((s) => s.pads);
  const learning = useLive((s) => s.padLearn);
  const learned = pads.filter(Boolean).length;
  return (
    <div className="loop-keybed">
      <div className="loop-bank">
        <span className="engraved">KeyLab pads · bank A</span>
        {learning !== null ? (
          <button className="btn small loop-toggle on" onClick={stopPadLearn}>
            <span className="lamp lit rec" />
            Hit pad {learning + 1} of {PAD_COUNT}…
          </button>
        ) : (
          <button className="btn small" onClick={startPadLearn} title="Switch the keyboard to pad bank A, then hit the pads 1 to 8 in order">
            {learned ? "Learn pads again" : "Learn pads"}
          </button>
        )}
        {learned > 0 && learning === null && (
          <button className="btn small" onClick={clearPads} title="Forget the pads: they play drums again">
            Forget
          </button>
        )}
      </div>
      <div className="loop-pads">
        {Array.from({ length: PAD_COUNT }, (_, i) => {
          const t = tracks[i];
          const pad = pads[i];
          return (
            <button
              key={i}
              className={`loop-pad ${t?.mute ? "muted" : ""} ${learning === i ? "learning" : ""}`}
              disabled={!t}
              onClick={() => t && toggleMute(t.id)}
              title={pad ? `Channel ${pad[0]} · note ${pad[1]} (${noteName(pad[1])})` : "Not learned"}
            >
              <span className={`lamp ${t && !t.mute && t.kind === "instrument" && liveClip(t)?.notes.length ? "lit" : ""}`} style={{ "--c": t?.color } as React.CSSProperties} />
              <span className="loop-pad-name">{t?.name ?? "—"}</span>
              <span className="loop-pad-sub">{!t ? "empty" : t.mute ? "muted" : "mute"}</span>
            </button>
          );
        })}
      </div>
      <div className="loop-legend" title="Map these to buttons of the keyboard with MIDI learn (top bar)">
        <span className="engraved">Bank B: drums, as usual</span>
        <LegendKey action="undoLayer" label="remove last layer" />
        <LegendKey action="length" label="loop length 1 → 8" />
        <LegendKey action="prevTrack" label="previous track" />
        <LegendKey action="nextTrack" label="next track" />
        <LegendKey action="transpose" label="keys set transpose" />
      </div>
    </div>
  );
}

const LEGEND_KEYS: Record<LiveAction, string> = {
  undoLayer: "Undo",
  length: "Loop",
  prevTrack: "◀",
  nextTrack: "▶",
  transpose: "Transp.",
};

function LegendKey({ action, label }: { action: LiveAction; label: string }) {
  const learn = useLearn({ kind: "live", action });
  const mapped = learn["data-learn-cc"];
  return (
    <span className="loop-legend-row" {...learn}>
      <span className="k">{LEGEND_KEYS[action]}</span>
      <span>{label}</span>
      <span className="cc">{mapped ?? "—"}</span>
    </span>
  );
}

export function Loop() {
  const tracks = useStore((s) => s.project.tracks);
  const project = useStore((s) => s.project);
  const pos = useLoopPosition();
  const { start, len } = loopInfo(project);
  const frac = Math.min(1, Math.max(0, (pos - start) / len));
  const slots = Math.max(PAD_COUNT, Math.ceil(tracks.length / 4) * 4);
  return (
    <div className="loop-page">
      <div className="loop-body">
        <Side pos={pos} />
        <main className="loop-main">
          <div className="section-title">
            <span>Tracks</span>
          </div>
          <div className="loop-grid">
            {Array.from({ length: slots }, (_, i) =>
              tracks[i] ? (
                <TrackCard key={tracks[i].id} track={tracks[i]} index={i} frac={frac} loop={project.loop} />
              ) : (
                <div key={`empty${i}`} className="loop-card hole">
                  <span className="engraved">{i === tracks.length ? "Add tracks in Arrange or from the browser" : ""}</span>
                </div>
              ),
            )}
          </div>
          <LayersRow />
          <TransposeRow />
        </main>
      </div>
      <PadRow />
    </div>
  );
}
