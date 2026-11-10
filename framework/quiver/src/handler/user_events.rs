use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_widget::Widget;

use crate::aimer_app::AimerNativePlatformEvent;
use crate::handler::AimerApplicationHandler;
use crate::handler::event_handler::WindowEventHandler;

/// Delivers a platform event that did not come from the window itself.
///
/// The soft keyboard on iOS and Android, and the display link that drives
/// frames on iOS, all report through here rather than as window events. A
/// headless application drives the very same path, which is how a test can
/// inject text the way a platform keyboard does. `on_frame_ready` delivers the
/// frame through the platform's drawing entry point: iOS renders on the display
/// tick itself instead of deferring it through another UIKit draw callback.
pub(crate) fn handle_user_event<W: Widget + 'static>(
    app: &mut AimerApplicationHandler<W>,
    event: AimerNativePlatformEvent,
    on_frame_ready: impl FnOnce(&mut AimerApplicationHandler<W>, crate::aimer_app::FrameRequestKind),
) {
    match event {
        // Answered by the windowed loop, which can also leave the app.
        AimerNativePlatformEvent::Back(_) => {}
        AimerNativePlatformEvent::ScreenChanged => {
            // A fold or hinge moved: widgets that follow the display features
            // were marked, and need a frame to show it.
            if aimer_screen::apply_pending() > 0 {
                app.request_full_redraw();
            }
        }
        AimerNativePlatformEvent::Accessibility => app.sync_accessibility(),
        // Someone wants a picture of the window. An app that is not changing draws
        // nothing, so a frame is drawn in full for the renderer to copy.
        AimerNativePlatformEvent::CaptureRequested => app.request_full_redraw(),
        // Someone attached or left: the badge changes, and nothing else does.
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        AimerNativePlatformEvent::AttachmentChanged => {
            if app.attach_overlay.refresh_names() {
                app.request_overlay_frame();
            }
        }
        // An injected pointer is about to act: show where it is.
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        AimerNativePlatformEvent::SyntheticPointer(pointer) => {
            app.attach_overlay.note_pointer(pointer, aimer_utils::AnimInstant::now());
            app.request_overlay_frame();
        }
        // A release build has no overlay to tell.
        #[cfg(not(all(debug_assertions, not(target_arch = "wasm32"))))]
        AimerNativePlatformEvent::AttachmentChanged | AimerNativePlatformEvent::SyntheticPointer(_) => {}
        AimerNativePlatformEvent::OpenLinks => {
            crate::link_inbox::deliver_posted_links();
            // Opening a link changes navigator state, which asks for its own frame.
        }
        AimerNativePlatformEvent::NativeInput { event, modifiers } => {
            WindowEventHandler::handle_native_input(app, event, modifiers);
        }
        AimerNativePlatformEvent::ForceBackspace => {
            if app.active_root().is_some() {
                let ev = ElementEvent::KeyInput {
                    key: NamedKey::Backspace,
                    action: KeyAction::Pressed,
                    modifiers: Default::default(),
                };
                let result = app.dispatch_element_event(app.cursor_pos, &ev);
                let handled = result.is_consumed();
                if handled || result.needs_redraw() {
                    app.request_full_redraw();
                }
            }
        }
        AimerNativePlatformEvent::InsertText(text) => {
            // Injected text is inserted as one edit, exactly like a committed
            // IME phrase, instead of one event per `char`.
            //
            // It *is* the commit on mobile, so it closes any composition the
            // keyboard had open; otherwise the next keystroke would still be
            // suppressed as belonging to a composition that is already gone.
            app.ime_composing = false;
            WindowEventHandler::dispatch_text(
                &text,
                &KeyAction::Pressed,
                &Modifiers::default(),
                app,
            );
        }
        AimerNativePlatformEvent::SetPreedit { text, cursor } => {
            // The mobile equivalent of `Ime::Preedit`: while the composition is
            // open the raw keystrokes behind it stay suppressed, and an empty
            // string closes it.
            app.ime_composing = !text.is_empty();
            WindowEventHandler::dispatch_ime_preedit(text, cursor, app);
        }
        AimerNativePlatformEvent::TextEditingDelta(delta) => {
            if app.active_root().is_some() {
                let result = app
                    .dispatch_element_event(app.cursor_pos, &ElementEvent::TextEditingDelta(delta));
                if result.is_consumed() || result.needs_redraw() {
                    app.request_full_redraw();
                }
            }
        }
        AimerNativePlatformEvent::MenuShortcut(shortcut) => {
            // The menu bar already consumed the key press, so this *is* the
            // shortcut: it travels the same positional path a `Ctrl` shortcut
            // takes, which is what makes one widget-side implementation enough.
            if app.active_root().is_some() {
                let ev = shortcut.to_event();
                let result = app.dispatch_element_event(app.cursor_pos, &ev);
                let handled = result.is_consumed();
                if handled || result.needs_redraw() {
                    app.request_full_redraw();
                }
            }
        }
        AimerNativePlatformEvent::MenuItem(item) => {
            // Chosen from the native menu bar, which is not the widget tree, so
            // the action runs here on the event-loop thread that owns the tree.
            if aimer_shortcut::menu::activate_item(item) {
                app.request_full_redraw();
            }
        }
        AimerNativePlatformEvent::GlobalShortcut(id) => {
            // Heard on a backend thread, run here on the loop that owns the tree.
            if aimer_shortcut::global::dispatch(id) {
                app.request_full_redraw();
            }
        }
        AimerNativePlatformEvent::Tray { id, event } => {
            // Heard on whichever thread the tray reports from, run here on the
            // loop that owns the tree.
            if aimer_platform::tray::dispatch(id, event) {
                app.request_full_redraw();
            }
        }
        AimerNativePlatformEvent::FrameReady => {
            aimer_utils::frame_trace("frame_ready", 0.0, 0.0);
            let kind = crate::aimer_app::frame_ready_delivered();
            app.note_frame_request(kind);
            on_frame_ready(app, kind);
        }
        // The windowed handler exits its loop before reaching this shared path;
        // a headless application has no loop to exit.
        AimerNativePlatformEvent::ShutdownRequested => {}
        // Windowed native handlers deliver completions before this shared path.
        AimerNativePlatformEvent::RenderWorkerReady => {}
        // Answered by the multi-window host, which owns the windows they name.
        AimerNativePlatformEvent::WindowFrameReady(_) | AimerNativePlatformEvent::WindowRequests => {}
    }
}

pub(crate) fn request_window_redraw<W: Widget + 'static>(
    app: &mut AimerApplicationHandler<W>,
    kind: crate::aimer_app::FrameRequestKind,
) {
    if let Some(window) = &app.window {
        crate::aimer_app::with_frame_request_kind(kind, || window.request_redraw());
    }
}
