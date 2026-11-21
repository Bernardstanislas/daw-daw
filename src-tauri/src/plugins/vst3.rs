//! VST3 hosting (macOS, Windows, Linux): folder scanning, module loading,
//! instantiation, audio/MIDI processing and state.
//!
//! Threading rule: a plugin instance is created, edited, saved/restored and
//! destroyed on the main thread (plugin UIs live there). The audio thread only
//! calls `IAudioProcessor::process` and `IAudioProcessor::setProcessing`.

#![allow(non_snake_case, non_upper_case_globals, clippy::missing_safety_doc)]

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper, Interface};

use super::PluginInfo;
use daw_core::dsp::{Effect, Instrument, ParamValues, ProcessCtx};

/// Largest block handed to the plugin in one `process` call.
pub const MAX_FRAMES: usize = 1024;

/// Class category of audio plugins in a VST3 factory.
const AUDIO_CLASS: &str = "Audio Module Class";
const MAX_SCAN_DEPTH: u32 = 6;

fn ok(r: tresult) -> bool {
    r == kResultOk
}

// ------------------------------------------------------------------ folders

static EXTRA_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CACHE_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// User-configured plugin folders, scanned in addition to the default ones.
pub fn set_extra_dirs(dirs: Vec<PathBuf>) {
    *EXTRA_DIRS.lock() = dirs;
}

/// Where the scan results are cached (JSON). Without it nothing is cached.
pub fn set_cache_file(path: PathBuf) {
    *CACHE_FILE.lock() = Some(path);
}

/// Standard VST3 folders of this OS.
pub fn default_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        if let Some(p) = std::env::var_os("COMMONPROGRAMFILES") {
            v.push(PathBuf::from(p).join("VST3"));
        }
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(p).join("Programs").join("Common").join("VST3"));
        }
        v
    }
    #[cfg(target_os = "macos")]
    {
        let mut v = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
        if let Some(h) = dirs::home_dir() {
            v.push(h.join("Library/Audio/Plug-Ins/VST3"));
        }
        v
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let mut v = Vec::new();
        if let Some(h) = dirs::home_dir() {
            v.push(h.join(".vst3"));
        }
        v.push(PathBuf::from("/usr/lib/vst3"));
        v.push(PathBuf::from("/usr/local/lib/vst3"));
        v
    }
}

/// Folders actually scanned: the user's first (they win on duplicates), then the defaults.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = EXTRA_DIRS.lock().iter().filter(|p| !p.as_os_str().is_empty()).cloned().collect();
    v.extend(default_dirs());
    v
}

fn is_vst3_name(name: &str) -> bool {
    name.to_lowercase().ends_with(".vst3")
}

/// Recursively collects `.vst3` entries (never descending into one).
fn collect_bundles(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("._") {
            continue;
        }
        let path = e.path();
        if is_vst3_name(&name) {
            out.push(path);
        } else if depth < MAX_SCAN_DEPTH && path.is_dir() {
            collect_bundles(&path, depth + 1, out);
        }
    }
}

/// The loadable binary of a `.vst3` entry.
fn resolve_binary(bundle: &Path) -> Option<PathBuf> {
    let stem = bundle.file_stem()?.to_string_lossy().into_owned();
    #[cfg(target_os = "macos")]
    {
        let dir = bundle.join("Contents/MacOS");
        let main = dir.join(&stem);
        if main.is_file() {
            return Some(main);
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
        files.sort();
        files.into_iter().next()
    }
    #[cfg(target_os = "windows")]
    {
        if bundle.is_file() {
            return Some(bundle.to_path_buf());
        }
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64-win"
        } else if cfg!(target_arch = "x86") {
            "x86-win"
        } else {
            "x86_64-win"
        };
        let dir = bundle.join("Contents").join(arch);
        let main = dir.join(format!("{stem}.vst3"));
        if main.is_file() {
            return Some(main);
        }
        let mut files: Vec<PathBuf> =
            std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3"))).collect();
        files.sort();
        files.into_iter().next()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let arch = if cfg!(target_arch = "aarch64") {
            "aarch64-linux"
        } else if cfg!(target_arch = "x86") {
            "i386-linux"
        } else {
            "x86_64-linux"
        };
        let main = bundle.join("Contents").join(arch).join(format!("{stem}.so"));
        main.is_file().then_some(main)
    }
}

// ------------------------------------------------------------------ module loading

type GetFactory = unsafe extern "system" fn() -> *mut IPluginFactory;

#[cfg(target_os = "macos")]
mod lib_impl {
    use super::*;
    use crate::plugins::ffi::{CFRelease, kCFStringEncodingUTF8};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, buf: *const u8, len: isize, is_dir: u8) -> *const c_void;
        fn CFBundleCreate(alloc: *const c_void, url: *const c_void) -> *const c_void;
        fn CFBundleLoadExecutable(bundle: *const c_void) -> u8;
        fn CFBundleGetFunctionPointerForName(bundle: *const c_void, name: *const c_void) -> *mut c_void;
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, enc: u32) -> *const c_void;
    }

    /// A loaded bundle; `bundleExit` runs when it is dropped.
    pub struct Lib {
        bundle: *const c_void,
        exit: Option<unsafe extern "C" fn() -> bool>,
    }

    unsafe fn symbol(bundle: *const c_void, name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        let s = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), kCFStringEncodingUTF8) };
        let p = unsafe { CFBundleGetFunctionPointerForName(bundle, s) };
        unsafe { CFRelease(s) };
        p
    }

    /// CPU types of a Mach-O file (thin or universal).
    fn cpu_types(bin: &Path) -> Option<Vec<u32>> {
        use std::io::Read;
        let mut head = [0u8; 8 + 20 * 8];
        let n = std::fs::File::open(bin).ok()?.read(&mut head).ok()?;
        let magic = u32::from_be_bytes(head[0..4].try_into().ok()?);
        match if magic == 0xCFFA_EDFE { 0xFEED_FACF } else { magic } {
            0xCAFEBABE | 0xCAFEBABF => {
                let count = u32::from_be_bytes(head[4..8].try_into().ok()?) as usize;
                let size = if magic == 0xCAFEBABE { 20 } else { 32 };
                Some((0..count.min(8)).filter(|i| 8 + i * size + 4 <= n.min(head.len())).map(|i| u32::from_be_bytes(head[8 + i * size..12 + i * size].try_into().unwrap())).collect())
            }
            0xFEEDFACF => Some(vec![u32::from_le_bytes(head[4..8].try_into().ok()?)]),
            _ => None,
        }
    }

    pub fn open(bundle_path: &Path) -> Result<(Lib, GetFactory)> {
        use std::os::unix::ffi::OsStrExt;
        // A plugin built for the other architecture cannot be loaded: say so.
        let (this, name) = if cfg!(target_arch = "aarch64") { (0x0100_000Cu32, "arm64") } else { (0x0100_0007u32, "x86_64") };
        if let Some(types) = resolve_binary(bundle_path).as_deref().and_then(cpu_types) {
            if !types.contains(&this) {
                bail!("this plugin is not built for {name} (it only contains {}); this app runs as {name}", types.iter().map(|t| if *t == 0x0100_0007 { "x86_64" } else if *t == 0x0100_000C { "arm64" } else { "another architecture" }).collect::<Vec<_>>().join(" + "));
            }
        }
        let bytes = bundle_path.as_os_str().as_bytes();
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, 1);
            if url.is_null() {
                bail!("bad plugin path");
            }
            let bundle = CFBundleCreate(std::ptr::null(), url);
            CFRelease(url);
            if bundle.is_null() {
                bail!("cannot open the bundle {}", bundle_path.display());
            }
            if CFBundleLoadExecutable(bundle) == 0 {
                CFRelease(bundle);
                bail!("cannot load the plugin binary (wrong architecture or missing dependency?)");
            }
            let get = symbol(bundle, "GetPluginFactory");
            if get.is_null() {
                CFRelease(bundle);
                bail!("not a VST3 plugin (no GetPluginFactory)");
            }
            let entry = symbol(bundle, "bundleEntry");
            let exit = symbol(bundle, "bundleExit");
            let exit: Option<unsafe extern "C" fn() -> bool> = (!exit.is_null()).then(|| std::mem::transmute(exit));
            if !entry.is_null() {
                let entry: unsafe extern "C" fn(*const c_void) -> bool = std::mem::transmute(entry);
                if !entry(bundle) {
                    CFRelease(bundle);
                    bail!("bundleEntry failed");
                }
            }
            Ok((Lib { bundle, exit }, std::mem::transmute::<*mut c_void, GetFactory>(get)))
        }
    }

    impl Drop for Lib {
        fn drop(&mut self) {
            unsafe {
                if let Some(exit) = self.exit {
                    exit();
                }
                CFRelease(self.bundle);
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod lib_impl {
    use super::*;

    /// A loaded DLL; `ExitDll` runs when it is dropped.
    pub struct Lib {
        lib: Option<libloading::Library>,
        exit: Option<unsafe extern "system" fn() -> bool>,
    }

    pub fn open(bundle_path: &Path) -> Result<(Lib, GetFactory)> {
        let bin = resolve_binary(bundle_path).ok_or_else(|| anyhow!("no VST3 binary for this architecture in {}", bundle_path.display()))?;
        unsafe {
            let lib = libloading::Library::new(&bin).map_err(|e| anyhow!("cannot load {}: {e}", bin.display()))?;
            let get: GetFactory = *lib.get::<GetFactory>(b"GetPluginFactory\0").map_err(|_| anyhow!("not a VST3 plugin (no GetPluginFactory)"))?;
            let init: Option<unsafe extern "system" fn() -> bool> = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0").ok().map(|s| *s);
            let exit: Option<unsafe extern "system" fn() -> bool> = lib.get::<unsafe extern "system" fn() -> bool>(b"ExitDll\0").ok().map(|s| *s);
            if let Some(init) = init {
                if !init() {
                    bail!("InitDll failed");
                }
            }
            Ok((Lib { lib: Some(lib), exit }, get))
        }
    }

    impl Drop for Lib {
        fn drop(&mut self) {
            unsafe {
                if let Some(exit) = self.exit {
                    exit();
                }
            }
            self.lib.take();
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod lib_impl {
    use super::*;

    /// A loaded shared object; `ModuleExit` runs when it is dropped.
    pub struct Lib {
        handle: *mut c_void,
        exit: Option<unsafe extern "C" fn() -> bool>,
    }

    pub fn open(bundle_path: &Path) -> Result<(Lib, GetFactory)> {
        let bin = resolve_binary(bundle_path).ok_or_else(|| anyhow!("no VST3 binary for this architecture in {}", bundle_path.display()))?;
        unsafe {
            let lib = libloading::os::unix::Library::new(&bin).map_err(|e| anyhow!("cannot load {}: {e}", bin.display()))?;
            let get: GetFactory = *lib.get::<GetFactory>(b"GetPluginFactory\0").map_err(|_| anyhow!("not a VST3 plugin (no GetPluginFactory)"))?;
            let entry: Option<unsafe extern "C" fn(*mut c_void) -> bool> = lib.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0").ok().map(|s| *s);
            let exit: Option<unsafe extern "C" fn() -> bool> = lib.get::<unsafe extern "C" fn() -> bool>(b"ModuleExit\0").ok().map(|s| *s);
            let handle = lib.into_raw();
            if let Some(entry) = entry {
                if !entry(handle) {
                    drop(libloading::os::unix::Library::from_raw(handle));
                    bail!("ModuleEntry failed");
                }
            }
            Ok((Lib { handle, exit }, get))
        }
    }

    impl Drop for Lib {
        fn drop(&mut self) {
            unsafe {
                if let Some(exit) = self.exit {
                    exit();
                }
                drop(libloading::os::unix::Library::from_raw(self.handle));
            }
        }
    }
}

/// A loaded plugin binary and its factory.
pub struct Module {
    // Declared first: the factory is released before the library is unloaded.
    factory: ComPtr<IPluginFactory>,
    _lib: lib_impl::Lib,
}

// Factories are thread-safe by the VST3 contract; the library handle is only dropped.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

impl Module {
    fn load(bundle: &Path) -> Result<Module> {
        let (lib, get) = lib_impl::open(bundle)?;
        let raw = unsafe { get() };
        let factory = unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| anyhow!("the plugin returned no factory"))?;
        Ok(Module { factory, _lib: lib })
    }
}

/// Modules that have been instantiated; never unloaded (plugins keep static state).
static MODULES: Mutex<Option<HashMap<PathBuf, Arc<Module>>>> = Mutex::new(None);

/// A cached module, or a fresh one (kept in the cache only when `keep`).
fn get_module(bundle: &Path, keep: bool) -> Result<Arc<Module>> {
    let mut guard = MODULES.lock();
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(m) = map.get(bundle) {
        return Ok(m.clone());
    }
    let m = Arc::new(Module::load(bundle)?);
    if keep {
        map.insert(bundle.to_path_buf(), m.clone());
    }
    Ok(m)
}

// ------------------------------------------------------------------ class ids

/// Canonical 4×u32 form of a class id (what `vst3::uid` takes), identical on
/// every platform so projects are portable.
fn tuid_to_quad(t: &TUID) -> [u32; 4] {
    let b: Vec<u8> = t.iter().map(|c| *c as u8).collect();
    quad_from_bytes(&b, cfg!(target_os = "windows"))
}

/// Inverse of `vst3::uid`'s byte layout (`com` = Windows COM layout).
fn quad_from_bytes(b: &[u8], com: bool) -> [u32; 4] {
    let be = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    if com {
        [
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            ((b[5] as u32) << 24) | ((b[4] as u32) << 16) | ((b[7] as u32) << 8) | b[6] as u32,
            be(8),
            be(12),
        ]
    } else {
        [be(0), be(4), be(8), be(12)]
    }
}

pub fn class_id_hex(t: &TUID) -> String {
    let q = tuid_to_quad(t);
    format!("vst3:{:08X}{:08X}{:08X}{:08X}", q[0], q[1], q[2], q[3])
}

/// Parses a plugin id ("vst3:" + 32 hex digits) back into a class id.
pub fn parse_id(id: &str) -> Option<TUID> {
    let hex = id.strip_prefix("vst3:")?;
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    let p = |i: usize| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok();
    Some(vst3::uid(p(0)?, p(1)?, p(2)?, p(3)?))
}

// ------------------------------------------------------------------ scanning

fn cstr(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedClass {
    id: String,
    name: String,
    vendor: String,
    instrument: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    mtime: u64,
    size: u64,
    classes: Vec<CachedClass>,
}

#[derive(Default, Serialize, Deserialize)]
struct DiskCache {
    entries: HashMap<String, CacheEntry>,
}

fn file_stamp(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((t, m.len()))
}

/// Reads the audio classes of a loaded module.
fn read_classes(m: &Module) -> Vec<CachedClass> {
    let mut out = Vec::new();
    unsafe {
        let mut finfo: PFactoryInfo = std::mem::zeroed();
        let factory_vendor = if ok(m.factory.getFactoryInfo(&mut finfo)) { cstr(&finfo.vendor) } else { String::new() };
        let f2 = m.factory.cast::<IPluginFactory2>();
        for i in 0..m.factory.countClasses() {
            let (cid, category, name, vendor, sub) = if let Some(f2) = &f2 {
                let mut info: PClassInfo2 = std::mem::zeroed();
                if !ok(f2.getClassInfo2(i, &mut info)) {
                    continue;
                }
                let v = cstr(&info.vendor);
                (info.cid, cstr(&info.category), cstr(&info.name), if v.is_empty() { factory_vendor.clone() } else { v }, cstr(&info.subCategories))
            } else {
                let mut info: PClassInfo = std::mem::zeroed();
                if !ok(m.factory.getClassInfo(i, &mut info)) {
                    continue;
                }
                (info.cid, cstr(&info.category), cstr(&info.name), factory_vendor.clone(), String::new())
            };
            if category != AUDIO_CLASS {
                continue;
            }
            out.push(CachedClass {
                id: class_id_hex(&cid),
                name,
                vendor,
                instrument: sub.split('|').any(|s| s.eq_ignore_ascii_case("Instrument")),
            });
        }
    }
    out
}

/// Lists the audio classes of the plugins found in the search folders. A plugin
/// is loaded only when it is new or changed (disk cache); reusing a module that
/// is already loaded for an instance costs nothing.
pub fn scan() -> Vec<PluginInfo> {
    scan_dirs(&search_dirs())
}

pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<PluginInfo> {
    let cache_file = CACHE_FILE.lock().clone();
    let old: DiskCache = cache_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut fresh = DiskCache::default();
    let mut bundles = Vec::new();
    for d in dirs {
        collect_bundles(d, 0, &mut bundles);
    }
    let mut out: Vec<PluginInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for bundle in bundles {
        let Some(bin) = resolve_binary(&bundle) else { continue };
        let key = bin.to_string_lossy().into_owned();
        let stamp = file_stamp(&bin);
        let cached = stamp.and_then(|(t, s)| old.entries.get(&key).filter(|e| e.mtime == t && e.size == s).cloned());
        let classes = match cached {
            Some(e) => e.classes,
            None => match get_module(&bundle, false) {
                Ok(m) => {
                    let c = read_classes(&m);
                    drop(m);
                    c
                }
                Err(e) => {
                    eprintln!("[vst3] {}: {e}", bundle.display());
                    continue;
                }
            },
        };
        if let Some((mtime, size)) = stamp {
            fresh.entries.insert(key, CacheEntry { mtime, size, classes: classes.clone() });
        }
        for c in classes {
            if seen.insert(c.id.clone()) {
                out.push(PluginInfo {
                    id: c.id,
                    name: c.name,
                    vendor: c.vendor,
                    kind: if c.instrument { "instrument" } else { "effect" },
                    format: "VST3",
                    path: Some(bundle.clone()),
                });
            }
        }
    }
    if let Some(p) = cache_file {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::to_string(&fresh).unwrap_or_default());
    }
    out.sort_by(|a, b| (a.kind, a.vendor.to_lowercase(), a.name.to_lowercase()).cmp(&(b.kind, b.vendor.to_lowercase(), b.name.to_lowercase())));
    out
}

// ------------------------------------------------------------------ host objects

fn copy_wstring(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    let cap = dst.len().saturating_sub(1);
    for (s, d) in src.encode_utf16().zip(dst.iter_mut().take(cap)) {
        *d = s as TChar;
        n += 1;
    }
    if n < dst.len() {
        dst[n] = 0;
    }
}

unsafe fn wstr_len(p: *const TChar) -> usize {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// `IHostApplication`: names the host and creates messages / attribute lists.
struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        copy_wstring("daw-daw", unsafe { &mut *name });
        kResultOk
    }

    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }
        let want: [u8; 16] = unsafe { (*cid).map(|c| c as u8) };
        let instance: Option<ComPtr<FUnknown>> = if want == IMessage::IID {
            ComWrapper::new(HostMessage::default()).to_com_ptr::<FUnknown>()
        } else if want == IAttributeList::IID {
            ComWrapper::new(HostAttributes::default()).to_com_ptr::<FUnknown>()
        } else {
            None
        };
        match instance {
            Some(p) => unsafe {
                let raw = p.as_ptr();
                ((*(*raw).vtbl).queryInterface)(raw, iid as *const TUID, obj)
            },
            None => kNoInterface,
        }
    }
}

enum Attr {
    Int(i64),
    Float(f64),
    Str(Vec<TChar>),
    Bin(Vec<u8>),
}

/// `IAttributeList`: a mutex-protected map (never touched by the audio thread).
#[derive(Default)]
struct HostAttributes {
    map: Mutex<HashMap<String, Attr>>,
}

impl Class for HostAttributes {
    type Interfaces = (IAttributeList,);
}

fn attr_key(id: *const c_char) -> Option<String> {
    (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned())
}

impl IAttributeListTrait for HostAttributes {
    unsafe fn setInt(&self, id: *const c_char, value: i64) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        self.map.lock().insert(k, Attr::Int(value));
        kResultOk
    }
    unsafe fn getInt(&self, id: *const c_char, value: *mut i64) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        match self.map.lock().get(&k) {
            Some(Attr::Int(v)) if !value.is_null() => {
                unsafe { *value = *v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }
    unsafe fn setFloat(&self, id: *const c_char, value: f64) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        self.map.lock().insert(k, Attr::Float(value));
        kResultOk
    }
    unsafe fn getFloat(&self, id: *const c_char, value: *mut f64) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        match self.map.lock().get(&k) {
            Some(Attr::Float(v)) if !value.is_null() => {
                unsafe { *value = *v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }
    unsafe fn setString(&self, id: *const c_char, string: *const TChar) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        if string.is_null() {
            return kInvalidArgument;
        }
        let n = unsafe { wstr_len(string) };
        let mut v = unsafe { std::slice::from_raw_parts(string, n) }.to_vec();
        v.push(0);
        self.map.lock().insert(k, Attr::Str(v));
        kResultOk
    }
    unsafe fn getString(&self, id: *const c_char, string: *mut TChar, size_in_bytes: u32) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        match self.map.lock().get(&k) {
            Some(Attr::Str(v)) if !string.is_null() => {
                let cap = (size_in_bytes as usize) / std::mem::size_of::<TChar>();
                if cap == 0 {
                    return kResultFalse;
                }
                let n = v.len().min(cap);
                unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), string, n) };
                unsafe { *string.add(n - 1) = 0 };
                kResultOk
            }
            _ => kResultFalse,
        }
    }
    unsafe fn setBinary(&self, id: *const c_char, data: *const c_void, size_in_bytes: u32) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        let bytes = if data.is_null() || size_in_bytes == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize) }.to_vec() };
        self.map.lock().insert(k, Attr::Bin(bytes));
        kResultOk
    }
    unsafe fn getBinary(&self, id: *const c_char, data: *mut *const c_void, size_in_bytes: *mut u32) -> tresult {
        let Some(k) = attr_key(id) else { return kInvalidArgument };
        match self.map.lock().get(&k) {
            // The pointer stays valid while the attribute is not replaced (SDK behaviour).
            Some(Attr::Bin(v)) if !data.is_null() && !size_in_bytes.is_null() => {
                unsafe {
                    *data = v.as_ptr() as *const c_void;
                    *size_in_bytes = v.len() as u32;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

/// `IMessage`: an id and its attributes (component <-> controller messages).
struct HostMessage {
    id: Mutex<CString>,
    attrs: ComWrapper<HostAttributes>,
}

impl Default for HostMessage {
    fn default() -> Self {
        Self { id: Mutex::new(CString::default()), attrs: ComWrapper::new(HostAttributes::default()) }
    }
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.lock().as_ptr()
    }
    unsafe fn setMessageID(&self, id: FIDString) {
        if !id.is_null() {
            *self.id.lock() = unsafe { CStr::from_ptr(id) }.to_owned();
        }
    }
    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Owned by the message (no add-ref), as in the SDK.
        self.attrs.as_com_ref::<IAttributeList>().map(|r| r.as_ptr()).unwrap_or(std::ptr::null_mut())
    }
}

/// `IBStream` over a byte vector.
#[derive(Default)]
struct MemStream {
    inner: Mutex<(Vec<u8>, usize)>,
}

impl Class for MemStream {
    type Interfaces = (IBStream,);
}

impl MemStream {
    fn with(data: Vec<u8>) -> ComWrapper<MemStream> {
        ComWrapper::new(MemStream { inner: Mutex::new((data, 0)) })
    }
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.inner.lock().0)
    }
    fn rewind(&self) {
        self.inner.lock().1 = 0;
    }
}

fn stream_ptr(w: &ComWrapper<MemStream>) -> *mut IBStream {
    w.as_com_ref::<IBStream>().map(|r| r.as_ptr()).unwrap_or(std::ptr::null_mut())
}

impl IBStreamTrait for MemStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: i32, num_read: *mut i32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let mut g = self.inner.lock();
        let (data, pos) = &mut *g;
        let n = (num_bytes as usize).min(data.len().saturating_sub(*pos));
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().add(*pos), buffer as *mut u8, n) };
        *pos += n;
        if !num_read.is_null() {
            unsafe { *num_read = n as i32 };
        }
        kResultOk
    }
    unsafe fn write(&self, buffer: *mut c_void, num_bytes: i32, num_written: *mut i32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let mut g = self.inner.lock();
        let (data, pos) = &mut *g;
        let n = num_bytes as usize;
        if data.len() < *pos + n {
            data.resize(*pos + n, 0);
        }
        unsafe { std::ptr::copy_nonoverlapping(buffer as *const u8, data.as_mut_ptr().add(*pos), n) };
        *pos += n;
        if !num_written.is_null() {
            unsafe { *num_written = n as i32 };
        }
        kResultOk
    }
    unsafe fn seek(&self, pos: i64, mode: i32, result: *mut i64) -> tresult {
        let mut g = self.inner.lock();
        let (data, cur) = &mut *g;
        let base = match mode {
            m if m == IBStream_::IStreamSeekMode_::kIBSeekSet as i32 => 0,
            m if m == IBStream_::IStreamSeekMode_::kIBSeekCur as i32 => *cur as i64,
            m if m == IBStream_::IStreamSeekMode_::kIBSeekEnd as i32 => data.len() as i64,
            _ => return kInvalidArgument,
        };
        let target = base + pos;
        if target < 0 {
            return kResultFalse;
        }
        *cur = target as usize;
        if !result.is_null() {
            unsafe { *result = target };
        }
        kResultOk
    }
    unsafe fn tell(&self, pos: *mut i64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        unsafe { *pos = self.inner.lock().1 as i64 };
        kResultOk
    }
}

/// `IComponentHandler`: parameter edits made in the plugin's own UI are queued
/// for the audio thread (the processor must hear about them).
struct Handler {
    edits: Mutex<rtrb::Producer<(u32, f64)>>,
}

impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        let _ = self.edits.lock().push((id, value));
        kResultOk
    }
    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _flags: i32) -> tresult {
        kResultOk
    }
}

/// `IEventList` over a preallocated vector. Audio thread only.
struct EventList {
    events: UnsafeCell<Vec<Event>>,
    cap: usize,
}

// Only touched by the audio thread (and by the plugin during `process`, on that thread).
unsafe impl Send for EventList {}
unsafe impl Sync for EventList {}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl EventList {
    fn new(cap: usize) -> ComWrapper<EventList> {
        ComWrapper::new(EventList { events: UnsafeCell::new(Vec::with_capacity(cap)), cap })
    }
    #[allow(clippy::mut_from_ref)]
    fn vec(&self) -> &mut Vec<Event> {
        unsafe { &mut *self.events.get() }
    }
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> i32 {
        self.vec().len() as i32
    }
    unsafe fn getEvent(&self, index: i32, e: *mut Event) -> tresult {
        match self.vec().get(index as usize) {
            Some(ev) if !e.is_null() => {
                unsafe { *e = *ev };
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        let v = self.vec();
        if e.is_null() || v.len() >= self.cap {
            return kResultFalse;
        }
        v.push(unsafe { *e });
        kResultOk
    }
}

/// A one-point `IParamValueQueue` (sample offset 0, last value wins).
struct ParamQueue {
    id: AtomicU32,
    value: AtomicU64,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.load(Ordering::Relaxed)
    }
    unsafe fn getPointCount(&self) -> i32 {
        1
    }
    unsafe fn getPoint(&self, index: i32, sample_offset: *mut i32, value: *mut ParamValue) -> tresult {
        if index != 0 || sample_offset.is_null() || value.is_null() {
            return kResultFalse;
        }
        unsafe {
            *sample_offset = 0;
            *value = f64::from_bits(self.value.load(Ordering::Relaxed));
        }
        kResultOk
    }
    unsafe fn addPoint(&self, _sample_offset: i32, value: ParamValue, index: *mut i32) -> tresult {
        self.value.store(value.to_bits(), Ordering::Relaxed);
        if !index.is_null() {
            unsafe { *index = 0 };
        }
        kResultOk
    }
}

const PARAM_POOL: usize = 128;

/// `IParameterChanges` backed by a fixed pool of queues (no allocation on the audio thread).
struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    count: AtomicUsize,
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl ParamChanges {
    fn new() -> ComWrapper<ParamChanges> {
        let queues = (0..PARAM_POOL).map(|_| ComWrapper::new(ParamQueue { id: AtomicU32::new(0), value: AtomicU64::new(0) })).collect();
        ComWrapper::new(ParamChanges { queues, count: AtomicUsize::new(0) })
    }
    fn clear(&self) {
        self.count.store(0, Ordering::Relaxed);
    }
    /// Queue index of `id`, claiming a free one if needed.
    fn slot(&self, id: u32) -> Option<usize> {
        let n = self.count.load(Ordering::Relaxed);
        for i in 0..n {
            if self.queues[i].id.load(Ordering::Relaxed) == id {
                return Some(i);
            }
        }
        if n >= PARAM_POOL {
            return None;
        }
        self.queues[n].id.store(id, Ordering::Relaxed);
        self.count.store(n + 1, Ordering::Relaxed);
        Some(n)
    }
    fn set(&self, id: u32, value: f64) {
        if let Some(i) = self.slot(id) {
            self.queues[i].value.store(value.to_bits(), Ordering::Relaxed);
        }
    }
    fn queue_ptr(&self, i: usize) -> *mut IParamValueQueue {
        self.queues[i].as_com_ref::<IParamValueQueue>().map(|r| r.as_ptr()).unwrap_or(std::ptr::null_mut())
    }
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> i32 {
        self.count.load(Ordering::Relaxed) as i32
    }
    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.count.load(Ordering::Relaxed) {
            return std::ptr::null_mut();
        }
        self.queue_ptr(index as usize)
    }
    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut i32) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        match self.slot(unsafe { *id }) {
            Some(i) => {
                if !index.is_null() {
                    unsafe { *index = i as i32 };
                }
                self.queue_ptr(i)
            }
            None => std::ptr::null_mut(),
        }
    }
}

// ------------------------------------------------------------------ the plugin instance

/// The plugin's COM objects. Main-thread rule: only `process` and `setProcessing`
/// may be called from the audio thread, everything else on the main thread.
struct Plugin {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is a separate object that we initialized and must terminate.
    separate: bool,
    comp_cp: Option<ComPtr<IConnectionPoint>>,
    ctrl_cp: Option<ComPtr<IConnectionPoint>>,
    initialized: bool,
    connected: bool,
    active: bool,
    processing: bool,
}

impl Drop for Plugin {
    /// Main thread. Tear-down order: stop, deactivate, disconnect, terminate.
    fn drop(&mut self) {
        unsafe {
            if self.processing {
                self.processor.setProcessing(0);
            }
            if self.active {
                self.component.setActive(0);
            }
            if self.connected {
                if let (Some(a), Some(b)) = (&self.comp_cp, &self.ctrl_cp) {
                    a.disconnect(b.as_ptr());
                    b.disconnect(a.as_ptr());
                }
            }
            if self.separate {
                if let Some(c) = &self.controller {
                    c.terminate();
                }
            }
            if self.initialized {
                self.component.terminate();
            }
        }
    }
}

/// Everything torn down together (and in this order) when the unit goes away.
struct Inner {
    plugin: Plugin,
    _handler: ComWrapper<Handler>,
    _host: ComWrapper<HostApp>,
    _module: Arc<Module>,
}

// See the threading rule on `Plugin`.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

/// A live VST3 instance. Shared (Arc) between the engine and the plugin
/// manager (editor window, state); torn down on the main thread when the last
/// reference drops.
pub struct Vst3Unit {
    pub info: PluginInfo,
    inner: Option<Inner>,
    is_effect: bool,
    /// Channel count of every audio input / output bus.
    ins: Vec<i32>,
    outs: Vec<i32>,
    sample_rate: f32,
    offline: bool,
    edits: Mutex<Option<rtrb::Consumer<(u32, f64)>>>,
}

unsafe impl Send for Vst3Unit {}
unsafe impl Sync for Vst3Unit {}

unsafe fn create_instance<I: Interface>(factory: &ComPtr<IPluginFactory>, cid: &TUID) -> Result<ComPtr<I>> {
    let mut obj: *mut c_void = std::ptr::null_mut();
    let r = unsafe { factory.createInstance(cid.as_ptr(), I::IID.as_ptr() as FIDString, &mut obj) };
    if !ok(r) || obj.is_null() {
        bail!("createInstance failed (result {r})");
    }
    unsafe { ComPtr::from_raw(obj as *mut I) }.ok_or_else(|| anyhow!("createInstance returned null"))
}

fn check(r: tresult, what: &str) -> Result<()> {
    if ok(r) { Ok(()) } else { bail!("{what} failed (result {r})") }
}

unsafe fn bus_channels(c: &ComPtr<IComponent>, dir: i32, n: i32) -> Vec<i32> {
    (0..n)
        .map(|i| {
            let mut info: BusInfo = unsafe { std::mem::zeroed() };
            if ok(unsafe { c.getBusInfo(MediaTypes_::kAudio as i32, dir, i, &mut info) }) { info.channelCount.max(0) } else { 0 }
        })
        .collect()
}

impl Vst3Unit {
    /// Main thread.
    pub fn new(info: PluginInfo, sample_rate: f32, offline: bool) -> Result<Arc<Self>> {
        let path = info.path.clone().ok_or_else(|| anyhow!("{} has no file path", info.name))?;
        let cid = parse_id(&info.id).ok_or_else(|| anyhow!("bad plugin id {}", info.id))?;
        let module = get_module(&path, true)?;
        let host = ComWrapper::new(HostApp);
        let host_ptr = host.as_com_ref::<IHostApplication>().unwrap().as_ptr() as *mut FUnknown;
        let (producer, consumer) = rtrb::RingBuffer::new(1024);
        let handler = ComWrapper::new(Handler { edits: Mutex::new(producer) });
        let handler_ptr = handler.as_com_ref::<IComponentHandler>().unwrap().as_ptr();

        unsafe {
            let component: ComPtr<IComponent> = create_instance(&module.factory, &cid)?;
            let processor = component.cast::<IAudioProcessor>().ok_or_else(|| anyhow!("the plugin has no audio processor"))?;
            let mut p = Plugin {
                component,
                processor,
                controller: None,
                separate: false,
                comp_cp: None,
                ctrl_cp: None,
                initialized: false,
                connected: false,
                active: false,
                processing: false,
            };
            check(p.component.initialize(host_ptr), "initialize")?;
            p.initialized = true;

            // Controller: the component itself, or a separate class.
            if let Some(c) = p.component.cast::<IEditController>() {
                p.controller = Some(c);
            } else {
                let mut ccid: TUID = [0; 16];
                if ok(p.component.getControllerClassId(&mut ccid)) {
                    if let Ok(c) = create_instance::<IEditController>(&module.factory, &ccid) {
                        check(c.initialize(host_ptr), "controller initialize")?;
                        p.controller = Some(c);
                        p.separate = true;
                    }
                }
            }
            if let Some(ctrl) = &p.controller {
                ctrl.setComponentHandler(handler_ptr);
                if p.separate {
                    p.comp_cp = p.component.cast::<IConnectionPoint>();
                    p.ctrl_cp = ctrl.cast::<IConnectionPoint>();
                    if let (Some(a), Some(b)) = (&p.comp_cp, &p.ctrl_cp) {
                        a.connect(b.as_ptr());
                        b.connect(a.as_ptr());
                        p.connected = true;
                    }
                }
                // The controller learns the component's current state.
                let s = ComWrapper::new(MemStream::default());
                if ok(p.component.getState(stream_ptr(&s))) {
                    s.rewind();
                    ctrl.setComponentState(stream_ptr(&s));
                }
            }

            // Buses: stereo main in/out when the plugin accepts it, then read what it actually uses.
            let kAudio = MediaTypes_::kAudio as i32;
            let kIn = BusDirections_::kInput as i32;
            let kOut = BusDirections_::kOutput as i32;
            let in_n = p.component.getBusCount(kAudio, kIn);
            let out_n = p.component.getBusCount(kAudio, kOut);
            let arrangement = |dir: i32, n: i32| -> Vec<SpeakerArrangement> {
                (0..n)
                    .map(|i| {
                        let mut a: SpeakerArrangement = 0;
                        if i == 0 {
                            SpeakerArr::kStereo
                        } else if ok(p.processor.getBusArrangement(dir, i, &mut a)) {
                            a
                        } else {
                            0
                        }
                    })
                    .collect()
            };
            let mut ins = arrangement(kIn, in_n);
            let mut outs = arrangement(kOut, out_n);
            let _ = p.processor.setBusArrangements(ins.as_mut_ptr(), in_n, outs.as_mut_ptr(), out_n);
            let ins = bus_channels(&p.component, kIn, in_n);
            let outs = bus_channels(&p.component, kOut, out_n);
            if in_n > 0 {
                p.component.activateBus(kAudio, kIn, 0, 1);
            }
            if out_n > 0 {
                p.component.activateBus(kAudio, kOut, 0, 1);
            }
            if p.component.getBusCount(MediaTypes_::kEvent as i32, kIn) > 0 {
                p.component.activateBus(MediaTypes_::kEvent as i32, kIn, 0, 1);
            }

            let mut setup = ProcessSetup {
                processMode: (if offline { ProcessModes_::kOffline } else { ProcessModes_::kRealtime }) as i32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                maxSamplesPerBlock: MAX_FRAMES as i32,
                sampleRate: sample_rate as f64,
            };
            check(p.processor.setupProcessing(&mut setup), "setupProcessing")?;
            check(p.component.setActive(1), "setActive")?;
            p.active = true;
            // Some plugins answer kNotImplemented here and process anyway.
            p.processing = ok(p.processor.setProcessing(1));

            let is_effect = info.kind == "effect";
            Ok(Arc::new(Vst3Unit {
                info,
                inner: Some(Inner { plugin: p, _handler: handler, _host: host, _module: module }),
                is_effect,
                ins,
                outs,
                sample_rate,
                offline,
                edits: Mutex::new(Some(consumer)),
            }))
        }
    }

    fn plugin(&self) -> &Plugin {
        &self.inner.as_ref().expect("unit alive").plugin
    }

    /// Saved state: u32 LE length of the component state, component bytes, controller bytes. Main thread.
    pub fn get_state(&self) -> Option<Vec<u8>> {
        let p = self.plugin();
        unsafe {
            let cs = ComWrapper::new(MemStream::default());
            if !ok(p.component.getState(stream_ptr(&cs))) {
                return None;
            }
            let comp = cs.take();
            let ctrl = match &p.controller {
                Some(c) => {
                    let s = ComWrapper::new(MemStream::default());
                    if ok(c.getState(stream_ptr(&s))) { s.take() } else { Vec::new() }
                }
                None => Vec::new(),
            };
            let mut out = Vec::with_capacity(4 + comp.len() + ctrl.len());
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&comp);
            out.extend_from_slice(&ctrl);
            Some(out)
        }
    }

    /// Main thread.
    pub fn set_state(&self, bytes: &[u8]) -> Result<()> {
        if bytes.len() < 4 {
            bail!("invalid plugin state");
        }
        let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if bytes.len() < 4 + n {
            bail!("invalid plugin state");
        }
        let (comp, ctrl) = (&bytes[4..4 + n], &bytes[4 + n..]);
        let p = self.plugin();
        unsafe {
            let s = MemStream::with(comp.to_vec());
            check(p.component.setState(stream_ptr(&s)), "restore state")?;
            if let Some(c) = &p.controller {
                s.rewind();
                c.setComponentState(stream_ptr(&s));
                if !ctrl.is_empty() {
                    let cs = MemStream::with(ctrl.to_vec());
                    c.setState(stream_ptr(&cs));
                }
            }
        }
        Ok(())
    }

    /// The plugin's editor view (main thread).
    pub fn create_view(&self) -> Result<ComPtr<IPlugView>> {
        let ctrl = self.plugin().controller.as_ref().ok_or_else(|| anyhow!("this plugin has no editor window"))?;
        let raw = unsafe { ctrl.createView(b"editor\0".as_ptr() as FIDString) };
        unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| anyhow!("this plugin has no editor window"))
    }
}

impl Drop for Vst3Unit {
    fn drop(&mut self) {
        // Often dropped by the engine's clean-up thread: tear down on the main thread.
        if let Some(inner) = self.inner.take() {
            super::on_main_async(move || drop(inner));
        }
    }
}

// ------------------------------------------------------------------ audio-thread device

struct BusBuf {
    _chans: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

impl BusBuf {
    fn new(channels: i32) -> Self {
        let mut chans: Vec<Vec<f32>> = (0..channels.max(0)).map(|_| vec![0.0; MAX_FRAMES]).collect();
        let ptrs = chans.iter_mut().map(|c| c.as_mut_ptr()).collect();
        Self { _chans: chans, ptrs }
    }
    fn chan(&mut self, i: usize) -> Option<&mut [f32]> {
        self.ptrs.get(i).map(|p| unsafe { std::slice::from_raw_parts_mut(*p, MAX_FRAMES) })
    }
}

const MAX_PENDING: usize = 512;
const EVENT_CAP: usize = 1024;

/// Engine-side device wrapping a Vst3Unit (implements Instrument and Effect).
pub struct Vst3Device {
    unit: Arc<Vst3Unit>,
    edits: rtrb::Consumer<(u32, f64)>,
    in_bufs: Vec<BusBuf>,
    out_bufs: Vec<BusBuf>,
    in_buses: Vec<AudioBusBuffers>,
    out_buses: Vec<AudioBusBuffers>,
    events: ComWrapper<EventList>,
    out_events: ComWrapper<EventList>,
    params: ComWrapper<ParamChanges>,
    out_params: ComWrapper<ParamChanges>,
    context: ProcessContext,
    sample_pos: i64,
    /// Note events waiting for the next `process` (fixed capacity, never grows).
    pending: Vec<Event>,
    held: [bool; 128],
}

// The raw pointers refer to heap buffers owned by this struct; used by the audio thread only.
unsafe impl Send for Vst3Device {}

fn make_buses(bufs: &mut [BusBuf], channels: &[i32]) -> Vec<AudioBusBuffers> {
    bufs.iter_mut()
        .zip(channels)
        .map(|(b, c)| AudioBusBuffers { numChannels: *c, silenceFlags: 0, __field0: AudioBusBuffers__type0 { channelBuffers32: b.ptrs.as_mut_ptr() } })
        .collect()
}

impl Vst3Device {
    pub fn new(unit: Arc<Vst3Unit>) -> Self {
        let edits = unit.edits.lock().take().unwrap_or_else(|| rtrb::RingBuffer::new(1).1);
        let mut in_bufs: Vec<BusBuf> = unit.ins.iter().map(|c| BusBuf::new(*c)).collect();
        let mut out_bufs: Vec<BusBuf> = unit.outs.iter().map(|c| BusBuf::new(*c)).collect();
        let in_buses = make_buses(&mut in_bufs, &unit.ins);
        let out_buses = make_buses(&mut out_bufs, &unit.outs);
        let mut context: ProcessContext = unsafe { std::mem::zeroed() };
        context.sampleRate = unit.sample_rate as f64;
        context.tempo = 120.0;
        context.timeSigNumerator = 4;
        context.timeSigDenominator = 4;
        Self {
            unit,
            edits,
            in_bufs,
            out_bufs,
            in_buses,
            out_buses,
            events: EventList::new(EVENT_CAP),
            out_events: EventList::new(EVENT_CAP),
            params: ParamChanges::new(),
            out_params: ParamChanges::new(),
            context,
            sample_pos: 0,
            pending: Vec::with_capacity(MAX_PENDING),
            held: [false; 128],
        }
    }

    fn push_note(&mut self, pitch: u8, velocity: f32, on: bool) {
        if self.pending.len() >= MAX_PENDING {
            return; // full: drop rather than allocate
        }
        let mut e: Event = unsafe { std::mem::zeroed() };
        e.busIndex = 0;
        e.sampleOffset = 0;
        if on {
            e.r#type = Event_::EventTypes_::kNoteOnEvent as u16;
            e.__field0 = Event__type0 { noteOn: NoteOnEvent { channel: 0, pitch: pitch as i16, tuning: 0.0, velocity, length: 0, noteId: -1 } };
        } else {
            e.r#type = Event_::EventTypes_::kNoteOffEvent as u16;
            e.__field0 = Event__type0 { noteOff: NoteOffEvent { channel: 0, pitch: pitch as i16, velocity: 0.0, noteId: -1, tuning: 0.0 } };
        }
        self.pending.push(e);
    }

    /// Processes `l.len()` frames. Instruments add into l/r, effects replace them.
    fn render(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32], add: bool) {
        let is_effect = self.unit.is_effect;
        let proc_mode = (if self.unit.offline { ProcessModes_::kOffline } else { ProcessModes_::kRealtime }) as i32;
        let total = l.len().min(r.len());
        let bpm = if ctx.bpm > 0.0 { ctx.bpm as f64 } else { 120.0 };
        let mut beat = ctx.beat;
        let mut done = 0;
        let mut first = true;
        let in_ev = self.events.as_com_ref::<IEventList>().unwrap().as_ptr();
        let out_ev = self.out_events.as_com_ref::<IEventList>().unwrap().as_ptr();
        let in_pc = self.params.as_com_ref::<IParameterChanges>().unwrap().as_ptr();
        let out_pc = self.out_params.as_com_ref::<IParameterChanges>().unwrap().as_ptr();

        // Parameter edits made in the plugin's UI, and the events queued since the last block.
        self.params.clear();
        while let Ok((id, v)) = self.edits.pop() {
            self.params.set(id, v);
        }
        {
            let list = self.events.vec();
            list.clear();
            list.extend_from_slice(&self.pending);
            self.pending.clear();
        }

        while done < total {
            let n = (total - done).min(MAX_FRAMES);
            if !first {
                self.events.vec().clear();
                self.params.clear();
            }
            first = false;
            self.out_events.vec().clear();
            self.out_params.clear();

            if is_effect {
                if let Some(bus) = self.in_bufs.first_mut() {
                    let mono = bus.ptrs.len() == 1;
                    if mono {
                        if let Some(c0) = bus.chan(0) {
                            for i in 0..n {
                                c0[i] = 0.5 * (l[done + i] + r[done + i]);
                            }
                        }
                    } else {
                        if let Some(c0) = bus.chan(0) {
                            c0[..n].copy_from_slice(&l[done..done + n]);
                        }
                        if let Some(c1) = bus.chan(1) {
                            c1[..n].copy_from_slice(&r[done..done + n]);
                        }
                    }
                }
            }
            for bus in &mut self.out_bufs {
                for c in 0..bus.ptrs.len() {
                    if let Some(ch) = bus.chan(c) {
                        ch[..n].fill(0.0);
                    }
                }
            }

            self.context.state = (ProcessContext_::StatesAndFlags_::kTempoValid | ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid | ProcessContext_::StatesAndFlags_::kTimeSigValid) as u32
                | if ctx.playing { ProcessContext_::StatesAndFlags_::kPlaying as u32 } else { 0 };
            self.context.tempo = bpm;
            self.context.projectTimeMusic = beat;
            self.context.projectTimeSamples = self.sample_pos;
            self.context.continousTimeSamples = self.sample_pos;

            let mut data = ProcessData {
                processMode: proc_mode,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                numSamples: n as i32,
                numInputs: self.in_buses.len() as i32,
                numOutputs: self.out_buses.len() as i32,
                inputs: if self.in_buses.is_empty() { std::ptr::null_mut() } else { self.in_buses.as_mut_ptr() },
                outputs: if self.out_buses.is_empty() { std::ptr::null_mut() } else { self.out_buses.as_mut_ptr() },
                inputParameterChanges: in_pc,
                outputParameterChanges: out_pc,
                inputEvents: in_ev,
                outputEvents: out_ev,
                processContext: &mut self.context,
            };
            let res = unsafe { self.unit.plugin().processor.process(&mut data) };
            if ok(res) {
                if let Some(bus) = self.out_bufs.first_mut() {
                    let channels = bus.ptrs.len();
                    if channels > 0 {
                        let (a, b) = (bus.ptrs[0] as *const f32, bus.ptrs[if channels > 1 { 1 } else { 0 }] as *const f32);
                        let (ol, or) = unsafe { (std::slice::from_raw_parts(a, n), std::slice::from_raw_parts(b, n)) };
                        for i in 0..n {
                            let (x, y) = (if ol[i].is_finite() { ol[i] } else { 0.0 }, if or[i].is_finite() { or[i] } else { 0.0 });
                            if add {
                                l[done + i] += x;
                                r[done + i] += y;
                            } else {
                                l[done + i] = x;
                                r[done + i] = y;
                            }
                        }
                    }
                }
            }
            // On a failed `process` an effect leaves its input untouched (pass-through).
            beat += n as f64 * bpm / 60.0 / self.unit.sample_rate.max(1.0) as f64;
            self.sample_pos += n as i64;
            done += n;
        }
    }

    fn restart_processing(&mut self) {
        unsafe {
            let p = &self.unit.plugin().processor;
            p.setProcessing(0);
            p.setProcessing(1);
        }
    }
}

impl Instrument for Vst3Device {
    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let pitch = pitch.min(127);
        self.held[pitch as usize] = true;
        self.push_note(pitch, velocity.clamp(0.0, 1.0).max(1.0 / 127.0), true);
    }
    fn note_off(&mut self, pitch: u8) {
        let pitch = pitch.min(127);
        self.held[pitch as usize] = false;
        self.push_note(pitch, 0.0, false);
    }
    fn all_notes_off(&mut self, hard: bool) {
        // VST3 has no "all notes off": release every note we started.
        for p in 0..128u8 {
            if self.held[p as usize] {
                self.held[p as usize] = false;
                self.push_note(p, 0.0, false);
            }
        }
        if hard {
            self.restart_processing();
        }
    }
    fn set_params(&mut self, _values: &ParamValues) {}
    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        self.render(ctx, l, r, true);
    }
}

impl Effect for Vst3Device {
    fn set_params(&mut self, _values: &ParamValues) {}
    fn process(&mut self, ctx: &ProcessCtx, l: &mut [f32], r: &mut [f32]) {
        self.render(ctx, l, r, false);
    }
    fn reset(&mut self) {
        self.pending.clear();
        self.restart_processing();
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::super::{Provider, manager};
    use super::*;
    use base64::Engine as _;
    use daw_core::dsp::registry::ExternalDevices;
    use daw_core::model::DeviceState;

    /// Plugin loads from several test threads at once are not what the app does.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn find(kind: &str, name: &str) -> Option<PluginInfo> {
        scan().into_iter().find(|p| p.kind == kind && p.name.eq_ignore_ascii_case(name))
    }

    /// An installed (and loadable) VST3 instrument: Vital first, then other synths.
    fn find_synth() -> Option<PluginInfo> {
        ["vital", "surge xt", "zebra2", "analog lab v"].iter().find_map(|n| find("instrument", n))
    }

    fn device(id: &str, p: &PluginInfo) -> DeviceState {
        DeviceState { id: id.into(), type_id: p.id.clone(), enabled: true, ..Default::default() }
    }

    fn ctx() -> ProcessCtx {
        ProcessCtx { sample_rate: 48_000.0, bpm: 120.0, playing: true, beat: 0.0 }
    }

    /// Plays a note through an instrument, returns (peak, any non-finite).
    fn play(inst: &mut dyn Instrument, blocks: usize) -> f32 {
        let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
        inst.note_on(60, 0.9);
        let mut peak = 0f32;
        for _ in 0..blocks {
            l.fill(0.0);
            r.fill(0.0);
            inst.process(&ctx(), &mut l, &mut r);
            peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
        }
        inst.note_off(60);
        peak
    }

    /// Runs a noise burst through an effect: (peak out, finite, energy).
    fn burst(fx: &mut dyn Effect) -> (f32, bool, f32) {
        let (mut peak, mut finite, mut energy) = (0f32, true, 0f32);
        let mut phase = 0f32;
        for block in 0..100 {
            let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
            if block < 20 {
                for i in 0..256 {
                    phase += 440.0 / 48_000.0;
                    l[i] = (phase * std::f32::consts::TAU).sin() * 0.5;
                    r[i] = l[i];
                }
            }
            fx.process(&ctx(), &mut l, &mut r);
            for v in l.iter().chain(&r) {
                finite &= v.is_finite();
                peak = peak.max(v.abs());
                energy += v * v;
            }
        }
        (peak, finite, energy)
    }

    #[test]
    fn id_round_trip() {
        let t = vst3::uid(0x12345678, 0x9ABCDEF0, 0x0FEDCBA9, 0x87654321);
        let id = class_id_hex(&t);
        assert_eq!(id, "vst3:123456789ABCDEF00FEDCBA987654321");
        assert_eq!(parse_id(&id).unwrap(), t);
        assert!(parse_id("vst3:xyz").is_none());
        assert!(parse_id("au:00000000").is_none());
        // The Windows (COM) byte layout maps to the same canonical id.
        let com: [u8; 16] = [0x78, 0x56, 0x34, 0x12, 0xBC, 0x9A, 0xF0, 0xDE, 0x0F, 0xED, 0xCB, 0xA9, 0x87, 0x65, 0x43, 0x21];
        assert_eq!(quad_from_bytes(&com, true), [0x12345678, 0x9ABCDEF0, 0x0FEDCBA9, 0x87654321]);
        let be: Vec<u8> = [0x12345678u32, 0x9ABCDEF0, 0x0FEDCBA9, 0x87654321].iter().flat_map(|v| v.to_be_bytes()).collect();
        assert_eq!(quad_from_bytes(&be, false), [0x12345678, 0x9ABCDEF0, 0x0FEDCBA9, 0x87654321]);
    }

    #[test]
    fn scan_finds_installed_plugins() {
        let _g = SERIAL.lock();
        let found = scan();
        let names: Vec<String> = found.iter().map(|p| format!("{} [{}] by {} ({})", p.name, p.kind, p.vendor, p.id)).collect();
        eprintln!("{} VST3 classes:\n  {}", found.len(), names.join("\n  "));
        let installed = default_dirs().iter().any(|d| d.is_dir() && std::fs::read_dir(d).map(|mut r| r.next().is_some()).unwrap_or(false));
        if !installed {
            eprintln!("no VST3 installed, skipping");
            return;
        }
        assert!(!found.is_empty(), "VST3 plugins are installed but none was found");
        assert!(found.iter().all(|p| p.id.starts_with("vst3:") && p.format == "VST3" && p.path.is_some()));
        // A second scan is served by the disk cache when one is set.
        let dir = std::env::temp_dir().join(format!("daw-vst3-cache-{}", std::process::id()));
        set_cache_file(dir.join("cache.json"));
        let a = scan().len();
        assert!(dir.join("cache.json").exists());
        let t = std::time::Instant::now();
        let b = scan().len();
        eprintln!("cached rescan: {:?}", t.elapsed());
        assert_eq!(a, b);
        *CACHE_FILE.lock() = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn extra_dir_is_scanned() {
        let _g = SERIAL.lock();
        let Some(src) = default_dirs().iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten()).map(|e| e.path()).find(|p| p.to_string_lossy().ends_with("SPAN.vst3") || p.to_string_lossy().ends_with("Surge XT Effects.vst3")) else {
            eprintln!("no suitable VST3 installed, skipping");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("daw-vst3-extra-{}", std::process::id()));
        let nested = tmp.join("some").join("deeper");
        std::fs::create_dir_all(&nested).unwrap();
        std::os::unix::fs::symlink(&src, nested.join(src.file_name().unwrap())).unwrap();
        set_extra_dirs(vec![tmp.clone()]);
        let found = scan();
        set_extra_dirs(Vec::new());
        let _ = std::fs::remove_dir_all(&tmp);
        let from_extra: Vec<_> = found.iter().filter(|p| p.path.as_ref().is_some_and(|x| x.starts_with(&tmp))).collect();
        eprintln!("found via the extra folder: {:?}", from_extra.iter().map(|p| &p.name).collect::<Vec<_>>());
        assert!(!from_extra.is_empty(), "the plugin in the extra folder was not found");
    }

    #[test]
    fn vital_plays_a_note_offline() {
        let _g = SERIAL.lock();
        let Some(vital) = find_synth() else {
            eprintln!("no loadable VST3 instrument installed (Vital's VST3 here is x86_64 only), skipping");
            return;
        };
        let mut inst = Provider.create_instrument(&device("test-vst3-vital", &vital), 48_000.0, true).expect("Vital instantiates");
        let peak = play(inst.as_mut(), 200);
        eprintln!("{} by {} ({}) peak {peak:.3}", vital.name, vital.vendor, vital.id);
        assert!(peak > 0.001, "Vital VST3 produced sound (peak {peak})");
        // Held notes are released by all_notes_off.
        inst.note_on(64, 0.8);
        inst.all_notes_off(false);
        inst.all_notes_off(true);
    }

    #[test]
    fn vital_state_round_trips() {
        let _g = SERIAL.lock();
        let Some(vital) = find_synth() else {
            eprintln!("no loadable VST3 instrument installed (Vital's VST3 here is x86_64 only), skipping");
            return;
        };
        let _first = Provider.create_instrument(&device("test-vst3-state", &vital), 48_000.0, false).expect("instantiates");
        let s1 = manager().states().get("test-vst3-state").cloned().expect("state saved");
        let bytes = base64::engine::general_purpose::STANDARD.decode(&s1).unwrap();
        assert!(bytes.len() > 100, "state is {} bytes", bytes.len());
        // A new instance restoring the saved state gives an equal-size state.
        let mut d = device("test-vst3-state2", &vital);
        d.state = Some(s1.clone());
        let _second = Provider.create_instrument(&d, 48_000.0, false).expect("instantiates with state");
        let s2 = manager().states().get("test-vst3-state2").cloned().expect("state saved");
        let b2 = base64::engine::general_purpose::STANDARD.decode(&s2).unwrap();
        eprintln!("state sizes: {} -> {}", bytes.len(), b2.len());
        assert_eq!(bytes.len(), b2.len());
        let comp_len = u32::from_le_bytes([b2[0], b2[1], b2[2], b2[3]]) as usize;
        assert!(comp_len > 100 && comp_len < b2.len());
    }

    #[test]
    fn vst3_effect_processes_audio() {
        let _g = SERIAL.lock();
        let Some(fx) = find("effect", "SPAN").or_else(|| scan().into_iter().find(|p| p.kind == "effect" && p.name.starts_with("Surge XT"))) else {
            eprintln!("no VST3 effect (SPAN, Surge XT Effects) installed, skipping");
            return;
        };
        let mut e = Provider.create_effect(&device("test-vst3-fx", &fx), 48_000.0, false).expect("effect instantiates");
        let (peak, finite, energy) = burst(e.as_mut());
        eprintln!("{} by {}: peak {peak:.3}, energy {energy:.1}", fx.name, fx.vendor);
        assert!(finite, "no NaN/inf");
        assert!(peak > 0.01, "signal passes through the effect (peak {peak})");
        e.reset();
    }

    /// Tries every other installed VST3; tolerant, prints what happened.
    #[test]
    fn every_installed_vst3_report() {
        let _g = SERIAL.lock();
        for (i, p) in scan().into_iter().enumerate() {
            let id = format!("test-vst3-all-{i}");
            let start = std::time::Instant::now();
            let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if p.kind == "instrument" {
                    let mut inst = Provider.create_instrument(&device(&id, &p), 48_000.0, true)?;
                    let mut peak = play(inst.as_mut(), 200);
                    if peak < 0.001 {
                        // Some synths (Zebra2) need a while after loading.
                        peak = play(inst.as_mut(), 3000);
                    }
                    Some(format!("instrument peak {peak:.3}"))
                } else {
                    let mut fx = Provider.create_effect(&device(&id, &p), 48_000.0, true)?;
                    let (peak, finite, _) = burst(fx.as_mut());
                    Some(format!("effect peak {peak:.3}, finite {finite}"))
                }
            }));
            let verdict = match attempt {
                Ok(Some(s)) => s,
                Ok(None) => "FAILED to instantiate (see [plugins] log line)".to_string(),
                Err(_) => "PANICKED".to_string(),
            };
            eprintln!("VST3 {} [{}] ({}): {verdict} in {:?}", p.name, p.kind, p.vendor, start.elapsed());
        }
    }
}
