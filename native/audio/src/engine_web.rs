//! The engine on the web, where there are no threads.
//!
//! The engine lives in a thread-local on the page's thread. A command is
//! handled the moment it is sent (the web backend prepares every source in
//! place, so nothing waits on a helper thread), and a `setInterval` timer
//! ticks it while any track needs ticking, cleared again when none does.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{Command, Engine, TICK};
use crate::backend::Backend;

struct Runtime {
    engine: Engine<crate::PlatformBackend>,
    /// Commands sent while the engine was busy, such as from a callback that
    /// ran inside it.
    inbox: Receiver<Command>,
    /// The ticking timer, while one is set.
    interval: Option<i32>,
    /// Created once and kept: a closure must not be dropped while the timer
    /// that calls it may still fire.
    on_tick: Closure<dyn FnMut()>,
}

aimer_tls::thread_local! {
    static OUTBOX: Sender<Command> = {
        let (tx, rx) = mpsc::channel();
        INBOX.with(|inbox| *inbox.borrow_mut() = Some(rx));
        tx
    };
    static INBOX: RefCell<Option<Receiver<Command>>> = const { RefCell::new(None) };
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Creates the engine on first use.
pub(crate) fn start() {
    let tx = OUTBOX.with(Clone::clone);
    RUNTIME.with(|runtime| {
        let Ok(mut runtime) = runtime.try_borrow_mut() else {
            return;
        };
        if runtime.is_some() {
            return;
        }
        let Some(inbox) = INBOX.with(|inbox| inbox.borrow_mut().take()) else {
            return;
        };
        *runtime = Some(Runtime {
            engine: Engine::new(crate::PlatformBackend::new(), tx),
            inbox,
            interval: None,
            on_tick: Closure::new(on_tick),
        });
    });
}

/// Hands `command` to the engine and applies it at once.
pub(crate) fn send(command: Command) -> bool {
    start();
    if OUTBOX.with(|tx| tx.send(command)).is_err() {
        return false;
    }
    pump(false);
    true
}

fn on_tick() {
    pump(true);
}

/// Applies queued commands, ticks if asked, and sets or clears the timer.
fn pump(tick: bool) {
    RUNTIME.with(|runtime| {
        // Busy: this call came from inside the engine; the outer call drains.
        let Ok(mut runtime) = runtime.try_borrow_mut() else {
            return;
        };
        let Some(runtime) = runtime.as_mut() else {
            return;
        };
        runtime.engine.handle_pending(None, &runtime.inbox);
        if tick {
            runtime.engine.tick();
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        match (runtime.engine.needs_tick(), runtime.interval) {
            (true, None) => {
                runtime.interval = window
                    .set_interval_with_callback_and_timeout_and_arguments_0(
                        runtime.on_tick.as_ref().unchecked_ref(),
                        TICK.as_millis() as i32,
                    )
                    .ok();
            }
            (false, Some(handle)) => {
                window.clear_interval_with_handle(handle);
                runtime.interval = None;
            }
            _ => {}
        }
    });
}
