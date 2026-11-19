//! Third-party plugin hosting (desktop only). macOS: Audio Units (v2), which
//! covers Vital, Surge, u-he, Serum… The engine sees plugins through
//! `daw_core::dsp::registry::ExternalDevices`, so other formats (VST3, CLAP)
//! can be added later behind the same interface.

#[cfg(target_os = "macos")]
pub mod au;
#[cfg(target_os = "macos")]
pub mod ffi;
#[cfg(target_os = "macos")]
pub mod ui;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use parking_lot::Mutex;
use serde::Serialize;

use daw_core::dsp::registry::ExternalDevices;
use daw_core::dsp::{Effect, Instrument};
use daw_core::model::{DeviceState, Project};

#[cfg(target_os = "macos")]
pub use au::PluginInfo;

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub kind: &'static str,
    pub format: &'static str,
}

#[cfg(target_os = "macos")]
type Live = Arc<au::AuUnit>;
#[cfg(not(target_os = "macos"))]
type Live = ();

#[derive(Default)]
pub struct PluginManager {
    catalog: Mutex<Option<Vec<PluginInfo>>>,
    /// Device id → live instance (shared with the engine).
    live: Mutex<HashMap<String, Live>>,
    #[cfg(target_os = "macos")]
    windows: Mutex<HashMap<String, ui::EditorWindow>>,
    /// Windows of instances that were replaced/removed, closed on the main thread.
    #[cfg(target_os = "macos")]
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

/// Installs the function that runs a closure on the main thread (Tauri).
pub fn set_main_dispatcher(f: MainDispatch) {
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
    true
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
        let mut c = self.catalog.lock();
        if rescan || c.is_none() {
            #[cfg(target_os = "macos")]
            {
                *c = Some(au::scan());
            }
            #[cfg(not(target_os = "macos"))]
            {
                *c = Some(Vec::new());
            }
        }
        c.clone().unwrap_or_default()
    }

    fn info(&self, type_id: &str) -> Option<PluginInfo> {
        self.list(false).into_iter().find(|p| p.id == type_id)
    }

    #[cfg(target_os = "macos")]
    fn instantiate(&self, d: &DeviceState, sample_rate: f32, offline: bool) -> Option<au::AuDevice> {
        let info = self.info(&d.type_id)?;
        let name = info.name.clone();
        let unit = match on_main(move || au::AuUnit::new(info, sample_rate)) {
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
        Some(au::AuDevice::new(unit))
    }

    /// Current state of every live plugin, base64 (to store in the project).
    pub fn states(&self) -> HashMap<String, String> {
        #[cfg(target_os = "macos")]
        {
            self.live
                .lock()
                .iter()
                .filter_map(|(id, u)| u.get_state().map(|b| (id.clone(), B64.encode(b))))
                .collect()
        }
        #[cfg(not(target_os = "macos"))]
        HashMap::new()
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
        #[cfg(target_os = "macos")]
        {
            let mut stale = std::mem::take(&mut *self.stale_windows.lock());
            stale.extend(removed.iter().cloned());
            let mut windows = self.windows.lock();
            for id in stale {
                if let Some(w) = windows.remove(&id) {
                    w.close();
                }
            }
        }
        let mut live = self.live.lock();
        for id in removed {
            live.remove(&id);
        }
    }

    /// True while at least one editor window exists (cheap, any thread).
    pub fn has_editors(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            !self.windows.lock().is_empty()
        }
        #[cfg(not(target_os = "macos"))]
        false
    }

    /// Sizes of an editor window (diagnostics).
    #[cfg(target_os = "macos")]
    pub fn editor_sizes(&self, device_id: &str) -> Option<((f64, f64), (f64, f64))> {
        self.windows.lock().get(device_id).map(|w| w.sizes())
    }

    /// Closes an editor window (main thread).
    #[cfg(target_os = "macos")]
    pub fn close_editor(&self, device_id: &str) {
        if let Some(w) = self.windows.lock().remove(device_id) {
            w.close();
        }
    }

    /// Makes every editor window follow its plugin view's size. Main thread.
    pub fn fit_editors(&self) {
        #[cfg(target_os = "macos")]
        {
            for w in self.windows.lock().values_mut() {
                w.fit();
            }
        }
    }

    /// On-screen plugin editor windows (NSWindow pointers), for Ctrl+Tab.
    pub fn visible_editor_windows(&self) -> Vec<usize> {
        #[cfg(target_os = "macos")]
        {
            self.windows.lock().values().map(|w| w.ns_window()).filter(|(_, shown)| *shown).map(|(p, _)| p).collect()
        }
        #[cfg(not(target_os = "macos"))]
        Vec::new()
    }

    /// Opens (or brings to front) the editor window of a device. Main thread.
    pub fn open_editor(&self, device_id: &str, title: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(w) = self.windows.lock().get_mut(device_id) {
                w.show();
                return Ok(());
            }
            let unit = self.live.lock().get(device_id).cloned().ok_or("plugin is not loaded (no audio device?)")?;
            let w = unsafe { ui::EditorWindow::open(unit.unit, title) }.map_err(|e| e.to_string())?;
            self.windows.lock().insert(device_id.to_string(), w);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (device_id, title);
            Err("plugins are only supported on macOS for now".into())
        }
    }
}

/// Bridges the manager to the engine's external-device hook.
pub struct Provider;

impl ExternalDevices for Provider {
    fn handles(&self, type_id: &str) -> bool {
        type_id.starts_with("au:")
    }

    fn create_instrument(&self, device: &DeviceState, sample_rate: f32, offline: bool) -> Option<Box<dyn Instrument>> {
        #[cfg(target_os = "macos")]
        {
            manager().instantiate(device, sample_rate, offline).map(|d| Box::new(d) as Box<dyn Instrument>)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (device, sample_rate, offline);
            None
        }
    }

    fn create_effect(&self, device: &DeviceState, sample_rate: f32, offline: bool) -> Option<Box<dyn Effect>> {
        #[cfg(target_os = "macos")]
        {
            manager().instantiate(device, sample_rate, offline).map(|d| Box::new(d) as Box<dyn Effect>)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (device, sample_rate, offline);
            None
        }
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
        let live_before = Arc::as_ptr(manager().live.lock().get("test-vital").unwrap());
        let _offline = Provider.create_instrument(&device, 48_000.0, true).expect("offline instance");
        assert_eq!(live_before, Arc::as_ptr(manager().live.lock().get("test-vital").unwrap()));
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
