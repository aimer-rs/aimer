//! A host whose timers fire only when a test says so.

use std::cell::{Cell, RefCell};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use crate::{PollContext, Venus};

#[derive(Default)]
pub(crate) struct TimerState {
    pub(crate) fired: Cell<bool>,
    pub(crate) dropped: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

struct FakeTimer(Rc<TimerState>);

impl Future for FakeTimer {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0.fired.get() {
            Poll::Ready(())
        } else {
            self.0.waker.borrow_mut().replace(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for FakeTimer {
    fn drop(&mut self) {
        self.0.dropped.set(true);
    }
}

/// A host whose timers only fire when the test says so.
#[derive(Default)]
pub(crate) struct Timers {
    requested: RefCell<Vec<(Duration, Rc<TimerState>)>>,
}

impl Timers {
    pub(crate) fn fire(&self, index: usize) {
        let state = self.requested.borrow()[index].1.clone();
        state.fired.set(true);
        if let Some(waker) = state.waker.borrow_mut().take() {
            waker.wake();
        }
    }

    pub(crate) fn duration(&self, index: usize) -> Duration {
        self.requested.borrow()[index].0
    }

    pub(crate) fn len(&self) -> usize {
        self.requested.borrow().len()
    }

    pub(crate) fn state(&self, index: usize) -> Rc<TimerState> {
        self.requested.borrow()[index].1.clone()
    }
}

impl PollContext for Rc<Timers> {
    fn enter(&self, poll: &mut dyn FnMut()) {
        poll();
    }

    fn sleep(&self, duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
        let state = Rc::new(TimerState::default());
        self.requested.borrow_mut().push((duration, state.clone()));
        Some(Box::pin(FakeTimer(state)))
    }
}

pub(crate) fn with_timers() -> (Rc<Venus>, Rc<Timers>) {
    let venus = Venus::new();
    let timers = Rc::new(Timers::default());
    venus.set_poll_context(timers.clone());
    (venus, timers)
}
