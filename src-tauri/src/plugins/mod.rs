//! Third-party plugin hosting (desktop only). Audio Units on macOS (Vital,
//! Surge, u-he, Serum…) and VST3 on every desktop OS. The engine sees plugins
//! through `daw_core::dsp::registry::ExternalDevices`, so other formats (CLAP)
//! can be added later behind the same interface.

#[cfg(target_os = "macos")]
pub mod au;
#[cfg(target_os = "macos")]
pub mod ffi;
#[cfg(target_os = "macos")]
pub mod ui;
pub mod vst3;
#[cfg(target_os = "windows")]
pub mod vst3_win;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use parking_lot::Mutex;
use serde::Serialize;

use daw_core::dsp::registry::ExternalDevices;
use daw_core::dsp::{Effect, Instrument};
use daw_core::model::{DeviceState, Project};

/// An installed plugin (any format).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    /// "au:<type><subtype><manufacturer>" (hex four-char codes) or "vst3:<class id, 32 hex>".
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// "instrument" | "effect"
    pub kind: &'static str,
    /// "AU" | "VST3"
    pub format: &'static str,
    /// Where a VST3 lives (the `.vst3` bundle or file).
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// A live plugin instance, whatever its format.
#[derive(Clone)]
enum Live {
    #[cfg(target_os = "macos")]
    Au(Arc<au::AuUnit>),
    Vst3(Arc<vst3::Vst3Unit>),
}

impl Live {
    /// Main thread.
    fn create(info: PluginInfo, sample_rate: f32, offline: bool) -> anyhow::Result<Live> {
        match info.format {
            #[cfg(target_os = "macos")]
            "AU" => Ok(Live::Au(au::AuUnit::new(info, sample_rate)?)),
            "VST3" => Ok(Live::Vst3(vst3::Vst3Unit::new(info, sample_rate, offline)?)),
            f => anyhow::bail!("unsupported plugin format {f}"),
        }
    }

    fn get_state(&self) -> Option<Vec<u8>> {
        match self {
            #[cfg(target_os = "macos")]
            Live::Au(u) => u.get_state(),
            Live::Vst3(u) => u.get_state(),
        }
    }

    fn set_state(&self, bytes: &[u8]) -> anyhow::Result<()> {
        match self {
            #[cfg(target_os = "macos")]
            Live::Au(u) => u.set_state(bytes),
            Live::Vst3(u) => u.set_state(bytes),
        }
    }

    fn instrument(&self) -> Box<dyn Instrument> {
        match self {
            #[cfg(target_os = "macos")]
            Live::Au(u) => Box::new(au::AuDevice::new(u.clone())),
            Live::Vst3(u) => Box::new(vst3::Vst3Device::new(u.clone())),
        }
    }

    fn effect(&self) -> Box<dyn Effect> {
        match self {
            #[cfg(target_os = "macos")]
            Live::Au(u) => Box::new(au::AuDevice::new(u.clone())),
            Live::Vst3(u) => Box::new(vst3::Vst3Device::new(u.clone())),
        }
    }

    /// Identity of the underlying instance (tests).
    #[cfg(all(test, target_os = "macos"))]
    fn ptr(&self) -> usize {
        match self {
            #[cfg(target_os = "macos")]
            Live::Au(u) => Arc::as_ptr(u) as usize,
            Live::Vst3(u) => Arc::as_ptr(u) as usize,
        }
    }

    /// Opens the plugin's editor window. Main thread.
    fn open_editor(&self, title: &str) -> Result<Editor, String> {
        match self {
            #[cfg(target_os = "macos")]
            Live::Au(u) => unsafe { ui::EditorWindow::open(u.unit, title) }.map(Editor::Au).map_err(|e| e.to_string()),
            #[cfg(target_os = "macos")]
            Live::Vst3(u) => unsafe { ui::Vst3EditorWindow::open(u.clone(), title) }.map(Editor::Vst3).map_err(|e| e.to_string()),
            #[cfg(target_os = "windows")]
            Live::Vst3(u) => vst3_win::EditorWindow::open(u.clone(), title).map(Editor::Win).map_err(|e| e.to_string()),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Live::Vst3(_) => {
                let _ = title;
                Err("plugin editors are not supported on this platform yet".into())
            }
        }
    }
}

/// An open plugin editor window. Main thread only.
enum Editor {
    #[cfg(target_os = "macos")]
    Au(ui::EditorWindow),
    #[cfg(target_os = "macos")]
    Vst3(ui::Vst3EditorWindow),
    #[cfg(target_os = "windows")]
    Win(vst3_win::EditorWindow),
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Unsupported(std::convert::Infallible),
}

impl Editor {
    fn show(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Editor::Au(w) => w.show(),
            #[cfg(target_os = "macos")]
            Editor::Vst3(w) => w.show(),
            #[cfg(target_os = "windows")]
            Editor::Win(w) => w.show(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Editor::Unsupported(n) => match *n {},
        }
    }

    /// Lets the window follow the plugin view's size.
    fn fit(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Editor::Au(w) => w.fit(),
            #[cfg(target_os = "macos")]
            Editor::Vst3(w) => w.fit(),
            #[cfg(target_os = "windows")]
            Editor::Win(w) => w.fit(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Editor::Unsupported(n) => match *n {},
        }
    }

    fn close(self) {
        match self {
            #[cfg(target_os = "macos")]
            Editor::Au(w) => w.close(),
            #[cfg(target_os = "macos")]
            Editor::Vst3(w) => w.close(),
            #[cfg(target_os = "windows")]
            Editor::Win(w) => w.close(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Editor::Unsupported(n) => match n {},
        }
    }

    /// Native window handle (NSWindow / HWND) and whether it is on screen.
    fn handle(&self) -> (usize, bool) {
        match self {
            #[cfg(target_os = "macos")]
            Editor::Au(w) => w.ns_window(),
            #[cfg(target_os = "macos")]
            Editor::Vst3(w) => w.ns_window(),
            #[cfg(target_os = "windows")]
            Editor::Win(w) => w.handle(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Editor::Unsupported(n) => match *n {},
        }
    }
}

/// Scanned plugins. `hidden` ones are not offered in the browser (a VST3 whose
/// Audio Unit twin is listed) but still load, e.g. from a project made elsewhere.
struct Catalog {
    shown: Vec<PluginInfo>,
    hidden: Vec<PluginInfo>,
}

#[derive(Default)]
pub struct PluginManager {
    catalog: Mutex<Option<Catalog>>,
    /// Device id → live instance (shared with the engine).
    live: Mutex<HashMap<String, Live>>,
    windows: Mutex<HashMap<String, Editor>>,
    /// Windows of instances that were replaced/removed, closed on the main thread.
    stale_windows: Mutex<Vec<String>>,
}

static MANAGER: OnceLock<Arc<PluginManager>> = OnceLock::new();

// ------------------------------------------------------------------ main thread
//
// Plugins (JUCE ones like Vital in particular) must be created and destroyed on
// the main thread, where their UI lives. Destroying one from the engine's
// clean-up thread corrupts Objective-C weak references.

type MainDispatch = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;
static MAIN: OnceLock<MainDispatch> = OnceLock::new();
/// The thread that installed the dispatcher (Tauri's setup runs on the main thread).
#[cfg(not(target_os = "macos"))]
static MAIN_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Installs the function that runs a closure on the main thread (Tauri).
/// Must be called from the main thread.
pub fn set_main_dispatcher(f: MainDispatch) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = MAIN_THREAD.set(std::thread::current().id());
    }
    let _ = MAIN.set(f);
}

pub fn is_main_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_main_np() -> i32;
        }
        unsafe { pthread_main_np() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Before a dispatcher is installed nothing can be dispatched anyway.
        MAIN_THREAD.get().is_none_or(|id| *id == std::thread::current().id())
    }
}

/// Runs `f` on the main thread and waits for its result. Without a dispatcher
/// (tests, tools) it runs in place.
pub fn on_main<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    match MAIN.get() {
        Some(dispatch) if !is_main_thread() => {
            let (tx, rx) = std::sync::mpsc::channel();
            dispatch(Box::new(move || {
                let _ = tx.send(f());
            }));
            rx.recv().expect("main thread dispatch")
        }
        _ => f(),
    }
}

/// Runs `f` on the main thread without waiting.
pub fn on_main_async(f: impl FnOnce() + Send + 'static) {
    match MAIN.get() {
        Some(dispatch) if !is_main_thread() => dispatch(Box::new(f)),
        _ => f(),
    }
}

pub fn manager() -> &'static Arc<PluginManager> {
    MANAGER.get_or_init(|| Arc::new(PluginManager::default()))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PluginStates {
    pub states: HashMap<String, String>,
}

impl PluginManager {
    /// Installed plugins (scanned once; `rescan` forces a new scan).
    pub fn list(&self, rescan: bool) -> Vec<PluginInfo> {
        self.with_catalog(rescan, |c| c.shown.clone())
    }

    fn with_catalog<R>(&self, rescan: bool, f: impl FnOnce(&Catalog) -> R) -> R {
        let mut c = self.catalog.lock();
        if rescan || c.is_none() {
            let mut shown: Vec<PluginInfo> = Vec::new();
            #[cfg(target_os = "macos")]
            shown.extend(au::scan());
            let mut hidden = Vec::new();
            for v in vst3::scan() {
                // Audio Units stay the preferred path on macOS: a plugin that
                // exists in both formats is listed once.
                if shown.iter().any(|a| a.format == "AU" && a.kind == v.kind && a.name.eq_ignore_ascii_case(&v.name)) {
                    hidden.push(v);
                } else {
                    shown.push(v);
                }
            }
            shown.sort_by(|a, b| (a.kind, a.vendor.to_lowercase(), a.name.to_lowercase()).cmp(&(b.kind, b.vendor.to_lowercase(), b.name.to_lowercase())));
            *c = Some(Catalog { shown, hidden });
        }
        f(c.as_ref().unwrap())
    }

    /// Forgets the scanned catalog (e.g. the plugin folders changed): the next `list` rescans.
    pub fn invalidate(&self) {
        *self.catalog.lock() = None;
    }

    fn info(&self, type_id: &str) -> Option<PluginInfo> {
        self.with_catalog(false, |c| c.shown.iter().chain(&c.hidden).find(|p| p.id == type_id).cloned())
    }

    fn instantiate(&self, d: &DeviceState, sample_rate: f32, offline: bool) -> Option<Live> {
        let info = self.info(&d.type_id)?;
        let name = info.name.clone();
        let unit = match on_main(move || Live::create(info, sample_rate, offline)) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[plugins] {name}: {e}");
                return None;
            }
        };
        // Restore: the live instance being replaced (sample-rate change,
        // re-creation) knows the freshest state; else the saved project state.
        let previous = self.live.lock().get(&d.id).cloned();
        let saved_b64 = d.state.clone();
        let target = unit.clone();
        on_main(move || {
            let saved = previous.and_then(|old| old.get_state()).or_else(|| saved_b64.and_then(|s| B64.decode(s).ok()));
            if let Some(bytes) = saved {
                if let Err(e) = target.set_state(&bytes) {
                    eprintln!("[plugins] {name}: {e}");
                }
            }
        });
        // Export renders use private instances and leave the live one alone.
        if !offline && self.live.lock().insert(d.id.clone(), unit.clone()).is_some() {
            self.stale_windows.lock().push(d.id.clone());
        }
        Some(unit)
    }

    /// Current state of every live plugin, base64 (to store in the project).
    pub fn states(&self) -> HashMap<String, String> {
        self.live.lock().iter().filter_map(|(id, u)| u.get_state().map(|b| (id.clone(), B64.encode(b)))).collect()
    }

    /// Drops instances whose device left the project and closes stale windows.
    /// Main thread (closes windows).
    pub fn prune(&self, project: &Project) {
        let mut ids: HashSet<&str> = HashSet::new();
        for t in &project.tracks {
            if let Some(i) = &t.instrument {
                ids.insert(&i.id);
            }
            for e in &t.effects {
                ids.insert(&e.id);
            }
        }
        for e in &project.master.effects {
            ids.insert(&e.id);
        }
        let removed: Vec<String> = self.live.lock().keys().filter(|k| !ids.contains(k.as_str())).cloned().collect();
        let mut stale = std::mem::take(&mut *self.stale_windows.lock());
        stale.extend(removed.iter().cloned());
        let closing: Vec<Editor> = {
            let mut windows = self.windows.lock();
            stale.into_iter().filter_map(|id| windows.remove(&id)).collect()
        };
        for w in closing {
            w.close();
        }
        let mut live = self.live.lock();
        for id in removed {
            live.remove(&id);
        }
    }

    /// True while at least one editor window exists (cheap, any thread).
    pub fn has_editors(&self) -> bool {
        !self.windows.lock().is_empty()
    }

    /// Sizes of a plugin editor window (diagnostics).
    #[cfg(target_os = "macos")]
    pub fn editor_sizes(&self, device_id: &str) -> Option<((f64, f64), (f64, f64))> {
        match self.windows.lock().get(device_id) {
            Some(Editor::Au(w)) => Some(w.sizes()),
            Some(Editor::Vst3(w)) => Some(w.sizes()),
            _ => None,
        }
    }

    /// Closes an editor window (main thread).
    pub fn close_editor(&self, device_id: &str) {
        let w = self.windows.lock().remove(device_id);
        if let Some(w) = w {
            w.close();
        }
    }

    /// Makes every editor window follow its plugin view's size. Main thread.
    pub fn fit_editors(&self) {
        for w in self.windows.lock().values_mut() {
            w.fit();
        }
    }

    /// On-screen plugin editor windows (native handles), for Ctrl+Tab.
    pub fn visible_editor_windows(&self) -> Vec<usize> {
        self.windows.lock().values().map(|w| w.handle()).filter(|(_, shown)| *shown).map(|(p, _)| p).collect()
    }

    /// Opens (or brings to front) the editor window of a device. Main thread.
    pub fn open_editor(&self, device_id: &str, title: &str) -> Result<(), String> {
        if let Some(w) = self.windows.lock().get_mut(device_id) {
            w.show();
            return Ok(());
        }
        let live = self.live.lock().get(device_id).cloned().ok_or("plugin is not loaded (no audio device?)")?;
        let w = live.open_editor(title)?;
        self.windows.lock().insert(device_id.to_string(), w);
        Ok(())
    }
}

/// Bridges the manager to the engine's external-device hook.
pub struct Provider;

impl ExternalDevices for Provider {
    fn handles(&self, type_id: &str) -> bool {
        type_id.starts_with("au:") || type_id.starts_with("vst3:")
    }

    fn create_instrument(&self, device: &DeviceState, sample_rate: f32, offline: bool) -> Option<Box<dyn Instrument>> {
        manager().instantiate(device, sample_rate, offline).map(|l| l.instrument())
    }

    fn create_effect(&self, device: &DeviceState, sample_rate: f32, offline: bool) -> Option<Box<dyn Effect>> {
        manager().instantiate(device, sample_rate, offline).map(|l| l.effect())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use daw_core::dsp::ProcessCtx;

    /// Instantiates every installed "Vital" AU offline and checks it plays.
    /// Skipped (passes) when Vital is not installed.
    #[test]
    fn vital_plays_a_note_offline() {
        let plugins = manager().list(true);
        assert!(!plugins.is_empty(), "macOS always ships Apple Audio Units");
        let third_party: Vec<_> = plugins.iter().filter(|p| p.vendor != "Apple").map(|p| format!("{} ({})", p.name, p.kind)).collect();
        eprintln!("{} plugins, third-party: {}", plugins.len(), third_party.join(", "));
        let Some(vital) = plugins.iter().find(|p| p.name.eq_ignore_ascii_case("vital") && p.kind == "instrument") else {
            eprintln!("Vital not installed, skipping");
            return;
        };
        let device = DeviceState { id: "test-vital".into(), type_id: vital.id.clone(), enabled: true, ..Default::default() };
        let mut inst = Provider.create_instrument(&device, 48_000.0, false).expect("Vital instantiates");
        let ctx = ProcessCtx { sample_rate: 48_000.0, bpm: 120.0, playing: true, beat: 0.0 };
        let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
        inst.note_on(60, 0.9);
        let mut peak = 0f32;
        for _ in 0..200 {
            l.fill(0.0);
            r.fill(0.0);
            inst.process(&ctx, &mut l, &mut r);
            peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
        }
        inst.note_off(60);
        eprintln!("{} by {} ({}) — peak {peak:.3}", vital.name, vital.vendor, vital.id);
        assert!(peak > 0.001, "Vital produced sound (peak {peak})");
        // An offline (export) instance must not replace the live one.
        let live_before = manager().live.lock().get("test-vital").unwrap().ptr();
        let _offline = Provider.create_instrument(&device, 48_000.0, true).expect("offline instance");
        assert_eq!(live_before, manager().live.lock().get("test-vital").unwrap().ptr());
        // State round trip.
        let states = manager().states();
        let s = states.get("test-vital").expect("state saved");
        assert!(B64.decode(s).unwrap().len() > 100);
    }

    /// Full path: a project with a Vital track, rendered by the offline exporter.
    #[test]
    fn vital_track_renders_through_the_engine() {
        daw_core::dsp::registry::set_external(Box::new(Provider));
        let Some(vital) = manager().list(false).into_iter().find(|p| p.name.eq_ignore_ascii_case("vital") && p.kind == "instrument") else {
            return;
        };
        let json = format!(
            r#"{{"bpm":120,"timeSigNum":4,"timeSigDen":4,"tracks":[{{"id":"t","kind":"instrument",
            "instrument":{{"id":"engine-vital","type":"{}","enabled":true,"params":{{}}}},
            "clips":[{{"id":"c","kind":"midi","start":0,"length":4,"notes":[{{"pitch":57,"start":0,"length":2,"velocity":110}}]}}]}}]}}"#,
            vital.id
        );
        let project: Project = serde_json::from_str(&json).unwrap();
        let opts = daw_core::project_io::ExportOptions { sample_rate: 48_000, bits: 32, start: 0.0, end: 4.0, tail: 0.5 };
        let out = daw_core::project_io::render_offline(&project, &Default::default(), opts, |_| {});
        let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.01, "Vital audible in an exported mixdown (peak {peak})");
        // The offline render did not register a live instance.
        assert!(!manager().live.lock().contains_key("engine-vital"));
    }

    #[test]
    fn apple_effect_processes_audio() {
        let plugins = manager().list(false);
        let Some(delay) = plugins.iter().find(|p| p.kind == "effect" && p.vendor == "Apple" && p.name.contains("Delay")) else {
            eprintln!("no Apple delay effect found, skipping");
            return;
        };
        let device = DeviceState { id: "test-delay".into(), type_id: delay.id.clone(), enabled: true, ..Default::default() };
        let mut fx = Provider.create_effect(&device, 48_000.0, false).expect("delay instantiates");
        let ctx = ProcessCtx { sample_rate: 48_000.0, bpm: 120.0, playing: true, beat: 0.0 };
        let mut energy_after = 0f32;
        for block in 0..200 {
            let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
            if block == 0 {
                l[0] = 1.0;
                r[0] = 1.0;
            }
            fx.process(&ctx, &mut l, &mut r);
            if block > 2 {
                energy_after += l.iter().map(|v| v * v).sum::<f32>();
            }
        }
        assert!(energy_after > 1e-6, "the delay produced echoes");
    }
}
