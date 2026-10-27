//! Telling the application's own services that the process is about to lose its
//! chance to run.
//!
//! Platforms give an app a moment of warning before the things that cost it
//! unsaved state: it is moved to the background (after which a mobile OS may kill
//! it without a further word), its window loses focus, or it is quitting. The
//! event loop turns those into a [`Lifecycle`] and calls [`notify_lifecycle`];
//! anything that holds work it cannot afford to lose, such as a persistence
//! layer with writes still pending, registers with [`on_lifecycle`] and uses the
//! moment to finish.
//!
//! The registry is per thread, like the runtime itself: callbacks run on the UI
//! thread, inside the event loop's handler, and are not `Send`.

use std::cell::RefCell;
use std::rc::Rc;

/// A change in the application's standing that is worth finishing work for.
///
/// Ordered by urgency: each later variant leaves less time than the one before.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Lifecycle {
    /// The app lost the user's attention but is still running and visible to the
    /// OS: its window lost focus, or its tab was hidden. Cheap work that starts
    /// slow things now is appropriate; blocking is not.
    Inactive,
    /// The app moved to the background. On a mobile OS this is the last
    /// guaranteed warning before the process may be killed, so work that must
    /// not be lost has to be finished before the callback returns.
    Background,
    /// The app is quitting. As with `Background`, finish before returning.
    Exit,
    /// A thread of the app panicked and, in a build that aborts on panic, the
    /// process is about to be torn down the moment the panic hook returns.
    ///
    /// Sent by the hook [`install_panic_hook`](crate::install_panic_hook)
    /// sets up, on the UI thread whichever thread panicked. State may be
    /// inconsistent, and the runtime may be in the middle of anything, including
    /// a widget build: do the minimum that preserves data, do not run tasks, and
    /// never panic (Rust aborts on a panic raised while handling one). Use
    /// `try_borrow` rather than `borrow`.
    Panic,
}

type Callback = Rc<RefCell<dyn FnMut(Lifecycle)>>;

struct Registry {
    next_id: u64,
    callbacks: Vec<(u64, Callback)>,
}

aimer_tls::thread_local! {
    static REGISTRY: RefCell<Registry> = const {
        RefCell::new(Registry { next_id: 0, callbacks: Vec::new() })
    };
}

/// Keeps an [`on_lifecycle`] callback registered; dropping it unregisters.
#[must_use = "the callback is removed as soon as the subscription is dropped"]
pub struct LifecycleSubscription {
    id: u64,
}

impl Drop for LifecycleSubscription {
    fn drop(&mut self) {
        // The thread's registry may already be gone while the thread exits.
        let _ = REGISTRY.try_with(|registry| {
            registry
                .borrow_mut()
                .callbacks
                .retain(|(id, _)| *id != self.id);
        });
    }
}

/// Registers `callback` to be called on every [`notify_lifecycle`] made on this
/// thread, until the returned subscription is dropped.
///
/// The callback may block, and should when the event is [`Lifecycle::Background`]
/// or [`Lifecycle::Exit`] and it has work to finish. It must not register or
/// notify from inside itself.
pub fn on_lifecycle(callback: impl FnMut(Lifecycle) + 'static) -> LifecycleSubscription {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let id = registry.next_id;
        registry.next_id += 1;
        registry.callbacks.push((id, Rc::new(RefCell::new(callback))));
        LifecycleSubscription { id }
    })
}

/// Tells every callback registered on this thread that `event` happened.
///
/// Called by the event loop. Callbacks run in registration order. A callback
/// that is already running, because it caused a notification itself, is skipped
/// rather than re-entered.
pub fn notify_lifecycle(event: Lifecycle) {
    // Copied out first so a callback is free to register or drop subscriptions.
    let callbacks: Vec<Callback> = REGISTRY.with(|registry| {
        registry
            .borrow()
            .callbacks
            .iter()
            .map(|(_, callback)| Rc::clone(callback))
            .collect()
    });
    for callback in callbacks {
        if let Ok(mut callback) = callback.try_borrow_mut() {
            callback(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    #[test]
    fn every_callback_hears_each_event_in_registration_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (first, second) = (log.clone(), log.clone());
        let _a = on_lifecycle(move |event| first.borrow_mut().push(("a", event)));
        let _b = on_lifecycle(move |event| second.borrow_mut().push(("b", event)));

        notify_lifecycle(Lifecycle::Background);
        notify_lifecycle(Lifecycle::Exit);

        assert_eq!(
            *log.borrow(),
            [
                ("a", Lifecycle::Background),
                ("b", Lifecycle::Background),
                ("a", Lifecycle::Exit),
                ("b", Lifecycle::Exit),
            ]
        );
    }

    #[test]
    fn a_dropped_subscription_stays_silent() {
        let calls = Rc::new(Cell::new(0));
        let counted = calls.clone();
        let subscription = on_lifecycle(move |_| counted.set(counted.get() + 1));

        notify_lifecycle(Lifecycle::Inactive);
        drop(subscription);
        notify_lifecycle(Lifecycle::Inactive);

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn notifying_with_nobody_listening_is_harmless() {
        notify_lifecycle(Lifecycle::Exit);
    }

    #[test]
    fn a_callback_may_unregister_itself_and_others_while_running() {
        let calls = Rc::new(Cell::new(0));
        let holder: Rc<RefCell<Option<LifecycleSubscription>>> = Rc::new(RefCell::new(None));

        let (counted, own) = (calls.clone(), holder.clone());
        *holder.borrow_mut() = Some(on_lifecycle(move |_| {
            counted.set(counted.get() + 1);
            own.borrow_mut().take();
        }));

        notify_lifecycle(Lifecycle::Background);
        notify_lifecycle(Lifecycle::Background);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn a_callback_that_notifies_is_not_re_entered() {
        let depth = Rc::new(Cell::new(0));
        let max = Rc::new(Cell::new(0));
        let (d, m) = (depth.clone(), max.clone());
        let _subscription = on_lifecycle(move |event| {
            d.set(d.get() + 1);
            m.set(m.get().max(d.get()));
            if event == Lifecycle::Inactive {
                notify_lifecycle(Lifecycle::Background);
            }
            d.set(d.get() - 1);
        });

        notify_lifecycle(Lifecycle::Inactive);
        assert_eq!(max.get(), 1);
    }

    #[test]
    fn events_are_ordered_by_urgency() {
        assert!(Lifecycle::Inactive < Lifecycle::Background);
        assert!(Lifecycle::Background < Lifecycle::Exit);
        assert!(Lifecycle::Exit < Lifecycle::Panic);
    }
}
