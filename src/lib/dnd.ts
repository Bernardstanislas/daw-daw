// Drag & drop payloads from the browser panel to the arrangement, the Sound page and the mixer.

import { findPattern, patternNotes } from "./patterns";
import { addInstrumentTrack, addEffect, importAudioFiles, makeMidiClip, setInstrument } from "./actions";
import { commit, draftTrack, findTrack, select, toast, useStore } from "./store";
import { getPlugin, getSpec } from "./catalog";

export type DragPayload =
  | { kind: "instrument"; type: string; preset?: string }
  | { kind: "effect"; type: string; preset?: string }
  | { kind: "pattern"; id: string }
  | { kind: "file"; path: string };

let current: DragPayload | null = null;

export function startDrag(e: React.DragEvent, payload: DragPayload) {
  current = payload;
  e.dataTransfer.effectAllowed = "copy";
  e.dataTransfer.setData("text/plain", JSON.stringify(payload));
}

export function endDrag() {
  current = null;
}

export function currentDrag(): DragPayload | null {
  return current;
}

export function readDrag(e: React.DragEvent): DragPayload | null {
  if (current) return current;
  try {
    return JSON.parse(e.dataTransfer.getData("text/plain"));
  } catch {
    return null;
  }
}

/** Handles a drop on the arrangement. `trackId` null = empty area (new track). */
export function dropOnArrange(payload: DragPayload, trackId: string | null, beat: number) {
  const p = useStore.getState().project;
  const track = findTrack(p, trackId);
  switch (payload.kind) {
    case "instrument": {
      if (track && track.kind === "instrument") {
        setInstrument(track.id, payload.type, payload.preset);
        const spec = getSpec(payload.type);
        toast(`${track.name}: ${payload.preset && payload.preset !== "Default" ? payload.preset : (spec?.name ?? getPlugin(payload.type)?.name)}`);
      } else {
        addInstrumentTrack(payload.type, payload.preset);
      }
      break;
    }
    case "effect": {
      if (track) addEffect(track.id, payload.type, payload.preset);
      else toast("Drop effects on a track (or on the Sound page).");
      break;
    }
    case "pattern": {
      const pat = findPattern(payload.id);
      if (!pat) return;
      let tid = track && track.kind === "instrument" ? track.id : null;
      if (!tid) tid = addInstrumentTrack(pat.instrument.type, pat.instrument.preset);
      const clip = makeMidiClip(Math.max(0, beat), pat.length, patternNotes(pat), pat.name);
      clip.loopLength = pat.length;
      commit((d) => {
        draftTrack(d, tid!)?.clips.push(clip);
      });
      select({ trackId: tid, clipIds: [clip.id] });
      useStore.setState({ editorClipId: clip.id });
      break;
    }
    case "file": {
      importAudioFiles([payload.path], track?.kind === "audio" ? track.id : null, Math.max(0, beat));
      break;
    }
  }
}
