// Modal dialogs: audio settings, export, new project, shortcuts.

import { useEffect, useState } from "react";
import { useStore, toast } from "../lib/store";
import { backend, isTauri, listen, type AudioStatus } from "../lib/backend";
import { exportMixdown, newProject } from "../lib/fileOps";
import { TEMPLATES } from "../lib/templates";
import { IconClose } from "./icons";

function Modal({ title, children, onClose, wide }: { title: string; children: React.ReactNode; onClose: () => void; wide?: boolean }) {
  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className={`modal ${wide ? "wide" : ""}`} onMouseDown={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div className="modal-title">{title}</div>
          <button className="icon-btn" onClick={onClose}>
            <IconClose />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

const close = () => useStore.setState({ dialog: null });

function SettingsDialog() {
  const [st, setSt] = useState<AudioStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const refresh = () => backend.getAudioStatus().then(setSt);
  useEffect(() => {
    refresh();
  }, []);
  if (!st) return <Modal title="Audio & MIDI" onClose={close}>Loading…</Modal>;
  const s = st.settings;
  const apply = async (patch: Partial<typeof s>) => {
    setBusy(true);
    try {
      await backend.setAudioSettings({ ...s, ...patch });
      toast("Audio settings applied", "success");
    } catch (e) {
      toast(`Audio device error: ${e}`, "error", 6000);
    }
    await refresh();
    setBusy(false);
  };
  return (
    <Modal title="Audio & MIDI settings" onClose={close}>
      <div className="form">
        {st.error && <div className="form-error">Audio output error: {st.error}</div>}
        <label>
          <span>Output device</span>
          <select disabled={busy} value={s.outputDevice ?? ""} onChange={(e) => apply({ outputDevice: e.target.value || null })}>
            <option value="">System default{st.devices.defaultOutput ? ` (${st.devices.defaultOutput})` : ""}</option>
            {st.devices.outputs.map((d) => (
              <option key={d}>{d}</option>
            ))}
          </select>
        </label>
        <label>
          <span>Input device</span>
          <select disabled={busy} value={s.inputDevice ?? ""} onChange={(e) => apply({ inputDevice: e.target.value || null })}>
            <option value="">System default{st.devices.defaultInput ? ` (${st.devices.defaultInput})` : ""}</option>
            {st.devices.inputs.map((d) => (
              <option key={d}>{d}</option>
            ))}
          </select>
        </label>
        <label>
          <span>Buffer size</span>
          <select disabled={busy} value={s.bufferSize ?? ""} onChange={(e) => apply({ bufferSize: e.target.value ? +e.target.value : null })}>
            <option value="">Device default</option>
            {[64, 128, 256, 512, 1024, 2048].map((b) => (
              <option key={b} value={b}>
                {b} samples
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Recording offset</span>
          <span className="inline">
            <input
              type="number"
              className="text-input small"
              value={s.recordOffsetMs}
              step={1}
              onChange={(e) => apply({ recordOffsetMs: +e.target.value || 0 })}
            />{" "}
            ms (shifts recorded audio earlier to compensate latency)
          </span>
        </label>
        <div className="form-info">
          {st.output ? (
            <>
              Running on <b>{st.output.device}</b> · {st.output.sampleRate} Hz · {st.output.channels} ch
              {st.output.bufferSize ? ` · ${st.output.bufferSize} samples` : ""}
            </>
          ) : (
            "No audio output running."
          )}
        </div>
        <div className="form-row">
          <span>
            MIDI inputs: {st.midiPorts.length ? st.midiPorts.join(", ") : <i>none detected</i>}
          </span>
          <button className="btn small" onClick={() => backend.rescanMidi().then(refresh)}>
            Rescan
          </button>
        </div>
        <div className="form-info small">Projects, recordings and exports are stored in {st.baseDir}</div>
        {!isTauri && <div className="form-error">Running in a browser: audio is disabled (UI preview only).</div>}
      </div>
    </Modal>
  );
}

function ExportDialog() {
  const [sr, setSr] = useState(48000);
  const [bits, setBits] = useState(24);
  const [range, setRange] = useState<"song" | "loop">("song");
  const progress = useStore((s) => s.exportProgress);
  useEffect(() => {
    const un = listen<number>("export-progress", (p) => useStore.setState({ exportProgress: p }));
    return () => {
      un.then((f) => f());
    };
  }, []);
  return (
    <Modal title="Export mixdown" onClose={() => progress === null && close()}>
      <div className="form">
        <label>
          <span>Range</span>
          <select value={range} onChange={(e) => setRange(e.target.value as "song" | "loop")}>
            <option value="song">Whole song</option>
            <option value="loop">Loop region</option>
          </select>
        </label>
        <label>
          <span>Sample rate</span>
          <select value={sr} onChange={(e) => setSr(+e.target.value)}>
            <option value={44100}>44.1 kHz</option>
            <option value={48000}>48 kHz</option>
            <option value={96000}>96 kHz</option>
          </select>
        </label>
        <label>
          <span>Format</span>
          <select value={bits} onChange={(e) => setBits(+e.target.value)}>
            <option value={16}>WAV 16-bit</option>
            <option value={24}>WAV 24-bit</option>
            <option value={32}>WAV 32-bit float</option>
          </select>
        </label>
        {progress !== null && (
          <div className="progress">
            <div className="progress-bar" style={{ width: `${progress * 100}%` }} />
          </div>
        )}
        <div className="modal-actions">
          <button className="btn" onClick={close} disabled={progress !== null}>
            Cancel
          </button>
          <button className="btn primary" disabled={progress !== null} onClick={() => exportMixdown({ sampleRate: sr, bits, range })}>
            {progress !== null ? `Rendering… ${Math.round(progress * 100)}%` : "Export WAV…"}
          </button>
        </div>
      </div>
    </Modal>
  );
}

function NewDialog() {
  return (
    <Modal title="New project" onClose={close} wide>
      <div className="templates">
        {TEMPLATES.map((t) => (
          <button key={t.id} className="template-card" onClick={() => newProject(t.id)}>
            <div className="template-name">{t.name}</div>
            <div className="template-desc">{t.description}</div>
          </button>
        ))}
      </div>
    </Modal>
  );
}

const SHORTCUTS: [string, string][] = [
  ["Space", "Play / stop"],
  ["R", "Record"],
  ["Home / Enter", "Go to start"],
  ["L", "Toggle loop"],
  ["K", "Toggle metronome"],
  ["⌘Z / ⇧⌘Z", "Undo / redo"],
  ["⌘C / ⌘X / ⌘V", "Copy / cut / paste clips (paste at playhead)"],
  ["⌘D", "Duplicate selected clips (or notes in the editor)"],
  ["S", "Split selected clips at the playhead"],
  ["M", "Mute selected clips"],
  ["⌫ / Delete", "Delete selection"],
  ["⌘A", "Select all clips (or notes in the editor)"],
  ["⌘S / ⇧⌘S", "Save / save as"],
  ["⌘O / ⌘N", "Open / new project"],
  ["⌘E", "Export mixdown"],
  ["B / E / X / I", "Browser / editor / mixer / inspector"],
  ["+ / −", "Zoom in / out (⌘ + wheel too)"],
  ["F", "Follow playhead"],
  ["Caps Lock or ⌨ button", "Musical typing: A W S E D F T G Y H U J K = notes, Z / X = octave, C / V = velocity"],
  ["↑ / ↓ (editor)", "Transpose selected notes (⇧ = octave)"],
  ["Q (editor)", "Quantize notes"],
  ["Esc", "Panic (stop all sound) when stopped"],
];

function HelpDialog() {
  return (
    <Modal title="Keyboard shortcuts & tips" onClose={close} wide>
      <div className="help">
        <table>
          <tbody>
            {SHORTCUTS.map(([k, v]) => (
              <tr key={k}>
                <td>
                  <kbd>{k}</kbd>
                </td>
                <td>{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="help-tips">
          <h4>Tips</h4>
          <ul>
            <li>Drag instruments, loops and effects from the browser onto tracks or the empty area.</li>
            <li>Double-click an empty spot on an instrument track to create a MIDI clip.</li>
            <li>Drag a clip with ⌥ (Alt) to copy it. Hold ⌘ while dragging to ignore the grid.</li>
            <li>Draw song sections in the Arranger lane, then drag a section to re-order the whole song.</li>
            <li>Drag in the top strip of the ruler to set the loop region.</li>
            <li>Every knob: drag vertically (⇧ for fine), double-click to reset.</li>
          </ul>
        </div>
      </div>
    </Modal>
  );
}

export function Dialogs() {
  const dialog = useStore((s) => s.dialog);
  if (dialog === "settings") return <SettingsDialog />;
  if (dialog === "export") return <ExportDialog />;
  if (dialog === "new") return <NewDialog />;
  if (dialog === "help") return <HelpDialog />;
  return null;
}
