//! Minimal hand-written bindings to AudioToolbox (Audio Unit v2 hosting) and
//! the CoreFoundation calls we need. Only what the plugin host uses.

#![allow(non_upper_case_globals, non_snake_case, dead_code)]

use std::ffi::c_void;

pub type OSStatus = i32;
pub type AudioComponent = *mut c_void;
pub type AudioUnit = *mut c_void;
pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFDataRef = *const c_void;
pub type CFURLRef = *const c_void;
pub type CFPropertyListRef = *const c_void;

pub const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

pub fn fourcc_str(v: u32) -> String {
    let b = v.to_be_bytes();
    b.iter().map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }).collect()
}

pub fn fourcc_parse(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    (b.len() == 4).then(|| fourcc(&[b[0], b[1], b[2], b[3]]))
}

pub const kAudioUnitType_MusicDevice: u32 = fourcc(b"aumu");
pub const kAudioUnitType_Effect: u32 = fourcc(b"aufx");
pub const kAudioUnitType_MusicEffect: u32 = fourcc(b"aumf");

pub const kAudioUnitScope_Global: u32 = 0;
pub const kAudioUnitScope_Input: u32 = 1;
pub const kAudioUnitScope_Output: u32 = 2;

pub const kAudioUnitProperty_ClassInfo: u32 = 0;
pub const kAudioUnitProperty_StreamFormat: u32 = 8;
pub const kAudioUnitProperty_ElementCount: u32 = 11;
pub const kAudioUnitProperty_MaximumFramesPerSlice: u32 = 14;
pub const kAudioUnitProperty_SetRenderCallback: u32 = 23;
pub const kAudioUnitProperty_HostCallbacks: u32 = 27;
pub const kAudioUnitProperty_CocoaUI: u32 = 31;

pub const kAudioFormatLinearPCM: u32 = fourcc(b"lpcm");
pub const kAudioFormatFlagIsFloat: u32 = 1;
pub const kAudioFormatFlagIsPacked: u32 = 1 << 3;
pub const kAudioFormatFlagIsNonInterleaved: u32 = 1 << 5;
pub const kAudioTimeStampSampleTimeValid: u32 = 1;

pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;
pub const kCFPropertyListBinaryFormat_v1_0: isize = 200;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioComponentDescription {
    pub componentType: u32,
    pub componentSubType: u32,
    pub componentManufacturer: u32,
    pub componentFlags: u32,
    pub componentFlagsMask: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioStreamBasicDescription {
    pub mSampleRate: f64,
    pub mFormatID: u32,
    pub mFormatFlags: u32,
    pub mBytesPerPacket: u32,
    pub mFramesPerPacket: u32,
    pub mBytesPerFrame: u32,
    pub mChannelsPerFrame: u32,
    pub mBitsPerChannel: u32,
    pub mReserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SMPTETime {
    pub mSubframes: i16,
    pub mSubframeDivisor: i16,
    pub mCounter: u32,
    pub mType: u32,
    pub mFlags: u32,
    pub mHours: i16,
    pub mMinutes: i16,
    pub mSeconds: i16,
    pub mFrames: i16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioTimeStamp {
    pub mSampleTime: f64,
    pub mHostTime: u64,
    pub mRateScalar: f64,
    pub mWordClockTime: u64,
    pub mSMPTETime: SMPTETime,
    pub mFlags: u32,
    pub mReserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioBuffer {
    pub mNumberChannels: u32,
    pub mDataByteSize: u32,
    pub mData: *mut c_void,
}

/// AudioBufferList with room for two (non-interleaved stereo) buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioBufferList2 {
    pub mNumberBuffers: u32,
    pub mBuffers: [AudioBuffer; 2],
}

pub type AURenderCallback = unsafe extern "C" fn(
    in_ref_con: *mut c_void,
    io_action_flags: *mut u32,
    in_time_stamp: *const AudioTimeStamp,
    in_bus_number: u32,
    in_number_frames: u32,
    io_data: *mut AudioBufferList2,
) -> OSStatus;

#[repr(C)]
pub struct AURenderCallbackStruct {
    pub inputProc: AURenderCallback,
    pub inputProcRefCon: *mut c_void,
}

pub type BeatAndTempoProc = unsafe extern "C" fn(user: *mut c_void, beat: *mut f64, tempo: *mut f64) -> OSStatus;
pub type TransportStateProc = unsafe extern "C" fn(
    user: *mut c_void,
    is_playing: *mut u8,
    transport_changed: *mut u8,
    sample_in_time_line: *mut f64,
    is_cycling: *mut u8,
    cycle_start_beat: *mut f64,
    cycle_end_beat: *mut f64,
) -> OSStatus;

#[repr(C)]
pub struct HostCallbackInfo {
    pub hostUserData: *mut c_void,
    pub beatAndTempoProc: Option<BeatAndTempoProc>,
    pub musicalTimeLocationProc: Option<unsafe extern "C" fn()>,
    pub transportStateProc: Option<TransportStateProc>,
    pub transportStateProc2: Option<unsafe extern "C" fn()>,
}

#[repr(C)]
pub struct AudioUnitCocoaViewInfo {
    pub mCocoaAUViewBundleLocation: CFURLRef,
    pub mCocoaAUViewClass: [CFStringRef; 1],
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    pub fn AudioComponentFindNext(in_component: AudioComponent, desc: *const AudioComponentDescription) -> AudioComponent;
    pub fn AudioComponentGetDescription(component: AudioComponent, out_desc: *mut AudioComponentDescription) -> OSStatus;
    pub fn AudioComponentCopyName(component: AudioComponent, out_name: *mut CFStringRef) -> OSStatus;
    pub fn AudioComponentInstanceNew(component: AudioComponent, out_instance: *mut AudioUnit) -> OSStatus;
    pub fn AudioComponentInstanceDispose(instance: AudioUnit) -> OSStatus;
    pub fn AudioUnitInitialize(unit: AudioUnit) -> OSStatus;
    pub fn AudioUnitUninitialize(unit: AudioUnit) -> OSStatus;
    pub fn AudioUnitReset(unit: AudioUnit, scope: u32, element: u32) -> OSStatus;
    pub fn AudioUnitSetProperty(unit: AudioUnit, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> OSStatus;
    pub fn AudioUnitGetProperty(unit: AudioUnit, id: u32, scope: u32, element: u32, data: *mut c_void, size: *mut u32) -> OSStatus;
    pub fn AudioUnitGetPropertyInfo(unit: AudioUnit, id: u32, scope: u32, element: u32, size: *mut u32, writable: *mut u8) -> OSStatus;
    pub fn AudioUnitRender(
        unit: AudioUnit,
        io_action_flags: *mut u32,
        in_time_stamp: *const AudioTimeStamp,
        in_output_bus: u32,
        in_number_frames: u32,
        io_data: *mut AudioBufferList2,
    ) -> OSStatus;
    pub fn MusicDeviceMIDIEvent(unit: AudioUnit, status: u32, data1: u32, data2: u32, offset_sample_frame: u32) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFStringGetLength(s: CFStringRef) -> isize;
    pub fn CFStringGetCString(s: CFStringRef, buffer: *mut u8, size: isize, encoding: u32) -> u8;
    pub fn CFDataCreate(alloc: *const c_void, bytes: *const u8, length: isize) -> CFDataRef;
    pub fn CFDataGetLength(data: CFDataRef) -> isize;
    pub fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    pub fn CFPropertyListCreateData(
        alloc: *const c_void,
        plist: CFPropertyListRef,
        format: isize,
        options: usize,
        error: *mut *const c_void,
    ) -> CFDataRef;
    pub fn CFPropertyListCreateWithData(
        alloc: *const c_void,
        data: CFDataRef,
        options: usize,
        format: *mut isize,
        error: *mut *const c_void,
    ) -> CFPropertyListRef;
}

/// Converts a CFString to a Rust String (does not release it).
pub unsafe fn cfstring_to_string(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    let len = unsafe { CFStringGetLength(s) };
    let cap = (len * 4 + 1).max(1) as usize;
    let mut buf = vec![0u8; cap];
    if unsafe { CFStringGetCString(s, buf.as_mut_ptr(), cap as isize, kCFStringEncodingUTF8) } == 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}
