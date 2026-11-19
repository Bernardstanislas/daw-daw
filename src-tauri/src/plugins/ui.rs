//! Native editor window of an Audio Unit (its Cocoa view in an NSWindow).
//! Uses the Objective-C runtime directly. MUST be called on the main thread.

#![allow(clippy::missing_transmute_annotations)]

use std::ffi::{CString, c_char, c_void};

use anyhow::{Result, anyhow, bail};

use super::ffi::*;

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSSize {
    w: f64,
    h: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

fn class(name: &str) -> Id {
    let c = CString::new(name).unwrap();
    unsafe { objc_getClass(c.as_ptr()) }
}

fn sel(name: &str) -> Sel {
    let c = CString::new(name).unwrap();
    unsafe { sel_registerName(c.as_ptr()) }
}

// Typed views of objc_msgSend for the signatures we use.
unsafe fn send0(obj: Id, s: &str) -> Id {
    let f: unsafe extern "C" fn(Id, Sel) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s)) }
}
unsafe fn send1(obj: Id, s: &str, a: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Id) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), a) }
}
unsafe fn send_bool(obj: Id, s: &str, a: bool) {
    let f: unsafe extern "C" fn(Id, Sel, bool) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), a) }
}
unsafe fn send_isize(obj: Id, s: &str, a: isize) {
    let f: unsafe extern "C" fn(Id, Sel, isize) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), a) }
}
unsafe fn frame_of(view: Id) -> NSRect {
    #[cfg(target_arch = "aarch64")]
    {
        let f: unsafe extern "C" fn(Id, Sel) -> NSRect = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f(view, sel("frame")) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        unsafe extern "C" {
            fn objc_msgSend_stret();
        }
        let f: unsafe extern "C" fn(*mut NSRect, Id, Sel) =
            unsafe { std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn()) };
        let mut r = NSRect::default();
        unsafe { f(&mut r, view, sel("frame")) };
        r
    }
}

unsafe fn send_size(obj: Id, s: &str, size: NSSize) {
    let f: unsafe extern "C" fn(Id, Sel, NSSize) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), size) }
}
unsafe fn send_point(obj: Id, s: &str, p: NSPoint) {
    let f: unsafe extern "C" fn(Id, Sel, NSPoint) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), p) }
}
unsafe fn send_rect(obj: Id, s: &str, r: NSRect) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, NSRect) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), r) }
}
unsafe fn is_visible(obj: Id) -> bool {
    let f: unsafe extern "C" fn(Id, Sel) -> bool = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel("isVisible")) }
}

/// Size used until the plugin view reports its real size.
const FALLBACK: NSSize = NSSize { w: 900.0, h: 640.0 };
/// Preferred size passed to the plugin (most plugins pick their own).
const PREFERRED: NSSize = NSSize { w: 1000.0, h: 700.0 };

fn usable(s: NSSize) -> bool {
    s.w >= 120.0 && s.h >= 80.0 && s.w < 8000.0 && s.h < 8000.0
}

unsafe fn nsstring(s: &str) -> Id {
    let alloc = unsafe { send0(class("NSString"), "alloc") };
    let f: unsafe extern "C" fn(Id, Sel, *const u8, usize, usize) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    // NSUTF8StringEncoding = 4
    unsafe { f(alloc, sel("initWithBytes:length:encoding:"), s.as_ptr(), s.len(), 4) }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorCreateGenericRGB(r: f64, g: f64, b: f64, a: f64) -> *const c_void;
    fn CGColorRelease(c: *const c_void);
}

unsafe fn send_usize(obj: Id, s: &str, a: usize) {
    let f: unsafe extern "C" fn(Id, Sel, usize) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(obj, sel(s), a) }
}
unsafe fn set_frame(view: Id, r: NSRect) {
    let f: unsafe extern "C" fn(Id, Sel, NSRect) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(view, sel("setFrame:"), r) }
}

fn same(a: NSSize, b: NSSize) -> bool {
    (a.w - b.w).abs() < 0.5 && (a.h - b.h).abs() < 0.5
}

/// Asks the Audio Unit for its Cocoa editor view (retained). Main thread.
unsafe fn create_plugin_view(unit: AudioUnit) -> Result<Id> {
    let mut size = 0u32;
    let mut writable = 0u8;
    let st = unsafe { AudioUnitGetPropertyInfo(unit, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global, 0, &mut size, &mut writable) };
    if st != 0 || size < std::mem::size_of::<AudioUnitCocoaViewInfo>() as u32 {
        bail!("this plugin has no editor window");
    }
    let mut buf = vec![0u8; size as usize];
    let mut sz = size;
    let st = unsafe { AudioUnitGetProperty(unit, kAudioUnitProperty_CocoaUI, kAudioUnitScope_Global, 0, buf.as_mut_ptr() as *mut c_void, &mut sz) };
    if st != 0 {
        bail!("cannot read the plugin editor info (OSStatus {st})");
    }
    let info = unsafe { &*(buf.as_ptr() as *const AudioUnitCocoaViewInfo) };
    let n_classes = (sz as usize - std::mem::size_of::<CFURLRef>()) / std::mem::size_of::<CFStringRef>();
    let url = info.mCocoaAUViewBundleLocation;
    let class_name = info.mCocoaAUViewClass[0];
    let result = (|| unsafe {
        // CFURL / CFString are toll-free bridged with NSURL / NSString.
        let bundle = send1(class("NSBundle"), "bundleWithURL:", url as Id);
        if bundle.is_null() {
            bail!("cannot load the plugin editor bundle");
        }
        let cls = send1(bundle, "classNamed:", class_name as Id);
        if cls.is_null() {
            bail!("plugin editor class not found");
        }
        let factory = send0(send0(cls, "alloc"), "init");
        if factory.is_null() {
            bail!("cannot create the plugin editor factory");
        }
        let f: unsafe extern "C" fn(Id, Sel, AudioUnit, NSSize) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let view = f(factory, sel("uiViewForAudioUnit:withSize:"), unit, PREFERRED);
        send0(factory, "release");
        if view.is_null() {
            bail!("the plugin did not create its editor view");
        }
        send0(view, "retain");
        Ok(view)
    })();
    unsafe {
        if !url.is_null() {
            CFRelease(url);
        }
        for i in 0..n_classes.max(1) {
            let s = *info.mCocoaAUViewClass.as_ptr().add(i);
            if !s.is_null() {
                CFRelease(s);
            }
        }
    }
    result.map_err(|e| anyhow!("{e}"))
}

/// A dark container NSView holding a plugin view, scaled to fit (aspect kept,
/// centered). Many plugins (JUCE ones like Vital) only report their real size
/// after being attached, so `layout` is called periodically.
struct PluginHost {
    container: Id,
    view: Id,
    /// Plugin's own size once known.
    natural: Option<NSSize>,
    /// Size we last gave the plugin view.
    applied: NSSize,
}

impl PluginHost {
    unsafe fn new(unit: AudioUnit, frame: NSRect) -> Result<Self> {
        let view = unsafe { create_plugin_view(unit)? };
        unsafe {
            let container = send_rect(send0(class("NSView"), "alloc"), "initWithFrame:", frame);
            send_bool(container, "setWantsLayer:", true);
            let layer = send0(container, "layer");
            if !layer.is_null() {
                let c = CGColorCreateGenericRGB(0.06, 0.065, 0.075, 1.0);
                send1(layer, "setBackgroundColor:", c as Id);
                CGColorRelease(c);
                send_bool(layer, "setMasksToBounds:", true);
            }
            send1(container, "addSubview:", view);
            let natural = frame_of(view).size;
            let mut h = PluginHost { container, view, natural: usable(natural).then_some(natural), applied: natural };
            h.layout();
            Ok(h)
        }
    }

    /// Fits the plugin view into the container. Returns the plugin's natural size when known.
    fn layout(&mut self) -> Option<NSSize> {
        unsafe {
            let cs = frame_of(self.container).size;
            let vs = frame_of(self.view).size;
            // The plugin changed its own size (first real size, or its resize handle).
            if usable(vs) && !same(vs, self.applied) {
                self.natural = Some(vs);
            }
            let Some(n) = self.natural else {
                return None;
            };
            if cs.w < 2.0 || cs.h < 2.0 {
                return Some(n);
            }
            let scale = (cs.w / n.w).min(cs.h / n.h);
            let t = NSSize { w: (n.w * scale).floor(), h: (n.h * scale).floor() };
            let origin = NSPoint { x: ((cs.w - t.w) / 2.0).floor(), y: ((cs.h - t.h) / 2.0).floor() };
            if !same(vs, t) {
                set_frame(self.view, NSRect { origin, size: t });
            } else {
                send_point(self.view, "setFrameOrigin:", origin);
            }
            // If the plugin refused the size, keep what it took (centered).
            let got = frame_of(self.view).size;
            self.applied = got;
            if !same(got, t) {
                let o = NSPoint { x: ((cs.w - got.w) / 2.0).floor(), y: ((cs.h - got.h) / 2.0).floor() };
                send_point(self.view, "setFrameOrigin:", o);
            }
            Some(n)
        }
    }

    fn release(self) {
        unsafe {
            send0(self.container, "removeFromSuperview");
            send0(self.view, "release");
            send0(self.container, "release");
        }
    }
}

/// The plugin editor in its own window (resizable, can go full screen).
pub struct EditorWindow {
    window: Id,
    host: PluginHost,
    sized: bool,
}

// Only ever touched on the main thread; kept in Mutex-protected maps.
unsafe impl Send for EditorWindow {}

impl EditorWindow {
    /// # Safety
    /// Main thread only; `unit` must be a live, initialized Audio Unit.
    pub unsafe fn open(unit: AudioUnit, title: &str) -> Result<Self> {
        unsafe {
            let rect = NSRect { origin: NSPoint::default(), size: FALLBACK };
            let host = PluginHost::new(unit, rect)?;
            // titled | closable | miniaturizable | resizable
            let style: usize = 1 | 2 | 4 | 8;
            let init: unsafe extern "C" fn(Id, Sel, NSRect, usize, usize, bool) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let window = init(send0(class("NSWindow"), "alloc"), sel("initWithContentRect:styleMask:backing:defer:"), rect, style, 2, false);
            if window.is_null() {
                host.release();
                bail!("cannot create the editor window");
            }
            send_bool(window, "setReleasedWhenClosed:", false);
            let t = nsstring(title);
            send1(window, "setTitle:", t);
            send0(t, "release");
            send1(window, "setContentView:", host.container);
            // The green button makes the plugin full screen.
            send_usize(window, "setCollectionBehavior:", 1 << 7); // FullScreenPrimary
            send0(window, "center");
            send1(window, "makeKeyAndOrderFront:", std::ptr::null_mut());
            let mut ed = EditorWindow { window, host, sized: false };
            ed.fit();
            Ok(ed)
        }
    }

    pub fn show(&mut self) {
        unsafe { send1(self.window, "makeKeyAndOrderFront:", std::ptr::null_mut()) };
        self.fit();
    }

    /// Lays the plugin out; the first time its real size is known, the window takes it.
    pub fn fit(&mut self) {
        unsafe {
            if !is_visible(self.window) {
                return;
            }
            if let Some(n) = self.host.layout() {
                if !self.sized {
                    self.sized = true;
                    send_size(self.window, "setContentSize:", n);
                    send0(self.window, "center");
                    self.host.layout();
                }
            }
        }
    }

    /// (plugin view size, window content size) — diagnostics.
    pub fn sizes(&self) -> ((f64, f64), (f64, f64)) {
        unsafe {
            let v = frame_of(self.host.view).size;
            let c = frame_of(send0(self.window, "contentView")).size;
            ((v.w, v.h), (c.w, c.h))
        }
    }

    /// The NSWindow pointer and whether it is on screen.
    pub fn ns_window(&self) -> (usize, bool) {
        (self.window as usize, unsafe { is_visible(self.window) })
    }

    pub fn close(self) {
        unsafe {
            send0(self.window, "close");
            send1(self.window, "setContentView:", std::ptr::null_mut());
            self.host.release();
            send0(self.window, "release");
        }
    }
}

/// Runs the main run loop for `seconds` (diagnostics / tests without Tauri).
pub fn pump_run_loop(seconds: f64) {
    unsafe {
        let date_cls = class("NSDate");
        let f: unsafe extern "C" fn(Id, Sel, f64) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let until = f(date_cls, sel("dateWithTimeIntervalSinceNow:"), seconds);
        let rl = send0(class("NSRunLoop"), "currentRunLoop");
        send1(rl, "runUntilDate:", until);
    }
}

/// Makes this process a regular GUI app (diagnostics without Tauri).
pub fn init_app() {
    unsafe {
        let app = send0(class("NSApplication"), "sharedApplication");
        send_isize(app, "setActivationPolicy:", 0);
        send0(app, "finishLaunching");
    }
}

/// Diagnostics: dequeues and dispatches app events for `seconds` (like NSApp run).
pub fn pump_events(seconds: f64) {
    unsafe {
        let app = send0(class("NSApplication"), "sharedApplication");
        let date: unsafe extern "C" fn(Id, Sel, f64) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let until = date(class("NSDate"), sel("dateWithTimeIntervalSinceNow:"), seconds);
        let mode = nsstring("kCFRunLoopDefaultMode");
        let next: unsafe extern "C" fn(Id, Sel, usize, Id, Id, bool) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        loop {
            let ev = next(app, sel("nextEventMatchingMask:untilDate:inMode:dequeue:"), usize::MAX, until, mode, true);
            if ev.is_null() {
                break;
            }
            send1(app, "sendEvent:", ev);
        }
    }
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// Runs `f` inside an autorelease pool (diagnostics without an app run loop).
pub fn with_pool<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let r = f();
        objc_autoreleasePoolPop(pool);
        r
    }
}

/// Ctrl+Tab (Ctrl+Shift+Tab backwards) cycles between the main window and the
/// open plugin windows. A local event monitor sees the keys even when a plugin
/// editor has keyboard focus. Main thread, once.
pub fn install_window_cycling(main_window: *mut c_void, plugin_windows: fn() -> Vec<usize>) {
    const KEY_DOWN_MASK: u64 = 1 << 10;
    const SHIFT: usize = 1 << 17;
    const CONTROL: usize = 1 << 18;
    const TAB_KEY: u16 = 48;
    let main = main_window as usize;
    let handler = block2::RcBlock::new(move |event: *mut c_void| -> *mut c_void {
        unsafe {
            let key: unsafe extern "C" fn(Id, Sel) -> u16 = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let mods: unsafe extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let flags = mods(event, sel("modifierFlags"));
            if key(event, sel("keyCode")) != TAB_KEY || flags & CONTROL == 0 {
                return event;
            }
            let plugins = plugin_windows();
            if plugins.is_empty() {
                return event;
            }
            let mut order = vec![main];
            order.extend(plugins);
            let app = send0(class("NSApplication"), "sharedApplication");
            let key_window = send0(app, "keyWindow") as usize;
            let i = order.iter().position(|w| *w == key_window).unwrap_or(0);
            let n = order.len();
            let next = if flags & SHIFT != 0 { (i + n - 1) % n } else { (i + 1) % n };
            send1(order[next] as Id, "makeKeyAndOrderFront:", std::ptr::null_mut());
            std::ptr::null_mut() // swallow the shortcut
        }
    });
    unsafe {
        let add: unsafe extern "C" fn(Id, Sel, u64, *mut c_void) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let monitor = add(class("NSEvent"), sel("addLocalMonitorForEventsMatchingMask:handler:"), KEY_DOWN_MASK, block2::RcBlock::as_ptr(&handler) as *mut c_void);
        if !monitor.is_null() {
            send0(monitor, "retain");
        }
    }
    // The monitor keeps a copy of the block; keep ours too for the app's lifetime.
    std::mem::forget(handler);
}

/// Diagnostics: a plain titled window (returns the NSWindow).
pub fn test_window(title: &str) -> *mut c_void {
    unsafe {
        let rect = NSRect { origin: NSPoint { x: 80.0, y: 80.0 }, size: NSSize { w: 500.0, h: 300.0 } };
        let init: unsafe extern "C" fn(Id, Sel, NSRect, usize, usize, bool) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let window = init(send0(class("NSWindow"), "alloc"), sel("initWithContentRect:styleMask:backing:defer:"), rect, 1 | 2, 2, false);
        let t = nsstring(title);
        send1(window, "setTitle:", t);
        send1(window, "makeKeyAndOrderFront:", std::ptr::null_mut());
        window
    }
}

/// Diagnostics: activates the app, returns the key window pointer.
pub fn activate_and_key_window() -> usize {
    unsafe {
        let app = send0(class("NSApplication"), "sharedApplication");
        send_bool(app, "activateIgnoringOtherApps:", true);
        send0(app, "keyWindow") as usize
    }
}

/// Diagnostics: posts a Ctrl+Tab key-down event to the app's event queue.
pub fn post_ctrl_tab() {
    unsafe {
        let app = send0(class("NSApplication"), "sharedApplication");
        let tab = nsstring("\t");
        // keyEventWithType:location:modifierFlags:timestamp:windowNumber:context:characters:charactersIgnoringModifiers:isARepeat:keyCode:
        let f: unsafe extern "C" fn(Id, Sel, usize, NSPoint, usize, f64, isize, Id, Id, Id, bool, u16) -> Id =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let ev = f(
            class("NSEvent"),
            sel("keyEventWithType:location:modifierFlags:timestamp:windowNumber:context:characters:charactersIgnoringModifiers:isARepeat:keyCode:"),
            10, // NSEventTypeKeyDown
            NSPoint::default(),
            1 << 18, // control
            0.0,
            0,
            std::ptr::null_mut(),
            tab,
            tab,
            false,
            48,
        );
        let post: unsafe extern "C" fn(Id, Sel, Id, bool) = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        post(app, sel("postEvent:atStart:"), ev, false);
    }
}
