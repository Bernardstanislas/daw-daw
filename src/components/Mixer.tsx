// Mix page: the console. One channel strip per track plus the master strip,
// over the full height of the window.

import { memo, useLayoutEffect, useRef, useState } from "react";
import type { DeviceState, Track } from "../lib/types";
import { beginGesture, commit, draftTrack, endGesture, gesture, openInSound, select, useStore } from "../lib/store";
import { addEffect, addInstrumentTrack, addTrack, draftDevice, removeEffect, setInstrument, toggleTrackFlag } from "../lib/actions";
import { Fader, Knob, Meter, VuMeter } from "./controls";
import { clamp, formatDb } from "../lib/music";
import { deviceName, useCatalog } from "../lib/catalog";
import { MenuButton, openContextMenu } from "./Menu";
import { addEffectMenu, trackMenu } from "./arrange/menus";
import { endDrag, readDrag } from "../lib/dnd";
import { IconMic, IconPlus, IconSynth } from "./icons";
import { useLearn } from "../lib/midiMap";

/** Insert slots of a strip: click = bypass, right-click = more, "+" = add. */
function Inserts({ target, effects }: { target: string | "master"; effects: DeviceState[] }) {
  // Subscribed so that names refresh once the catalog / plugin scan has loaded.
  useCatalog((s) => s.specs);
  useCatalog((s) => s.plugins);
  const toggle = (id: string) =>
    commit((p) => {
      const d = draftDevice(p, id);
      if (d) d.enabled = !d.enabled;
    });
  return (
    <div className="strip-fx">
      <div className="strip-fx-head">
        <span className="engraved">inserts</span>
        <MenuButton className="icon-btn tiny" title="Add an effect" items={() => addEffectMenu(target)}>
          <IconPlus size={11} />
        </MenuButton>
      </div>
      <div className="strip-fx-list">
        {effects.length === 0 && <div className="strip-fx-empty">drop an effect</div>}
        {effects.map((e) => (
          <button
            key={e.id}
            className={`strip-fx-item ${e.enabled ? "" : "off"}`}
            title={`${deviceName(e)} — click: ${e.enabled ? "bypass" : "enable"} · right-click: more`}
            onClick={() => toggle(e.id)}
            onContextMenu={(ev) => {
              ev.stopPropagation();
              openContextMenu(ev, [
                { label: e.enabled ? "Bypass" : "Enable", onClick: () => toggle(e.id) },
                ...(target === "master" ? [] : [{ label: "Edit on the Sound page", onClick: () => openInSound(target) }]),
                { separator: true },
                { label: "Remove", danger: true, onClick: () => removeEffect(target, e.id) },
              ]);
            }}
          >
            <span className={`lamp ${e.enabled ? "lit" : ""}`} />
            <span className="strip-fx-name">{deviceName(e)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

/** Lets effects (and instruments, on instrument tracks) be dropped from the browser. */
function useDeviceDrop(target: string | "master", acceptsInstrument: boolean) {
  const [hot, setHot] = useState(false);
  return {
    hot,
    handlers: {
      onDragOver: (e: React.DragEvent) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        setHot(true);
      },
      onDragLeave: () => setHot(false),
      onDrop: (e: React.DragEvent) => {
        e.preventDefault();
        setHot(false);
        const p = readDrag(e);
        endDrag();
        if (p?.kind === "effect") addEffect(target, p.type, p.preset);
        else if (p?.kind === "instrument" && acceptsInstrument && target !== "master") setInstrument(target, p.type, p.preset);
      },
    },
  };
}

const Strip = memo(function Strip({ track, selected, faderHeight }: { track: Track; selected: boolean; faderHeight: number }) {
  useCatalog((s) => s.specs);
  const drop = useDeviceDrop(track.id, track.kind === "instrument");
  const learnMute = useLearn({ kind: "track", trackId: track.id, prop: "mute" });
  const learnSolo = useLearn({ kind: "track", trackId: track.id, prop: "solo" });
  const learnArm = useLearn({ kind: "track", trackId: track.id, prop: "armed" });
  const inst = track.instrument;
  const instLabel = inst ? (inst.preset && inst.preset !== "Default" ? inst.preset : deviceName(inst)) : "Audio";
  return (
    <div
      className={`strip ${selected ? "selected" : ""} ${drop.hot ? "hot" : ""}`}
      style={{ ["--track-color" as string]: track.color }}
      onPointerDown={() => select({ trackId: track.id })}
      onContextMenu={(e) => openContextMenu(e, trackMenu(track))}
      {...drop.handlers}
    >
      <div className="scribble" title={`${track.name} — double-click: open on the Sound page`} onDoubleClick={() => openInSound(track.id)}>
        {track.name}
      </div>
      <button className="strip-source" onClick={() => openInSound(track.id)} title={`${instLabel} — open on the Sound page`}>
        {track.kind === "audio" ? <IconMic size={12} /> : <IconSynth size={12} />}
        <span>{instLabel}</span>
      </button>
      <Inserts target={track.id} effects={track.effects} />
      <Knob
        size={34}
        value={track.pan}
        min={-1}
        max={1}
        defaultValue={0}
        display={track.pan === 0 ? "C" : track.pan < 0 ? `L${Math.round(-track.pan * 100)}` : `R${Math.round(track.pan * 100)}`}
        onBegin={beginGesture}
        onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.pan = Math.round(v * 100) / 100))}
        onEnd={endGesture}
        title="Pan"
        learn={{ kind: "track", trackId: track.id, prop: "pan" }}
      />
      <div className="strip-buttons">
        <button className={`tb mute ${track.mute ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "mute")} title="Mute" {...learnMute}>
          M
        </button>
        <button className={`tb solo ${track.solo ? "on" : ""}`} onClick={(e) => toggleTrackFlag(track.id, "solo", e.altKey || e.metaKey)} title="Solo (⌥-click: exclusive)" {...learnSolo}>
          S
        </button>
        <button className={`tb arm ${track.armed ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "armed")} title="Arm for recording" {...learnArm}>
          ●
        </button>
      </div>
      <div className="strip-fader-row" style={{ height: faderHeight + 4 }}>
        <Fader
          db={track.volume}
          height={faderHeight}
          onBegin={beginGesture}
          onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.volume = v))}
          onEnd={endGesture}
          learn={{ kind: "track", trackId: track.id, prop: "volume" }}
        />
        <Meter source={track.armed && track.kind === "audio" ? "input" : track.id} thickness={6} />
      </div>
      <div className="strip-db">{formatDb(track.volume)} dB</div>
    </div>
  );
});

function MasterStrip({ faderHeight }: { faderHeight: number }) {
  const master = useStore((s) => s.project.master);
  const drop = useDeviceDrop("master", false);
  return (
    <div className={`strip master ${drop.hot ? "hot" : ""}`} style={{ ["--track-color" as string]: "var(--accent)" }} {...drop.handlers}>
      <div className="scribble master">Master</div>
      <div className="master-vus">
        <VuMeter source="master" channel={0} width={96} label="L" />
        <VuMeter source="master" channel={1} width={96} label="R" />
      </div>
      <Inserts target="master" effects={master.effects} />
      <div className="strip-fader-row" style={{ height: faderHeight + 4 }}>
        <Fader db={master.volume} height={faderHeight} onBegin={beginGesture} onChange={(v) => gesture((p) => void (p.master.volume = v))} onEnd={endGesture} learn={{ kind: "master" }} />
        <Meter source="master" thickness={8} />
      </div>
      <div className="strip-db">{formatDb(master.volume)} dB</div>
    </div>
  );
}

export function Mixer() {
  const tracks = useStore((s) => s.project.tracks);
  const sel = useStore((s) => s.selection.trackId);
  const ref = useRef<HTMLDivElement>(null);
  // Faders grow with the window; the insert lists take what is left.
  const [faderHeight, setFaderHeight] = useState(220);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setFaderHeight(Math.round(clamp(el.clientHeight * 0.42, 120, 340)));
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);
  return (
    <div className="mixer" ref={ref}>
      <div className="mixer-strips">
        {tracks.map((t) => (
          <Strip key={t.id} track={t} selected={sel === t.id} faderHeight={faderHeight} />
        ))}
        <div className="mixer-add">
          <button className="btn small" onClick={() => addInstrumentTrack("pulse")} title="Add an instrument track">
            <IconSynth /> Instrument
          </button>
          <button className="btn small" onClick={() => addTrack("audio")} title="Add an audio track">
            <IconMic /> Audio
          </button>
          {tracks.length === 0 && <div className="empty-note">No tracks yet.</div>}
        </div>
      </div>
      <MasterStrip faderHeight={faderHeight} />
    </div>
  );
}
