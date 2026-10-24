//! The thread-local state a window brings onto the thread while it is active.
//!
//! Several libraries keep state in thread-locals that belongs to one window: a
//! dialog is presented in a window, keyboard focus is confined within one, and
//! what has changed since the last frame is what changed *there*. A host with
//! several windows on a thread keeps one copy of each per window and puts the
//! active window's on the thread (see
//! [`aimer_events::window::enter_window_scope`]).
//!
//! Whatever was recorded while no window was active has no window to wait for,
//! so the window that becomes active takes it.

use std::cell::RefCell;

use aimer_events::window::WindowLocals;
use aimer_modal::WindowModalState;
use aimer_widget::components::element::WindowPaintState;
use aimer_widget::focus::WindowFocusTraps;

/// Everything of one window that lives in a thread-local while it is active.
#[derive(Default)]
pub(crate) struct WindowThreadState {
    paint: RefCell<WindowPaintState>,
    modal: RefCell<WindowModalState>,
    focus: RefCell<WindowFocusTraps>,
}

impl WindowLocals for WindowThreadState {
    fn install(&self) {
        let mut paint = self.paint.borrow_mut();
        paint.swap_with_thread();
        paint.absorb_into_thread();
        let mut modal = self.modal.borrow_mut();
        modal.swap_with_thread();
        modal.absorb_into_thread();
        self.focus.borrow_mut().swap_with_thread();
    }

    fn uninstall(&self) {
        self.focus.borrow_mut().swap_with_thread();
        self.modal.borrow_mut().swap_with_thread();
        self.paint.borrow_mut().swap_with_thread();
    }
}
