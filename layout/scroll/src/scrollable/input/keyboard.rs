//! Keyboard scrolling: arrows and pages glide, Home and End ease to an end.

use aimer_animation::curves;
use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_widget::{Element, EventResult};

use super::handle_scroll::Flow;
use crate::ScrollAxis;
use crate::raw_scroll::RawScrollableContainer;
use crate::scrollable::constants::*;
use crate::scrollable::overscroll_source::OverscrollSource;

impl<E: Element> RawScrollableContainer<E> {
    /// Handles a key press or platform repeat.
    ///
    /// A held key arrives as a press followed by repeats, and each one is
    /// another step: the glide builds on itself, so holding scrolls
    /// continuously. A key the content already consumed is left alone.
    pub(super) fn on_key_press(&self, event: &ElementEvent, child_result: EventResult) -> Flow {
        let ElementEvent::KeyInput {
            key,
            action: KeyAction::Pressed | KeyAction::Repeat,
            ..
        } = event
        else {
            return Flow::Continue(false);
        };
        if child_result.is_consumed() {
            return Flow::Return(child_result);
        }
        let scale = self.ctrl.last_scale.get();
        let (vp_w, vp_h) = self.ctrl.cached_viewport.get();
        let line = self.ctrl.key_scroll_strength * scale;
        let page_v = vp_h * KEYBOARD_PAGE_FRACTION;
        let page_h = vp_w * KEYBOARD_PAGE_FRACTION;

        // Steps glide: a push that carries the content on and settles.
        // Home and End name a destination, so they ease to it.
        let max = self.ctrl.cached_max_scroll.get();
        let vertical = matches!(self.ctrl.axis, ScrollAxis::Vertical);
        let (step, destination) = match (&self.ctrl.axis, key) {
            (ScrollAxis::Vertical, NamedKey::ArrowUp) => (Some(Vec2d { x: 0.0, y: line }), None),
            (ScrollAxis::Vertical, NamedKey::ArrowDown) => (Some(Vec2d { x: 0.0, y: -line }), None),
            (ScrollAxis::Vertical, NamedKey::PageUp) => (Some(Vec2d { x: 0.0, y: page_v }), None),
            (ScrollAxis::Vertical, NamedKey::PageDown) => (Some(Vec2d { x: 0.0, y: -page_v }), None),
            (ScrollAxis::Horizontal, NamedKey::ArrowLeft) => (Some(Vec2d { x: line, y: 0.0 }), None),
            (ScrollAxis::Horizontal, NamedKey::ArrowRight) => (Some(Vec2d { x: -line, y: 0.0 }), None),
            (ScrollAxis::Horizontal, NamedKey::PageUp) => (Some(Vec2d { x: page_h, y: 0.0 }), None),
            (ScrollAxis::Horizontal, NamedKey::PageDown) => (Some(Vec2d { x: -page_h, y: 0.0 }), None),
            // Top: offset 0. Bottom: -max_scroll.
            (_, NamedKey::Home) => (None, Some(Vec2d::ZERO)),
            (_, NamedKey::End) => (
                None,
                Some(if vertical {
                    Vec2d { x: 0.0, y: -max.y }
                } else {
                    Vec2d { x: -max.x, y: 0.0 }
                }),
            ),
            _ => (None, None),
        };

        let handled = if let Some(step) = step {
            self.ctrl.set_overscroll_source(OverscrollSource::Keyboard);
            self.ctrl.glide_by(step, true);
            true
        } else if let Some(destination) = destination {
            self.ctrl.set_overscroll_source(OverscrollSource::Keyboard);
            self.ctrl.clear_velocity_history();
            self.ctrl.animate_scroll_to(
                destination,
                KEYBOARD_SCROLL_DURATION_S,
                curves::EASE_OUT,
            );
            true
        } else {
            false
        };
        Flow::Continue(handled)
    }
}
