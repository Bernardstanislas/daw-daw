// Mixer: one channel strip per track plus the master strip.

import { memo } from "react";
import type { Track } from "../lib/types";
import { beginGesture, draftTrack, endGesture, gesture, select, useStore } from "../lib/store";
import { toggleTrackFlag } from "../lib/actions";
import { Fader, Knob, Meter, VuMeter } from "./controls";
import { formatDb } from "../lib/music";
import { deviceName, useCatalog } from "../lib/catalog";
import { openContextMenu } from "./Menu";
import { trackMenu } from "./arrange/menus";

const Strip = memo(function Strip({ track, selected }: { track: Track; selected: boolean }) {
  const specs = useCatalog((s) => s.specs);
  const fxNames = track.effects.map((e) => ({ id: e.id, name: specs.find((s) => s.typeId === e.type)?.name ?? deviceName(e), on: e.enabled }));
  return (
    <div
      className={`strip ${selected ? "selected" : ""}`}
      style={{ ["--track-color" as string]: track.color }}
      onPointerDown={() => select({ trackId: track.id })}
      onContextMenu={(e) => openContextMenu(e, trackMenu(track))}
    >
      <div className="scribble" title={track.name}>
        {track.name}
      </div>
      <div className="strip-fx">
        {fxNames.length === 0 && <div className="strip-fx-empty">no fx</div>}
        {fxNames.map((f) => (
          <div key={f.id} className={`strip-fx-item ${f.on ? "" : "off"}`} title={f.name}>
            {f.name}
          </div>
        ))}
      </div>
      <Knob
        size={30}
        value={track.pan}
        min={-1}
        max={1}
        defaultValue={0}
        onBegin={beginGesture}
        onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.pan = Math.round(v * 100) / 100))}
        onEnd={endGesture}
        title="Pan"
      />
      <div className="strip-buttons">
        <button className={`tb mute ${track.mute ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "mute")}>
          M
        </button>
        <button className={`tb solo ${track.solo ? "on" : ""}`} onClick={(e) => toggleTrackFlag(track.id, "solo", e.altKey)}>
          S
        </button>
        <button className={`tb arm ${track.armed ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "armed")}>
          ●
        </button>
      </div>
      <div className="strip-fader-row">
        <Fader
          db={track.volume}
          height={130}
          onBegin={beginGesture}
          onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.volume = v))}
          onEnd={endGesture}
        />
        <Meter source={track.armed && track.kind === "audio" ? "input" : track.id} thickness={5} />
      </div>
      <div className="strip-db">{formatDb(track.volume)} dB</div>
    </div>
  );
});

export function Mixer() {
  const tracks = useStore((s) => s.project.tracks);
  const master = useStore((s) => s.project.master);
  const sel = useStore((s) => s.selection.trackId);
  const specs = useCatalog((s) => s.specs);
  return (
    <div className="mixer">
      <div className="mixer-strips">
        {tracks.map((t) => (
          <Strip key={t.id} track={t} selected={sel === t.id} />
        ))}
        {tracks.length === 0 && <div className="empty-note big">No tracks yet.</div>}
      </div>
      <div className="strip master" style={{ ["--track-color" as string]: "var(--accent)" }}>
        <div className="scribble master">Master</div>
        <div className="master-vus">
          <VuMeter source="master" channel={0} width={96} label="L" />
          <VuMeter source="master" channel={1} width={96} label="R" />
        </div>
        <div className="strip-fx">
          {master.effects.length === 0 && <div className="strip-fx-empty">no fx</div>}
          {master.effects.map((e) => (
            <div key={e.id} className={`strip-fx-item ${e.enabled ? "" : "off"}`}>
              {specs.find((s) => s.typeId === e.type)?.name ?? deviceName(e)}
            </div>
          ))}
        </div>
        <div className="strip-fader-row">
          <Fader db={master.volume} height={130} onBegin={beginGesture} onChange={(v) => gesture((p) => void (p.master.volume = v))} onEnd={endGesture} />
          <Meter source="master" thickness={6} />
        </div>
        <div className="strip-db">{formatDb(master.volume)} dB</div>
      </div>
    </div>
  );
}
