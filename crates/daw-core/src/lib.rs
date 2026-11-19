//! daw-daw audio engine, shared by the desktop app (Tauri + cpal) and the
//! browser build (WebAssembly in an AudioWorklet).

pub mod assets;
pub mod dsp;
pub mod engine;
pub mod model;
pub mod project_io;
