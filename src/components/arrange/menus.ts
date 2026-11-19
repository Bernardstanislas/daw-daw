// Context menus for clips and tracks.

import type { MenuItem } from "../Menu";
import type { Clip, Track } from "../../lib/types";
import {
  copyClips,
  cutClips,
  deleteClips,
  deleteTrack,
  duplicateClips,
  duplicateTrack,
  splitClipsAt,
  toggleTrackFlag,
  updateClip,
  updateTrack,
} from "../../lib/actions";
import { commit, TRACK_COLORS, useStore } from "../../lib/store";
import { livePosition } from "../../lib/engine";
import { promptText } from "../Prompt";

const mod = navigator.platform.includes("Mac") ? "⌘" : "Ctrl+";

export function colorSubmenu(apply: (c: string | undefined) => void, withDefault = false): MenuItem[] {
  const items: MenuItem[] = TRACK_COLORS.map((c) => ({
    label: `● ${c}`,
    onClick: () => apply(c),
  }));
  if (withDefault) items.unshift({ label: "Track color", onClick: () => apply(undefined) });
  return items;
}

export function clipMenu(clip: Clip): MenuItem[] {
  const ids = () => useStore.getState().selection.clipIds;
  return [
    {
      label: "Open in editor",
      onClick: () => useStore.setState((s) => ({ editorClipId: clip.id, ui: { ...s.ui, bottom: "editor" } })),
    },
    {
      label: "Rename…",
      onClick: async () => {
        const name = await promptText("Clip name", clip.name);
        if (name) updateClip(clip.id, (c) => void (c.name = name));
      },
    },
    { separator: true },
    { label: "Cut", shortcut: `${mod}X`, onClick: () => cutClips(ids()) },
    { label: "Copy", shortcut: `${mod}C`, onClick: () => copyClips(ids()) },
    { label: "Duplicate", shortcut: `${mod}D`, onClick: () => duplicateClips(ids()) },
    { label: "Split at playhead", shortcut: "S", onClick: () => splitClipsAt(ids(), livePosition()) },
    { separator: true },
    {
      label: clip.muted ? "Unmute" : "Mute",
      shortcut: "M",
      onClick: () =>
        commit((p) => {
          const set = new Set(ids());
          for (const t of p.tracks) for (const c of t.clips) if (set.has(c.id)) c.muted = !clip.muted;
        }),
    },
    ...(clip.kind === "midi"
      ? [
          {
            label: clip.loopLength > 0 ? "Disable looping" : "Loop content",
            onClick: () =>
              updateClip(clip.id, (c) => {
                if (c.loopLength > 0) c.loopLength = 0;
                else {
                  const end = Math.max(1, ...c.notes.map((n) => n.start + n.length));
                  c.loopLength = Math.min(c.length, Math.ceil(end / 4) * 4) || c.length;
                }
              }),
          } as MenuItem,
        ]
      : []),
    { label: "Color", submenu: colorSubmenu((col) => updateClip(clip.id, (c) => void (c.color = col)), true) },
    { separator: true },
    { label: "Delete", shortcut: "⌫", danger: true, onClick: () => deleteClips(ids()) },
  ];
}

export function trackMenu(track: Track): MenuItem[] {
  return [
    {
      label: "Rename…",
      onClick: async () => {
        const name = await promptText("Track name", track.name);
        if (name) updateTrack(track.id, (t) => void (t.name = name));
      },
    },
    { label: "Duplicate track", onClick: () => duplicateTrack(track.id) },
    { label: "Color", submenu: colorSubmenu((c) => c && updateTrack(track.id, (t) => void (t.color = c))) },
    { separator: true },
    { label: track.mute ? "Unmute" : "Mute", onClick: () => toggleTrackFlag(track.id, "mute") },
    { label: track.solo ? "Unsolo" : "Solo", onClick: () => toggleTrackFlag(track.id, "solo") },
    { label: track.armed ? "Disarm" : "Arm for recording", onClick: () => toggleTrackFlag(track.id, "armed") },
    { separator: true },
    { label: "Delete track", danger: true, onClick: () => deleteTrack(track.id) },
  ];
}
