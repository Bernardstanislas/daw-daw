// Generic device UI generated from the Rust DeviceSpec (knobs, choices, toggles),
// shown on the Sound page: parameter groups side by side, like the modules of a faceplate.

import { memo, useState } from "react";
import type { DeviceSpec, DeviceState, ParamSpec } from "../lib/types";
import { beginGesture, commit, endGesture, gesture } from "../lib/store";
import { applyPreset, draftDevice, moveEffect, removeEffect } from "../lib/actions";
import { Knob, formatParam } from "./controls";
import { IconArrowLeft, IconArrowRight, IconChevronDown, IconChevronRight, IconClose, IconPower } from "./icons";
import { isPluginType, useCatalog } from "../lib/catalog";
import { backend, isTauri } from "../lib/backend";
import { useStore } from "../lib/store";
import { useLearn } from "../lib/midiMap";

/** Steps to the previous / next preset, like the ‹ › keys of a hardware synth. */
function stepPreset(presets: { name: string }[], device: DeviceState, dir: number) {
  const names = ["Default", ...presets.map((p) => p.name).filter((n) => n !== "Default")];
  const i = Math.max(0, names.indexOf(device.preset ?? "Default"));
  applyPreset(device.id, names[(i + dir + names.length) % names.length]);
}

function setParamGesture(deviceId: string, pid: string, v: number) {
  gesture((p) => {
    const d = draftDevice(p, deviceId);
    if (d) d.params[pid] = v;
  });
}

function setParamCommit(deviceId: string, pid: string, v: number) {
  commit((p) => {
    const d = draftDevice(p, deviceId);
    if (d) d.params[pid] = v;
  });
}

function ParamControl({ device, spec }: { device: DeviceState; spec: ParamSpec }) {
  const v = device.params[spec.id] ?? spec.default;
  const target = { kind: "param", deviceId: device.id, paramId: spec.id } as const;
  const learn = useLearn(target);
  if (spec.kind === "choice") {
    return (
      <label className="param choice" title={spec.name} {...learn}>
        <span className="param-name">{spec.name}</span>
        <select value={Math.round(v)} onChange={(e) => setParamCommit(device.id, spec.id, +e.target.value)}>
          {spec.choices.map((c, i) => (
            <option key={i} value={i}>
              {c}
            </option>
          ))}
        </select>
      </label>
    );
  }
  if (spec.kind === "toggle") {
    return (
      <div className="param toggle">
        <button className={`toggle-btn ${v >= 0.5 ? "on" : ""}`} onClick={() => setParamCommit(device.id, spec.id, v >= 0.5 ? 0 : 1)} {...learn}>
          {spec.name}
        </button>
      </div>
    );
  }
  return (
    <div className="param knob-param">
      <Knob
        value={v}
        min={spec.min}
        max={spec.max}
        defaultValue={spec.default}
        log={spec.kind === "log"}
        step={spec.kind === "int" ? 1 : undefined}
        size={46}
        label={spec.name}
        display={formatParam(spec, v)}
        onBegin={beginGesture}
        onChange={(x) => setParamGesture(device.id, spec.id, x)}
        onEnd={endGesture}
        title={`${spec.name}: ${formatParam(spec, v)} (double-click: reset)`}
        learn={target}
      />
    </div>
  );
}

function groupParams(spec: DeviceSpec): [string, ParamSpec[]][] {
  const map = new Map<string, ParamSpec[]>();
  for (const p of spec.params) {
    const g = p.group || "";
    if (!map.has(g)) map.set(g, []);
    map.get(g)!.push(p);
  }
  return [...map.entries()];
}

/** Columns of a group: one row up to 4 controls, two rows beyond (a choice is 2 wide). */
function groupCols(params: ParamSpec[]): number {
  const units = params.reduce((n, p) => n + (p.kind === "choice" ? 2 : 1), 0);
  return units <= 4 ? units : Math.min(6, Math.ceil(units / 2));
}

export const DevicePanel = memo(function DevicePanel({
  device,
  target,
  isInstrument,
}: {
  device: DeviceState;
  target: string | "master";
  isInstrument?: boolean;
}) {
  const spec = useCatalog((s) => s.specs.find((x) => x.typeId === device.type));
  const [open, setOpen] = useState(true);
  if (!spec && isPluginType(device.type)) {
    return <PluginPanel device={device} target={target} isInstrument={isInstrument} />;
  }
  if (!spec) {
    return (
      <div className="device missing">
        Unknown device “{device.type}”
        {!isInstrument && (
          <button className="icon-btn" onClick={() => removeEffect(target, device.id)}>
            <IconClose />
          </button>
        )}
      </div>
    );
  }
  const groups = groupParams(spec);
  const presetPicker = (
    <>
      <button
        className="icon-btn tiny"
        onClick={() => stepPreset(spec.presets, device, -1)}
        title="Previous preset"
      >
        ‹
      </button>
      <select className="preset-select" value={device.preset ?? "Default"} onChange={(e) => applyPreset(device.id, e.target.value)} title="Preset">
        <option value="Default">Default</option>
        {spec.presets
          .filter((p) => p.name !== "Default")
          .map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
      </select>
      <button
        className="icon-btn tiny"
        onClick={() => stepPreset(spec.presets, device, 1)}
        title="Next preset"
      >
        ›
      </button>
    </>
  );
  return (
    <div className={`device ${device.enabled ? "" : "bypassed"} ${isInstrument ? "instrument" : ""}`}>
      <div className="device-header">
        <button className="icon-btn tiny" onClick={() => setOpen(!open)} title={open ? "Collapse" : "Expand"}>
          {open ? <IconChevronDown size={12} /> : <IconChevronRight size={12} />}
        </button>
        {!isInstrument && (
          <button
            className={`icon-btn tiny power ${device.enabled ? "on" : ""}`}
            onClick={() =>
              commit((p) => {
                const d = draftDevice(p, device.id);
                if (d) d.enabled = !d.enabled;
              })
            }
            title={device.enabled ? "Bypass" : "Enable"}
          >
            <IconPower size={12} />
          </button>
        )}
        <span className="device-name" title={spec.description}>
          {spec.name}
        </span>
        {!isInstrument && <span className="device-spacer" />}
        {isInstrument && presetPicker}
        {!isInstrument && (
          <>
            <button className="icon-btn tiny move" onClick={() => moveEffect(target, device.id, -1)} title="Move earlier in the chain">
              <IconArrowLeft size={12} />
            </button>
            <button className="icon-btn tiny move" onClick={() => moveEffect(target, device.id, 1)} title="Move later in the chain">
              <IconArrowRight size={12} />
            </button>
            <button className="icon-btn tiny" onClick={() => removeEffect(target, device.id)} title="Remove">
              <IconClose size={12} />
            </button>
          </>
        )}
      </div>
      {open && !isInstrument && <div className="device-presets">{presetPicker}</div>}
      {open && (
        <div className="device-body">
          {groups.map(([g, params]) => (
            <div className="param-group" key={g} style={{ ["--cols" as string]: groupCols(params) }}>
              {g && !(params.length === 1 && params[0].name === g) && <div className="param-group-title">{g}</div>}
              <div className="param-grid">
                {params.map((ps) => (
                  <ParamControl key={ps.id} device={device} spec={ps} />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
});

/** A third-party plugin: its controls live in its own editor window. */
function PluginPanel({ device, target, isInstrument }: { device: DeviceState; target: string | "master"; isInstrument?: boolean }) {
  const plugin = useCatalog((s) => s.plugins.find((p) => p.id === device.type));
  const trackName = useStore((s) => (target === "master" ? "Master" : s.project.tracks.find((t) => t.id === target)?.name ?? ""));
  const [error, setError] = useState<string | null>(null);
  const name = plugin?.name ?? device.label ?? "Plugin";
  const openEditor = async () => {
    setError(null);
    try {
      await backend.pluginOpenEditor(device.id, `${name} — ${trackName}`);
    } catch (e) {
      setError(String(e));
    }
  };
  return (
    <div className={`device plugin ${device.enabled ? "" : "bypassed"} ${isInstrument ? "instrument" : ""}`}>
      <div className="device-header">
        {!isInstrument && (
          <button
            className={`icon-btn tiny power ${device.enabled ? "on" : ""}`}
            onClick={() =>
              commit((p) => {
                const d = draftDevice(p, device.id);
                if (d) d.enabled = !d.enabled;
              })
            }
            title={device.enabled ? "Bypass" : "Enable"}
          >
            <IconPower size={12} />
          </button>
        )}
        <span className="device-name" title={plugin ? `${plugin.vendor} · ${plugin.format}` : "Plugin"}>
          {name}
        </span>
        {!isInstrument && <span className="device-spacer" />}
        <span className="plugin-vendor">{plugin ? `${plugin.vendor} · ${plugin.format}` : isTauri ? "not installed" : "desktop only"}</span>
        {!isInstrument && (
          <>
            <button className="icon-btn tiny move" onClick={() => moveEffect(target, device.id, -1)} title="Move earlier in the chain">
              <IconArrowLeft size={12} />
            </button>
            <button className="icon-btn tiny move" onClick={() => moveEffect(target, device.id, 1)} title="Move later in the chain">
              <IconArrowRight size={12} />
            </button>
            <button className="icon-btn tiny" onClick={() => removeEffect(target, device.id)} title="Remove">
              <IconClose size={12} />
            </button>
          </>
        )}
      </div>
      <div className="device-body plugin-body">
        <button className="btn primary" onClick={openEditor} disabled={!plugin} title="Open the plugin in its own window (Ctrl+Tab switches windows)">
          Open editor
        </button>
        <span className="muted-text">Sounds and settings are edited in the plugin window and saved with the project.</span>
        {error && <span className="plugin-error">{error}</span>}
      </div>
    </div>
  );
}
