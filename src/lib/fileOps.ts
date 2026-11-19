// New / open / save / export.

import { backend, confirmDialog, isTauri, openDialog, saveDialog } from "./backend";
import { emptyProject, isDirty, silent, toast, TRACK_COLORS, useStore } from "./store";
import type { Project } from "./types";
import { TEMPLATES } from "./templates";
import { isPluginType } from "./catalog";
import { seek } from "./engine";

const LAST_PROJECT_KEY = "dawdaw:lastProject";
const AUTOSAVE_KEY = "dawdaw:autosave";

/** Colors of the first palette, mapped to the night-studio palette (same order). */
const OLD_TRACK_COLORS = ["#ff7a6b", "#ffaa4d", "#f5d04a", "#7ed67a", "#3fd1b0", "#4fb2ff", "#8a8cff", "#cf7dff", "#ff7ab8", "#b9c2cf"];
const OLD_SECTION_COLORS = ["#5b8cff", "#ff8a5b", "#4fd18b", "#d36bff", "#ffd05b", "#5bd6ff"];
const NEW_SECTION_COLORS = ["#6f8fb3", "#b58a5c", "#7f9e74", "#9a83b0", "#b3a05e", "#6e9f9a"];

function restyle(color: string, from: string[], to: string[]): string {
  const i = from.indexOf(color.toLowerCase());
  return i >= 0 ? to[i % to.length] : color;
}

function migrate(raw: Partial<Project>): Project {
  const base = emptyProject();
  const p = { ...base, ...raw } as Project;
  p.loop = { ...base.loop, ...(raw.loop ?? {}) };
  p.metronome = { ...base.metronome, ...(raw.metronome ?? {}) };
  p.master = { ...base.master, ...(raw.master ?? {}) };
  p.sections = raw.sections ?? [];
  p.assets = raw.assets ?? [];
  p.sections = p.sections.map((s) => ({ ...s, color: restyle(s.color, OLD_SECTION_COLORS, NEW_SECTION_COLORS) }));
  p.tracks = (raw.tracks ?? []).map((t) => ({
    ...t,
    color: restyle(t.color, OLD_TRACK_COLORS, TRACK_COLORS),
    height: t.height ?? 72,
    effects: t.effects ?? [],
    clips: (t.clips ?? []).map((c) => ({ ...c, notes: c.notes ?? [], loopLength: c.loopLength ?? 0 })),
  }));
  return p;
}

export function loadDocument(project: Project, path: string | null) {
  const p = migrate(project);
  useStore.setState({
    project: p,
    projectPath: path,
    savedProject: p,
    past: [],
    future: [],
    selection: { trackId: p.tracks[0]?.id ?? null, clipIds: [], noteIds: [], sectionId: null },
    editorClipId: null,
  });
  seek(0);
  if (p.assets.length) {
    backend.loadAssets(p.assets).then((res) => {
      const assets = { ...useStore.getState().assets };
      const missing: string[] = [];
      for (const r of res) {
        if (r.info) assets[r.id] = r.info;
        else missing.push(p.assets.find((a) => a.id === r.id)?.name ?? r.id);
      }
      useStore.setState({ assets });
      if (missing.length) toast(`Missing audio files: ${missing.join(", ")}`, "error", 6000);
    });
  }
}

async function confirmDiscard(): Promise<boolean> {
  if (!isDirty()) return true;
  return confirmDialog("Discard unsaved changes to the current project?");
}

export async function newProject(templateId = "empty") {
  if (!(await confirmDiscard())) return;
  const t = TEMPLATES.find((x) => x.id === templateId) ?? TEMPLATES[0];
  loadDocument(t.build(), null);
  useStore.setState({ dialog: null, savedProject: null });
}

/** Browser: pick a project JSON file (saved with "Save" in the browser). */
function openProjectFromFile(): Promise<Project | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,.dawdaw,application/json";
    input.onchange = async () => {
      const f = input.files?.[0];
      if (!f) return resolve(null);
      try {
        resolve(JSON.parse(await f.text()));
      } catch {
        toast("Not a daw-daw project file", "error");
        resolve(null);
      }
    };
    input.click();
  });
}

export async function openProject(path?: string) {
  if (!(await confirmDiscard())) return;
  if (!isTauri) {
    const project = await openProjectFromFile();
    if (project) {
      loadDocument(project, null);
      toast(`Opened ${project.name} — re-import its audio files if it uses any`, "success", 5000);
    }
    return;
  }
  let target = path;
  if (!target) {
    const dirs = await backend.defaultDirs();
    const res = await openDialog({ directory: true, title: "Open project folder (.dawdaw)", defaultPath: dirs.projects });
    if (!res) return;
    target = res[0];
  }
  try {
    const { path: dir, project } = await backend.loadProject(target);
    loadDocument(project, dir);
    localStorage.setItem(LAST_PROJECT_KEY, dir);
    toast(`Opened ${project.name}`, "success");
  } catch (e) {
    toast(`Cannot open project: ${e}`, "error", 6000);
  }
}

/** Copies the live state of every third-party plugin into the project (no undo step). */
export async function capturePluginStates() {
  if (!isTauri) return;
  const p = useStore.getState().project;
  const devices = [...p.master.effects, ...p.tracks.flatMap((t) => [...(t.instrument ? [t.instrument] : []), ...t.effects])];
  if (!devices.some((d) => isPluginType(d.type))) return;
  try {
    const states = await backend.pluginStates();
    silent((d) => {
      const all = [...d.master.effects, ...d.tracks.flatMap((t) => [...(t.instrument ? [t.instrument] : []), ...t.effects])];
      for (const dev of all) {
        const st = states[dev.id];
        if (st && dev.state !== st) dev.state = st;
      }
    });
  } catch (e) {
    console.error("plugin states", e);
  }
}

export async function saveProject(saveAs = false): Promise<boolean> {
  await capturePluginStates();
  const s = useStore.getState();
  if (!isTauri) {
    // Browser: download the project as JSON (audio files are not included).
    const blob = new Blob([JSON.stringify(s.project, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${s.project.name || "Untitled"}.dawdaw.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
    useStore.setState({ savedProject: s.project });
    toast("Project downloaded", "success");
    return true;
  }
  let path = s.projectPath;
  if (!path || saveAs) {
    const dirs = await backend.defaultDirs();
    const chosen = await saveDialog({
      title: "Save project",
      defaultPath: `${dirs.projects}/${s.project.name || "Untitled"}.dawdaw`,
    });
    if (!chosen) return false;
    path = chosen.endsWith(".dawdaw") ? chosen : `${chosen}.dawdaw`;
  }
  try {
    const name = path.split("/").pop()!.replace(/\.(dawdaw|brioche)$/, "");
    const doc = { ...s.project, name: s.projectPath && !saveAs ? s.project.name : name };
    const saved = await backend.saveProject(path, doc);
    const p = migrate(saved);
    useStore.setState({ project: p, savedProject: p, projectPath: path });
    localStorage.setItem(LAST_PROJECT_KEY, path);
    toast("Project saved", "success");
    return true;
  } catch (e) {
    toast(`Save failed: ${e}`, "error", 6000);
    return false;
  }
}

export async function exportMixdown(opts: { sampleRate: number; bits: number; range: "song" | "loop" }) {
  const s = useStore.getState();
  const p = s.project;
  const end = Math.max(...p.tracks.flatMap((t) => t.clips.map((c) => c.start + c.length)), 0);
  if (end <= 0 && opts.range === "song") {
    toast("Nothing to export yet.", "error");
    return;
  }
  const [start, stop] = opts.range === "loop" ? [p.loop.start, p.loop.end] : [0, end];
  const dirs = await backend.defaultDirs();
  const path = !isTauri
    ? `${p.name || "Mixdown"}.wav` // browser: downloaded
    : await saveDialog({
        title: "Export mixdown",
        defaultPath: `${dirs.exports}/${p.name || "Mixdown"}.wav`,
        filters: [{ name: "WAV audio", extensions: ["wav"] }],
      });
  if (!path) return;
  useStore.setState({ exportProgress: 0 });
  try {
    await backend.exportAudio(path.endsWith(".wav") ? path : `${path}.wav`, p, {
      sampleRate: opts.sampleRate,
      bits: opts.bits,
      start,
      end: stop,
      tail: 2.5,
    });
    toast(`Exported to ${path.split("/").pop()}`, "success", 5000);
  } catch (e) {
    toast(`Export failed: ${e}`, "error", 6000);
  } finally {
    useStore.setState({ exportProgress: null, dialog: null });
  }
}

// ---------------------------------------------------------------- autosave / restore

export function startAutosave() {
  let last: Project | null = null;
  // Plugin sounds edited in their own windows are not in the project until captured.
  setInterval(capturePluginStates, 20_000);
  setInterval(() => {
    const p = useStore.getState().project;
    if (p !== last) {
      last = p;
      try {
        localStorage.setItem(AUTOSAVE_KEY, JSON.stringify({ project: p, path: useStore.getState().projectPath }));
      } catch {
        /* quota */
      }
    }
  }, 4000);
}

export function restoreSession(): boolean {
  try {
    // Sessions saved before the rename are picked up once.
    const raw = localStorage.getItem(AUTOSAVE_KEY) ?? localStorage.getItem("brioche:autosave");
    if (!raw) return false;
    const { project, path } = JSON.parse(raw);
    if (!project?.tracks) return false;
    loadDocument(project, path ?? null);
    useStore.setState({ savedProject: null });
    return true;
  } catch {
    return false;
  }
}
