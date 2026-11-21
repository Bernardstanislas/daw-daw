// Left panel: instruments, effects, loops (patterns) and audio files.

import { useEffect, useMemo, useState } from "react";
import { setUi, useStore } from "../lib/store";
import { loadPlugins, useCatalog } from "../lib/catalog";
import { PATTERNS } from "../lib/patterns";
import { endDrag, startDrag, dropOnArrange, type DragPayload } from "../lib/dnd";
import { backend, isTauri, openDialog, type DirEntry } from "../lib/backend";
import { addEffect, importAudioFiles } from "../lib/actions";
import { IconChevronDown, IconChevronRight, IconDrum, IconFile, IconFolder, IconFx, IconLoops, IconPiano, IconPlay, IconSearch, IconStop, IconSynth } from "./icons";
import { livePosition } from "../lib/engine";
import type { DeviceSpec } from "../lib/types";

function Item({
  label,
  sub,
  payload,
  icon,
  onActivate,
  onSelect,
  active,
  hint,
}: {
  label: string;
  sub?: string;
  payload: DragPayload;
  icon?: React.ReactNode;
  onActivate?: () => void;
  onSelect?: () => void;
  active?: boolean;
  hint?: string;
}) {
  return (
    <div
      className={`browser-item ${active ? "previewing" : ""}`}
      draggable
      onDragStart={(e) => startDrag(e, payload)}
      onDragEnd={endDrag}
      onClick={onSelect}
      onDoubleClick={onActivate}
      title={`${label}${sub ? " — " + sub : ""}\n${hint ?? "Drag to the arrangement, or double-click"}`}
    >
      {icon && <span className="browser-item-icon">{icon}</span>}
      <span className="browser-item-label">{label}</span>
      {sub && <span className="browser-item-sub">{sub}</span>}
    </div>
  );
}

function Group({ title, children, defaultOpen = true, count }: { title: string; children: React.ReactNode; defaultOpen?: boolean; count?: number }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="browser-group">
      <div className="browser-group-title" onClick={() => setOpen(!open)}>
        {open ? <IconChevronDown size={12} /> : <IconChevronRight size={12} />}
        <span>{title}</span>
        {count !== undefined && <span className="count">{count}</span>}
      </div>
      {open && <div className="browser-group-body">{children}</div>}
    </div>
  );
}

function instIcon(type: string) {
  return ["kit", "samplekit", "rhythm"].includes(type) ? <IconDrum /> : ["keys", "sampler", "grand"].includes(type) ? <IconPiano /> : <IconSynth />;
}

function DeviceList({ specs, filter, kind }: { specs: DeviceSpec[]; filter: string; kind: "instrument" | "effect" }) {
  const f = filter.toLowerCase();
  const selectedTrack = useStore((s) => s.selection.trackId);
  const preview = useStore((s) => s.preview);
  const previewName = preview && (preview.preset && preview.preset !== "Default" ? preview.preset : specs.find((x) => x.typeId === preview.type)?.name);
  return (
    <>
      {kind === "instrument" && preview && (
        <div className="preview-banner">
          <span className="preview-dot" />
          <span>
            Auditioning <b>{previewName}</b> — play your MIDI keyboard or ⌨
          </span>
          <button className="btn small" onClick={() => useStore.setState({ preview: null })} title="Stop auditioning">
            Stop
          </button>
        </div>
      )}
      {specs.map((spec) => {
        const presets = [{ name: "Default" }, ...spec.presets.filter((p) => p.name !== "Default")];
        const shown = presets.filter((p) => !f || p.name.toLowerCase().includes(f) || spec.name.toLowerCase().includes(f));
        if (!shown.length) return null;
        return (
          <Group key={spec.typeId} title={spec.name} count={shown.length} defaultOpen={kind === "instrument" || !!f}>
            {shown.map((p) => (
              <Item
                key={p.name}
                label={p.name === "Default" ? `${spec.name}` : p.name}
                icon={kind === "instrument" ? instIcon(spec.typeId) : <IconFx />}
                payload={{ kind, type: spec.typeId, preset: p.name } as DragPayload}
                active={kind === "instrument" && preview?.type === spec.typeId && preview?.preset === p.name}
                hint={kind === "instrument" ? "Click: audition with your MIDI keyboard · drag or double-click: add a track" : undefined}
                onSelect={
                  kind === "instrument"
                    ? () => {
                        const cur = useStore.getState().preview;
                        const same = cur?.type === spec.typeId && cur?.preset === p.name;
                        useStore.setState({ preview: same ? null : { type: spec.typeId, preset: p.name } });
                      }
                    : undefined
                }
                onActivate={() => {
                  useStore.setState({ preview: null });
                  if (kind === "instrument") dropOnArrange({ kind: "instrument", type: spec.typeId, preset: p.name }, null, 0);
                  else if (selectedTrack) addEffect(selectedTrack, spec.typeId, p.name);
                }}
              />
            ))}
          </Group>
        );
      })}
      <PluginGroup kind={kind} filter={f} />
    </>
  );
}

/** Installed third-party plugins (Audio Units on macOS). */
function PluginGroup({ kind, filter }: { kind: "instrument" | "effect"; filter: string }) {
  const plugins = useCatalog((s) => s.plugins);
  const preview = useStore((s) => s.preview);
  const selectedTrack = useStore((s) => s.selection.trackId);
  const [scanning, setScanning] = useState(false);
  if (!isTauri) {
    return (
      <Group title="Plugins" defaultOpen={false}>
        <div className="browser-note">VST / Audio Unit plugins are available in the desktop app.</div>
      </Group>
    );
  }
  const list = plugins.filter(
    (p) => p.kind === kind && (!filter || p.name.toLowerCase().includes(filter) || p.vendor.toLowerCase().includes(filter)),
  );
  return (
    <Group title="Plugins" count={list.length}>
      {list.map((p) => (
        <Item
          key={p.id}
          label={p.name}
          sub={p.vendor}
          icon={kind === "instrument" ? <IconPlug /> : <IconFx />}
          payload={{ kind, type: p.id } as DragPayload}
          active={kind === "instrument" && preview?.type === p.id}
          hint={
            kind === "instrument"
              ? `${p.format} plugin · click: audition with your MIDI keyboard · drag or double-click: add a track`
              : `${p.format} plugin · drag onto a track, or double-click to add it to the selected track`
          }
          onSelect={
            kind === "instrument"
              ? () => {
                  const cur = useStore.getState().preview;
                  useStore.setState({ preview: cur?.type === p.id ? null : { type: p.id } });
                }
              : undefined
          }
          onActivate={() => {
            useStore.setState({ preview: null });
            if (kind === "instrument") dropOnArrange({ kind: "instrument", type: p.id }, null, 0);
            else if (selectedTrack) addEffect(selectedTrack, p.id);
          }}
        />
      ))}
      {list.length === 0 && <div className="browser-note">{filter ? "No matching plugin." : "No plugins found."}</div>}
      <div className="browser-note">
        <button
          className="btn small"
          disabled={scanning}
          onClick={async () => {
            setScanning(true);
            await loadPlugins(true);
            setScanning(false);
          }}
        >
          {scanning ? "Scanning…" : "Rescan plugins"}
        </button>
      </div>
    </Group>
  );
}

const IconPlug = () => (
  <svg width={16} height={16} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round">
    <path d="M6 2v3M10 2v3M4.5 5h7v3a3.5 3.5 0 0 1-7 0zM8 11.5V14" />
  </svg>
);

function Loops({ filter }: { filter: string }) {
  const f = filter.toLowerCase();
  const cats = ["Drums", "Bass", "Chords", "Melody"] as const;
  const trackId = useStore((s) => s.selection.trackId);
  return (
    <>
      {cats.map((cat) => {
        const items = PATTERNS.filter((p) => p.category === cat && (!f || p.name.toLowerCase().includes(f)));
        if (!items.length) return null;
        return (
          <Group key={cat} title={cat} count={items.length}>
            {items.map((p) => (
              <Item
                key={p.id}
                label={p.name}
                sub={`${p.length / 4} bar${p.length > 4 ? "s" : ""}`}
                icon={<IconLoops />}
                payload={{ kind: "pattern", id: p.id }}
                onActivate={() => {
                  const t = useStore.getState().project.tracks.find((t) => t.id === trackId);
                  dropOnArrange({ kind: "pattern", id: p.id }, t?.kind === "instrument" ? t.id : null, Math.floor(livePosition() / 4) * 4);
                }}
              />
            ))}
          </Group>
        );
      })}
    </>
  );
}

function Files({ filter }: { filter: string }) {
  const [dir, setDir] = useState<string | null>(null);
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState<string | null>(null);
  useEffect(() => {
    const saved = localStorage.getItem("dawdaw:browseDir");
    if (saved) setDir(saved);
    else backend.defaultDirs().then((d) => setDir(d.music || d.home));
  }, []);
  useEffect(() => {
    if (!dir) return;
    localStorage.setItem("dawdaw:browseDir", dir);
    backend
      .listDir(dir)
      .then((e) => {
        setEntries(e);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, [dir]);
  const f = filter.toLowerCase();
  const parent = dir && dir !== "/" ? dir.replace(/\/[^/]+\/?$/, "") || "/" : null;
  const preview = (path: string | null) => {
    setPreviewing(path);
    backend.previewFile(path).catch(() => setPreviewing(null));
  };
  return (
    <div className="files">
      <div className="files-bar">
        <button
          className="btn small"
          onClick={async () => {
            const res = await openDialog({ directory: true, title: "Choose a folder", defaultPath: dir ?? undefined });
            if (res) setDir(res[0]);
          }}
        >
          <IconFolder /> Folder…
        </button>
        <button
          className="btn small"
          onClick={async () => {
            const res = await openDialog({
              multiple: true,
              title: "Import audio",
              filters: [{ name: "Audio", extensions: ["wav", "mp3", "flac", "ogg", "aif", "aiff", "m4a"] }],
            });
            if (res) {
              const s = useStore.getState();
              const t = s.project.tracks.find((t) => t.id === s.selection.trackId);
              importAudioFiles(res, t?.kind === "audio" ? t.id : null, Math.floor(livePosition()));
            }
          }}
        >
          <IconFile /> Import…
        </button>
        <button
          className="btn small"
          title="Built-in samples (instruments and drums)"
          onClick={() => backend.libraryDir().then((d) => d && setDir(d)).catch((e) => setError(String(e)))}
        >
          <IconLoops /> Library
        </button>
      </div>
      <div className="files-path" title={dir ?? ""}>
        {dir}
      </div>
      {!isTauri && (
        <div className="browser-note">
          <label className="btn small">
            <IconFile /> Import audio…
            <input
              type="file"
              accept="audio/*,.wav,.mp3,.flac,.ogg,.aif,.aiff,.m4a"
              multiple
              hidden
              onChange={(e) => {
                const files = [...(e.target.files ?? [])];
                e.target.value = "";
                const s = useStore.getState();
                const t = s.project.tracks.find((t) => t.id === s.selection.trackId);
                importAudioFiles(files, t?.kind === "audio" ? t.id : null, Math.floor(livePosition()));
              }}
            />
          </label>
          <div style={{ marginTop: 8 }}>Or drop audio files from your desktop onto the arrangement.</div>
        </div>
      )}
      {error && <div className="browser-note error">{error}</div>}
      <div className="files-list">
        {parent && (
          <div className="browser-item" onClick={() => setDir(parent)}>
            <span className="browser-item-icon">
              <IconFolder />
            </span>
            <span className="browser-item-label">..</span>
          </div>
        )}
        {entries
          .filter((e) => !f || e.name.toLowerCase().includes(f))
          .map((e) =>
            e.isDir ? (
              <div key={e.path} className="browser-item" onClick={() => setDir(e.path)}>
                <span className="browser-item-icon">
                  <IconFolder />
                </span>
                <span className="browser-item-label">{e.name}</span>
              </div>
            ) : (
              <div
                key={e.path}
                className={`browser-item ${previewing === e.path ? "playing" : ""}`}
                draggable
                onDragStart={(ev) => startDrag(ev, { kind: "file", path: e.path })}
                onDragEnd={endDrag}
                onDoubleClick={() => {
                  const s = useStore.getState();
                  const t = s.project.tracks.find((t) => t.id === s.selection.trackId);
                  importAudioFiles([e.path], t?.kind === "audio" ? t.id : null, Math.floor(livePosition()));
                }}
                title="Drag to the arrangement · double-click to import at the playhead"
              >
                <button
                  className="preview-btn"
                  onClick={(ev) => {
                    ev.stopPropagation();
                    preview(previewing === e.path ? null : e.path);
                  }}
                  title="Preview"
                >
                  {previewing === e.path ? <IconStop size={10} /> : <IconPlay size={10} />}
                </button>
                <span className="browser-item-label">{e.name}</span>
              </div>
            ),
          )}
      </div>
    </div>
  );
}

export function Browser() {
  const tab = useStore((s) => s.ui.browserTab);
  const specs = useCatalog((s) => s.specs);
  const [filter, setFilter] = useState("");
  const instruments = useMemo(() => specs.filter((s) => s.kind === "instrument"), [specs]);
  const effects = useMemo(() => specs.filter((s) => s.kind === "effect"), [specs]);
  const tabs: { id: typeof tab; label: string; icon: React.ReactNode }[] = [
    { id: "instruments", label: "Instruments", icon: <IconSynth /> },
    { id: "loops", label: "Loops", icon: <IconLoops /> },
    { id: "effects", label: "Effects", icon: <IconFx /> },
    { id: "files", label: "Files", icon: <IconFolder /> },
  ];
  return (
    <aside className="browser panel">
      <div className="panel-tabs">
        {tabs.map((t) => (
          <button key={t.id} className={`panel-tab ${tab === t.id ? "on" : ""}`} onClick={() => setUi({ browserTab: t.id })} title={t.label}>
            {t.icon}
            <span>{t.label}</span>
          </button>
        ))}
      </div>
      <div className="browser-search">
        <IconSearch />
        <input
          placeholder="Search…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Escape") setFilter("");
          }}
        />
      </div>
      <div className="browser-body">
        {tab === "instruments" && <DeviceList specs={instruments} filter={filter} kind="instrument" />}
        {tab === "effects" && <DeviceList specs={effects} filter={filter} kind="effect" />}
        {tab === "loops" && <Loops filter={filter} />}
        {tab === "files" && <Files filter={filter} />}
      </div>
    </aside>
  );
}
