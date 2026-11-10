//! Windows: a notification-area balloon, shown from a hidden window on a thread
//! of its own.
//!
//! Without WinRT bindings the portable way to put text on screen is the balloon
//! of a notification-area icon (`Shell_NotifyIconW` with `NIF_INFO`). The shell
//! talks to an icon through window messages, and a window belongs to the thread
//! that made it, so one thread creates a hidden message-only window, owns the
//! icon and runs the message loop. The application's thread reaches it through
//! a [`CommandQueue`] and a posted message; the thread reports clicks to the
//! handler directly, which is why [`NotificationHandler`] is `Send + Sync`.
//!
//! Limits that come with balloons:
//!
//! - Only one balloon is on screen at a time. Showing another replaces it, and
//!   nothing is queued; the replaced notification is simply gone.
//! - Balloons have no buttons, so [`Notification::action`] buttons are ignored
//!   and [`crate::NotificationEvent::Action`] is never produced.
//! - Title and body are cut to what the shell accepts (63 and 255 UTF-16 units).
//! - The shell cannot say whether a balloon went away because the user closed it
//!   or because it timed out on recent Windows versions, so `Dismissed` is only
//!   reported when the shell says the balloon was hidden.
//! - The icon is the stock application icon, and it is in the notification area
//!   only while a balloon is.

use std::collections::VecDeque;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_NOSOUND, NIIF_USER, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIN_BALLOONHIDE, NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetMessageW,
    GetWindowLongPtrW, HICON, HWND_MESSAGE, IDI_APPLICATION, LoadIconW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, SetWindowLongPtrW, TranslateMessage, UnregisterClassW, WM_APP, WM_DESTROY, WNDCLASSW,
};

use super::Backend;
use crate::scheduler::Scheduler;
use crate::{
    Notification, NotificationError, NotificationEvent, NotificationHandler, NotificationId, Permission,
    PermissionCallback,
};

/// The shell's callback message for the icon.
const WM_TRAY: u32 = WM_APP + 1;
/// "Look at the command queue."
const WM_COMMANDS: u32 = WM_APP + 2;
/// The one icon this window owns.
const ICON_ID: u32 = 1;
/// Units in `szInfoTitle`, terminator included.
const TITLE_CAPACITY: usize = 64;
/// Units in `szInfo`, terminator included.
const BODY_CAPACITY: usize = 256;
/// Units in `szTip`, terminator included.
const TIP_CAPACITY: usize = 128;

/// Something for the window thread to do.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Show(Notification),
    Close(NotificationId),
    Quit,
}

/// Commands from any thread, taken in order by the window thread.
#[derive(Default)]
struct CommandQueue {
    commands: Mutex<VecDeque<Command>>,
}

impl CommandQueue {
    fn push(&self, command: Command) {
        self.commands.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push_back(command);
    }

    /// Everything queued so far, oldest first, leaving the queue empty.
    fn drain(&self) -> Vec<Command> {
        self.commands.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain(..).collect()
    }
}

/// The way to the window thread: its queue and the window to wake it through.
#[derive(Clone)]
struct CommandPort {
    hwnd: usize,
    queue: Arc<CommandQueue>,
}

impl CommandPort {
    fn send(&self, command: Command) {
        self.queue.push(command);
        // SAFETY: posting to a window is allowed from any thread; if the window
        // is already gone the call just fails.
        unsafe { PostMessageW(self.hwnd as HWND, WM_COMMANDS, 0, 0) };
    }
}

/// What the window procedure needs; it lives in the window thread only.
struct Local {
    hwnd: HWND,
    /// The stock application icon: shared by the system, never destroyed.
    icon: HICON,
    /// Whether the icon is in the notification area.
    shown: bool,
    /// The notification whose balloon is up, or was last put up and not yet
    /// answered.
    current: Option<NotificationId>,
    handler: NotificationHandler,
    queue: Arc<CommandQueue>,
}

/// Runs `f` on the state of the window `hwnd`; `None` before the state is
/// attached, after it is detached, or while it is already borrowed (a handler
/// that pumps messages comes back in here).
fn with_local<R>(hwnd: HWND, f: impl FnOnce(&mut Local) -> R) -> Option<R> {
    // SAFETY: the user data is 0 or the pointer `run` made with `Box::into_raw`
    // and keeps alive until it clears the slot; only this window's thread, the
    // only caller, ever reads it.
    let cell = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<Local>).as_ref() }?;
    let mut local = cell.try_borrow_mut().ok()?;
    Some(f(&mut local))
}

struct WindowsNotifier {
    port: CommandPort,
    scheduler: Option<Scheduler>,
    thread: Option<JoinHandle<()>>,
}

pub(super) fn create(handler: NotificationHandler) -> Result<Box<dyn Backend>, NotificationError> {
    let queue: Arc<CommandQueue> = Arc::default();
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread_queue = Arc::clone(&queue);
    let thread = std::thread::Builder::new()
        .name("aimer-notification".into())
        .spawn(move || run(handler, thread_queue, ready_tx))
        .map_err(|error| NotificationError::Platform(format!("could not start the notification thread: {error}")))?;
    let hwnd = match ready_rx.recv() {
        Ok(Ok(hwnd)) => hwnd,
        Ok(Err(error)) => {
            let _ = thread.join();
            return Err(error);
        }
        Err(_) => return Err(NotificationError::Platform("the notification thread ended".into())),
    };
    let port = CommandPort { hwnd, queue };
    let fire_port = port.clone();
    let scheduler = match Scheduler::new(move |notification| fire_port.send(Command::Show(notification))) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            port.send(Command::Quit);
            let _ = thread.join();
            return Err(NotificationError::Platform(format!("could not start the scheduler: {error}")));
        }
    };
    Ok(Box::new(WindowsNotifier { port, scheduler: Some(scheduler), thread: Some(thread) }))
}

impl Backend for WindowsNotifier {
    fn show(&self, notification: &Notification) -> Result<(), NotificationError> {
        match (notification.delay, &self.scheduler) {
            (Some(delay), Some(scheduler)) => scheduler.schedule(notification.clone(), delay),
            _ => self.port.send(Command::Show(notification.clone())),
        }
        Ok(())
    }

    fn close(&self, id: NotificationId) -> Result<(), NotificationError> {
        if let Some(scheduler) = &self.scheduler {
            scheduler.cancel(id);
        }
        self.port.send(Command::Close(id));
        Ok(())
    }

    fn permission(&self, done: PermissionCallback) {
        done(Permission::Granted);
    }

    fn request_permission(&self, done: PermissionCallback) {
        done(Permission::Granted);
    }

    fn register_push(&self) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }
}

impl Drop for WindowsNotifier {
    fn drop(&mut self) {
        // The scheduler goes first so nothing fires into a window being closed.
        self.scheduler.take();
        self.port.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            // A handler running on the window thread may be the one dropping
            // the notifier; joining oneself would never return.
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

/// `text` as UTF-16 in a zero-terminated array of `N` units, cut at a character
/// boundary when it does not fit: a surrogate pair is never split.
fn truncate_utf16<const N: usize>(text: &str) -> [u16; N] {
    let mut out = [0u16; N];
    let mut used = 0;
    let mut buffer = [0u16; 2];
    for character in text.chars() {
        let units = character.encode_utf16(&mut buffer);
        if used + units.len() > N.saturating_sub(1) {
            break;
        }
        out[used..used + units.len()].copy_from_slice(units);
        used += units.len();
    }
    out
}

/// `dwInfoFlags` for a balloon: the application icon, quiet when `sound` is off.
fn info_flags(sound: bool) -> u32 {
    let flags = NIIF_USER | NIIF_INFO;
    if sound { flags } else { flags | NIIF_NOSOUND }
}

/// What a message from the shell asks for.
#[derive(Debug, PartialEq, Eq)]
struct Reaction {
    /// The balloon is over, so the icon goes.
    remove_icon: bool,
    event: Option<NotificationEvent>,
}

/// Maps the shell's callback `message` to what to do, given the notification
/// whose balloon is up. Messages for a balloon nobody is waiting on produce no
/// event, which also keeps a click followed by a hide from reporting twice.
fn react(message: u32, current: Option<NotificationId>) -> Reaction {
    match message {
        NIN_BALLOONUSERCLICK => Reaction { remove_icon: true, event: current.map(NotificationEvent::Clicked) },
        NIN_BALLOONHIDE => Reaction { remove_icon: true, event: current.map(NotificationEvent::Dismissed) },
        NIN_BALLOONTIMEOUT => Reaction { remove_icon: true, event: None },
        _ => Reaction { remove_icon: false, event: None },
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn run(handler: NotificationHandler, queue: Arc<CommandQueue>, ready: mpsc::Sender<Result<usize, NotificationError>>) {
    static CLASS_COUNTER: AtomicU32 = AtomicU32::new(0);
    let class_name = wide(&format!(
        "AimerNotificationWindow-{}-{}",
        std::process::id(),
        CLASS_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    // SAFETY: every pointer handed to the system below is valid for the call;
    // `wnd_proc` matches the required signature; the class name is unique, so
    // the registration is this thread's own.
    let (hwnd, instance) = unsafe {
        let instance = GetModuleHandleW(null());
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = Some(wnd_proc);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        if RegisterClassW(&class) == 0 {
            let _ = ready.send(Err(NotificationError::Platform("could not register the notification window class".into())));
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            instance,
            null(),
        );
        (hwnd, instance)
    };
    if hwnd.is_null() {
        // SAFETY: the class was registered above by this thread.
        unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
        let _ = ready.send(Err(NotificationError::Platform("could not create the notification window".into())));
        return;
    }
    // SAFETY: `IDI_APPLICATION` is a stock-icon id; the system owns the result.
    let icon = unsafe { LoadIconW(null_mut(), IDI_APPLICATION) };
    let state = Box::into_raw(Box::new(RefCell::new(Local { hwnd, icon, shown: false, current: None, handler, queue })));
    // SAFETY: the window is this thread's; `state` stays valid until it is
    // detached and freed below, after the message loop.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize) };
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
    with_local(hwnd, remove_icon);
    // SAFETY: the slot is cleared before `state` is freed, so the window
    // procedure can no longer reach it; it came from `Box::into_raw` above and
    // is freed once. `DestroyWindow` is a no-op failure when the window is
    // already gone, and the class belongs to this thread.
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        drop(Box::from_raw(state));
        DestroyWindow(hwnd);
        UnregisterClassW(class_name.as_ptr(), instance);
    }
}

/// A `NOTIFYICONDATAW` naming this window's icon and nothing else.
fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: an all-zero `NOTIFYICONDATAW` is valid; `uFlags` stays 0 until
    // the caller announces the fields it fills.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data
}

/// Puts `notification`'s balloon up, adding the icon first when it is not in
/// the notification area; a rejected modification (the shell restarted and
/// forgot the icon) falls back to adding it.
fn show_balloon(local: &mut Local, notification: &Notification) {
    let mut data = base_data(local.hwnd);
    data.uFlags = NIF_MESSAGE | NIF_INFO | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = local.icon;
    data.szTip = truncate_utf16::<TIP_CAPACITY>(&notification.title);
    data.szInfoTitle = truncate_utf16::<TITLE_CAPACITY>(&notification.title);
    data.szInfo = truncate_utf16::<BODY_CAPACITY>(&notification.flat_body());
    data.dwInfoFlags = info_flags(notification.sound);
    // SAFETY: `data` is fully initialised above.
    let modified = local.shown && unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0;
    if !modified {
        // SAFETY: as above.
        local.shown = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
    }
    local.current = local.shown.then_some(notification.id);
}

/// Takes the icon, and with it the balloon, out of the notification area.
fn remove_icon(local: &mut Local) {
    if local.shown {
        let data = base_data(local.hwnd);
        // SAFETY: `data` names the icon by window and id only, which is all
        // `NIM_DELETE` reads.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
        local.shown = false;
    }
    local.current = None;
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TRAY => {
            // The handler runs with nothing borrowed: it may call back into
            // the notifier.
            let call = with_local(hwnd, |local| {
                let reaction = react(lparam as u32, local.current);
                if reaction.remove_icon {
                    remove_icon(local);
                }
                reaction.event.map(|event| (event, Arc::clone(&local.handler)))
            });
            if let Some((event, handler)) = call.flatten() {
                handler(event);
            }
            0
        }
        WM_COMMANDS => {
            drain_commands(hwnd);
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: forwards the unchanged arguments of this call.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn drain_commands(hwnd: HWND) {
    let Some(queue) = with_local(hwnd, |local| Arc::clone(&local.queue)) else { return };
    for command in queue.drain() {
        match command {
            Command::Quit => {
                with_local(hwnd, remove_icon);
                // SAFETY: the window belongs to this thread.
                unsafe { DestroyWindow(hwnd) };
                return;
            }
            Command::Show(notification) => {
                with_local(hwnd, |local| show_balloon(local, &notification));
            }
            Command::Close(id) => {
                with_local(hwnd, |local| {
                    if local.current == Some(id) {
                        remove_icon(local);
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(units: &[u16]) -> String {
        let end = units.iter().position(|&unit| unit == 0).expect("the array is zero-terminated");
        String::from_utf16(&units[..end]).expect("truncation keeps valid UTF-16")
    }

    #[test]
    fn short_text_is_kept_and_terminated() {
        let out = truncate_utf16::<8>("abc");
        assert_eq!(out, [97, 98, 99, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn long_text_is_cut_to_capacity_minus_terminator() {
        let out = truncate_utf16::<4>("abcdef");
        assert_eq!(text(&out), "abc");
        assert_eq!(out[3], 0);
    }

    #[test]
    fn exact_fit_keeps_everything() {
        assert_eq!(text(&truncate_utf16::<4>("abc")), "abc");
    }

    #[test]
    fn surrogate_pairs_are_never_split() {
        // Each emoji is two units: three fit in 7 usable units only as three pairs = 6.
        let out = truncate_utf16::<8>("😀😀😀😀");
        assert_eq!(text(&out), "😀😀😀");
        // One unit left over is not enough for a pair, nor is it filled.
        assert_eq!(out[6], 0);
        assert_eq!(out[7], 0);
    }

    #[test]
    fn multibyte_text_is_cut_at_character_boundary() {
        assert_eq!(text(&truncate_utf16::<3>("héllo")), "hé");
    }

    #[test]
    fn empty_text_and_tiny_arrays_are_safe() {
        assert_eq!(truncate_utf16::<4>(""), [0; 4]);
        assert_eq!(truncate_utf16::<1>("abc"), [0]);
        assert_eq!(truncate_utf16::<0>("abc"), [0u16; 0]);
    }

    #[test]
    fn real_capacities_hold_the_shell_limits() {
        let long = "x".repeat(1000);
        assert_eq!(text(&truncate_utf16::<TITLE_CAPACITY>(&long)).len(), 63);
        assert_eq!(text(&truncate_utf16::<BODY_CAPACITY>(&long)).len(), 255);
    }

    #[test]
    fn silent_notifications_carry_the_no_sound_flag() {
        assert_eq!(info_flags(true) & NIIF_NOSOUND, 0);
        assert_eq!(info_flags(false) & NIIF_NOSOUND, NIIF_NOSOUND);
        assert_eq!(info_flags(true) & NIIF_USER, NIIF_USER);
    }

    #[test]
    fn queue_drains_in_order_and_empties() {
        let queue = CommandQueue::default();
        assert!(queue.drain().is_empty());
        queue.push(Command::Close(NotificationId(1)));
        queue.push(Command::Show(Notification::new(NotificationId(2)).title("t")));
        queue.push(Command::Quit);
        let drained = queue.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0], Command::Close(NotificationId(1)));
        assert_eq!(drained[2], Command::Quit);
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn click_reports_the_current_notification_and_removes_the_icon() {
        let reaction = react(NIN_BALLOONUSERCLICK, Some(NotificationId(7)));
        assert!(reaction.remove_icon);
        assert_eq!(reaction.event, Some(NotificationEvent::Clicked(NotificationId(7))));
    }

    #[test]
    fn hide_reports_dismissed_and_timeout_reports_nothing() {
        let hidden = react(NIN_BALLOONHIDE, Some(NotificationId(7)));
        assert!(hidden.remove_icon);
        assert_eq!(hidden.event, Some(NotificationEvent::Dismissed(NotificationId(7))));
        let timed_out = react(NIN_BALLOONTIMEOUT, Some(NotificationId(7)));
        assert!(timed_out.remove_icon);
        assert_eq!(timed_out.event, None);
    }

    #[test]
    fn messages_without_a_current_notification_produce_no_event() {
        assert_eq!(react(NIN_BALLOONUSERCLICK, None).event, None);
        assert_eq!(react(NIN_BALLOONHIDE, None).event, None);
    }

    #[test]
    fn other_shell_messages_are_ignored() {
        let reaction = react(0x0200, Some(NotificationId(7)));
        assert!(!reaction.remove_icon);
        assert_eq!(reaction.event, None);
    }
}
