//! The system's "go back" key, as the windowed loop sees it.
//!
//! Android delivers Back (and the back gesture, when predictive back is off) as
//! a `BrowserBack` key. winit reports every Android key as handled, so the
//! system never leaves the app on its own: an application that does not answer
//! Back has a Back key that does nothing, forever. The request is therefore
//! offered to the registered back handlers (see [`aimer_events::back`]) and, if
//! none takes it, the platform's default is applied here.

use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

/// What the key event means for back handling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackKey {
    /// Not a back key; carry on as for any key.
    NotBack,
    /// A back key going down: ask the handlers.
    Request,
    /// A back key's release, or its auto-repeat. It is still the back key, so it
    /// must not reach the widget tree as a key named "BrowserBack", but it asks
    /// for nothing: a held key must not pop a page per repeat.
    Swallow,
}

pub fn classify(key: &Key, state: ElementState, repeat: bool) -> BackKey {
    if !matches!(key, Key::Named(NamedKey::BrowserBack)) {
        return BackKey::NotBack;
    }
    if state == ElementState::Pressed && !repeat { BackKey::Request } else { BackKey::Swallow }
}

/// Whether a back request nothing answered leaves the app.
///
/// Android's convention is that Back at the root of the app finishes it. A
/// desktop window is closed with its own controls, so there it does nothing.
pub const UNHANDLED_BACK_EXITS: bool = cfg!(target_os = "android");

/// One step of a back the system reports in pieces, as Android's predictive back
/// does: it starts, may progress, and then is invoked or cancelled. A plain back
/// is just [`Invoked`](Self::Invoked).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackSignal {
    /// The user started the back gesture.
    Started,
    /// How far the gesture has gone, from `0.0` to `1.0`.
    Progressed(f32),
    /// The gesture completed, or a plain back was pressed.
    Invoked,
    /// The user let go without completing the gesture.
    Cancelled,
}

/// Whether the app took a back signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackOutcome {
    Handled,
    /// Nothing answered; the platform's default applies (see
    /// [`UNHANDLED_BACK_EXITS`]).
    Unhandled,
}

/// Carries a gesture-driven back from its start to its end.
#[derive(Default)]
struct BackDriver {
    gesture: Option<Box<dyn aimer_events::back::BackGesture>>,
}

impl BackDriver {
    fn handle(&mut self, signal: BackSignal) -> BackOutcome {
        use aimer_events::back::{request_back, start_back_gesture};
        match signal {
            BackSignal::Started => {
                // A gesture that never ended (the system dropped its cancel) must
                // not be left holding a page open.
                if let Some(stale) = self.gesture.take() {
                    stale.cancel();
                }
                self.gesture = start_back_gesture();
                BackOutcome::Handled
            }
            BackSignal::Progressed(progress) => {
                if let Some(gesture) = &self.gesture {
                    gesture.progress(progress);
                }
                BackOutcome::Handled
            }
            BackSignal::Invoked => match self.gesture.take() {
                Some(gesture) => {
                    gesture.commit();
                    BackOutcome::Handled
                }
                None if request_back() => BackOutcome::Handled,
                None => BackOutcome::Unhandled,
            },
            BackSignal::Cancelled => {
                if let Some(gesture) = self.gesture.take() {
                    gesture.cancel();
                }
                BackOutcome::Handled
            }
        }
    }
}

aimer_tls::thread_local! {
    static DRIVER: std::cell::RefCell<BackDriver> = std::cell::RefCell::new(BackDriver::default());
}

/// Applies a back signal from the platform bridge on the UI thread.
pub fn handle_signal(signal: BackSignal) -> BackOutcome {
    DRIVER.with(|driver| driver.borrow_mut().handle(signal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_back_key_going_down_is_a_request() {
        assert_eq!(classify(&Key::Named(NamedKey::BrowserBack), ElementState::Pressed, false), BackKey::Request);
    }

    #[test]
    fn its_release_and_its_repeats_are_swallowed_not_requests() {
        let back = Key::Named(NamedKey::BrowserBack);
        assert_eq!(classify(&back, ElementState::Released, false), BackKey::Swallow);
        assert_eq!(classify(&back, ElementState::Pressed, true), BackKey::Swallow);
    }

    #[test]
    fn other_keys_are_not_back() {
        for key in [Key::Named(NamedKey::Escape), Key::Named(NamedKey::BrowserForward), Key::Character("b".into())] {
            assert_eq!(classify(&key, ElementState::Pressed, false), BackKey::NotBack, "{key:?}");
        }
    }

    mod driver {
        use std::cell::RefCell;
        use std::rc::Rc;

        use aimer_events::back::{BackGesture, BackHandler, register};

        use super::super::*;

        type Log = Rc<RefCell<Vec<String>>>;

        struct Page {
            log: Log,
            gesture: bool,
            handles: bool,
        }
        impl BackHandler for Page {
            fn on_back(&self) -> bool {
                self.log.borrow_mut().push("back".into());
                self.handles
            }
            fn on_back_started(&self) -> Option<Box<dyn BackGesture>> {
                self.gesture.then(|| Box::new(Gesture(self.log.clone())) as Box<dyn BackGesture>)
            }
        }
        struct Gesture(Log);
        impl BackGesture for Gesture {
            fn progress(&self, progress: f32) { self.0.borrow_mut().push(format!("progress {progress}")); }
            fn commit(self: Box<Self>) { self.0.borrow_mut().push("commit".into()); }
            fn cancel(self: Box<Self>) { self.0.borrow_mut().push("cancel".into()); }
        }

        fn page(gesture: bool, handles: bool) -> (Log, aimer_events::back::BackGuard) {
            let log = Log::default();
            let guard = register(Page { log: log.clone(), gesture, handles });
            (log, guard)
        }

        #[test]
        fn a_gesture_runs_from_start_through_progress_to_commit() {
            let (log, _guard) = page(true, true);
            let mut driver = BackDriver::default();
            assert_eq!(driver.handle(BackSignal::Started), BackOutcome::Handled);
            driver.handle(BackSignal::Progressed(0.25));
            driver.handle(BackSignal::Progressed(0.75));
            assert_eq!(driver.handle(BackSignal::Invoked), BackOutcome::Handled);
            assert_eq!(*log.borrow(), ["progress 0.25", "progress 0.75", "commit"]);
        }

        #[test]
        fn a_cancelled_gesture_cancels_and_does_not_go_back() {
            let (log, _guard) = page(true, true);
            let mut driver = BackDriver::default();
            driver.handle(BackSignal::Started);
            driver.handle(BackSignal::Progressed(0.4));
            driver.handle(BackSignal::Cancelled);
            assert_eq!(*log.borrow(), ["progress 0.4", "cancel"]);
            // And a later plain back is a fresh request, not the dead gesture's commit.
            assert_eq!(driver.handle(BackSignal::Invoked), BackOutcome::Handled);
            assert_eq!(log.borrow().last().map(String::as_str), Some("back"));
        }

        #[test]
        fn a_plain_back_with_no_gesture_asks_the_handlers() {
            let (log, _guard) = page(false, true);
            let mut driver = BackDriver::default();
            assert_eq!(driver.handle(BackSignal::Invoked), BackOutcome::Handled);
            assert_eq!(*log.borrow(), ["back"]);
        }

        #[test]
        fn a_back_nobody_answers_is_left_to_the_platform() {
            let (_log, _guard) = page(false, false);
            let mut driver = BackDriver::default();
            assert_eq!(driver.handle(BackSignal::Invoked), BackOutcome::Unhandled);
            // Progress and cancel with no gesture are harmless.
            assert_eq!(driver.handle(BackSignal::Progressed(0.5)), BackOutcome::Handled);
            assert_eq!(driver.handle(BackSignal::Cancelled), BackOutcome::Handled);
        }

        #[test]
        fn a_gesture_whose_end_never_came_is_cancelled_when_the_next_one_starts() {
            let (log, _guard) = page(true, true);
            let mut driver = BackDriver::default();
            driver.handle(BackSignal::Started);
            driver.handle(BackSignal::Started);
            assert_eq!(*log.borrow(), ["cancel"], "the stale gesture is released, the new one is live");
            driver.handle(BackSignal::Invoked);
            assert_eq!(log.borrow().last().map(String::as_str), Some("commit"));
        }
    }
}
