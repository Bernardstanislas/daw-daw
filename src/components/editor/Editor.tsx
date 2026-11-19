// Bottom "Editor" panel: piano roll for MIDI clips, waveform view for audio clips.

import { findClip, findTrack, useStore } from "../../lib/store";
import { PianoRoll } from "./PianoRoll";
import { EditorModeSwitch, StepSequencer } from "./StepSequencer";
import { noteNamesFor } from "../../lib/catalog";
import { ClipCanvas } from "../arrange/ClipCanvas";
import { createMidiClip } from "../../lib/actions";
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
      <div className="editor-hint">Gain and fades are in the inspector · trim the clip edges in the arrangement.</div>
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
  if (found.clip.kind === "audio") return <AudioClipView clipId={found.clip.id} />;
  const mode = editorMode === "auto" ? (noteNamesFor(found.track.instrument?.type).size > 0 ? "steps" : "piano") : editorMode;
  const modeSwitch = <EditorModeSwitch mode={mode} />;
  if (mode === "steps") return <StepSequencer key={found.clip.id} clip={found.clip} track={found.track} modeSwitch={modeSwitch} />;
  return <PianoRoll key={found.clip.id} clip={found.clip} track={found.track} modeSwitch={modeSwitch} />;
}
