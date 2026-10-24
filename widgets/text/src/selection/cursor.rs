use std::cell::Cell;

use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_utils::cursor::CursorIcon;
use aimer_widget::base::WindowHandle;

/// The hover cursor policy of one selectable text element.
///
/// The precedence is fixed: a hovered link wins with [`CursorIcon::Pointer`],
/// glyphs of a selectable text show the I-beam [`CursorIcon::Text`], and
/// anything else restores the platform default. Only a **mouse** ever changes
/// the cursor; touch and pen never do.
///
/// The element remembers the shape it last asked for, so a mouse crossing a
/// paragraph issues exactly one platform call per shape *change* instead of one
/// per move event.
///
/// # Examples
///
/// ```ignore
/// let cursor = HoverCursor::new();
/// cursor.apply(&window, &event, true, false, true); // I-beam over glyphs
/// ```
#[derive(Default)]
pub(crate) struct HoverCursor {
    current: Cell<Option<CursorIcon>>,
}

impl HoverCursor {
    /// Creates a policy that has not requested any shape yet.
    #[inline]
    pub fn new() -> Self {
        Self {
            current: Cell::new(None),
        }
    }

    /// The shape `event` asks for, or `None` when the cursor must be left alone.
    ///
    /// `selectable` and `over_link` describe the element, `over_glyphs` whether
    /// the pointer is over real glyph geometry rather than merely inside the
    /// element's bounds.
    pub fn resolve(
        selectable: bool,
        over_link: bool,
        over_glyphs: bool,
        event: &ElementEvent,
    ) -> Option<CursorIcon> {
        if let Some(pointer) = event.pointer() {
            if pointer.source != PointerSource::Mouse {
                return None;
            }
            return if over_link {
                Some(CursorIcon::Pointer)
            } else if selectable && over_glyphs {
                Some(CursorIcon::Text)
            } else {
                Some(CursorIcon::Default)
            };
        }
        match event {
            ElementEvent::PointerExited(PointerSource::Mouse, _) | ElementEvent::Cancel => {
                Some(CursorIcon::Default)
            }
            _ => None,
        }
    }

    /// Applies the policy, writing to the window only when the shape changes.
    ///
    /// Leaving the element restores the default **only if this element still
    /// owns the current shape**, so moving straight onto a neighbouring text
    /// does not undo the shape the neighbour just requested.
    ///
    /// Returns whether *this event* left the element owning a non-default
    /// shape. The caller must **consume** the event when it does: an unconsumed
    /// pointer move makes the application reset the cursor to the platform
    /// default. Events the policy does not act on — a key press, a touch move —
    /// always report `false`, so they stay free to travel up the tree.
    pub fn apply(
        &self,
        window: &WindowHandle,
        event: &ElementEvent,
        selectable: bool,
        over_link: bool,
        over_glyphs: bool,
    ) -> bool {
        let Some(desired) = Self::resolve(selectable, over_link, over_glyphs, event) else {
            return false;
        };
        if desired == CursorIcon::Default {
            if self.current.take().is_some() {
                window.reset_cursor();
            }
            return false;
        }
        if self.current.get() != Some(desired) {
            self.current.set(Some(desired));
            window.set_cursor(desired);
        }
        true
    }

    /// The shape this element currently owns, if any.
    #[cfg(test)]
    pub fn owned(&self) -> Option<CursorIcon> {
        self.current.get()
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_utils::cursor::CursorIcon;
    use aimer_widget::base::WindowHandle;

    use super::HoverCursor;

    fn window() -> WindowHandle {
        WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0)
    }

    fn moved(source: PointerSource) -> ElementEvent {
        ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 5.0, y: 5.0 },
            source,
            0,
            PointerButton::Primary,
        ))
    }

    #[test]
    fn glyphs_of_a_selectable_text_show_the_i_beam() {
        let window = window();
        let cursor = HoverCursor::new();

        assert!(cursor.apply(&window, &moved(PointerSource::Mouse), true, false, true));

        assert_eq!(window.headless_cursor(), Some(CursorIcon::Text));
    }

    #[test]
    fn a_hovered_link_outranks_the_i_beam() {
        let window = window();
        let cursor = HoverCursor::new();

        assert!(cursor.apply(&window, &moved(PointerSource::Mouse), true, true, true));

        assert_eq!(window.headless_cursor(), Some(CursorIcon::Pointer));
    }

    #[test]
    fn inside_the_bounds_but_away_from_glyphs_stays_on_the_default() {
        let window = window();
        let cursor = HoverCursor::new();

        assert!(!cursor.apply(&window, &moved(PointerSource::Mouse), true, false, false));

        assert_eq!(window.headless_cursor(), Some(CursorIcon::Default));
        assert_eq!(cursor.owned(), None);
    }

    #[test]
    fn an_event_the_policy_ignores_is_never_claimed() {
        let window = window();
        let cursor = HoverCursor::new();
        cursor.apply(&window, &moved(PointerSource::Mouse), true, false, true);

        let claimed = cursor.apply(
            &window,
            &ElementEvent::KeyInput {
                key: aimer_events::element::NamedKey::Other("a".into()),
                action: aimer_events::element::KeyAction::Pressed,
                modifiers: Default::default(),
            },
            true,
            false,
            false,
        );

        assert!(!claimed);
        assert_eq!(cursor.owned(), Some(CursorIcon::Text));
    }

    #[test]
    fn a_touch_pointer_never_changes_the_cursor() {
        let window = window();
        let cursor = HoverCursor::new();

        assert!(!cursor.apply(&window, &moved(PointerSource::Touch), true, true, true));

        assert_eq!(window.headless_cursor(), Some(CursorIcon::Default));
        assert_eq!(cursor.owned(), None);
    }

    #[test]
    fn a_text_that_is_neither_selectable_nor_linked_leaves_the_cursor_alone() {
        let cursor = HoverCursor::new();

        assert_eq!(
            HoverCursor::resolve(false, false, true, &moved(PointerSource::Mouse)),
            Some(CursorIcon::Default)
        );
        assert_eq!(cursor.owned(), None);
    }

    #[test]
    fn repeated_moves_over_the_same_text_request_one_shape_only() {
        let window = window();
        let cursor = HoverCursor::new();

        let _ = cursor.apply(&window, &moved(PointerSource::Mouse), true, false, true);
        window.set_cursor(CursorIcon::Crosshair);
        let _ = cursor.apply(&window, &moved(PointerSource::Mouse), true, false, true);

        assert_eq!(window.headless_cursor(), Some(CursorIcon::Crosshair));
    }

    #[test]
    fn leaving_resets_only_a_shape_this_element_still_owns() {
        let window = window();
        let neighbour = HoverCursor::new();
        let left = HoverCursor::new();

        let _ = neighbour.apply(&window, &moved(PointerSource::Mouse), true, false, true);
        let _ = left.apply(
            &window,
            &ElementEvent::PointerExited(PointerSource::Mouse, 0),
            true,
            false,
            false,
        );

        assert_eq!(window.headless_cursor(), Some(CursorIcon::Text));
        assert_eq!(neighbour.owned(), Some(CursorIcon::Text));
    }
}
