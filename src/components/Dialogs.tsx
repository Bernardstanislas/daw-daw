// Modal dialogs: audio settings, export, new project, shortcuts.

import { useEffect, useState } from "react";
import { useStore, toast } from "../lib/store";
import { backend, isTauri, listen, openDialog, type AudioStatus } from "../lib/backend";
import { exportMixdown, newProject } from "../lib/fileOps";
import { TEMPLATES } from "../lib/templates";
import { IconClose } from "./icons";
import { loadPlugins } from "../lib/catalog";
import { ccLabel, removeMapping, setLearnMode, setTrackChannel, targetLabel, useMidiLearn } from "../lib/midiMap";

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
  const [scanning, setScanning] = useState(false);
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
  const pluginDirs = s.pluginDirs ?? [];
  const rescanPlugins = async () => {
    setScanning(true);
    const found = await loadPlugins(true);
    setScanning(false);
    toast(`${found.length} plugins found`, "success");
  };
  const setPluginDirs = async (dirs: string[]) => {
    await apply({ pluginDirs: dirs });
    await rescanPlugins();
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
        <label>
          <span>Samples folder</span>
          <span className="inline path-row">
            <span className="path" title={st.samples.dir}>
              {st.samples.dir}
            </span>{" "}
            {isTauri && (
              <button
                className="btn small"
                disabled={busy}
                onClick={async () => {
                  const d = await openDialog({ directory: true, title: "Samples folder", defaultPath: st.samples.dir });
                  if (d?.[0]) apply({ samplesDir: d[0] });
                }}
              >
                Choose…
              </button>
            )}
            {isTauri && s.samplesDir && (
              <button className="btn small" disabled={busy} onClick={() => apply({ samplesDir: null })}>
                Default
              </button>
            )}
          </span>
        </label>
        <div className={st.samples.missing.length ? "form-error" : "form-info small"}>
          {st.samples.missing.length === 0
            ? `Sampler and Sample Kit: ${st.samples.total} samples loaded.`
            : `${st.samples.missing.length} of ${st.samples.total} samples missing: the folder must contain instruments/*.wav and drums/*.wav (e.g. ${st.samples.missing[0]}). Missing sounds stay silent.`}
        </div>
        <div className="form-row">
          <span>
            <b>Plugin folders</b>
          </span>
          {isTauri && (
            <span className="inline">
              <button
                className="btn small"
                disabled={busy || scanning}
                onClick={async () => {
                  const d = await openDialog({ directory: true, title: "Plugin folder" });
                  if (d?.[0] && !pluginDirs.includes(d[0])) setPluginDirs([...pluginDirs, d[0]]);
                }}
              >
                Add folder…
              </button>{" "}
              <button className="btn small" disabled={busy || scanning} onClick={rescanPlugins}>
                {scanning ? "Scanning…" : "Rescan"}
              </button>
            </span>
          )}
        </div>
        {pluginDirs.map((d) => (
          <div className="form-row" key={d}>
            <span className="inline path-row">
              <span className="path" title={d}>
                {d}
              </span>
            </span>
            <button className="btn small" disabled={busy || scanning} onClick={() => setPluginDirs(pluginDirs.filter((x) => x !== d))}>
              Remove
            </button>
          </div>
        ))}
        <div className="form-info small">
          VST3 plugins are searched in these folders and everything below them, plus the standard folders
          {st.defaultPluginDirs.length ? `: ${st.defaultPluginDirs.join(" · ")}` : ""}.
        </div>
        <div className="form-info small">Projects, recordings and exports are stored in {st.baseDir}</div>
        {!isTauri && <div className="form-error">Running in a browser: audio is disabled (UI preview only).</div>}
        <MidiControllerSettings />
      </div>
    </Modal>
  );
}

/** MIDI activity, the channel each track listens to, and the controller mappings. */
function MidiControllerSettings() {
  const last = useMidiLearn((s) => s.last);
  const project = useStore((s) => s.project);
  const tracks = project.tracks.filter((t) => t.kind === "instrument");
  const map = project.midiMap ?? [];
  return (
    <>
      <div className="form-row">
        <span>
          <b>Controller</b>
        </span>
        <span className="midi-monitor" title="Last message received: play a pad or turn a knob to see its channel">
          {last ? `${last.type === "note" ? `Note ${last.number}` : `CC${last.number}`} · ch ${last.channel} · ${last.value}` : "No MIDI received yet"}
        </span>
      </div>
      <div className="form-info small">
        Each track can listen to its own MIDI channel: set the drum track to the pads' channel (often 10) and the keys keep playing the selected
        track. Turn MIDI learn on (MIDI button, ⌘L), click any control, then move a knob, fader or button.
      </div>
      {tracks.length > 0 && (
        <table className="midi-table">
          <tbody>
            {tracks.map((t) => (
              <tr key={t.id}>
                <td>{t.name}</td>
                <td>
                  <select value={t.midiChannel ?? 0} onChange={(e) => setTrackChannel(t.id, +e.target.value || null)}>
                    <option value={0}>Follow selection</option>
                    {Array.from({ length: 16 }, (_, i) => (
                      <option key={i} value={i + 1}>
                        Channel {i + 1}
                      </option>
                    ))}
                  </select>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="form-row">
        <span>Mappings ({map.length})</span>
        <button
          className="btn small"
          onClick={() => {
            useStore.setState({ dialog: null });
            setLearnMode(true);
          }}
        >
          MIDI learn…
        </button>
      </div>
      {map.length > 0 && (
        <table className="midi-table">
          <tbody>
            {map.map((m) => {
              const label = targetLabel(project, m.target);
              return (
                <tr key={m.id}>
                  <td className="cc">{ccLabel(m)}</td>
                  <td className={label ? "" : "missing"}>{label ?? "deleted control"}</td>
                  <td>
                    <button className="icon-btn tiny" onClick={() => removeMapping(m.id)} title="Remove this mapping">
                      <IconClose size={10} />
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </>
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
  ["1 / 2 / 3", "Pages: Arrange / Sound / Mix (also the tabs at the bottom of the window)"],
  ["X", "Mix page, and back to Arrange"],
  ["B / E", "Browser / editor (Arrange page)"],
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
            <li>Instruments and effects are edited on the Sound page: double-click a track's instrument name (Arrange) or its name on a mixer strip to open it there.</li>
            <li>The settings of a clip (name, position, loop, gain, fades) are at the left of the editor: double-click the clip to open it.</li>
            <li>On the Mix page, click an insert to bypass it.</li>
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
