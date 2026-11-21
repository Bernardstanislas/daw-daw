//! Native editor window of a VST3 plugin on Windows: a top-level Win32 window
//! the plugin attaches its HWND child to. MUST be used on the main thread,
//! whose event loop (Tauri's) dispatches the window's messages.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use anyhow::{Result, anyhow, bail};
use vst3::Steinberg::{
    IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewContentScaleSupport, IPlugViewContentScaleSupportTrait, IPlugViewTrait, ViewRect, kPlatformTypeHWND, kResultOk,
    tresult,
};
use vst3::{Class, ComPtr, ComRef, ComWrapper};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, HICON, IDC_ARROW, IsWindowVisible,
    LoadCursorW, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_CLOSE,
    WM_SIZE, WNDCLASSEXW, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME,
};

use super::vst3::Vst3Unit;

const CLASS_NAME: &str = "DawDawVst3Editor";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn rect_size(r: &ViewRect) -> (i32, i32) {
    (r.right - r.left, r.bottom - r.top)
}

fn style(resizable: bool) -> u32 {
    let base = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    if resizable { base | WS_THICKFRAME | WS_MAXIMIZEBOX } else { base }
}

/// Window size for a given client size.
fn outer_size(client: (i32, i32), resizable: bool) -> (i32, i32) {
    let mut r = RECT { left: 0, top: 0, right: client.0, bottom: client.1 };
    unsafe { AdjustWindowRectEx(&mut r, style(resizable), 0, 0) };
    (r.right - r.left, r.bottom - r.top)
}

/// State shared with the window procedure (its address is stored in the window).
struct WinState {
    view: ComPtr<IPlugView>,
    can_resize: bool,
    /// Set while we resize the window ourselves, so `WM_SIZE` does not echo back to the view.
    resizing: Cell<bool>,
}

/// `IPlugFrame`: the plugin asks to change the size of its editor.
struct WinFrame {
    hwnd: AtomicIsize,
    resizable: AtomicBool,
}

impl Class for WinFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for WinFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> tresult {
        let hwnd = self.hwnd.load(Ordering::Relaxed) as HWND;
        if hwnd.is_null() || view.is_null() || new_size.is_null() {
            return kResultOk;
        }
        unsafe {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WinState;
            if !state.is_null() && (*state).resizing.replace(true) {
                return kResultOk; // already resizing: the plugin called back from onSize
            }
            let (w, h) = outer_size(rect_size(&*new_size), self.resizable.load(Ordering::Relaxed));
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            if let Some(v) = ComRef::from_raw(view) {
                v.onSize(new_size);
            }
            if !state.is_null() {
                (*state).resizing.set(false);
            }
        }
        kResultOk
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WinState;
        match msg {
            // Closing only hides the window: the plugin stays attached and reopening is instant.
            WM_CLOSE => {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            }
            WM_SIZE if !state.is_null() => {
                let st = &*state;
                if st.can_resize && !st.resizing.replace(true) {
                    let mut r: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut r);
                    let mut vr = ViewRect { left: 0, top: 0, right: r.right, bottom: r.bottom };
                    if vr.right > 0 && vr.bottom > 0 {
                        st.view.onSize(&mut vr);
                    }
                    st.resizing.set(false);
                }
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn register_class() -> Result<()> {
    static DONE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *DONE.get_or_init(|| unsafe {
        let name = wide(CLASS_NAME);
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: std::ptr::null_mut() as HICON,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&class) != 0
    });
    if ok { Ok(()) } else { bail!("cannot register the editor window class") }
}

/// A VST3 editor in its own top-level window.
pub struct EditorWindow {
    hwnd: HWND,
    // The view is released before the frame and the unit.
    state: Box<WinState>,
    frame: ComWrapper<WinFrame>,
    _unit: Arc<Vst3Unit>,
}

// Only ever touched on the main thread; kept in Mutex-protected maps.
unsafe impl Send for EditorWindow {}

impl EditorWindow {
    /// Main thread only.
    pub fn open(unit: Arc<Vst3Unit>, title: &str) -> Result<Self> {
        register_class()?;
        unsafe {
            let view = unit.create_view()?;
            if view.isPlatformTypeSupported(kPlatformTypeHWND) != kResultOk {
                bail!("this plugin's editor does not support Windows");
            }
            let can_resize = view.canResize() == kResultOk;
            let name = wide(CLASS_NAME);
            let title_w = wide(title);
            let hwnd = CreateWindowExW(
                0,
                name.as_ptr(),
                title_w.as_ptr(),
                style(can_resize),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                300,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                bail!("cannot create the editor window");
            }
            // High-DPI screens: tell the plugin before it reports its size.
            let dpi = GetDpiForWindow(hwnd);
            if dpi > 0 {
                if let Some(scale) = view.cast::<IPlugViewContentScaleSupport>() {
                    scale.setContentScaleFactor(dpi as f32 / 96.0);
                }
            }
            let mut r = ViewRect { left: 0, top: 0, right: 0, bottom: 0 };
            let client = if view.getSize(&mut r) == kResultOk && rect_size(&r).0 > 0 && rect_size(&r).1 > 0 { rect_size(&r) } else { (900, 640) };
            let (w, h) = outer_size(client, can_resize);
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);

            let frame = ComWrapper::new(WinFrame { hwnd: AtomicIsize::new(hwnd as isize), resizable: AtomicBool::new(can_resize) });
            let state = Box::new(WinState { view, can_resize, resizing: Cell::new(false) });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const WinState as isize);
            state.view.setFrame(frame.as_com_ref::<IPlugFrame>().ok_or_else(|| anyhow!("frame"))?.as_ptr());
            if state.view.attached(hwnd as *mut c_void, kPlatformTypeHWND) != kResultOk {
                state.view.setFrame(std::ptr::null_mut());
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DestroyWindow(hwnd);
                bail!("the plugin refused to attach its editor");
            }
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            Ok(EditorWindow { hwnd, state, frame, _unit: unit })
        }
    }

    pub fn show(&mut self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
            SetForegroundWindow(self.hwnd);
        }
    }

    /// Windows editors resize through `IPlugFrame::resizeView`: nothing to poll.
    pub fn fit(&mut self) {}

    /// The HWND and whether it is on screen.
    pub fn handle(&self) -> (usize, bool) {
        (self.hwnd as usize, unsafe { IsWindowVisible(self.hwnd) != 0 })
    }

    pub fn close(self) {
        unsafe {
            self.state.view.removed();
            self.state.view.setFrame(std::ptr::null_mut());
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
        }
        let _ = &self.frame;
    }
}
