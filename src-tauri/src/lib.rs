//! daw-daw — a small DAW. Tauri entry point and IPC commands.

pub mod audio_io;
pub mod midi_in;
pub mod plugins;

pub use daw_core::{assets, dsp, engine, model, project_io};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use assets::AssetInfo;
use audio_io::{AudioHost, Capture, EngineSlot, OutputInfo, RecordTarget};
use engine::commands::Command;
use engine::controller::Controller;
use midi_in::{MidiInputs, MidiMsg};
use model::{AssetRef, Project};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub output_device: Option<String>,
    pub input_device: Option<String>,
    pub buffer_size: Option<u32>,
    /// Shift applied to recorded audio to compensate round-trip latency (ms).
    pub record_offset_ms: f64,
    /// Folder holding the sampler's sample set (default: `<base>/Samples`).
    pub samples_dir: Option<String>,
}

/// Where the sampler's sample set was loaded from, and what is missing.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplesStatus {
    pub dir: String,
    pub total: usize,
    pub missing: Vec<String>,
}

pub struct AppState {
    ctrl: Arc<Mutex<Controller>>,
    slot: EngineSlot,
    host: AudioHost,
    _capture: Arc<Mutex<Capture>>,
    rec_target: Arc<Mutex<Option<RecordTarget>>>,
    settings: Mutex<AudioSettings>,
    output: Mutex<Option<OutputInfo>>,
    input: Mutex<Option<(u32, u16)>>,
    midi: Mutex<Option<MidiInputs>>,
    audio_error: Mutex<Option<String>>,
    samples: Mutex<SamplesStatus>,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn base_dir() -> PathBuf {
    dirs::audio_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("daw-daw")
}

fn samples_dir(settings: &AudioSettings) -> PathBuf {
    match settings.samples_dir.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => base_dir().join("Samples"),
    }
}

/// (Re)loads the sample set from the configured folder; missing files stay silent.
fn load_samples(settings: &AudioSettings) -> SamplesStatus {
    let dir = samples_dir(settings);
    let missing = dsp::sampler::load_samples(|f| std::fs::read(dir.join(f)).ok());
    if !missing.is_empty() {
        eprintln!("[samples] {} of {} files missing in {}", missing.len(), dsp::sampler::sample_files().count(), dir.display());
    }
    SamplesStatus { dir: dir.to_string_lossy().into_owned(), total: dsp::sampler::sample_files().count(), missing }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("audio-settings.json"))
}

fn load_settings(app: &AppHandle) -> AudioSettings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_settings(app: &AppHandle, s: &AudioSettings) {
    if let Some(p) = settings_path(app) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::to_string_pretty(s).unwrap_or_default());
    }
}

/// (Re)opens the output device, rebuilding the engine if the rate changed.
fn open_output(state: &AppState) -> CmdResult<OutputInfo> {
    let settings = state.settings.lock().clone();
    if let Some(rate) = audio_io::probe_output_rate(settings.output_device.as_deref()) {
        let mut ctrl = state.ctrl.lock();
        if (ctrl.sample_rate - rate as f32).abs() > 0.5 {
            let engine = ctrl.rebuild(rate as f32);
            *state.slot.lock() = Some(engine);
        }
    }
    match state.host.open_output(settings.output_device.clone(), settings.buffer_size) {
        Ok(info) => {
            *state.output.lock() = Some(info.clone());
            *state.audio_error.lock() = None;
            Ok(info)
        }
        Err(e) => {
            *state.output.lock() = None;
            *state.audio_error.lock() = Some(e.clone());
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_catalog() -> Vec<&'static dsp::DeviceSpec> {
    dsp::registry::all_specs()
}

/// Runs on the main thread (sync command): plugin instances may be created
/// here and their editor windows closed when devices leave the project.
#[tauri::command]
fn sync_project(state: State<'_, AppState>, project: Project) {
    state.ctrl.lock().sync(&project);
    plugins::manager().prune(&project);
}

/// Installed third-party plugins (Audio Units on macOS).
#[tauri::command]
async fn list_plugins(rescan: Option<bool>) -> Vec<plugins::PluginInfo> {
    tauri::async_runtime::spawn_blocking(move || plugins::manager().list(rescan.unwrap_or(false)))
        .await
        .unwrap_or_default()
}

/// Opens the plugin's own editor window (main thread: sync command).
#[tauri::command]
fn plugin_open_editor(device_id: String, title: String) -> CmdResult<()> {
    plugins::manager().open_editor(&device_id, &title)
}

/// Current state of every loaded plugin (device id → base64), saved in the project.
#[tauri::command]
fn plugin_states() -> HashMap<String, String> {
    plugins::manager().states()
}

#[tauri::command]
fn transport(state: State<'_, AppState>, action: String, position: Option<f64>) -> CmdResult<()> {
    let mut c = state.ctrl.lock();
    match action.as_str() {
        "play" => c.send(Command::Play),
        "stop" => {
            c.send(Command::Stop);
            // A take that never started (stopped during count-in) must not linger.
            if !c.shared.recording.load(std::sync::atomic::Ordering::Relaxed) {
                *state.rec_target.lock() = None;
            }
        }
        "seek" => c.send(Command::Seek(position.unwrap_or(0.0))),
        "panic" => c.send(Command::AllNotesOff),
        _ => return Err(format!("unknown transport action {action}")),
    }
    Ok(())
}

/// Starts recording. Audio is captured for `audio_track` (if any), MIDI is
/// captured for armed instrument tracks via the live target.
#[tauri::command]
fn record(state: State<'_, AppState>, audio_track: Option<String>, take_name: Option<String>) -> CmdResult<()> {
    *state.rec_target.lock() = None;
    if let Some(track_id) = audio_track {
        let input = *state.input.lock();
        let input = match input {
            Some(i) => i,
            None => {
                let dev = state.settings.lock().input_device.clone();
                let i = state.host.open_input(dev)?;
                *state.input.lock() = Some(i);
                i
            }
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let name = take_name.unwrap_or_else(|| "Take".into()).replace(['/', '\\', ':'], "-");
        let path = base_dir().join("Recordings").join(format!("{name} {stamp}.wav"));
        *state.rec_target.lock() = Some(RecordTarget { track_id, path, sample_rate: input.0 });
    }
    state.ctrl.lock().send(Command::Record);
    Ok(())
}

#[tauri::command]
fn set_input_enabled(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    if enabled {
        if state.input.lock().is_none() {
            let dev = state.settings.lock().input_device.clone();
            let i = state.host.open_input(dev)?;
            *state.input.lock() = Some(i);
        }
    } else if state.input.lock().take().is_some() {
        state.host.close_input();
    }
    Ok(())
}

/// Plays a note live. `track_id` = a specific track (editor previews, never
/// recorded); None = the live target (keyboard / musical typing, recordable).
#[tauri::command]
fn live_note(state: State<'_, AppState>, pitch: u8, velocity: f32, on: bool, track_id: Option<String>) {
    let mut c = state.ctrl.lock();
    let handle = track_id.as_deref().and_then(|id| c.handle_of(id));
    if track_id.is_some() && handle.is_none() {
        return;
    }
    let record = track_id.is_none();
    c.send(Command::LiveNote { handle, pitch: pitch.min(127), velocity: velocity.clamp(0.0, 1.0), on, record });
}

#[tauri::command]
fn set_live_target(state: State<'_, AppState>, track_id: Option<String>) {
    state.ctrl.lock().set_live_target(track_id);
}

#[tauri::command]
async fn import_audio(state: State<'_, AppState>, id: String, path: String) -> CmdResult<AssetInfo> {
    let p = PathBuf::from(&path);
    let buf = tauri::async_runtime::spawn_blocking(move || assets::decode_file(&p))
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(state.ctrl.lock().assets.insert(&id, &path, buf))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetLoadResult {
    id: String,
    info: Option<AssetInfo>,
    error: Option<String>,
}

#[tauri::command]
async fn load_assets(app: AppHandle, state: State<'_, AppState>, assets: Vec<AssetRef>) -> CmdResult<Vec<AssetLoadResult>> {
    let mut out = Vec::new();
    for a in assets {
        let cached = {
            let c = state.ctrl.lock();
            c.assets.get(&a.id).filter(|_| c.assets.paths.get(&a.id) == Some(&a.path))
        };
        let res = match cached {
            Some(b) => Ok(assets::AssetCache::info(&a.id, &a.path, &b)),
            None => {
                let p = PathBuf::from(&a.path);
                match tauri::async_runtime::spawn_blocking(move || assets::decode_file(&p)).await {
                    Ok(Ok(buf)) => Ok(state.ctrl.lock().assets.insert(&a.id, &a.path, buf)),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
        };
        match res {
            Ok(info) => out.push(AssetLoadResult { id: a.id, info: Some(info), error: None }),
            Err(e) => out.push(AssetLoadResult { id: a.id, info: None, error: Some(e) }),
        }
    }
    // Re-sync so clips referencing freshly loaded assets become audible. On the
    // main thread: a sync may create plugins, which must happen there.
    let ctrl = state.ctrl.clone();
    let _ = app.run_on_main_thread(move || {
        let mut c = ctrl.lock();
        let p = c.project().clone();
        c.sync(&p);
    });
    Ok(out)
}

#[tauri::command]
async fn preview_file(state: State<'_, AppState>, path: Option<String>) -> CmdResult<()> {
    let buf = match path {
        Some(p) => {
            let pb = PathBuf::from(p);
            Some(Arc::new(
                tauri::async_runtime::spawn_blocking(move || assets::decode_file(&pb))
                    .await
                    .map_err(err)?
                    .map_err(err)?,
            ))
        }
        None => None,
    };
    state.ctrl.lock().send(Command::Preview(buf));
    Ok(())
}

#[tauri::command]
fn save_project(path: String, project: Value) -> CmdResult<Value> {
    project_io::save(&PathBuf::from(path), project).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadedProject {
    path: String,
    project: Value,
}

#[tauri::command]
fn load_project(path: String) -> CmdResult<LoadedProject> {
    let (dir, project) = project_io::load(&PathBuf::from(path)).map_err(err)?;
    Ok(LoadedProject { path: dir.to_string_lossy().into_owned(), project })
}

#[tauri::command]
async fn export_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    project: Project,
    options: project_io::ExportOptions,
) -> CmdResult<()> {
    let assets = {
        let c = state.ctrl.lock();
        assets::AssetCache { buffers: c.assets.buffers.clone(), paths: c.assets.paths.clone() }
    };
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        project_io::export_wav(&PathBuf::from(path), &project, &assets, options, |p| {
            let _ = app2.emit("export-progress", p);
        })
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioStatus {
    output: Option<OutputInfo>,
    input: Option<(u32, u16)>,
    devices: audio_io::DeviceList,
    settings: AudioSettings,
    midi_ports: Vec<String>,
    error: Option<String>,
    base_dir: String,
    samples: SamplesStatus,
}

#[tauri::command]
fn get_audio_status(state: State<'_, AppState>) -> AudioStatus {
    AudioStatus {
        output: state.output.lock().clone(),
        input: *state.input.lock(),
        devices: audio_io::list_devices(),
        settings: state.settings.lock().clone(),
        midi_ports: state.midi.lock().as_ref().map(|m| m.port_names.clone()).unwrap_or_default(),
        error: state.audio_error.lock().clone(),
        base_dir: base_dir().to_string_lossy().into_owned(),
        samples: state.samples.lock().clone(),
    }
}

#[tauri::command]
fn set_audio_settings(app: AppHandle, state: State<'_, AppState>, settings: AudioSettings) -> CmdResult<OutputInfo> {
    let (input_changed, samples_changed) = {
        let old = state.settings.lock();
        (old.input_device != settings.input_device, samples_dir(&old) != samples_dir(&settings))
    };
    *state.settings.lock() = settings.clone();
    if samples_changed {
        *state.samples.lock() = load_samples(&settings);
    }
    save_settings(&app, &settings);
    if input_changed && state.input.lock().take().is_some() {
        state.host.close_input();
        let i = state.host.open_input(settings.input_device.clone())?;
        *state.input.lock() = Some(i);
    }
    open_output(&state)
}

#[tauri::command]
fn rescan_midi(app: AppHandle, state: State<'_, AppState>) -> Vec<String> {
    if let Some(m) = state.midi.lock().take() {
        m.close();
    }
    let m = connect_midi(&app, state.ctrl.clone());
    let names = m.port_names.clone();
    *state.midi.lock() = Some(m);
    names
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
}

const AUDIO_EXT: &[&str] = &["wav", "mp3", "flac", "ogg", "aif", "aiff", "m4a", "caf"];

#[tauri::command]
fn list_dir(path: String) -> CmdResult<Vec<DirEntry>> {
    let mut v: Vec<DirEntry> = std::fs::read_dir(&path)
        .map_err(err)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let p = e.path();
            let is_dir = p.is_dir();
            let ext_ok = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| AUDIO_EXT.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false);
            (is_dir || ext_ok).then(|| DirEntry { name, path: p.to_string_lossy().into_owned(), is_dir })
        })
        .collect();
    v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(v)
}

/// The samples folder, for the browser (created if needed).
#[tauri::command]
fn library_dir(state: State<'_, AppState>) -> CmdResult<String> {
    let dir = PathBuf::from(&state.samples.lock().dir);
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
fn default_dirs() -> HashMap<&'static str, String> {
    let base = base_dir();
    let mut m = HashMap::new();
    m.insert("base", base.to_string_lossy().into_owned());
    m.insert("projects", base.join("Projects").to_string_lossy().into_owned());
    m.insert("recordings", base.join("Recordings").to_string_lossy().into_owned());
    m.insert("exports", base.join("Exports").to_string_lossy().into_owned());
    m.insert("home", dirs::home_dir().unwrap_or_default().to_string_lossy().into_owned());
    m.insert("music", dirs::audio_dir().unwrap_or_default().to_string_lossy().into_owned());
    m
}

// ---------------------------------------------------------------- background

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EngineStatus {
    position: f64,
    playing: bool,
    recording: bool,
    count_in: bool,
    cpu: f32,
    master: (f32, f32),
    input: (f32, f32),
    meters: HashMap<String, (f32, f32)>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecordedNote {
    track_id: String,
    pitch: u8,
    velocity: f32,
    on: bool,
    beat: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AudioRecorded {
    track_id: String,
    start_beat: f64,
    asset: AssetInfo,
}

fn spawn_status_thread(app: AppHandle, ctrl: Arc<Mutex<Controller>>) {
    std::thread::Builder::new()
        .name("status".into())
        .spawn(move || {
            use std::sync::atomic::Ordering;
            loop {
                std::thread::sleep(Duration::from_millis(30));
                let (status, notes) = {
                    let mut c = ctrl.lock();
                    c.collect_garbage();
                    let s = c.shared.clone();
                    let meters = c.track_meters().into_iter().map(|(id, l, r)| (id, (l, r))).collect();
                    let notes: Vec<RecordedNote> = c
                        .drain_recorded()
                        .into_iter()
                        .map(|(track_id, e)| RecordedNote { track_id, pitch: e.pitch, velocity: e.velocity, on: e.on, beat: e.beat })
                        .collect();
                    (
                        EngineStatus {
                            position: s.position.load(),
                            playing: s.playing.load(Ordering::Relaxed),
                            recording: s.recording.load(Ordering::Relaxed),
                            count_in: s.count_in.load(Ordering::Relaxed),
                            cpu: s.cpu.load(),
                            master: s.master.take(),
                            input: s.input.take(),
                            meters,
                        },
                        notes,
                    )
                };
                if !notes.is_empty() {
                    let _ = app.emit("midi-recorded", notes);
                }
                let _ = app.emit("engine-status", status);
            }
        })
        .expect("spawn status thread");
}

fn connect_midi(app: &AppHandle, ctrl: Arc<Mutex<Controller>>) -> MidiInputs {
    let app = app.clone();
    MidiInputs::connect_all(Arc::new(move |m: MidiMsg| {
        match m {
            MidiMsg::NoteOn { channel, pitch, velocity } => {
                ctrl.lock().midi_note(channel, pitch, velocity as f32 / 127.0, true);
                let _ = app.emit("midi-input", (pitch, velocity, true, channel));
            }
            MidiMsg::NoteOff { channel, pitch } => {
                ctrl.lock().midi_note(channel, pitch, 0.0, false);
                let _ = app.emit("midi-input", (pitch, 0u8, false, channel));
            }
            // Controllers are mapped to parameters by the UI (MIDI learn).
            MidiMsg::Control { channel, cc, value } => {
                let _ = app.emit("midi-cc", (channel, cc, value));
            }
        }
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Third-party plugins become available to the engine; their creation
            // and destruction are routed to the main thread.
            daw_core::dsp::registry::set_external(Box::new(plugins::Provider));
            {
                let h = app.handle().clone();
                plugins::set_main_dispatcher(Box::new(move |f| {
                    let _ = h.run_on_main_thread(f);
                }));
            }
            let handle = app.handle().clone();
            let settings = load_settings(&handle);
            let samples = load_samples(&settings);
            let rate = audio_io::probe_output_rate(settings.output_device.as_deref()).unwrap_or(48_000);
            let (ctrl, engine) = Controller::new(rate as f32);
            let shared = ctrl.shared.clone();
            let ctrl = Arc::new(Mutex::new(ctrl));
            let slot: EngineSlot = Arc::new(Mutex::new(Some(engine)));
            let capture = Arc::new(Mutex::new(Capture { producer: None }));
            let rec_target = Arc::new(Mutex::new(None));
            let shared_for_host = shared.clone();
            let host = AudioHost::start(slot.clone(), Arc::new(move || shared_for_host.clone()), capture.clone());

            // Recorder: finished takes are decoded and handed to the UI.
            {
                let app2 = handle.clone();
                let ctrl2 = ctrl.clone();
                let shared2 = shared.clone();
                audio_io::spawn_recorder(Arc::new(move || shared2.clone()), capture.clone(), rec_target.clone(), move |take| {
                    let id = format!("rec-{}", take.path.file_stem().and_then(|s| s.to_str()).unwrap_or("take").replace(' ', "-"));
                    let path = take.path.to_string_lossy().into_owned();
                    match assets::decode_file(&take.path) {
                        Ok(buf) => {
                            let info = ctrl2.lock().assets.insert(&id, &path, buf);
                            let offset_ms = app2.state::<AppState>().settings.lock().record_offset_ms;
                            let bpm = ctrl2.lock().project().bpm.max(1.0);
                            let start_beat = take.start_beat - offset_ms / 1000.0 * bpm / 60.0;
                            let _ = app2.emit("audio-recorded", AudioRecorded { track_id: take.track_id, start_beat, asset: info });
                        }
                        Err(e) => eprintln!("[rec] cannot decode take: {e}"),
                    }
                });
            }

            // Plugin editors size themselves after being shown: keep their windows in step.
            {
                let app2 = handle.clone();
                std::thread::Builder::new()
                    .name("plugin-editors".into())
                    .spawn(move || loop {
                        std::thread::sleep(Duration::from_millis(250));
                        if plugins::manager().has_editors() {
                            let _ = app2.run_on_main_thread(|| plugins::manager().fit_editors());
                        }
                    })
                    .expect("spawn plugin editor watcher");
            }

            let midi = connect_midi(&handle, ctrl.clone());
            spawn_status_thread(handle.clone(), ctrl.clone());

            app.manage(AppState {
                ctrl,
                slot,
                host,
                _capture: capture,
                rec_target,
                settings: Mutex::new(settings),
                output: Mutex::new(None),
                input: Mutex::new(None),
                midi: Mutex::new(Some(midi)),
                audio_error: Mutex::new(None),
                samples: Mutex::new(samples),
            });
            // Ctrl+Tab: main window <-> plugin editor windows.
            #[cfg(target_os = "macos")]
            if let Some(win) = handle.get_webview_window("main") {
                if let Ok(ns) = win.ns_window() {
                    plugins::ui::install_window_cycling(ns, || plugins::manager().visible_editor_windows());
                }
            }
            let state = handle.state::<AppState>();
            if let Err(e) = open_output(&state) {
                eprintln!("[audio] cannot open output: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_catalog,
            sync_project,
            transport,
            record,
            set_input_enabled,
            live_note,
            set_live_target,
            import_audio,
            load_assets,
            preview_file,
            save_project,
            load_project,
            export_audio,
            get_audio_status,
            set_audio_settings,
            rescan_midi,
            list_dir,
            default_dirs,
            library_dir,
            list_plugins,
            plugin_open_editor,
            plugin_states,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
