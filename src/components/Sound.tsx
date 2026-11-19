// Sound page: one track at a time, its instrument and its effect chain laid out
// over the whole window, with a keybed to play it.

import { useLayoutEffect, useRef, useState } from "react";
import type { DeviceState, Track } from "../lib/types";
import { select, useStore } from "../lib/store";
import { addEffect, addInstrumentTrack, addTrack, setInstrument, toggleTrackFlag, updateTrack } from "../lib/actions";
import { liveNote, useTransport } from "../lib/engine";
import { deviceName, noteNamesFor, useCatalog } from "../lib/catalog";
import { capturePointer, noteName } from "../lib/music";
import { endDrag, readDrag } from "../lib/dnd";
import { DevicePanel } from "./DevicePanel";
import { MenuButton } from "./Menu";
import { addEffectMenu } from "./arrange/menus";
import { Meter } from "./controls";
import { IconMic, IconPlus, IconSynth } from "./icons";
import { promptText } from "./Prompt";

const WHITE_W = 30;
const WHITE_STEPS = [0, 2, 4, 5, 7, 9, 11];
const BLACK_AFTER: Record<number, number> = { 0: 1, 1: 3, 3: 6, 4: 8, 5: 10 };

/** Plays the pitch under the pointer for as long as it is held; sliding moves to the next key. */
function useKeyPlay(trackId: string) {
  const held = useRef<number | null>(null);
  const play = (pitch: number | null) => {
    if (pitch === held.current) return;
    if (held.current !== null) liveNote(held.current, 0, false, trackId);
    held.current = pitch;
    if (pitch !== null) liveNote(pitch, 0.8, true, trackId);
  };
  const pitchAt = (x: number, y: number) => {
    const el = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>("[data-pitch]");
    return el ? Number(el.dataset.pitch) : null;
  };
  return {
    onPointerDown: (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      capturePointer(e.currentTarget, e.pointerId);
      play(pitchAt(e.clientX, e.clientY));
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (held.current !== null) play(pitchAt(e.clientX, e.clientY));
    },
    onPointerUp: () => play(null),
    onPointerCancel: () => play(null),
  };
}

/** Piano keys for melodic instruments, named pads for drum kits. */
function Keybed({ track }: { track: Track }) {
  // Subscribed so that the pads appear once the catalog has loaded.
  useCatalog((s) => s.specs);
  const live = useTransport((s) => s.liveNotes);
  const handlers = useKeyPlay(track.id);
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(900);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  const names = noteNamesFor(track.instrument?.type);
  if (names.size > 0) {
    return (
      <div className="keybed pads" ref={ref} {...handlers}>
        {[...names.entries()].map(([pitch, name]) => (
          <div key={pitch} data-pitch={pitch} className={`pad ${live.has(pitch) ? "down" : ""}`}>
            {name}
          </div>
        ))}
      </div>
    );
  }

  // As many octaves as fit, centred around middle C.
  const octaves = Math.max(2, Math.min(7, Math.floor((width - 24 - WHITE_W) / (7 * WHITE_W))));
  const first = 60 - 12 * Math.ceil(octaves / 2);
  const whites: number[] = [];
  for (let o = 0; o < octaves; o++) for (const s of WHITE_STEPS) whites.push(first + o * 12 + s);
  whites.push(first + octaves * 12);
  return (
    <div className="keybed" ref={ref} {...handlers}>
      <div className="keys" style={{ width: whites.length * WHITE_W }}>
        {whites.map((p) => (
          <div key={p} data-pitch={p} className={`key white ${live.has(p) ? "down" : ""}`}>
            {p % 12 === 0 && <span>{noteName(p)}</span>}
          </div>
        ))}
        {whites.slice(0, -1).map((_, i) => {
          const black = BLACK_AFTER[i % 7];
          if (black === undefined) return null;
          const pitch = first + Math.floor(i / 7) * 12 + black;
          return <div key={pitch} data-pitch={pitch} className={`key black ${live.has(pitch) ? "down" : ""}`} style={{ left: (i + 1) * WHITE_W - 9 }} />;
        })}
      </div>
    </div>
  );
}

/** Effect chain of a track or of the master: drop target, "Add" menu, one panel per effect. */
function EffectChain({ target, effects }: { target: string | "master"; effects: DeviceState[] }) {
  const [hot, setHot] = useState(false);
  return (
    <div
      className={`fx-chain ${hot ? "hot" : ""}`}
      onDragOver={(e) => {
        e.preventDefault();
        setHot(true);
      }}
      onDragLeave={() => setHot(false)}
      onDrop={(e) => {
        e.preventDefault();
        setHot(false);
        const p = readDrag(e);
        endDrag();
        if (p?.kind === "effect") addEffect(target, p.type, p.preset);
      }}
    >
      <div className="section-title">
        <span>Effects</span>
        <MenuButton className="btn small" title="Add an effect" items={() => addEffectMenu(target)}>
          <IconPlus size={12} /> Add
        </MenuButton>
      </div>
      {effects.length === 0 && <div className="empty-note">No effects. Add one or drag it from the browser.</div>}
      {effects.map((d) => (
        <DevicePanel key={d.id} device={d} target={target} />
      ))}
    </div>
  );
}

/** Picks the instrument of a track: built-ins, installed plugins, or the missing one it was saved with. */
function InstrumentSelect({ track }: { track: Track }) {
  const specs = useCatalog((s) => s.specs);
  const plugins = useCatalog((s) => s.plugins);
  return (
    <select className="inst-select" value={track.instrument?.type ?? ""} onChange={(e) => setInstrument(track.id, e.target.value)} title="Change instrument">
      {specs
        .filter((s) => s.kind === "instrument")
        .map((s) => (
          <option key={s.typeId} value={s.typeId}>
            {s.name}
          </option>
        ))}
      {plugins.some((p) => p.kind === "instrument") && (
        <optgroup label="Plugins">
          {plugins
            .filter((p) => p.kind === "instrument")
            .map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} ({p.vendor})
              </option>
            ))}
        </optgroup>
      )}
      {track.instrument && !specs.some((s) => s.typeId === track.instrument!.type) && !plugins.some((p) => p.id === track.instrument!.type) && (
        <option value={track.instrument.type}>{deviceName(track.instrument)} (not installed)</option>
      )}
    </select>
  );
}

/** Mute / solo and level of the track being worked on, at the right of its first section title. */
function TrackMonitor({ track }: { track: Track }) {
  return (
    <div className="sound-monitor">
      <div className="sound-monitor-meter">
        <Meter source={track.id} vertical={false} thickness={4} />
      </div>
      <button className={`tb mute ${track.mute ? "on" : ""}`} onClick={() => toggleTrackFlag(track.id, "mute")} title="Mute">
        M
      </button>
      <button className={`tb solo ${track.solo ? "on" : ""}`} onClick={(e) => toggleTrackFlag(track.id, "solo", e.altKey || e.metaKey)} title="Solo (⌥-click: exclusive)">
        S
      </button>
    </div>
  );
}

function TrackSound({ track }: { track: Track }) {
  const [hot, setHot] = useState(false);
  return (
    <>
      <div className="sound-body">
        {track.kind === "instrument" ? (
          <div
            className={`sound-section ${hot ? "hot" : ""}`}
            onDragOver={(e) => {
              e.preventDefault();
              setHot(true);
            }}
            onDragLeave={() => setHot(false)}
            onDrop={(e) => {
              e.preventDefault();
              setHot(false);
              const p = readDrag(e);
              if (p?.kind !== "instrument") return;
              endDrag();
              setInstrument(track.id, p.type, p.preset);
            }}
          >
            <div className="section-title">
              <span>Instrument</span>
              <TrackMonitor track={track} />
              <InstrumentSelect track={track} />
            </div>
            {track.instrument && <DevicePanel device={track.instrument} target={track.id} isInstrument />}
          </div>
        ) : (
          <div className="sound-section">
            <div className="section-title">
              <span>Source</span>
              <TrackMonitor track={track} />
            </div>
            <div className="empty-note">Audio track: the sound comes from its clips. The effects below shape it.</div>
          </div>
        )}
        <EffectChain target={track.id} effects={track.effects} />
      </div>
      {track.kind === "instrument" && <Keybed key={track.id} track={track} />}
    </>
  );
}

export function Sound() {
  const tracks = useStore((s) => s.project.tracks);
  const selId = useStore((s) => s.selection.trackId);
  const masterFx = useStore((s) => s.project.master.effects);
  const [master, setMaster] = useState(false);
  const track = tracks.find((t) => t.id === selId) ?? tracks[0];
  const showMaster = master || !track;
  return (
    <div className="sound">
      <div className="sound-tracks">
        <span className="engraved">track</span>
        {tracks.map((t) => (
          <button
            key={t.id}
            className={`sound-chip ${!showMaster && t.id === track?.id ? "on" : ""}`}
            style={{ ["--track-color" as string]: t.color }}
            onClick={() => {
              select({ trackId: t.id });
              setMaster(false);
            }}
            onDoubleClick={async () => {
              const n = await promptText("Track name", t.name);
              if (n) updateTrack(t.id, (x) => void (x.name = n));
            }}
            title={`${t.name} — double-click to rename`}
          >
            <span className="chip-dot" />
            {t.name}
          </button>
        ))}
        <button
          className="icon-btn tiny"
          onClick={() => {
            setMaster(false);
            addInstrumentTrack("pulse");
          }}
          title="Add an instrument track"
        >
          <IconSynth size={12} />
        </button>
        <button
          className="icon-btn tiny"
          onClick={() => {
            setMaster(false);
            addTrack("audio");
          }}
          title="Add an audio track"
        >
          <IconMic size={12} />
        </button>
        <div className="spacer" />
        <button className={`sound-chip master ${showMaster ? "on" : ""}`} onClick={() => setMaster(true)} title="Master effect chain">
          Master
        </button>
      </div>
      {showMaster ? (
        <div className="sound-body">
          <EffectChain target="master" effects={masterFx} />
        </div>
      ) : (
        <TrackSound track={track} />
      )}
    </div>
  );
}
