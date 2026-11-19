// Arrangement view: ruler, arranger sections, track headers and clip lanes.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { select, setUi, useStore } from "../../lib/store";
import { livePosition, useTransport } from "../../lib/engine";
import { addTrack, createMidiClip, addInstrumentTrack } from "../../lib/actions";
import { beatsPerBar, clamp, capturePointer } from "../../lib/music";
import { dropOnArrange, endDrag, readDrag } from "../../lib/dnd";
import { isTauri } from "../../lib/backend";
import { importAudioFiles } from "../../lib/actions";
import { ARRANGER_H, HEADER_W, RULER_H, TOP_H, beatAtClientX, geom, snapBeat, trackIndexAtClientY } from "./geom";
import { Ruler } from "./Ruler";
import { ArrangerLane } from "./ArrangerLane";
import { TrackHeader } from "./TrackHeader";
import { ClipView } from "./ClipView";
import { HFader } from "../controls";
import { AutomationOverlay } from "./AutomationOverlay";
import { IconFollow, IconMic, IconMinus, IconPlus, IconSection, IconSynth } from "../icons";
import { openContextMenu } from "../Menu";
import { useCatalog } from "../../lib/catalog";
import type { Track } from "../../lib/types";
import { addSection, hasClipboard, pasteClips } from "../../lib/actions";

function Playhead() {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const pos = livePosition();
      const ppb = geom.ppb;
      if (ref.current) ref.current.style.transform = `translateX(${HEADER_W + pos * ppb}px)`;
      // Follow playback
      const el = geom.el;
      const t = useTransport.getState();
      if (el && t.playing && useStore.getState().ui.follow) {
        const x = pos * ppb;
        const visW = el.clientWidth - HEADER_W;
        if (x > el.scrollLeft + visW - 40 || x < el.scrollLeft) {
          el.scrollLeft = Math.max(0, x - 60);
        }
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);
  return <div className="playhead" ref={ref} />;
}

function TakePreview({ track, ppb }: { track: Track; ppb: number }) {
  const notes = useTransport((s) => s.takeNotes[track.id]);
  const recording = useTransport((s) => s.recording);
  const takeStart = useTransport((s) => s.takeStart);
  const pos = useTransport((s) => s.position);
  if (!recording || takeStart === null) return null;
  const isAudio = track.kind === "audio" && track.armed;
  if (!notes?.length && !isAudio) return null;
  const start = takeStart;
  const end = Math.max(pos, start + 0.1);
  let lo = 127,
    hi = 0;
  for (const n of notes ?? []) {
    lo = Math.min(lo, n.pitch);
    hi = Math.max(hi, n.pitch);
  }
  const range = Math.max(12, hi - lo + 2);
  const h = track.height - 20;
  return (
    <div className="clip recording-take" style={{ left: start * ppb, width: (end - start) * ppb, height: track.height - 2 }}>
      <div className="clip-header">
        <span className="clip-name">● Recording…</span>
      </div>
      <div className="clip-body" style={{ position: "relative" }}>
        {(notes ?? []).map((n) => (
          <div
            key={n.id}
            className="take-note"
            style={{
              left: (n.start - start) * ppb,
              width: Math.max(2, n.length * ppb),
              top: ((hi + 1 - n.pitch) / range) * h,
              height: Math.max(2, h / range - 1),
            }}
          />
        ))}
      </div>
    </div>
  );
}

function Lane({ track, width, ppb, bpm, selectedClips }: { track: Track; width: number; ppb: number; bpm: number; selectedClips: Set<string> }) {
  return (
    <div className="lane" style={{ height: track.height, width }} data-track={track.id}>
      {track.clips.map((c) => (
        <ClipView key={c.id} clip={c} track={track} selected={selectedClips.has(c.id)} ppb={ppb} bpm={bpm} height={track.height} />
      ))}
      <TakePreview track={track} ppb={ppb} />
      {track.showAutomation && <AutomationOverlay track={track} param={track.showAutomation} width={width} ppb={ppb} />}
    </div>
  );
}

export function Arrange() {
  const project = useStore((s) => s.project);
  const selection = useStore((s) => s.selection);
  const ppb = useStore((s) => s.ui.pxPerBeat);
  const follow = useStore((s) => s.ui.follow);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [marquee, setMarquee] = useState<null | { x0: number; y0: number; x1: number; y1: number }>(null);
  const [dropHint, setDropHint] = useState<null | { trackIdx: number; beat: number }>(null);
  const specsLoaded = useCatalog((s) => s.loaded);

  const bpb = beatsPerBar(project.timeSigNum, project.timeSigDen);
  const contentEnd = Math.max(
    0,
    ...project.tracks.flatMap((t) => t.clips.map((c) => c.start + c.length)),
    ...project.sections.map((s) => s.start + s.length),
    project.loop.end,
  );
  const [viewW, setViewW] = useState(1200);
  const width = Math.max(viewW - HEADER_W, (Math.ceil(contentEnd / bpb) + 32) * bpb * ppb);

  // Geometry for children
  geom.el = scrollRef.current;
  geom.ppb = ppb;
  let acc = 0;
  geom.tops = project.tracks.map((t) => {
    const top = acc;
    acc += t.height;
    return top;
  });
  geom.heights = project.tracks.map((t) => t.height);

  useLayoutEffect(() => {
    geom.el = scrollRef.current;
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewW(el.clientWidth));
    ro.observe(el);
    setViewW(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  // Ctrl/Cmd + wheel = zoom around pointer
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      const beat = beatAtClientX(e.clientX);
      const old = useStore.getState().ui.pxPerBeat;
      const next = clamp(old * Math.pow(1.0018, -e.deltaY), 2, 400);
      setUi({ pxPerBeat: next });
      requestAnimationFrame(() => {
        const r = el.getBoundingClientRect();
        el.scrollLeft = beat * next - (e.clientX - r.left - HEADER_W);
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // OS file drops (Tauri)
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      unlisten = await getCurrentWebview().onDragDropEvent((ev) => {
        const payload = ev.payload as { type: string; paths?: string[]; position?: { x: number; y: number } };
        if (payload.type === "drop" && payload.paths?.length && payload.position) {
          const dpr = window.devicePixelRatio || 1;
          const x = payload.position.x / dpr;
          const y = payload.position.y / dpr;
          const el = scrollRef.current;
          const r = el?.getBoundingClientRect();
          if (!r || x < r.left || x > r.right || y < r.top || y > r.bottom) {
            importAudioFiles(payload.paths, null, 0);
            return;
          }
          const idx = trackIndexAtClientY(y);
          const t = useStore.getState().project.tracks[idx];
          importAudioFiles(payload.paths, t?.kind === "audio" ? t.id : null, Math.max(0, snapBeat(beatAtClientX(x), "round")));
        }
      });
    })();
    return () => unlisten?.();
  }, []);

  const selectedClips = new Set(selection.clipIds);
  const tracksH = acc;

  // ----- background interactions (marquee / double-click create)
  const contentPoint = (clientX: number, clientY: number) => {
    const el = scrollRef.current!;
    const r = el.getBoundingClientRect();
    return { x: clientX - r.left + el.scrollLeft - HEADER_W, y: clientY - r.top + el.scrollTop - TOP_H };
  };

  const onLanesDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".clip")) return;
    const p0 = contentPoint(e.clientX, e.clientY);
    const idx = trackIndexAtClientY(e.clientY);
    const t = project.tracks[idx];
    if (t) select({ trackId: t.id });
    if (!e.shiftKey) select({ clipIds: [], sectionId: null });
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const base = e.shiftKey ? selection.clipIds : [];
    const move = (ev: PointerEvent) => {
      const p1 = contentPoint(ev.clientX, ev.clientY);
      setMarquee({ x0: p0.x, y0: p0.y, x1: p1.x, y1: p1.y });
      const [xa, xb] = [Math.min(p0.x, p1.x) / ppb, Math.max(p0.x, p1.x) / ppb];
      const [ya, yb] = [Math.min(p0.y, p1.y), Math.max(p0.y, p1.y)];
      const ids: string[] = [...base];
      project.tracks.forEach((tr, i) => {
        const top = geom.tops[i];
        if (top + tr.height < ya || top > yb) return;
        for (const c of tr.clips) if (c.start < xb && c.start + c.length > xa && !ids.includes(c.id)) ids.push(c.id);
      });
      select({ clipIds: ids });
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      setMarquee(null);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const onLanesDoubleClick = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest(".clip")) return;
    const idx = trackIndexAtClientY(e.clientY);
    const t = project.tracks[idx];
    if (!t) {
      addInstrumentTrack("pulse");
      return;
    }
    if (t.kind !== "instrument") return;
    const b = snapBeat(beatAtClientX(e.clientX), "floor");
    const start = Math.floor(b / bpb) * bpb;
    createMidiClip(t.id, start, bpb);
  };

  const onLanesContext = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest(".clip")) return;
    const idx = trackIndexAtClientY(e.clientY);
    const t = project.tracks[idx];
    const beat = Math.max(0, snapBeat(beatAtClientX(e.clientX), "floor"));
    openContextMenu(e, [
      ...(t?.kind === "instrument"
        ? [{ label: "Create MIDI clip here", onClick: () => createMidiClip(t.id, Math.floor(beat / bpb) * bpb, bpb) }]
        : []),
      { label: "Paste here", disabled: !hasClipboard(), onClick: () => pasteClips(beat) },
      { separator: true },
      { label: "Add instrument track", onClick: () => addInstrumentTrack("pulse") },
      { label: "Add audio track", onClick: () => addTrack("audio") },
    ]);
  };

  // ----- drag & drop from the browser
  const onDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    const idx = trackIndexAtClientY(e.clientY);
    const beat = Math.max(0, snapBeat(beatAtClientX(e.clientX), "floor"));
    setDropHint({ trackIdx: idx, beat });
  };
  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDropHint(null);
    const idx = trackIndexAtClientY(e.clientY);
    const t = project.tracks[idx];
    const beat = Math.max(0, snapBeat(beatAtClientX(e.clientX), "floor"));
    // Files dropped from the desktop (browser build; Tauri uses its own drop event).
    if (!isTauri && e.dataTransfer.files.length) {
      endDrag();
      const files = [...e.dataTransfer.files].filter((f) => f.type.startsWith("audio/") || /\.(wav|mp3|flac|ogg|aiff?|m4a)$/i.test(f.name));
      importAudioFiles(files, t?.kind === "audio" ? t.id : null, beat);
      return;
    }
    const payload = readDrag(e);
    endDrag();
    if (!payload) return;
    dropOnArrange(payload, t?.id ?? null, beat);
  };

  return (
    <div className="arrange">
      <div className="arrange-toolbar">
        <button className="btn small" onClick={() => addInstrumentTrack("pulse")} title="Add an instrument track">
          <IconSynth /> Instrument
        </button>
        <button className="btn small" onClick={() => addTrack("audio")} title="Add an audio track">
          <IconMic /> Audio
        </button>
        <button
          className="btn small"
          title="Add a song section at the playhead"
          onClick={() => {
            const p = useStore.getState().project;
            const start = Math.floor(livePosition() / bpb) * bpb;
            if (p.sections.some((s) => start >= s.start && start < s.start + s.length)) return;
            addSection(start, bpb * 4);
          }}
        >
          <IconSection /> Section
        </button>
        <div className="spacer" />
        <button className={`icon-btn ${follow ? "on" : ""}`} onClick={() => setUi({ follow: !follow })} title="Follow playhead (F)">
          <span className={`lamp ${follow ? "lit" : ""}`} />
          <IconFollow />
        </button>
        <button className="icon-btn" onClick={() => setUi({ pxPerBeat: clamp(ppb / 1.4, 2, 400) })} title="Zoom out (-)">
          <IconMinus />
        </button>
        <HFader value={ppb} min={2} max={400} log width={120} label="zoom" onChange={(v) => setUi({ pxPerBeat: v })} title="Horizontal zoom (⌘ + wheel)" />
        <button className="icon-btn" onClick={() => setUi({ pxPerBeat: clamp(ppb * 1.4, 2, 400) })} title="Zoom in (+)">
          <IconPlus />
        </button>
      </div>
      <div className="arrange-scroll" ref={scrollRef} onDragOver={onDragOver} onDragLeave={() => setDropHint(null)} onDrop={onDrop}>
        <div
          className="arrange-grid"
          style={{
            width: HEADER_W + width,
            gridTemplateColumns: `${HEADER_W}px ${width}px`,
            gridTemplateRows: `${RULER_H}px ${ARRANGER_H}px auto 1fr`,
            ["--bar-px" as string]: `${bpb * ppb}px`,
            ["--beat-px" as string]: `${ppb}px`,
          }}
        >
          <div className="corner corner-ruler">
            <span>{project.tracks.length} tracks</span>
          </div>
          <div className="ruler-cell">
            <Ruler width={width} ppb={ppb} />
          </div>
          <div className="corner corner-arranger">Arranger</div>
          <div className="arranger-cell">
            <ArrangerLane width={width} ppb={ppb} />
          </div>

          <div className="headers-col">
            {project.tracks.map((t, i) => (
              <TrackHeader key={t.id} track={t} index={i} selected={selection.trackId === t.id} />
            ))}
          </div>
          <div
            className="lanes-col"
            onPointerDown={onLanesDown}
            onDoubleClick={onLanesDoubleClick}
            onContextMenu={onLanesContext}
            style={{ backgroundSize: `${bpb * ppb}px 100%, ${ppb}px 100%` }}
          >
            {project.tracks.map((t) => (
              <Lane key={t.id} track={t} width={width} ppb={ppb} bpm={project.bpm} selectedClips={selectedClips} />
            ))}
            {project.loop.enabled && (
              <div
                className="loop-shade"
                style={{ left: project.loop.start * ppb, width: (project.loop.end - project.loop.start) * ppb, height: tracksH }}
              />
            )}
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
            {dropHint && dropHint.trackIdx >= 0 && dropHint.trackIdx < project.tracks.length && (
              <div
                className="drop-hint"
                style={{ left: dropHint.beat * ppb, top: geom.tops[dropHint.trackIdx], height: geom.heights[dropHint.trackIdx] }}
              />
            )}
          </div>
          <div className="headers-filler" />
          <div className={`lanes-filler ${dropHint && dropHint.trackIdx >= project.tracks.length ? "hot" : ""}`} onDoubleClick={() => addInstrumentTrack("pulse")}>
            {specsLoaded && (
              <div className="empty-hint">
                {project.tracks.length === 0 ? (
                  <>
                    <b>Start here</b>: drag an instrument, a loop or an audio file from the browser,
                    <br /> or double-click to add an instrument track.
                  </>
                ) : (
                  "Drop instruments, loops or audio files here to create a new track"
                )}
              </div>
            )}
          </div>
        </div>
        <Playhead />
      </div>
    </div>
  );
}
