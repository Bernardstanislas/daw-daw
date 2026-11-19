// A clip in the arrangement: rendering + move / resize / trim / copy drags.

import { memo } from "react";
import type { Clip, Track } from "../../lib/types";
import { beginGesture, endGesture, gesture, select, useStore, draftTrack, cancelGesture } from "../../lib/store";
import { secondsToBeats, uid, capturePointer } from "../../lib/music";
import { ClipCanvas } from "./ClipCanvas";
import { beatAtClientX, snapBeat, trackIndexAtClientY, arrangeGrid } from "./geom";
import { openContextMenu } from "../Menu";
import { clipMenu } from "./menus";

type Mode = "move" | "resize" | "trim";

const CLIP_INK = "rgba(14, 16, 20, 0.78)";

function startClipDrag(e: React.PointerEvent, clip: Clip, track: Track) {
  if (e.button !== 0) return;
  e.stopPropagation();
  const state = useStore.getState();
  const el = e.currentTarget as HTMLElement;
  const rect = el.getBoundingClientRect();
  const localX = e.clientX - rect.left;
  const edge = Math.min(8, rect.width / 4);
  const mode: Mode = localX < edge ? "trim" : localX > rect.width - edge ? "resize" : "move";

  // Selection
  let ids = state.selection.clipIds;
  if (e.shiftKey || e.metaKey || e.ctrlKey) {
    ids = ids.includes(clip.id) ? ids.filter((x) => x !== clip.id) : [...ids, clip.id];
    select({ clipIds: ids, trackId: track.id });
    if (!ids.includes(clip.id)) return;
  } else if (!ids.includes(clip.id)) {
    ids = [clip.id];
    select({ clipIds: ids, trackId: track.id });
  } else {
    select({ trackId: track.id });
  }
  if (clip.kind === "midi") useStore.setState({ editorClipId: clip.id });

  const project = state.project;
  const tracks = project.tracks;
  const startX = e.clientX;
  const startBeat = beatAtClientX(e.clientX);
  const startTrackIdx = tracks.findIndex((t) => t.id === track.id);
  const copy = e.altKey && mode === "move";
  const moving = ids
    .map((id) => {
      const ti = tracks.findIndex((t) => t.clips.some((c) => c.id === id));
      const c = ti >= 0 ? tracks[ti].clips.find((c) => c.id === id)! : null;
      return c ? { clip: c, trackIdx: ti, newId: uid("clip") } : null;
    })
    .filter(Boolean) as { clip: Clip; trackIdx: number; newId: string }[];
  const minStart = Math.min(...moving.map((m) => m.clip.start));
  let started = false;
  const bpm = project.bpm;
  const asset = clip.assetId ? state.assets[clip.assetId] : undefined;

  capturePointer(el, e.pointerId);
  const onMove = (ev: PointerEvent) => {
    if (!started) {
      if (Math.abs(ev.clientX - startX) < 3 && Math.abs(trackIndexAtClientY(ev.clientY) - startTrackIdx) === 0) return;
      started = true;
      beginGesture();
    }
    const bypass = ev.metaKey || ev.ctrlKey;
    const dBeats = beatAtClientX(ev.clientX) - startBeat;
    if (mode === "move") {
      let delta = snapBeat(clip.start + dBeats, "round", bypass) - clip.start;
      if (minStart + delta < 0) delta = -minStart;
      let dTrack = trackIndexAtClientY(ev.clientY) - startTrackIdx;
      if (trackIndexAtClientY(ev.clientY) < 0) dTrack = 0;
      const ok = moving.every((m) => {
        const t = tracks[m.trackIdx + dTrack];
        return t && (t.kind === "audio") === (m.clip.kind === "audio");
      });
      if (!ok) dTrack = 0;
      gesture((p) => {
        for (const m of moving) {
          const src = p.tracks[m.trackIdx];
          const dst = p.tracks[m.trackIdx + dTrack];
          if (copy) {
            dst.clips.push({
              ...JSON.parse(JSON.stringify(m.clip)),
              id: m.newId,
              start: m.clip.start + delta,
              notes: m.clip.notes.map((n) => ({ ...n, id: uid("n") })),
            });
          } else {
            const i = src.clips.findIndex((c) => c.id === m.clip.id);
            const [c] = src.clips.splice(i, 1);
            c.start = m.clip.start + delta;
            dst.clips.push(c);
          }
        }
      });
    } else if (mode === "resize") {
      const grid = bypass ? 0 : arrangeGrid();
      const minLen = grid || 0.125;
      let end = snapBeat(clip.start + clip.length + dBeats, "round", bypass);
      end = Math.max(clip.start + minLen, end);
      if (clip.kind === "audio" && asset) {
        const maxLen = secondsToBeats(asset.duration - clip.offset, bpm);
        end = Math.max(clip.start + minLen, Math.min(end, clip.start + maxLen));
      }
      gesture((p) => {
        const c = draftTrack(p, track.id)?.clips.find((c) => c.id === clip.id);
        if (c) c.length = end - clip.start;
      });
    } else {
      const grid = bypass ? 0 : arrangeGrid();
      const minLen = grid || 0.125;
      const end = clip.start + clip.length;
      let start = snapBeat(clip.start + dBeats, "round", bypass);
      start = Math.min(start, end - minLen);
      start = Math.max(0, start);
      let d = start - clip.start;
      if (clip.kind === "audio") {
        const minD = -secondsToBeats(clip.offset, bpm);
        d = Math.max(d, minD);
      }
      gesture((p) => {
        const c = draftTrack(p, track.id)?.clips.find((c) => c.id === clip.id);
        if (!c) return;
        c.start = clip.start + d;
        c.length = clip.length - d;
        if (c.kind === "audio") c.offset = Math.max(0, clip.offset + (d * 60) / bpm);
        else c.offset = clip.offset + d;
      });
    }
  };
  const onUp = (ev: PointerEvent) => {
    el.removeEventListener("pointermove", onMove);
    el.removeEventListener("pointerup", onUp);
    el.removeEventListener("pointercancel", onCancel);
    if (!started) return;
    endGesture();
    if (copy) select({ clipIds: moving.map((m) => m.newId) });
    void ev;
  };
  const onCancel = () => {
    el.removeEventListener("pointermove", onMove);
    el.removeEventListener("pointerup", onUp);
    el.removeEventListener("pointercancel", onCancel);
    if (started) cancelGesture();
  };
  el.addEventListener("pointermove", onMove);
  el.addEventListener("pointerup", onUp);
  el.addEventListener("pointercancel", onCancel);
}

export const ClipView = memo(function ClipView({
  clip,
  track,
  selected,
  ppb,
  bpm,
  height,
}: {
  clip: Clip;
  track: Track;
  selected: boolean;
  ppb: number;
  bpm: number;
  height: number;
}) {
  const asset = useStore((s) => (clip.assetId ? s.assets[clip.assetId] : undefined));
  const color = clip.color ?? track.color;
  const width = Math.max(4, clip.length * ppb);
  const missing = clip.kind === "audio" && !asset;
  return (
    <div
      className={`clip ${selected ? "selected" : ""} ${clip.muted ? "muted" : ""} ${clip.kind}`}
      style={{ left: clip.start * ppb, width, height: height - 2, ["--clip-color" as string]: color }}
      onPointerDown={(e) => startClipDrag(e, clip, track)}
      onDoubleClick={(e) => {
        e.stopPropagation();
        useStore.setState((s) => ({ editorClipId: clip.id, ui: { ...s.ui, bottom: "editor" } }));
      }}
      onContextMenu={(e) => {
        e.stopPropagation();
        const s = useStore.getState();
        if (!s.selection.clipIds.includes(clip.id)) select({ clipIds: [clip.id], trackId: track.id });
        openContextMenu(e, clipMenu(clip));
      }}
    >
      <div className="clip-header">
        <span className="clip-name">
          {clip.loopLength > 0 && <span className="clip-loop-icon">↻</span>}
          {clip.name}
          {missing && " (missing audio)"}
        </span>
      </div>
      <div className="clip-body">
        <ClipCanvas clip={clip} width={width} height={height - 18} pxPerBeat={ppb} bpm={bpm} asset={asset} color={CLIP_INK} />
      </div>
      {clip.kind === "audio" && clip.fadeIn > 0 && <div className="clip-fade in" style={{ width: clip.fadeIn * ppb }} />}
      {clip.kind === "audio" && clip.fadeOut > 0 && <div className="clip-fade out" style={{ width: clip.fadeOut * ppb }} />}
      <div className="clip-edge left" />
      <div className="clip-edge right" />
    </div>
  );
});

