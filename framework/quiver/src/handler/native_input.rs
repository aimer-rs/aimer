use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, Modifiers, ScrollDeltaKind};
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_widget::Widget;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase};

use super::{AimerApplicationHandler, event_handler::WindowEventHandler};

impl WindowEventHandler {
    /// The wheel delta winit would have reported for a scroll of `kind`.
    ///
    /// A pixel delta arrives in logical pixels and goes back to physical ones,
    /// which `handle_mouse_wheel` divides out again; a line delta counts wheel
    /// steps and has no scale.
    fn wheel_delta(delta: Vec2d, kind: ScrollDeltaKind, scale: f64) -> MouseScrollDelta {
        match kind {
            ScrollDeltaKind::Pixel => MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                f64::from(delta.x) * scale,
                f64::from(delta.y) * scale,
            )),
            ScrollDeltaKind::Line => MouseScrollDelta::LineDelta(delta.x, delta.y),
        }
    }

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
            // A finger a driver injected: the same path a platform touch takes.
            ElementEvent::PointerDown(pointer)
            | ElementEvent::PointerMove(pointer)
            | ElementEvent::PointerUp(pointer)
                if pointer.source == PointerSource::Touch =>
            {
                let phase = match event {
                    ElementEvent::PointerDown(_) => TouchPhase::Started,
                    ElementEvent::PointerMove(_) => TouchPhase::Moved,
                    _ => TouchPhase::Ended,
                };
                Self::handle_touch_contact(pointer.pos, pointer.id, phase, app);
            }
            ElementEvent::HoveredFile { path, .. } => Self::handle_file_hovered(path, app),
            ElementEvent::HoveredFileCancelled => Self::handle_file_hover_cancelled(app),
            ElementEvent::DroppedFile { path, .. } => Self::handle_file_dropped(path, app),
            ElementEvent::PointerMove(pointer) if pointer.source == PointerSource::Mouse => {
                // Files in tow make a move the file drag continuing, as
                // `WindowEvent::CursorMoved` does.
                if app.file_drag.is_active() {
                    Self::report_file_drag_move(app, pointer.pos);
                    return;
                }
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
            ElementEvent::Scroll { delta, phase, kind, .. } => {
                Self::handle_mouse_wheel(Self::wheel_delta(delta, kind, scale), phase, app);
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

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_attribute::position::Vec2d;
    use aimer_events::element::ScrollDeltaKind;

    #[test]
    fn a_pixel_scroll_is_scaled_to_physical_pixels() {
        let delta = WindowEventHandler::wheel_delta(Vec2d { x: 3.0, y: -4.0 }, ScrollDeltaKind::Pixel, 2.0);
        assert!(matches!(delta, MouseScrollDelta::PixelDelta(p) if p.x == 6.0 && p.y == -8.0));
    }

    #[test]
    fn a_line_scroll_stays_in_lines_whatever_the_scale() {
        let delta = WindowEventHandler::wheel_delta(Vec2d { x: 0.0, y: -3.0 }, ScrollDeltaKind::Line, 2.0);
        assert!(matches!(delta, MouseScrollDelta::LineDelta(x, y) if x == 0.0 && y == -3.0));
    }
}
