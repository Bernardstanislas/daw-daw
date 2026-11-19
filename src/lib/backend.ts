// Thin wrapper around the Tauri IPC. When the UI runs in a plain browser
// (vite dev without Tauri) the engine runs as WebAssembly in the page (webEngine.ts).

import type { AssetInfo, AssetRef, DeviceSpec, PluginInfo, Project } from "./types";
import catalogFixture from "./catalog.fixture.json";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Unlisten = () => void;

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function tauriListen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => cb(e.payload));
}

export interface AudioStatus {
  output: { device: string; sampleRate: number; channels: number; bufferSize: number | null } | null;
  input: [number, number] | null;
  devices: { outputs: string[]; inputs: string[]; defaultOutput: string | null; defaultInput: string | null };
  settings: AudioSettings;
  midiPorts: string[];
  error: string | null;
  baseDir: string;
}

export interface AudioSettings {
  outputDevice: string | null;
  inputDevice: string | null;
  bufferSize: number | null;
  recordOffsetMs: number;
}

export interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
}

export interface RecordedNote {
  trackId: string;
  pitch: number;
  velocity: number;
  on: boolean;
  beat: number;
}

export interface AudioRecorded {
  trackId: string;
  startBeat: number;
  asset: AssetInfo;
}

export interface ExportOptions {
  sampleRate: number;
  bits: number;
  start: number;
  end: number;
  tail: number;
}

// ------------------------------------------------------------------ browser

// In a plain browser the Rust engine runs as WebAssembly in an AudioWorklet
// (see webEngine.ts); desktop-only features (files on disk, audio input) are stubbed.
const web = (() => {
  const notInBrowser = (what: string) => Promise.reject(new Error(`${what} is only available in the desktop app`));
  return {
    async invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
      const W = await import("./webEngine");
      switch (cmd) {
        case "get_catalog":
          return catalogFixture as unknown as T;
        case "sync_project":
          W.syncProject(args.project as Project);
          return undefined as T;
        case "transport": {
          const a = args.action as string;
          W.transport(a === "play" ? 0 : a === "stop" ? 1 : a === "seek" ? 2 : 4, (args.position as number) ?? 0);
          return undefined as T;
        }
        case "record":
          if (args.audioTrack) return notInBrowser("Audio recording");
          W.transport(3);
          return undefined as T;
        case "set_input_enabled":
          return (args.enabled ? notInBrowser("Audio input") : undefined) as T;
        case "live_note":
          W.liveNote(args.pitch as number, args.velocity as number, args.on as boolean, (args.trackId as string | null) ?? null);
          return undefined as T;
        case "set_live_target":
          W.setLiveTarget((args.trackId as string | null) ?? null);
          return undefined as T;
        case "load_assets": {
          const list = args.assets as { id: string }[];
          return list.map((a) => {
            const info = W.assetInfo(a.id);
            return { id: a.id, info, error: info ? null : "not loaded in this browser session (re-import the file)" };
          }) as T;
        }
        case "get_audio_status": {
          const st = W.webEngineState();
          return {
            output: st.ready ? { device: `Web Audio${st.running ? "" : " (click to start)"}`, sampleRate: st.sampleRate, channels: 2, bufferSize: 128 } : null,
            input: null,
            devices: { outputs: ["Web Audio"], inputs: [], defaultOutput: "Web Audio", defaultInput: null },
            settings: { outputDevice: null, inputDevice: null, bufferSize: null, recordOffsetMs: 0 },
            midiPorts: st.midiPorts,
            error: st.failure,
            baseDir: "(browser)",
          } as T;
        }
        case "set_audio_settings":
          return null as T;
        case "rescan_midi":
          return W.rescanMidi() as T;
        case "export_audio": {
          const path = (args.path as string) || "mixdown.wav";
          await W.exportWav(path.split("/").pop()!, args.project as Project, args.options as ExportOptions);
          return undefined as T;
        }
        case "default_dirs":
          return { base: "", projects: "", recordings: "", exports: "", home: "", music: "" } as T;
        case "list_dir":
          return [] as T;
        case "preview_file":
          return undefined as T;
        case "import_audio":
        case "save_project":
        case "load_project":
        case "library_dir":
          return notInBrowser("Files on disk");
        case "list_plugins":
          return [] as T;
        case "plugin_states":
          return {} as T;
        case "plugin_open_editor":
          return notInBrowser("Plugins");
        default:
          return undefined as T;
      }
    },
    listen<T>(event: string, cb: (p: T) => void): Unlisten {
      let off: (() => void) | null = null;
      let cancelled = false;
      import("./webEngine").then((W) => {
        if (!cancelled) off = W.on(event, cb as (p: unknown) => void);
      });
      return () => {
        cancelled = true;
        off?.();
      };
    },
  };
})();

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return isTauri ? tauriInvoke<T>(cmd, args) : web.invoke<T>(cmd, args);
}

export async function listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  return isTauri ? tauriListen<T>(event, cb) : web.listen<T>(event, cb);
}

// ------------------------------------------------------------------ API

export const backend = {
  getCatalog: () => invoke<DeviceSpec[]>("get_catalog"),
  syncProject: (project: Project) => invoke<void>("sync_project", { project }),
  play: () => invoke<void>("transport", { action: "play" }),
  stop: () => invoke<void>("transport", { action: "stop" }),
  seek: (position: number) => invoke<void>("transport", { action: "seek", position }),
  panic: () => invoke<void>("transport", { action: "panic" }),
  record: (audioTrack: string | null, takeName: string | null) => invoke<void>("record", { audioTrack, takeName }),
  setInputEnabled: (enabled: boolean) => invoke<void>("set_input_enabled", { enabled }),
  liveNote: (pitch: number, velocity: number, on: boolean, trackId: string | null = null) =>
    invoke<void>("live_note", { pitch, velocity, on, trackId }),
  setLiveTarget: (trackId: string | null) => invoke<void>("set_live_target", { trackId }),
  importAudio: (id: string, path: string) => invoke<AssetInfo>("import_audio", { id, path }),
  loadAssets: (assets: AssetRef[]) =>
    invoke<{ id: string; info: AssetInfo | null; error: string | null }[]>("load_assets", {
      assets: assets.map((a) => ({ id: a.id, path: a.path, name: a.name })),
    }),
  previewFile: (path: string | null) => invoke<void>("preview_file", { path }),
  saveProject: (path: string, project: Project) => invoke<Project>("save_project", { path, project }),
  loadProject: (path: string) => invoke<{ path: string; project: Project }>("load_project", { path }),
  exportAudio: (path: string, project: Project, options: ExportOptions) =>
    invoke<void>("export_audio", { path, project, options }),
  getAudioStatus: () => invoke<AudioStatus>("get_audio_status"),
  setAudioSettings: (settings: AudioSettings) => invoke<AudioStatus["output"]>("set_audio_settings", { settings }),
  rescanMidi: () => invoke<string[]>("rescan_midi"),
  listDir: (path: string) => invoke<DirEntry[]>("list_dir", { path }),
  defaultDirs: () => invoke<Record<string, string>>("default_dirs"),
  libraryDir: () => invoke<string>("library_dir"),
  listPlugins: (rescan = false) => invoke<PluginInfo[]>("list_plugins", { rescan }),
  pluginOpenEditor: (deviceId: string, title: string) => invoke<void>("plugin_open_editor", { deviceId, title }),
  pluginStates: () => invoke<Record<string, string>>("plugin_states"),
};

// ------------------------------------------------------------------ dialogs

export async function openDialog(opts: {
  directory?: boolean;
  multiple?: boolean;
  filters?: { name: string; extensions: string[] }[];
  defaultPath?: string;
  title?: string;
}): Promise<string[] | null> {
  if (!isTauri) {
    const p = window.prompt(`${opts.title ?? "Open"} — path:`);
    return p ? [p] : null;
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ ...opts });
  if (res == null) return null;
  return Array.isArray(res) ? res : [res];
}

export async function saveDialog(opts: {
  defaultPath?: string;
  filters?: { name: string; extensions: string[] }[];
  title?: string;
}): Promise<string | null> {
  if (!isTauri) {
    return window.prompt(`${opts.title ?? "Save"} — path:`, opts.defaultPath ?? "");
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({ ...opts });
}

export async function confirmDialog(message: string, title = "daw-daw"): Promise<boolean> {
  if (!isTauri) return window.confirm(message);
  const { ask } = await import("@tauri-apps/plugin-dialog");
  return ask(message, { title, kind: "warning" });
}
