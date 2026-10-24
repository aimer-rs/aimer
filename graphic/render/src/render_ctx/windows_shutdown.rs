//! Emergency shutdown keeps DXGI's synchronous window messages serviceable.
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, TranslateMessage, WM_QUIT,
};

pub(super) fn pump_messages() {
    // SAFETY: called only on the window/event-loop thread. PeekMessageW
    // initializes MSG before it is read; dispatch uses that same live value.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            if message.message == WM_QUIT {
                // Preserve the exit signal for winit after the GPU owner stops.
                PostQuitMessage(message.wParam.0 as i32);
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    std::thread::yield_now();
}
