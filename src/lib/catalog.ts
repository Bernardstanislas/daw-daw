// Device catalog (instruments & effects), provided by the Rust engine.

import { create } from "zustand";
import { backend } from "./backend";
import type { DeviceSpec, DeviceState, PluginInfo } from "./types";
import { uid } from "./music";

interface CatalogState {
  specs: DeviceSpec[];
  /** Installed third-party plugins (desktop). */
  plugins: PluginInfo[];
  loaded: boolean;
}

export const useCatalog = create<CatalogState>(() => ({ specs: [], plugins: [], loaded: false }));

export async function loadCatalog() {
  try {
    const specs = await backend.getCatalog();
    useCatalog.setState({ specs, loaded: true });
  } catch (e) {
    console.error("catalog", e);
    useCatalog.setState({ loaded: true });
  }
  loadPlugins(false);
}

/** Scans installed plugins (in the background: the first scan can take a moment). */
export async function loadPlugins(rescan: boolean) {
  try {
    const plugins = await backend.listPlugins(rescan);
    useCatalog.setState({ plugins });
    return plugins;
  } catch (e) {
    console.error("plugins", e);
    return [];
  }
}

export function getPlugin(type: string): PluginInfo | undefined {
  return useCatalog.getState().plugins.find((p) => p.id === type);
}

export function isPluginType(type: string): boolean {
  return type.startsWith("au:");
}

/** Display name of any device (built-in spec, plugin, or saved label). */
export function deviceName(d: { type: string; label?: string; preset?: string }): string {
  const spec = getSpec(d.type);
  if (spec) return spec.name;
  return getPlugin(d.type)?.name ?? d.label ?? d.type;
}

export function getSpec(type: string): DeviceSpec | undefined {
  return useCatalog.getState().specs.find((s) => s.typeId === type);
}

export function instrumentSpecs() {
  return useCatalog.getState().specs.filter((s) => s.kind === "instrument");
}

export function effectSpecs() {
  return useCatalog.getState().specs.filter((s) => s.kind === "effect");
}

export function defaultParams(spec: DeviceSpec): Record<string, number> {
  const out: Record<string, number> = {};
  for (const p of spec.params) out[p.id] = p.default;
  return out;
}

export function presetParams(spec: DeviceSpec, presetName?: string): Record<string, number> {
  const out = defaultParams(spec);
  const preset = spec.presets.find((p) => p.name === presetName);
  if (preset) for (const [k, v] of preset.values) if (k in out) out[k] = v;
  return out;
}

export function makeDevice(type: string, presetName?: string): DeviceState {
  const spec = getSpec(type);
  const plugin = spec ? undefined : getPlugin(type);
  return {
    id: uid("dev"),
    type,
    enabled: true,
    params: spec ? presetParams(spec, presetName) : {},
    preset: presetName ?? (spec ? "Default" : undefined),
    ...(plugin ? { label: plugin.name } : {}),
  };
}

/** Drum-style note names for an instrument (empty map = keyboard). */
export function noteNamesFor(type: string | undefined): Map<number, string> {
  const spec = type ? getSpec(type) : undefined;
  return new Map(spec?.noteNames ?? []);
}
