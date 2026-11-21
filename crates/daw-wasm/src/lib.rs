//! daw-daw engine for the browser. Exposes a tiny C ABI (no wasm-bindgen) so
//! that it can be instantiated synchronously inside an AudioWorklet, where
//! the usual JS glue (fetch, TextDecoder…) is not available.
//!
//! Strings and JSON cross the boundary as UTF-8 bytes: the host writes them
//! into memory obtained with `dd_alloc`, results are read back from the
//! pointer returned by a call plus `dd_result_len()`.

use std::cell::RefCell;
use std::sync::atomic::Ordering;

use daw_core::assets::{AssetCache, AudioBuffer};
use daw_core::engine::Engine;
use daw_core::engine::commands::Command;
use daw_core::engine::controller::Controller;
use daw_core::model::Project;
use daw_core::project_io::{ExportOptions, render_offline};
use serde::Serialize;

struct State {
    ctrl: Controller,
    engine: Engine,
    out: Vec<f32>,
    status: [f64; 8],
    result: Vec<u8>,
    render: Vec<f32>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static RESULT_LEN: RefCell<usize> = const { RefCell::new(0) };
    /// Sample files handed over by the host, applied by `dd_load_samples`.
    static PENDING_SAMPLES: RefCell<std::collections::HashMap<String, Vec<u8>>> = RefCell::new(Default::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R, default: R) -> R {
    STATE.with(|s| match s.borrow_mut().as_mut() {
        Some(st) => f(st),
        None => default,
    })
}

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(ptr, len) } }
}

unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    std::str::from_utf8(unsafe { bytes(ptr, len) }).unwrap_or("")
}

fn set_result(st: &mut State, data: Vec<u8>) -> *const u8 {
    st.result = data;
    RESULT_LEN.with(|l| *l.borrow_mut() = st.result.len());
    st.result.as_ptr()
}

// ------------------------------------------------------------------ memory

/// Allocates `len` bytes (aligned for f32/f64) and returns the pointer.
#[unsafe(no_mangle)]
pub extern "C" fn dd_alloc(len: usize) -> *mut u8 {
    let words = len.div_ceil(8).max(1);
    let mut v: Vec<u64> = Vec::with_capacity(words);
    let p = v.as_mut_ptr() as *mut u8;
    std::mem::forget(v);
    p
}

/// Frees memory from `dd_alloc` (same `len`).
///
/// # Safety
/// `ptr` must come from `dd_alloc(len)` and not be freed twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let words = len.div_ceil(8).max(1);
    drop(unsafe { Vec::from_raw_parts(ptr as *mut u64, 0, words) });
}

/// Length of the last byte result (JSON) returned by a call.
#[unsafe(no_mangle)]
pub extern "C" fn dd_result_len() -> usize {
    RESULT_LEN.with(|l| *l.borrow())
}

// ------------------------------------------------------------------ lifecycle

/// Creates (or re-creates) the engine at the given sample rate.
#[unsafe(no_mangle)]
pub extern "C" fn dd_init(sample_rate: f32) {
    let (ctrl, engine) = Controller::new(sample_rate.max(8000.0));
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            ctrl,
            engine,
            out: vec![0.0; 4096 * 2],
            status: [0.0; 8],
            result: Vec::new(),
            render: Vec::new(),
        })
    });
}

/// Applies a project document (JSON). Returns 0 on success, -1 on parse error.
///
/// # Safety
/// `ptr..ptr+len` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_sync(ptr: *const u8, len: usize) -> i32 {
    let Ok(p) = serde_json::from_slice::<Project>(unsafe { bytes(ptr, len) }) else { return -1 };
    with(
        |st| {
            st.ctrl.sync(&p);
            0
        },
        -1,
    )
}

/// 0 play · 1 stop · 2 seek(position) · 3 record · 4 panic (all notes off)
#[unsafe(no_mangle)]
pub extern "C" fn dd_transport(cmd: u32, position: f64) {
    with(
        |st| {
            let c = match cmd {
                0 => Command::Play,
                1 => Command::Stop,
                2 => Command::Seek(position),
                3 => Command::Record,
                _ => Command::AllNotesOff,
            };
            st.ctrl.send(c);
        },
        (),
    )
}

/// Live note. Empty track id = live target (recordable); else that track only (preview).
///
/// # Safety
/// `track_ptr..+track_len` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_live_note(pitch: u32, velocity: f32, on: u32, track_ptr: *const u8, track_len: usize) {
    let track = unsafe { text(track_ptr, track_len) };
    with(
        |st| {
            let handle = if track.is_empty() { None } else { st.ctrl.handle_of(track) };
            if !track.is_empty() && handle.is_none() {
                return;
            }
            st.ctrl.send(Command::LiveNote {
                handle,
                pitch: pitch.min(127) as u8,
                velocity: velocity.clamp(0.0, 1.0),
                on: on != 0,
                record: track.is_empty(),
            });
        },
        (),
    )
}

/// Sets the track that receives MIDI-keyboard / typing notes (empty = none).
///
/// # Safety
/// `ptr..+len` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_set_live_target(ptr: *const u8, len: usize) {
    let id = unsafe { text(ptr, len) }.to_string();
    with(|st| st.ctrl.set_live_target(if id.is_empty() { None } else { Some(id) }), ())
}

/// Registers decoded audio for an asset id. `data` is planar f32 (`channels` × `frames`).
///
/// # Safety
/// `id_ptr..+id_len` and `data..+channels*frames` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_add_asset(id_ptr: *const u8, id_len: usize, sample_rate: f32, channels: u32, frames: u32, data: *const f32) {
    let id = unsafe { text(id_ptr, id_len) }.to_string();
    let ch = channels.clamp(1, 2) as usize;
    let frames = frames as usize;
    let all = if data.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(data, ch * frames) } };
    let buf = AudioBuffer { sample_rate, channels: (0..ch).map(|c| all[c * frames..(c + 1) * frames].to_vec()).collect() };
    with(
        |st| {
            st.ctrl.assets.insert(&id, &format!("browser:{id}"), buf);
            // Re-sync so clips using this asset become audible.
            let p = st.ctrl.project().clone();
            st.ctrl.sync(&p);
        },
        (),
    )
}

// ------------------------------------------------------------------ audio

/// Renders `frames` frames; returns a pointer to interleaved stereo f32.
#[unsafe(no_mangle)]
pub extern "C" fn dd_process(frames: u32) -> *const f32 {
    with(
        |st| {
            let n = (frames as usize).min(4096);
            let out = &mut st.out[..n * 2];
            st.engine.process(out, 2);
            st.ctrl.collect_garbage();
            st.out.as_ptr()
        },
        std::ptr::null(),
    )
}

/// The host measured `ms` of processing for `frames` frames (feeds the DSP meter).
#[unsafe(no_mangle)]
pub extern "C" fn dd_report_load(ms: f64, frames: u32) {
    with(|st| st.engine.report_load((ms / 1000.0) as f32, frames as usize), ())
}

/// Status snapshot: [position, playing, recording, count_in, cpu, master_l, master_r, 0].
#[unsafe(no_mangle)]
pub extern "C" fn dd_status() -> *const f64 {
    with(
        |st| {
            let s = &st.ctrl.shared;
            let (ml, mr) = s.master.take();
            st.status = [
                s.position.load(),
                s.playing.load(Ordering::Relaxed) as u8 as f64,
                s.recording.load(Ordering::Relaxed) as u8 as f64,
                s.count_in.load(Ordering::Relaxed) as u8 as f64,
                s.cpu.load() as f64,
                ml as f64,
                mr as f64,
                0.0,
            ];
            st.status.as_ptr()
        },
        std::ptr::null(),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Extra {
    meters: std::collections::HashMap<String, (f32, f32)>,
    recorded: Vec<RecordedNote>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordedNote {
    track_id: String,
    pitch: u8,
    velocity: f32,
    on: bool,
    beat: f64,
}

/// Track meters and recorded MIDI events since the last call (JSON).
#[unsafe(no_mangle)]
pub extern "C" fn dd_poll() -> *const u8 {
    with(
        |st| {
            let meters = st.ctrl.track_meters().into_iter().map(|(id, l, r)| (id, (l, r))).collect();
            let recorded = st
                .ctrl
                .drain_recorded()
                .into_iter()
                .map(|(track_id, e)| RecordedNote { track_id, pitch: e.pitch, velocity: e.velocity, on: e.on, beat: e.beat })
                .collect();
            let json = serde_json::to_vec(&Extra { meters, recorded }).unwrap_or_default();
            set_result(st, json)
        },
        std::ptr::null(),
    )
}

/// Device catalog (instruments + effects) as JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dd_catalog() -> *const u8 {
    let json = serde_json::to_vec(&daw_core::dsp::registry::all_specs()).unwrap_or_default();
    with(|st| set_result(st, json), std::ptr::null())
}

// ------------------------------------------------------------------ samples

/// JSON array of the sample files the sampler needs (paths relative to the samples folder).
#[unsafe(no_mangle)]
pub extern "C" fn dd_sample_files() -> *const u8 {
    let files: Vec<&str> = daw_core::dsp::sampler::sample_files().collect();
    let json = serde_json::to_vec(&files).unwrap_or_default();
    with(|st| set_result(st, json), std::ptr::null())
}

/// Hands over one sample file (raw WAV bytes); applied by `dd_load_samples`.
///
/// # Safety
/// Both ranges must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_add_sample(path_ptr: *const u8, path_len: usize, data_ptr: *const u8, data_len: usize) {
    let path = unsafe { text(path_ptr, path_len) }.to_string();
    let data = unsafe { bytes(data_ptr, data_len) }.to_vec();
    PENDING_SAMPLES.with(|m| m.borrow_mut().insert(path, data));
}

/// Loads the handed-over samples (missing ones stay silent). Returns a JSON array
/// of the missing files.
#[unsafe(no_mangle)]
pub extern "C" fn dd_load_samples() -> *const u8 {
    let files = PENDING_SAMPLES.with(|m| std::mem::take(&mut *m.borrow_mut()));
    let missing = daw_core::dsp::sampler::load_samples(|f| files.get(f).cloned());
    let json = serde_json::to_vec(&missing).unwrap_or_default();
    with(|st| set_result(st, json), std::ptr::null())
}

// ------------------------------------------------------------------ export

/// Renders the project offline (uses the assets registered in this instance).
/// Returns a pointer to interleaved stereo f32; frame count = `dd_result_len() / 2`.
///
/// # Safety
/// `ptr..+len` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dd_render(ptr: *const u8, len: usize, sample_rate: u32, start: f64, end: f64, tail: f64) -> *const f32 {
    let Ok(p) = serde_json::from_slice::<Project>(unsafe { bytes(ptr, len) }) else { return std::ptr::null() };
    with(
        |st| {
            let assets = AssetCache { buffers: st.ctrl.assets.buffers.clone(), paths: st.ctrl.assets.paths.clone() };
            let opts = ExportOptions { sample_rate, bits: 32, start, end, tail };
            st.render = render_offline(&p, &assets, opts, |_| {});
            RESULT_LEN.with(|l| *l.borrow_mut() = st.render.len());
            st.render.as_ptr()
        },
        std::ptr::null(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_json(json: &str, f: unsafe extern "C" fn(*const u8, usize) -> i32) -> i32 {
        unsafe { f(json.as_ptr(), json.len()) }
    }

    #[test]
    fn end_to_end_through_the_c_api() {
        dd_init(48_000.0);
        let project = r#"{"bpm":120,"timeSigNum":4,"timeSigDen":4,"tracks":[{"id":"t1","kind":"instrument",
            "instrument":{"id":"i","type":"pulse","enabled":true,"params":{}},
            "clips":[{"id":"c","kind":"midi","start":0,"length":4,"notes":[{"pitch":60,"start":0,"length":1,"velocity":100}]}]}]}"#;
        assert_eq!(call_json(project, dd_sync), 0);
        dd_transport(0, 0.0);
        let mut peak = 0f32;
        for _ in 0..100 {
            let p = dd_process(128);
            let out = unsafe { std::slice::from_raw_parts(p, 256) };
            peak = out.iter().fold(peak, |m, v| m.max(v.abs()));
        }
        assert!(peak > 0.01, "synth note audible: {peak}");
        let st = unsafe { std::slice::from_raw_parts(dd_status(), 8) };
        assert!(st[0] > 0.5 && st[1] == 1.0, "position advanced, playing");
        let p = dd_poll();
        let json = unsafe { std::slice::from_raw_parts(p, dd_result_len()) };
        let v: serde_json::Value = serde_json::from_slice(json).unwrap();
        assert!(v["meters"]["t1"][0].as_f64().unwrap() > 0.0);

        // Live notes to the live target.
        let id = "t1";
        unsafe { dd_set_live_target(id.as_ptr(), id.len()) };
        unsafe { dd_live_note(64, 1.0, 1, std::ptr::null(), 0) };
        dd_process(128);

        // Offline render.
        let out = unsafe { dd_render(project.as_ptr(), project.len(), 48_000, 0.0, 2.0, 0.5) };
        let n = dd_result_len();
        assert_eq!(n, (1.5 * 48_000.0) as usize * 2);
        let samples = unsafe { std::slice::from_raw_parts(out, n) };
        assert!(samples.iter().any(|v| v.abs() > 0.01));

        // Memory helpers.
        let m = dd_alloc(100);
        unsafe { dd_free(m, 100) };
        let cat = dd_catalog();
        assert!(!cat.is_null() && dd_result_len() > 1000);
    }
}
