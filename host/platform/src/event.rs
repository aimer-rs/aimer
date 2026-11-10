//! The event the platform bridges send to the event loop, and the handles they
//! find it through.

use std::sync::OnceLock;

use aimer_events::text_editing::TextEditingDelta;
use winit::event_loop::EventLoopProxy;
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

/// The activity the generated Android entry point registered.
#[cfg(target_os = "android")]
pub static ANDROID_APP: OnceLock<AndroidApp> = OnceLock::new();

#[derive(Debug, Clone)]
pub enum AimerNativePlatformEvent {
    /// The process was asked to terminate (`SIGTERM`, Ctrl-C, a closing
    /// terminal). The event loop exits in an orderly way, so the app's services
    /// are told and can save what they hold.
    ShutdownRequested,
    /// Pointer or hardware-key input from a platform bridge, in logical window
    /// coordinates. Text commits continue through the IME/text-editing path.
    NativeInput {
        event: aimer_events::element::ElementEvent,
        modifiers: aimer_events::element::Modifiers,
    },
    ForceBackspace,
    InsertText(String),
    /// Provisional text a soft keyboard is still composing.
    ///
    /// A multistage input method — Chinese Pinyin, Japanese Kana, Korean
    /// Hangul — shows the user what it has so far before any candidate is
    /// chosen. On desktop that arrives as `WindowEvent::Ime(Ime::Preedit)`;
    /// neither iOS nor Android reports it as a window event, so their keyboard
    /// shims report it here instead and it is dispatched down the very same
    /// path.
    ///
    /// `cursor` is the byte range of the active segment inside `text`, and an
    /// empty `text` ends the composition — the same contract winit's
    /// [`Ime::Preedit`](winit::event::Ime::Preedit) carries.
    SetPreedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    TextEditingDelta(TextEditingDelta),
    FrameReady,
    /// A bounded render completion or initialization result is ready for the UI.
    RenderWorkerReady,
    /// An editing shortcut the macOS menu bar claimed before the window could
    /// see it, handed back to the widget tree. See [`crate::menu`].
    MenuShortcut(crate::menu::MenuShortcut),
    /// An item of the application's own menu was chosen, by click or by its
    /// accelerator. See [`aimer_shortcut::menu`].
    MenuItem(aimer_shortcut::menu::MenuItemId),
    /// A global (system-wide) shortcut fired. Backends hear the key on threads
    /// of their own; the event loop runs the action. See
    /// [`aimer_shortcut::global`].
    GlobalShortcut(aimer_shortcut::global::GlobalShortcutId),
    /// The user clicked a system tray icon or chose one of its menu items.
    /// Trays report from threads of their own on Windows and Linux; the event
    /// loop runs the handler. See [`crate::tray`].
    Tray {
        id: crate::tray::TrayId,
        event: crate::tray::TrayEvent,
    },
    /// The user answered a system notification, or push registration reported.
    /// The platform reports from threads of its own; the event loop runs the
    /// handler. See [`crate::notification`].
    Notification {
        id: crate::notification::NotifierId,
        event: crate::notification::NotificationEvent,
    },
    /// The user asked the running application to show itself again: a click
    /// on its Dock icon on macOS. `has_visible_windows` is what the platform
    /// sees on screen. See [`crate::reopen`].
    Reopen { has_visible_windows: bool },
    /// A step of the system's back, reported by a platform bridge that is not a
    /// key: Android's predictive back arrives from Kotlin as a start, progress
    /// and an end. See [`crate::system_back`].
    Back(crate::system_back::BackSignal),
    /// Links were posted from the platform (see [`crate::link_inbox`]).
    OpenLinks,
    /// The device's folds or hinges changed, reported by a platform bridge from
    /// a thread other than the event loop's (Android's `ScreenBridge`).
    ///
    /// The new value waits in `aimer_screen`'s inbox; handling this event
    /// applies it on the event-loop thread.
    ScreenChanged,
    /// One window of a multi-window host has a frame to draw.
    ///
    /// Where [`Self::FrameReady`] wakes the application as a whole, this names
    /// the window that asked, so the host draws that window and not every one.
    WindowFrameReady(winit::window::WindowId),
    /// Windows were queued to be opened or closed; the host drains the queue.
    WindowRequests,
    /// Assistive technology asked for the semantic tree or requested an action
    /// (see [`crate::accessibility`]).
    Accessibility,
    /// A tool attached to the app or detached from it.
    ///
    /// The new list waits in [`crate::attachment`]; handling this event is what
    /// makes the debug overlay show it. Only a debug build is ever told.
    AttachmentChanged,
    /// Someone asked for a picture of the window (see [`crate::capture`]).
    ///
    /// Handling it draws a full frame, which the renderer answers the request
    /// with: an app that is not changing would otherwise draw nothing to copy.
    CaptureRequested,
    /// An injected mouse or finger is about to do something, sent ahead of the
    /// input itself so the debug overlay can draw where it is. See
    /// [`crate::attachment::SyntheticPointer`].
    SyntheticPointer(crate::attachment::SyntheticPointer),
}

pub static EVENT_PROXY: OnceLock<EventLoopProxy<AimerNativePlatformEvent>> = OnceLock::new();

/// Makes global shortcuts reach the event loop.
///
/// A backend reports a press from whichever thread heard it; this posts it to
/// the loop, which owns the widget tree and runs the action. Call it once the
/// loop's proxy exists.
pub fn install_global_shortcut_notifier() {
    aimer_shortcut::global::set_notifier(|id| {
        if let Some(proxy) = EVENT_PROXY.get() {
            let _ = proxy.send_event(AimerNativePlatformEvent::GlobalShortcut(id));
        }
    });
}
