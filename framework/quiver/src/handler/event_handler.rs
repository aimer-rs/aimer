use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::{FILE_DRAG_POINTER_ID, PointerButton, PointerInfo, PointerSource};
use aimer_widget::{CaptureRequest, EventResult, PointerKey, Widget};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
};
use winit::event_loop::ActiveEventLoop;
use winit::window::{CursorIcon, Theme, WindowId};

use crate::handler::system_back::{self, BackKey};
use crate::handler::{AimerApplicationHandler, platform_gesture};

pub struct WindowEventHandler;

/// Whether a press begins a drag of the window.
///
/// Only the primary button drags, only from inside the strip at the top of the
/// window — `band` points tall, `None` when the framework does not own window
/// dragging — and only when nothing `claimed` the press. A pointer that is
/// outside the window, which is reported far above and left of it, is in no
/// strip.
#[inline]
fn press_starts_window_drag(
    band: Option<f32>,
    pos: Vec2d,
    button: PointerButton,
    claimed: bool,
) -> bool {
    button == PointerButton::Primary
        && !claimed
        && band.is_some_and(|band| pos.x >= 0.0 && pos.y >= 0.0 && pos.y < band)
}

pub(crate) const CURSOR_OUTSIDE_POSITION: Vec2d = Vec2d {
    x: f32::MIN,
    y: f32::MIN,
};

/// What a headless application has to do after an event was handled.
///
/// The windowed loop answers these three questions to `winit` — exit, redraw,
/// present a resized surface — and a headless application has no loop to
/// answer them to, so they are handed back to its owner instead.
pub(crate) enum HeadlessEventAction {
    None,
    Render,
    Exit,
}

impl WindowEventHandler {
    #[inline]
    fn should_redraw(result: EventResult, legacy_handled: bool) -> bool {
        legacy_handled || result.needs_redraw()
    }

    /// Tells the application's services that the user's attention has left it.
    ///
    /// On a mobile OS a window that loses focus is about to be backgrounded and
    /// may then be killed without notice, so it is as urgent as backgrounding
    /// itself and pending saves are finished before this returns. Elsewhere it is
    /// merely a good moment to start them.
    fn notify_attention_lost() {
        let event = if cfg!(any(target_os = "android", target_os = "ios")) {
            aimer_venus::Lifecycle::Background
        } else {
            aimer_venus::Lifecycle::Inactive
        };
        aimer_venus::notify_lifecycle(event);
    }

    pub(crate) fn handle_events<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        // debug!("======> Event: {event:?}");
        #[cfg(target_os = "ios")]
        if Self::defer_uikit_frame_event(app, &event) {
            return;
        }
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            match system_back::classify(&key.logical_key, key.state, key.repeat) {
                BackKey::NotBack => {}
                BackKey::Swallow => return,
                BackKey::Request => {
                    // Nothing answering is the platform's to decide: winit tells
                    // Android every key was handled, so without this Back would
                    // never leave the app.
                    if !aimer_events::back::request_back() && system_back::UNHANDLED_BACK_EXITS {
                        Self::close_window(app, event_loop);
                    }
                    return;
                }
            }
        }
        match event {
            WindowEvent::CloseRequested => Self::close_window(app, event_loop),

            WindowEvent::RedrawRequested => app.render(event_loop),

            #[cfg(not(target_os = "ios"))]
            WindowEvent::Resized(size) => Self::handle_resize(size, app, event_loop),

            other => Self::handle_common_event(app, other),
        }
    }

    /// Closes the window: the render worker is stopped first, and the loop exits
    /// once it has.
    pub(crate) fn close_window<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>, event_loop: &ActiveEventLoop) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            app.render_ctx.close();
            if app.render_ctx.is_stopped() {
                app.window_finished(event_loop);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = app;
            event_loop.exit();
        }
    }

    /// Handles every event whose answer does not depend on who is driving the
    /// application.
    ///
    /// A frame is asked for through [`AimerApplicationHandler::window`], which
    /// is a real window under the platform loop and a recording one in a
    /// headless application — so both see the same events, dispatched the same
    /// way, requesting frames on exactly the same conditions. The three events
    /// that *do* depend on the driver — closing, redrawing, and resizing, each
    /// of which the windowed loop answers by calling back into `winit` — are
    /// handled by the callers instead.
    fn handle_common_event<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Touch(item) => Self::handle_touch(item, app),

            WindowEvent::CursorMoved { position, .. } => {
                // A cursor moving with files in tow is not a hover and not a
                // click: it is the file drag continuing, and the widgets under
                // it want to hear about it as one.
                if app.file_drag.is_active() {
                    Self::report_file_drag_move(app, Self::logical_cursor(position, app));
                    return;
                }
                Self::handle_cursor_move(position, app)
            }

            WindowEvent::CursorLeft { .. } => Self::handle_cursor_left(app),

            WindowEvent::CursorEntered { .. } => Self::handle_cursor_entered(app),

            WindowEvent::MouseInput { state, button, .. } => {
                Self::handle_mouse_input(state, button, app)
            }

            WindowEvent::ModifiersChanged(mods) => {
                let state = mods.state();
                app.current_modifiers = Modifiers {
                    ctrl: state.control_key(),
                    shift: state.shift_key(),
                    alt: state.alt_key(),
                    meta: state.super_key(),
                };
            }

            WindowEvent::KeyboardInput { event, .. } => Self::handle_keyboard_input(event, app),

            WindowEvent::Ime(ime) => Self::handle_ime(ime, app),

            // The reported phase is unused on the web, where it is replaced.
            #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
            WindowEvent::MouseWheel { delta, phase, .. } => {
                #[cfg(target_arch = "wasm32")]
                let phase = Self::web_wheel_phase(app);
                Self::handle_mouse_wheel(delta, phase, app);
            }

            // Gestures the windowing system recognized itself. winit reports no
            // position with them, so they land where the cursor is — which on a
            // touch device is where the last contact was.
            WindowEvent::PinchGesture { delta, phase, .. } => {
                let pos = app.cursor_pos;
                Self::dispatch_platform_gesture(app, platform_gesture::pinch(pos, delta, phase));
            }
            WindowEvent::RotationGesture { delta, phase, .. } => {
                let pos = app.cursor_pos;
                Self::dispatch_platform_gesture(app, platform_gesture::rotation(pos, delta, phase));
            }
            WindowEvent::PanGesture { delta, phase, .. } => {
                let pos = app.cursor_pos;
                let event = platform_gesture::pan(pos, delta, app.window_scale, phase);
                Self::dispatch_platform_gesture(app, event);
            }
            WindowEvent::DoubleTapGesture { .. } => {
                let pos = app.cursor_pos;
                Self::dispatch_platform_gesture(app, platform_gesture::smart_zoom(pos));
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                Self::update_scale_factor(&mut app.window_scale, scale_factor);
                #[cfg(all(
                    any(target_os = "macos", target_os = "ios"),
                    feature = "native",
                    not(feature = "wgpu")
                ))]
                if app.render_ctx.is_ready()
                    && let Some(window) = app.native_window()
                {
                    // Keep CAMetalLayer's contents scale current even when the
                    // physical drawable size did not change with the display.
                    app.render_ctx
                        .resize(aimer_widget::base::WindowHandle::native(window.clone()).inner_size());
                }
                app.sync_headless_metrics(None);
                if let Some(root) = app.active_root() {
                    root.invalidate_layout();
                    app.notify_window_metrics_changed();
                }
                Self::refresh_system_appearance(app);
                Self::refresh_safe_area(app);
                app.request_full_redraw();
            }

            WindowEvent::ThemeChanged(theme) => {
                if Self::handle_theme_changed(theme) {
                    app.request_full_redraw();
                }
            }

            WindowEvent::Focused(is_focus) => {
                if !is_focus {
                    Self::notify_attention_lost();
                }
                if app.active_root().is_none() {
                    return;
                }
                if is_focus {
                    let result = app.cancel_element_events();
                    if Self::should_redraw(result, true) {
                        app.request_full_redraw();
                    }
                } else {
                    #[cfg(target_arch = "wasm32")]
                    app.web_scroll_phase.reset();
                    app.scroll_smoother.clear();
                }
            }

            // A hidden browser tab, or a window the OS has covered or minimized.
            WindowEvent::Occluded(true) => {
                // The GPU may not be used once the app is backgrounded, but the
                // surface must stay attached: the system snapshots it for the
                // app switcher right after this returns.
                #[cfg(all(target_os = "ios", not(target_arch = "wasm32")))]
                app.render_ctx.set_paused(true);
                Self::notify_attention_lost();
            }

            #[cfg(all(target_os = "ios", not(target_arch = "wasm32")))]
            WindowEvent::Occluded(false) => {
                app.render_ctx.set_paused(false);
                app.request_full_redraw();
            }

            WindowEvent::HoveredFile(path) => {
                let pos = Self::refresh_file_drag_cursor(app);
                app.file_drag.enter(&path, app.cursor_pos);
                Self::handle_generic_event(app, &ElementEvent::HoveredFile { path, pos });
            }

            WindowEvent::HoveredFileCancelled => {
                app.file_drag.finish();
                Self::handle_generic_event(app, &ElementEvent::HoveredFileCancelled);
            }

            WindowEvent::DroppedFile(path) => {
                let pos = Self::refresh_file_drag_cursor(app);
                // The drag is over, but the platform reports a five-file drop as
                // five events: the tracker is emptied by the first of them and
                // the rest simply find nothing in flight.
                app.file_drag.finish();
                Self::handle_generic_event(app, &ElementEvent::DroppedFile { path, pos });
            }

            _ => (),
        }
    }

    /// Delivers a window event to an application that has no platform loop
    /// behind it.
    ///
    /// Every event a widget can observe travels through
    /// [`handle_common_event`](Self::handle_common_event), the very code the
    /// windowed loop runs, so a headless application sees the same dispatch,
    /// the same cursor changes, and the same frame requests. Only the answers
    /// the windowed loop gives back to `winit` are returned to the caller
    /// instead of being acted on here.
    pub(crate) fn handle_headless_event<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event: WindowEvent,
    ) -> HeadlessEventAction {
        match event {
            WindowEvent::CloseRequested => HeadlessEventAction::Exit,
            WindowEvent::RedrawRequested => HeadlessEventAction::Render,
            WindowEvent::Resized(size) => {
                Self::apply_resize(size, app);
                HeadlessEventAction::Render
            }
            other => {
                Self::handle_common_event(app, other);
                HeadlessEventAction::None
            }
        }
    }

    fn update_scale_factor(window_scale: &mut f64, scale_factor: f64) {
        *window_scale = scale_factor;
    }

    /// Reports the appearance the system just switched to, and answers whether a
    /// frame has to be drawn for it.
    ///
    /// The switch reaches the widgets that follow the appearance and no others,
    /// so an application that pins its theme costs nothing here: no widget is
    /// marked, and no repaint is asked for.
    fn handle_theme_changed(theme: Theme) -> bool {
        aimer_widget::set_platform_brightness(theme.into()) > 0
    }

    /// Re-reads the appearance from the platform after it reported that its
    /// configuration moved.
    ///
    /// Android is the platform that needs this: it announces a light/dark switch
    /// as a configuration change — which winit forwards as
    /// [`WindowEvent::ScaleFactorChanged`] — and nothing else tells the
    /// application about it. A frame is already being asked for by the
    /// configuration change itself, so the answer is not needed here.
    ///
    /// Everywhere else the appearance arrives as
    /// [`WindowEvent::ThemeChanged`] and this reads nothing at all.
    #[cfg_attr(not(target_os = "android"), allow(unused_variables))]
    fn refresh_system_appearance<W: Widget + 'static>(app: &AimerApplicationHandler<W>) {
        #[cfg(target_os = "android")]
        if let Some(window) = app.native_window() {
            crate::system_appearance::announce(window);
        }
    }

    /// Re-reads the region the platform reserves in the window after its
    /// geometry moved.
    ///
    /// A rotation moves the notch and the home indicator to other edges, and
    /// reaches the application as a resize and a scale-factor change rather than
    /// as anything that mentions the safe area. This is the whole of the cost:
    /// one query per event that could have changed the answer, and none per
    /// frame. A reservation that did not actually move asks for no frame — see
    /// [`aimer_widget::set_safe_area_insets`].
    ///
    /// On the platforms that reserve nothing this compiles to an empty body:
    /// there is no query, and not even a look at the window.
    #[cfg_attr(
        not(any(target_os = "ios", target_arch = "wasm32")),
        allow(unused_variables)
    )]
    fn refresh_safe_area<W: Widget + 'static>(app: &AimerApplicationHandler<W>) {
        #[cfg(any(target_os = "ios", target_arch = "wasm32"))]
        if let Some(window) = app.native_window() {
            crate::system_safe_area::announce(window);
        }
    }

    fn handle_touch<W: Widget + 'static>(item: Touch, app: &mut AimerApplicationHandler<W>) {
        let scale = app.window_scale;
        let pos = Vec2d {
            x: (item.location.x / scale) as f32,
            y: (item.location.y / scale) as f32,
        };
        // Wheel/trackpad streams can arrive while a finger is still down. Keep
        // the shared event position at the latest contact so those scroll
        // frames hit the viewport instead of the outside-window sentinel.
        app.cursor_pos = pos;
        app.touch_pointer = true;
        // info!("Location: {pos:?}" );
        let touch_id = item.id;
        // All touch events are passed through with their finger ID.
        // Individual widgets (scrollable, gesture detector) decide which
        // fingers to track — the scrollable keeps its own primary-finger
        // filter so a second touch doesn't jump the scroll position.
        let contact = PointerInfo::touch(pos, touch_id);
        let event = match item.phase {
            TouchPhase::Started => ElementEvent::PointerDown(contact),
            TouchPhase::Moved => ElementEvent::PointerMove(contact),
            TouchPhase::Ended => ElementEvent::PointerUp(contact),
            TouchPhase::Cancelled => ElementEvent::Cancel,
        };
        #[allow(clippy::collapsible_if)]
        {
            if app.active_root().is_some() {
                let mut result = app.dispatch_element_event(pos, &event);
                if !result.is_consumed() {
                    // Broadcast PointerUp/Cancel alongside PointerDown so that
                    // elements with an active drag (e.g. scrollable fling) receive
                    // the release event even when the finger lifts outside their
                    // bounds — the common case for a fast flick on touch screens.
                    if matches!(&event, ElementEvent::PointerDown(_)) {
                        result = result.merge(app.broadcast_element_event(&event));
                    }
                }
                if Self::should_redraw(result, true) {
                    app.request_animation_frame();
                }
            }
        }
    }

    pub(super) fn handle_cursor_move<W: Widget + 'static>(
        position: PhysicalPosition<f64>,
        app: &mut AimerApplicationHandler<W>,
    ) {
        let new_pos = Self::logical_cursor(position, app);
        let dx = (new_pos.x - app.cursor_pos.x).abs();
        let dy = (new_pos.y - app.cursor_pos.y).abs();
        if dx < 1.0 && dy < 1.0 && !app.touch_pointer {
            return;
        }
        app.cursor_pos = new_pos;
        app.touch_pointer = false;
        if Self::move_window_with_pointer(app, new_pos) {
            return;
        }
        // Aiming at another target ends the scroll intent. A browser never
        // reports the lift, so a real pointer move is the only evidence that
        // the next wheel event belongs to a new gesture.
        #[cfg(target_arch = "wasm32")]
        Self::end_web_scroll_gesture(app);
        if app.active_root().is_some() {
            // The button reported with a move is the one being held, so a drag
            // started with the secondary button stays a secondary-button drag all
            // the way to its release.
            let event = ElementEvent::PointerMove(PointerInfo::mouse(
                app.cursor_pos,
                app.pressed_button.unwrap_or_default(),
            ));
            let result = app.dispatch_element_event(app.cursor_pos, &event);
            aimer_utils::pointer_trace!(
                "move ({:.1}, {:.1}) consumed={} redraw={} captured_pointers={} held_button={:?}",
                app.cursor_pos.x,
                app.cursor_pos.y,
                result.is_consumed(),
                result.needs_redraw(),
                app.event_dispatcher.capture_count(),
                app.pressed_button,
            );
            if let Some(window) = &app.window {
                if !result.is_consumed() {
                    window.set_cursor(CursorIcon::Default);
                }
                // A consumed move is a claim, not a repaint: a widget guarding
                // its cursor icon or tracking a hover consumes every move it
                // covers, and rendering a frame for each one is what used to
                // pin a core while the cursor merely crossed the window. Only
                // an explicit redraw request — a drag, a crossed hover edge —
                // buys the frame.
                if Self::should_redraw(result, false) {
                    app.request_animation_frame();
                }
            }
        }
    }

    pub(super) fn handle_cursor_left<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>) {
        app.cursor_pos = CURSOR_OUTSIDE_POSITION;
        if app.active_root().is_some() {
            let pointer = PointerKey::new(PointerSource::Mouse, 0);
            let was_captured = app.event_dispatcher.is_captured(pointer);
            let event = ElementEvent::PointerExited(pointer.source, pointer.id);
            let mut result = app.dispatch_element_event(app.cursor_pos, &event);
            if !was_captured {
                result = result.merge(app.broadcast_element_event(&event));
            }
            if Self::should_redraw(result, true) {
                app.request_full_redraw();
            }
        }
    }

    fn handle_cursor_entered<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>) {
        // CursorEntered carries no coordinates. Keep hover disabled until the
        // following CursorMoved supplies a valid position.
        app.cursor_pos = CURSOR_OUTSIDE_POSITION;
        app.request_full_redraw();
    }

    /// Translates a winit mouse button into the framework's own.
    ///
    /// Named by role rather than by side, so a left-handed OS mouse setting needs
    /// no special case anywhere above this line. Every button is translated: the
    /// middle button used to be dropped here, which made middle-click impossible
    /// to observe at all, and the right button was translated to nothing at all —
    /// so a secondary press arrived as an ordinary one and fired tap handlers.
    fn pointer_button(button: MouseButton) -> PointerButton {
        match button {
            MouseButton::Left => PointerButton::Primary,
            MouseButton::Right => PointerButton::Secondary,
            MouseButton::Middle => PointerButton::Middle,
            MouseButton::Back => PointerButton::Other(3),
            MouseButton::Forward => PointerButton::Other(4),
            MouseButton::Other(code) => PointerButton::Other(code),
        }
    }

    pub(super) fn handle_mouse_input<W: Widget + 'static>(
        state: ElementState,
        button: MouseButton,
        app: &mut AimerApplicationHandler<W>,
    ) {
        let c = app.cursor_pos;
        let pressed = state.is_pressed();
        let button = Self::pointer_button(button);
        let pointer = PointerInfo::mouse(c, button);

        // Remembered so a move or release during a drag reports the button that
        // started it, which a hover-time move has no way of knowing.
        app.pressed_button = pressed.then_some(button);
        if !pressed && button == PointerButton::Primary {
            app.macos_windowing.end_window_drag();
        }

        let event = if pressed {
            ElementEvent::PointerDown(pointer)
        } else {
            ElementEvent::PointerUp(pointer)
        };

        #[allow(clippy::collapsible_if)]
        if app.active_root().is_some() {
            let mut result = app.dispatch_element_event(c, &event);
            if !result.is_consumed() {
                if matches!(&event, ElementEvent::PointerDown(_)) {
                    result = result.merge(app.broadcast_element_event(&event));
                }
            }
            if pressed {
                Self::start_window_drag_if_unclaimed(app, c, button, result);
            }
            if Self::should_redraw(result, true) {
                app.request_animation_frame();
            }
        }
    }

    /// Starts moving the window with the pointer when no element claimed the
    /// press and it landed in the strip the framework drags the window from.
    ///
    /// The platform used to decide this alone: for a window whose content runs
    /// under the titlebar, AppKit moved the window for any press in the strip
    /// before the application heard of it, so a button or a field there dragged
    /// the window as well as being pressed. Elements now have the press first.
    /// A press one of them consumed, captured the pointer for, or took as a
    /// focus target (a text field consumes nothing) is theirs; only one that was
    /// left over starts the drag, which the framework carries out
    /// itself as the pointer moves; see [`Self::move_window_with_pointer`].
    fn start_window_drag_if_unclaimed<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        pos: Vec2d,
        button: PointerButton,
        result: EventResult,
    ) -> bool {
        let claimed = result.is_consumed()
            || !matches!(result.capture_request(), CaptureRequest::None)
            || app.event_dispatcher.last_press_hit_focus_target()
            || app
                .event_dispatcher
                .is_captured(PointerKey::new(PointerSource::Mouse, 0));
        if !press_starts_window_drag(app.macos_windowing.window_drag_height(), pos, button, claimed)
        {
            return false;
        }
        app.macos_windowing.begin_window_drag((pos.x, pos.y));
        true
    }

    /// Moves the window with the pointer while a drag the framework started is
    /// under way, and reports whether it did, in which case the move is not
    /// delivered to the elements: the window is what moves, and the pointer's
    /// position inside it changes with every step.
    ///
    /// A drag only lives while the primary button is held. One whose release
    /// the application never saw — focus was lost mid-drag — would otherwise
    /// carry the window along with an idle pointer.
    fn move_window_with_pointer<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        pos: Vec2d,
    ) -> bool {
        if !app.macos_windowing.window_drag_active() {
            return false;
        }
        if app.pressed_button != Some(PointerButton::Primary) {
            app.macos_windowing.end_window_drag();
            return false;
        }
        if let Some(window) = app.window.as_ref().and_then(|window| window.native_window()) {
            app.macos_windowing
                .move_window_with_pointer(window, (pos.x, pos.y));
        }
        true
    }

    fn handle_keyboard_input<W: Widget + 'static>(
        event: KeyEvent,
        app: &mut AimerApplicationHandler<W>,
    ) {
        use winit::event::ElementState;
        use winit::keyboard::{Key, NamedKey as WinitNamedKey};

        #[cfg(debug_assertions)]
        if app.handle_debug_key(&event.logical_key, event.state, event.repeat) {
            return;
        }

        let action = if event.repeat {
            KeyAction::Repeat
        } else {
            match event.state {
                ElementState::Pressed => KeyAction::Pressed,
                ElementState::Released => KeyAction::Released,
            }
        };

        let modifiers = app.current_modifiers.clone();

        if modifiers.ctrl || modifiers.meta {
            use winit::keyboard::{KeyCode, PhysicalKey};
            let named = match event.physical_key {
                PhysicalKey::Code(KeyCode::KeyA) => Some(NamedKey::Other("a".into())),
                PhysicalKey::Code(KeyCode::KeyC) => Some(NamedKey::Other("c".into())),
                PhysicalKey::Code(KeyCode::KeyV) => Some(NamedKey::Other("v".into())),
                PhysicalKey::Code(KeyCode::KeyX) => Some(NamedKey::Other("x".into())),
                PhysicalKey::Code(KeyCode::KeyZ) => Some(NamedKey::Other("z".into())),
                PhysicalKey::Code(KeyCode::KeyY) => Some(NamedKey::Other("y".into())),
                _ => None,
            };
            if let Some(key) = named {
                let ev = ElementEvent::KeyInput {
                    key,
                    action,
                    modifiers,
                };
                if app.active_root().is_some() {
                    let result = app.dispatch_element_event(app.cursor_pos, &ev);
                    let handled = result.is_consumed();
                    if Self::should_redraw(result, handled) {
                        app.request_full_redraw();
                    }
                }
                return;
            }
        }

        if app.ime_composing {
            return;
        }

        let text_input: Option<String> = match &event.text {
            Some(t) => Some(t.to_string()),
            #[cfg(target_arch = "wasm32")]
            None => match &event.logical_key {
                Key::Character(ch) => Some(ch.to_string()),
                _ => None,
            },
            #[cfg(not(target_arch = "wasm32"))]
            _ => None,
        };

        if let Some(text) = text_input
            && !text.is_empty()
            && text.chars().all(|c| !c.is_control())
        {
            Self::dispatch_text(&text, &action, &modifiers, app);
            return;
        }

        // On the web backend, space is delivered as a named key without any
        // `event.text`, so handle it explicitly. On native platforms a real
        // space arrives through `event.text` above; the named `Space` here only
        // appears as an IME confirm key, which must NOT insert a space.
        #[cfg(target_arch = "wasm32")]
        if let Key::Named(WinitNamedKey::Space) = event.logical_key {
            Self::dispatch_text(" ", &action, &modifiers, app);
            return;
        }

        // Handle named keys
        if let Key::Named(named) = &event.logical_key {
            let key = match named {
                WinitNamedKey::Backspace => NamedKey::Backspace,
                WinitNamedKey::Delete => NamedKey::Delete,
                WinitNamedKey::ArrowUp => NamedKey::ArrowUp,
                WinitNamedKey::ArrowDown => NamedKey::ArrowDown,
                WinitNamedKey::ArrowLeft => NamedKey::ArrowLeft,
                WinitNamedKey::ArrowRight => NamedKey::ArrowRight,
                WinitNamedKey::PageUp => NamedKey::PageUp,
                WinitNamedKey::PageDown => NamedKey::PageDown,
                WinitNamedKey::Home => NamedKey::Home,
                WinitNamedKey::End => NamedKey::End,
                WinitNamedKey::Enter => NamedKey::Enter,
                WinitNamedKey::Escape => NamedKey::Escape,
                WinitNamedKey::Tab => NamedKey::Tab,
                other => NamedKey::Other(format!("{:?}", other)),
            };
            let ev = ElementEvent::KeyInput {
                key,
                action,
                modifiers: modifiers.clone(),
            };
            if app.active_root().is_some() {
                let result = app.dispatch_element_event(app.cursor_pos, &ev);
                let handled = result.is_consumed();
                if Self::should_redraw(result, handled) {
                    app.request_full_redraw();
                }
            }
        }
    }

    /// Dispatches a text payload to the widget tree. This is the single path
    /// used for plain typed characters, web text input, and committed IME text,
    /// so CJK phrases and emoji are inserted correctly.
    ///
    /// A single character travels as `CharInput`, while a longer payload — an
    /// IME commit such as `"你好世界"` — travels as one `TextInput` event. Batching
    /// keeps a committed phrase a single edit: one tree traversal, one undo
    /// entry, and one change notification instead of one per `char`.
    pub(crate) fn dispatch_text<W: Widget + 'static>(
        text: &str,
        action: &KeyAction,
        modifiers: &Modifiers,
        app: &mut AimerApplicationHandler<W>,
    ) {
        if app.active_root().is_none() {
            return;
        }
        let mut chars = text.chars();
        let event = match (chars.next(), chars.next()) {
            (None, _) => return,
            (Some(ch), None) => ElementEvent::CharInput {
                ch,
                action: action.clone(),
                modifiers: modifiers.clone(),
            },
            (Some(_), Some(_)) => ElementEvent::TextInput {
                text: text.to_owned(),
                action: action.clone(),
                modifiers: modifiers.clone(),
            },
        };
        let result = app.dispatch_element_event(app.cursor_pos, &event);
        let handled = result.is_consumed();
        if Self::should_redraw(result, handled) {
            app.request_full_redraw();
        }
    }

    /// Handles input-method (IME) events so that languages requiring
    /// composition — Chinese, Japanese, Korean, etc. — can be typed.
    ///
    /// While a composition is active (`Ime::Preedit`) raw key strokes are
    /// suppressed in `handle_keyboard_input`; the finished text arrives through
    /// `Ime::Commit` and is inserted via the normal text path.
    fn handle_ime<W: Widget + 'static>(ime: Ime, app: &mut AimerApplicationHandler<W>) {
        // Composition reports one event per keystroke, so this must not format
        // a string in a release build.
        // #[cfg(debug_assertions)]
        // aimer_utils::debug!("IME : {ime:?}");
        match ime {
            Ime::Enabled => {
                app.ime_composing = false;
                // A freshly enabled input method owns no composition yet, so
                // drop whatever the previous one left painted.
                Self::dispatch_ime_preedit(String::new(), None, app);
            }
            Ime::Preedit(text, cursor) => {
                app.ime_composing = !text.is_empty();
                Self::dispatch_ime_preedit(text, cursor, app);
            }
            Ime::Commit(text) => {
                app.ime_composing = false;
                let modifiers = app.current_modifiers.clone();
                Self::dispatch_text(&text, &KeyAction::Pressed, &modifiers, app);
            }
            Ime::Disabled => {
                app.ime_composing = false;
                // Dismissing the input method abandons the composition; without
                // this the field keeps painting ghost preedit text until the
                // next click or blur.
                Self::dispatch_ime_preedit(String::new(), None, app);
            }
        }
    }

    /// Forwards composition state to the focused widget for rendering.
    ///
    /// An empty `text` ends the composition. The redraw is driven by the
    /// dispatch result, so a preedit no field consumed does not repaint.
    pub(crate) fn dispatch_ime_preedit<W: Widget + 'static>(
        text: String,
        cursor: Option<(usize, usize)>,
        app: &mut AimerApplicationHandler<W>,
    ) {
        if app.active_root().is_none() {
            return;
        }
        let event = ElementEvent::ImePreedit { text, cursor };
        let result = app.dispatch_element_event(app.cursor_pos, &event);
        let handled = result.is_consumed();
        if Self::should_redraw(result, handled) {
            app.request_full_redraw();
        }
    }

    /// Feeds a platform wheel event into the frame smoother.
    ///
    /// The `phase` travels with the delta instead of being dropped here: the
    /// smoother spreads one platform pulse across several frames, so only it
    /// knows which of those frames opens and which one closes the gesture.
    /// Widgets then receive `Scroll` events whose phase describes the real
    /// gesture — a trackpad lift ends the scroll, and a plain mouse wheel,
    /// which never reports a phase of its own, ends when its distance is spent.
    pub fn handle_mouse_wheel<W: Widget + 'static>(
        delta: MouseScrollDelta,
        phase: TouchPhase,
        app: &mut AimerApplicationHandler<W>,
    ) {
        const DELTA_MULTIPLY: f64 = if cfg!(target_os = "windows") {
            4.5
        } else {
            1.0
        };
        let (scroll_delta, kind) = Self::normalize_wheel_delta(delta, app.window_scale);

        // Where the platform has no pinch event, `ctrl` + wheel is the zoom
        // gesture: it is what a precision touchpad's pinch is delivered as on
        // Windows and in the browsers, and what a mouse has always done. If no
        // widget under the cursor zooms, the wheel scrolls as usual.
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        if app.current_modifiers.ctrl && Self::dispatch_wheel_zoom(app, scroll_delta, kind) {
            return;
        }

        if app.active_root().is_some() {
            aimer_widget::record_scroll_event();
            let delta = PhysicalPosition::new(
                scroll_delta.x as f64 * DELTA_MULTIPLY,
                scroll_delta.y as f64 * DELTA_MULTIPLY,
            );
            match kind {
                aimer_events::element::ScrollDeltaKind::Pixel => {
                    app.scroll_smoother.on_pixel_delta(delta, phase);
                }
                aimer_events::element::ScrollDeltaKind::Line => {
                    app.scroll_smoother.on_wheel_delta(delta, phase);
                }
            }

            // Keep wheel input on the same frame-synchronized path as the
            // scroll step delivered during `begin_frame`. A direct redraw here
            // can race that later request and make one input burst produce an
            // extra native frame; the platform requester coalesces the wake
            // until the display loop delivers it.
            crate::aimer_app::request_scroll_frame(|| app.request_animation_frame());
        }
    }

    /// Delivers a gesture the windowing system recognized to the widget tree at
    /// the cursor, and asks for a frame if anything reacted.
    pub(super) fn dispatch_platform_gesture<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event: ElementEvent,
    ) -> EventResult {
        if app.active_root().is_none() {
            return EventResult::ignored();
        }
        let result = app.dispatch_element_event(app.cursor_pos, &event);
        if Self::should_redraw(result, result.is_consumed()) {
            app.request_animation_frame();
        }
        result
    }

    /// Turns one `ctrl` + wheel event into a complete pinch, and reports whether
    /// a widget took it.
    ///
    /// A wheel event has no beginning or end of its own, so each one is a whole
    /// gesture — start, step, end — rather than a phase of a longer one. That
    /// keeps a lost event from leaving a pinch open forever, and costs a
    /// consumer nothing: it applies `delta_scale` as it arrives, or commits its
    /// base scale at each end.
    ///
    /// The end is sent even if nobody consumed the start, so a detector that
    /// merely observes does not stay mid-pinch. Only a consumed start stops the
    /// wheel from scrolling.
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    fn dispatch_wheel_zoom<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        delta: Vec2d,
        kind: aimer_events::element::ScrollDeltaKind,
    ) -> bool {
        let step = platform_gesture::wheel_zoom_step(delta, kind);
        if step == 0.0 {
            return false;
        }
        let pos = app.cursor_pos;
        let started = Self::dispatch_platform_gesture(
            app,
            platform_gesture::pinch(pos, f64::from(step), TouchPhase::Started),
        );
        Self::dispatch_platform_gesture(
            app,
            platform_gesture::pinch(pos, 0.0, TouchPhase::Ended),
        );
        started.is_consumed()
    }

    /// Resolves the gesture phase of one browser wheel event.
    ///
    /// The DOM `wheel` event carries no phase, so winit's web backend emits
    /// every event as [`TouchPhase::Moved`]. The boundaries are inferred from
    /// cadence by [`WebScrollPhase`](crate::handler::web_scroll_phase::WebScrollPhase)
    /// instead. A restart means the previous gesture was still open when the
    /// idle gap had already elapsed — no frame was rendered during the pause,
    /// so the missing end is injected here before the new gesture opens.
    #[cfg(target_arch = "wasm32")]
    fn web_wheel_phase<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>) -> TouchPhase {
        use crate::handler::web_scroll_phase::WebScrollTransition;

        match app.web_scroll_phase.on_wheel() {
            WebScrollTransition::Continue => TouchPhase::Moved,
            WebScrollTransition::Begin => TouchPhase::Started,
            WebScrollTransition::Restart => {
                app.scroll_smoother.end_gesture();
                TouchPhase::Started
            }
        }
    }

    /// Closes an open browser scroll gesture and schedules its final frame.
    ///
    /// The terminating phase travels through the smoother, so a redraw is
    /// requested to make sure the widget tree actually receives it.
    #[cfg(target_arch = "wasm32")]
    fn end_web_scroll_gesture<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>) {
        if !app.web_scroll_phase.end() {
            return;
        }
        app.scroll_smoother.end_gesture();
        crate::aimer_app::request_scroll_frame(|| app.request_animation_frame());
    }

    pub(crate) const SCROLL_MULTIPLIER: f32 = 1.5;

    #[inline]
    fn normalize_wheel_delta(
        delta: MouseScrollDelta,
        scale_factor: f64,
    ) -> (Vec2d, aimer_events::element::ScrollDeltaKind) {
        match delta {
            MouseScrollDelta::LineDelta(x, y) => (
                Vec2d {
                    x: x * 20.0,
                    y: y * 20.0,
                },
                aimer_events::element::ScrollDeltaKind::Line,
            ),
            MouseScrollDelta::PixelDelta(pos) => {
                let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
                    scale_factor
                } else {
                    1.0
                };
                (
                    Vec2d {
                        x: (pos.x / scale) as f32 * Self::SCROLL_MULTIPLIER,
                        y: (pos.y / scale) as f32 * Self::SCROLL_MULTIPLIER,
                    },
                    aimer_events::element::ScrollDeltaKind::Pixel,
                )
            }
        }
    }

    #[cfg(not(target_os = "ios"))]
    fn handle_resize<W: Widget + 'static>(
        size: PhysicalSize<u32>,
        app: &mut AimerApplicationHandler<W>,
        _event_loop: &ActiveEventLoop,
    ) {
        #[cfg(target_os = "macos")]
        // The fields are named, not read through `native_window`, so the borrow of
        // the window does not cover the rest of the application.
        if let Some(window) = app.window.as_ref().and_then(aimer_widget::base::WindowHandle::native_window) {
            app.macos_windowing.window_layout_changed(window);
        }

        #[cfg(target_os = "android")]
        let size = {
            if let Some(android_app) = crate::aimer_app::ANDROID_APP.get() {
                if let Some(window) = android_app.native_window() {
                    let width = window.width() as u32;
                    let height = window.height() as u32;
                    winit::dpi::PhysicalSize::new(width, height)
                } else {
                    size
                }
            } else {
                size
            }
        };

        Self::apply_resize(size, app);

        // Desktop compositors need the new drawable before this callback
        // returns to avoid stretching the previous frame to the new bounds.
        app.render(_event_loop);
    }

    /// Prepares the tree for a window that is now `size`.
    ///
    /// A drag delivers one of these per pixel the edge travels, so this runs on
    /// the frame budget: the cached measurements are dropped, because every
    /// constraint below the root has changed, but only the widgets that read the
    /// window metrics while building are rebuilt. Marking the whole tree dirty
    /// instead re-runs every `build` in the application for a window that got
    /// one pixel wider, which on a page of any size is the difference between a
    /// drag that tracks the cursor and one that crawls.
    pub(super) fn apply_resize<W: Widget + 'static>(
        size: PhysicalSize<u32>,
        app: &mut AimerApplicationHandler<W>,
    ) {
        app.pending_resize = Some(size);
        app.native_window_size = Some(aimer_attribute::ResolvedSize {
            width: size.width as f32,
            height: size.height as f32,
        });
        // A platform window has already grown to `size` by the time it reports
        // the resize, so anything that reads the window while the event is
        // handled — a breakpoint, a media query — sees the new one. A headless
        // window only knows what it is told, and is told here.
        app.sync_headless_metrics(Some(size));

        // A window that changed shape may have changed the region the system
        // reserves in it: on a phone, this event is the rotation.
        Self::refresh_safe_area(app);

        if let Some(root) = app.active_root() {
            root.invalidate_layout();
            app.notify_window_metrics_changed();
        }
    }

    /// Brings `app.cursor_pos` up to date for a file drag, and reports the
    /// position the event should carry.
    ///
    /// winit attaches no position to the file events, and macOS delivers no
    /// cursor motion at all while a drag session is running, so on that
    /// platform the position is queried directly from AppKit. Everywhere else
    /// the last cursor position is the best answer available and is used as-is:
    /// it is correct on the platforms that keep sending motion during a drag,
    /// and no worse than the previous behaviour on the ones that do not.
    fn refresh_file_drag_cursor<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
    ) -> Option<Vec2d> {
        #[cfg(target_os = "macos")]
        if let Some(window) = app.native_window()
            && let Some(pos) = crate::ffi_utils::macos_drag::cursor_in_window(window)
        {
            app.cursor_pos = pos;
        }

        Some(app.cursor_pos)
    }

    /// Converts a physical cursor position into the logical coordinates every
    /// element is measured in.
    #[inline]
    fn logical_cursor<W: Widget + 'static>(
        position: PhysicalPosition<f64>,
        app: &AimerApplicationHandler<W>,
    ) -> Vec2d {
        let scale = app.window_scale as f32;
        Vec2d {
            x: position.x as f32 / scale,
            y: position.y as f32 / scale,
        }
    }

    /// Reports the file drag in flight at `at`, if it has travelled far enough
    /// to matter.
    ///
    /// The platform announces a file drag *entering* the window and then falls
    /// silent, so this is what keeps the drop zones informed: one hit-tested
    /// [`ElementEvent::HoveredFileMoved`] carrying the whole batch, whatever its
    /// size.
    ///
    /// A drag that reaches nothing — wandering over the background between two
    /// zones — is a leave, and the zone that lit up has to hear it, so a
    /// [`ElementEvent::DragLeave`] is broadcast on the move that leaves and on no
    /// other: it is the one case where the drag has no addressee to route to.
    fn report_file_drag_move<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>, at: Vec2d) {
        let Some(paths) = app.file_drag.moved_to(at) else {
            return;
        };
        app.cursor_pos = at;

        let event = ElementEvent::HoveredFileMoved { paths, pos: at };
        let mut result = app.dispatch_element_event(at, &event);

        if app.file_drag.note_answered(result.is_consumed()) {
            let left = ElementEvent::DragLeave {
                source: PointerSource::Mouse,
                id: FILE_DRAG_POINTER_ID,
            };
            result = result.merge(app.broadcast_element_event(&left));
        }

        if Self::should_redraw(result, true) {
            app.request_full_redraw();
        }
    }

    /// Asks the platform where the file drag is now, and reports it.
    ///
    /// Called once per frame while a drag is in flight, because macOS runs its
    /// drag session without delivering any cursor motion at all: there is no
    /// event to react to, only a question to ask. Every other platform keeps
    /// sending [`WindowEvent::CursorMoved`] throughout the drag and needs no
    /// polling, which is why this is compiled for macOS alone.
    #[cfg(target_os = "macos")]
    pub(crate) fn poll_file_drag<W: Widget + 'static>(app: &mut AimerApplicationHandler<W>) {
        if let Some(at) = Self::refresh_file_drag_cursor(app) {
            Self::report_file_drag_move(app, at);
        }
    }

    fn handle_generic_event<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event: &ElementEvent,
    ) {
        // info!("ElementEvent : {:?}", event);
        let mut result = app.dispatch_element_event(app.cursor_pos, event);

        // A cancelled file drag carries no position, and the region that lit up
        // is not necessarily the one the cursor rests on now. Everyone who
        // reacted to the drag has to hear that it is over, so this one is
        // broadcast rather than hit-tested.
        if matches!(event, ElementEvent::HoveredFileCancelled) {
            result = result.merge(app.broadcast_element_event(event));
        }

        if result.needs_redraw() {
            app.request_full_redraw();
        }
    }
}

#[cfg(test)]
#[path = "event_handler/tests.rs"]
mod tests;
