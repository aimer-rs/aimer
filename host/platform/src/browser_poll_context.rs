//! Browser timers used by Venus deadlines without a native async runtime.

use std::cell::{Cell, RefCell};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use aimer_venus::PollContext;
use wasm_bindgen::{JsCast, closure::Closure};

pub struct BrowserPollContext;

impl PollContext for BrowserPollContext {
    fn enter(&self, poll: &mut dyn FnMut()) {
        poll();
    }

    fn sleep(&self, duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
        let window = web_sys::window()?;
        let timer = BrowserSleep::new(window.clone(), duration)?;
        Some(Box::pin(async move {
            let mut remaining = timer.remaining;
            timer.await;
            // setTimeout uses a signed 32-bit millisecond delay. Long deadlines
            // are split so they cannot wrap and expire immediately.
            while !remaining.is_zero() {
                let timer = BrowserSleep::new(window.clone(), remaining)
                    .expect("the browser must continue accepting timer registrations");
                remaining = timer.remaining;
                timer.await;
            }
        }))
    }
}

struct TimerState {
    expired: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

struct BrowserSleep {
    window: web_sys::Window,
    id: i32,
    state: Rc<TimerState>,
    remaining: Duration,
    _callback: Closure<dyn FnMut()>,
}

impl BrowserSleep {
    fn new(window: web_sys::Window, duration: Duration) -> Option<Self> {
        const MAX_DELAY: Duration = Duration::from_millis(i32::MAX as u64);
        let chunk = duration.min(MAX_DELAY);
        // Round upwards: a sub-millisecond timeout must never fire early.
        let millis = chunk.as_millis() + u128::from(chunk.subsec_nanos() % 1_000_000 != 0);
        let state = Rc::new(TimerState {
            expired: Cell::new(false),
            waker: RefCell::new(None),
        });
        let target = state.clone();
        let callback = Closure::new(move || {
            target.expired.set(true);
            let waker = target.waker.borrow_mut().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        });
        let id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            millis as i32,
        ).ok()?;
        Some(Self {
            window,
            id,
            state,
            remaining: duration - chunk,
            _callback: callback,
        })
    }
}

impl Future for BrowserSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.state.expired.get() {
            return Poll::Ready(());
        }
        self.state.waker.borrow_mut().replace(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for BrowserSleep {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.id);
    }
}
