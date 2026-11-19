import { useEffect, useRef } from "react";
import { setUi, useStore } from "./lib/store";
import { TopBar } from "./components/TopBar";
import { Browser } from "./components/Browser";
import { Inspector } from "./components/Inspector";
import { Arrange } from "./components/arrange/Arrange";
import { Editor } from "./components/editor/Editor";
import { Mixer } from "./components/Mixer";
import { Dialogs } from "./components/Dialogs";
import { ContextMenuHost } from "./components/Menu";
import { PromptHost } from "./components/Prompt";
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
      <div className="bottom-tabs">
        <button className={`panel-tab ${bottom === "editor" ? "on" : ""}`} onClick={() => setUi({ bottom: "editor" })}>
          Editor
        </button>
        <button className={`panel-tab ${bottom === "mixer" ? "on" : ""}`} onClick={() => setUi({ bottom: "mixer" })}>
          Mixer
        </button>

        <div className="spacer" />
        <button className="panel-tab" onClick={() => setUi({ bottom: null })} title="Hide (E / X)">
          ✕
        </button>
      </div>
      <div className="bottom-content">{bottom === "editor" ? <Editor /> : <Mixer />}</div>
    </div>
  );
}

/** Drag handle on the inner edge of a side panel (double-click: default width). */
function PanelResizer({ side }: { side: "left" | "right" }) {
  const key = side === "left" ? "browserWidth" : "inspectorWidth";
  const [min, max, def] = side === "left" ? [180, 560, 250] : [260, 720, 320];
  const onPointerDown = (e: React.PointerEvent) => {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const x0 = e.clientX;
    const w0 = useStore.getState().ui[key];
    document.body.classList.add("resizing-x");
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - x0;
      setUi({ [key]: clamp(side === "left" ? w0 + dx : w0 - dx, min, Math.min(max, window.innerWidth - 500)) });
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing-x");
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };
  return (
    <div
      className={`panel-resizer ${side}`}
      onPointerDown={onPointerDown}
      onDoubleClick={() => setUi({ [key]: def })}
      title="Drag to resize · double-click: default width"
    />
  );
}

export default function App() {
  const browser = useStore((s) => s.ui.browser);
  const inspector = useStore((s) => s.ui.inspector);
  const browserWidth = useStore((s) => s.ui.browserWidth);
  const inspectorWidth = useStore((s) => s.ui.inspectorWidth);
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
      <div className="main" style={{ gridTemplateColumns: `${browser ? `${browserWidth}px ` : ""}1fr${inspector ? ` ${inspectorWidth}px` : ""}` }}>
        {browser && (
          <div className="side-panel">
            <Browser />
            <PanelResizer side="left" />
          </div>
        )}
        <div className="center">
          <Arrange />
          <BottomPanel />
        </div>
        {inspector && (
          <div className="side-panel">
            <Inspector />
            <PanelResizer side="right" />
          </div>
        )}
      </div>
      <StatusBar />
      <Dialogs />
      <ContextMenuHost />
      <PromptHost />
    </div>
  );
}
