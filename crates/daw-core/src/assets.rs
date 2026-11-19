//! Audio file decoding (symphonia), WAV writing (hound) and waveform peaks.

use anyhow::Result;
#[cfg(feature = "decode")]
use anyhow::{Context, anyhow};
use serde::Serialize;
use std::collections::HashMap;
#[cfg(feature = "decode")]
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "decode")]
use symphonia::core::codecs::audio::AudioDecoderOptions;
#[cfg(feature = "decode")]
use symphonia::core::errors::Error as SymError;
#[cfg(feature = "decode")]
use symphonia::core::formats::probe::Hint;
#[cfg(feature = "decode")]
use symphonia::core::formats::{FormatOptions, TrackType};
#[cfg(feature = "decode")]
use symphonia::core::io::MediaSourceStream;
#[cfg(feature = "decode")]
use symphonia::core::meta::MetadataOptions;

/// Decoded audio kept in memory, de-interleaved. Mono files have one channel.
#[derive(Debug)]
pub struct AudioBuffer {
    pub sample_rate: f32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioBuffer {
    pub fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    /// 4-point Hermite interpolated read at fractional frame `pos`.
    #[inline]
    pub fn read(&self, ch: usize, pos: f64) -> f32 {
        let data = &self.channels[ch.min(self.channels.len() - 1)];
        let n = data.len() as isize;
        let i = pos.floor() as isize;
        let t = (pos - i as f64) as f32;
        let at = |k: isize| -> f32 { if k >= 0 && k < n { data[k as usize] } else { 0.0 } };
        let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c0 = x0;
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + c0
    }
}

#[cfg(feature = "decode")]
pub fn decode_file(path: &Path) -> Result<AudioBuffer> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| anyhow!("unsupported audio file: {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow!("no audio track"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("no audio codec parameters"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow!("unsupported codec: {e}"))?;
    let track_id = track.id;

    let mut sample_rate = 0u32;
    let mut n_ch = 0usize;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut tmp: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::ResetRequired) => break,
            Err(SymError::IoError(_)) => break,
            Err(e) => return Err(anyhow!("decode error: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if n_ch == 0 {
                    sample_rate = buf.spec().rate();
                    n_ch = buf.spec().channels().count().max(1);
                    channels = vec![Vec::new(); n_ch.min(2)];
                }
                tmp.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut tmp);
                let ch_in = buf.spec().channels().count().max(1);
                for frame in tmp.chunks_exact(ch_in) {
                    if channels.len() == 1 {
                        channels[0].push(frame[0]);
                    } else {
                        channels[0].push(frame[0]);
                        channels[1].push(frame[1.min(ch_in - 1)]);
                    }
                }
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("decode error: {e}")),
        }
    }
    if channels.is_empty() || channels[0].is_empty() {
        return Err(anyhow!("file contains no audio"));
    }
    Ok(AudioBuffer { sample_rate: sample_rate as f32, channels })
}

/// Writes interleaved stereo f32 samples as a WAV file.
pub fn write_wav(path: &Path, interleaved: &[f32], channels: u16, sample_rate: u32, bits: u16) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: bits,
        sample_format: if bits == 32 { hound::SampleFormat::Float } else { hound::SampleFormat::Int },
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    match bits {
        16 => {
            for &s in interleaved {
                w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
            }
        }
        24 => {
            for &s in interleaved {
                w.write_sample((s.clamp(-1.0, 1.0) * 8_388_607.0) as i32)?;
            }
        }
        _ => {
            for &s in interleaved {
                w.write_sample(s)?;
            }
        }
    }
    w.finalize()?;
    Ok(())
}

/// Waveform overview: max(|x|) per bucket of `PEAK_RES` frames, both channels mixed.
pub const PEAKS_PER_SECOND: f64 = 200.0;

pub fn compute_peaks(buf: &AudioBuffer) -> Vec<f32> {
    let bucket = (buf.sample_rate as f64 / PEAKS_PER_SECOND).max(1.0);
    let frames = buf.frames();
    let n = (frames as f64 / bucket).ceil() as usize;
    let mut out = vec![0f32; n];
    for ch in &buf.channels {
        for (i, s) in ch.iter().enumerate() {
            let b = (i as f64 / bucket) as usize;
            let a = s.abs();
            if a > out[b] {
                out[b] = a;
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub duration: f64,
    pub channels: usize,
    pub sample_rate: f32,
    pub peaks: Vec<f32>,
    pub peaks_per_second: f64,
}

#[derive(Default)]
pub struct AssetCache {
    pub buffers: HashMap<String, Arc<AudioBuffer>>,
    pub paths: HashMap<String, String>,
}

impl AssetCache {
    #[cfg(feature = "decode")]
    pub fn load(&mut self, id: &str, path: &str) -> Result<AssetInfo> {
        let buf = if let (Some(b), Some(p)) = (self.buffers.get(id), self.paths.get(id)) {
            if p == path { Some(b.clone()) } else { None }
        } else {
            None
        };
        let buf = match buf {
            Some(b) => b,
            None => {
                let b = Arc::new(decode_file(Path::new(path))?);
                self.buffers.insert(id.to_string(), b.clone());
                self.paths.insert(id.to_string(), path.to_string());
                b
            }
        };
        Ok(Self::info(id, path, &buf))
    }

    pub fn insert(&mut self, id: &str, path: &str, buf: AudioBuffer) -> AssetInfo {
        let b = Arc::new(buf);
        self.buffers.insert(id.to_string(), b.clone());
        self.paths.insert(id.to_string(), path.to_string());
        Self::info(id, path, &b)
    }

    pub fn info(id: &str, path: &str, buf: &AudioBuffer) -> AssetInfo {
        AssetInfo {
            id: id.to_string(),
            path: path.to_string(),
            name: Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Audio")
                .to_string(),
            duration: buf.duration(),
            channels: buf.channels.len(),
            sample_rate: buf.sample_rate,
            peaks: compute_peaks(buf),
            peaks_per_second: PEAKS_PER_SECOND,
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<AudioBuffer>> {
        self.buffers.get(id).cloned()
    }
}
