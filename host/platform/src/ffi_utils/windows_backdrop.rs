//! The system backdrop of Windows 11: Mica and Acrylic, drawn by DWM.
//!
//! DWM draws the material behind the window's client area once the frame is
//! extended over it, and shows it wherever the window's own pixels are
//! transparent. Before Windows 11 22H2 the attribute is not known and DWM
//! refuses it, which is reported as `false`.

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::window_backdrop::WindowMaterial;

/// `DWMWA_SYSTEMBACKDROP_TYPE`.
const SYSTEM_BACKDROP_TYPE: u32 = 38;
/// `DWMSBT_AUTO`: the system decides, which for a plain window is no material.
const BACKDROP_AUTO: u32 = 0;

/// `MARGINS`.
#[repr(C)]
struct Margins {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut c_void,
        attribute: u32,
        value: *const c_void,
        size: u32,
    ) -> i32;
    fn DwmExtendFrameIntoClientArea(hwnd: *mut c_void, margins: *const Margins) -> i32;
}

/// Gives `window` a system backdrop of `material`, or removes it.
///
/// Returns whether DWM accepted the request.
pub fn apply(window: &Window, material: Option<WindowMaterial>) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as *mut c_void;
    let (backdrop, margin) = match material {
        Some(material) => (material.dwm_backdrop_type(), -1),
        None => (BACKDROP_AUTO, 0),
    };
    let margins = Margins {
        left: margin,
        right: margin,
        top: margin,
        bottom: margin,
    };
    // SAFETY: `hwnd` belongs to the live window `window_handle` was taken from,
    // and the pointers are to values that outlive the calls.
    unsafe {
        let extended = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let set = DwmSetWindowAttribute(
            hwnd,
            SYSTEM_BACKDROP_TYPE,
            (&raw const backdrop).cast(),
            size_of::<u32>() as u32,
        );
        extended >= 0 && set >= 0
    }
}
