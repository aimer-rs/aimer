//! Pointer, focus, and keyboard plumbing shared by every control.
//!
//! The transient flags a control paints from — hovered, pressed, focused —
//! change on almost every pointer event, so they are kept out of the control's
//! `State`. They live in one shared [`Cell`] that the event handlers write
//! immediately and the next build reads. A change that does not alter a flag is
//! dropped before it reaches the state queue, and a change that does alter one
//! queues a single no-op mutation purely to request a rebuild.
//!
//! The handler closures capture only that cell and the state's updater, so
//! they are created once in `init_state` and a rebuild merely clones the `Rc`s.

use std::cell::Cell;
use std::rc::Rc;

use aimer_input::gesture::GestureEvent;
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::mouse_region::MouseRegion;
use aimer_widget::{AnyWidget, FocusNode, Focusable, StateUpdater, Widget};

use crate::keys::{Key, KeyRelay};

/// Transient interaction flags read by a control's build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    pub(crate) hovered: bool,
    pub(crate) pressed: bool,
    pub(crate) focused: bool,
    /// Whether the most recent interaction was a key press. A focus ring is
    /// drawn only then, so clicking a control does not leave a ring behind.
    pub(crate) keyboard: bool,
}

impl Flags {
    /// Whether the focus ring should be drawn.
    #[inline]
    pub(crate) fn focus_visible(self) -> bool {
        self.focused && self.keyboard
    }
}

/// What a control state must answer for the shared handlers.
pub(crate) trait Controlled: 'static {
    /// Whether `key` is handled, so the event is consumed. Read synchronously.
    fn consumes_key(&self, key: Key) -> bool;

    /// Applies a key already accepted by [`Self::consumes_key`].
    fn on_key(&mut self, key: Key);

    /// Applies a completed tap.
    fn on_tap(&mut self);
}

/// The event handlers and live flags of one control.
pub(crate) struct Handlers {
    flags: Rc<Cell<Flags>>,
    tap: Rc<dyn Fn()>,
    press: Rc<dyn Fn(bool)>,
    hover: Rc<dyn Fn(bool)>,
    focus: Rc<dyn Fn(bool)>,
    key: Rc<dyn Fn(Key) -> bool>,
}

impl Clone for Handlers {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            flags: Rc::clone(&self.flags),
            tap: Rc::clone(&self.tap),
            press: Rc::clone(&self.press),
            hover: Rc::clone(&self.hover),
            focus: Rc::clone(&self.focus),
            key: Rc::clone(&self.key),
        }
    }
}

/// Writes one flag and requests a rebuild only if it changed.
fn update_flags<S: Controlled>(
    flags: &Cell<Flags>,
    updater: StateUpdater<S>,
    change: impl FnOnce(&mut Flags),
) {
    let before = flags.get();
    let mut after = before;
    change(&mut after);
    if after != before {
        flags.set(after);
        updater.set_state(|_| {});
    }
}

impl Handlers {
    /// Creates the handlers for the state behind `updater`.
    pub(crate) fn new<S: Controlled>(updater: StateUpdater<S>) -> Self {
        let flags = Rc::new(Cell::new(Flags::default()));
        Self {
            tap: Rc::new(move || updater.set_state(|state| state.on_tap())),
            press: Rc::new({
                let flags = Rc::clone(&flags);
                move |pressed| {
                    update_flags(&flags, updater, |flags| {
                        flags.pressed = pressed;
                        if pressed {
                            flags.keyboard = false;
                        }
                    })
                }
            }),
            hover: Rc::new({
                let flags = Rc::clone(&flags);
                move |hovered| update_flags(&flags, updater, |flags| flags.hovered = hovered)
            }),
            focus: Rc::new({
                let flags = Rc::clone(&flags);
                move |focused| {
                    update_flags(&flags, updater, |flags| {
                        flags.focused = focused;
                        if !focused {
                            flags.keyboard = false;
                        }
                    })
                }
            }),
            key: Rc::new({
                let flags = Rc::clone(&flags);
                move |key| {
                    if !updater.read(|state| state.consumes_key(key)) {
                        return false;
                    }
                    update_flags(&flags, updater, |flags| flags.keyboard = true);
                    updater.set_state(move |state| state.on_key(key));
                    true
                }
            }),
            flags,
        }
    }

    /// The flags as of the latest event.
    #[inline]
    pub(crate) fn flags(&self) -> Flags {
        self.flags.get()
    }

    /// Clears hover and press, which a disabled control cannot have.
    #[inline]
    pub(crate) fn clear_pointer(&self) {
        let mut flags = self.flags.get();
        flags.hovered = false;
        flags.pressed = false;
        self.flags.set(flags);
    }

    /// Wraps `child` so it is focusable and keyboard operable, for a control
    /// whose parts handle the pointer themselves.
    pub(crate) fn wrap_keys(&self, disabled: bool, node: &FocusNode, child: AnyWidget) -> AnyWidget {
        if disabled {
            return child;
        }
        let focus = Rc::clone(&self.focus);
        let key = Rc::clone(&self.key);
        Focusable::new()
            .node(node.clone())
            .on_focus_change(move |focused| focus(focused))
            .child(KeyRelay::new().on_key(move |pressed| key(pressed)).child(child))
            .boxed()
    }

    /// Wraps `child` so it is focusable, hoverable, tappable, and keyboard
    /// operable. A disabled control is returned unwrapped: it takes no focus
    /// and no pointer.
    pub(crate) fn wrap(&self, disabled: bool, node: &FocusNode, child: AnyWidget) -> AnyWidget {
        if disabled {
            return child;
        }
        let Self {
            tap,
            press,
            hover,
            focus,
            key,
            ..
        } = self.clone();
        let hover_exit = Rc::clone(&hover);
        Focusable::new()
            .node(node.clone())
            .on_focus_change(move |focused| focus(focused))
            .child(
                KeyRelay::new().on_key(move |pressed| key(pressed)).child(
                    MouseRegion::new()
                        .on_hover_enter(move || hover(true))
                        .on_hover_exit(move || hover_exit(false))
                        .child(
                            GestureDetector::new()
                                .on_tap(move || tap())
                                .on_gesture(move |event: GestureEvent| match event {
                                    GestureEvent::TapDown { .. } => press(true),
                                    GestureEvent::TapUp { .. } | GestureEvent::TapCancel => {
                                        press(false)
                                    }
                                    _ => {}
                                })
                                .child(child),
                        ),
                ),
            )
            .boxed()
    }
}
