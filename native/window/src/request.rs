//! What the application asks of the event loop, and the queue it waits in.
//!
//! Only the event loop can create, resize or close a window, and the widgets
//! that ask are not `Send`. So a request is put in a queue that lives on the UI
//! thread, the loop is woken, and the loop drains the queue the next time it
//! runs. Nothing here blocks, and nothing here waits for the answer.

use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use aimer_widget::AnyWidget;
use aimer_widget::components::context::InheritedStates;

use crate::{WindowAttr, WindowId};

/// A change to a window that already exists.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    /// Closes the window.
    Close,
    /// Brings the window to the front and gives it the keyboard.
    Focus,
    /// Replaces the title in the title bar.
    SetTitle(String),
    /// Asks for a new content size, in logical pixels.
    Resize(u32, u32),
    /// Minimizes or restores the window.
    SetMinimized(bool),
    /// Maximizes or restores the window.
    SetMaximized(bool),
}

/// A new window to create.
#[doc(hidden)]
pub struct OpenRequest {
    pub id: WindowId,
    pub attr: WindowAttr,
    /// The root of the window's tree.
    pub root: AnyWidget,
    /// The providers the window starts with, if it inherits any.
    pub inherited: Option<Rc<RefCell<InheritedStates>>>,
    /// Called on the UI thread once the window has closed, however it closed.
    pub on_closed: Option<Box<dyn FnOnce()>>,
}

/// One request, in the order it was made.
#[doc(hidden)]
pub enum Request {
    Open(OpenRequest),
    Command(WindowId, WindowCommand),
}

aimer_tls::thread_local! {
    static QUEUE: RefCell<VecDeque<Request>> = const { RefCell::new(VecDeque::new()) };
    static OPEN: RefCell<HashSet<WindowId>> = RefCell::new(HashSet::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

pub(crate) fn next_id() -> WindowId {
    NEXT_ID.with(|next| {
        let id = next.get();
        // Zero is the main window's, which exists without having been asked for.
        next.set(id.wrapping_add(1).max(1));
        WindowId::from_raw(id)
    })
}

pub(crate) fn push(request: Request) {
    QUEUE.with(|queue| queue.borrow_mut().push_back(request));
    if let Some(proxy) = aimer_platform::EVENT_PROXY.get() {
        let _ = proxy.send_event(aimer_platform::AimerNativePlatformEvent::WindowRequests);
    }
}

/// Takes every request made so far, oldest first.
#[doc(hidden)]
pub fn drain() -> Vec<Request> {
    QUEUE.with(|queue| queue.borrow_mut().drain(..).collect())
}

/// Records that the host created the window `id`.
#[doc(hidden)]
pub fn mark_open(id: WindowId) {
    OPEN.with(|open| {
        open.borrow_mut().insert(id);
    });
}

/// Records that the window `id` is gone.
#[doc(hidden)]
pub fn mark_closed(id: WindowId) {
    OPEN.with(|open| {
        open.borrow_mut().remove(&id);
    });
}

pub(crate) fn is_open(id: WindowId) -> bool {
    OPEN.with(|open| open.borrow().contains(&id))
}
