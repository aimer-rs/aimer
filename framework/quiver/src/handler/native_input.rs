use aimer_events::element::{ElementEvent, Modifiers};
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_widget::Widget;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta};

use super::{AimerApplicationHandler, event_handler::WindowEventHandler};

impl WindowEventHandler {
    pub(super) fn handle_native_input<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event: ElementEvent,
        modifiers: Modifiers,
    ) {
        app.current_modifiers = modifiers;
        let scale = if app.window_scale.is_finite() && app.window_scale > 0.0 {
            app.window_scale
        } else {
            1.0
        };
        match event {
            ElementEvent::Cancel => {
                app.pressed_button = None;
                let result = app.cancel_element_events();
                if result.is_consumed() || result.needs_redraw() {
                    app.request_full_redraw();
                }
            }
            ElementEvent::PointerMove(pointer) if pointer.source == PointerSource::Mouse => {
                Self::handle_cursor_move(
                    PhysicalPosition::new(
                        f64::from(pointer.pos.x) * scale,
                        f64::from(pointer.pos.y) * scale,
                    ),
                    app,
                );
            }
            ElementEvent::PointerExited(PointerSource::Mouse, _) => Self::handle_cursor_left(app),
            ElementEvent::PointerDown(pointer) | ElementEvent::PointerUp(pointer)
                if pointer.source == PointerSource::Mouse =>
            {
                // A button event carries its own precise position, even if a
                // hover move was coalesced or moved less than one point.
                app.cursor_pos = pointer.pos;
                app.touch_pointer = false;
                let button = match pointer.button {
                    PointerButton::Primary => MouseButton::Left,
                    PointerButton::Secondary => MouseButton::Right,
                    PointerButton::Middle => MouseButton::Middle,
                    PointerButton::Other(code) => MouseButton::Other(code),
                };
                let state = if matches!(event, ElementEvent::PointerDown(_)) {
                    ElementState::Pressed
                } else {
                    ElementState::Released
                };
                Self::handle_mouse_input(state, button, app);
            }
            ElementEvent::Scroll { delta, phase, .. } => {
                Self::handle_mouse_wheel(
                    MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                        f64::from(delta.x) * scale,
                        f64::from(delta.y) * scale,
                    )),
                    phase,
                    app,
                );
            }
            // UIKit recognizes these itself and reports where they happened, in
            // logical points, so no scale factor applies. Anchoring the cursor
            // first keeps later wheel events on the same widget.
            ElementEvent::Pinch { pos, .. }
            | ElementEvent::Rotate { pos, .. }
            | ElementEvent::Pan { pos, .. }
            | ElementEvent::SmartZoom { pos } => {
                app.cursor_pos = pos;
                Self::dispatch_platform_gesture(app, event);
            }
            ElementEvent::KeyInput { key, action, .. } => {
                if app.ime_composing && !app.current_modifiers.ctrl && !app.current_modifiers.meta {
                    return;
                }
                if app.active_root().is_some() {
                    let result = app.dispatch_element_event(
                        app.cursor_pos,
                        &ElementEvent::KeyInput {
                            key,
                            action,
                            modifiers: app.current_modifiers.clone(),
                        },
                    );
                    if result.is_consumed() || result.needs_redraw() {
                        app.request_full_redraw();
                    }
                }
            }
            _ => {}
        }
    }
}
