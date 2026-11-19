// Top bar: menu, undo/redo, transport, time display, tempo, snap and view toggles.

import { useEffect, useRef } from "react";
import { useStore, undo, redo, setUi, commit, beginGesture, gesture, endGesture } from "../lib/store";
import { livePosition, play, record, seek, stop, useTransport } from "../lib/engine";
import { formatBBT, formatTime, SNAP_OPTIONS, type SnapValue } from "../lib/music";
import { newProject, openProject, saveProject } from "../lib/fileOps";
import { DragNumber, VuMeter } from "./controls";
import { MenuButton, type MenuItem } from "./Menu";
import {
  IconBrowser,
  IconExport,
  IconKeyboard,
  IconLoop,
  IconMetronome,
  IconPiano,
  IconPlay,
  IconRecord,
  IconRedo,
  IconRewind,
  IconSettings,
  IconStop,
  IconUndo,
  IconPause,
  IconHelp,
} from "./icons";
import { TEMPLATES } from "../lib/templates";

const mod = navigator.platform.includes("Mac") ? "⌘" : "Ctrl+";

/** Pads a 7-segment string: "!" is DSEG's blank digit (same width as "8"). */
function seg(text: string, digits: number): string {
  return text.padStart(digits, "!");
}

/** Bar.beat.tick and time on a 7-segment display, ghost segments behind. */
function TimeDisplay() {
  const bbt = useRef<HTMLSpanElement>(null);
  const time = useRef<HTMLSpanElement>(null);
  const flag = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const p = useStore.getState().project;
      const pos = livePosition();
      const t = useTransport.getState();
      const [bar, beat, ticks] = formatBBT(pos, p.timeSigNum, p.timeSigDen).split(".");
      if (bbt.current) bbt.current.textContent = `${seg(bar, 3)}.${beat}.${ticks}`;
      const [m, rest] = formatTime(pos, p.bpm).split(":");
      if (time.current) time.current.textContent = `${seg(m, 2)}:${rest}`;
      if (flag.current) flag.current.dataset.on = t.countIn ? "1" : "0";
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);
  return (
    <div className="lcd-cell" title="Position (bar.beat.tick) and time">
      <div className="lcd-digits big">
        <span className="ghost">888.8.888</span>
        <span ref={bbt} />
      </div>
      <div className="lcd-row">
        <div className="lcd-digits small">
          <span className="ghost">88:88.888</span>
          <span ref={time} />
        </div>
        <span className="lcd-flag" ref={flag}>
          count-in
        </span>
      </div>
    </div>
  );
}

export function fileMenu(): MenuItem[] {
  return [
    { label: "New project", shortcut: `${mod}N`, onClick: () => useStore.setState({ dialog: "new" }) },
    {
      label: "New from template",
      submenu: TEMPLATES.map((t) => ({ label: t.name, onClick: () => newProject(t.id) })),
    },
    { label: "Open…", shortcut: `${mod}O`, onClick: () => openProject() },
    { separator: true },
    { label: "Save", shortcut: `${mod}S`, onClick: () => saveProject() },
    { label: "Save as…", shortcut: `⇧${mod}S`, onClick: () => saveProject(true) },
    { separator: true },
    { label: "Export mixdown…", shortcut: `${mod}E`, onClick: () => useStore.setState({ dialog: "export" }) },
    { separator: true },
    { label: "Audio & MIDI settings…", shortcut: `${mod},`, onClick: () => useStore.setState({ dialog: "settings" }) },
    { label: "Keyboard shortcuts", shortcut: "?", onClick: () => useStore.setState({ dialog: "help" }) },
  ];
}

export function TopBar() {
  const playing = useTransport((s) => s.playing);
  const recording = useTransport((s) => s.recording || s.countIn);
  const cpu = useTransport((s) => Math.round(s.cpu * 100) / 100);
  const project = useStore((s) => s.project);
  const ui = useStore((s) => s.ui);
  const canUndo = useStore((s) => s.past.length > 0);
  const canRedo = useStore((s) => s.future.length > 0);
  const projectPath = useStore((s) => s.projectPath);
  const dirty = useStore((s) => s.savedProject !== s.project);
  const arrange = ui.page === "arrange";

  return (
    <header className="topbar">
      <div className="topbar-left">
        <MenuButton className="brand" items={fileMenu} title="Menu">
          <span className={`lamp brand-lamp ${playing ? "lit" : ""}`} />
          <span className="brand-name">daw-daw</span>
        </MenuButton>
        <div className="project-name" title={projectPath ?? "Not saved yet"}>
          {project.name}
          {dirty && <span className="dirty-dot">•</span>}
        </div>
        <div className="btn-group">
          <button className="icon-btn" disabled={!canUndo} onClick={undo} title={`Undo (${mod}Z)`}>
            <IconUndo />
          </button>
          <button className="icon-btn" disabled={!canRedo} onClick={redo} title={`Redo (⇧${mod}Z)`}>
            <IconRedo />
          </button>
        </div>
      </div>

      <div className="topbar-center">
        <div className="transport">
          <button className="tbtn" onClick={() => seek(project.loop.enabled ? project.loop.start : 0)} title="Go to start (Home)">
            <IconRewind />
          </button>
          <button className="tbtn" onClick={stop} title="Stop (Space) — press twice to rewind">
            <IconStop />
          </button>
          <button className={`tbtn play ${playing ? "on" : ""}`} onClick={() => (playing ? stop() : play())} title="Play / Pause (Space)">
            <span className={`lamp ${playing ? "lit" : ""}`} />
            {playing ? <IconPause /> : <IconPlay />}
          </button>
          <button className={`tbtn rec ${recording ? "on" : ""}`} onClick={record} title="Record (R)">
            <span className={`lamp rec ${recording ? "lit" : ""}`} />
            <IconRecord />
          </button>
          <button
            className={`tbtn toggle ${project.loop.enabled ? "on" : ""}`}
            onClick={() =>
              commit((p) => {
                p.loop.enabled = !p.loop.enabled;
              })
            }
            title="Loop (L)"
          >
            <span className={`lamp ${project.loop.enabled ? "lit" : ""}`} />
            <IconLoop />
          </button>
          <button
            className={`tbtn toggle ${project.metronome.enabled ? "on" : ""}`}
            onClick={() =>
              commit((p) => {
                p.metronome.enabled = !p.metronome.enabled;
              })
            }
            onContextMenu={(e) => {
              e.preventDefault();
              commit((p) => {
                p.metronome.countInBars = (p.metronome.countInBars + 1) % 3;
              });
            }}
            title={`Metronome (K) — right-click: count-in ${project.metronome.countInBars} bar(s)`}
          >
            <span className={`lamp ${project.metronome.enabled ? "lit" : ""}`} />
            <IconMetronome />
            {project.metronome.countInBars > 0 && <span className="tbtn-badge">{project.metronome.countInBars}</span>}
          </button>
        </div>
        <div className="lcd">
        <TimeDisplay />
        <div className="lcd-sep" />
        <div className="lcd-cell tempo-box">
          <DragNumber
            className="tempo"
            value={project.bpm}
            min={20}
            max={300}
            step={1}
            digits={project.bpm % 1 ? 2 : 0}
            pxPerStep={3}
            onBegin={beginGesture}
            onChange={(v) =>
              gesture((p) => {
                p.bpm = v;
              })
            }
            onEnd={endGesture}
            title="Tempo (BPM) — drag or double-click"
          />
          <span className="lcd-label">bpm</span>
        </div>
        <div className="lcd-sep" />
        <div className="lcd-cell sig-box" title="Time signature">
          <select
            value={project.timeSigNum}
            onChange={(e) =>
              commit((p) => {
                p.timeSigNum = +e.target.value;
              })
            }
          >
            {[2, 3, 4, 5, 6, 7, 9, 12].map((n) => (
              <option key={n}>{n}</option>
            ))}
          </select>
          <span className="sig-slash">/</span>
          <select
            value={project.timeSigDen}
            onChange={(e) =>
              commit((p) => {
                p.timeSigDen = +e.target.value;
              })
            }
          >
            {[2, 4, 8].map((n) => (
              <option key={n}>{n}</option>
            ))}
          </select>
          <span className="lcd-label">meter</span>
        </div>
        </div>
      </div>

      <div className="topbar-right">
        <label className="snap-box lcd-mini" title="Snap to grid">
          <span className="engraved">snap</span>
          <select value={ui.snap} onChange={(e) => setUi({ snap: e.target.value as SnapValue })}>
            {SNAP_OPTIONS.map((s) => (
              <option key={s} value={s}>
                {s === "off" ? "Off" : s === "bar" ? "Bar" : s}
              </option>
            ))}
          </select>
        </label>
        <div className="btn-group">
          <button className={`icon-btn ${ui.browser ? "on" : ""}`} onClick={() => setUi({ browser: !ui.browser })} title="Browser (B)">
            <span className={`lamp ${ui.browser ? "lit" : ""}`} />
            <IconBrowser />
          </button>
          <button
            className={`icon-btn ${arrange && ui.bottom === "editor" ? "on" : ""}`}
            disabled={!arrange}
            onClick={() => setUi({ bottom: ui.bottom === "editor" ? null : "editor" })}
            title="Editor (E) — Arrange page"
          >
            <span className={`lamp ${arrange && ui.bottom === "editor" ? "lit" : ""}`} />
            <IconPiano />
          </button>
        </div>
        <button className={`icon-btn ${ui.typing ? "on" : ""}`} onClick={() => setUi({ typing: !ui.typing })} title="Musical typing: play notes with your computer keyboard (Caps Lock)">
          <span className={`lamp ${ui.typing ? "lit" : ""}`} />
          <IconKeyboard />
        </button>
        <button className="icon-btn" onClick={() => useStore.setState({ dialog: "export" })} title="Export mixdown">
          <IconExport />
        </button>
        <button className="icon-btn" onClick={() => useStore.setState({ dialog: "settings" })} title="Audio & MIDI settings">
          <IconSettings />
        </button>
        <button className="icon-btn" onClick={() => useStore.setState({ dialog: "help" })} title="Shortcuts & help">
          <IconHelp />
        </button>
        <div className="master-vu" title={`Master output — DSP load ${(cpu * 100).toFixed(0)}%`}>
          <VuMeter source="master" width={92} label="VU" />
          <div className="cpu">
            dsp <b>{(cpu * 100).toFixed(0)}</b>
          </div>
        </div>
      </div>
    </header>
  );
}
