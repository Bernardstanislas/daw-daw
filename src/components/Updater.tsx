// Auto-update: checks GitHub releases (latest.json), offers the new version,
// then downloads, installs and relaunches the app on its own.

import { useEffect, useRef, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";
import { isTauri } from "../lib/backend";

const FIRST_CHECK_MS = 5_000;
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

type Phase =
  | { kind: "idle" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; done: number; total: number | null }
  | { kind: "restarting"; update: Update }
  | { kind: "error"; update: Update; message: string };

export function Updater() {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const dismissed = useRef<string | null>(null);
  const busy = useRef(false);

  useEffect(() => {
    // Dev builds and the browser version never update themselves.
    if (!isTauri || import.meta.env.DEV) return;
    const check = async () => {
      if (busy.current) return;
      try {
        const { check } = await import("@tauri-apps/plugin-updater");
        const update = await check();
        if (update && update.version !== dismissed.current && !busy.current) setPhase({ kind: "available", update });
      } catch (e) {
        console.warn("[updater] check failed:", e);
      }
    };
    const first = setTimeout(check, FIRST_CHECK_MS);
    const every = setInterval(check, CHECK_EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, []);

  if (phase.kind === "idle") return null;
  const { update } = phase;

  const install = async () => {
    busy.current = true;
    let done = 0;
    let total: number | null = null;
    setPhase({ kind: "downloading", update, done, total });
    try {
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? null;
        else if (ev.event === "Progress") done += ev.data.chunkLength;
        setPhase({ kind: "downloading", update, done, total });
      });
      setPhase({ kind: "restarting", update });
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (e) {
      busy.current = false;
      setPhase({ kind: "error", update, message: String(e) });
    }
  };

  const later = () => {
    dismissed.current = update.version;
    setPhase({ kind: "idle" });
  };

  const pct = phase.kind === "downloading" && phase.total ? Math.min(100, (phase.done / phase.total) * 100) : null;

  return (
    <div className="update-card" role="status">
      <div className="update-title">
        {phase.kind === "error" ? "Update failed" : phase.kind === "restarting" ? "Restarting…" : `daw-daw ${update.version} is available`}
      </div>
      {phase.kind === "available" && (
        <>
          <div className="update-sub">You have {update.currentVersion}. The app restarts once the update is installed — save your project first.</div>
          {update.body && <div className="update-notes">{update.body}</div>}
          <div className="modal-actions">
            <button className="btn" onClick={later}>
              Later
            </button>
            <button className="btn primary" onClick={install}>
              Update &amp; restart
            </button>
          </div>
        </>
      )}
      {phase.kind === "downloading" && (
        <>
          <div className="update-sub">{pct == null ? "Downloading…" : pct >= 100 ? "Installing…" : `Downloading… ${Math.round(pct)}%`}</div>
          <div className="update-bar">
            <div className={`update-bar-fill ${pct == null ? "indeterminate" : ""}`} style={pct == null ? undefined : { width: `${pct}%` }} />
          </div>
        </>
      )}
      {phase.kind === "error" && (
        <>
          <div className="update-sub">{phase.message}</div>
          <div className="modal-actions">
            <button className="btn" onClick={later}>
              Dismiss
            </button>
            <button className="btn primary" onClick={install}>
              Retry
            </button>
          </div>
        </>
      )}
    </div>
  );
}
