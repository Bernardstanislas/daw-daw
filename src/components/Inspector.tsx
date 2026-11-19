// Right panel: selected track (instrument + effect chain), selected clip, master.

import { useState } from "react";
import { beginGesture, endGesture, findClip, findTrack, gesture, draftTrack, draftClip, useStore, commit } from "../lib/store";
import { addEffect, setInstrument, updateClip, updateTrack } from "../lib/actions";
import { deviceName, useCatalog } from "../lib/catalog";
import { DevicePanel } from "./DevicePanel";
import { DragNumber, Knob } from "./controls";
import { MenuButton } from "./Menu";
import { IconPlus } from "./icons";
import { endDrag, readDrag } from "../lib/dnd";
import { formatDb } from "../lib/music";
import type { DeviceState } from "../lib/types";

function EffectChain({ target, effects, color }: { target: string | "master"; effects: DeviceState[]; color?: string }) {
  const specs = useCatalog((s) => s.specs);
  const plugins = useCatalog((s) => s.plugins);
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
        <MenuButton
          className="btn small"
          title="Add an effect"
          items={() => {
            const builtIn = specs
              .filter((s) => s.kind === "effect")
              .map((s) => ({
                label: s.name,
                submenu: [{ name: "Default" }, ...s.presets].map((p) => ({ label: p.name, onClick: () => addEffect(target, s.typeId, p.name) })),
              }));
            const fx = plugins.filter((p) => p.kind === "effect");
            if (!fx.length) return builtIn;
            return [
              ...builtIn,
              { separator: true },
              { label: "Plugins", submenu: fx.map((p) => ({ label: `${p.name} (${p.vendor})`, onClick: () => addEffect(target, p.id) })) },
            ];
          }}
        >
          <IconPlus size={12} /> Add
        </MenuButton>
      </div>
      {effects.length === 0 && <div className="empty-note">No effects. Add one or drag it from the browser.</div>}
      {effects.map((d) => (
        <DevicePanel key={d.id} device={d} target={target} color={color} />
      ))}
    </div>
  );
}

function ClipInspector({ clipId }: { clipId: string }) {
  const foundClip = useStore((s) => findClip(s.project, clipId)?.clip);
  const foundTrack = useStore((s) => findClip(s.project, clipId)?.track);
  const found = foundClip && foundTrack ? { clip: foundClip, track: foundTrack } : undefined;
  if (!found) return null;
  const { clip } = found;
  const set = (recipe: (c: NonNullable<ReturnType<typeof draftClip>>) => void) =>
    gesture((p) => {
      const c = draftClip(p, clipId);
      if (c) recipe(c);
    });
  return (
    <div className="inspector-section">
      <div className="section-title">
        <span>Clip</span>
        <span className="muted-text">{clip.kind === "midi" ? `${clip.notes.length} notes` : "audio"}</span>
      </div>
      <div className="field-row">
        <label>Name</label>
        <input className="text-input" value={clip.name} onChange={(e) => updateClip(clipId, (c) => void (c.name = e.target.value))} onKeyDown={(e) => e.stopPropagation()} />
      </div>
      <div className="field-row">
        <label>Start</label>
        <DragNumber value={clip.start} min={0} max={100000} step={0.25} digits={2} onBegin={beginGesture} onChange={(v) => set((c) => void (c.start = v))} onEnd={endGesture} suffix=" beats" />
      </div>
      <div className="field-row">
        <label>Length</label>
        <DragNumber value={clip.length} min={0.125} max={100000} step={0.25} digits={2} onBegin={beginGesture} onChange={(v) => set((c) => void (c.length = v))} onEnd={endGesture} suffix=" beats" />
      </div>
      {clip.kind === "midi" && (
        <div className="field-row">
          <label>Loop</label>
          <DragNumber
            value={clip.loopLength}
            min={0}
            max={64}
            step={1}
            digits={2}
            onBegin={beginGesture}
            onChange={(v) => set((c) => void (c.loopLength = v))}
            onEnd={endGesture}
            suffix={clip.loopLength > 0 ? " beats" : " (off)"}
            title="Loop length in beats (0 = no loop)"
          />
        </div>
      )}
      {clip.kind === "audio" && (
        <div className="knob-row">
          <Knob value={clip.gain} min={-36} max={12} defaultValue={0} label="Gain" display={`${formatDb(clip.gain)} dB`} onBegin={beginGesture} onChange={(v) => set((c) => void (c.gain = v))} onEnd={endGesture} />
          <Knob value={clip.fadeIn} min={0} max={Math.max(0.01, clip.length / 2)} defaultValue={0} label="Fade in" display={`${clip.fadeIn.toFixed(2)} b`} onBegin={beginGesture} onChange={(v) => set((c) => void (c.fadeIn = v))} onEnd={endGesture} />
          <Knob value={clip.fadeOut} min={0} max={Math.max(0.01, clip.length / 2)} defaultValue={0} label="Fade out" display={`${clip.fadeOut.toFixed(2)} b`} onBegin={beginGesture} onChange={(v) => set((c) => void (c.fadeOut = v))} onEnd={endGesture} />
        </div>
      )}
      <div className="field-row">
        <button className={`toggle-btn ${clip.muted ? "on" : ""}`} onClick={() => updateClip(clipId, (c) => void (c.muted = !c.muted))}>
          Mute clip
        </button>
      </div>
    </div>
  );
}

function MasterInspector() {
  const master = useStore((s) => s.project.master);
  return (
    <>
      <div className="inspector-section">
        <div className="section-title">
          <span>Master</span>
        </div>
        <div className="knob-row">
          <Knob
            value={master.volume}
            min={-60}
            max={6}
            defaultValue={0}
            size={40}
            label="Volume"
            display={`${formatDb(master.volume)} dB`}
            onBegin={beginGesture}
            onChange={(v) => gesture((p) => void (p.master.volume = Math.round(v * 10) / 10))}
            onEnd={endGesture}
          />
        </div>
      </div>
      <EffectChain target="master" effects={master.effects} />
    </>
  );
}

export function Inspector() {
  const [view, setView] = useState<"track" | "master">("track");
  const track = useStore((s) => findTrack(s.project, s.selection.trackId));
  const clipId = useStore((s) => (s.selection.clipIds.length === 1 ? s.selection.clipIds[0] : null));
  const specs = useCatalog((s) => s.specs);
  const plugins = useCatalog((s) => s.plugins);

  return (
    <aside className="inspector panel">
      <div className="panel-tabs">
        <button className={`panel-tab ${view === "track" ? "on" : ""}`} onClick={() => setView("track")}>
          <span>Track</span>
        </button>
        <button className={`panel-tab ${view === "master" ? "on" : ""}`} onClick={() => setView("master")}>
          <span>Master</span>
        </button>
      </div>
      <div className="inspector-body">
        {view === "master" && <MasterInspector />}
        {view === "track" && !track && <div className="empty-note big">Select a track to edit its instrument and effects.</div>}
        {view === "track" && track && (
          <>
            <div className="inspector-section track-summary" style={{ ["--track-color" as string]: track.color }}>
              <div className="track-summary-head">
                <span className="swatch" />
                <input className="text-input title tape" value={track.name} onChange={(e) => updateTrack(track.id, (t) => void (t.name = e.target.value))} onKeyDown={(e) => e.stopPropagation()} />
              </div>
              <div className="knob-row">
                <Knob
                  value={track.volume}
                  min={-60}
                  max={6}
                  defaultValue={0}
                  label="Volume"
                  display={`${formatDb(track.volume)} dB`}
                  onBegin={beginGesture}
                  onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.volume = Math.round(v * 10) / 10))}
                  onEnd={endGesture}
                />
                <Knob
                  value={track.pan}
                  min={-1}
                  max={1}
                  defaultValue={0}
                  label="Pan"
                  display={track.pan === 0 ? "C" : track.pan < 0 ? `L${Math.round(-track.pan * 100)}` : `R${Math.round(track.pan * 100)}`}
                  onBegin={beginGesture}
                  onChange={(v) => gesture((p) => void (draftTrack(p, track.id)!.pan = Math.round(v * 100) / 100))}
                  onEnd={endGesture}
                />
                <div className="flag-col">
                  <button className={`tb mute ${track.mute ? "on" : ""}`} onClick={() => commit((p) => void (draftTrack(p, track.id)!.mute = !track.mute))}>
                    M
                  </button>
                  <button className={`tb solo ${track.solo ? "on" : ""}`} onClick={() => commit((p) => void (draftTrack(p, track.id)!.solo = !track.solo))}>
                    S
                  </button>
                  <button className={`tb arm ${track.armed ? "on" : ""}`} onClick={() => commit((p) => void (draftTrack(p, track.id)!.armed = !track.armed))}>
                    ●
                  </button>
                </div>
              </div>
            </div>
            {clipId && track.clips.some((c) => c.id === clipId) && <ClipInspector clipId={clipId} />}
            {track.kind === "instrument" && (
              <div className="inspector-section">
                <div className="section-title">
                  <span>Instrument</span>
                  <select
                    className="inst-select"
                    value={track.instrument?.type ?? ""}
                    onChange={(e) => setInstrument(track.id, e.target.value)}
                    title="Change instrument"
                  >
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
                </div>
                {track.instrument && <DevicePanel device={track.instrument} target={track.id} isInstrument color={track.color} />}
              </div>
            )}
            <EffectChain target={track.id} effects={track.effects} color={track.color} />
          </>
        )}
      </div>
    </aside>
  );
}
