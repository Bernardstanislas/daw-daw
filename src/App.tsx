import { useEffect, useRef } from "react";
import { setUi, toast, useStore } from "./lib/store";
import { backend, isTauri } from "./lib/backend";
import { TopBar } from "./components/TopBar";
import { Browser } from "./components/Browser";
import { Arrange } from "./components/arrange/Arrange";
import { Editor } from "./components/editor/Editor";
import { Mixer } from "./components/Mixer";
import { Sound } from "./components/Sound";
import { Dialogs } from "./components/Dialogs";
import { ContextMenuHost } from "./components/Menu";
import { PromptHost } from "./components/Prompt";
import { Updater } from "./components/Updater";
import { StatusBar } from "./components/StatusBar";
import { clamp, capturePointer } from "./lib/music";

function BottomPanel() {
  const bottom = useStore((s) => s.ui.bottom);
  const height = useStore((s) => s.ui.bottomHeight);
  const ref = useRef<HTMLDivElement>(null);
  if (!bottom) return null;
  const onResize = (e: React.PointerEvent) => {
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const y0 = e.clientY;
    const h0 = height;
    const move = (ev: PointerEvent) => setUi({ bottomHeight: clamp(h0 - (ev.clientY - y0), 140, window.innerHeight - 220) });
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };
  return (
    <div className="bottom-panel" style={{ height }} ref={ref}>
      <div className="bottom-resize" onPointerDown={onResize} />
      <div className="bottom-content">
        <Editor />
      </div>
    </div>
  );
}

/** Drag handle on the inner edge of the browser (double-click: default width). */
function BrowserResizer() {
  const onPointerDown = (e: React.PointerEvent) => {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const x0 = e.clientX;
    const w0 = useStore.getState().ui.browserWidth;
    document.body.classList.add("resizing-x");
    const move = (ev: PointerEvent) => setUi({ browserWidth: clamp(w0 + ev.clientX - x0, 180, Math.min(560, window.innerWidth - 500)) });
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing-x");
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };
  return <div className="panel-resizer left" onPointerDown={onPointerDown} onDoubleClick={() => setUi({ browserWidth: 250 })} title="Drag to resize · double-click: default width" />;
}

export default function App() {
  const page = useStore((s) => s.ui.page);
  const browser = useStore((s) => s.ui.browser);
  const browserWidth = useStore((s) => s.ui.browserWidth);
  useEffect(() => {
    // The sampler's sample set lives outside the app (proprietary audio): say when it is missing.
    if (!isTauri) return;
    backend
      .getAudioStatus()
      .then((st) => {
        if (st.samples.missing.length) toast(`Sampler: ${st.samples.missing.length} samples missing in ${st.samples.dir} — set the folder in Audio & MIDI settings`, "error", 9000);
      })
      .catch(() => {});
  }, []);
  useEffect(() => {
    const prevent = (e: MouseEvent) => {
      // Native context menu only in text fields.
      const t = e.target as HTMLElement;
      if (!(t.tagName === "INPUT" || t.tagName === "TEXTAREA")) e.preventDefault();
    };
    // Keep keyboard shortcuts working: buttons / selects must not keep focus
    // (Space would re-trigger the last clicked button, arrows would change selects).
    const blurButtons = () => {
      const a = document.activeElement as HTMLElement | null;
      if (a && (a.tagName === "BUTTON" || (a.tagName === "INPUT" && (a as HTMLInputElement).type === "range"))) a.blur();
    };
    const blurSelect = (e: Event) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "SELECT") t.blur();
    };
    window.addEventListener("contextmenu", prevent);
    window.addEventListener("pointerup", blurButtons);
    window.addEventListener("change", blurSelect, true);
    return () => {
      window.removeEventListener("contextmenu", prevent);
      window.removeEventListener("pointerup", blurButtons);
      window.removeEventListener("change", blurSelect, true);
    };
  }, []);
  return (
    <div className="app">
      <TopBar />
      <div className="main" style={{ gridTemplateColumns: `${browser ? `${browserWidth}px ` : ""}1fr` }}>
        {browser && (
          <div className="side-panel">
            <Browser />
            <BrowserResizer />
          </div>
        )}
        <div className="center">
          {page === "arrange" && (
            <>
              <Arrange />
              <BottomPanel />
            </>
          )}
          {page === "sound" && <Sound />}
          {page === "mix" && <Mixer />}
        </div>
      </div>
      <StatusBar />
      <Dialogs />
      <ContextMenuHost />
      <PromptHost />
      <Updater />
    </div>
  );
}
