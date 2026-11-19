// Track header: name, instrument, mute / solo / arm, volume, meter, reorder & resize.

import { memo, useState } from "react";
import type { Track } from "../../lib/types";
import { beginGesture, endGesture, gesture, draftTrack, openInSound, select, useStore, silent } from "../../lib/store";
import { moveTrack, toggleTrackFlag, updateTrack } from "../../lib/actions";
import { deviceName, useCatalog } from "../../lib/catalog";
import { Knob, Meter } from "../controls";
import { openContextMenu } from "../Menu";
import { trackMenu } from "./menus";
import { promptText } from "../Prompt";
import { IconDrum, IconMic, IconPiano, IconSynth } from "../icons";
import { trackIndexAtClientY } from "./geom";
import { formatDb, capturePointer } from "../../lib/music";

function TrackIcon({ track }: { track: Track }) {
  if (track.kind === "audio") return <IconMic />;
  if (track.instrument?.type === "kit" || track.instrument?.type === "samplekit") return <IconDrum />;
  if (track.instrument?.type === "keys" || track.instrument?.type === "sampler") return <IconPiano />;
  return <IconSynth />;
}

export const TrackHeader = memo(function TrackHeader({ track, index, selected }: { track: Track; index: number; selected: boolean }) {
  const specs = useCatalog((s) => s.specs);
  const instName = track.instrument
    ? (track.instrument.preset && track.instrument.preset !== "Default"
        ? track.instrument.preset
        : specs.find((s) => s.typeId === track.instrument!.type)?.name) ?? deviceName(track.instrument)
    : "Audio";
  const [dropLine, setDropLine] = useState<null | "above" | "below">(null);

  const onGripDown = (e: React.PointerEvent) => {
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    let target = index;
    const move = (ev: PointerEvent) => {
      const i = trackIndexAtClientY(ev.clientY);
      target = Math.max(0, Math.min(useStore.getState().project.tracks.length - 1, i));
      setDropLine(target < index ? "above" : target > index ? "below" : null);
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      setDropLine(null);
      if (target !== index) moveTrack(track.id, target);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const onResizeDown = (e: React.PointerEvent) => {
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const y0 = e.clientY;
    const h0 = track.height;
    const move = (ev: PointerEvent) => {
      const h = Math.max(40, Math.min(240, h0 + ev.clientY - y0));
      silent((p) => {
        const t = draftTrack(p, track.id);
        if (t) t.height = h;
      });
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const compact = track.height < 60;
  const hasAuto = !!(track.automation?.volume?.length || track.automation?.pan?.length);

  return (
    <div
      className={`track-header ${selected ? "selected" : ""} ${dropLine ?? ""}`}
      style={{ height: track.height, ["--track-color" as string]: track.color }}
      onPointerDown={() => select({ trackId: track.id })}
      onContextMenu={(e) => openContextMenu(e, trackMenu(track))}
    >
      <div className="track-color" onPointerDown={onGripDown} title="Drag to reorder" />
      <div className="track-main">
        <div className="track-row">
          <span className="track-icon">
            <TrackIcon track={track} />
          </span>
          <span
            className="track-name tape"
            onDoubleClick={async () => {
              const n = await promptText("Track name", track.name);
              if (n) updateTrack(track.id, (t) => void (t.name = n));
            }}
            title={`${track.name} — double-click to rename`}
          >
            {track.name}
          </span>
          <div className="track-buttons">
            <button className={`tb mute ${track.mute ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "mute")} title="Mute">
              M
            </button>
            <button
              className={`tb solo ${track.solo ? "on" : ""}`}
              onClick={(e) => toggleTrackFlag(track.id, "solo", e.altKey || e.metaKey)}
              title="Solo (⌥-click: exclusive)"
            >
              S
            </button>
            <button className={`tb arm ${track.armed ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "armed")} title="Arm for recording">
              ●
            </button>
            <button
              className={`tb auto ${track.showAutomation ? "on" : ""} ${hasAuto ? "has" : ""}`}
              onClick={() =>
                silent((p) => {
                  const t = draftTrack(p, track.id);
                  if (t) t.showAutomation = t.showAutomation === "volume" ? "pan" : t.showAutomation === "pan" ? null : "volume";
                })
              }
              title={`Automation: ${track.showAutomation ?? "hidden"} (click: volume → pan → hide)`}
            >
              {track.showAutomation === "pan" ? "P" : "A"}
            </button>
          </div>
        </div>
        {!compact && (
          <div className="track-row second">
            <span className="track-inst" title={`${instName} — double-click: edit on the Sound page`} onDoubleClick={() => openInSound(track.id)}>
              {instName}
            </span>
            <Knob
              size={20}
              value={track.volume}
              min={-60}
              max={6}
              defaultValue={0}
              onBegin={beginGesture}
              onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.volume = Math.round(v * 10) / 10))}
              onEnd={endGesture}
              title={`Volume ${formatDb(track.volume)} dB`}
            />
            <Knob
              size={20}
              value={track.pan}
              min={-1}
              max={1}
              defaultValue={0}
              onBegin={beginGesture}
              onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.pan = Math.round(v * 100) / 100))}
              onEnd={endGesture}
              title={`Pan ${track.pan === 0 ? "C" : track.pan < 0 ? `L${Math.round(-track.pan * 100)}` : `R${Math.round(track.pan * 100)}`}`}
            />
          </div>
        )}
      </div>
      <div className="track-meter">
        <Meter source={track.armed && track.kind === "audio" ? "input" : track.id} thickness={3} />
      </div>
      <div className="track-resize" onPointerDown={onResizeDown} />
    </div>
  );
});

