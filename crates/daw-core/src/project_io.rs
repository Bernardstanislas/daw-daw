//! Project save/load and offline mixdown.
//!
//! A project is a folder (`MySong.dawdaw/`) containing `project.json` and an
//! `Audio/` sub-folder with every audio file the project uses.

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::assets::{AssetCache, write_wav};
use crate::engine::commands::Command;
use crate::engine::controller::Controller;
use crate::model::Project;

pub const PROJECT_FILE: &str = "project.json";
pub const AUDIO_DIR: &str = "Audio";

fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    let p = dir.join(file_name);
    if !p.exists() {
        return p;
    }
    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
    let ext = Path::new(file_name).extension().and_then(|s| s.to_str()).unwrap_or("wav");
    for i in 2.. {
        let p = dir.join(format!("{stem} {i}.{ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

/// Saves the document. Audio files outside the project folder are copied
/// into `Audio/`. Returns the document with (absolute) updated asset paths.
pub fn save(dir: &Path, mut doc: Value) -> Result<Value> {
    std::fs::create_dir_all(dir.join(AUDIO_DIR)).with_context(|| format!("cannot create {}", dir.display()))?;
    let dir = dir.canonicalize()?;
    let audio_dir = dir.join(AUDIO_DIR);
    let mut stored = doc.clone();

    if let (Some(assets), Some(stored_assets)) = (
        doc.get_mut("assets").and_then(|a| a.as_array_mut()),
        stored.get_mut("assets").and_then(|a| a.as_array_mut()),
    ) {
        for (a, s) in assets.iter_mut().zip(stored_assets.iter_mut()) {
            let Some(path) = a.get("path").and_then(|p| p.as_str()).map(PathBuf::from) else { continue };
            let abs = if path.is_absolute() { path.clone() } else { dir.join(&path) };
            let inside = abs.canonicalize().map(|c| c.starts_with(&dir)).unwrap_or(false);
            let final_abs = if inside {
                abs.canonicalize()?
            } else if abs.exists() {
                let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("audio.wav").to_string();
                let dest = unique_path(&audio_dir, &name);
                std::fs::copy(&abs, &dest).with_context(|| format!("cannot copy {}", abs.display()))?;
                dest
            } else {
                // Missing file: keep the reference as is.
                abs
            };
            let rel = final_abs.strip_prefix(&dir).map(|p| p.to_path_buf()).unwrap_or(final_abs.clone());
            a["path"] = Value::String(final_abs.to_string_lossy().into_owned());
            s["path"] = Value::String(rel.to_string_lossy().into_owned());
        }
    }
    let json = serde_json::to_string_pretty(&stored)?;
    let tmp = dir.join(format!("{PROJECT_FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, dir.join(PROJECT_FILE))?;
    Ok(doc)
}

/// Loads a project folder (or a `project.json` path). Asset paths are made absolute.
pub fn load(path: &Path) -> Result<(PathBuf, Value)> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().ok_or_else(|| anyhow!("invalid project path"))?.to_path_buf()
    };
    let file = dir.join(PROJECT_FILE);
    let text = std::fs::read_to_string(&file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut doc: Value = serde_json::from_str(&text).context("project.json is not valid JSON")?;
    if let Some(assets) = doc.get_mut("assets").and_then(|a| a.as_array_mut()) {
        for a in assets.iter_mut() {
            if let Some(p) = a.get("path").and_then(|p| p.as_str()) {
                let pb = PathBuf::from(p);
                if !pb.is_absolute() {
                    a["path"] = Value::String(dir.join(pb).to_string_lossy().into_owned());
                }
            }
        }
    }
    Ok((dir, doc))
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub sample_rate: u32,
    pub bits: u16,
    pub start: f64,
    pub end: f64,
    /// Extra seconds rendered after `end` for reverb / release tails.
    pub tail: f64,
}

/// Renders the project offline into interleaved stereo samples.
pub fn render_offline(
    project: &Project,
    assets: &AssetCache,
    opts: ExportOptions,
    mut progress: impl FnMut(f32),
) -> Vec<f32> {
    let sr = opts.sample_rate as f32;
    let (mut ctrl, mut engine) = Controller::new(sr);
    ctrl.offline = true;
    ctrl.assets.buffers = assets.buffers.clone();
    ctrl.assets.paths = assets.paths.clone();
    let mut p = project.clone();
    p.metronome.enabled = false;
    p.loop_region.enabled = false;
    ctrl.sync(&p);
    ctrl.send(Command::Seek(opts.start.max(0.0)));
    ctrl.send(Command::Play);

    let bpm = if p.bpm > 0.0 { p.bpm } else { 120.0 };
    let seconds = (opts.end - opts.start).max(0.0) * 60.0 / bpm + opts.tail.max(0.0);
    let total = (seconds * sr as f64) as usize;
    let mut out = vec![0f32; total * 2];
    let chunk = 1024;
    let mut done = 0;
    let mut last = 0.0;
    while done < total {
        let n = chunk.min(total - done);
        engine.process(&mut out[done * 2..(done + n) * 2], 2);
        ctrl.collect_garbage();
        done += n;
        let pr = done as f32 / total.max(1) as f32;
        if pr - last > 0.01 {
            progress(pr);
            last = pr;
        }
    }
    progress(1.0);
    out
}

pub fn export_wav(path: &Path, project: &Project, assets: &AssetCache, opts: ExportOptions, progress: impl FnMut(f32)) -> Result<()> {
    let samples = render_offline(project, assets, opts, progress);
    write_wav(path, &samples, 2, opts.sample_rate, opts.bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_copies_audio_and_load_restores_absolute_paths() {
        let tmp = std::env::temp_dir().join(format!("dawdaw-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let wav = tmp.join("loop.wav");
        write_wav(&wav, &vec![0.1; 200], 2, 48_000, 16).unwrap();
        let proj_dir = tmp.join("Song.dawdaw");
        let doc = serde_json::json!({
            "name": "Song", "bpm": 100.0, "sections": [{"id": "s", "name": "Intro"}],
            "assets": [{"id": "a", "path": wav.to_string_lossy(), "name": "loop"}],
            "tracks": []
        });
        let saved = save(&proj_dir, doc).unwrap();
        let abs = saved["assets"][0]["path"].as_str().unwrap().to_string();
        assert!(abs.contains("Song.dawdaw/Audio/loop.wav"), "{abs}");
        assert!(std::path::Path::new(&abs).exists());
        // On disk the path is relative.
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(proj_dir.join(PROJECT_FILE)).unwrap()).unwrap();
        assert_eq!(on_disk["assets"][0]["path"], "Audio/loop.wav");
        // UI-only fields survive.
        assert_eq!(on_disk["sections"][0]["name"], "Intro");
        // Saving again does not duplicate the file.
        save(&proj_dir, saved.clone()).unwrap();
        assert_eq!(std::fs::read_dir(proj_dir.join(AUDIO_DIR)).unwrap().count(), 1);
        let (dir, loaded) = load(&proj_dir).unwrap();
        assert_eq!(dir, proj_dir);
        assert_eq!(loaded["assets"][0]["path"].as_str().unwrap(), proj_dir.join("Audio/loop.wav").to_string_lossy());
        #[cfg(feature = "decode")]
        {
            let buf = crate::assets::decode_file(std::path::Path::new(&abs)).unwrap();
            assert_eq!(buf.frames(), 100);
            assert_eq!(buf.channels.len(), 2);
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
