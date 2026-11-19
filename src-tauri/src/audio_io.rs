//! Audio device management (cpal). Streams live on a dedicated thread
//! because cpal streams are not `Send` on every platform.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use serde::Serialize;

use crate::engine::Engine;
use crate::engine::shared::Shared;

pub type EngineSlot = Arc<Mutex<Option<Engine>>>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfo {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

fn device_name(d: &cpal::Device) -> String {
    d.description()
        .map(|desc| desc.name().to_string())
        .or_else(|_| d.id().map(|i| i.to_string()))
        .unwrap_or_else(|_| "Unknown device".into())
}

pub fn list_devices() -> DeviceList {
    let host = cpal::default_host();
    let outputs = host
        .output_devices()
        .map(|it| it.map(|d| device_name(&d)).collect())
        .unwrap_or_default();
    let inputs = host
        .input_devices()
        .map(|it| it.map(|d| device_name(&d)).collect())
        .unwrap_or_default();
    DeviceList {
        outputs,
        inputs,
        default_output: host.default_output_device().map(|d| device_name(&d)),
        default_input: host.default_input_device().map(|d| device_name(&d)),
    }
}

fn find_output(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(n) = name
        && let Ok(mut it) = host.output_devices()
            && let Some(d) = it.find(|d| device_name(d) == n) {
                return Some(d);
            }
    host.default_output_device()
}

fn find_input(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(n) = name
        && let Ok(mut it) = host.input_devices()
            && let Some(d) = it.find(|d| device_name(d) == n) {
                return Some(d);
            }
    host.default_input_device()
}

/// Returns the sample rate the output device would run at.
pub fn probe_output_rate(name: Option<&str>) -> Option<u32> {
    find_output(name)?.default_output_config().ok().map(|c| c.sample_rate())
}

/// Where captured input samples go while recording.
pub struct Capture {
    pub producer: Option<rtrb::Producer<f32>>,
}

enum HostMsg {
    OpenOutput { device: Option<String>, buffer: Option<u32>, reply: mpsc::Sender<Result<OutputInfo, String>> },
    OpenInput { device: Option<String>, reply: mpsc::Sender<Result<(u32, u16), String>> },
    CloseInput,
}

#[derive(Clone)]
pub struct AudioHost {
    tx: mpsc::Sender<HostMsg>,
}

impl AudioHost {
    pub fn start(slot: EngineSlot, shared_fn: Arc<dyn Fn() -> Arc<Shared> + Send + Sync>, capture: Arc<Mutex<Capture>>) -> Self {
        let (tx, rx) = mpsc::channel::<HostMsg>();
        thread::Builder::new()
            .name("audio-host".into())
            .spawn(move || {
                let mut output: Option<cpal::Stream> = None;
                let mut input: Option<cpal::Stream> = None;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        HostMsg::OpenOutput { device, buffer, reply } => {
                            output = None; // drop the previous stream first
                            let res = open_output(&slot, device.as_deref(), buffer);
                            match res {
                                Ok((stream, info)) => {
                                    output = Some(stream);
                                    let _ = reply.send(Ok(info));
                                }
                                Err(e) => {
                                    let _ = reply.send(Err(e));
                                }
                            }
                        }
                        HostMsg::OpenInput { device, reply } => {
                            input = None;
                            match open_input(device.as_deref(), shared_fn(), capture.clone()) {
                                Ok((s, sr, ch)) => {
                                    input = Some(s);
                                    let _ = reply.send(Ok((sr, ch)));
                                }
                                Err(e) => {
                                    let _ = reply.send(Err(e));
                                }
                            }
                        }
                        HostMsg::CloseInput => input = None,
                    }
                }
                drop(output);
                drop(input);
            })
            .expect("spawn audio host thread");
        Self { tx }
    }

    pub fn open_output(&self, device: Option<String>, buffer: Option<u32>) -> Result<OutputInfo, String> {
        let (reply, rx) = mpsc::channel();
        self.tx.send(HostMsg::OpenOutput { device, buffer, reply }).map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?
    }

    pub fn open_input(&self, device: Option<String>) -> Result<(u32, u16), String> {
        let (reply, rx) = mpsc::channel();
        self.tx.send(HostMsg::OpenInput { device, reply }).map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?
    }

    pub fn close_input(&self) {
        let _ = self.tx.send(HostMsg::CloseInput);
    }
}

fn open_output(slot: &EngineSlot, name: Option<&str>, buffer: Option<u32>) -> Result<(cpal::Stream, OutputInfo), String> {
    let device = find_output(name).ok_or("no audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let sample_format = supported.sample_format();
    let mut config: cpal::StreamConfig = supported.into();
    if let Some(b) = buffer {
        config.buffer_size = cpal::BufferSize::Fixed(b);
    }
    let channels = config.channels as usize;
    let info = OutputInfo {
        device: device_name(&device),
        sample_rate: config.sample_rate,
        channels: config.channels,
        buffer_size: buffer,
    };
    let err_fn = |e: cpal::Error| eprintln!("[audio] output stream error: {e}");

    let build = |config: &cpal::StreamConfig| -> Result<cpal::Stream, cpal::Error> {
        match sample_format {
            cpal::SampleFormat::F32 => {
                let slot = slot.clone();
                device.build_output_stream(
                    *config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| render(&slot, data, channels),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let slot = slot.clone();
                let mut scratch = vec![0f32; 1 << 16];
                device.build_output_stream(
                    *config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        if scratch.len() < data.len() {
                            scratch.resize(data.len(), 0.0);
                        }
                        let s = &mut scratch[..data.len()];
                        render(&slot, s, channels);
                        for (o, i) in data.iter_mut().zip(s.iter()) {
                            *o = (i * 32767.0) as i16;
                        }
                    },
                    err_fn,
                    None,
                )
            }
            _ => {
                let slot = slot.clone();
                let mut scratch = vec![0f32; 1 << 16];
                device.build_output_stream(
                    *config,
                    move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                        if scratch.len() < data.len() {
                            scratch.resize(data.len(), 0.0);
                        }
                        let s = &mut scratch[..data.len()];
                        render(&slot, s, channels);
                        for (o, i) in data.iter_mut().zip(s.iter()) {
                            *o = (*i as f64 * i32::MAX as f64) as i32;
                        }
                    },
                    err_fn,
                    None,
                )
            }
        }
    };

    let stream = match build(&config) {
        Ok(s) => s,
        Err(e) if buffer.is_some() => {
            eprintln!("[audio] fixed buffer rejected ({e}), using default");
            config.buffer_size = cpal::BufferSize::Default;
            build(&config).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, info))
}

#[inline]
fn render(slot: &EngineSlot, data: &mut [f32], channels: usize) {
    match slot.try_lock() {
        Some(mut guard) => match guard.as_mut() {
            Some(engine) => engine.process(data, channels),
            None => data.fill(0.0),
        },
        None => data.fill(0.0),
    }
}

fn open_input(name: Option<&str>, shared: Arc<Shared>, capture: Arc<Mutex<Capture>>) -> Result<(cpal::Stream, u32, u16), String> {
    let device = find_input(name).ok_or("no audio input device")?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let ch = config.channels as usize;
    let sr = config.sample_rate;
    let err_fn = |e: cpal::Error| eprintln!("[audio] input stream error: {e}");

    // Pushes the first (up to) two channels as interleaved stereo.
    fn feed(frame_iter: impl Iterator<Item = (f32, f32)>, shared: &Shared, capture: &Mutex<Capture>) {
        let recording = shared.recording.load(Ordering::Acquire);
        let mut guard = capture.try_lock();
        let prod = guard.as_mut().and_then(|g| g.producer.as_mut());
        let mut pl = 0f32;
        let mut pr = 0f32;
        match (recording, prod) {
            (true, Some(p)) => {
                for (l, r) in frame_iter {
                    pl = pl.max(l.abs());
                    pr = pr.max(r.abs());
                    let _ = p.push(l);
                    let _ = p.push(r);
                }
            }
            _ => {
                for (l, r) in frame_iter {
                    pl = pl.max(l.abs());
                    pr = pr.max(r.abs());
                }
            }
        }
        shared.input.feed(pl, pr);
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let it = data.chunks_exact(ch).map(|f| (f[0], if ch > 1 { f[1] } else { f[0] }));
                feed(it, &shared, &capture);
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let it = data.chunks_exact(ch).map(|f| {
                    let l = f[0] as f32 / 32768.0;
                    (l, if ch > 1 { f[1] as f32 / 32768.0 } else { l })
                });
                feed(it, &shared, &capture);
            },
            err_fn,
            None,
        ),
        _ => device.build_input_stream(
            config,
            move |data: &[i32], _: &cpal::InputCallbackInfo| {
                let it = data.chunks_exact(ch).map(|f| {
                    let l = f[0] as f32 / i32::MAX as f32;
                    (l, if ch > 1 { f[1] as f32 / i32::MAX as f32 } else { l })
                });
                feed(it, &shared, &capture);
            },
            err_fn,
            None,
        ),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, sr, config.channels))
}

/// What a finished recording produced.
#[derive(Debug, Clone)]
pub struct RecordedTake {
    pub track_id: String,
    pub path: PathBuf,
    pub start_beat: f64,
}

pub struct RecordTarget {
    pub track_id: String,
    pub path: PathBuf,
    pub sample_rate: u32,
}

/// Background thread draining captured samples into a WAV file while the
/// engine reports `recording`.
pub fn spawn_recorder(
    shared_fn: Arc<dyn Fn() -> Arc<Shared> + Send + Sync>,
    capture: Arc<Mutex<Capture>>,
    target: Arc<Mutex<Option<RecordTarget>>>,
    on_done: impl Fn(RecordedTake) + Send + 'static,
) {
    thread::Builder::new()
        .name("recorder".into())
        .spawn(move || {
            let mut consumer: Option<rtrb::Consumer<f32>> = None;
            let mut writer: Option<(hound::WavWriter<std::io::BufWriter<std::fs::File>>, String, PathBuf, f64)> = None;
            loop {
                thread::sleep(Duration::from_millis(5));
                let shared = shared_fn();
                // (Re)create the ring buffer the first time.
                if consumer.is_none() {
                    let (p, c) = rtrb::RingBuffer::<f32>::new(48_000 * 2 * 20);
                    capture.lock().producer = Some(p);
                    consumer = Some(c);
                }
                let c = consumer.as_mut().unwrap();
                let recording = shared.recording.load(Ordering::Acquire);
                if recording && writer.is_none()
                    && let Some(t) = target.lock().take() {
                        let spec = hound::WavSpec {
                            channels: 2,
                            sample_rate: t.sample_rate,
                            bits_per_sample: 32,
                            sample_format: hound::SampleFormat::Float,
                        };
                        if let Some(dir) = t.path.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        match hound::WavWriter::create(&t.path, spec) {
                            Ok(w) => writer = Some((w, t.track_id, t.path, shared.record_start.load())),
                            Err(e) => eprintln!("[rec] cannot create {}: {e}", t.path.display()),
                        }
                    }
                let n = c.slots();
                if let Ok(chunk) = c.read_chunk(n) {
                    if let Some((w, ..)) = writer.as_mut() {
                        for s in chunk.into_iter() {
                            let _ = w.write_sample(s);
                        }
                    } else {
                        chunk.commit_all();
                    }
                }
                if !recording
                    && let Some((w, track_id, path, start_beat)) = writer.take()
                        && w.finalize().is_ok() {
                            on_done(RecordedTake { track_id, path, start_beat });
                        }
            }
        })
        .expect("spawn recorder");
}
