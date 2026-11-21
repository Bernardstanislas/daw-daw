// Bottom status bar: hints, musical typing state, audio info, toasts.

import { useEffect, useState } from "react";
import { PAGES, setUi, useStore, type Page } from "../lib/store";
import { IconArrange, IconLoop, IconMixer, IconSynth } from "./icons";
import { useTyping } from "../lib/keyboard";
import { useTransport } from "../lib/engine";
import { backend, isTauri, type AudioStatus } from "../lib/backend";
import { noteName } from "../lib/music";

const PAGE_ICONS: Record<Page, React.ReactNode> = {
  arrange: <IconArrange size={14} />,
  loop: <IconLoop size={14} />,
  sound: <IconSynth size={14} />,
  mix: <IconMixer size={14} />,
};

/** Page selector, bottom centre: Arrange · Loop · Sound · Mix, in the order of the work. */
function PageTabs() {
  const page = useStore((s) => s.ui.page);
  return (
    <nav className="page-tabs">
      {PAGES.map((p, i) => (
        <button key={p.id} className={`panel-tab ${page === p.id ? "on" : ""}`} onClick={() => setUi({ page: p.id })} title={`${p.hint} (${i + 1})`}>
          {PAGE_ICONS[p.id]}
          <span>{p.label}</span>
        </button>
      ))}
    </nav>
  );
}

export function StatusBar() {
  const toasts = useStore((s) => s.toasts);
  const typing = useStore((s) => s.ui.typing);
  const { octave, velocity } = useTyping();
  const live = useTransport((s) => s.liveNotes);
  const recording = useTransport((s) => s.recording);
  const countIn = useTransport((s) => s.countIn);
  const playing = useTransport((s) => s.playing);
  const [audio, setAudio] = useState<AudioStatus | null>(null);
  const dialog = useStore((s) => s.dialog);
  useEffect(() => {
    // The browser engine starts asynchronously (and after a click): keep polling there.
    if (!isTauri) {
      const t = setInterval(() => backend.getAudioStatus().then(setAudio).catch(() => {}), 1500);
      return () => clearInterval(t);
    }
  }, []);
  useEffect(() => {
    backend.getAudioStatus().then(setAudio).catch(() => {});
  }, [dialog]);
  return (
    <footer className="statusbar">
      <span className="status-state">
        <span className={`lamp ${recording || countIn ? "lit rec" : playing ? "lit" : ""}`} />
        {recording ? "Recording" : countIn ? "Count-in" : playing ? "Playing" : "Stopped"}
      </span>
      {typing && (
        <span className="status-item">
          ⌨ Musical typing · octave {octave} (Z/X) · velocity {velocity} (C/V)
        </span>
      )}
      {live.size > 0 && <span className="status-item notes">{[...live].sort((a, b) => a - b).map(noteName).join(" ")}</span>}
      <span className="spacer" />
      <PageTabs />
      <span className="status-item dim">
        {!isTauri
          ? audio?.error
            ? `No audio: ${audio.error}`
            : audio?.output
              ? `${audio.output.device} · ${(audio.output.sampleRate / 1000).toFixed(1)} kHz (browser)`
              : "Starting audio…"
          : audio?.output
            ? `${audio.output.device} · ${(audio.output.sampleRate / 1000).toFixed(1)} kHz`
            : audio?.error
              ? `No audio: ${audio.error}`
              : "Audio…"}
        {audio && audio.midiPorts.length > 0 ? ` · MIDI: ${audio.midiPorts.length} port${audio.midiPorts.length > 1 ? "s" : ""}` : ""}
      </span>
      <div className="toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </footer>
  );
}
