import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource-variable/figtree";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import dseg7 from "dseg/fonts/DSEG7-Classic/DSEG7Classic-Bold.woff2?url";
import "./styles/app.css";

// 7-segment face for the displays (bundled: the app works offline).
document.fonts.add(new FontFace("DSEG7", `url(${dseg7})`));
import { loadCatalog } from "./lib/catalog";
import { initEngineBridge } from "./lib/engine";
import { initKeyboard } from "./lib/keyboard";
import { initMidiControl } from "./lib/midiMap";
import { initStepRecord, initStepRecordKeys } from "./lib/stepRecord";
import { loadDocument, restoreSession, startAutosave } from "./lib/fileOps";
import { TEMPLATES } from "./lib/templates";
import { useStore } from "./lib/store";
import { isTauri } from "./lib/backend";

async function boot() {
  await loadCatalog();
  if (!restoreSession()) {
    // First launch: open the demo song so there is something to hear.
    loadDocument(TEMPLATES.find((t) => t.id === "beat")!.build(), null);
    useStore.setState({ savedProject: null });
  }
  if (!isTauri) {
    // Browser build: run the Rust engine (WebAssembly) in an AudioWorklet.
    import("./lib/webEngine").then((w) => w.startWebEngine());
  }
  initEngineBridge();
  initKeyboard();
  initMidiControl();
  initStepRecord();
  initStepRecordKeys();
  startAutosave();
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

boot();
