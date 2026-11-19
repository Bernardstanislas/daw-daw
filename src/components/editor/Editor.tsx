// Bottom "Editor" panel: the settings of the open clip at the left, then the piano
// roll or step grid for MIDI clips, the waveform for audio clips.

import { beginGesture, draftClip, endGesture, findClip, findTrack, gesture, useStore } from "../../lib/store";
import { PianoRoll } from "./PianoRoll";
import { EditorModeSwitch, StepSequencer } from "./StepSequencer";
import { noteNamesFor } from "../../lib/catalog";
import { ClipCanvas } from "../arrange/ClipCanvas";
import { createMidiClip, updateClip } from "../../lib/actions";
import { DragNumber, Knob } from "../controls";
import { formatDb } from "../../lib/music";
import type { Clip } from "../../lib/types";
import { livePosition } from "../../lib/engine";
import { beatsPerBar } from "../../lib/music";
import { useLayoutEffect, useRef, useState } from "react";

function AudioClipView({ clipId }: { clipId: string }) {
  const foundClip = useStore((s) => findClip(s.project, clipId)?.clip);
  const foundTrack = useStore((s) => findClip(s.project, clipId)?.track);
  const found = foundClip && foundTrack ? { clip: foundClip, track: foundTrack } : undefined;
  const asset = useStore((s) => (found?.clip.assetId ? s.assets[found.clip.assetId] : undefined));
  const bpm = useStore((s) => s.project.bpm);
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 800, h: 200 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  if (!found) return null;
  const { clip, track } = found;
  const ppb = size.w / Math.max(0.25, clip.length);
  return (
    <div className="audio-editor">
      <div className="editor-toolbar">
        <span className="editor-title" style={{ ["--track-color" as string]: track.color }}>
          <span className="swatch" /> {track.name} › {clip.name}
        </span>
        <div className="spacer" />
        {asset && (
          <span className="muted-text">
            {asset.path.split("/").pop()} · {asset.duration.toFixed(2)} s · {asset.sampleRate} Hz · {asset.channels === 1 ? "mono" : "stereo"}
          </span>
        )}
      </div>
      <div className="audio-editor-wave" ref={ref} style={{ ["--clip-color" as string]: track.color }}>
        {asset ? (
          <ClipCanvas clip={clip} width={size.w} height={size.h} pxPerBeat={ppb} bpm={bpm} asset={asset} color={track.color} />
        ) : (
          <div className="empty-note big">Audio file not loaded.</div>
        )}
      </div>
      <div className="editor-hint">Trim the clip edges in the arrangement.</div>
    </div>
  );
}

/** Settings of the clip open in the editor: name, position, loop (MIDI), gain and fades (audio). */
function ClipPanel({ clip }: { clip: Clip }) {
  const set = (recipe: (c: NonNullable<ReturnType<typeof draftClip>>) => void) =>
    gesture((p) => {
      const c = draftClip(p, clip.id);
      if (c) recipe(c);
    });
  return (
    <div className="clip-panel">
      <div className="section-title">
        <span>Clip</span>
        <span className="muted-text">{clip.kind === "midi" ? `${clip.notes.length} notes` : "audio"}</span>
      </div>
      <input className="text-input" value={clip.name} onChange={(e) => updateClip(clip.id, (c) => void (c.name = e.target.value))} onKeyDown={(e) => e.stopPropagation()} title="Clip name" />
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
      <button className={`toggle-btn ${clip.muted ? "on" : ""}`} onClick={() => updateClip(clip.id, (c) => void (c.muted = !c.muted))}>
        Mute clip
      </button>
    </div>
  );
}

export function Editor() {
  const editorMode = useStore((s) => s.ui.editorMode);
  const editorClipId = useStore((s) => s.editorClipId);
  const foundClip = useStore((s) => findClip(s.project, s.editorClipId)?.clip);
  const foundTrack = useStore((s) => findClip(s.project, s.editorClipId)?.track);
  const found = foundClip && foundTrack ? { clip: foundClip, track: foundTrack } : undefined;
  const selTrack = useStore((s) => findTrack(s.project, s.selection.trackId));
  const bpb = useStore((s) => beatsPerBar(s.project.timeSigNum, s.project.timeSigDen));
  if (!editorClipId || !found) {
    return (
      <div className="editor-empty">
        <div>
          <b>No clip open.</b> Double-click a clip to edit it here.
        </div>
        {selTrack?.kind === "instrument" && (
          <button className="btn" onClick={() => createMidiClip(selTrack.id, Math.floor(livePosition() / bpb) * bpb, bpb * 2)}>
            Create a 2-bar MIDI clip on “{selTrack.name}” at the playhead
          </button>
        )}
      </div>
    );
  }
  const mode = editorMode === "auto" ? (noteNamesFor(found.track.instrument?.type).size > 0 ? "steps" : "piano") : editorMode;
  const modeSwitch = <EditorModeSwitch mode={mode} />;
  return (
    <div className="editor-split">
      <ClipPanel clip={found.clip} />
      <div className="editor-main">
        {found.clip.kind === "audio" ? (
          <AudioClipView clipId={found.clip.id} />
        ) : mode === "steps" ? (
          <StepSequencer key={found.clip.id} clip={found.clip} track={found.track} modeSwitch={modeSwitch} />
        ) : (
          <PianoRoll key={found.clip.id} clip={found.clip} track={found.track} modeSwitch={modeSwitch} />
        )}
      </div>
    </div>
  );
}
