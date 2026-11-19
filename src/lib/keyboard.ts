// Global keyboard shortcuts and "musical typing" (computer keyboard as a MIDI keyboard).

import { commit, redo, setUi, undo, useStore } from "./store";
import { livePosition, liveNote, record, seek, togglePlay, useTransport } from "./engine";
import { copyClips, cutClips, deleteClips, duplicateClips, pasteClips, splitClipsAt, deleteTrack } from "./actions";
import { exportMixdown, newProject, openProject, saveProject } from "./fileOps";
import { clamp } from "./music";
import { backend } from "./backend";
import { create } from "zustand";

const TYPING_KEYS: Record<string, number> = {
  a: 0, w: 1, s: 2, e: 3, d: 4, f: 5, t: 6, g: 7, y: 8, h: 9, u: 10, j: 11, k: 12, o: 13, l: 14, p: 15, ";": 16,
};

export const useTyping = create<{ octave: number; velocity: number }>(() => ({ octave: 4, velocity: 100 }));

const held = new Map<string, number>();

function isEditable(t: EventTarget | null) {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}

function typingActive(e: KeyboardEvent) {
  return useStore.getState().ui.typing || e.getModifierState?.("CapsLock");
}

function onKeyDown(e: KeyboardEvent) {
  if (isEditable(e.target)) return;
  if (useStore.getState().dialog) return;
  const meta = e.metaKey || e.ctrlKey;
  const key = e.key.toLowerCase();

  // ---- musical typing
  if (!meta && typingActive(e)) {
    if (key in TYPING_KEYS) {
      e.preventDefault();
      if (e.repeat || held.has(key)) return;
      const { octave, velocity } = useTyping.getState();
      const pitch = clamp((octave + 1) * 12 + TYPING_KEYS[key], 0, 127);
      held.set(key, pitch);
      liveNote(pitch, velocity / 127, true);
      return;
    }
    if (key === "z" || key === "x") {
      e.preventDefault();
      useTyping.setState((s) => ({ octave: clamp(s.octave + (key === "x" ? 1 : -1), 0, 8) }));
      return;
    }
    if (key === "c" || key === "v") {
      e.preventDefault();
      useTyping.setState((s) => ({ velocity: clamp(s.velocity + (key === "v" ? 20 : -20), 7, 127) }));
      return;
    }
  }

  const s = useStore.getState();
  const sel = s.selection.clipIds;

  if (meta) {
    switch (key) {
      case "z":
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
        return;
      case "y":
        e.preventDefault();
        redo();
        return;
      case "s":
        e.preventDefault();
        saveProject(e.shiftKey);
        return;
      case "o":
        e.preventDefault();
        openProject();
        return;
      case "n":
        e.preventDefault();
        useStore.setState({ dialog: "new" });
        void newProject;
        return;
      case "e":
        e.preventDefault();
        useStore.setState({ dialog: "export" });
        void exportMixdown;
        return;
      case ",":
        e.preventDefault();
        useStore.setState({ dialog: "settings" });
        return;
      case "c":
        if (sel.length) {
          e.preventDefault();
          copyClips(sel);
        }
        return;
      case "x":
        if (sel.length) {
          e.preventDefault();
          cutClips(sel);
        }
        return;
      case "v":
        e.preventDefault();
        pasteClips(Math.max(0, livePosition()));
        return;
      case "d":
        if (sel.length) {
          e.preventDefault();
          duplicateClips(sel);
        }
        return;
      case "a": {
        e.preventDefault();
        useStore.setState((st) => ({ selection: { ...st.selection, clipIds: st.project.tracks.flatMap((t) => t.clips.map((c) => c.id)) } }));
        return;
      }
    }
    return;
  }

  switch (e.key) {
    case " ":
      e.preventDefault();
      togglePlay();
      return;
    case "Home":
    case "Enter":
      e.preventDefault();
      seek(0);
      return;
    case "Delete":
    case "Backspace":
      e.preventDefault();
      if (sel.length) deleteClips(sel);
      else if (e.shiftKey && s.selection.trackId) deleteTrack(s.selection.trackId);
      return;
    case "Escape": {
      const t = useTransport.getState();
      if (!t.playing) backend.panic();
      useStore.setState((st) => ({ selection: { ...st.selection, clipIds: [], noteIds: [] } }));
      return;
    }
    case "+":
    case "=":
      setUi({ pxPerBeat: clamp(s.ui.pxPerBeat * 1.3, 2, 400) });
      return;
    case "-":
      setUi({ pxPerBeat: clamp(s.ui.pxPerBeat / 1.3, 2, 400) });
      return;
    case "?":
      useStore.setState({ dialog: "help" });
      return;
  }

  switch (key) {
    case "r":
      e.preventDefault();
      record();
      return;
    case "l":
      commit((p) => void (p.loop.enabled = !p.loop.enabled));
      return;
    case "k":
      commit((p) => void (p.metronome.enabled = !p.metronome.enabled));
      return;
    case "s":
      if (sel.length) splitClipsAt(sel, livePosition());
      return;
    case "m":
      if (sel.length) {
        const set = new Set(sel);
        const anyUnmuted = s.project.tracks.some((t) => t.clips.some((c) => set.has(c.id) && !c.muted));
        commit((p) => {
          for (const t of p.tracks) for (const c of t.clips) if (set.has(c.id)) c.muted = anyUnmuted;
        });
      }
      return;
    case "b":
      setUi({ browser: !s.ui.browser });
      return;
    case "e":
      setUi({ bottom: s.ui.bottom === "editor" ? null : "editor" });
      return;
    case "x":
      setUi({ bottom: s.ui.bottom === "mixer" ? null : "mixer" });
      return;
    case "i":
      setUi({ inspector: !s.ui.inspector });
      return;
    case "f":
      setUi({ follow: !s.ui.follow });
      return;
  }
}

function onKeyUp(e: KeyboardEvent) {
  const key = e.key.toLowerCase();
  const pitch = held.get(key);
  if (pitch !== undefined) {
    held.delete(key);
    liveNote(pitch, 0, false);
  }
}

export function initKeyboard() {
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", () => {
    for (const p of held.values()) liveNote(p, 0, false);
    held.clear();
  });
}
