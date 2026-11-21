//! Audio Unit (v2) hosting: scanning, instantiation, rendering, MIDI, state.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Result, anyhow, bail};
use super::PluginInfo;
use super::ffi::*;
use daw_core::dsp::{Effect, Instrument, ParamValues, ProcessCtx};

/// Largest slice rendered in one AudioUnitRender call.
pub const MAX_FRAMES: usize = 1024;

/// The component description encoded in an "au:" plugin id.
fn desc(info: &PluginInfo) -> Option<AudioComponentDescription> {
    let hex = info.id.strip_prefix("au:")?;
    if hex.len() != 24 {
        return None;
    }
    let p = |i: usize| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok();
    Some(AudioComponentDescription {
        componentType: p(0)?,
        componentSubType: p(1)?,
        componentManufacturer: p(2)?,
        componentFlags: 0,
        componentFlagsMask: 0,
    })
}

fn make_id(d: &AudioComponentDescription) -> String {
    format!("au:{:08x}{:08x}{:08x}", d.componentType, d.componentSubType, d.componentManufacturer)
}

/// Lists installed instrument and effect Audio Units.
pub fn scan() -> Vec<PluginInfo> {
    let mut out = Vec::new();
    for (ty, kind) in [
        (kAudioUnitType_MusicDevice, "instrument"),
        (kAudioUnitType_Effect, "effect"),
        (kAudioUnitType_MusicEffect, "effect"),
    ] {
        let want = AudioComponentDescription { componentType: ty, ..Default::default() };
        let mut comp: AudioComponent = std::ptr::null_mut();
        loop {
            comp = unsafe { AudioComponentFindNext(comp, &want) };
            if comp.is_null() {
                break;
            }
            let mut d = AudioComponentDescription::default();
            if unsafe { AudioComponentGetDescription(comp, &mut d) } != 0 {
                continue;
            }
            let mut name_ref: CFStringRef = std::ptr::null();
            let full = if unsafe { AudioComponentCopyName(comp, &mut name_ref) } == 0 {
                let s = unsafe { cfstring_to_string(name_ref) };
                unsafe { CFRelease(name_ref) };
                s
            } else {
                fourcc_str(d.componentSubType)
            };
            let (vendor, name) = match full.split_once(": ") {
                Some((v, n)) => (v.trim().to_string(), n.trim().to_string()),
                None => (fourcc_str(d.componentManufacturer), full),
            };
            out.push(PluginInfo { id: make_id(&d), name, vendor, kind, format: "AU", path: None });
        }
    }
    out.sort_by(|a, b| (a.kind, a.vendor.to_lowercase(), a.name.to_lowercase()).cmp(&(b.kind, b.vendor.to_lowercase(), b.name.to_lowercase())));
    out
}

/// Transport info the plugin can query (tempo-synced LFOs, arpeggiators…).
#[derive(Default)]
pub struct HostTime {
    bpm: AtomicU64,
    beat: AtomicU64,
    playing: AtomicBool,
}

impl HostTime {
    fn update(&self, ctx: &ProcessCtx) {
        self.bpm.store((ctx.bpm as f64).to_bits(), Ordering::Relaxed);
        self.beat.store(ctx.beat.to_bits(), Ordering::Relaxed);
        self.playing.store(ctx.playing, Ordering::Relaxed);
    }
}

unsafe extern "C" fn beat_and_tempo(user: *mut c_void, beat: *mut f64, tempo: *mut f64) -> OSStatus {
    let t = unsafe { &*(user as *const HostTime) };
    if !beat.is_null() {
        unsafe { *beat = f64::from_bits(t.beat.load(Ordering::Relaxed)) };
    }
    if !tempo.is_null() {
        let bpm = f64::from_bits(t.bpm.load(Ordering::Relaxed));
        unsafe { *tempo = if bpm > 0.0 { bpm } else { 120.0 } };
    }
    0
}

unsafe extern "C" fn transport_state(
    user: *mut c_void,
    is_playing: *mut u8,
    transport_changed: *mut u8,
    _sample_in_time_line: *mut f64,
    is_cycling: *mut u8,
    _cycle_start: *mut f64,
    _cycle_end: *mut f64,
) -> OSStatus {
    let t = unsafe { &*(user as *const HostTime) };
    if !is_playing.is_null() {
        unsafe { *is_playing = t.playing.load(Ordering::Relaxed) as u8 };
    }
    if !transport_changed.is_null() {
        unsafe { *transport_changed = 0 };
    }
    if !is_cycling.is_null() {
        unsafe { *is_cycling = 0 };
    }
    0
}

/// Input of an effect plugin for the current render (audio thread only).
struct InputSlot {
    l: *const f32,
    r: *const f32,
    frames: usize,
}

struct InputState(UnsafeCell<InputSlot>);

unsafe extern "C" fn render_input(
    ref_con: *mut c_void,
    _flags: *mut u32,
    _ts: *const AudioTimeStamp,
    _bus: u32,
    frames: u32,
    io: *mut AudioBufferList2,
) -> OSStatus {
    let st = unsafe { &*(*(ref_con as *const InputState)).0.get() };
    let io = unsafe { &mut *io };
    let n = (frames as usize).min(st.frames);
    for b in 0..(io.mNumberBuffers as usize).min(2) {
        let src = if b == 0 { st.l } else { st.r };
        let buf = &mut io.mBuffers[b];
        if src.is_null() {
            continue;
        }
        if buf.mData.is_null() {
            // The unit asks us for the data pointer.
            buf.mData = src as *mut c_void;
            buf.mDataByteSize = (n * 4) as u32;
        } else {
            let dst = buf.mData as *mut f32;
            unsafe { std::ptr::copy_nonoverlapping(src, dst, n) };
        }
    }
    0
}

/// A live Audio Unit instance. Shared (Arc) between the engine and the
/// plugin manager (editor window, state); disposed when the last ref drops.
pub struct AuUnit {
    pub unit: AudioUnit,
    pub info: PluginInfo,
    host_time: Box<HostTime>,
    input: Box<InputState>,
    is_effect: bool,
}

// The AudioUnit API is thread-safe for rendering vs. property access;
// the input slot is only touched by the audio thread during a render.
unsafe impl Send for AuUnit {}
unsafe impl Sync for AuUnit {}

fn check(status: OSStatus, what: &str) -> Result<()> {
    if status == 0 { Ok(()) } else { bail!("{what} failed (OSStatus {status})") }
}

impl AuUnit {
    pub fn new(info: PluginInfo, sample_rate: f32) -> Result<Arc<Self>> {
        let desc = desc(&info).ok_or_else(|| anyhow!("bad plugin id {}", info.id))?;
        let comp = unsafe { AudioComponentFindNext(std::ptr::null_mut(), &desc) };
        if comp.is_null() {
            bail!("{} is not installed", info.name);
        }
        let mut unit: AudioUnit = std::ptr::null_mut();
        check(unsafe { AudioComponentInstanceNew(comp, &mut unit) }, "AudioComponentInstanceNew")?;
        let is_effect = info.kind == "effect";
        let me = Self {
            unit,
            info,
            host_time: Box::new(HostTime::default()),
            input: Box::new(InputState(UnsafeCell::new(InputSlot { l: std::ptr::null(), r: std::ptr::null(), frames: 0 }))),
            is_effect,
        };
        me.configure(sample_rate)?;
        check(unsafe { AudioUnitInitialize(unit) }, "AudioUnitInitialize")?;
        Ok(Arc::new(me))
    }

    fn set_prop<T>(&self, id: u32, scope: u32, element: u32, value: &T) -> OSStatus {
        unsafe { AudioUnitSetProperty(self.unit, id, scope, element, value as *const T as *const c_void, std::mem::size_of::<T>() as u32) }
    }

    fn configure(&self, sample_rate: f32) -> Result<()> {
        let fmt = AudioStreamBasicDescription {
            mSampleRate: sample_rate as f64,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagIsNonInterleaved,
            mBytesPerPacket: 4,
            mFramesPerPacket: 1,
            mBytesPerFrame: 4,
            mChannelsPerFrame: 2,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        check(self.set_prop(kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 0, &fmt), "output format")?;
        if self.is_effect {
            check(self.set_prop(kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &fmt), "input format")?;
            let cb = AURenderCallbackStruct {
                inputProc: render_input,
                inputProcRefCon: &*self.input as *const InputState as *mut c_void,
            };
            check(self.set_prop(kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &cb), "render callback")?;
        }
        let max = MAX_FRAMES as u32;
        let _ = self.set_prop(kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, &max);
        let callbacks = HostCallbackInfo {
            hostUserData: &*self.host_time as *const HostTime as *mut c_void,
            beatAndTempoProc: Some(beat_and_tempo),
            musicalTimeLocationProc: None,
            transportStateProc: Some(transport_state),
            transportStateProc2: None,
        };
        let _ = self.set_prop(kAudioUnitProperty_HostCallbacks, kAudioUnitScope_Global, 0, &callbacks);
        Ok(())
    }

    /// Saved plugin state (binary property list of the AU ClassInfo).
    pub fn get_state(&self) -> Option<Vec<u8>> {
        let mut plist: CFPropertyListRef = std::ptr::null();
        let mut size = std::mem::size_of::<CFPropertyListRef>() as u32;
        let st = unsafe {
            AudioUnitGetProperty(self.unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &mut plist as *mut _ as *mut c_void, &mut size)
        };
        if st != 0 || plist.is_null() {
            return None;
        }
        let data = unsafe { CFPropertyListCreateData(std::ptr::null(), plist, kCFPropertyListBinaryFormat_v1_0, 0, std::ptr::null_mut()) };
        unsafe { CFRelease(plist) };
        if data.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize).to_vec() };
        unsafe { CFRelease(data) };
        Some(bytes)
    }

    pub fn set_state(&self, bytes: &[u8]) -> Result<()> {
        let data = unsafe { CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize) };
        if data.is_null() {
            bail!("CFDataCreate failed");
        }
        let plist = unsafe { CFPropertyListCreateWithData(std::ptr::null(), data, 0, std::ptr::null_mut(), std::ptr::null_mut()) };
        unsafe { CFRelease(data) };
        if plist.is_null() {
            bail!("invalid plugin state");
        }
        let st = self.set_prop(kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &plist);
        unsafe { CFRelease(plist) };
        check(st, "restore state")
    }

    #[inline]
    fn midi(&self, status: u32, d1: u32, d2: u32) {
        unsafe { MusicDeviceMIDIEvent(self.unit, status, d1, d2, 0) };
    }
}

impl Drop for AuUnit {
    fn drop(&mut self) {
        // The last reference is often released by the engine's clean-up thread:
        // dispose on the main thread, where the plugin's UI objects live.
        let unit = self.unit as usize;
        super::on_main_async(move || unsafe {
            let unit = unit as AudioUnit;
            AudioUnitUninitialize(unit);
            AudioComponentInstanceDispose(unit);
        });
    }
}

/// Engine-side device wrapping an AuUnit (implements Instrument and Effect).
pub struct AuDevice {
    unit: Arc<AuUnit>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    sample_time: f64,
}

impl AuDevice {
    pub fn new(unit: Arc<AuUnit>) -> Self {
        Self { unit, out_l: vec![0.0; MAX_FRAMES], out_r: vec![0.0; MAX_FRAMES], sample_time: 0.0 }
    }

    /// Renders `l.len()` frames. Instruments add into l/r, effects replace them.
    fn render(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32], add: bool) {
        self.unit.host_time.update(ctx);
        let mut done = 0;
        let total = l.len();
        while done < total {
            let n = (total - done).min(MAX_FRAMES);
            if self.unit.is_effect {
                let slot = unsafe { &mut *self.unit.input.0.get() };
                slot.l = l[done..].as_ptr();
                slot.r = r[done..].as_ptr();
                slot.frames = n;
            }
            let mut list = AudioBufferList2 {
                mNumberBuffers: 2,
                mBuffers: [
                    AudioBuffer { mNumberChannels: 1, mDataByteSize: (n * 4) as u32, mData: self.out_l.as_mut_ptr() as *mut c_void },
                    AudioBuffer { mNumberChannels: 1, mDataByteSize: (n * 4) as u32, mData: self.out_r.as_mut_ptr() as *mut c_void },
                ],
            };
            let ts = AudioTimeStamp { mSampleTime: self.sample_time, mFlags: kAudioTimeStampSampleTimeValid, ..Default::default() };
            let mut flags = 0u32;
            let st = unsafe { AudioUnitRender(self.unit.unit, &mut flags, &ts, 0, n as u32, &mut list) };
            self.sample_time += n as f64;
            if st == 0 {
                // The unit may hand back its own buffers.
                let pl = list.mBuffers[0].mData as *const f32;
                let pr = if list.mNumberBuffers > 1 { list.mBuffers[1].mData as *const f32 } else { pl };
                let (ol, or) = unsafe { (std::slice::from_raw_parts(pl, n), std::slice::from_raw_parts(pr, n)) };
                for i in 0..n {
                    let (a, b) = (ol[i], or[i]);
                    let (a, b) = (if a.is_finite() { a } else { 0.0 }, if b.is_finite() { b } else { 0.0 });
                    if add {
                        l[done + i] += a;
                        r[done + i] += b;
                    } else {
                        l[done + i] = a;
                        r[done + i] = b;
                    }
                }
            }
            // On a render error an effect leaves its input untouched (pass-through).
            if self.unit.is_effect {
                let slot = unsafe { &mut *self.unit.input.0.get() };
                slot.l = std::ptr::null();
                slot.r = std::ptr::null();
                slot.frames = 0;
            }
            done += n;
        }
    }
}

impl Instrument for AuDevice {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let v = ((velocity.clamp(0.0, 1.0) * 127.0).round() as u32).max(1);
        self.unit.midi(0x90, pitch as u32, v);
    }
    fn note_off(&mut self, pitch: u8) {
        self.unit.midi(0x80, pitch as u32, 0);
    }
    fn all_notes_off(&mut self, hard: bool) {
        self.unit.midi(0xB0, 123, 0); // all notes off
        self.unit.midi(0xB0, 64, 0); // sustain off
        if hard {
            self.unit.midi(0xB0, 120, 0); // all sound off
        }
    }
    fn set_params(&mut self, _values: &ParamValues) {}
    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        self.render(ctx, l, r, true);
    }
}

impl Effect for AuDevice {
    fn set_params(&mut self, _values: &ParamValues) {}
    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        self.render(ctx, l, r, false);
    }
    fn reset(&mut self) {
        unsafe { AudioUnitReset(self.unit.unit, kAudioUnitScope_Global, 0) };
    }
}
