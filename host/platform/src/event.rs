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
    /// A live-reload native callback is waiting at the host safe point.
    ///
    /// This has its own coalescing slot so an animation frame already waiting
    /// in the event loop cannot hide the callback's redraw request.
    HotReloadCallbackReady,
    /// An editing shortcut the macOS menu bar claimed before the window could
    /// see it, handed back to the widget tree. See [`crate::menu`].
    MenuShortcut(crate::menu::MenuShortcut),
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
}

pub static EVENT_PROXY: OnceLock<EventLoopProxy<AimerNativePlatformEvent>> = OnceLock::new();
