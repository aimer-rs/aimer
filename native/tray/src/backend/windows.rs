//! Windows: `Shell_NotifyIconW` with a hidden window on a thread of its own.
//!
//! The shell talks to a tray icon through window messages, and a window belongs
//! to the thread that made it, so one thread creates the window, owns the icon
//! and its popup menu, and runs the message loop. The application's thread
//! reaches it through a queue and a posted message; the thread reports clicks
//! to the handler directly, which is why [`TrayHandler`] is `Send + Sync`.
//!
//! The window is a hidden top-level one, not message-only, because only
//! top-level windows are told that Explorer restarted (`TaskbarCreated`) — the
//! moment the icon has to be added again.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, HICON, HMENU, MF_CHECKED, MF_GRAYED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WM_APP, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONUP, WM_NULL,
    WM_RBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW,
};

use super::Backend;
use crate::{TrayButton, TrayError, TrayEvent, TrayHandler, TrayIcon, TrayItemId, TrayMenu, TrayMenuEntry};

/// The shell's callback message for the icon.
const WM_TRAY: u32 = WM_APP + 1;
/// "Look at the request queue."
const WM_REQUESTS: u32 = WM_APP + 2;
/// Characters in `szTip`, terminator included.
const TIP_CAPACITY: usize = 128;

enum Request {
    SetIcon(TrayIcon),
    SetTooltip(String),
    SetMenu(Option<TrayMenu>),
    Quit,
}

type Queue = Arc<Mutex<VecDeque<Request>>>;

/// What the window procedure needs; it lives in the tray thread only.
struct Local {
    hwnd: HWND,
    icon: HICON,
    tooltip: String,
    menu: Option<TrayMenu>,
    handler: TrayHandler,
    queue: Queue,
    taskbar_created: u32,
}

thread_local! {
    static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) };
}

struct WindowsTray {
    hwnd: usize,
    queue: Queue,
    thread: Option<JoinHandle<()>>,
}

pub(super) fn create(
    icon: &TrayIcon,
    tooltip: &str,
    menu: Option<&TrayMenu>,
    handler: TrayHandler,
) -> Result<Box<dyn Backend>, TrayError> {
    let queue: Queue = Arc::default();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (icon, tooltip, menu) = (icon.clone(), tooltip.to_owned(), menu.cloned());
    let thread_queue = Arc::clone(&queue);
    let thread = std::thread::Builder::new()
        .name("aimer-tray".into())
        .spawn(move || run(icon, tooltip, menu, handler, thread_queue, ready_tx))
        .map_err(|error| TrayError::Platform(format!("could not start the tray thread: {error}")))?;
    match ready_rx.recv() {
        Ok(Ok(hwnd)) => Ok(Box::new(WindowsTray { hwnd, queue, thread: Some(thread) })),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => Err(TrayError::Platform("the tray thread ended".into())),
    }
}

impl WindowsTray {
    fn send(&self, request: Request) {
        self.queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push_back(request);
        // SAFETY: posting to a window is allowed from any thread; if the window
        // is already gone the call just fails.
        unsafe { PostMessageW(self.hwnd as HWND, WM_REQUESTS, 0, 0) };
    }
}

impl Backend for WindowsTray {
    fn set_icon(&mut self, icon: &TrayIcon) {
        self.send(Request::SetIcon(icon.clone()));
    }

    fn set_tooltip(&mut self, tooltip: &str) {
        self.send(Request::SetTooltip(tooltip.to_owned()));
    }

    fn set_menu(&mut self, menu: Option<&TrayMenu>) {
        self.send(Request::SetMenu(menu.cloned()));
    }
}

impl Drop for WindowsTray {
    fn drop(&mut self) {
        self.send(Request::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn run(
    icon: TrayIcon,
    tooltip: String,
    menu: Option<TrayMenu>,
    handler: TrayHandler,
    queue: Queue,
    ready: mpsc::Sender<Result<usize, TrayError>>,
) {
    let class_name = wide("AimerTrayWindow");
    // SAFETY: every pointer handed to the system below is valid for the call;
    // `wnd_proc` matches the required signature. A second registration of the
    // class fails harmlessly and the first one is reused.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(null());
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = Some(wnd_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class);
        CreateWindowExW(0, class_name.as_ptr(), class_name.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), instance, null())
    };
    if hwnd.is_null() {
        let _ = ready.send(Err(TrayError::Platform("could not create the tray window".into())));
        return;
    }
    let Some(hicon) = make_icon(&icon) else {
        // SAFETY: the window was created above on this thread.
        unsafe { DestroyWindow(hwnd) };
        let _ = ready.send(Err(TrayError::Platform("could not create the tray icon".into())));
        return;
    };
    // SAFETY: a null-terminated name.
    let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    let local = Local { hwnd, icon: hicon, tooltip, menu, handler, queue, taskbar_created };
    if !notify(NIM_ADD, &local) {
        // SAFETY: both were created above on this thread.
        unsafe {
            DestroyIcon(hicon);
            DestroyWindow(hwnd);
        }
        let _ = ready.send(Err(TrayError::Platform("the shell refused the tray icon".into())));
        return;
    }
    LOCAL.with(|slot| *slot.borrow_mut() = Some(local));
    let _ = ready.send(Ok(hwnd as usize));

    // SAFETY: `msg` is a valid out buffer; a result of 0 or less is a quit or
    // an error and ends the loop.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    LOCAL.with(|slot| {
        if let Some(local) = slot.borrow_mut().take() {
            // SAFETY: the icon was created on this thread and is no longer shown.
            unsafe { DestroyIcon(local.icon) };
        }
    });
}

/// Applies `message` (`NIM_ADD`, `NIM_MODIFY`, `NIM_DELETE`) for the icon.
fn notify(message: u32, local: &Local) -> bool {
    // SAFETY: an all-zero `NOTIFYICONDATAW` is valid; the fields set below are
    // the ones `uFlags` announces.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = local.hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = local.icon;
    for (slot, unit) in data.szTip[..TIP_CAPACITY - 1].iter_mut().zip(local.tooltip.encode_utf16()) {
        *slot = unit;
    }
    // SAFETY: `data` is fully initialised above.
    unsafe { Shell_NotifyIconW(message, &data) != 0 }
}

/// An `HICON` of `icon`: 32-bit BGRA colour data and an all-clear AND mask, so
/// the alpha channel decides the shape.
fn make_icon(icon: &TrayIcon) -> Option<HICON> {
    let (width, height) = (icon.width() as i32, icon.height() as i32);
    let mut bgra = icon.rgba().to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    // An AND mask row is one bit per pixel, padded to 16 bits.
    let mask = vec![0u8; ((icon.width() as usize).div_ceil(16) * 2) * icon.height() as usize];
    // SAFETY: the buffers hold exactly the bits the call is told about.
    let handle = unsafe { CreateIcon(GetModuleHandleW(null()), width, height, 1, 32, mask.as_ptr(), bgra.as_ptr()) };
    (!handle.is_null()).then_some(handle)
}

/// The native popup menu for `menu`. A command is the item id plus one, so
/// that 0 can mean "dismissed" in `TrackPopupMenu`'s answer.
fn build_menu(menu: &TrayMenu) -> HMENU {
    // SAFETY: plain creation.
    let native = unsafe { CreatePopupMenu() };
    for entry in menu.entries() {
        match entry {
            TrayMenuEntry::Separator => {
                // SAFETY: a separator ignores its id and text.
                unsafe { AppendMenuW(native, MF_SEPARATOR, 0, null()) };
            }
            TrayMenuEntry::Item(item) => {
                let Some(command) = command_for(item.id) else { continue };
                let mut flags = MF_STRING;
                if !item.enabled {
                    flags |= MF_GRAYED;
                }
                if item.checked == Some(true) {
                    flags |= MF_CHECKED;
                }
                // SAFETY: `title` is null-terminated and copied by the call.
                unsafe { AppendMenuW(native, flags, command, wide(&item.title).as_ptr()) };
            }
            TrayMenuEntry::Submenu { title, enabled, menu } => {
                let mut flags = MF_POPUP | MF_STRING;
                if !enabled {
                    flags |= MF_GRAYED;
                }
                // SAFETY: the submenu handle becomes owned by `native` and is
                // destroyed with it.
                unsafe { AppendMenuW(native, flags, build_menu(menu) as usize, wide(title).as_ptr()) };
            }
        }
    }
    native
}

fn command_for(id: TrayItemId) -> Option<usize> {
    (id.0 < i32::MAX as u32).then(|| id.0 as usize + 1)
}

fn show_menu(hwnd: HWND, menu: &TrayMenu, handler: &TrayHandler) {
    let native = build_menu(menu);
    // SAFETY: `native` is a menu this function owns; the foreground window and
    // the trailing `WM_NULL` are what the popup needs to close when the user
    // clicks elsewhere. `TrackPopupMenu` runs its own loop and returns the
    // chosen command because of `TPM_RETURNCMD`.
    let command = unsafe {
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        SetForegroundWindow(hwnd);
        let command =
            TrackPopupMenu(native, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD, at.x, at.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(native);
        command
    };
    if command > 0 {
        handler(TrayEvent::MenuItem(TrayItemId(command as u32 - 1)));
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TRAY => {
            let event = lparam as u32;
            let click = match event {
                WM_LBUTTONUP => Some((TrayButton::Left, false)),
                WM_RBUTTONUP => Some((TrayButton::Right, false)),
                WM_MBUTTONUP => Some((TrayButton::Middle, false)),
                WM_LBUTTONDBLCLK => Some((TrayButton::Left, true)),
                WM_RBUTTONDBLCLK => Some((TrayButton::Right, true)),
                WM_MBUTTONDBLCLK => Some((TrayButton::Middle, true)),
                _ => None,
            };
            if let Some((button, double)) = click {
                // Nothing is borrowed while the handler or the popup runs: both
                // can pump messages that come back into this procedure.
                let shared = LOCAL.with(|slot| slot.borrow().as_ref().map(|l| (Arc::clone(&l.handler), l.menu.clone())));
                if let Some((handler, menu)) = shared {
                    handler(if double { TrayEvent::DoubleClick(button) } else { TrayEvent::Click(button) });
                    if button == TrayButton::Right && !double {
                        if let Some(menu) = menu {
                            show_menu(hwnd, &menu, &handler);
                        }
                    }
                }
            }
            0
        }
        WM_REQUESTS => {
            drain_requests(hwnd);
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => {
            let readd = LOCAL.with(|slot| {
                slot.borrow().as_ref().filter(|local| local.taskbar_created != 0 && local.taskbar_created == message).map(|l| notify(NIM_ADD, l))
            });
            if readd.is_some() {
                return 0;
            }
            // SAFETY: forwards the unchanged arguments of this call.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

fn drain_requests(hwnd: HWND) {
    let queue = LOCAL.with(|slot| slot.borrow().as_ref().map(|local| Arc::clone(&local.queue)));
    let Some(queue) = queue else { return };
    let requests: Vec<Request> = queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain(..).collect();
    for request in requests {
        match request {
            Request::Quit => {
                LOCAL.with(|slot| {
                    if let Some(local) = slot.borrow().as_ref() {
                        notify(NIM_DELETE, local);
                    }
                });
                // SAFETY: the window belongs to this thread.
                unsafe { DestroyWindow(hwnd) };
                return;
            }
            Request::SetIcon(icon) => {
                if let Some(new) = make_icon(&icon) {
                    LOCAL.with(|slot| {
                        if let Some(local) = slot.borrow_mut().as_mut() {
                            let old = std::mem::replace(&mut local.icon, new);
                            notify(NIM_MODIFY, local);
                            // SAFETY: the shell now shows `new`; `old` is unused.
                            unsafe { DestroyIcon(old) };
                        }
                    });
                }
            }
            Request::SetTooltip(tooltip) => LOCAL.with(|slot| {
                if let Some(local) = slot.borrow_mut().as_mut() {
                    local.tooltip = tooltip;
                    notify(NIM_MODIFY, local);
                }
            }),
            Request::SetMenu(menu) => LOCAL.with(|slot| {
                if let Some(local) = slot.borrow_mut().as_mut() {
                    local.menu = menu;
                }
            }),
        }
    }
}
