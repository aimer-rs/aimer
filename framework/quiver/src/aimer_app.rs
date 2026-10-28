use std::cell::Cell;
#[cfg(feature = "wasm-hot-reload")]
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use aimer_attribute::size::ResolvedSize;
use aimer_cupid::AntiAlias;
#[cfg(any(target_os = "ios", target_os = "android"))]
use aimer_events::text_editing::NativeTextRange;
use aimer_events::text_editing::TextEditingDelta;
use aimer_modal::ModalHost;
use aimer_rubick::UiMemory;
use aimer_utils::info;
use aimer_venus::Venus;
use aimer_widget::Widget;
use aimer_widget::base::WindowHandle;
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Runtime;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};

use crate::window_attr::WindowAttr;
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

use crate::handler::event_handler::{HeadlessEventAction, WindowEventHandler};
use crate::handler::{AimerApplicationHandler, StartupHook, WindowRenderTree};
#[cfg(feature = "wasm-hot-reload")]
use crate::hot_reload::{LiveReloadConfig, LiveReloadHost};
#[cfg(feature = "wasm-hot-reload")]
use crate::reload_protocol::SessionCredentials;
use crate::render_ctx::AimerRenderContext;

static APP_STARTED: AtomicBool = AtomicBool::new(false);

pub use aimer_platform::AimerNativePlatformEvent;
#[cfg(target_os = "android")]
pub use aimer_platform::ANDROID_APP;
pub use aimer_platform::EVENT_PROXY;

/// The reason a pending frame was scheduled, kept private to the event loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameRequestKind {
    /// Scroll input arrived and nothing else wants the frame. When the input
    /// smoother has nothing left to deliver the frame can be dropped.
    ScrollOnly,
    /// The scroll engine asked for the frame that continues its own motion.
    /// Nothing but scrolling wants it, but the motion is advanced by drawing,
    /// so it is never dropped.
    ScrollPhysics,
    /// Only elements that can be revisited alone want this frame: a compositor
    /// animation advancing, or a state update rebuilding one subtree. Such a
    /// frame may skip the rest of the tree. It merges with every other reason
    /// into a full frame.
    Scoped,
    Full,
}

impl FrameRequestKind {
    pub(crate) fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Full, _) | (_, Self::Full) => Self::Full,
            (Self::Scoped, Self::Scoped) => Self::Scoped,
            // Scrolling moves content and an animation moves a node: a frame
            // that must do both cannot take either shortcut.
            (Self::Scoped, _) | (_, Self::Scoped) => Self::Full,
            (Self::ScrollPhysics, _) | (_, Self::ScrollPhysics) => Self::ScrollPhysics,
            _ => Self::ScrollOnly,
        }
    }

    /// Whether scrolling is the only reason for the frame.
    pub(crate) const fn is_scroll_driven(self) -> bool {
        matches!(self, Self::ScrollOnly | Self::ScrollPhysics)
    }

    const fn encode(self) -> u8 {
        match self {
            Self::ScrollOnly => 1,
            Self::Full => 2,
            Self::ScrollPhysics => 3,
            Self::Scoped => 4,
        }
    }

    const fn decode(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::ScrollOnly),
            2 => Some(Self::Full),
            3 => Some(Self::ScrollPhysics),
            4 => Some(Self::Scoped),
            _ => None,
        }
    }
}

aimer_tls::thread_local! {
    static CURRENT_FRAME_REQUEST_KIND: Cell<FrameRequestKind> = const {
        Cell::new(FrameRequestKind::Full)
    };
}

struct FrameRequestKindGuard(FrameRequestKind);

impl Drop for FrameRequestKindGuard {
    fn drop(&mut self) {
        CURRENT_FRAME_REQUEST_KIND.with(|current| current.set(self.0));
    }
}

pub(crate) fn with_frame_request_kind<T>(kind: FrameRequestKind, request: impl FnOnce() -> T) -> T {
    let previous = CURRENT_FRAME_REQUEST_KIND.with(|current| current.replace(kind));
    let _restore = FrameRequestKindGuard(previous);
    request()
}

pub(crate) fn current_frame_request_kind() -> FrameRequestKind {
    // Scroll physics asks for its continuation frames from inside the draw,
    // far from anything that could open a `request_scroll_frame` scope here.
    if aimer_events::window::is_scroll_frame_request() {
        return FrameRequestKind::ScrollPhysics;
    }
    if aimer_events::window::is_scoped_frame_request() {
        return FrameRequestKind::Scoped;
    }
    CURRENT_FRAME_REQUEST_KIND.with(Cell::get)
}

pub(crate) fn request_scroll_frame(request: impl FnOnce()) {
    with_frame_request_kind(FrameRequestKind::ScrollOnly, request);
}

pub(crate) fn observe_frame_request(slot: &Cell<Option<FrameRequestKind>>) {
    let kind = current_frame_request_kind();
    let kind = slot.get().map_or(kind, |pending| pending.merge(kind));
    slot.set(Some(kind));
    if kind == FrameRequestKind::Full {
        promote_pending_scroll_frame_request();
    }
}

/// Encoded redraw reason for a queued `FrameReady` event; zero means no event is pending.
///
/// Cursor movement can request a direct redraw while an animation event is
/// already queued. Rendering that redraw schedules another animation frame;
/// coalescing here prevents those requests from accumulating faster than the
/// event loop can deliver them.
static FRAME_READY_PENDING: AtomicU8 = AtomicU8::new(0);

/// Whether a live-reload native callback wake is waiting in the event loop.
///
/// Callback delivery and animation polling both end in a redraw, but they are
/// distinct wake sources: a pending animation request must not suppress the
/// wake that makes a freshly queued callback visible.
static CALLBACK_READY_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn try_begin_frame_ready_request(pending: &AtomicU8, kind: FrameRequestKind) -> bool {
    let requested = kind.encode();
    loop {
        let current = pending.load(Ordering::Acquire);
        if current == 0 {
            if pending
                .compare_exchange(0, requested, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        } else if let Some(current_kind) = FrameRequestKind::decode(current) {
            let merged = current_kind.merge(kind).encode();
            if merged == current {
                return false;
            }
            if pending
                .compare_exchange(current, merged, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return false;
            }
        } else {
            return false;
        }
    }
}

pub(crate) fn complete_frame_ready_request(pending: &AtomicU8) -> FrameRequestKind {
    FrameRequestKind::decode(pending.swap(0, Ordering::AcqRel)).unwrap_or(FrameRequestKind::Full)
}

pub(crate) fn promote_pending_scroll_frame_request() {
    for scroll_driven in [
        FrameRequestKind::ScrollOnly,
        FrameRequestKind::ScrollPhysics,
        FrameRequestKind::Scoped,
    ] {
        let _ = FRAME_READY_PENDING.compare_exchange(
            scroll_driven.encode(),
            FrameRequestKind::Full.encode(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn request_direct_frame(
    pending: &AtomicU8,
    kind: FrameRequestKind,
    request_redraw: impl FnOnce(),
) -> bool {
    if !try_begin_frame_ready_request(pending, kind) {
        return false;
    }
    request_redraw();
    true
}

fn try_begin_callback_ready_request(pending: &AtomicBool) -> bool {
    pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn complete_callback_ready_request(pending: &AtomicBool) {
    pending.store(false, Ordering::Release);
}

pub(crate) fn frame_ready_delivered() -> FrameRequestKind {
    let kind = complete_frame_ready_request(&FRAME_READY_PENDING);
    crate::frame_stats::record_display_tick();
    kind
}

pub(crate) fn callback_ready_delivered() {
    complete_callback_ready_request(&CALLBACK_READY_PENDING);
}

/// Asks the platform for the frame that continues whatever is unfinished.
///
/// Native platforms route the request through a `FrameReady` user event: iOS
/// coalesces a synchronous `request_redraw()` issued from inside the draw
/// cycle, and a worker finishing while the loop is parked needs an event-loop
/// wake. The browser is different: JavaScript promise callbacks run on the
/// event-loop owner thread while winit may be parked in `Wait`, so WebAssembly
/// requests the canvas redraw directly.
fn request_frame_ready() {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = aimer_events::window::get_window() {
        // Browser promise completions and animation ticks already run on the
        // JavaScript thread. Wake the canvas directly instead of sending a
        // user event through winit: the web loop may be parked in Wait even
        // though this callback is executing on its owner thread.
        let _ = request_direct_frame(&FRAME_READY_PENDING, current_frame_request_kind(), || {
            window.request_redraw()
        });
        return;
    }

    if !try_begin_frame_ready_request(&FRAME_READY_PENDING, current_frame_request_kind()) {
        crate::frame_stats::record_frame_request_coalesced();
        return;
    }
    crate::frame_stats::record_frame_request_accepted();
    let sent = EVENT_PROXY.get().is_some_and(|proxy| {
        proxy
            .send_event(AimerNativePlatformEvent::FrameReady)
            .is_ok()
    });
    if !sent {
        complete_frame_ready_request(&FRAME_READY_PENDING);
    }
}

/// Wakes the event loop for a native callback queued by a hot-reloaded tree.
///
/// This deliberately does not share [`FRAME_READY_PENDING`]. An animation can
/// already have a frame queued when a button callback arrives; sharing that
/// flag made the callback wait for the animation wake to be delivered before
/// the host safe point could process it.
#[cfg(feature = "wasm-hot-reload")]
fn request_callback_ready() {
    if !try_begin_callback_ready_request(&CALLBACK_READY_PENDING) {
        return;
    }
    let sent = EVENT_PROXY.get().is_some_and(|proxy| {
        proxy
            .send_event(AimerNativePlatformEvent::HotReloadCallbackReady)
            .is_ok()
    });
    if !sent {
        complete_callback_ready_request(&CALLBACK_READY_PENDING);
    }
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn trigger_rust_backspace() {
    let Some(proxy) = EVENT_PROXY.get() else {
        aimer_utils::debug!("trigger_rust_backspace: EVENT_PROXY not initialized yet");
        return;
    };

    if let Err(e) = proxy.send_event(AimerNativePlatformEvent::ForceBackspace) {
        aimer_utils::error!("trigger_rust_backspace: failed to send event: {:?}", e);
    }
}

// iOS frame scheduling: driven by a Swift `CADisplayLink` (see `main.swift`).
#[cfg(target_os = "ios")]
unsafe extern "C" {
    /// Pause the Swift `CADisplayLink` so it stops delivering vsync ticks while
    /// the app is idle.
    fn aimer_ios_pause_frames();
}

/// Called from Swift on every display-link vsync.
///
/// If a frame was requested since the last tick, forward a `FrameReady` through
/// the event loop, which renders directly on iOS. If nothing is pending,
/// pause the display link so the app does not render while idle. Mirrors the
/// `EVENT_PROXY` guard used by the other `trigger_rust_*` entry points.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_frame_tick() {
    if !aimer_events::window::take_frame_requested() {
        // No frame pending — idle the display link until the next request.
        unsafe {
            aimer_ios_pause_frames();
        }
        return;
    }

    let Some(proxy) = EVENT_PROXY.get() else {
        aimer_utils::debug!("aimer_ios_frame_tick: EVENT_PROXY not initialized yet");
        return;
    };

    if let Err(e) = proxy.send_event(AimerNativePlatformEvent::FrameReady) {
        aimer_utils::error!("aimer_ios_frame_tick: failed to send event: {:?}", e);
    }
}

#[cfg(target_os = "ios")]
fn dereference_ptr<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// Where the caret sits inside composing text reported by a soft keyboard.
///
/// Neither UIKit's marked text nor Android's composing span tells us which part
/// of the composition the input method considers active, so the caret is placed
/// at its end — the position the user is typing at, and where every soft
/// keyboard draws it. An empty composition has no caret at all, which is what
/// ends it.
#[cfg_attr(
    not(any(target_os = "ios", target_os = "android", test)),
    allow(dead_code)
)]
fn preedit_cursor(text: &str) -> Option<(usize, usize)> {
    if text.is_empty() {
        None
    } else {
        Some((text.len(), text.len()))
    }
}

/// Reports the text the iOS input method is still composing.
///
/// UIKit calls this "marked text": while a multistage keyboard — Pinyin, Kana,
/// Hangul — assembles a word, the provisional characters live in the hidden
/// text view's `markedTextRange` and are never committed. The Swift shim
/// forwards them here on every change, and a commit arrives separately through
/// [`trigger_rust_insert_text`]. Passing an empty string ends the composition.
///
/// # Safety
///
/// `ptr` must point at `len` initialized bytes that stay valid for the duration
/// of the call, exactly as [`trigger_rust_insert_text`] requires. A null
/// pointer is treated as an empty composition.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn trigger_rust_set_marked_text(ptr: *const u8, len: usize) {
    let text = if ptr.is_null() || len == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(dereference_ptr(ptr, len)).to_string()
    };

    let Some(proxy) = EVENT_PROXY.get() else {
        aimer_utils::debug!("trigger_rust_set_marked_text: EVENT_PROXY not initialized yet");
        return;
    };

    let cursor = preedit_cursor(&text);
    if let Err(e) = proxy.send_event(AimerNativePlatformEvent::SetPreedit { text, cursor }) {
        aimer_utils::error!(
            "trigger_rust_set_marked_text: failed to send event: {:?}",
            e
        );
    }
}

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn trigger_rust_insert_text(ptr: *const u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }

    let bytes = dereference_ptr(ptr, len);
    let text = String::from_utf8_lossy(bytes).to_string();

    let Some(proxy) = EVENT_PROXY.get() else {
        aimer_utils::debug!(
            "trigger_rust_insert_text: EVENT_PROXY not initialized yet (len={})",
            len
        );
        return;
    };

    if let Err(e) = proxy.send_event(AimerNativePlatformEvent::InsertText(text)) {
        aimer_utils::error!("trigger_rust_insert_text: failed to send event: {:?}", e);
    }
}

/// Reports one revisioned edit from the mirrored iOS text view.
///
/// # Safety
///
/// `text_ptr` must address `text_len` initialized UTF-8 bytes for the duration
/// of this call. A null pointer is accepted only when `text_len` is zero.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn trigger_rust_text_editing_delta(
    session_id: u64,
    revision: u64,
    replace_start: usize,
    replace_end: usize,
    text_ptr: *const u8,
    text_len: usize,
    selection_start: usize,
    selection_end: usize,
    composing_start: isize,
    composing_end: isize,
) {
    if text_ptr.is_null() && text_len != 0 {
        return;
    }
    let replacement_text = if text_len == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(dereference_ptr(text_ptr, text_len)).into_owned()
    };
    let composing = (composing_start >= 0 && composing_end >= 0)
        .then(|| NativeTextRange::new(composing_start as usize, composing_end as usize));
    let Some(proxy) = EVENT_PROXY.get() else {
        return;
    };
    let _ = proxy.send_event(AimerNativePlatformEvent::TextEditingDelta(
        TextEditingDelta {
            session_id,
            revision,
            replacement: NativeTextRange::new(replace_start, replace_end),
            replacement_text,
            selection: NativeTextRange::new(selection_start, selection_end),
            composing,
        },
    ));
}

// Android software-keyboard forwarding into Rust.
//
// These are the JNI entry points invoked by the Kotlin `com.aimer.AimerActivity`
// helper (see the Android build template). The hidden `EditText` managed by
// that activity captures everything the soft keyboard produces — including
// IME-composed CJK text once a candidate is committed — and forwards it here.
// The text is then pushed through the same platform-agnostic
// `AimerCustomAppEvent` path used by iOS, so the focused text field inserts the
// characters exactly once.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_AimerActivity_nativeInsertText<'caller>(
    mut env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    text: jni::objects::JString<'caller>,
) {
    env.with_env(|env| -> Result<(), jni::errors::Error> {
        let text = String::from(text.mutf8_chars(env)?);
        if text.is_empty() {
            return Ok(());
        }

        let Some(proxy) = EVENT_PROXY.get() else {
            aimer_utils::debug!("nativeInsertText: EVENT_PROXY not initialized yet");
            return Ok(());
        };

        if let Err(e) = proxy.send_event(AimerNativePlatformEvent::InsertText(text)) {
            aimer_utils::error!("nativeInsertText: failed to send event: {:?}", e);
        }
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// A link the platform opened the app with, from `onCreate` or `onNewIntent`.
///
/// Called on the JVM's UI thread, possibly before the event loop exists, so the
/// link is posted to a thread-safe inbox that the loop drains on its own thread.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_AimerActivity_nativeOpenLink<'caller>(
    mut env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    url: jni::objects::JString<'caller>,
) {
    env.with_env(|env| -> Result<(), jni::errors::Error> {
        let url = String::from(url.mutf8_chars(env)?);
        crate::link_inbox::post_link(url);
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// Android's back, in the pieces predictive back reports it in.
///
/// `phase` is `0` started, `1` progressed (with `progress` in `0.0..=1.0`), `2`
/// invoked and `3` cancelled, matching the constants in `AimerActivity`. It is
/// called on the JVM's UI thread, so it only queues the signal: the app's
/// back handlers live on the event-loop thread.
/// The display's folds and hinges, from `ScreenBridge` (see `aimer_screen`'s
/// `android/` directory).
///
/// `encoded` is `aimer_screen`'s wire form; an empty string means no features.
/// Called on the JVM's UI thread, so it posts the list to a thread-safe inbox and
/// wakes the event loop, which applies it on its own thread.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_ScreenBridge_nativeDisplayFeatures<'caller>(
    mut env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    encoded: jni::objects::JString<'caller>,
) {
    env.with_env(|env| -> Result<(), jni::errors::Error> {
        let encoded = String::from(encoded.mutf8_chars(env)?);
        aimer_screen::post_display_features(aimer_screen::decode_display_features(&encoded));
        let Some(proxy) = EVENT_PROXY.get() else {
            // Applied by the first `ScreenChanged` or refresh after start-up.
            aimer_utils::debug!("nativeDisplayFeatures: EVENT_PROXY not initialized yet");
            return Ok(());
        };
        if let Err(e) = proxy.send_event(AimerNativePlatformEvent::ScreenChanged) {
            aimer_utils::error!("nativeDisplayFeatures: failed to send event: {:?}", e);
        }
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// Wakes the event loop for a screen change UIKit reported from a callback of
/// its own (the hinge), so it applies what `aimer_screen` posted.
///
/// Registered with `aimer_screen::set_change_waker`. UIKit calls on the main
/// thread, which is the event-loop thread, but outside the loop's own handling,
/// so the change is queued like any other event and not applied in place.
#[cfg(target_os = "ios")]
pub(crate) fn wake_for_screen_change() {
    let Some(proxy) = EVENT_PROXY.get() else {
        // Applied by the first `ScreenChanged` or refresh after start-up.
        return;
    };
    if let Err(e) = proxy.send_event(AimerNativePlatformEvent::ScreenChanged) {
        aimer_utils::error!("wake_for_screen_change: failed to send event: {:?}", e);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_AimerActivity_nativeBack<'caller>(
    _env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    phase: i32,
    progress: f32,
) {
    use crate::handler::system_back::BackSignal;
    let signal = match phase {
        0 => BackSignal::Started,
        1 => BackSignal::Progressed(if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 }),
        2 => BackSignal::Invoked,
        3 => BackSignal::Cancelled,
        _ => return,
    };
    let Some(proxy) = EVENT_PROXY.get() else {
        aimer_utils::debug!("nativeBack: EVENT_PROXY not initialized yet");
        return;
    };
    if let Err(e) = proxy.send_event(AimerNativePlatformEvent::Back(signal)) {
        aimer_utils::error!("nativeBack: failed to send event: {:?}", e);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_AimerActivity_nativeTextEditingDelta<'caller>(
    mut env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    session_id: i64,
    revision: i64,
    replace_start: i32,
    replace_end: i32,
    text: jni::objects::JString<'caller>,
    selection_start: i32,
    selection_end: i32,
    composing_start: i32,
    composing_end: i32,
) {
    env.with_env(|env| -> Result<(), jni::errors::Error> {
        if session_id < 0
            || revision < 0
            || replace_start < 0
            || replace_end < 0
            || selection_start < 0
            || selection_end < 0
        {
            return Ok(());
        }
        let replacement_text = String::from(text.mutf8_chars(env)?);
        let composing = (composing_start >= 0 && composing_end >= 0)
            .then(|| NativeTextRange::new(composing_start as usize, composing_end as usize));
        if let Some(proxy) = EVENT_PROXY.get() {
            let _ = proxy.send_event(AimerNativePlatformEvent::TextEditingDelta(
                TextEditingDelta {
                    session_id: session_id as u64,
                    revision: revision as u64,
                    replacement: NativeTextRange::new(replace_start as usize, replace_end as usize),
                    replacement_text,
                    selection: NativeTextRange::new(
                        selection_start as usize,
                        selection_end as usize,
                    ),
                    composing,
                },
            ));
        }
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// Reports the text the Android input method is still composing.
///
/// The hidden `EditText` the activity owns marks its composing region with
/// `Spanned::SPAN_COMPOSING`; the Kotlin side reads that span out on every
/// change and passes it here, so the focused field paints the provisional text
/// instead of showing nothing until a candidate is chosen. An empty string
/// ends the composition.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_AimerActivity_nativeSetComposingText<'caller>(
    mut env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
    text: jni::objects::JString<'caller>,
) {
    env.with_env(|env| -> Result<(), jni::errors::Error> {
        let text = String::from(text.mutf8_chars(env)?);

        let Some(proxy) = EVENT_PROXY.get() else {
            aimer_utils::debug!("nativeSetComposingText: EVENT_PROXY not initialized yet");
            return Ok(());
        };

        let cursor = preedit_cursor(&text);
        if let Err(e) = proxy.send_event(AimerNativePlatformEvent::SetPreedit { text, cursor }) {
            aimer_utils::error!("nativeSetComposingText: failed to send event: {:?}", e);
        }
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_aimer_AimerActivity_nativeBackspace<'caller>(
    mut env: jni::EnvUnowned<'caller>,
    _class: jni::objects::JClass<'caller>,
) {
    env.with_env(|_env| -> Result<(), jni::errors::Error> {
        let Some(proxy) = EVENT_PROXY.get() else {
            aimer_utils::debug!("nativeBackspace: EVENT_PROXY not initialized yet");
            return Ok(());
        };

        if let Err(e) = proxy.send_event(AimerNativePlatformEvent::ForceBackspace) {
            aimer_utils::error!("nativeBackspace: failed to send event: {:?}", e);
        }
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// Configures and starts an Aimer application.
///
/// Construct an application with [`Self::new`], apply configuration, and call
/// [`Self::child`] last to produce a runnable application. Existing static
/// start functions remain available and use the default configuration.
pub struct AimerApp<W = ()> {
    child: W,
    antialiasing: AntiAlias,
    render_mode: crate::RenderMode,
    ui_memory_limit: usize,
    startup_hooks: Vec<StartupHook>,
    window_attr: WindowAttr,
    exit_policy: crate::ExitPolicy,
    #[cfg(feature = "wasm-hot-reload")]
    live_reload: Option<LiveReloadLaunch>,
}

#[cfg(feature = "wasm-hot-reload")]
struct LiveReloadLaunch {
    address: SocketAddr,
    credentials: SessionCredentials,
    config: LiveReloadConfig,
}

#[cfg(feature = "wasm-hot-reload")]
fn reload_listener_readiness_line(
    session_id: [u8; 16],
    address: SocketAddr,
    process_id: u32,
) -> String {
    use std::fmt::Write;

    let mut encoded_session = String::with_capacity(32);
    for byte in session_id {
        write!(&mut encoded_session, "{byte:02x}").expect("writing to a string cannot fail");
    }
    format!(
        "AIMER_RELOAD_LISTENER_READY session={encoded_session} port={} pid={process_id} protocol=1.0",
        address.port(),
    )
}

/// The setup the framework itself needs before a native window appears.
///
/// Only the platform loop runs these: they install native objects — the macOS
/// application menu — which need a real application behind them and mean
/// nothing to an application running without a window.
fn platform_startup_hooks() -> Vec<StartupHook> {
    #[cfg(target_os = "macos")]
    {
        vec![Box::new(|| Box::new(crate::menu::install_macos_menu()))]
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// The hooks a native run performs, framework setup first.
///
/// The platform's own setup runs before anything the application registered,
/// so a callback that needs the native application to exist finds it there.
fn native_startup_hooks(application_hooks: Vec<StartupHook>) -> Vec<StartupHook> {
    let mut hooks = platform_startup_hooks();
    hooks.extend(application_hooks);
    hooks
}
/// Mocked display properties used by a headless application.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadlessOptions {
    pub size: PhysicalSize<u32>,
    pub scale_factor: f64,
}

impl Default for HeadlessOptions {
    fn default() -> Self {
        Self {
            size: PhysicalSize::new(1150, 800),
            scale_factor: 1.0,
        }
    }
}

/// A running Aimer application that builds, lays out, draws, and handles events
/// without creating a native window or a `winit` event loop.
///
/// The application is the same one the platform loop runs: events go through
/// the same handlers, frames through the same drawer, and a frame is asked for
/// on exactly the same conditions. What a window would do — present the frame,
/// deliver the request for the next one — is done by whoever owns this value,
/// by calling [`render_frame`](Self::render_frame) or
/// [`pump_frames`](Self::pump_frames).
pub struct HeadlessAimerApp<W: Widget + 'static> {
    app: AimerApplicationHandler<W>,
    canvas: aimer_canvas::InnerCanvas,
    window: WindowHandle,
    size: PhysicalSize<u32>,
    exit_requested: bool,
    /// The frame requester that was installed for this thread before this
    /// application took it over, put back when the application is dropped.
    previous_frame_requester: Option<std::rc::Rc<dyn Fn()>>,
    /// The redraw observer installed for this thread, restored when this app is
    /// dropped so later headless apps receive their own redraw reasons.
    previous_redraw_observer: Option<Rc<dyn Fn()>>,
    /// The UI-thread runtime that was installed for this thread before this
    /// application took it over, put back when the application is dropped.
    previous_runtime: Option<std::rc::Rc<Venus>>,
    /// How this window asks for frames while another window of the thread is
    /// the active one. Dropped with the application.
    _window_scope: aimer_events::window::WindowScopeRegistration,
    #[cfg(test)]
    last_frame_result: Option<(f32, aimer_cupid::damage_region::DamageSet)>,
    /// The runtime the application's tasks run on; dropped after the rest.
    #[cfg(not(target_arch = "wasm32"))]
    _async_runtime: Runtime,
}

impl<W: Widget + 'static> HeadlessAimerApp<W> {
    fn new(
        widget: W,
        options: HeadlessOptions,
        antialiasing: AntiAlias,
        startup_hooks: Vec<StartupHook>,
        ui_memory_limit: usize,
        #[cfg(feature = "wasm-hot-reload")] live_reload: Option<LiveReloadLaunch>,
    ) -> HeadlessAimerApp<W> {
        let scale_factor = if options.scale_factor.is_finite() && options.scale_factor > 0.0 {
            options.scale_factor
        } else {
            1.0
        };

        #[cfg(not(target_arch = "wasm32"))]
        let async_runtime = Runtime::new().expect("Failed to create async runtime");

        // A headless application is the UI thread for as long as it lives, so
        // its runtime is the one a callback deep in the tree reaches for. The
        // one that was installed before is remembered rather than overwritten,
        // because a test may well be running an application inside another.
        let venus = Venus::new();
        let previous_runtime = Venus::uninstall();
        venus.install();

        // Every task Venus polls is polled inside the async runtime, so a
        // future that builds its resources on the first poll — a request, a
        // timer, a file read — finds a runtime there instead of panicking.
        #[cfg(not(target_arch = "wasm32"))]
        venus.set_poll_context(crate::poll_context::TokioPollContext::new(
            async_runtime.handle().clone(),
        ));
        #[cfg(target_arch = "wasm32")]
        venus.set_poll_context(crate::browser_poll_context::BrowserPollContext);

        let window = WindowHandle::headless(options.size, scale_factor);
        let frame_request_reason = Rc::new(Cell::new(None));
        #[cfg(feature = "wasm-hot-reload")]
        let live_reload = live_reload.map(|launch| {
            let wake_window = window.clone();
            let wake_reason = frame_request_reason.clone();
            LiveReloadHost::bind(
                launch.address,
                launch.credentials,
                launch.config,
                Rc::clone(venus.scheduler()),
                move || {
                    wake_reason.set(Some(FrameRequestKind::Full));
                    wake_window.request_redraw();
                },
            )
            .expect("failed to start configured live reload listener")
        });
        // The thread-wide requester below answers for the application created
        // last. Registering the window as well is what lets every entry point
        // make *this* window the active one, so two applications on one thread
        // each get their own frame requests.
        let window_scope = aimer_events::window::register_window_scope_with(
            window.key(),
            aimer_events::window::WindowScopeConfig::new(
                {
                    let window = window.clone();
                    move || window.request_redraw()
                },
                {
                    let frame_request_reason = frame_request_reason.clone();
                    move || observe_frame_request(&frame_request_reason)
                },
            )
            .locals(Rc::new(crate::window_locals::WindowThreadState::default())),
        );
        let mut headless = Self {
            app: AimerApplicationHandler {
                window: Some(window.clone()),
                hosted: false,
                closed: false,
                inherited_states: None,
                macos_windowing: Default::default(),
                render_ctx: AimerRenderContext::new(antialiasing),
                ui_memory: UiMemory::new(ui_memory_limit),
                #[cfg(debug_assertions)]
                debug_overlay: Default::default(),
                window_attr: WindowAttr::new(),
                render_tree: WindowRenderTree::default(),
                #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
                show_window_after_first_frame: false,
                widget_root: None,
                event_dispatcher: aimer_widget::EventDispatcher::new(),
                scroll_smoother: crate::handler::scroll_classifier::DualScroller::new(),
                frame_request_reason: frame_request_reason.clone(),
                #[cfg(target_arch = "wasm32")]
                web_scroll_phase: crate::handler::web_scroll_phase::WebScrollPhase::new(),
                pending_widget: Some(widget),
                cursor_pos: crate::handler::event_handler::CURSOR_OUTSIDE_POSITION,
                touch_pointer: false,
                pressed_button: None,
                current_modifiers: Default::default(),
                ime_composing: false,
                window_scale: scale_factor,
                native_window_size: None,
                pending_resize: None,
                startup_hooks,
                startup_resources: Vec::new(),
                #[cfg(not(target_arch = "wasm32"))]
                async_handle: async_runtime.handle().clone(),
                active_touch_id: None,
                venus,
                file_drag: crate::handler::file_drag::FileDrag::new(),
                #[cfg(feature = "wasm-hot-reload")]
                live_reload,
            },
            canvas: aimer_canvas::InnerCanvas::new(),
            window: window.clone(),
            size: options.size,
            exit_requested: false,
            // A widget schedules its next frame through the platform requester:
            // a state update, an animation step, an overlay that just opened.
            // Without a platform there is nothing to schedule through, so the
            // requests are taken here and land on this application's window,
            // just as they land on a real one.
            previous_frame_requester: aimer_events::window::set_thread_redraw_requester(
                move || window.request_redraw(),
            ),
            previous_redraw_observer: aimer_events::window::set_thread_redraw_observer({
                let frame_request_reason = frame_request_reason.clone();
                move || observe_frame_request(&frame_request_reason)
            }),
            previous_runtime,
            _window_scope: window_scope,
            #[cfg(test)]
            last_frame_result: None,
            // Last, so it outlives everything that holds a handle to it.
            #[cfg(not(target_arch = "wasm32"))]
            _async_runtime: async_runtime,
        };

        // The windowed application runs its setup the moment the platform loop
        // resumes, before the first window exists. Construction is the same
        // moment here: an application is running from the point it is handed
        // back, so whatever its setup produces is already in place.
        crate::handler::run_startup_hooks(
            &mut headless.app.startup_hooks,
            &mut headless.app.startup_resources,
        );
        // A headless setup may install macOS window customization through the
        // same public hook, but there is no native window to consume it. Clear
        // that thread-local value so it cannot leak into a later windowed run.
        drop(aimer_native::macos_windowing::take_pending());

        // An application that has just come up owes the screen a frame, and the
        // windowed loop asks for it the moment its window exists. Starting with
        // the same request pending is what lets `pump_frames` draw the first
        // frame without being told to.
        headless.window.request_redraw();

        headless
    }

    /// Makes this application the one the thread answers for until the guard
    /// is dropped: frame requests made meanwhile land on its window, and the
    /// runtime that is current is its own.
    ///
    /// Every way into the application does this first. A second application on
    /// the same thread would otherwise be built, and asked for frames, under
    /// whichever one was created last.
    fn activate(&self) -> ActiveHeadless {
        let window = aimer_events::window::enter_window_scope(self.window.key());
        let runtime_is_current = Venus::current()
            .is_some_and(|current| Rc::ptr_eq(&current, &self.app.venus));
        let previous_runtime = if runtime_is_current {
            None
        } else {
            let previous = Venus::uninstall();
            self.app.venus.install();
            Some(previous)
        };
        ActiveHeadless {
            _window: window,
            previous_runtime,
        }
    }

    /// Builds and draws one frame into the non-presenting in-memory canvas.
    ///
    /// A frame does what a windowed frame does: this frame's share of a scroll
    /// gesture is delivered, a resize the surface has not caught up with is
    /// applied, and the tree is drawn through the shared frame drawer when the
    /// scroll tick or other work requires it. An unchanged no-op scroll tick still
    /// advances the gesture without drawing the tree. An animation that is not
    /// finished asks for the next frame, which
    /// [`take_redraw_request`](Self::take_redraw_request) reports and
    /// [`pump_frames`](Self::pump_frames) acts on.
    pub fn render_frame(&mut self) {
        if self.exit_requested {
            return;
        }
        let _active = self.activate();
        #[cfg(test)]
        {
            self.last_frame_result = None;
        }

        // Drawing is the pending request being delivered, exactly as a window
        // clears it when it hands over `RedrawRequested`. Whatever this frame
        // asks for is a request for the *next* one.
        self.window.take_redraw_request();

        let kind = self.app.take_frame_request_reason();
        let preparation = self.app.begin_frame();
        let had_pending_resize = self.app.pending_resize.is_some();
        self.apply_pending_resize();

        let skip_draw = self
            .app
            .should_skip_scroll_frame(kind, &preparation, had_pending_resize);
        if !skip_draw {
            self.app
                .mark_scroll_only_frame(kind, &preparation, had_pending_resize);
            self.app
                .mark_animation_only_frame(kind, &preparation, had_pending_resize);
            let build = crate::frame_stats::PhaseTimer::start();
            let canvas = aimer_canvas::FrameCanvas::new(&self.canvas);
            canvas.begin_frame();

            let (width, height) = (self.size.width, self.size.height);
            let window = self.window.clone();
            let frame_result = self
                .app
                .split_for_frame(window)
                .1
                .draw(&self.canvas, width, height);
            #[cfg(test)]
            {
                self.last_frame_result = Some(frame_result);
            }
            #[cfg(not(test))]
            let _ = frame_result;
            build.finish(crate::frame_stats::FramePhase::Build);
        }

        self.app.end_frame();

        if !skip_draw {
            crate::first_frame::notify_first_frame_presented(true);
        }
    }

    /// Counts the mounted root's render nodes by paint source.
    ///
    /// `None` until a root exists. See [`crate::handler::census::PaintSourceCensus`].
    #[doc(hidden)]
    pub fn paint_source_census(&self) -> Option<crate::handler::census::PaintSourceCensus> {
        self.app.paint_source_census()
    }

    /// Every render node in paint order, as the renderer would be handed them
    /// without damage culling, for tests that check what a frame draws and where.
    #[doc(hidden)]
    pub fn render_ops(&self) -> Vec<aimer_cupid::draw_cmd_v2::RenderOp> {
        self.app.render_tree().render_all()
    }

    /// Turns the incremental render-tree synchronization and path-index patching
    /// on or off together. Off, every change that touches the tree is handled by
    /// rebuilding the whole structure, as it was before they existed. For
    /// benchmarks and for comparing the two.
    #[doc(hidden)]
    pub fn set_incremental_sync(&mut self, enabled: bool) {
        self.app.render_tree.scoped_sync_disabled = !enabled;
        self.app.event_dispatcher.set_index_patching(enabled);
    }

    /// How many render-tree synchronizations took each path so far, as
    /// `(full, scoped)`. A scoped one re-collected only a replaced subtree.
    #[doc(hidden)]
    pub fn sync_counts(&self) -> (u64, u64) {
        let counts = self.app.render_tree.sync_counts;
        (counts.full, counts.scoped)
    }

    /// Compares each element's cached interaction rectangle with its render
    /// node's world rectangle. See [`crate::handler::bounds_audit::BoundsAudit`].
    #[doc(hidden)]
    pub fn bounds_audit(&self) -> Option<crate::handler::bounds_audit::BoundsAudit> {
        self.app.bounds_audit()
    }

    /// Returns a handle to this application's UI memory pool.
    ///
    /// The handle exposes committed usage and the configured limit, and keeps
    /// the pool alive if allocations made by the application still use it.
    #[inline]
    pub fn ui_memory(&self) -> UiMemory {
        self.app.ui_memory.clone()
    }

    /// Renders frames for as long as the application keeps asking for them, up
    /// to `max_frames`, and reports how many were drawn.
    ///
    /// This is the headless stand-in for the platform's frame loop: an
    /// animation, a scroll gesture gliding to a stop, or a widget rebuilding
    /// itself all ask for the next frame the same way they do on a window, and
    /// this is what delivers it. The budget is what keeps a permanently
    /// animating tree from turning a test into a hang, so a caller that runs
    /// out of it can tell: the count equals `max_frames` and the request is
    /// still pending.
    pub fn pump_frames(&mut self, max_frames: usize) -> usize {
        let mut frames = 0;
        while frames < max_frames && self.take_redraw_request() && !self.exit_requested {
            self.render_frame();
            frames += 1;
        }
        frames
    }

    /// Adopts the size a resize event asked for.
    ///
    /// A window reconfigures its surface on the frame that answers the resize
    /// rather than while the event is being handled, and this is that frame.
    /// The metrics the tree reads were already brought up to date by the event
    /// itself, exactly as a platform window reports its new size the moment it
    /// has one.
    fn apply_pending_resize(&mut self) {
        if let Some(size) = self.app.pending_resize.take() {
            self.size = size;
        }
    }

    /// Delivers a `winit` window event to the headless application.
    ///
    /// The event travels the path the windowed loop uses, so a frame follows
    /// only where a window would have been sent one — which
    /// [`take_redraw_request`](Self::take_redraw_request) reports. A resize is
    /// the one event a window answers with a frame of its own, drawn before the
    /// compositor can stretch the old one, and it is answered that way here too.
    pub fn send_window_event(&mut self, event: WindowEvent) {
        let _active = self.activate();
        let action = WindowEventHandler::handle_headless_event(&mut self.app, event);
        match action {
            HeadlessEventAction::None => {}
            HeadlessEventAction::Render => self.render_frame(),
            HeadlessEventAction::Exit => self.exit_requested = true,
        }
    }

    /// Delivers an Aimer user event through the same path as the native event
    /// loop.
    pub fn send_user_event(&mut self, event: AimerNativePlatformEvent) {
        let _active = self.activate();
        crate::handler::user_events::handle_user_event(
            &mut self.app,
            event,
            crate::handler::user_events::request_window_redraw,
        );
    }

    /// Returns whether an input method currently holds an open composition.
    ///
    /// While it does, raw keystrokes are suppressed: the characters the user
    /// pressed belong to the composition, not to the field, and arrive only
    /// once a candidate is committed.
    pub fn is_composing(&self) -> bool {
        self.app.ime_composing
    }

    pub fn physical_size(&self) -> PhysicalSize<u32> {
        self.size
    }

    pub fn logical_size(&self) -> ResolvedSize {
        ResolvedSize {
            width: self.size.width as f32 / self.app.window_scale as f32,
            height: self.size.height as f32 / self.app.window_scale as f32,
        }
    }

    pub fn scale_factor(&self) -> f64 {
        self.app.window_scale
    }

    pub fn has_native_window(&self) -> bool {
        self.app.native_window().is_some()
    }

    pub fn is_exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// Returns the cursor icon most recently selected by the widget tree.
    pub fn cursor_icon(&self) -> winit::window::CursorIcon {
        self.window
            .headless_cursor()
            .expect("a headless application always owns a headless window")
    }

    /// Returns and clears whether application code requested another frame.
    pub fn take_redraw_request(&self) -> bool {
        self.window.take_redraw_request()
    }

    /// Returns redraw coalescing counters for the headless display-tick
    /// driver.
    ///
    /// Headless rendering does not have a native event-loop wake, so its
    /// counters describe the equivalent request bit: one accepted request can
    /// represent many producers until the next rendered tick consumes it.
    /// Native applications use [`crate::frame_stats::frame_request_stats`]
    /// for the event-loop wake counters instead.
    #[doc(hidden)]
    pub fn frame_request_stats(&self) -> crate::frame_stats::FrameRequestStats {
        let (accepted, coalesced, display_ticks) = self
            .window
            .headless_redraw_request_counts()
            .unwrap_or_default();
        crate::frame_stats::FrameRequestStats {
            accepted,
            coalesced,
            display_ticks,
        }
    }

    /// Returns the debug listener endpoint selected for this application.
    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub fn live_reload_addr(&self) -> Option<SocketAddr> {
        self.app
            .live_reload
            .as_ref()
            .map(LiveReloadHost::local_addr)
    }

    /// Returns the interpreted generation visible to this application.
    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub fn live_reload_generation(&self) -> Option<aimer_anteros::GenerationId> {
        self.app
            .live_reload
            .as_ref()
            .and_then(LiveReloadHost::active_generation)
    }

    /// Returns the bounded diagnostic currently shown by the host reload
    /// overlay, if the latest candidate was rejected or a guest callback
    /// failed.
    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub fn live_reload_diagnostic(&self) -> Option<&str> {
        self.app
            .live_reload
            .as_ref()
            .and_then(LiveReloadHost::reload_diagnostic)
    }

    /// Exports the active interpreted state for development diagnostics.
    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub fn live_reload_state(&mut self) -> Result<Option<Vec<u8>>, aimer_anteros::RuntimeError> {
        match self.app.live_reload.as_mut() {
            Some(host) => host.export_active_state(),
            None => Ok(None),
        }
    }

    /// Returns the active root's framework debug name.
    #[inline]
    pub fn active_root_name(&self) -> Option<&'static str> {
        self.app.active_root().map(|root| root.debug_name())
    }

    #[cfg(all(test, feature = "wasm-hot-reload"))]
    pub(crate) fn active_element_bounds(
        &self,
        name: &str,
    ) -> Option<(
        aimer_attribute::position::Vec2d,
        aimer_attribute::position::Vec2d,
    )> {
        fn find(
            element: &dyn aimer_widget::Element,
            name: &str,
        ) -> Option<(
            aimer_attribute::position::Vec2d,
            aimer_attribute::position::Vec2d,
        )> {
            if element.debug_name() == name {
                return element.pos_start_end();
            }
            let mut bounds = None;
            element.event_children(&mut |child| {
                if bounds.is_none() {
                    bounds = find(child, name);
                }
            });
            bounds
        }

        self.app
            .active_root()
            .and_then(|root| find(root.as_ref(), name))
    }

    #[cfg(all(test, feature = "wasm-hot-reload"))]
    pub(crate) fn active_contains_element(&self, name: &str) -> bool {
        fn contains(element: &dyn aimer_widget::Element, name: &str) -> bool {
            if element.debug_name() == name {
                return true;
            }
            let mut found = false;
            element.event_children(&mut |child| {
                if !found {
                    found = contains(child, name);
                }
            });
            found
        }

        self.app
            .active_root()
            .is_some_and(|root| contains(root.as_ref(), name))
    }

    /// The UI-thread runtime this application's frames are scheduled by.
    ///
    /// A test drives asynchronous work the way the application does: spawn on
    /// this, then render a frame.
    #[inline]
    pub fn venus(&self) -> &std::rc::Rc<Venus> {
        &self.app.venus
    }
}

/// Keeps a headless application active; see [`HeadlessAimerApp::activate`].
struct ActiveHeadless {
    _window: aimer_events::window::WindowScopeGuard,
    /// The runtime to put back, if this application had to install its own.
    /// The inner `None` means there was no runtime installed before.
    previous_runtime: Option<Option<Rc<Venus>>>,
}

impl Drop for ActiveHeadless {
    fn drop(&mut self) {
        if let Some(previous) = self.previous_runtime.take() {
            Venus::uninstall();
            if let Some(previous) = previous {
                previous.install();
            }
        }
    }
}

impl<W: Widget + 'static> Drop for HeadlessAimerApp<W> {
    /// Stops taking the frame requests of this thread once the application is
    /// gone, so a later one — or none at all — receives them instead.
    fn drop(&mut self) {
        aimer_events::window::restore_thread_redraw_requester(self.previous_frame_requester.take());
        aimer_events::window::restore_thread_redraw_observer(self.previous_redraw_observer.take());
        aimer_widget::safe_area::forget_safe_area_of(&self.window);
        Venus::uninstall();
        if let Some(previous) = self.previous_runtime.take() {
            previous.install();
        }
    }
}

impl AimerApp {
    /// Creates an application builder using lightweight analytic antialiasing.
    #[inline]
    pub fn new() -> Self {
        #[cfg(feature = "wasm-hot-reload")]
        let live_reload =
            crate::hot_reload::take_hot_reload_config().map(|(address, credentials, config)| {
                LiveReloadLaunch {
                    address,
                    credentials,
                    config,
                }
            });
        Self {
            child: (),
            antialiasing: AntiAlias::default(),
            render_mode: crate::RenderMode::default(),
            ui_memory_limit: usize::MAX,
            startup_hooks: Vec::new(),
            window_attr: WindowAttr::new(),
            exit_policy: crate::ExitPolicy::default(),
            #[cfg(feature = "wasm-hot-reload")]
            live_reload,
        }
    }

    /// Selects the antialiasing strategy used by the Cupid renderer.
    #[inline]
    pub fn with_antialiasing(mut self, antialiasing: AntiAlias) -> Self {
        self.antialiasing = antialiasing;
        self
    }

    /// Selects native rendering ownership. The default is [`crate::RenderMode::Threaded`].
    ///
    /// Inline mode is useful when comparing profiles or debugging a renderer.
    /// Browser and headless rendering remain inline regardless of this setting.
    #[inline]
    pub fn render_mode(mut self, mode: crate::RenderMode) -> Self {
        self.render_mode = mode;
        self
    }

    /// Sets the maximum system memory committed by this application's UI
    /// heap. The heap grows lazily in 2 MiB regions when the limit allows it.
    /// WebAssembly retains grown linear-memory pages for the module lifetime.
    #[inline]
    pub fn ui_memory_limit(mut self, max_bytes: usize) -> Self {
        self.ui_memory_limit = max_bytes;
        self
    }

    /// Configures the native window created when the application starts.
    ///
    /// The attributes use Aimer's window API and are translated to the active
    /// platform backend only when the native event loop creates the window.
    /// Headless applications continue to use [`HeadlessOptions`] instead.
    #[inline]
    pub fn window(mut self, window_attr: WindowAttr) -> Self {
        self.window_attr = window_attr;
        self
    }

    /// Chooses what closing windows does to the application.
    ///
    /// The default ends the application when its last window closes. Only
    /// desktop platforms have more than one window; elsewhere the single window
    /// is the application whatever the policy says.
    #[inline]
    pub fn exit_policy(mut self, policy: crate::ExitPolicy) -> Self {
        self.exit_policy = policy;
        self
    }

    /// Enables the debug-only authenticated in-process reload listener.
    ///
    /// `config` carries explicit interpreter and resource ceilings. The child
    /// remains the native placeholder until the first authenticated module is
    /// committed at an application frame safe point.
    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub fn hot_reload(
        mut self,
        address: SocketAddr,
        credentials: SessionCredentials,
        config: LiveReloadConfig,
    ) -> Self {
        self.live_reload = Some(LiveReloadLaunch {
            address,
            credentials,
            config,
        });
        self
    }

    /// Registers a callback to run when the native event loop first resumes.
    ///
    /// Setup callbacks run once, in registration order, before Aimer creates
    /// its first window. The framework's platform setup runs before callbacks
    /// registered by the application.
    ///
    /// The returned value is retained until the application exits, allowing
    /// native resources created by the callback to remain alive for the full
    /// application lifecycle.
    ///
    /// macOS window customization can be registered here by constructing an
    /// [`aimer_native::macos_windowing::MacosWindowing`] value and calling its
    /// `install` method. That API is a no-op on other platforms, so the same
    /// setup callback can be compiled and used cross-platform.
    #[inline]
    pub fn setup<R: 'static>(mut self, setup: impl FnOnce() -> R + 'static) -> Self {
        self.startup_hooks.push(Box::new(move || Box::new(setup())));
        self
    }

    /// Installs the root widget and completes the application builder.
    #[inline]
    pub fn child<W: Widget + 'static>(self, child: W) -> AimerApp<W> {
        AimerApp {
            child,
            antialiasing: self.antialiasing,
            render_mode: self.render_mode,
            ui_memory_limit: self.ui_memory_limit,
            startup_hooks: self.startup_hooks,
            window_attr: self.window_attr,
            exit_policy: self.exit_policy,
            #[cfg(feature = "wasm-hot-reload")]
            live_reload: self.live_reload,
        }
    }

    /// Starts a native application with `widget` as its root widget.
    pub fn start<W: Widget + 'static>(widget: W) {
        Self::new().child(widget).run();
    }

    /// Starts a native application and runs `setup` once the platform event
    /// loop is ready.
    ///
    /// The value returned by `setup` is retained until the application exits.
    /// This allows platform resources such as macOS application menus to remain
    /// alive for the complete native application lifecycle.
    ///
    /// # Platform initialization
    ///
    /// The callback runs on the event-loop thread during the application's
    /// first resume, before Aimer creates its window. APIs that require an
    /// initialized native application or main-thread access should be called
    /// from this callback rather than before [`Self::start_with_setup`].
    pub fn start_with_setup<W, R>(widget: W, setup: impl FnOnce() -> R + 'static)
    where
        W: Widget + 'static,
        R: 'static,
    {
        Self::new().child(widget).run_with_setup(setup);
    }

    pub fn start_headless<W: Widget + 'static>(widget: W) -> HeadlessAimerApp<ModalHost<W>> {
        Self::new().child(widget).run_headless()
    }

    pub fn start_headless_with<W: Widget + 'static>(
        widget: W,
        options: HeadlessOptions,
    ) -> HeadlessAimerApp<ModalHost<W>> {
        Self::new().child(widget).run_headless_with(options)
    }
}

impl Default for AimerApp {
    fn default() -> Self {
        Self::new()
    }
}

impl<W> AimerApp<W> {
    /// Returns the selected mode; browser and headless drivers always execute inline.
    #[inline]
    pub fn configured_render_mode(&self) -> crate::RenderMode {
        self.render_mode
    }
}

impl<W: Widget + 'static> AimerApp<W> {
    /// Returns the configured antialiasing strategy.
    #[inline]
    pub fn antialiasing(&self) -> AntiAlias {
        self.antialiasing
    }

    /// Starts this configured application on the native event loop.
    pub fn run(self) {
        start_event_loop(
            ModalHost::new().child(self.child),
            native_startup_hooks(self.startup_hooks),
            self.antialiasing,
            self.render_mode,
            self.window_attr,
            self.ui_memory_limit,
            self.exit_policy,
            #[cfg(feature = "wasm-hot-reload")]
            self.live_reload,
        );
    }

    /// Starts this configured application and runs `setup` before its window is
    /// created, retaining the returned resource until shutdown.
    pub fn run_with_setup<R: 'static>(mut self, setup: impl FnOnce() -> R + 'static) {
        self.startup_hooks.push(Box::new(move || Box::new(setup())));
        start_event_loop(
            ModalHost::new().child(self.child),
            native_startup_hooks(self.startup_hooks),
            self.antialiasing,
            self.render_mode,
            self.window_attr,
            self.ui_memory_limit,
            self.exit_policy,
            #[cfg(feature = "wasm-hot-reload")]
            self.live_reload,
        );
    }

    /// Starts this configured application without creating a native window.
    pub fn run_headless(self) -> HeadlessAimerApp<ModalHost<W>> {
        self.run_headless_with(HeadlessOptions::default())
    }

    /// Starts this configured application headlessly with explicit display
    /// properties.
    pub fn run_headless_with(self, options: HeadlessOptions) -> HeadlessAimerApp<ModalHost<W>> {
        HeadlessAimerApp::new(
            ModalHost::new().child(self.child),
            options,
            self.antialiasing,
            self.startup_hooks,
            self.ui_memory_limit,
            #[cfg(feature = "wasm-hot-reload")]
            self.live_reload,
        )
    }
}

fn start_event_loop(
    widget: impl Widget + 'static,
    startup_hooks: Vec<StartupHook>,
    antialiasing: AntiAlias,
    render_mode: crate::RenderMode,
    window_attr: WindowAttr,
    ui_memory_limit: usize,
    exit_policy: crate::ExitPolicy,
    #[cfg(feature = "wasm-hot-reload")] live_reload: Option<LiveReloadLaunch>,
) {
    if APP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    // As early as the app controls: the signal mask only covers threads started
    // after it is set.
    #[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
    crate::shutdown_signals::install(|| {
        EVENT_PROXY
            .get()
            .is_some_and(|proxy| proxy.send_event(AimerNativePlatformEvent::ShutdownRequested).is_ok())
    });

    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    info!("Initializing EventLoop...");
    #[cfg(not(target_os = "android"))]
    let event_loop = EventLoop::<AimerNativePlatformEvent>::with_user_event()
        .build()
        .expect("Failed to create EventLoop");

    #[cfg(target_os = "android")]
    let event_loop = {
        use aimer_events::android_app;
        use winit::platform::android::EventLoopBuilderExtAndroid;
        let app = crate::aimer_app::ANDROID_APP
            .get()
            .expect("ANDROID_APP not set")
            .clone();

        // Normally done already by the generated entry point; repeated here for
        // apps whose hand-written one stored `ANDROID_APP` directly.
        crate::android_storage::publish_data_dir(&app);
        android_app::set_android_app(app.clone());

        // Keep the JNI entry points used by `com.aimer.AimerActivity` reachable.
        // They are only ever called by the JVM at runtime (never from Rust), so
        // without an explicit reference, the linker may garbage-collect them out of
        // the final `cdylib`, which would make the soft-keyboard text bridge fail
        // with `UnsatisfiedLinkError`.
        let _keep_jni: [*const (); 7] = [
            Java_com_aimer_ScreenBridge_nativeDisplayFeatures as *const (),
            Java_com_aimer_AimerActivity_nativeOpenLink as *const (),
            Java_com_aimer_AimerActivity_nativeBack as *const (),
            Java_com_aimer_AimerActivity_nativeInsertText as *const (),
            Java_com_aimer_AimerActivity_nativeSetComposingText as *const (),
            Java_com_aimer_AimerActivity_nativeBackspace as *const (),
            Java_com_aimer_AimerActivity_nativeTextEditingDelta as *const (),
        ];
        std::hint::black_box(_keep_jni);

        EventLoop::<AimerNativePlatformEvent>::with_user_event()
            .with_android_app(app)
            .build()
            .expect("Failed to create EventLoop")
    };

    EVENT_PROXY.set(event_loop.create_proxy()).ok();

    // Route animation redraws requests through the event loop instead of letting
    // animating widgets (e.g. scroll momentum) spawn a sleeping thread per frame.
    // `FrameReady` is delivered via `user_event` after the current frame, which
    // schedules the next redraw safely even on platforms (iOS) that coalesce a
    // synchronous `request_redraw()` issued from inside the draw cycle.
    aimer_events::window::set_redraw_requester(request_frame_ready);

    event_loop.set_control_flow(ControlFlow::Wait);

    aimer_utils::debug!("Creating async runtime...");
    #[cfg(not(target_arch = "wasm32"))]
    let async_runtime = Runtime::new().expect("Failed to create async runtime");

    // The runtime the frames are scheduled by, installed for this thread so a
    // handler deep in the tree reaches it without anyone handing it down. A
    // worker finishing while the loop is parked wakes it through the same
    // request the widget tree uses — the only thing a task on another thread is
    // allowed to do to this one.
    let previous_venus = Venus::uninstall();
    let venus = Venus::new();
    venus.install();
    // A release build aborts on panic, which ends the process the moment the hook
    // returns, so the hook is where pending state gets saved. Installed once the
    // runtime exists: that is what carries a panic on another thread back here.
    aimer_venus::install_panic_hook();
    // A task that became ready asks for a frame without saying what it will do.
    // What it does asks for its own frames while the frame is prepared, so the
    // wake itself is neutral: a state update it makes keeps the frame scoped,
    // and anything else it requests turns the frame into a full one.
    venus.set_notifier(|| with_frame_request_kind(FrameRequestKind::Scoped, request_frame_ready));

    #[cfg(feature = "wasm-hot-reload")]
    let live_reload = live_reload.map(|launch| {
        let session_id = *launch.credentials.session_id();
        let host = LiveReloadHost::bind_with_callback_wake(
            launch.address,
            launch.credentials,
            launch.config,
            Rc::clone(venus.scheduler()),
            request_frame_ready,
            request_callback_ready,
        )
        .expect("failed to start configured live reload listener");
        println!(
            "{}",
            reload_listener_readiness_line(session_id, host.local_addr(), std::process::id())
        );
        host
    });

    // Every task Venus polls is polled inside the async runtime, so a future
    // that builds its resources on the first poll — a request, a timer, a file
    // read — finds a runtime there instead of panicking. The driver itself
    // stays on its own threads: only the lookup happens here.
    #[cfg(not(target_arch = "wasm32"))]
    venus.set_poll_context(crate::poll_context::TokioPollContext::new(
        async_runtime.handle().clone(),
    ));
    #[cfg(target_arch = "wasm32")]
    venus.set_poll_context(crate::browser_poll_context::BrowserPollContext);

    let run_app = || {
        // A desktop application can have several windows, so its loop is the
        // host's. Mobile and web platforms have one surface and keep the
        // single-window handler.
        #[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
        {
            let shared = crate::desktop_host::Shared {
                venus: venus.clone(),
                async_handle: async_runtime.handle().clone(),
                ui_memory: UiMemory::new(ui_memory_limit),
                antialiasing,
                render_mode,
            };
            #[allow(unused_mut)]
            let mut main = shared.window_app(window_attr, widget.boxed());
            #[cfg(feature = "wasm-hot-reload")]
            {
                main.live_reload = live_reload;
            }
            let mut host =
                crate::desktop_host::DesktopHost::new(shared, main, exit_policy, startup_hooks);

            info!("Started main event loop");
            match event_loop.run_app(&mut host) {
                Ok(_) => info!("EventLoop finished successfully"),
                Err(e) => aimer_utils::error!("EventLoop::run_app failed: {:?}", e),
            }
            // The windows, renderers and widget trees drop here, before the
            // runtime they hold a handle to is shut down.
            drop(host);
        }
        #[cfg(any(target_os = "ios", target_os = "android", target_arch = "wasm32"))]
        {
            info!("Creating App instance...");
            #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
            let show_window_after_first_frame = window_attr.is_visible();
            let mut app = AimerApplicationHandler {
                window: None,
                hosted: false,
                closed: false,
                inherited_states: None,
                macos_windowing: Default::default(),
                render_ctx: crate::render_ctx::configured_context(antialiasing, render_mode),
                ui_memory: UiMemory::new(ui_memory_limit),
                #[cfg(debug_assertions)]
                debug_overlay: Default::default(),
                window_attr,
                render_tree: WindowRenderTree::default(),
                #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
                show_window_after_first_frame,
                widget_root: None,
                event_dispatcher: aimer_widget::EventDispatcher::new(),
                scroll_smoother: crate::handler::scroll_classifier::DualScroller::new(),
                frame_request_reason: Rc::new(Cell::new(None)),
                #[cfg(target_arch = "wasm32")]
                web_scroll_phase: crate::handler::web_scroll_phase::WebScrollPhase::new(),
                pending_widget: Some(widget),
                cursor_pos: crate::handler::event_handler::CURSOR_OUTSIDE_POSITION,
                touch_pointer: false,
                pressed_button: None,
                current_modifiers: Default::default(),
                ime_composing: false,
                window_scale: 1.0,
                native_window_size: None,
                pending_resize: None,
                startup_hooks,
                startup_resources: Vec::new(),
                #[cfg(not(target_arch = "wasm32"))]
                async_handle: async_runtime.handle().clone(),
                active_touch_id: None,
                venus,
                file_drag: crate::handler::file_drag::FileDrag::new(),
                #[cfg(feature = "wasm-hot-reload")]
                live_reload,
            };

            let previous_redraw_observer = aimer_events::window::set_thread_redraw_observer({
                let frame_request_reason = app.frame_request_reason.clone();
                move || observe_frame_request(&frame_request_reason)
            });

            info!("Started main event loop");

            // On iOS, this function never returns.
            match event_loop.run_app(&mut app) {
                Ok(_) => info!("EventLoop finished successfully"),
                Err(e) => aimer_utils::error!("EventLoop::run_app failed: {:?}", e),
            }
            aimer_events::window::restore_thread_redraw_observer(previous_redraw_observer);

            // The app, renderer, widget tree and task context drop at the end of
            // this closure, before the runtime they hold a handle to is shut down.
            drop(app);
        }
    };

    run_app();

    // `Venus::install` keeps a thread-local strong reference. Drop it on the
    // event-loop thread while Windows is still running, so OffloadPool joins
    // its workers before thread-local teardown starts.
    drop(Venus::uninstall());
    #[cfg(not(target_arch = "wasm32"))]
    async_runtime.shutdown_background();

    if let Some(previous_venus) = previous_venus {
        previous_venus.install();
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use aimer_attribute::Dimension;
    use aimer_attribute::position::Vec2d;
    use aimer_attribute::size::{ResolvedSize, Size};
    use aimer_canvas::Canvas;
    use aimer_container::SizedBox;
    use aimer_cupid::draw_cmd::DrawCommand;
    use aimer_cupid::draw_cmd_v2::{Rect, RenderNodeId};
    use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata, RetainedRenderPlan};
    use aimer_events::element::{ElementEvent, ScrollDeltaKind};
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_flex::{Column, Row};
    use aimer_input::TextEditingController;
    use aimer_input::input::{CaretContext, DefaultCaret, TextArea, TextField};
    use aimer_scroll::{ScrollAxis, ScrollController, Scrollable};
    use aimer_widget::base::{BuildContext, Color};
    use aimer_widget::{
        AnyElement, Drawable, Element, ElementId, EventElement, FocusNode, LayoutElement,
        Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget, VisitorElement,
    };
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};

    use super::*;

    mod animation_only;
    mod broadcast;
    mod lifecycle;
    mod multi_window;
    mod multi_window_paint;
    mod window_api;
    mod builder_prepass;
    mod builder_retained_child;
    mod forwarding_elements;
    #[cfg(all(feature = "wgpu", target_os = "macos"))]
    mod backdrop_blur_cost;
    #[cfg(all(feature = "wgpu", target_os = "macos"))]
    mod bypass_target;
    mod collapsed_chevron;
    mod collapsible_animation;
    mod compositor_reset;
    mod custom_theme_switch;
    #[cfg(debug_assertions)]
    mod debug_overlay;
    mod decoration_box;
    mod floating_menu_settles;
    mod foreground_decoration_box;
    mod masked_decoration_box;
    mod opacity_mask_box;
    mod focus_click;
    mod focus_repaint;
    mod glass_bar_cost;
    mod hover_bar_click;
    mod hover_border_damage;
    mod implicit_animation_plays;
    mod ios_input;
    mod loading_damage;
    mod post_bar_toggle;
    #[cfg(not(target_arch = "wasm32"))]
    mod render_worker;
    mod resize_grab_area;
    mod resize_live;
    mod resize_sidebar;
    mod retained_button;
    mod retained_modal;
    mod retained_switcher;
    mod rows_in_resizable;
    mod scoped_sync;
    mod stack_overlays;
    mod shadow_clip_origin;
    mod stack_bar_touch;
    mod autofocus_settles;
    mod focus_origin;
    mod window_drag;
    mod text_field_caret;
    mod text_field_in_scrollable;
    mod text_field_chrome;
    mod text_follows_resize;
    mod stack_occlusion;
    mod scalable_overflow;
    mod scalable_text;
    mod scroll_frame_kind;
    mod scroll_one_step;
    mod settled_skip;
    mod skeleton_ticks;
    mod skeleton_to_rows;
    mod theme_switch;
    mod touch_scroll;
    mod trackpad_gestures;
    mod window_resize;

    static VIRTUALIZED_RENDER_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct RetainedTextInputWidget {
        controller: TextEditingController,
        focus_node: FocusNode,
        caret_context: Rc<RefCell<Option<CaretContext>>>,
        multiline: bool,
    }

    fn captured_caret(
        slot: Rc<RefCell<Option<CaretContext>>>,
    ) -> impl Fn(CaretContext) -> DefaultCaret + 'static {
        move |context| {
            *slot.borrow_mut() = Some(context.clone());
            DefaultCaret::new(context, Color::BLACK)
        }
    }

    impl Widget for RetainedTextInputWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let RetainedTextInputWidget {
                controller,
                focus_node,
                caret_context,
                multiline,
            } = self;
            if multiline {
                let area = TextArea::new()
                    .controller(controller)
                    .focus_node(focus_node)
                    .auto_focus(true)
                    .min_lines(2)
                    .max_lines(Some(2))
                    .caret(captured_caret(caret_context));
                SizedBox::new()
                    .width(Dimension::Px(260.0))
                    .height(Dimension::Px(64.0))
                    .child(area)
                    .to_element(ctx)
            } else {
                let field = TextField::new()
                    .controller(controller)
                    .focus_node(focus_node)
                    .auto_focus(true)
                    .caret(captured_caret(caret_context));
                SizedBox::new()
                    .width(Dimension::Px(260.0))
                    .height(Dimension::Px(48.0))
                    .child(field)
                    .to_element(ctx)
            }
        }
    }

    impl aimer_widget::PortableWidget for RetainedTextInputWidget {}

    struct MixedPacketWidget {
        paints: Rc<Cell<usize>>,
    }

    impl Widget for MixedPacketWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            MixedPacketElement {
                paints: self.paints,
                child: LegacyPacketElement.boxed(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for MixedPacketWidget {}

    struct HeadlessVirtualizedWidget {
        controller: ScrollController,
        paints: Rc<Cell<usize>>,
        item_count: u32,
        viewport_width: Dimension,
        viewport_height: Dimension,
        updater: Rc<RefCell<Option<StateUpdater<HeadlessVerticalRowsState>>>>,
    }

    impl Widget for HeadlessVirtualizedWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let rows = HeadlessVerticalRowsWidget {
                item_count: self.item_count,
                paints: self.paints.clone(),
                updater: self.updater.clone(),
            };
            SizedBox::new()
                .width(self.viewport_width)
                .height(self.viewport_height)
                .child(
                    Scrollable::new()
                        .controller(self.controller)
                        .vertical_scroll_bar(None)
                        .horizontal_scroll_bar(None)
                        .child(rows),
                )
                .to_element(ctx)
        }
    }

    impl aimer_widget::PortableWidget for HeadlessVirtualizedWidget {}

    struct HeadlessVerticalRowsWidget {
        item_count: u32,
        paints: Rc<Cell<usize>>,
        updater: Rc<RefCell<Option<StateUpdater<HeadlessVerticalRowsState>>>>,
    }

    impl Widget for HeadlessVerticalRowsWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let updater_slot = self.updater.clone();
            let (element, updater) = StatefulElement::new(
                HeadlessVerticalRowsStateWidget {
                    item_count: self.item_count,
                    paints: self.paints,
                },
                ctx,
            );
            *updater_slot.borrow_mut() = Some(updater);
            element.boxed()
        }
    }

    impl aimer_widget::PortableWidget for HeadlessVerticalRowsWidget {}

    struct HeadlessVerticalRowsStateWidget {
        item_count: u32,
        paints: Rc<Cell<usize>>,
    }

    impl StatefulWidget for HeadlessVerticalRowsStateWidget {
        type State = HeadlessVerticalRowsState;

        fn create_state(self) -> Self::State {
            HeadlessVerticalRowsState {
                item_count: self.item_count,
                paints: self.paints,
            }
        }
    }

    struct HeadlessVerticalRowsState {
        item_count: u32,
        paints: Rc<Cell<usize>>,
    }

    impl State<HeadlessVerticalRowsStateWidget> for HeadlessVerticalRowsState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            let paints = self.paints.clone();
            Column::new()
                .list(0..self.item_count)
                .item_extent(Dimension::Px(20.0))
                .builder(move |row_index| HeadlessVirtualizedRow {
                    row_index: *row_index,
                    paints: paints.clone(),
                })
        }
    }

    struct HeadlessVirtualizedRow {
        row_index: u32,
        paints: Rc<Cell<usize>>,
    }

    impl Widget for HeadlessVirtualizedRow {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            HeadlessVirtualizedRowElement {
                row_index: self.row_index,
                paints: self.paints,
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for HeadlessVirtualizedRow {}

    struct HeadlessVirtualizedRowElement {
        row_index: u32,
        paints: Rc<Cell<usize>>,
    }

    fn headless_virtualized_row_color(row_index: u32) -> Color {
        match row_index % 8 {
            0 => Color::RED,
            1 => Color::GREEN,
            2 => Color::BLUE,
            3 => Color::WHITE,
            4 => Color::YELLOW,
            5 => Color::MAGENTA,
            6 => Color::CYAN,
            _ => Color::ORANGE,
        }
    }

    impl VisitorElement for HeadlessVirtualizedRowElement {
        fn debug_name(&self) -> &'static str {
            "HeadlessVirtualizedRow"
        }
    }

    impl EventElement for HeadlessVirtualizedRowElement {}
    impl Rebuildable for HeadlessVirtualizedRowElement {}

    impl LayoutElement for HeadlessVirtualizedRowElement {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(100.0), Dimension::Px(20.0)))
        }
    }

    impl Drawable for HeadlessVirtualizedRowElement {
        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            self.paints.set(self.paints.get() + 1);
            let (red, green, blue, alpha) =
                headless_virtualized_row_color(self.row_index).to_rgba();
            let canvas = Canvas::of(ctx);
            canvas.fill_rect(Rect::new(0.0, 0.0, 100.0, 20.0), [red, green, blue, alpha]);
            canvas.finish();
        }
    }

    struct HeadlessHorizontalVirtualizedWidget {
        controller: ScrollController,
        paints: Rc<Cell<usize>>,
        item_count: u32,
        updater: Rc<RefCell<Option<StateUpdater<HeadlessHorizontalRowsState>>>>,
    }

    impl Widget for HeadlessHorizontalVirtualizedWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let rows = HeadlessHorizontalRowsWidget {
                item_count: self.item_count,
                paints: self.paints.clone(),
                updater: self.updater.clone(),
            };
            SizedBox::new()
                .width(Dimension::Percent(50.0))
                .height(Dimension::Percent(50.0))
                .child(
                    Scrollable::new()
                        .axis(ScrollAxis::Horizontal)
                        .controller(self.controller)
                        .vertical_scroll_bar(None)
                        .horizontal_scroll_bar(None)
                        .child(rows.boxed()),
                )
                .to_element(ctx)
        }
    }

    impl aimer_widget::PortableWidget for HeadlessHorizontalVirtualizedWidget {}

    struct HeadlessHorizontalRowsWidget {
        item_count: u32,
        paints: Rc<Cell<usize>>,
        updater: Rc<RefCell<Option<StateUpdater<HeadlessHorizontalRowsState>>>>,
    }

    impl Widget for HeadlessHorizontalRowsWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let updater_slot = self.updater.clone();
            let (element, updater) = StatefulElement::new(
                HeadlessHorizontalRowsStateWidget {
                    item_count: self.item_count,
                    paints: self.paints,
                },
                ctx,
            );
            *updater_slot.borrow_mut() = Some(updater);
            element.boxed()
        }
    }

    impl aimer_widget::PortableWidget for HeadlessHorizontalRowsWidget {}

    struct HeadlessHorizontalRowsStateWidget {
        item_count: u32,
        paints: Rc<Cell<usize>>,
    }

    impl StatefulWidget for HeadlessHorizontalRowsStateWidget {
        type State = HeadlessHorizontalRowsState;

        fn create_state(self) -> Self::State {
            HeadlessHorizontalRowsState {
                item_count: self.item_count,
                paints: self.paints,
            }
        }
    }

    struct HeadlessHorizontalRowsState {
        item_count: u32,
        paints: Rc<Cell<usize>>,
    }

    impl State<HeadlessHorizontalRowsStateWidget> for HeadlessHorizontalRowsState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            let paints = self.paints.clone();
            Row::new()
                .list(0..self.item_count)
                .item_extent(Dimension::Px(20.0))
                .builder(move |row_index| HeadlessHorizontalVirtualizedItem {
                    row_index: *row_index,
                    paints: paints.clone(),
                })
        }
    }

    struct HeadlessHorizontalVirtualizedItem {
        row_index: u32,
        paints: Rc<Cell<usize>>,
    }

    impl Widget for HeadlessHorizontalVirtualizedItem {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            HeadlessHorizontalVirtualizedItemElement {
                row_index: self.row_index,
                paints: self.paints,
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for HeadlessHorizontalVirtualizedItem {}

    struct HeadlessHorizontalStatefulVirtualizedWidget {
        controller: ScrollController,
        paints: Rc<Cell<usize>>,
        item_count: u32,
        row_updaters: Rc<RefCell<HashMap<u32, StateUpdater<HeadlessHorizontalItemState>>>>,
    }

    impl Widget for HeadlessHorizontalStatefulVirtualizedWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let paints = self.paints.clone();
            let row_updaters = self.row_updaters.clone();
            let rows = Row::new()
                .list(0..self.item_count)
                .item_extent(Dimension::Px(20.0))
                .builder(move |row_index| HeadlessHorizontalStatefulItemWidget {
                    row_index: *row_index,
                    paints: paints.clone(),
                    row_updaters: row_updaters.clone(),
                });
            SizedBox::new()
                .width(Dimension::Percent(50.0))
                .height(Dimension::Percent(50.0))
                .child(
                    Scrollable::new()
                        .axis(ScrollAxis::Horizontal)
                        .controller(self.controller)
                        .vertical_scroll_bar(None)
                        .horizontal_scroll_bar(None)
                        .child(rows),
                )
                .to_element(ctx)
        }
    }

    impl aimer_widget::PortableWidget for HeadlessHorizontalStatefulVirtualizedWidget {}

    struct HeadlessHorizontalStatefulItemWidget {
        row_index: u32,
        paints: Rc<Cell<usize>>,
        row_updaters: Rc<RefCell<HashMap<u32, StateUpdater<HeadlessHorizontalItemState>>>>,
    }

    impl Widget for HeadlessHorizontalStatefulItemWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let row_index = self.row_index;
            let (element, updater) = StatefulElement::new(
                HeadlessHorizontalItemStateWidget {
                    row_index,
                    paints: self.paints,
                },
                ctx,
            );
            self.row_updaters.borrow_mut().insert(row_index, updater);
            element.boxed()
        }
    }

    impl aimer_widget::PortableWidget for HeadlessHorizontalStatefulItemWidget {}

    struct HeadlessHorizontalItemStateWidget {
        row_index: u32,
        paints: Rc<Cell<usize>>,
    }

    impl StatefulWidget for HeadlessHorizontalItemStateWidget {
        type State = HeadlessHorizontalItemState;

        fn create_state(self) -> Self::State {
            HeadlessHorizontalItemState {
                color: headless_virtualized_row_color(self.row_index),
                height: 80.0,
                paints: self.paints,
            }
        }
    }

    struct HeadlessHorizontalItemState {
        color: Color,
        height: f32,
        paints: Rc<Cell<usize>>,
    }

    impl State<HeadlessHorizontalItemStateWidget> for HeadlessHorizontalItemState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            HeadlessHorizontalItemPaintWidget {
                color: self.color,
                height: self.height,
                paints: self.paints.clone(),
            }
        }
    }

    struct HeadlessHorizontalItemPaintWidget {
        color: Color,
        height: f32,
        paints: Rc<Cell<usize>>,
    }

    impl Widget for HeadlessHorizontalItemPaintWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            HeadlessHorizontalStatefulItemElement {
                color: self.color,
                height: self.height,
                paints: self.paints,
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for HeadlessHorizontalItemPaintWidget {}

    fn paint_horizontal_item(ctx: &BuildContext, paints: &Cell<usize>, color: Color, height: f32) {
        paints.set(paints.get() + 1);
        let (red, green, blue, alpha) = color.to_rgba();
        let canvas = Canvas::of(ctx);
        canvas.fill_rect(Rect::new(0.0, 0.0, 20.0, height), [red, green, blue, alpha]);
        canvas.finish();
    }

    struct HeadlessHorizontalVirtualizedItemElement {
        row_index: u32,
        paints: Rc<Cell<usize>>,
    }

    impl VisitorElement for HeadlessHorizontalVirtualizedItemElement {
        fn debug_name(&self) -> &'static str {
            "HeadlessHorizontalVirtualizedItem"
        }
    }

    impl EventElement for HeadlessHorizontalVirtualizedItemElement {}
    impl Rebuildable for HeadlessHorizontalVirtualizedItemElement {}

    impl LayoutElement for HeadlessHorizontalVirtualizedItemElement {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(20.0), Dimension::Px(80.0)))
        }
    }

    impl Drawable for HeadlessHorizontalVirtualizedItemElement {
        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            paint_horizontal_item(
                ctx,
                self.paints.as_ref(),
                headless_virtualized_row_color(self.row_index),
                80.0,
            );
        }
    }

    struct HeadlessHorizontalStatefulItemElement {
        color: Color,
        height: f32,
        paints: Rc<Cell<usize>>,
    }

    impl VisitorElement for HeadlessHorizontalStatefulItemElement {
        fn debug_name(&self) -> &'static str {
            "HeadlessHorizontalVirtualizedItem"
        }
    }

    impl EventElement for HeadlessHorizontalStatefulItemElement {}
    impl Rebuildable for HeadlessHorizontalStatefulItemElement {}

    impl LayoutElement for HeadlessHorizontalStatefulItemElement {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(20.0), Dimension::Px(self.height)))
        }
    }

    impl Drawable for HeadlessHorizontalStatefulItemElement {
        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            paint_horizontal_item(ctx, self.paints.as_ref(), self.color, self.height);
        }
    }

    fn direct_headless_frame_packet<W: Widget + 'static>(
        app: &mut HeadlessAimerApp<W>,
    ) -> (f32, FramePacket) {
        // Drives the frame by hand, so it has to make the window the active
        // one itself, as every public way into the application does.
        let _active = app.activate();
        let width = app.size.width;
        let height = app.size.height;
        app.canvas.begin_frame();
        let window = app.window.clone();
        let (scale, damage) = {
            let (_render_ctx, mut drawer) = app.app.split_for_frame(window);
            drawer.draw(&app.canvas, width, height)
        };
        app.app.end_frame();
        let packet = headless_packet_from_canvas(app, scale, damage);
        (scale, packet)
    }

    fn dispatch_headless_event<W: Widget + 'static>(
        app: &mut HeadlessAimerApp<W>,
        pos: Vec2d,
        event: &ElementEvent,
    ) {
        let _active = app.activate();
        let root = app
            .app
            .widget_root
            .as_ref()
            .expect("headless frame built the widget root");
        let _ = app.app.event_dispatcher.dispatch(root.as_ref(), pos, event);
    }

    fn focus_headless_field<W: Widget + 'static>(
        app: &mut HeadlessAimerApp<W>,
        focus_node: &FocusNode,
    ) {
        focus_node.request_focus();
        dispatch_headless_event(app, Vec2d { x: 0.0, y: 0.0 }, &ElementEvent::Cancel);
        assert!(
            focus_node.has_focus(),
            "headless dispatcher settled field focus"
        );
    }

    fn retained_text_field_node<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        packet: &FramePacket,
    ) -> (ElementId, RenderNodeId, u64, Rect) {
        let plan = packet
            .render_plan()
            .expect("direct headless frame has a retained plan");
        let root = app
            .app
            .widget_root
            .as_ref()
            .expect("headless frame built the widget root");
        let mut pending = vec![root.as_ref() as &dyn Element];
        while let Some(element) = pending.pop() {
            if element.debug_name() == "TextField"
                && element.element_type_id() != TypeId::of::<StatefulElement>()
                && let Some(render_node) = app.app.render_node_for_element(element.id())
                && let Some(revision) = plan.local_v2_revision(render_node)
            {
                let bounds = app
                    .app
                    .render_tree()
                    .element_bounds(render_node)
                    .expect("retained text field has render bounds");
                return (element.id(), render_node, revision, bounds);
            }
            element.visit_retained_v2_children(&mut |_, child| pending.push(child));
        }
        panic!("retained plan has no local-v2 TextField node");
    }

    fn retained_element_id_named<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        name: &str,
    ) -> Option<ElementId> {
        let root = app
            .app
            .widget_root
            .as_ref()
            .expect("headless frame built the widget root");
        let mut pending = vec![root.as_ref() as &dyn Element];
        while let Some(element) = pending.pop() {
            if element.debug_name() == name {
                return Some(element.id());
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        None
    }

    fn retained_element_ids_named<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        name: &str,
    ) -> Vec<ElementId> {
        let root = app
            .app
            .widget_root
            .as_ref()
            .expect("headless frame built the widget root");
        let mut pending = vec![root.as_ref() as &dyn Element];
        let mut ids = Vec::new();
        while let Some(element) = pending.pop() {
            if element.debug_name() == name {
                ids.push(element.id());
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        ids
    }

    fn legacy_text_command_count(packet: &FramePacket) -> usize {
        packet
            .frame()
            .draw_list
            .commands()
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    DrawCommand::DrawText { .. }
                        | DrawCommand::DrawRichText { .. }
                        | DrawCommand::DrawTextDecoration { .. }
                )
            })
            .count()
    }

    fn headless_packet_from_canvas<W: Widget + 'static>(
        app: &mut HeadlessAimerApp<W>,
        scale: f32,
        damage: aimer_cupid::damage_region::DamageSet,
    ) -> FramePacket {
        let width = app.size.width;
        let height = app.size.height;
        let plan = app
            .canvas
            .take_retained_render_plan()
            .expect("the headless frame includes its retained plan");
        let frame = Frame::new(app.canvas.take_draw_list(), width, height);
        let metadata = FrameRenderMetadata::new(scale, 0, 0, 0, 0, damage);
        FramePacket::with_render_plan(frame, metadata, None, Some(plan))
    }

    // Clean rows remain in the retained scene even when a frame's plan omits
    // them because they do not intersect its damage.
    fn visible_horizontal_rows<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        viewport: (f32, f32),
    ) -> Vec<RenderNodeId> {
        let root = app.app.widget_root.as_ref().unwrap();
        let mut visible_rows = Vec::new();
        let mut pending = vec![root.as_ref() as &dyn Element];
        while let Some(element) = pending.pop() {
            if element.debug_name() == "HeadlessHorizontalVirtualizedItem"
                && let Some(render_node) = app.app.render_node_for_element(element.id())
                && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                && bounds.x < viewport.0
                && bounds.x + bounds.width > 0.0
                && bounds.y < viewport.1
                && bounds.y + bounds.height > 0.0
            {
                assert!(
                    app.app
                        .render_tree()
                        .draw_list_revision(render_node)
                        .unwrap()
                        > 0
                );
                visible_rows.push(render_node);
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        visible_rows
    }

    #[test]
    fn retained_text_field_resolves_clicks_and_native_preedit_before_paint() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let text = "alpha beta";
        let controller = TextEditingController::with_text(text);
        let focus_node = FocusNode::new();
        let caret_slot = Rc::new(RefCell::new(None));
        let mut app = AimerApp::start_headless_with(
            RetainedTextInputWidget {
                controller: controller.clone(),
                focus_node: focus_node.clone(),
                caret_context: caret_slot.clone(),
                multiline: false,
            },
            HeadlessOptions {
                size: PhysicalSize::new(320, 120),
                scale_factor: 1.0,
            },
        );

        let (_, initial_packet) = direct_headless_frame_packet(&mut app);
        assert!(
            initial_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        focus_headless_field(&mut app, &focus_node);
        let (_, focused_packet) = direct_headless_frame_packet(&mut app);
        let (field_id, field_node, focused_revision, bounds) =
            retained_text_field_node(&app, &focused_packet);
        assert_eq!(
            app.app.render_tree().paint_source(field_node).unwrap(),
            aimer_cupid::draw_cmd_v2::RenderPaintSource::LocalV2
        );
        let caret = caret_slot
            .borrow()
            .as_ref()
            .expect("the retained caret factory ran")
            .clone();
        assert!(caret.is_focused());
        assert!(caret.is_available());
        let focused_caret_geometry = caret.geometry();
        let focused_offset = caret.offset();

        let click = Vec2d {
            x: bounds.x + 8.0,
            y: bounds.y + bounds.height * 0.5,
        };
        for event in [
            ElementEvent::PointerDown(PointerInfo::mouse(click, PointerButton::Primary)),
            ElementEvent::PointerUp(PointerInfo::mouse(click, PointerButton::Primary)),
        ] {
            dispatch_headless_event(&mut app, click, &event);
        }
        let (_, click_packet) = direct_headless_frame_packet(&mut app);
        let (updated_id, updated_node, click_revision, _) =
            retained_text_field_node(&app, &click_packet);
        assert_eq!(updated_id, field_id);
        assert_eq!(updated_node, field_node);
        assert_eq!(
            app.app.render_tree().paint_source(field_node).unwrap(),
            aimer_cupid::draw_cmd_v2::RenderPaintSource::LocalV2
        );
        assert!(
            controller.value().selection().focus() < text.len(),
            "deferred click moves the caret from the initial end position"
        );
        // The caret is a retained child: moving it leaves the field's text
        // commands unchanged, so the field recording keeps its revision.
        assert_eq!(click_revision, focused_revision);
        assert_ne!(caret.offset(), focused_offset);
        assert_ne!(caret.geometry(), focused_caret_geometry);
        assert_eq!(caret.offset(), controller.value().selection().focus());
        assert!(caret.geometry().height.is_finite() && caret.geometry().height > 0.0);
        assert_eq!(legacy_text_command_count(&click_packet), 0);

        app.send_user_event(AimerNativePlatformEvent::SetPreedit {
            text: "かな".to_owned(),
            cursor: Some((6, 6)),
        });
        assert!(app.is_composing());
        let (_, preedit_packet) = direct_headless_frame_packet(&mut app);
        let (_, preedit_node, preedit_revision, _) =
            retained_text_field_node(&app, &preedit_packet);
        assert_eq!(preedit_node, field_node);
        assert_eq!(
            app.app.render_tree().paint_source(field_node).unwrap(),
            aimer_cupid::draw_cmd_v2::RenderPaintSource::LocalV2
        );
        assert!(controller.value().composing().is_some());
        assert!(caret.is_composing());
        assert!(caret.is_available());
        assert_ne!(preedit_revision, click_revision);
        assert_eq!(legacy_text_command_count(&preedit_packet), 0);

        app.send_user_event(AimerNativePlatformEvent::InsertText("仮".to_owned()));
        assert!(!app.is_composing());
        let (_, committed_packet) = direct_headless_frame_packet(&mut app);
        let (_, committed_node, _, _) = retained_text_field_node(&app, &committed_packet);
        assert_eq!(committed_node, field_node);
        assert!(controller.value().composing().is_none());
        assert!(!caret.is_composing());
        assert_eq!(legacy_text_command_count(&committed_packet), 0);
    }

    #[test]
    fn retained_text_area_keeps_scrolled_caret_and_native_preedit_local() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let controller = TextEditingController::with_text(
            "first line\nsecond line\nthird line\nfourth line\nfifth line",
        );
        let focus_node = FocusNode::new();
        let caret_slot = Rc::new(RefCell::new(None));
        let mut app = AimerApp::start_headless_with(
            RetainedTextInputWidget {
                controller: controller.clone(),
                focus_node: focus_node.clone(),
                caret_context: caret_slot.clone(),
                multiline: true,
            },
            HeadlessOptions {
                size: PhysicalSize::new(320, 120),
                scale_factor: 1.0,
            },
        );

        let (_, first_packet) = direct_headless_frame_packet(&mut app);
        focus_headless_field(&mut app, &focus_node);
        let (_, focused_packet) = direct_headless_frame_packet(&mut app);
        let (field_id, field_node, focused_revision, bounds) =
            retained_text_field_node(&app, &focused_packet);
        let caret = caret_slot
            .borrow()
            .as_ref()
            .expect("the retained text-area caret factory ran")
            .clone();
        let geometry = caret.geometry();
        assert!(caret.is_focused());
        assert!(caret.is_available());
        assert!(geometry.height > 0.0);
        assert!(geometry.y >= 0.0);
        assert!(geometry.y + geometry.height <= bounds.height + 1.0);

        app.send_user_event(AimerNativePlatformEvent::SetPreedit {
            text: "かんじ".to_owned(),
            cursor: Some((9, 9)),
        });
        let (_, preedit_packet) = direct_headless_frame_packet(&mut app);
        let (updated_id, updated_node, preedit_revision, updated_bounds) =
            retained_text_field_node(&app, &preedit_packet);
        assert_eq!(updated_id, field_id);
        assert_eq!(updated_node, field_node);
        assert!(controller.value().composing().is_some());
        assert!(caret.is_composing());
        assert_eq!(
            app.app.render_tree().paint_source(field_node).unwrap(),
            aimer_cupid::draw_cmd_v2::RenderPaintSource::LocalV2
        );
        assert_ne!(preedit_revision, focused_revision);
        let geometry = caret.geometry();
        assert!(geometry.height > 0.0);
        assert!(geometry.y >= 0.0);
        assert!(geometry.y + geometry.height <= updated_bounds.height + 1.0);
        assert_eq!(legacy_text_command_count(&preedit_packet), 0);
        assert_eq!(legacy_text_command_count(&first_packet), 0);
    }

    #[test]
    fn retained_text_field_context_menu_stays_in_a_separate_overlay() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let text = "alpha beta";
        let controller = TextEditingController::with_text(text);
        let focus_node = FocusNode::new();
        let caret_slot = Rc::new(RefCell::new(None));
        let mut app = AimerApp::start_headless_with(
            RetainedTextInputWidget {
                controller: controller.clone(),
                focus_node: focus_node.clone(),
                caret_context: caret_slot,
                multiline: false,
            },
            HeadlessOptions {
                size: PhysicalSize::new(320, 160),
                scale_factor: 1.0,
            },
        );
        let (_, initial_packet) = direct_headless_frame_packet(&mut app);
        focus_headless_field(&mut app, &focus_node);
        let (_, focused_packet) = direct_headless_frame_packet(&mut app);
        let (field_id, field_node, focused_revision, bounds) =
            retained_text_field_node(&app, &focused_packet);

        let secondary_click = Vec2d {
            x: bounds.x + 18.0,
            y: bounds.y + bounds.height * 0.5,
        };
        dispatch_headless_event(
            &mut app,
            secondary_click,
            &ElementEvent::PointerDown(PointerInfo::mouse(
                secondary_click,
                PointerButton::Secondary,
            )),
        );
        let (_, menu_packet) = direct_headless_frame_packet(&mut app);
        let (selected_field_id, selected_field_node, menu_field_revision, _) =
            retained_text_field_node(&app, &menu_packet);
        assert_eq!(selected_field_id, field_id);
        assert_eq!(selected_field_node, field_node);
        assert_ne!(menu_field_revision, focused_revision);
        assert!(!controller.value().selection().is_collapsed());
        assert_eq!(
            app.app.render_tree().paint_source(field_node).unwrap(),
            aimer_cupid::draw_cmd_v2::RenderPaintSource::LocalV2
        );

        let panel_id = retained_element_id_named(&app, "ContextMenu")
            .expect("the hosted panel is part of the overlay tree");
        let panel_node = app
            .app
            .render_node_for_element(panel_id)
            .expect("the hosted panel has an independent render node");
        assert!(
            menu_packet
                .render_plan()
                .and_then(|plan| plan.local_v2_revision(panel_node))
                .is_some()
        );

        let overlay_id = retained_element_id_named(&app, "ModalOverlay")
            .expect("the modal overlay is a separate retained element");
        let overlay_node = app
            .app
            .render_node_for_element(overlay_id)
            .expect("the overlay has its own render node");
        assert_ne!(overlay_node, field_node);
        let mut ancestor = Some(overlay_node);
        while let Some(node) = ancestor {
            assert_ne!(node, field_node, "the overlay is outside the field subtree");
            ancestor = app
                .app
                .render_tree()
                .parent_of(node)
                .expect("the overlay parent chain is valid");
        }
        let rows_id = retained_element_id_named(&app, "ContextMenuRows")
            .expect("the hosted menu rows are part of the overlay tree");
        let rows_node = app
            .app
            .render_node_for_element(rows_id)
            .expect("the menu rows have an independent render node");
        assert_ne!(rows_node, field_node);
        let menu_plan = menu_packet.render_plan().unwrap();
        assert!(menu_plan.local_v2_revision(rows_node).is_none());
        let row_ids = retained_element_ids_named(&app, "ContextMenuRow");
        assert!(!row_ids.is_empty());
        for id in row_ids {
            let row_node = app
                .app
                .render_node_for_element(id)
                .expect("each menu row has its own retained render node");
            assert!(menu_plan.local_v2_revision(row_node).is_some());
            assert_ne!(row_node, field_node);
        }
        let mut ancestor = Some(rows_node);
        let mut reached_overlay = false;
        while let Some(node) = ancestor {
            assert_ne!(node, field_node, "the rows are outside the field subtree");
            reached_overlay |= node == overlay_node;
            ancestor = app
                .app
                .render_tree()
                .parent_of(node)
                .expect("the row parent chain is valid");
        }
        assert!(
            reached_overlay,
            "the rows belong to the modal overlay subtree"
        );
        assert_eq!(
            legacy_text_command_count(&menu_packet),
            0,
            "hosted context-menu labels use their retained row list"
        );

        assert_eq!(legacy_text_command_count(&initial_packet), 0);
    }

    #[test]
    fn virtualized_single_row_state_update_damages_only_that_row() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let controller = ScrollController::new();
        let paints = Rc::new(Cell::new(0));
        let row_updaters = Rc::new(RefCell::new(HashMap::new()));
        let mut app = AimerApp::start_headless_with(
            HeadlessHorizontalStatefulVirtualizedWidget {
                controller,
                paints: paints.clone(),
                item_count: 1_000,
                row_updaters: row_updaters.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(200, 160),
                scale_factor: 1.0,
            },
        );

        let (_, initial_packet) = direct_headless_frame_packet(&mut app);
        assert!(
            initial_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let (_, indexed_packet) = direct_headless_frame_packet(&mut app);
        assert!(indexed_packet.metadata().damage().is_empty());
        assert!(!indexed_packet.render_plan().unwrap().is_complete());
        let visible_rows = visible_horizontal_rows(&app, (100.0, 80.0));
        assert_eq!(visible_rows.len(), 5);
        let target_row = *visible_rows
            .iter()
            .find(|row| {
                (app.app.render_tree().element_bounds(**row).unwrap().x - 40.0).abs() < f32::EPSILON
            })
            .expect("the third row is visible at x=40");
        assert_eq!(
            app.app.render_tree().element_bounds(target_row).unwrap(),
            Rect::new(40.0, 0.0, 20.0, 80.0)
        );
        let revisions = visible_rows
            .iter()
            .map(|row| {
                (
                    *row,
                    app.app.render_tree().draw_list_revision(*row).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();
        let paints_before_update = paints.get();

        row_updaters
            .borrow()
            .get(&2)
            .expect("the visible row has its own state updater")
            .set_state(|state| state.color = Color::WHITE);
        let (_, updated_packet) = direct_headless_frame_packet(&mut app);
        assert!(!updated_packet.render_plan().unwrap().is_complete());
        assert_eq!(paints.get(), paints_before_update + 1);

        let damage = updated_packet.metadata().damage();
        assert!(
            !damage.is_full(),
            "one row state update stayed local: {:?}",
            damage.regions()
        );
        assert!(!damage.regions().is_empty());
        assert!(
            damage.regions().iter().all(|region| {
                region.x >= 40
                    && region.y == 0
                    && region.x + region.width <= 60
                    && region.y + region.height <= 80
            }),
            "row damage escaped the updated item's bounds: {:?}",
            damage.regions()
        );

        let updated_rows = visible_horizontal_rows(&app, (100.0, 80.0));
        assert_eq!(updated_rows, visible_rows);
        let updated_plan = updated_packet.render_plan().unwrap();
        for row in updated_rows {
            let expected_revision = revisions[&row] + u64::from(row == target_row);
            assert_eq!(
                app.app.render_tree().draw_list_revision(row).unwrap(),
                expected_revision
            );
            assert_eq!(
                updated_plan.local_v2_revision(row),
                (row == target_row).then_some(expected_revision),
                "only the changed row is submitted for partial damage"
            );
        }

        let old_bounds = app.app.render_tree().element_bounds(target_row).unwrap();
        assert_eq!(old_bounds, Rect::new(40.0, 0.0, 20.0, 80.0));
        let height_revisions = visible_rows
            .iter()
            .map(|row| {
                (
                    *row,
                    app.app.render_tree().draw_list_revision(*row).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();
        let paints_before_height_update = paints.get();
        row_updaters
            .borrow()
            .get(&2)
            .expect("the visible row has its own state updater")
            .set_state(|state| state.height = 60.0);
        let rebuild_before_height_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_height_frame = aimer_widget::layout_invalidation_generation();
        let (_, height_packet) = direct_headless_frame_packet(&mut app);
        assert!(!height_packet.render_plan().unwrap().is_complete());
        assert_eq!(paints.get(), paints_before_height_update + 1);

        let height_damage = height_packet.metadata().damage();
        let concurrent_height_invalidation = rebuild_before_height_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_height_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_height_invalidation {
            assert!(
                !height_damage.is_full(),
                "one row height update stayed partial: {:?}",
                height_damage.regions()
            );
        }
        if !height_damage.is_full() {
            assert!(!height_damage.regions().is_empty());
            assert!(
                height_damage.regions().iter().all(|region| {
                    region.x as f32 >= old_bounds.x
                        && region.y as f32 >= old_bounds.y
                        && (region.x + region.width) as f32 <= old_bounds.x + old_bounds.width
                        && (region.y + region.height) as f32 <= old_bounds.y + old_bounds.height
                }),
                "height-change damage escaped the old/new bounds union: {:?}",
                height_damage.regions()
            );
            assert!(
                height_damage.regions().iter().any(|region| {
                    region.x <= 50
                        && region.x + region.width > 50
                        && region.y <= 70
                        && region.y + region.height > 70
                }),
                "height-change damage missed the cleared old tail: {:?}",
                height_damage.regions()
            );
        }

        let height_rows = visible_horizontal_rows(&app, (100.0, 80.0));
        assert_eq!(height_rows, visible_rows);
        assert_eq!(
            app.app.render_tree().element_bounds(target_row).unwrap(),
            Rect::new(40.0, 0.0, 20.0, 60.0)
        );
        let height_plan = height_packet.render_plan().unwrap();
        for row in height_rows {
            let expected_revision = height_revisions[&row] + u64::from(row == target_row);
            assert_eq!(
                app.app.render_tree().draw_list_revision(row).unwrap(),
                expected_revision
            );
            assert_eq!(
                height_plan.local_v2_revision(row),
                (row == target_row).then_some(expected_revision),
                "only the changed row is submitted for partial damage"
            );
        }

        let shrunk_bounds = app.app.render_tree().element_bounds(target_row).unwrap();
        let grow_revisions = visible_rows
            .iter()
            .map(|row| {
                (
                    *row,
                    app.app.render_tree().draw_list_revision(*row).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();
        let paints_before_grow = paints.get();
        row_updaters
            .borrow()
            .get(&2)
            .expect("the visible row has its own state updater")
            .set_state(|state| state.height = 80.0);
        let rebuild_before_grow_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_grow_frame = aimer_widget::layout_invalidation_generation();
        let (_, grow_packet) = direct_headless_frame_packet(&mut app);
        assert!(!grow_packet.render_plan().unwrap().is_complete());
        assert_eq!(paints.get(), paints_before_grow + 1);

        let grow_damage = grow_packet.metadata().damage();
        let concurrent_grow_invalidation = rebuild_before_grow_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_grow_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_grow_invalidation {
            assert!(
                !grow_damage.is_full(),
                "one row height growth stayed partial: {:?}",
                grow_damage.regions()
            );
        }
        if !grow_damage.is_full() {
            assert!(!grow_damage.regions().is_empty());
            assert!(
                grow_damage.regions().iter().all(|region| {
                    region.x as f32 >= shrunk_bounds.x
                        && region.y as f32 >= shrunk_bounds.y
                        && (region.x + region.width) as f32 <= shrunk_bounds.x + shrunk_bounds.width
                        && (region.y + region.height) as f32 <= 80.0
                }),
                "height-growth damage escaped the old/new bounds union: {:?}",
                grow_damage.regions()
            );
            assert!(
                grow_damage.regions().iter().any(|region| {
                    region.x <= 50
                        && region.x + region.width > 50
                        && region.y <= 70
                        && region.y + region.height > 70
                }),
                "height-growth damage missed the newly exposed strip: {:?}",
                grow_damage.regions()
            );
        }

        let grow_rows = visible_horizontal_rows(&app, (100.0, 80.0));
        assert_eq!(grow_rows, visible_rows);
        assert_eq!(
            app.app.render_tree().element_bounds(target_row).unwrap(),
            Rect::new(40.0, 0.0, 20.0, 80.0)
        );
        let grow_plan = grow_packet.render_plan().unwrap();
        for row in grow_rows {
            let expected_revision = grow_revisions[&row] + u64::from(row == target_row);
            assert_eq!(
                app.app.render_tree().draw_list_revision(row).unwrap(),
                expected_revision
            );
            assert_eq!(
                grow_plan.local_v2_revision(row),
                (row == target_row).then_some(expected_revision),
                "only the changed row is submitted for partial damage"
            );
        }
    }

    struct MixedPacketElement {
        paints: Rc<Cell<usize>>,
        child: AnyElement,
    }

    impl Drawable for MixedPacketElement {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            self.paints.set(self.paints.get() + 1);
            let canvas = aimer_canvas::Canvas::of(ctx);
            canvas.fill_rect(
                aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 20.0, 10.0),
                [220, 20, 20, 255],
            );
            canvas.finish();
        }
    }

    impl VisitorElement for MixedPacketElement {
        fn debug_name(&self) -> &'static str {
            "MixedPacketElement"
        }

        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }
    }

    impl EventElement for MixedPacketElement {}
    impl Rebuildable for MixedPacketElement {}

    impl LayoutElement for MixedPacketElement {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(20.0), Dimension::Px(10.0)))
        }
    }

    struct LegacyPacketElement;

    impl Drawable for LegacyPacketElement {
        fn update(&self, ctx: &BuildContext) {}
    }

    impl LayoutElement for LegacyPacketElement {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(10.0), Dimension::Px(10.0)))
        }
    }

    impl Rebuildable for LegacyPacketElement {}

    impl VisitorElement for LegacyPacketElement {
        fn debug_name(&self) -> &'static str {
            "LegacyPacketElement"
        }
    }

    impl EventElement for LegacyPacketElement {}

    struct LegacyModalWidget;

    impl Widget for LegacyModalWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            LegacyModalElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for LegacyModalWidget {}

    struct LegacyModalElement;

    impl Drawable for LegacyModalElement {
        fn update(&self, ctx: &BuildContext) {}
    }

    impl LayoutElement for LegacyModalElement {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(12.0), Dimension::Px(12.0)))
        }
    }

    impl Rebuildable for LegacyModalElement {}

    impl VisitorElement for LegacyModalElement {
        fn debug_name(&self) -> &'static str {
            "LegacyModalElement"
        }
    }

    impl EventElement for LegacyModalElement {}

    #[test]
    fn headless_frame_keeps_retained_v2_out_of_the_flat_legacy_list() {
        let paints = Rc::new(Cell::new(0));
        let mut app = AimerApp::start_headless_with(
            MixedPacketWidget {
                paints: paints.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 2.0,
            },
        );
        let _modal = aimer_modal::Modal::new().child(LegacyModalWidget).show();
        app.render_frame();
        assert_eq!(paints.get(), 1);
        let root_id = app.app.widget_root.as_ref().unwrap().id();
        let render_node = app.app.render_node_for_element(root_id).unwrap();
        assert_eq!(
            app.app
                .render_tree
                .tree()
                .paint_source(render_node)
                .unwrap(),
            aimer_cupid::draw_cmd_v2::RenderPaintSource::LocalV2
        );

        let colors = app
            .canvas
            .draw_list()
            .commands()
            .iter()
            .filter_map(|command| match command {
                aimer_cupid::draw_cmd::DrawCommand::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();
        // Every paint travels in the render plan. The legacy-only elements (the
        // green packet child and the purple modal content) have no retained
        // paint and draw nothing, and the flat list holds no paint at all.
        assert!(
            colors.is_empty(),
            "the flat list kept paint commands: {colors:?}"
        );
    }

    /// An animation whose damage covers more than half of the target is
    /// repainted in full, so its frame must carry the whole tree. The culled
    /// slice used to be rejected, which left the animation frozen.
    #[test]
    fn an_animation_over_most_of_the_window_still_lowers_a_plan_every_frame() {
        use aimer_animation::{AnimationController, curves, FadeTransition};
        use aimer_container::SizedBox;

        struct Page;

        impl Widget for Page {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                let controller = AnimationController::with_millis(5_000, curves::LINEAR);
                controller.forward_from_first_tick();
                FadeTransition::new(
                    controller,
                    SizedBox::new()
                        .width(150.0)
                        .height(90.0)
                        .color(aimer_widget::base::Color::Rgb(20, 120, 220)),
                )
                .to_element(ctx)
            }
        }

        impl aimer_widget::PortableWidget for Page {}

        let mut app = AimerApp::start_headless_with(
            Page,
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 1.0,
            },
        );
        for frame in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.canvas.begin_frame();
            let window = app.window.clone();
            let (_, mut drawer) = app.app.split_for_frame(window);
            let (_, damage) = drawer.draw(&app.canvas, 200, 100);
            let plan = app.canvas.take_retained_render_plan();

            assert!(
                plan.is_some(),
                "frame {frame}: the render plan could not be lowered"
            );
            assert!(
                !damage.is_empty(),
                "frame {frame}: the animation produced no damage"
            );
        }
    }

    /// Presents headless frames through the real renderer on Metal and reads
    /// the pixels back, as a window does after each resize.
    #[cfg(all(feature = "wgpu", target_os = "macos"))]
    mod resize_gpu {
        use aimer_cupid::frame::FramePacket;

        pub(super) struct ResizeGpu {
            device: wgpu::Device,
            queue: wgpu::Queue,
            backend: aimer_cupid::WgpuBackend,
            renderer: aimer_cupid::renderer::Renderer<aimer_cupid::WgpuBackend>,
            target: Option<(wgpu::Texture, u32, u32)>,
            pub(super) resource_generation: u64,
        }

        impl ResizeGpu {
            pub(super) fn new() -> Option<Self> {
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::METAL,
                    flags: wgpu::InstanceFlags::default(),
                    backend_options: wgpu::BackendOptions::default(),
                    memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                    display: None,
                });
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                let adapter = runtime
                    .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                        apply_limit_buckets: true,
                    }))
                    .ok()?;
                let (device, queue) = runtime
                    .block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .ok()?;
                let backend = aimer_cupid::WgpuBackend::new(device.clone(), queue.clone());
                let renderer =
                    aimer_cupid::renderer::Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);
                Some(Self {
                    device,
                    queue,
                    backend,
                    renderer,
                    target: None,
                    resource_generation: 0,
                })
            }

            /// Renders one packet at `width` x `height` and returns the RGBA pixels.
            pub(super) fn present(
                &mut self,
                packet: &FramePacket,
                width: u32,
                height: u32,
            ) -> Vec<u8> {
                if self.target.as_ref().map(|(_, w, h)| (*w, *h)) != Some((width, height)) {
                    let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("probe target"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    self.target = Some((texture, width, height));
                }
                let texture = self.target.as_ref().unwrap().0.clone();
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.renderer
                    .render_packet(&self.backend, &view, packet, false);

                let unpadded = width * 4;
                let padded = unpadded.div_ceil(256) * 256;
                let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("probe readback"),
                    size: u64::from(padded) * u64::from(height),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = self
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                self.queue.submit(Some(encoder.finish()));
                let slice = readback.slice(..);
                slice.map_async(wgpu::MapMode::Read, |result| {
                    result.expect("map probe readback");
                });
                self.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("wait for probe readback");
                let mapped = slice.get_mapped_range().expect("probe range");
                let mut pixels = vec![0u8; (unpadded * height) as usize];
                for row in 0..height as usize {
                    let source = row * padded as usize;
                    let dest = row * unpadded as usize;
                    pixels[dest..dest + unpadded as usize]
                        .copy_from_slice(&mapped[source..source + unpadded as usize]);
                }
                pixels
            }
        }
    }

    /// When a crossfade ends, the outgoing child leaves the element tree, so its
    /// render node has to leave the render tree too, or the old picture stays
    /// painted behind the new one.
    fn a_finished_transition_leaves_only_the_incoming_child(morph: bool) {
        use aimer_animation::{AnimatedSwitcher, curves, MorphTransition};

        aimer_tls::thread_local! {
            static SWITCH: RefCell<Option<StateUpdater<SwitchState>>> = const { RefCell::new(None) };
        }

        struct SwitchPage {
            morph: bool,
        }
        struct SwitchState {
            morph: bool,
            second: bool,
        }

        impl StatefulWidget for SwitchPage {
            type State = SwitchState;
            fn create_state(self) -> Self::State {
                SwitchState {
                    morph: self.morph,
                    second: false,
                }
            }
        }

        impl State<SwitchPage> for SwitchState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                SWITCH.replace(Some(updater));
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                let (key, width) = if self.second {
                    ("b", 60.0)
                } else {
                    ("a", 90.0)
                };
                let picture = SizedBox::new()
                    .width(width)
                    .height(40.0)
                    .color(Color::Rgb(200, 30, 30));
                let duration = std::time::Duration::from_millis(120);
                if self.morph {
                    MorphTransition::new(duration, curves::LINEAR, picture)
                        .child_key(key)
                        .boxed()
                } else {
                    AnimatedSwitcher::new(duration, curves::LINEAR, picture)
                        .child_key(key)
                        .boxed()
                }
            }
        }

        impl Widget for SwitchPage {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                StatefulElement::new_with_name(self, ctx, "SwitchPage", None)
                    .0
                    .boxed()
            }
        }
        impl aimer_widget::PortableWidget for SwitchPage {}

        let mut app = AimerApp::start_headless_with(
            SwitchPage { morph },
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(4);
        let draws = |app: &HeadlessAimerApp<_>| {
            app.app
                .render_tree()
                .render_all()
                .iter()
                .filter(|op| matches!(op, aimer_cupid::draw_cmd_v2::RenderOp::Draw(_)))
                .count()
        };
        let single = draws(&app);

        SWITCH.with_borrow(|updater| {
            updater
                .as_ref()
                .unwrap()
                .set_state(|state| state.second = true)
        });
        app.render_frame();
        assert!(
            draws(&app) > single,
            "mid-transition both pictures are in the render tree"
        );

        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.render_frame();
        }
        app.pump_frames(4);
        assert_eq!(
            draws(&app),
            single,
            "the outgoing picture is gone for good once the transition ends"
        );
    }

    #[test]
    fn a_finished_crossfade_removes_the_outgoing_child_from_the_render_tree() {
        a_finished_transition_leaves_only_the_incoming_child(false);
    }

    #[test]
    fn a_finished_morph_removes_the_outgoing_child_from_the_render_tree() {
        a_finished_transition_leaves_only_the_incoming_child(true);
    }

    /// A frame that was built but never reached the screen (the surface could
    /// not be acquired, as happens constantly while a window is being resized)
    /// has already consumed its damage. The next frame must repaint everything,
    /// or whatever the lost frame changed stays missing until something else
    /// happens to repaint it.
    #[test]
    fn a_frame_after_a_lost_present_repaints_the_whole_target() {
        use aimer_container::SizedBox;

        struct Page;
        impl Widget for Page {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                SizedBox::new()
                    .width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0))
                    .color(aimer_widget::base::Color::Rgb(20, 120, 220))
                    .to_element(ctx)
            }
        }
        impl aimer_widget::PortableWidget for Page {}

        let mut app = AimerApp::start_headless_with(
            Page,
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(4);
        app.render_frame();
        let (_, settled) = app.last_frame_result.take().expect("a frame was drawn");
        assert!(settled.is_empty(), "nothing changed, so nothing is damaged");

        crate::render_ctx::note_frame_not_presented();
        app.render_frame();
        let (_, retried) = app.last_frame_result.take().expect("a frame was drawn");
        assert!(
            retried.is_full(),
            "the retry after a lost present must be a full repaint"
        );

        app.render_frame();
        let (_, after) = app.last_frame_result.take().expect("a frame was drawn");
        assert!(after.is_empty(), "the request is consumed by the retry");
    }

    /// A node that grows has to be recorded again: its background fills its
    /// size, so a list recorded for the old size left the new area of a resized
    /// window unpainted.
    #[cfg(all(feature = "wgpu", target_os = "macos"))]
    #[test]
    fn a_resized_window_paints_the_area_it_gained_on_metal() {
        use aimer_container::SizedBox;

        struct Page;
        impl Widget for Page {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                SizedBox::new()
                    .width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0))
                    .color(aimer_widget::base::Color::Rgb(20, 120, 220))
                    .to_element(ctx)
            }
        }
        impl aimer_widget::PortableWidget for Page {}

        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(mut gpu) = resize_gpu::ResizeGpu::new() else {
            assert!(
                std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_none(),
                "Metal adapter unavailable; run this test in a host GPU session"
            );
            eprintln!("skipping: Metal adapter unavailable");
            return;
        };
        let mut app = AimerApp::start_headless_with(
            Page,
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 1.0,
            },
        );
        app.render_frame();
        for (width, height) in [(200u32, 100u32), (300, 180), (400, 250), (260, 140)] {
            gpu.resource_generation += 1;
            app.send_window_event(WindowEvent::Resized(PhysicalSize::new(width, height)));
            let (scale, damage) = app
                .last_frame_result
                .take()
                .expect("the resize event draws a frame");
            let plan = app
                .canvas
                .take_retained_render_plan()
                .expect("the resize frame includes its retained plan");
            let frame = Frame::new(app.canvas.take_draw_list(), width, height);
            let metadata =
                FrameRenderMetadata::new(scale, 0, 0, 0, gpu.resource_generation, damage);
            let packet = FramePacket::with_render_plan(frame, metadata, None, Some(plan));
            let pixels = gpu.present(&packet, width, height);

            for (x, y) in [
                (0, 0),
                (width - 1, 0),
                (0, height - 1),
                (width - 1, height - 1),
            ] {
                let index = ((y * width + x) * 4) as usize;
                assert_eq!(
                    pixels[index..index + 4],
                    [20, 120, 220, 255],
                    "({x}, {y}) of the {width}x{height} window was left unpainted"
                );
            }
        }
    }

    #[test]
    fn windowed_drawer_submits_the_complete_retained_plan_and_no_flat_paint() {
        let paints = Rc::new(Cell::new(0));
        let mut app = AimerApp::start_headless_with(
            MixedPacketWidget {
                paints: paints.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 2.0,
            },
        );
        app.canvas.begin_frame();
        let window = app.window.clone();
        let (_, mut drawer) = app.app.split_for_frame(window);
        let (scale, damage) = drawer.draw(&app.canvas, 200, 100);

        assert_eq!(paints.get(), 1);
        assert_eq!(scale, 2.0);
        assert_eq!(damage.target_size(), (200, 100));
        assert!(!damage.is_empty());
        let plan = app
            .canvas
            .take_retained_render_plan()
            .expect("windowed drawer stores a retained plan");
        assert!(plan.is_complete());
        let mixed_id = retained_element_id_named(&app, "MixedPacketElement")
            .expect("the mixed widget has a retained element identity");
        let mixed_node = app
            .app
            .render_node_for_element(mixed_id)
            .expect("the mixed widget has a retained render node");
        assert!(plan.local_v2_revision(mixed_node).is_some());
        let flat_colors = app
            .canvas
            .draw_list()
            .commands()
            .iter()
            .filter_map(|command| match command {
                aimer_cupid::draw_cmd::DrawCommand::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();
        // The legacy-only packet child has no retained paint, so it draws
        // nothing, and the retained blue travels in the plan, not the flat list.
        assert!(
            flat_colors.is_empty(),
            "the flat list kept paint commands: {flat_colors:?}"
        );
    }

    #[test]
    fn virtualized_scroll_packet_contains_same_frame_v2_rows_and_viewport_damage() {
        let controller = ScrollController::new();
        let paints = Rc::new(Cell::new(0));
        let mut app = AimerApp::start_headless_with(
            HeadlessVirtualizedWidget {
                controller: controller.clone(),
                paints: paints.clone(),
                item_count: 1_000,
                viewport_width: Dimension::Px(100.0),
                viewport_height: Dimension::Px(80.0),
                updater: Rc::new(RefCell::new(None)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(300, 200),
                scale_factor: 1.0,
            },
        );
        let (_, first_packet) = direct_headless_frame_packet(&mut app);
        assert!(
            first_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        assert!(controller.is_attached());
        let initially_painted = paints.get();

        controller.jump_to(Vec2d { x: 0.0, y: 400.0 });
        let rebuild_before_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_frame = aimer_widget::layout_invalidation_generation();
        let (scale, packet) = direct_headless_frame_packet(&mut app);
        assert_eq!(scale, 1.0);
        assert!(!packet.render_plan().unwrap().is_complete());
        let damage = packet.metadata().damage();
        assert_eq!(damage.target_size(), (300, 200));
        assert!(!damage.regions().is_empty());
        // These epochs are process-global. Another parallel unit test can
        // legitimately trigger FrameDrawer's conservative full fallback, so
        // check the viewport-only contract when this frame's epochs are stable.
        let global_invalidation_during_frame = rebuild_before_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_frame != aimer_widget::layout_invalidation_generation();
        if !global_invalidation_during_frame {
            assert!(
                !damage.is_full(),
                "a scroll-window update stays partial: regions={:?}",
                damage.regions()
            );
            assert!(
                damage.regions().iter().all(|region| {
                    region.x + region.width <= 100 && region.y + region.height <= 80
                }),
                "packet damage escaped the scroll viewport: {:?}",
                damage.regions()
            );
        }

        let root = app.app.widget_root.as_ref().unwrap();
        let mut pending = vec![root.as_ref() as &dyn Element];
        let mut visible_row_nodes = Vec::new();
        while let Some(element) = pending.pop() {
            if element.debug_name() == "HeadlessVirtualizedRow"
                && let Some(render_node) = app.app.render_node_for_element(element.id())
                && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                && bounds.x < 100.0
                && bounds.x + bounds.width > 0.0
                && bounds.y < 80.0
                && bounds.y + bounds.height > 0.0
            {
                visible_row_nodes.push(render_node);
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        assert!(
            !visible_row_nodes.is_empty(),
            "the scroll frame has visible rows"
        );
        let plan = packet
            .render_plan()
            .expect("the submitted packet includes its retained plan");
        assert!(!plan.is_complete());
        for node in visible_row_nodes {
            assert_eq!(
                plan.local_v2_revision(node),
                Some(1),
                "visible row {node:?} is present in the submitted plan"
            );
        }
        assert!(
            paints.get() > initially_painted,
            "the newly visible rows were recorded in this frame"
        );
    }

    #[cfg(all(feature = "wgpu", target_os = "macos"))]
    #[test]
    fn actual_headless_virtualized_scroll_packet_renders_and_resizes_on_metal() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        const TARGET_WIDTH: u32 = 200;
        const TARGET_HEIGHT: u32 = 160;
        const RESIZED_WIDTH: u32 = 320;
        const RESIZED_HEIGHT: u32 = 160;
        const APP_RESIZED_WIDTH: u32 = 360;
        const APP_RESIZED_HEIGHT: u32 = 240;

        let require_gpu = std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some();
        let gpu_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create headless-scroll GPU runtime");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = gpu_runtime
            .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: true,
            }))
            .ok();
        let Some(adapter) = adapter else {
            if require_gpu {
                panic!("Metal adapter unavailable; run this test in a host GPU session");
            }
            eprintln!("skipping: Metal adapter unavailable");
            return;
        };
        assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
        let device_queue = gpu_runtime.block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Aimer headless virtualized scroll acceptance test"),
            ..Default::default()
        }));
        let Ok((device, queue)) = device_queue else {
            if require_gpu {
                panic!("could not request a Metal device for the headless scroll test");
            }
            eprintln!("skipping: Metal device unavailable");
            return;
        };

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer headless virtualized scroll target"),
            size: wgpu::Extent3d {
                width: TARGET_WIDTH,
                height: TARGET_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let backend = aimer_cupid::WgpuBackend::new(device.clone(), queue.clone());
        let mut renderer =
            aimer_cupid::renderer::Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);

        let controller = ScrollController::new();
        let paints = Rc::new(Cell::new(0));
        let vertical_updater = Rc::new(RefCell::new(None));
        let mut app = AimerApp::start_headless_with(
            HeadlessVirtualizedWidget {
                controller: controller.clone(),
                paints: paints.clone(),
                item_count: 1_000,
                viewport_width: Dimension::Percent(50.0),
                viewport_height: Dimension::Percent(50.0),
                updater: vertical_updater.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(TARGET_WIDTH, TARGET_HEIGHT),
                scale_factor: 1.0,
            },
        );
        let (_, first_packet) = direct_headless_frame_packet(&mut app);
        assert!(
            first_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        renderer.render_packet(&backend, &view, &first_packet, false);

        let read_target = |target: &wgpu::Texture, width: u32, height: u32| {
            let unpadded_bytes_per_row = width * 4;
            let bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Aimer headless virtualized scroll readback"),
                size: u64::from(bytes_per_row) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Aimer headless virtualized scroll readback encoder"),
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |result| {
                result.expect("map headless-scroll readback buffer");
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("wait for headless-scroll GPU readback");
            let pixels = {
                let mapped = slice
                    .get_mapped_range()
                    .expect("map headless-scroll readback range");
                let mut pixels = vec![0; (unpadded_bytes_per_row * height) as usize];
                for row in 0..height as usize {
                    let source_start = row * bytes_per_row as usize;
                    let target_start = row * unpadded_bytes_per_row as usize;
                    pixels[target_start..target_start + unpadded_bytes_per_row as usize]
                        .copy_from_slice(
                            &mapped[source_start..source_start + unpadded_bytes_per_row as usize],
                        );
                }
                pixels
            };
            readback.unmap();
            pixels
        };
        let read_pixel = |pixels: &[u8], width: u32, x: u32, y: u32| {
            let offset = ((y * width + x) * 4) as usize;
            <[u8; 4]>::try_from(&pixels[offset..offset + 4]).expect("RGBA pixel")
        };
        let color_bytes = |color: Color| {
            let (red, green, blue, alpha) = color.to_rgba();
            [red, green, blue, alpha]
        };

        let row_target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer single-row state update target"),
            size: wgpu::Extent3d {
                width: TARGET_WIDTH,
                height: TARGET_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row_view = row_target.create_view(&wgpu::TextureViewDescriptor::default());
        let row_controller = ScrollController::new();
        let row_paints = Rc::new(Cell::new(0));
        let row_updaters = Rc::new(RefCell::new(HashMap::new()));
        let mut row_app = AimerApp::start_headless_with(
            HeadlessHorizontalStatefulVirtualizedWidget {
                controller: row_controller.clone(),
                paints: row_paints.clone(),
                item_count: 1_000,
                row_updaters: row_updaters.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(TARGET_WIDTH, TARGET_HEIGHT),
                scale_factor: 1.0,
            },
        );
        let (_, row_initial_packet) = direct_headless_frame_packet(&mut row_app);
        assert!(
            row_initial_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let mut row_renderer =
            aimer_cupid::renderer::Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);
        row_renderer.render_packet(&backend, &row_view, &row_initial_packet, false);

        // The event index picks up newly materialized row state owners at the
        // next frame boundary, before the programmatic row update below.
        let (_, indexed_row_packet) = direct_headless_frame_packet(&mut row_app);
        assert!(
            indexed_row_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let indexed_rows = visible_horizontal_rows(&row_app, (100.0, 80.0));
        assert_eq!(indexed_rows.len(), 5);
        row_renderer.render_packet(&backend, &row_view, &indexed_row_packet, false);
        let row_before_pixels = read_target(&row_target, TARGET_WIDTH, TARGET_HEIGHT);
        assert_eq!(
            read_pixel(&row_before_pixels, TARGET_WIDTH, 50, 10),
            color_bytes(Color::BLUE)
        );

        let row_2 = *indexed_rows
            .iter()
            .find(|node| {
                (row_app.app.render_tree().element_bounds(**node).unwrap().x - 40.0).abs()
                    < f32::EPSILON
            })
            .expect("row 2 is visible at x=40");
        let row_2_bounds = row_app.app.render_tree().element_bounds(row_2).unwrap();
        assert_eq!(row_2_bounds, Rect::new(40.0, 0.0, 20.0, 80.0));
        let indexed_plan = indexed_row_packet.render_plan().unwrap();
        let row_revisions = indexed_rows
            .iter()
            .map(|row| {
                (
                    *row,
                    indexed_plan
                        .local_v2_revision(*row)
                        .expect("visible rows keep their local draw lists"),
                )
            })
            .collect::<HashMap<_, _>>();
        let paints_before_row_update = row_paints.get();
        row_updaters
            .borrow()
            .get(&2)
            .expect("the visible row has its own state updater")
            .set_state(|state| state.color = Color::WHITE);
        let rebuild_before_row_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_row_frame = aimer_widget::layout_invalidation_generation();
        let (row_update_scale, row_update_packet) = direct_headless_frame_packet(&mut row_app);
        assert_eq!(row_update_scale, 1.0);
        assert!(
            row_update_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        assert_eq!(row_paints.get(), paints_before_row_update + 1);
        let row_update_damage = row_update_packet.metadata().damage();
        let concurrent_row_invalidation = rebuild_before_row_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_row_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_row_invalidation {
            assert!(
                !row_update_damage.is_full(),
                "one row state update should remain partial: {:?}",
                row_update_damage.regions()
            );
        }
        if !row_update_damage.is_full() {
            assert!(!row_update_damage.regions().is_empty());
            assert!(
                row_update_damage.regions().iter().all(|region| {
                    region.x as f32 >= row_2_bounds.x
                        && region.y as f32 >= row_2_bounds.y
                        && (region.x + region.width) as f32 <= row_2_bounds.x + row_2_bounds.width
                        && (region.y + region.height) as f32 <= row_2_bounds.y + row_2_bounds.height
                }),
                "single-row state damage escaped its element bounds: {:?}",
                row_update_damage.regions()
            );
        }
        let updated_rows = visible_horizontal_rows(&row_app, (100.0, 80.0));
        assert_eq!(updated_rows, indexed_rows);
        let updated_plan = row_update_packet.render_plan().unwrap();
        for row in &updated_rows {
            let expected_revision = row_revisions[row] + u64::from(*row == row_2);
            assert_eq!(
                updated_plan.local_v2_revision(*row),
                Some(expected_revision)
            );
        }
        row_renderer.render_packet(&backend, &row_view, &row_update_packet, false);
        let row_after_pixels = read_target(&row_target, TARGET_WIDTH, TARGET_HEIGHT);
        if !concurrent_row_invalidation && !row_update_damage.is_full() {
            assert!(!row_renderer.compositor_stats().full_repaint);
        }
        assert_eq!(
            read_pixel(&row_after_pixels, TARGET_WIDTH, 50, 10),
            color_bytes(Color::WHITE)
        );
        for y in 0..TARGET_HEIGHT {
            for x in 0..TARGET_WIDTH {
                if !(40..60).contains(&x) || y >= 80 {
                    assert_eq!(
                        read_pixel(&row_after_pixels, TARGET_WIDTH, x, y),
                        read_pixel(&row_before_pixels, TARGET_WIDTH, x, y),
                        "row state update changed a pixel outside row 2 at ({x}, {y})"
                    );
                }
            }
        }

        let old_row_2_bounds = row_app.app.render_tree().element_bounds(row_2).unwrap();
        assert_eq!(old_row_2_bounds, Rect::new(40.0, 0.0, 20.0, 80.0));
        let revisions_before_height_update = updated_rows
            .iter()
            .map(|row| (*row, updated_plan.local_v2_revision(*row).unwrap()))
            .collect::<HashMap<_, _>>();
        let paints_before_height_update = row_paints.get();
        row_updaters
            .borrow()
            .get(&2)
            .expect("the visible row has its own state updater")
            .set_state(|state| state.height = 60.0);
        let rebuild_before_height_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_height_frame = aimer_widget::layout_invalidation_generation();
        let (height_scale, height_packet) = direct_headless_frame_packet(&mut row_app);
        assert_eq!(height_scale, 1.0);
        assert!(
            height_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        assert_eq!(row_paints.get(), paints_before_height_update + 1);

        let height_damage = height_packet.metadata().damage();
        let concurrent_height_invalidation = rebuild_before_height_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_height_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_height_invalidation {
            assert!(
                !height_damage.is_full(),
                "one row height update should remain partial: {:?}",
                height_damage.regions()
            );
        }
        if !height_damage.is_full() {
            assert!(!height_damage.regions().is_empty());
            assert!(
                height_damage.regions().iter().all(|region| {
                    region.x as f32 >= old_row_2_bounds.x
                        && region.y as f32 >= old_row_2_bounds.y
                        && (region.x + region.width) as f32
                            <= old_row_2_bounds.x + old_row_2_bounds.width
                        && (region.y + region.height) as f32
                            <= old_row_2_bounds.y + old_row_2_bounds.height
                }),
                "height-change damage escaped the old/new bounds union: {:?}",
                height_damage.regions()
            );
            assert!(
                height_damage.regions().iter().any(|region| {
                    region.x <= 50
                        && region.x + region.width > 50
                        && region.y <= 70
                        && region.y + region.height > 70
                }),
                "height-change damage missed the cleared old tail: {:?}",
                height_damage.regions()
            );
        }

        let height_rows = visible_horizontal_rows(&row_app, (100.0, 80.0));
        assert_eq!(height_rows, indexed_rows);
        assert_eq!(
            row_app.app.render_tree().element_bounds(row_2).unwrap(),
            Rect::new(40.0, 0.0, 20.0, 60.0)
        );
        let height_plan = height_packet.render_plan().unwrap();
        for row in &height_rows {
            let expected_revision = revisions_before_height_update[row] + u64::from(*row == row_2);
            assert_eq!(height_plan.local_v2_revision(*row), Some(expected_revision));
        }

        row_renderer.render_packet(&backend, &row_view, &height_packet, false);
        let row_after_height_pixels = read_target(&row_target, TARGET_WIDTH, TARGET_HEIGHT);
        if !concurrent_height_invalidation && !height_damage.is_full() {
            assert!(!row_renderer.compositor_stats().full_repaint);
        }
        assert_eq!(
            read_pixel(&row_after_height_pixels, TARGET_WIDTH, 50, 10),
            color_bytes(Color::WHITE)
        );
        assert_eq!(
            read_pixel(&row_after_height_pixels, TARGET_WIDTH, 50, 70),
            [0, 0, 0, 0],
            "the old tail pixels are cleared after the row shrinks"
        );
        for y in 0..TARGET_HEIGHT {
            for x in 0..TARGET_WIDTH {
                if !(40..60).contains(&x) || y >= 80 {
                    assert_eq!(
                        read_pixel(&row_after_height_pixels, TARGET_WIDTH, x, y),
                        read_pixel(&row_after_pixels, TARGET_WIDTH, x, y),
                        "row height update changed a pixel outside the old/new bounds at ({x}, {y})"
                    );
                }
            }
        }

        let shrunk_row_2_bounds = row_app.app.render_tree().element_bounds(row_2).unwrap();
        let revisions_before_grow = height_rows
            .iter()
            .map(|row| (*row, height_plan.local_v2_revision(*row).unwrap()))
            .collect::<HashMap<_, _>>();
        let paints_before_grow = row_paints.get();
        row_updaters
            .borrow()
            .get(&2)
            .expect("the visible row has its own state updater")
            .set_state(|state| state.height = 80.0);
        let rebuild_before_grow_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_grow_frame = aimer_widget::layout_invalidation_generation();
        let (grow_scale, grow_packet) = direct_headless_frame_packet(&mut row_app);
        assert_eq!(grow_scale, 1.0);
        assert!(
            grow_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        assert_eq!(row_paints.get(), paints_before_grow + 1);

        let grow_damage = grow_packet.metadata().damage();
        let concurrent_grow_invalidation = rebuild_before_grow_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_grow_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_grow_invalidation {
            assert!(
                !grow_damage.is_full(),
                "one row height growth should remain partial: {:?}",
                grow_damage.regions()
            );
        }
        if !grow_damage.is_full() {
            assert!(!grow_damage.regions().is_empty());
            assert!(
                grow_damage.regions().iter().all(|region| {
                    region.x as f32 >= shrunk_row_2_bounds.x
                        && region.y as f32 >= shrunk_row_2_bounds.y
                        && (region.x + region.width) as f32
                            <= shrunk_row_2_bounds.x + shrunk_row_2_bounds.width
                        && (region.y + region.height) as f32 <= 80.0
                }),
                "height-growth damage escaped the old/new bounds union: {:?}",
                grow_damage.regions()
            );
            assert!(
                grow_damage.regions().iter().any(|region| {
                    region.x <= 50
                        && region.x + region.width > 50
                        && region.y <= 70
                        && region.y + region.height > 70
                }),
                "height-growth damage missed the newly exposed strip: {:?}",
                grow_damage.regions()
            );
        }

        let grow_rows = visible_horizontal_rows(&row_app, (100.0, 80.0));
        assert_eq!(grow_rows, indexed_rows);
        assert_eq!(
            row_app.app.render_tree().element_bounds(row_2).unwrap(),
            Rect::new(40.0, 0.0, 20.0, 80.0)
        );
        let grow_plan = grow_packet.render_plan().unwrap();
        for row in &grow_rows {
            let expected_revision = revisions_before_grow[row] + u64::from(*row == row_2);
            assert_eq!(grow_plan.local_v2_revision(*row), Some(expected_revision));
        }

        row_renderer.render_packet(&backend, &row_view, &grow_packet, false);
        let row_after_grow_pixels = read_target(&row_target, TARGET_WIDTH, TARGET_HEIGHT);
        if !concurrent_grow_invalidation && !grow_damage.is_full() {
            assert!(!row_renderer.compositor_stats().full_repaint);
        }
        assert_eq!(
            read_pixel(&row_after_grow_pixels, TARGET_WIDTH, 50, 70),
            color_bytes(Color::WHITE),
            "the newly exposed strip is repainted after the row grows"
        );
        for y in 0..TARGET_HEIGHT {
            for x in 0..TARGET_WIDTH {
                if !(40..60).contains(&x) || y >= 80 {
                    assert_eq!(
                        read_pixel(&row_after_grow_pixels, TARGET_WIDTH, x, y),
                        read_pixel(&row_after_height_pixels, TARGET_WIDTH, x, y),
                        "row height growth changed a pixel outside the old/new bounds at ({x}, {y})"
                    );
                }
            }
        }
        drop(row_app);

        let first_pixels = read_target(&target, TARGET_WIDTH, TARGET_HEIGHT);
        let initial_paints = paints.get();
        assert!(initial_paints > 0, "the first frame paints windowed rows");
        let first_plan = first_packet.render_plan().unwrap();
        let initial_visible_rows = {
            let root = app.app.widget_root.as_ref().unwrap();
            let mut visible_rows = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessVirtualizedRow"
                    && let Some(render_node) = app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 100.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 80.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(first_plan.local_v2_revision(render_node), Some(1));
                    visible_rows.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            visible_rows
        };
        assert!(!initial_visible_rows.is_empty());
        assert_eq!(
            read_pixel(&first_pixels, TARGET_WIDTH, 10, 10),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&first_pixels, TARGET_WIDTH, 10, 30),
            color_bytes(Color::GREEN)
        );
        assert_eq!(
            read_pixel(&first_pixels, TARGET_WIDTH, 10, 50),
            color_bytes(Color::BLUE)
        );
        assert_eq!(
            read_pixel(&first_pixels, TARGET_WIDTH, 10, 70),
            color_bytes(Color::WHITE)
        );
        assert_eq!(
            read_pixel(&first_pixels, TARGET_WIDTH, 110, 10),
            [0, 0, 0, 0]
        );
        assert_eq!(
            read_pixel(&first_pixels, TARGET_WIDTH, 10, 90),
            [0, 0, 0, 0]
        );

        controller.jump_to(Vec2d { x: 0.0, y: 400.0 });
        let rebuild_before_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_frame = aimer_widget::layout_invalidation_generation();
        let (scale, second_packet) = direct_headless_frame_packet(&mut app);
        assert_eq!(scale, 1.0);
        assert!(
            second_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let concurrent_global_invalidation = rebuild_before_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_frame != aimer_widget::layout_invalidation_generation();
        let damage = second_packet.metadata().damage();
        assert_eq!(damage.target_size(), (TARGET_WIDTH, TARGET_HEIGHT));
        assert!(!damage.regions().is_empty());
        if !concurrent_global_invalidation {
            assert!(
                !damage.is_full(),
                "stable scroll frame damage: {:?}",
                damage.regions()
            );
            assert!(
                damage.regions().iter().all(|region| {
                    region.x + region.width <= 100 && region.y + region.height <= 80
                }),
                "headless scroll damage escaped its viewport: {:?}",
                damage.regions()
            );
        }

        let plan = second_packet.render_plan().unwrap();
        let visible_rows = {
            let root = app.app.widget_root.as_ref().unwrap();
            let mut visible_rows = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessVirtualizedRow"
                    && let Some(render_node) = app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 100.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 80.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(plan.local_v2_revision(render_node), Some(1));
                    visible_rows.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            visible_rows
        };
        assert!(!visible_rows.is_empty());
        assert_ne!(
            initial_visible_rows, visible_rows,
            "scrolling replaces the row window"
        );
        assert!(
            paints.get() > initial_paints,
            "new rows paint in this frame"
        );

        renderer.render_packet(&backend, &view, &second_packet, false);
        let second_pixels = read_target(&target, TARGET_WIDTH, TARGET_HEIGHT);
        let stats = renderer.compositor_stats();
        if !concurrent_global_invalidation {
            assert!(!stats.full_repaint);
            assert_eq!(stats.damage_regions, 1);
            assert_eq!(stats.damaged_pixels, 100 * 80);
        }
        assert_eq!(
            read_pixel(&second_pixels, TARGET_WIDTH, 10, 10),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&second_pixels, TARGET_WIDTH, 10, 30),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&second_pixels, TARGET_WIDTH, 10, 50),
            color_bytes(Color::CYAN)
        );
        assert_eq!(
            read_pixel(&second_pixels, TARGET_WIDTH, 10, 70),
            color_bytes(Color::ORANGE)
        );
        for y in 0..TARGET_HEIGHT {
            for x in 0..TARGET_WIDTH {
                if x >= 100 || y >= 80 {
                    assert_eq!(
                        read_pixel(&second_pixels, TARGET_WIDTH, x, y),
                        read_pixel(&first_pixels, TARGET_WIDTH, x, y),
                        "headless scrolling changed pixel outside the viewport at ({x}, {y})"
                    );
                }
            }
        }

        let resized_target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer resized headless virtualized scroll target"),
            size: wgpu::Extent3d {
                width: RESIZED_WIDTH,
                height: RESIZED_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let resized_view = resized_target.create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render_packet_at_size(
            &backend,
            &resized_view,
            &second_packet,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            false,
        );
        let resized_stats = renderer.compositor_stats();
        assert!(
            resized_stats.full_repaint,
            "a resized target requires full damage"
        );
        assert_eq!(resized_stats.damage_regions, 1);
        assert_eq!(
            resized_stats.damaged_pixels,
            u64::from(RESIZED_WIDTH) * u64::from(RESIZED_HEIGHT)
        );

        let resized_pixels = read_target(&resized_target, RESIZED_WIDTH, RESIZED_HEIGHT);
        assert_eq!(
            read_pixel(&resized_pixels, RESIZED_WIDTH, 10, 10),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&resized_pixels, RESIZED_WIDTH, 10, 30),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&resized_pixels, RESIZED_WIDTH, 10, 50),
            color_bytes(Color::CYAN)
        );
        assert_eq!(
            read_pixel(&resized_pixels, RESIZED_WIDTH, 10, 70),
            color_bytes(Color::ORANGE)
        );
        for y in 0..RESIZED_HEIGHT {
            for x in 0..RESIZED_WIDTH {
                let resized_pixel = read_pixel(&resized_pixels, RESIZED_WIDTH, x, y);
                if x < TARGET_WIDTH && y < TARGET_HEIGHT {
                    assert_eq!(
                        resized_pixel,
                        read_pixel(&second_pixels, TARGET_WIDTH, x, y),
                        "resize changed the complete old frame at ({x}, {y})"
                    );
                } else {
                    assert_eq!(
                        resized_pixel,
                        [0, 0, 0, 0],
                        "resized target was not cleared outside the submitted frame at ({x}, {y})"
                    );
                }
            }
        }

        app.send_window_event(WindowEvent::Resized(PhysicalSize::new(
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        )));
        assert_eq!(
            app.physical_size(),
            PhysicalSize::new(APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        let (resize_scale, resize_damage) = app
            .last_frame_result
            .take()
            .expect("the headless resize event draws a new frame");
        assert_eq!(resize_scale, 1.0);
        assert_eq!(
            resize_damage.target_size(),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        assert!(
            resize_damage.is_full(),
            "resizing the headless target is full damage"
        );
        let app_resized_packet = headless_packet_from_canvas(&mut app, resize_scale, resize_damage);
        assert_eq!(
            (
                app_resized_packet.frame().width,
                app_resized_packet.frame().height
            ),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        assert!(
            app_resized_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );

        let app_resized_plan = app_resized_packet.render_plan().unwrap();
        let (resized_viewport, app_resized_rows) = {
            let root = app.app.widget_root.as_ref().unwrap();
            let mut viewport_bounds = None;
            let mut visible_rows = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "RawScrollableContainer"
                    && let Some(render_node) = app.app.render_node_for_element(element.id())
                {
                    viewport_bounds = Some(
                        app.app
                            .render_tree()
                            .element_bounds(render_node)
                            .expect("the resized scroll viewport has retained bounds"),
                    );
                }
                if element.debug_name() == "HeadlessVirtualizedRow"
                    && let Some(render_node) = app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 180.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 120.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(app_resized_plan.local_v2_revision(render_node), Some(1));
                    visible_rows.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            (
                viewport_bounds.expect("the resized tree retains its scroll viewport"),
                visible_rows,
            )
        };
        assert_eq!(resized_viewport, Rect::new(0.0, 0.0, 180.0, 120.0));
        assert_eq!(
            app_resized_rows.len(),
            6,
            "the taller viewport materializes six rows"
        );
        assert!(
            app_resized_rows.len() > visible_rows.len(),
            "resizing exposes additional virtualized rows"
        );
        assert!(paints.get() > initial_paints);

        let app_resized_target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer resized Quiver application target"),
            size: wgpu::Extent3d {
                width: APP_RESIZED_WIDTH,
                height: APP_RESIZED_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let app_resized_view =
            app_resized_target.create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render_packet(&backend, &app_resized_view, &app_resized_packet, false);
        let app_resized_stats = renderer.compositor_stats();
        assert!(app_resized_stats.full_repaint);
        assert_eq!(app_resized_stats.damage_regions, 1);
        assert_eq!(
            app_resized_stats.damaged_pixels,
            u64::from(APP_RESIZED_WIDTH) * u64::from(APP_RESIZED_HEIGHT)
        );

        let app_resized_pixels =
            read_target(&app_resized_target, APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT);
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 30),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 50),
            color_bytes(Color::CYAN)
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 70),
            color_bytes(Color::ORANGE)
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 90),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 110),
            color_bytes(Color::GREEN)
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 120, 10),
            [0, 0, 0, 0]
        );
        assert_eq!(
            read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, 10, 130),
            [0, 0, 0, 0]
        );
        for y in 0..APP_RESIZED_HEIGHT {
            for x in 0..APP_RESIZED_WIDTH {
                if x >= 180 || y >= 120 {
                    assert_eq!(
                        read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, x, y),
                        [0, 0, 0, 0],
                        "resized Quiver content escaped the viewport at ({x}, {y})"
                    );
                }
            }
        }

        let max_extent = controller.max_extent();
        assert!(max_extent.y > 0.0, "the list has a reachable content end");
        controller.jump_to(max_extent);
        let rebuild_before_end_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_end_frame = aimer_widget::layout_invalidation_generation();
        let (end_scale, end_packet) = direct_headless_frame_packet(&mut app);
        assert_eq!(end_scale, 1.0);
        assert!(
            end_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let end_damage = end_packet.metadata().damage();
        assert_eq!(
            end_damage.target_size(),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        assert!(!end_damage.regions().is_empty());
        let concurrent_global_invalidation = rebuild_before_end_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_end_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_global_invalidation {
            assert!(
                !end_damage.is_full(),
                "end-scroll damage: {:?}",
                end_damage.regions()
            );
            assert!(
                end_damage.regions().iter().all(|region| {
                    region.x + region.width <= 180 && region.y + region.height <= 120
                }),
                "end-scroll damage escaped the resized viewport: {:?}",
                end_damage.regions()
            );
        }

        let end_plan = end_packet.render_plan().unwrap();
        let end_visible_rows = {
            let root = app.app.widget_root.as_ref().unwrap();
            let mut visible_rows = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessVirtualizedRow"
                    && let Some(render_node) = app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 180.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 120.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(end_plan.local_v2_revision(render_node), Some(1));
                    visible_rows.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            visible_rows
        };
        assert_eq!(
            end_visible_rows.len(),
            6,
            "six rows fill the resized viewport"
        );
        assert_ne!(
            app_resized_rows, end_visible_rows,
            "the row window moves to the list end"
        );
        let actual_offset = controller.offset();
        assert!((actual_offset.y - max_extent.y).abs() < 0.01);
        assert!(paints.get() > initial_paints);

        renderer.render_packet(&backend, &app_resized_view, &end_packet, false);
        let end_pixels = read_target(&app_resized_target, APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT);
        let end_stats = renderer.compositor_stats();
        if !concurrent_global_invalidation {
            assert!(!end_stats.full_repaint);
            assert_eq!(end_stats.damage_regions, 1);
            assert_eq!(end_stats.damaged_pixels, 180 * 120);
        }
        assert_eq!(
            read_pixel(&end_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::BLUE)
        );
        assert_eq!(
            read_pixel(&end_pixels, APP_RESIZED_WIDTH, 10, 30),
            color_bytes(Color::WHITE)
        );
        assert_eq!(
            read_pixel(&end_pixels, APP_RESIZED_WIDTH, 10, 50),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&end_pixels, APP_RESIZED_WIDTH, 10, 70),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&end_pixels, APP_RESIZED_WIDTH, 10, 90),
            color_bytes(Color::CYAN)
        );
        assert_eq!(
            read_pixel(&end_pixels, APP_RESIZED_WIDTH, 10, 110),
            color_bytes(Color::ORANGE)
        );
        for y in 0..APP_RESIZED_HEIGHT {
            for x in 0..APP_RESIZED_WIDTH {
                if x >= 180 || y >= 120 {
                    assert_eq!(
                        read_pixel(&end_pixels, APP_RESIZED_WIDTH, x, y),
                        read_pixel(&app_resized_pixels, APP_RESIZED_WIDTH, x, y),
                        "end scrolling changed pixels outside the resized viewport at ({x}, {y})"
                    );
                }
            }
        }

        let paints_before_vertical_shrink = paints.get();
        let find_vertical_scroll_id = |root: &dyn Element| {
            let mut pending = vec![root];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "RawScrollableContainer" {
                    return Some(element.id());
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            None
        };
        let root_before_vertical_shrink = app.app.widget_root.as_ref().unwrap();
        let root_id_before_vertical_shrink = root_before_vertical_shrink.id();
        let scroll_id_before_vertical_shrink =
            find_vertical_scroll_id(root_before_vertical_shrink.as_ref())
                .expect("the stateful rows are inside the scrollable");
        let scroll_node_before_vertical_shrink = app
            .app
            .render_node_for_element(scroll_id_before_vertical_shrink)
            .expect("the vertical viewport has a retained render node");
        vertical_updater
            .borrow()
            .as_ref()
            .expect("the vertical row state exposes its updater")
            .set_state(|state| state.item_count = 3);
        let rebuild_before_vertical_shrink = aimer_widget::rebuild_invalidation_generation();
        let layout_before_vertical_shrink = aimer_widget::layout_invalidation_generation();
        let (shrink_scale, vertical_shrink_packet) = direct_headless_frame_packet(&mut app);
        assert_eq!(shrink_scale, 1.0);
        assert!(
            vertical_shrink_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let root_after_vertical_shrink = app.app.widget_root.as_ref().unwrap();
        assert_eq!(
            root_after_vertical_shrink.id(),
            root_id_before_vertical_shrink
        );
        let scroll_id_after_vertical_shrink =
            find_vertical_scroll_id(root_after_vertical_shrink.as_ref())
                .expect("the in-place update retains the vertical scrollable");
        assert_eq!(
            scroll_id_after_vertical_shrink,
            scroll_id_before_vertical_shrink
        );
        assert_eq!(
            app.app
                .render_node_for_element(scroll_id_after_vertical_shrink),
            Some(scroll_node_before_vertical_shrink),
            "the in-place update retains the vertical viewport render node"
        );
        let vertical_shrink_damage = vertical_shrink_packet.metadata().damage();
        assert_eq!(
            vertical_shrink_damage.target_size(),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        assert!(!vertical_shrink_damage.regions().is_empty());
        let concurrent_global_invalidation = rebuild_before_vertical_shrink
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_vertical_shrink != aimer_widget::layout_invalidation_generation();
        if !vertical_shrink_damage.is_full() {
            assert!(
                vertical_shrink_damage.regions().iter().all(|region| {
                    region.x + region.width <= 180 && region.y + region.height <= 120
                }),
                "vertical list shrink damage escaped its viewport: {:?}",
                vertical_shrink_damage.regions()
            );
        }
        assert!(controller.is_attached());
        assert_eq!(controller.max_extent().y, 0.0);
        assert_eq!(controller.offset().y, 0.0);

        let vertical_shrink_plan = vertical_shrink_packet.render_plan().unwrap();
        let vertical_shrink_rows = {
            let root = app.app.widget_root.as_ref().unwrap();
            let mut rows = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessVirtualizedRow"
                    && let Some(render_node) = app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 180.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 120.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(vertical_shrink_plan.local_v2_revision(render_node), Some(1));
                    rows.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            rows
        };
        assert_eq!(vertical_shrink_rows.len(), 3);
        assert!(paints.get() > paints_before_vertical_shrink);

        renderer.render_packet(&backend, &app_resized_view, &vertical_shrink_packet, false);
        let vertical_shrink_pixels =
            read_target(&app_resized_target, APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT);
        let vertical_shrink_stats = renderer.compositor_stats();
        if !concurrent_global_invalidation && !vertical_shrink_damage.is_full() {
            assert!(!vertical_shrink_stats.full_repaint);
        }
        assert_eq!(
            read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, 10, 30),
            color_bytes(Color::GREEN)
        );
        assert_eq!(
            read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, 10, 50),
            color_bytes(Color::BLUE)
        );
        assert_eq!(
            read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, 10, 70),
            [0, 0, 0, 0]
        );
        assert_eq!(
            read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, 10, 110),
            [0, 0, 0, 0]
        );
        for y in 60..120 {
            for x in 0..180 {
                assert_eq!(
                    read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, x, y),
                    [0, 0, 0, 0],
                    "a removed bottom row left stale pixels at ({x}, {y})"
                );
            }
        }
        for y in 0..APP_RESIZED_HEIGHT {
            for x in 0..APP_RESIZED_WIDTH {
                if x >= 180 || y >= 120 {
                    assert_eq!(
                        read_pixel(&vertical_shrink_pixels, APP_RESIZED_WIDTH, x, y),
                        read_pixel(&end_pixels, APP_RESIZED_WIDTH, x, y),
                        "vertical list shrink changed pixels outside the viewport at ({x}, {y})"
                    );
                }
            }
        }

        drop(app);
        let horizontal_controller = ScrollController::new();
        let horizontal_paints = Rc::new(Cell::new(0));
        let horizontal_updater = Rc::new(RefCell::new(None));
        let mut horizontal_app = AimerApp::start_headless_with(
            HeadlessHorizontalVirtualizedWidget {
                controller: horizontal_controller.clone(),
                paints: horizontal_paints.clone(),
                item_count: 1_000,
                updater: horizontal_updater.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(TARGET_WIDTH, TARGET_HEIGHT),
                scale_factor: 1.0,
            },
        );
        let (_, horizontal_first_packet) = direct_headless_frame_packet(&mut horizontal_app);
        assert!(
            horizontal_first_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let mut horizontal_renderer =
            aimer_cupid::renderer::Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);
        horizontal_renderer.render_packet(&backend, &view, &horizontal_first_packet, false);
        let horizontal_first_pixels = read_target(&target, TARGET_WIDTH, TARGET_HEIGHT);
        let horizontal_initial_paints = horizontal_paints.get();
        let horizontal_first_plan = horizontal_first_packet.render_plan().unwrap();
        let horizontal_first_items = {
            let root = horizontal_app.app.widget_root.as_ref().unwrap();
            let mut visible_items = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessHorizontalVirtualizedItem"
                    && let Some(render_node) =
                        horizontal_app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = horizontal_app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 100.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 80.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(
                        horizontal_first_plan.local_v2_revision(render_node),
                        Some(1)
                    );
                    visible_items.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            visible_items
        };
        assert_eq!(horizontal_first_items.len(), 5);
        assert_eq!(horizontal_controller.max_extent().y, 0.0);
        assert_eq!(
            read_pixel(&horizontal_first_pixels, TARGET_WIDTH, 10, 10),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&horizontal_first_pixels, TARGET_WIDTH, 30, 10),
            color_bytes(Color::GREEN)
        );
        assert_eq!(
            read_pixel(&horizontal_first_pixels, TARGET_WIDTH, 50, 10),
            color_bytes(Color::BLUE)
        );
        assert_eq!(
            read_pixel(&horizontal_first_pixels, TARGET_WIDTH, 70, 10),
            color_bytes(Color::WHITE)
        );
        assert_eq!(
            read_pixel(&horizontal_first_pixels, TARGET_WIDTH, 90, 10),
            color_bytes(Color::YELLOW)
        );

        let horizontal_max = horizontal_controller.max_extent();
        assert!(
            horizontal_max.x > 0.0,
            "the horizontal list has a reachable end"
        );
        horizontal_controller.jump_to(horizontal_max);
        let rebuild_before_horizontal_end = aimer_widget::rebuild_invalidation_generation();
        let layout_before_horizontal_end = aimer_widget::layout_invalidation_generation();
        let (horizontal_scale, horizontal_end_packet) =
            direct_headless_frame_packet(&mut horizontal_app);
        assert_eq!(horizontal_scale, 1.0);
        assert!(
            horizontal_end_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let horizontal_end_damage = horizontal_end_packet.metadata().damage();
        assert_eq!(
            horizontal_end_damage.target_size(),
            (TARGET_WIDTH, TARGET_HEIGHT)
        );
        assert!(!horizontal_end_damage.regions().is_empty());
        let concurrent_global_invalidation = rebuild_before_horizontal_end
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_horizontal_end != aimer_widget::layout_invalidation_generation();
        if !concurrent_global_invalidation {
            assert!(
                !horizontal_end_damage.is_full(),
                "horizontal end-scroll damage: {:?}",
                horizontal_end_damage.regions()
            );
            assert!(
                horizontal_end_damage.regions().iter().all(|region| {
                    region.x + region.width <= 100 && region.y + region.height <= 80
                }),
                "horizontal damage escaped its viewport: {:?}",
                horizontal_end_damage.regions()
            );
        }

        let horizontal_end_plan = horizontal_end_packet.render_plan().unwrap();
        let horizontal_end_items = {
            let root = horizontal_app.app.widget_root.as_ref().unwrap();
            let mut visible_items = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessHorizontalVirtualizedItem"
                    && let Some(render_node) =
                        horizontal_app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = horizontal_app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 100.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 80.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(horizontal_end_plan.local_v2_revision(render_node), Some(1));
                    visible_items.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            visible_items
        };
        assert_eq!(horizontal_end_items.len(), 5);
        assert_ne!(horizontal_first_items, horizontal_end_items);
        let horizontal_offset = horizontal_controller.offset();
        assert!((horizontal_offset.x - horizontal_max.x).abs() < 0.01);
        assert!(horizontal_paints.get() > horizontal_initial_paints);

        horizontal_renderer.render_packet(&backend, &view, &horizontal_end_packet, false);
        let horizontal_end_pixels = read_target(&target, TARGET_WIDTH, TARGET_HEIGHT);
        let horizontal_stats = horizontal_renderer.compositor_stats();
        if !concurrent_global_invalidation {
            assert!(!horizontal_stats.full_repaint);
            assert_eq!(horizontal_stats.damage_regions, 1);
            assert_eq!(horizontal_stats.damaged_pixels, 100 * 80);
        }
        assert_eq!(
            read_pixel(&horizontal_end_pixels, TARGET_WIDTH, 10, 10),
            color_bytes(Color::WHITE)
        );
        assert_eq!(
            read_pixel(&horizontal_end_pixels, TARGET_WIDTH, 30, 10),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&horizontal_end_pixels, TARGET_WIDTH, 50, 10),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&horizontal_end_pixels, TARGET_WIDTH, 70, 10),
            color_bytes(Color::CYAN)
        );
        assert_eq!(
            read_pixel(&horizontal_end_pixels, TARGET_WIDTH, 90, 10),
            color_bytes(Color::ORANGE)
        );
        for y in 0..TARGET_HEIGHT {
            for x in 0..TARGET_WIDTH {
                if x >= 100 || y >= 80 {
                    assert_eq!(
                        read_pixel(&horizontal_end_pixels, TARGET_WIDTH, x, y),
                        read_pixel(&horizontal_first_pixels, TARGET_WIDTH, x, y),
                        "horizontal scrolling changed pixels outside its viewport at ({x}, {y})"
                    );
                }
            }
        }

        let old_horizontal_max = horizontal_controller.max_extent();
        horizontal_app.send_window_event(WindowEvent::Resized(PhysicalSize::new(
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        )));
        assert_eq!(
            horizontal_app.physical_size(),
            PhysicalSize::new(APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        let (horizontal_resize_scale, horizontal_resize_damage) = horizontal_app
            .last_frame_result
            .take()
            .expect("the horizontal headless resize event draws a frame");
        assert_eq!(horizontal_resize_scale, 1.0);
        assert_eq!(
            horizontal_resize_damage.target_size(),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        assert!(horizontal_resize_damage.is_full());
        let horizontal_resize_packet = headless_packet_from_canvas(
            &mut horizontal_app,
            horizontal_resize_scale,
            horizontal_resize_damage,
        );
        assert!(
            horizontal_resize_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );

        let new_horizontal_max = horizontal_controller.max_extent();
        assert!((old_horizontal_max.x - new_horizontal_max.x - 80.0).abs() < 0.01);
        assert_eq!(new_horizontal_max.y, 0.0);
        let clamped_offset = horizontal_controller.offset();
        assert!((clamped_offset.x - new_horizontal_max.x).abs() < 0.01);
        let new_horizontal_plan = horizontal_resize_packet.render_plan().unwrap();
        let (new_horizontal_viewport, new_horizontal_items) = {
            let root = horizontal_app.app.widget_root.as_ref().unwrap();
            let mut viewport_bounds = None;
            let mut visible_items = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "RawScrollableContainer"
                    && let Some(render_node) =
                        horizontal_app.app.render_node_for_element(element.id())
                {
                    viewport_bounds = Some(
                        horizontal_app
                            .app
                            .render_tree()
                            .element_bounds(render_node)
                            .expect("the resized horizontal viewport has retained bounds"),
                    );
                }
                if element.debug_name() == "HeadlessHorizontalVirtualizedItem"
                    && let Some(render_node) =
                        horizontal_app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = horizontal_app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 180.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 120.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(new_horizontal_plan.local_v2_revision(render_node), Some(1));
                    visible_items.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            (
                viewport_bounds.expect("the resized horizontal tree keeps its viewport"),
                visible_items,
            )
        };
        assert_eq!(new_horizontal_viewport, Rect::new(0.0, 0.0, 180.0, 120.0));
        assert_eq!(new_horizontal_items.len(), 9);
        assert!(new_horizontal_items.len() > horizontal_end_items.len());
        assert!(horizontal_paints.get() > horizontal_initial_paints);

        let horizontal_resized_target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer resized horizontal Quiver target at list end"),
            size: wgpu::Extent3d {
                width: APP_RESIZED_WIDTH,
                height: APP_RESIZED_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let horizontal_resized_view =
            horizontal_resized_target.create_view(&wgpu::TextureViewDescriptor::default());
        horizontal_renderer.render_packet(
            &backend,
            &horizontal_resized_view,
            &horizontal_resize_packet,
            false,
        );
        let horizontal_resize_stats = horizontal_renderer.compositor_stats();
        assert!(horizontal_resize_stats.full_repaint);
        assert_eq!(horizontal_resize_stats.damage_regions, 1);
        assert_eq!(
            horizontal_resize_stats.damaged_pixels,
            u64::from(APP_RESIZED_WIDTH) * u64::from(APP_RESIZED_HEIGHT)
        );

        let horizontal_resized_pixels = read_target(
            &horizontal_resized_target,
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::ORANGE)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 30, 10),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 50, 10),
            color_bytes(Color::GREEN)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 70, 10),
            color_bytes(Color::BLUE)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 90, 10),
            color_bytes(Color::WHITE)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 110, 10),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 130, 10),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 150, 10),
            color_bytes(Color::CYAN)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 170, 10),
            color_bytes(Color::ORANGE)
        );
        assert_eq!(
            read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, 10, 90),
            [0, 0, 0, 0]
        );
        for y in 0..APP_RESIZED_HEIGHT {
            for x in 0..APP_RESIZED_WIDTH {
                if x >= 180 || y >= 120 {
                    assert_eq!(
                        read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, x, y),
                        [0, 0, 0, 0],
                        "resized horizontal content escaped its viewport at ({x}, {y})"
                    );
                }
            }
        }

        let old_end_offset = horizontal_controller.offset();
        assert!((old_end_offset.x - new_horizontal_max.x).abs() < 0.01);
        let find_scroll_element_id = |root: &dyn Element| {
            let mut pending = vec![root];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "RawScrollableContainer" {
                    return Some(element.id());
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            None
        };
        let root_before_shrink = horizontal_app.app.widget_root.as_ref().unwrap();
        let root_element_id_before_shrink = root_before_shrink.id();
        let scroll_element_id_before_shrink = find_scroll_element_id(root_before_shrink.as_ref())
            .expect("the stateful rows remain inside the scrollable");
        let scroll_render_node_before_shrink = horizontal_app
            .app
            .render_node_for_element(scroll_element_id_before_shrink)
            .expect("the scroll viewport has a retained render node");
        let paints_before_shrink = horizontal_paints.get();
        horizontal_updater
            .borrow()
            .as_ref()
            .expect("the horizontal row state exposes its updater")
            .set_state(|state| state.item_count = 3);
        let rebuild_before_shrink = aimer_widget::rebuild_invalidation_generation();
        let layout_before_shrink = aimer_widget::layout_invalidation_generation();
        let (shrink_scale, shrink_packet) = direct_headless_frame_packet(&mut horizontal_app);
        assert_eq!(shrink_scale, 1.0);
        assert!(
            shrink_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let root_after_shrink = horizontal_app.app.widget_root.as_ref().unwrap();
        assert_eq!(root_after_shrink.id(), root_element_id_before_shrink);
        let scroll_element_id_after_shrink = find_scroll_element_id(root_after_shrink.as_ref())
            .expect("the in-place update retains the scrollable element");
        assert_eq!(
            scroll_element_id_after_shrink,
            scroll_element_id_before_shrink
        );
        assert_eq!(
            horizontal_app
                .app
                .render_node_for_element(scroll_element_id_after_shrink),
            Some(scroll_render_node_before_shrink),
            "the in-place update retains the viewport render node"
        );
        let shrink_damage = shrink_packet.metadata().damage();
        assert_eq!(
            shrink_damage.target_size(),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        assert!(!shrink_damage.regions().is_empty());
        let concurrent_global_invalidation = rebuild_before_shrink
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_shrink != aimer_widget::layout_invalidation_generation();
        if !shrink_damage.is_full() {
            assert!(
                shrink_damage.regions().iter().all(|region| {
                    region.x + region.width <= 180 && region.y + region.height <= 120
                }),
                "list shrink damage escaped the horizontal viewport: {:?}",
                shrink_damage.regions()
            );
        }

        assert!(horizontal_controller.is_attached());
        assert_eq!(horizontal_controller.max_extent().x, 0.0);
        assert_eq!(horizontal_controller.offset().x, 0.0);
        let shrink_plan = shrink_packet.render_plan().unwrap();
        let shrink_items = {
            let root = horizontal_app.app.widget_root.as_ref().unwrap();
            let mut items = Vec::new();
            let mut pending = vec![root.as_ref() as &dyn Element];
            while let Some(element) = pending.pop() {
                if element.debug_name() == "HeadlessHorizontalVirtualizedItem"
                    && let Some(render_node) =
                        horizontal_app.app.render_node_for_element(element.id())
                    && let Ok(bounds) = horizontal_app.app.render_tree().element_bounds(render_node)
                    && bounds.x < 180.0
                    && bounds.x + bounds.width > 0.0
                    && bounds.y < 120.0
                    && bounds.y + bounds.height > 0.0
                {
                    assert_eq!(shrink_plan.local_v2_revision(render_node), Some(1));
                    items.push(render_node);
                }
                element.visit_children(&mut |child| pending.push(child));
            }
            items
        };
        assert_eq!(
            shrink_items.len(),
            3,
            "the reduced row source retains only three items"
        );
        assert!(horizontal_paints.get() > paints_before_shrink);

        horizontal_renderer.render_packet(
            &backend,
            &horizontal_resized_view,
            &shrink_packet,
            false,
        );
        let shrink_pixels = read_target(
            &horizontal_resized_target,
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        );
        let shrink_stats = horizontal_renderer.compositor_stats();
        if !concurrent_global_invalidation {
            assert!(!shrink_stats.full_repaint || shrink_damage.is_full());
        }
        assert_eq!(
            read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, 30, 10),
            color_bytes(Color::GREEN)
        );
        assert_eq!(
            read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, 50, 10),
            color_bytes(Color::BLUE)
        );
        assert_eq!(
            read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, 70, 10),
            [0, 0, 0, 0]
        );
        assert_eq!(
            read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, 170, 10),
            [0, 0, 0, 0]
        );
        for y in 0..APP_RESIZED_HEIGHT {
            for x in 0..APP_RESIZED_WIDTH {
                if (60..180).contains(&x) && y < 80 {
                    assert_eq!(
                        read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, x, y),
                        [0, 0, 0, 0],
                        "a removed end item left stale pixels at ({x}, {y})"
                    );
                }
                if x >= 180 || y >= 120 {
                    assert_eq!(
                        read_pixel(&shrink_pixels, APP_RESIZED_WIDTH, x, y),
                        read_pixel(&horizontal_resized_pixels, APP_RESIZED_WIDTH, x, y),
                        "shrinking changed pixels outside the viewport at ({x}, {y})"
                    );
                }
            }
        }

        horizontal_updater
            .borrow()
            .as_ref()
            .expect("the horizontal row state updater remains live")
            .set_state(|state| state.item_count = 1_000);
        let (_, restored_packet) = direct_headless_frame_packet(&mut horizontal_app);
        assert!(
            restored_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        assert!(horizontal_controller.max_extent().x > 400.0);
        assert_eq!(horizontal_controller.offset().x, 0.0);
        horizontal_renderer.render_packet(
            &backend,
            &horizontal_resized_view,
            &restored_packet,
            false,
        );
        let restored_pixels = read_target(
            &horizontal_resized_target,
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        );
        assert_eq!(
            read_pixel(&restored_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::RED)
        );
        assert_eq!(
            read_pixel(&restored_pixels, APP_RESIZED_WIDTH, 170, 10),
            color_bytes(Color::RED)
        );

        horizontal_controller.jump_to(Vec2d { x: 400.0, y: 0.0 });
        let (middle_scale, middle_packet) = direct_headless_frame_packet(&mut horizontal_app);
        assert_eq!(middle_scale, 1.0);
        assert!(
            middle_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        assert_eq!(horizontal_controller.offset().x, 400.0);
        let middle_rows = visible_horizontal_rows(&horizontal_app, (180.0, 120.0));
        assert_eq!(middle_rows.len(), 9);
        horizontal_renderer.render_packet(
            &backend,
            &horizontal_resized_view,
            &middle_packet,
            false,
        );
        let middle_pixels = read_target(
            &horizontal_resized_target,
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        );
        assert_eq!(
            read_pixel(&middle_pixels, APP_RESIZED_WIDTH, 10, 10),
            color_bytes(Color::YELLOW)
        );
        assert_eq!(
            read_pixel(&middle_pixels, APP_RESIZED_WIDTH, 30, 10),
            color_bytes(Color::MAGENTA)
        );
        assert_eq!(
            read_pixel(&middle_pixels, APP_RESIZED_WIDTH, 50, 10),
            color_bytes(Color::CYAN)
        );

        let max_extent_before_middle_shrink = horizontal_controller.max_extent();
        assert!((max_extent_before_middle_shrink.x - horizontal_controller.offset().x) > 0.0);
        let root_before_middle_shrink = horizontal_app.app.widget_root.as_ref().unwrap();
        let root_id_before_middle_shrink = root_before_middle_shrink.id();
        let scroll_id_before_middle_shrink =
            find_scroll_element_id(root_before_middle_shrink.as_ref())
                .expect("middle-position state belongs to the retained scrollable");
        let scroll_node_before_middle_shrink = horizontal_app
            .app
            .render_node_for_element(scroll_id_before_middle_shrink)
            .expect("middle-position scrollable has a retained render node");
        horizontal_updater
            .borrow()
            .as_ref()
            .expect("the horizontal row state updater remains live")
            .set_state(|state| state.item_count = 500);
        let rebuild_before_middle_shrink = aimer_widget::rebuild_invalidation_generation();
        let layout_before_middle_shrink = aimer_widget::layout_invalidation_generation();
        let (middle_shrink_scale, middle_shrink_packet) =
            direct_headless_frame_packet(&mut horizontal_app);
        assert_eq!(middle_shrink_scale, 1.0);
        assert!(
            middle_shrink_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );
        let middle_shrink_damage = middle_shrink_packet.metadata().damage();
        assert_eq!(
            middle_shrink_damage.target_size(),
            (APP_RESIZED_WIDTH, APP_RESIZED_HEIGHT)
        );
        let concurrent_global_invalidation = rebuild_before_middle_shrink
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_middle_shrink != aimer_widget::layout_invalidation_generation();
        if !concurrent_global_invalidation {
            assert!(
                !middle_shrink_damage.is_full(),
                "a valid middle offset keeps list-shrink damage partial: {:?}",
                middle_shrink_damage.regions()
            );
        }
        if !middle_shrink_damage.is_full() {
            assert!(
                middle_shrink_damage.regions().iter().all(|region| {
                    region.x + region.width <= 180 && region.y + region.height <= 120
                }),
                "middle list shrink damage escaped the viewport: {:?}",
                middle_shrink_damage.regions()
            );
        }
        assert_eq!(horizontal_controller.offset().x, 400.0);
        assert!((horizontal_controller.max_extent().x - 9_820.0).abs() < 0.01);
        let root_after_middle_shrink = horizontal_app.app.widget_root.as_ref().unwrap();
        assert_eq!(root_after_middle_shrink.id(), root_id_before_middle_shrink);
        let scroll_id_after_middle_shrink =
            find_scroll_element_id(root_after_middle_shrink.as_ref())
                .expect("the middle shrink keeps the scrollable element");
        assert_eq!(
            scroll_id_after_middle_shrink,
            scroll_id_before_middle_shrink
        );
        assert_eq!(
            horizontal_app
                .app
                .render_node_for_element(scroll_id_after_middle_shrink),
            Some(scroll_node_before_middle_shrink)
        );
        let middle_shrink_rows = visible_horizontal_rows(&horizontal_app, (180.0, 120.0));
        assert_eq!(middle_shrink_rows.len(), 9);
        assert_eq!(
            middle_shrink_rows, middle_rows,
            "visible row identities are retained"
        );

        horizontal_renderer.render_packet(
            &backend,
            &horizontal_resized_view,
            &middle_shrink_packet,
            false,
        );
        let middle_shrink_pixels = read_target(
            &horizontal_resized_target,
            APP_RESIZED_WIDTH,
            APP_RESIZED_HEIGHT,
        );
        let middle_shrink_stats = horizontal_renderer.compositor_stats();
        if !concurrent_global_invalidation && !middle_shrink_damage.is_full() {
            assert!(!middle_shrink_stats.full_repaint);
        }
        for y in 0..APP_RESIZED_HEIGHT {
            for x in 0..APP_RESIZED_WIDTH {
                assert_eq!(
                    read_pixel(&middle_shrink_pixels, APP_RESIZED_WIDTH, x, y),
                    read_pixel(&middle_pixels, APP_RESIZED_WIDTH, x, y),
                    "a middle-position list shrink changed pixel ({x}, {y})"
                );
            }
        }
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[test]
    fn reload_listener_readiness_is_canonical_and_secret_free() {
        let line =
            reload_listener_readiness_line([0x11; 16], "127.0.0.1:37654".parse().unwrap(), 4242);

        assert_eq!(
            line,
            "AIMER_RELOAD_LISTENER_READY session=11111111111111111111111111111111 port=37654 pid=4242 protocol=1.0"
        );
        assert!(!line.contains("token"));
    }

    #[test]
    fn pending_frame_ready_requests_are_coalesced_until_delivery() {
        let pending = AtomicU8::new(0);

        assert!(try_begin_frame_ready_request(
            &pending,
            FrameRequestKind::ScrollOnly
        ));
        assert!(!try_begin_frame_ready_request(
            &pending,
            FrameRequestKind::ScrollOnly
        ));
        assert!(!try_begin_frame_ready_request(
            &pending,
            FrameRequestKind::Full
        ));

        assert_eq!(
            complete_frame_ready_request(&pending),
            FrameRequestKind::Full,
            "a coalesced full request must promote a pending scroll tick"
        );

        assert!(try_begin_frame_ready_request(
            &pending,
            FrameRequestKind::ScrollOnly
        ));
    }

    #[test]
    fn direct_frame_wakes_coalesce_and_preserve_the_strongest_reason() {
        let pending = AtomicU8::new(0);
        let redraws = AtomicUsize::new(0);

        assert!(request_direct_frame(
            &pending,
            FrameRequestKind::ScrollOnly,
            || {
                redraws.fetch_add(1, Ordering::SeqCst);
            }
        ));
        assert!(!request_direct_frame(
            &pending,
            FrameRequestKind::ScrollOnly,
            || {
                redraws.fetch_add(1, Ordering::SeqCst);
            }
        ));
        assert!(!request_direct_frame(
            &pending,
            FrameRequestKind::Full,
            || {
                redraws.fetch_add(1, Ordering::SeqCst);
            }
        ));

        assert_eq!(redraws.load(Ordering::SeqCst), 1);
        assert_eq!(
            complete_frame_ready_request(&pending),
            FrameRequestKind::Full
        );
        assert!(request_direct_frame(
            &pending,
            FrameRequestKind::ScrollOnly,
            || {
                redraws.fetch_add(1, Ordering::SeqCst);
            }
        ));
        assert_eq!(
            complete_frame_ready_request(&pending),
            FrameRequestKind::ScrollOnly
        );
    }

    #[test]
    fn callback_ready_requests_are_independent_of_animation_frame_requests() {
        let frame_pending = AtomicU8::new(0);
        let callback_pending = AtomicBool::new(false);

        assert!(try_begin_frame_ready_request(
            &frame_pending,
            FrameRequestKind::Full
        ));
        assert!(try_begin_callback_ready_request(&callback_pending));
        assert!(!try_begin_callback_ready_request(&callback_pending));

        complete_frame_ready_request(&frame_pending);
        complete_callback_ready_request(&callback_pending);

        assert!(try_begin_callback_ready_request(&callback_pending));
    }

    #[test]
    fn app_builder_defaults_to_analytic_antialiasing() {
        let app = AimerApp::new().child(RedrawWidget);

        assert_eq!(app.antialiasing(), AntiAlias::Analytic);
    }

    #[test]
    fn app_builder_preserves_the_selected_antialiasing_mode() {
        let app = AimerApp::new()
            .with_antialiasing(AntiAlias::Msaa2x)
            .child(RedrawWidget);

        assert_eq!(app.antialiasing(), AntiAlias::Msaa2x);
    }

    #[test]
    fn app_builder_appends_each_setup_hook() {
        let default_hook_count = 0;
        let app = AimerApp::new()
            .setup(|| "first resource")
            .setup(|| "second resource")
            .child(RedrawWidget);

        assert_eq!(app.startup_hooks.len(), default_hook_count + 2);
    }

    struct RecordingWidget {
        builds: Arc<AtomicUsize>,
        cancels: Arc<AtomicUsize>,
    }

    impl Widget for RecordingWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            self.builds.fetch_add(1, Ordering::SeqCst);
            RecordingElement {
                cancels: self.cancels.clone(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for RecordingWidget {}

    struct RecordingElement {
        cancels: Arc<AtomicUsize>,
    }

    impl Drawable for RecordingElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for RecordingElement {}
    impl Rebuildable for RecordingElement {}
    impl VisitorElement for RecordingElement {
        fn debug_name(&self) -> &'static str {
            "RecordingElement"
        }
    }
    impl EventElement for RecordingElement {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if matches!(event, ElementEvent::Cancel) {
                self.cancels.fetch_add(1, Ordering::SeqCst);
            }
            aimer_widget::EventResult::ignored()
        }
    }

    /// Reads a value at draw time, the way a widget reads the state it was
    /// built from.
    ///
    /// The two cells are `Rc`, not `Arc`: a Venus task is polled on the UI
    /// thread and may hold exactly this kind of handle, which is the property
    /// these tests exist to pin.
    struct ObservingWidget {
        source: Rc<Cell<i32>>,
        observed: Rc<Cell<i32>>,
    }

    impl Widget for ObservingWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            ObservingElement {
                source: self.source,
                observed: self.observed,
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for ObservingWidget {}

    struct ObservingElement {
        source: Rc<Cell<i32>>,
        observed: Rc<Cell<i32>>,
    }

    impl Drawable for ObservingElement {
        fn update(&self, _ctx: &BuildContext) {
            self.observed.set(self.source.get());
        }
    }
    impl LayoutElement for ObservingElement {}
    impl Rebuildable for ObservingElement {}
    impl VisitorElement for ObservingElement {
        fn debug_name(&self) -> &'static str {
            "ObservingElement"
        }
    }
    impl EventElement for ObservingElement {}

    /// An element that only implements `update` has no retained paint: it
    /// stays unresolved (and is asked again every frame) while the host around
    /// it paints locally.
    #[test]
    fn an_element_without_retained_paint_stays_unresolved_and_is_retried() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });
        app.pump_frames(3);

        let census = app
            .paint_source_census()
            .expect("a mounted app has a root to inspect");

        // `RecordingElement` only implements `update`.
        assert!(
            census
                .unresolved_roots
                .iter()
                .any(|element| element.debug_name == "RecordingElement"),
            "{census:?}"
        );
        assert!(census.local_v2 > 0, "the host around it paints locally");
    }

    /// The property the whole runtime exists for: an effect produced by a
    /// resolved future is visible to *this* frame's build, not the next one.
    #[test]
    fn a_microtask_lands_before_the_frame_it_was_spawned_for() {
        let source = Rc::new(Cell::new(0));
        let observed = Rc::new(Cell::new(-1));
        let mut app = AimerApp::start_headless(ObservingWidget {
            source: source.clone(),
            observed: observed.clone(),
        });
        app.render_frame();
        assert_eq!(observed.get(), 0, "the first frame drew the initial state");

        let mutated = source.clone();
        app.venus().spawn(async move {
            aimer_venus::yield_now().await;
            mutated.set(7);
        });

        app.render_frame();

        assert_eq!(observed.get(), 7);
    }

    /// Background work is spent on the slack a frame has left, so it must run
    /// after the tree was drawn and never before it.
    #[test]
    fn idle_work_runs_in_the_slack_after_the_build() {
        let order = Rc::new(RefCell::new(Vec::new()));
        let drawn = order.clone();
        let mut app = AimerApp::start_headless(OrderedWidget { order: drawn });

        let idled = order.clone();
        app.venus().spawn_idle(async move {
            idled.borrow_mut().push("idle");
        });

        app.render_frame();

        assert_eq!(*order.borrow(), vec!["build", "idle"]);
    }

    /// A handler eleven elements deep was never handed a spawner, and must
    /// still be able to spawn: the application installs its runtime for the
    /// thread it draws on.
    #[test]
    fn an_application_installs_its_runtime_for_the_thread_it_draws_on() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let spawned = aimer_venus::spawn_local(async move { flag.set(true) });

        assert!(spawned.is_some(), "no runtime was installed for the thread");
        app.render_frame();
        assert!(ran.get(), "the frame never drained what was spawned");
    }

    /// A task that is not finished has to bring the loop back, or the frame it
    /// is waiting for never arrives.
    #[test]
    fn unfinished_work_asks_for_another_frame() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });
        app.render_frame();
        app.take_redraw_request();

        // Idle work that outlives its budget is the ordinary case: it is sliced
        // across frames, so each frame owes the next one.
        app.venus().spawn_idle(async move {
            loop {
                aimer_venus::yield_now().await;
            }
        });

        app.render_frame();

        assert!(app.take_redraw_request());
    }

    /// An application with nothing to do must let the loop sleep.
    ///
    /// This is the other half of [`unfinished_work_asks_for_another_frame`], and
    /// the more expensive one to get wrong: a runtime that reports work it does
    /// not have turns every frame into a request for the next, so the
    /// application renders flat out forever and every real frame competes with a
    /// pointless one. That reads to a user as a *low* frame rate, which is the
    /// opposite of what the extra frames were spent on.
    #[test]
    fn an_idle_application_does_not_ask_for_another_frame() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });
        app.render_frame();
        app.take_redraw_request();

        for _ in 0..8 {
            app.render_frame();
            assert!(
                !app.take_redraw_request(),
                "an idle frame asked for another one"
            );
        }
    }

    /// A task that is waiting on something that has not happened is not ready
    /// work, and must not keep the loop spinning.
    #[test]
    fn a_pending_task_does_not_keep_the_loop_awake() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });

        // What an `AsyncBuilder` holds while a request is in flight: alive, and
        // with nothing to poll until its answer arrives.
        app.venus()
            .spawn(async { std::future::pending::<()>().await });

        app.render_frame();
        app.take_redraw_request();
        app.render_frame();

        assert_eq!(
            app.venus().task_count(),
            1,
            "the request is still in flight"
        );
        assert!(
            !app.take_redraw_request(),
            "a frame spent waiting asked for another one"
        );
    }

    /// Records the order the frame phases ran in, from inside the build.
    struct OrderedWidget {
        order: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Widget for OrderedWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            OrderedElement { order: self.order }.boxed()
        }
    }

    impl aimer_widget::PortableWidget for OrderedWidget {}

    struct OrderedElement {
        order: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Drawable for OrderedElement {
        fn update(&self, _ctx: &BuildContext) {
            self.order.borrow_mut().push("build");
        }
    }
    impl LayoutElement for OrderedElement {}
    impl Rebuildable for OrderedElement {}
    impl VisitorElement for OrderedElement {
        fn debug_name(&self) -> &'static str {
            "OrderedElement"
        }
    }
    impl EventElement for OrderedElement {}

    /// Records the composition the platform reported, the way a text field
    /// paints it.
    type RecordedPreedits = Arc<Mutex<Vec<(String, Option<(usize, usize)>)>>>;

    struct PreeditWidget {
        preedits: RecordedPreedits,
    }

    impl Widget for PreeditWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            PreeditElement {
                preedits: self.preedits.clone(),
                focus_node: FocusNode::new(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for PreeditWidget {}

    struct PreeditElement {
        preedits: RecordedPreedits,
        focus_node: FocusNode,
    }

    impl Drawable for PreeditElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for PreeditElement {}
    impl Rebuildable for PreeditElement {}
    impl VisitorElement for PreeditElement {
        fn debug_name(&self) -> &'static str {
            "PreeditElement"
        }
    }
    impl EventElement for PreeditElement {
        fn focus_node(&self) -> Option<&FocusNode> {
            Some(&self.focus_node)
        }

        fn autofocus(&self) -> bool {
            true
        }
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if let ElementEvent::ImePreedit { text, cursor } = event {
                self.preedits.lock().unwrap().push((text.clone(), *cursor));
                return aimer_widget::EventResult::consumed();
            }
            aimer_widget::EventResult::ignored()
        }
    }

    #[test]
    fn a_composition_puts_the_caret_at_its_end() {
        assert_eq!(preedit_cursor("nihao"), Some((5, 5)));
        // Byte offsets, not characters: the field slices the string with them.
        assert_eq!(preedit_cursor("\u{4f60}"), Some((3, 3)));
        assert_eq!(preedit_cursor(""), None);
    }

    /// A soft keyboard composing Chinese pinyin reports its provisional text
    /// through the platform event path, which no window event ever carries on
    /// iOS or Android.
    #[test]
    fn a_platform_preedit_reaches_the_widget_tree() {
        let preedits = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(PreeditWidget {
            preedits: preedits.clone(),
        });
        app.render_frame();

        app.send_user_event(AimerNativePlatformEvent::SetPreedit {
            text: "nihao".to_owned(),
            cursor: Some((5, 5)),
        });

        assert_eq!(
            preedits.lock().unwrap().as_slice(),
            [("nihao".to_owned(), Some((5, 5)))]
        );
    }

    /// While a composition is open the raw keystrokes behind it must stay
    /// suppressed, exactly as they are for a desktop input method.
    #[test]
    fn a_platform_preedit_opens_and_closes_the_composition() {
        let preedits = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(PreeditWidget {
            preedits: preedits.clone(),
        });
        app.render_frame();

        app.send_user_event(AimerNativePlatformEvent::SetPreedit {
            text: "ni".to_owned(),
            cursor: None,
        });
        assert!(app.is_composing());

        app.send_user_event(AimerNativePlatformEvent::SetPreedit {
            text: String::new(),
            cursor: None,
        });
        assert!(!app.is_composing());
    }

    /// Committing a candidate ends the composition, so a following keystroke is
    /// not swallowed by a composition that is no longer there.
    #[test]
    fn committed_text_ends_the_composition() {
        let mut app = AimerApp::start_headless(PreeditWidget {
            preedits: Arc::new(Mutex::new(Vec::new())),
        });
        app.render_frame();
        app.send_user_event(AimerNativePlatformEvent::SetPreedit {
            text: "ni".to_owned(),
            cursor: None,
        });

        app.send_user_event(AimerNativePlatformEvent::InsertText("你".to_owned()));

        assert!(!app.is_composing());
    }

    #[test]
    fn headless_start_builds_without_a_native_window() {
        let builds = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: builds.clone(),
            cancels: Arc::new(AtomicUsize::new(0)),
        });

        app.render_frame();

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(!app.has_native_window());
        assert_eq!(
            app.logical_size(),
            ResolvedSize {
                width: 1150.0,
                height: 800.0
            }
        );
        let element_id = app.app.active_root().unwrap().id();
        let render_node = app
            .app
            .render_node_for_element(element_id)
            .expect("frame drawer synchronizes the retained render tree");
        assert_eq!(
            app.app.render_tree().element_bounds(render_node).unwrap(),
            aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 1150.0, 800.0)
        );
        assert!(app
            .app
            .render_tree()
            .render_all()
            .iter()
            .any(|operation| matches!(operation, aimer_cupid::draw_cmd_v2::RenderOp::Draw(item) if item.element == render_node)));
    }

    #[test]
    fn headless_start_installs_the_framework_modal_host() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });

        app.render_frame();

        assert_eq!(
            app.app.widget_root.as_ref().map(|root| root.debug_name()),
            Some("ModalHost")
        );
    }

    #[test]
    fn framework_modal_show_and_dismiss_use_the_headless_root_overlay() {
        use aimer_modal::{Modal, ModalController};

        let cancels = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: cancels.clone(),
        });
        let handle = Modal::new()
            .child(RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            })
            .show();

        app.render_frame();

        assert!(ModalController::is_showing());
        assert_eq!(cancels.load(Ordering::SeqCst), 1);

        assert!(handle.dismiss());
        app.render_frame();

        assert!(!ModalController::is_showing());

        Modal::new()
            .child(RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            })
            .show();
        app.render_frame();
        assert!(ModalController::is_showing());

        drop(app);
        assert!(!ModalController::is_showing());
    }

    #[test]
    fn headless_window_events_update_metrics_and_reach_widgets() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless_with(
            RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: cancels.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(640, 480),
                scale_factor: 2.0,
            },
        );
        app.render_frame();

        app.send_window_event(WindowEvent::Focused(true));
        assert!(app.take_redraw_request());
        app.send_window_event(WindowEvent::Resized(PhysicalSize::new(800, 600)));

        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert_eq!(app.physical_size(), PhysicalSize::new(800, 600));
        assert_eq!(
            app.logical_size(),
            ResolvedSize {
                width: 400.0,
                height: 300.0
            }
        );
    }

    #[test]
    fn cursor_boundaries_invalidate_stale_position_without_cancelling_gestures() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: cancels.clone(),
        });
        app.render_frame();
        let device_id = DeviceId::dummy();

        app.send_window_event(WindowEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(20.0, 30.0),
        });
        assert_eq!((app.app.cursor_pos.x, app.app.cursor_pos.y), (20.0, 30.0));

        app.send_window_event(WindowEvent::CursorLeft { device_id });
        assert_eq!(
            (app.app.cursor_pos.x, app.app.cursor_pos.y),
            (
                crate::handler::event_handler::CURSOR_OUTSIDE_POSITION.x,
                crate::handler::event_handler::CURSOR_OUTSIDE_POSITION.y
            ),
        );
        assert_eq!(cancels.load(Ordering::SeqCst), 0);

        app.app.cursor_pos = Vec2d { x: 20.0, y: 30.0 };
        app.send_window_event(WindowEvent::CursorEntered { device_id });
        assert_eq!(
            (app.app.cursor_pos.x, app.app.cursor_pos.y),
            (
                crate::handler::event_handler::CURSOR_OUTSIDE_POSITION.x,
                crate::handler::event_handler::CURSOR_OUTSIDE_POSITION.y
            ),
        );
        assert!(app.take_redraw_request());
    }

    struct CapturingWidget {
        events: Arc<AtomicUsize>,
    }

    impl Widget for CapturingWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            CapturingElement {
                events: self.events.clone(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for CapturingWidget {}

    struct CapturingElement {
        events: Arc<AtomicUsize>,
    }

    impl Drawable for CapturingElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for CapturingElement {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
        }
    }
    impl Rebuildable for CapturingElement {}
    impl VisitorElement for CapturingElement {
        fn debug_name(&self) -> &'static str {
            "CapturingElement"
        }
    }
    impl EventElement for CapturingElement {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            match event {
                ElementEvent::PointerDown(pointer) => {
                    self.events.fetch_add(1, Ordering::SeqCst);
                    aimer_widget::EventResult::consumed().with_pointer_capture(
                        aimer_widget::PointerKey::new(pointer.source, pointer.id),
                    )
                }
                ElementEvent::PointerUp(pointer) => {
                    self.events.fetch_add(1, Ordering::SeqCst);
                    aimer_widget::EventResult::consumed().with_pointer_release(
                        aimer_widget::PointerKey::new(pointer.source, pointer.id),
                    )
                }
                ElementEvent::PointerMove(_) => {
                    self.events.fetch_add(1, Ordering::SeqCst);
                    aimer_widget::EventResult::consumed()
                }
                _ => aimer_widget::EventResult::ignored(),
            }
        }
    }

    #[test]
    fn headless_pointer_capture_persists_across_frames_and_releases_on_up() {
        use winit::event::{ElementState, MouseButton};

        let events = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(CapturingWidget {
            events: events.clone(),
        });
        app.render_frame();
        let device_id = DeviceId::dummy();
        app.send_window_event(WindowEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(20.0, 20.0),
        });
        events.store(0, Ordering::SeqCst);
        app.send_window_event(WindowEvent::MouseInput {
            device_id,
            state: ElementState::Pressed,
            button: MouseButton::Left,
        });
        app.send_window_event(WindowEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(200.0, 200.0),
        });
        app.render_frame();
        app.send_window_event(WindowEvent::MouseInput {
            device_id,
            state: ElementState::Released,
            button: MouseButton::Left,
        });
        assert_eq!(events.load(Ordering::SeqCst), 3);

        app.send_window_event(WindowEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(300.0, 300.0),
        });
        assert_eq!(events.load(Ordering::SeqCst), 3);
    }

    struct ScrollRecordingWidget {
        events: Arc<Mutex<Vec<(Vec2d, ScrollDeltaKind, TouchPhase)>>>,
    }

    impl Widget for ScrollRecordingWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            ScrollRecordingElement {
                events: self.events.clone(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for ScrollRecordingWidget {}

    struct ScrollRecordingElement {
        events: Arc<Mutex<Vec<(Vec2d, ScrollDeltaKind, TouchPhase)>>>,
    }

    impl Drawable for ScrollRecordingElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for ScrollRecordingElement {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
        }
    }
    impl Rebuildable for ScrollRecordingElement {}
    impl VisitorElement for ScrollRecordingElement {
        fn debug_name(&self) -> &'static str {
            "ScrollRecordingElement"
        }
    }
    impl EventElement for ScrollRecordingElement {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if let ElementEvent::Scroll {
                delta, kind, phase, ..
            } = event
            {
                self.events
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((*delta, *kind, *phase));
                return aimer_widget::EventResult::consumed();
            }
            aimer_widget::EventResult::ignored()
        }
    }

    #[test]
    fn headless_wasm_wheel_delta_is_smoothed_without_changing_distance() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(ScrollRecordingWidget {
            events: events.clone(),
        });
        app.render_frame();
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -8.00048828125)),
            phase: TouchPhase::Moved,
        });
        assert!(events.lock().unwrap().is_empty());

        let mut frames = 0;
        while app.app.scroll_smoother.is_active() || frames == 0 {
            app.render_frame();
            frames += 1;
            assert!(frames < 30);
        }

        let events = events.lock().unwrap();
        assert!(events.len() > 1);
        assert!(
            events
                .iter()
                .all(|(_, kind, _)| *kind == ScrollDeltaKind::Line)
        );
        assert!(events.iter().all(|(delta, _, _)| delta.y < 0.0));
    }

    #[test]
    fn wheel_scroll_phases_reach_the_child_widget() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(ScrollRecordingWidget {
            events: events.clone(),
        });
        app.render_frame();
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });

        let mut frames = 0;
        while app.app.scroll_smoother.is_active() || frames == 0 {
            app.render_frame();
            frames += 1;
            assert!(frames < 64);
        }

        let events = events.lock().unwrap();
        let phases: Vec<TouchPhase> = events.iter().map(|(_, _, phase)| *phase).collect();

        assert_eq!(phases.first(), Some(&TouchPhase::Started));
        assert_eq!(phases.last(), Some(&TouchPhase::Ended));
        assert_eq!(
            phases
                .iter()
                .filter(|phase| **phase == TouchPhase::Started)
                .count(),
            1
        );
        assert!(
            phases[1..phases.len() - 1]
                .iter()
                .all(|phase| *phase == TouchPhase::Moved)
        );
    }

    #[test]
    fn close_requested_stops_headless_application() {
        let app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });
        assert!(!app.is_exit_requested());

        let mut app = app;
        app.send_window_event(WindowEvent::CloseRequested);

        assert!(app.is_exit_requested());
    }

    #[test]
    fn invalid_headless_scale_uses_safe_default() {
        let app = AimerApp::start_headless_with(
            RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(320, 240),
                scale_factor: 0.0,
            },
        );

        assert_eq!(app.scale_factor(), 1.0);
        assert_eq!(
            app.logical_size(),
            ResolvedSize {
                width: 320.0,
                height: 240.0
            }
        );
    }

    #[test]
    fn headless_events_no_widget_answered_do_not_ask_for_a_frame() {
        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        });
        app.render_frame();
        assert!(!app.take_redraw_request());

        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(20.0, 30.0),
        });

        assert!(!app.take_redraw_request());
    }

    // Regression for "moving the cursor pins a core": consuming a hover move is
    // how a widget guards its cursor icon, not a promise that pixels changed.
    // Treating the claim as a repaint made every mouse move over any hover
    // widget render the whole window at input rate.
    #[test]
    fn headless_consumed_hover_moves_do_not_ask_for_a_frame() {
        let events = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(CapturingWidget {
            events: events.clone(),
        });
        app.render_frame();
        assert!(!app.take_redraw_request());

        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(20.0, 30.0),
        });

        assert_eq!(
            events.load(Ordering::SeqCst),
            1,
            "the widget heard the move"
        );
        assert!(
            !app.take_redraw_request(),
            "a consumed hover move must not cost a frame"
        );
    }

    struct CursorWidget;

    impl Widget for CursorWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            CursorElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for CursorWidget {}

    struct CursorElement;

    impl Drawable for CursorElement {
        fn update(&self, ctx: &BuildContext) {
            ctx.window.set_pointer_cursor();
        }
    }
    impl LayoutElement for CursorElement {}
    impl Rebuildable for CursorElement {}
    impl VisitorElement for CursorElement {
        fn debug_name(&self) -> &'static str {
            "CursorElement"
        }
    }
    impl EventElement for CursorElement {}

    #[test]
    fn headless_cursor_follows_the_widget_tree_and_resets_when_nothing_answers() {
        let mut app = AimerApp::start_headless(CursorWidget);

        app.render_frame();
        assert_eq!(app.cursor_icon(), winit::window::CursorIcon::Pointer);

        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(20.0, 30.0),
        });

        assert_eq!(app.cursor_icon(), winit::window::CursorIcon::Default);
    }

    #[test]
    fn headless_focus_changes_match_the_windowed_loop() {
        let cancels = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(ScrollRecordingWidget {
            events: Arc::new(Mutex::new(Vec::new())),
        });
        app.render_frame();
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });
        assert!(app.app.scroll_smoother.is_active());

        app.send_window_event(WindowEvent::Focused(false));
        assert!(!app.app.scroll_smoother.is_active());

        let mut app = AimerApp::start_headless(RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: cancels.clone(),
        });
        app.render_frame();

        app.send_window_event(WindowEvent::Focused(true));

        assert_eq!(cancels.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wheel_input_uses_the_frame_synchronized_requester() {
        let mut app = AimerApp::start_headless(ScrollRecordingWidget {
            events: Arc::new(Mutex::new(Vec::new())),
        });
        app.render_frame();
        let _ = app.take_redraw_request();

        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        let previous = aimer_events::window::set_thread_redraw_requester(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });

        WindowEventHandler::handle_mouse_wheel(
            MouseScrollDelta::LineDelta(0.0, -2.0),
            TouchPhase::Moved,
            &mut app.app,
        );

        aimer_events::window::restore_thread_redraw_requester(previous);

        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "wheel input must use the frame-synchronized request path"
        );
        assert!(app.app.scroll_smoother.is_active());
    }

    #[test]
    fn headless_frames_drive_a_scroll_gesture_to_its_end() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(ScrollRecordingWidget {
            events: events.clone(),
        });
        app.render_frame();
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
        let _ = app.take_redraw_request();

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });

        let mut frames = 0;
        while app.take_redraw_request() {
            app.render_frame();
            frames += 1;
            assert!(frames < 64);
        }

        assert!(frames > 1);
        assert!(!app.app.scroll_smoother.is_active());
        let events = events.lock().unwrap();
        assert_eq!(
            events.last().map(|(_, _, phase)| *phase),
            Some(TouchPhase::Ended)
        );
    }

    struct NoopScrollWidget {
        draws: Arc<AtomicUsize>,
        phases: Arc<Mutex<Vec<TouchPhase>>>,
        redraw_on_scroll: bool,
        direct_redraw_on_scroll: bool,
    }

    impl Widget for NoopScrollWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            NoopScrollElement {
                draws: self.draws,
                phases: self.phases,
                redraw_on_scroll: self.redraw_on_scroll,
                direct_redraw_on_scroll: self.direct_redraw_on_scroll,
                window: ctx.window.clone(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for NoopScrollWidget {}

    struct NoopScrollElement {
        draws: Arc<AtomicUsize>,
        phases: Arc<Mutex<Vec<TouchPhase>>>,
        redraw_on_scroll: bool,
        direct_redraw_on_scroll: bool,
        window: WindowHandle,
    }

    impl Drawable for NoopScrollElement {
        fn update(&self, _ctx: &BuildContext) {
            self.draws.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl LayoutElement for NoopScrollElement {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
        }
    }

    impl Rebuildable for NoopScrollElement {}

    impl VisitorElement for NoopScrollElement {
        fn debug_name(&self) -> &'static str {
            "NoopScrollElement"
        }
    }

    impl EventElement for NoopScrollElement {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if let ElementEvent::Scroll { phase, .. } = event {
                self.phases.lock().unwrap().push(*phase);
                if self.direct_redraw_on_scroll {
                    self.window.request_redraw();
                }
                if self.redraw_on_scroll {
                    return aimer_widget::EventResult::consumed().with_redraw();
                }
            }
            if matches!(event, ElementEvent::Scroll { .. }) {
                aimer_widget::EventResult::consumed()
            } else {
                aimer_widget::EventResult::ignored()
            }
        }
    }

    #[test]
    fn no_op_scroll_ticks_deliver_phases_without_redrawing_the_root() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let draws = Arc::new(AtomicUsize::new(0));
        let phases = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(NoopScrollWidget {
            draws: draws.clone(),
            phases: phases.clone(),
            redraw_on_scroll: false,
            direct_redraw_on_scroll: false,
        });
        app.render_frame();
        let initial_draws = draws.load(Ordering::SeqCst);
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
        let _ = app.take_redraw_request();

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });
        let frames = app.pump_frames(64);

        assert!(frames > 1, "the smoothed gesture must keep receiving ticks");
        assert!(!app.app.scroll_smoother.is_active());
        assert_eq!(
            draws.load(Ordering::SeqCst),
            initial_draws,
            "a no-op scroll tick must not draw an unchanged root"
        );
        let phases = phases.lock().unwrap();
        assert!(phases.contains(&TouchPhase::Started));
        assert_eq!(phases.last(), Some(&TouchPhase::Ended));
    }

    #[test]
    fn scroll_redraw_result_keeps_the_scroll_tick_drawable() {
        let draws = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(NoopScrollWidget {
            draws: draws.clone(),
            phases: Arc::new(Mutex::new(Vec::new())),
            redraw_on_scroll: true,
            direct_redraw_on_scroll: false,
        });
        app.render_frame();
        let initial_draws = draws.load(Ordering::SeqCst);
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
        let _ = app.take_redraw_request();

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });

        assert_eq!(app.pump_frames(1), 1);
        assert_eq!(draws.load(Ordering::SeqCst), initial_draws + 1);
    }

    #[test]
    fn coalesced_direct_redraw_promotes_a_scroll_tick() {
        let draws = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(NoopScrollWidget {
            draws: draws.clone(),
            phases: Arc::new(Mutex::new(Vec::new())),
            redraw_on_scroll: false,
            direct_redraw_on_scroll: false,
        });
        app.render_frame();
        let initial_draws = draws.load(Ordering::SeqCst);
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
        let _ = app.take_redraw_request();

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });
        app.window.request_redraw();

        assert_eq!(app.pump_frames(1), 1);
        assert_eq!(draws.load(Ordering::SeqCst), initial_draws + 1);
    }

    #[test]
    fn direct_redraw_during_a_scroll_tick_keeps_it_drawable() {
        let draws = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(NoopScrollWidget {
            draws: draws.clone(),
            phases: Arc::new(Mutex::new(Vec::new())),
            redraw_on_scroll: false,
            direct_redraw_on_scroll: true,
        });
        app.render_frame();
        let initial_draws = draws.load(Ordering::SeqCst);
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
        let _ = app.take_redraw_request();

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });

        assert_eq!(app.pump_frames(1), 1);
        assert_eq!(draws.load(Ordering::SeqCst), initial_draws + 1);
    }

    #[test]
    fn headless_pump_renders_until_the_tree_stops_asking_for_frames() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut app = AimerApp::start_headless(ScrollRecordingWidget {
            events: events.clone(),
        });
        app.render_frame();
        app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };

        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -2.0),
            phase: TouchPhase::Moved,
        });
        let frames = app.pump_frames(64);

        assert!(frames > 1);
        assert!(!app.take_redraw_request());
        assert!(!app.app.scroll_smoother.is_active());
    }

    #[test]
    fn headless_pump_stops_at_the_frame_budget() {
        let mut app = AimerApp::start_headless(RedrawWidget);

        let frames = app.pump_frames(4);

        assert_eq!(frames, 4);
        assert!(app.take_redraw_request());
    }

    struct AnimatingWidget {
        frames: Arc<AtomicUsize>,
    }

    impl Widget for AnimatingWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            AnimatingElement {
                frames: self.frames.clone(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for AnimatingWidget {}

    struct AnimatingElement {
        frames: Arc<AtomicUsize>,
    }

    impl Drawable for AnimatingElement {
        fn update(&self, _ctx: &BuildContext) {
            // The way an animation, a state update, or an opening overlay asks
            // for the frame that continues it.
            if self.frames.fetch_add(1, Ordering::SeqCst) < 2 {
                aimer_events::window::request_animation_frame();
            }
        }
    }
    impl LayoutElement for AnimatingElement {}
    impl Rebuildable for AnimatingElement {}
    impl VisitorElement for AnimatingElement {
        fn debug_name(&self) -> &'static str {
            "AnimatingElement"
        }
    }
    impl EventElement for AnimatingElement {}

    #[test]
    fn headless_receives_the_frame_requests_widgets_make_through_the_platform() {
        let frames = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::start_headless(AnimatingWidget {
            frames: frames.clone(),
        });

        let pumped = app.pump_frames(16);

        assert_eq!(pumped, 3);
        assert_eq!(frames.load(Ordering::SeqCst), 3);
        assert!(!app.take_redraw_request());
    }

    #[test]
    fn a_dropped_headless_application_stops_taking_frame_requests() {
        let frames = Arc::new(AtomicUsize::new(0));
        drop(AimerApp::start_headless(AnimatingWidget {
            frames: frames.clone(),
        }));

        let mut app = AimerApp::start_headless(AnimatingWidget {
            frames: frames.clone(),
        });
        frames.store(0, Ordering::SeqCst);

        assert_eq!(app.pump_frames(16), 3);
    }

    #[test]
    fn headless_startup_hooks_run_once_before_the_first_frame() {
        let runs = Arc::new(AtomicUsize::new(0));
        let hook_runs = runs.clone();
        let mut app = AimerApp::new()
            .setup(move || {
                hook_runs.fetch_add(1, Ordering::SeqCst);
            })
            .child(RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            })
            .run_headless();

        assert_eq!(runs.load(Ordering::SeqCst), 1);

        app.render_frame();
        app.render_frame();

        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn headless_frame_builds_use_the_configured_ui_memory_pool() {
        let observed_commit = Arc::new(AtomicUsize::new(0));
        let mut app = AimerApp::new()
            .ui_memory_limit(2 * 1024 * 1024)
            .child(UiMemoryProbeWidget {
                observed_commit: observed_commit.clone(),
            })
            .run_headless();

        assert_eq!(app.ui_memory().limit_bytes(), 2 * 1024 * 1024);
        app.render_frame();

        assert_eq!(observed_commit.load(Ordering::SeqCst), 2 * 1024 * 1024);
        assert_eq!(app.ui_memory().committed_bytes(), 2 * 1024 * 1024);
    }

    #[test]
    fn app_window_configuration_is_retained_when_child_is_attached() {
        let app = AimerApp::new()
            .window(
                WindowAttr::new()
                    .title("Configured Window")
                    .inner_size(900, 600),
            )
            .child(RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            });

        assert_eq!(
            app.window_attr,
            WindowAttr::new()
                .title("Configured Window")
                .inner_size(900, 600)
        );
    }

    struct RedrawWidget;

    impl Widget for RedrawWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            RedrawElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for RedrawWidget {}

    struct RedrawElement;

    impl Drawable for RedrawElement {
        fn update(&self, ctx: &BuildContext) {
            ctx.window.request_redraw();
        }
    }
    impl LayoutElement for RedrawElement {}
    impl Rebuildable for RedrawElement {}
    impl VisitorElement for RedrawElement {
        fn debug_name(&self) -> &'static str {
            "RedrawElement"
        }
    }
    impl EventElement for RedrawElement {}

    struct UiMemoryProbeWidget {
        observed_commit: Arc<AtomicUsize>,
    }

    impl Widget for UiMemoryProbeWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let allocator = ctx
                .ui_allocator()
                .expect("headless widget builds run in the app UI allocator scope");
            let element = UiMemoryProbeElement([0; 4]).boxed();
            self.observed_commit
                .store(allocator.committed_bytes(), Ordering::SeqCst);
            element
        }
    }

    impl aimer_widget::PortableWidget for UiMemoryProbeWidget {}

    struct UiMemoryProbeElement([u64; 4]);

    impl Drawable for UiMemoryProbeElement {
        fn update(&self, _ctx: &BuildContext) {
            let _ = self.0;
        }
    }
    impl LayoutElement for UiMemoryProbeElement {}
    impl Rebuildable for UiMemoryProbeElement {}
    impl VisitorElement for UiMemoryProbeElement {
        fn debug_name(&self) -> &'static str {
            "UiMemoryProbeElement"
        }
    }
    impl EventElement for UiMemoryProbeElement {}

    #[test]
    fn headless_redraw_requests_can_drive_a_frame_pump() {
        let mut app = AimerApp::start_headless(RedrawWidget);

        app.render_frame();

        assert!(app.take_redraw_request());
        assert!(!app.take_redraw_request());
    }
}
