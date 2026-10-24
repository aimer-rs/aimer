#[cfg(panic = "unwind")]
use std::any::Any;
use std::cell::RefCell;
#[cfg(panic = "unwind")]
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(panic = "unwind")]
use std::sync::OnceLock;

use crate::GuestSourceLocation;

thread_local! {
    static PANIC_CONTEXTS: RefCell<Vec<GuestPanicContext>> = const { RefCell::new(Vec::new()) };
    static WATCHED_LOCATIONS: RefCell<Vec<Option<GuestSourceLocation>>> =
        const { RefCell::new(Vec::new()) };
    static WATCHED_CONTEXTS: RefCell<Vec<Option<GuestPanicContext>>> =
        const { RefCell::new(Vec::new()) };
}

/// Identifies the generated guest widget operation currently being built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestPanicContext {
    widget: &'static str,
    phase: &'static str,
}

impl GuestPanicContext {
    /// Creates a context used to label a guest panic.
    #[inline]
    pub const fn new(widget: &'static str, phase: &'static str) -> Self {
        Self { widget, phase }
    }

    /// Returns the generated widget name.
    #[inline]
    pub const fn widget(self) -> &'static str {
        self.widget
    }

    /// Returns the guest build phase.
    #[inline]
    pub const fn phase(self) -> &'static str {
        self.phase
    }
}

/// Guard that labels panics raised while one generated guest widget is built.
#[derive(Debug)]
pub struct GuestPanicScope {
    _private: (),
}

impl GuestPanicScope {
    /// Enters a widget and phase context for the current thread.
    #[inline]
    pub fn new(widget: &'static str, phase: &'static str) -> Self {
        PANIC_CONTEXTS.with(|contexts| {
            contexts
                .borrow_mut()
                .push(GuestPanicContext::new(widget, phase));
        });
        Self { _private: () }
    }
}

impl Drop for GuestPanicScope {
    #[inline]
    fn drop(&mut self) {
        PANIC_CONTEXTS.with(|contexts| {
            let _ = contexts.borrow_mut().pop();
        });
    }
}

/// The sanitized information recovered from one guest panic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestPanicRecord {
    payload: String,
    location: Option<GuestSourceLocation>,
    context: Option<GuestPanicContext>,
}

impl GuestPanicRecord {
    /// Returns the panic payload before the ABI diagnostic limit is applied.
    #[inline]
    pub fn payload(&self) -> &str {
        &self.payload
    }

    /// Returns the compiler-reported guest source coordinate, when available.
    #[inline]
    pub fn location(&self) -> Option<&GuestSourceLocation> {
        self.location.as_ref()
    }

    /// Returns the generated widget context, when one was active.
    #[inline]
    pub const fn context(&self) -> Option<GuestPanicContext> {
        self.context
    }
}

/// Runs guest code behind a recoverable panic boundary and records its source.
///
/// On an unwind-capable target this catches the panic before it reaches the
/// guest export. A non-unwinding target cannot recover a panic, so on such a
/// target this function simply invokes the operation and lets the target's
/// panic policy decide its outcome.
pub fn capture_guest_panic<T>(operation: impl FnOnce() -> T) -> Result<T, GuestPanicRecord> {
    #[cfg(panic = "unwind")]
    {
        install_hook();
        WATCHED_LOCATIONS.with(|locations| locations.borrow_mut().push(None));
        WATCHED_CONTEXTS.with(|contexts| contexts.borrow_mut().push(None));
        let result = catch_unwind(AssertUnwindSafe(operation));
        let location = WATCHED_LOCATIONS.with(|locations| locations.borrow_mut().pop().flatten());
        let context = WATCHED_CONTEXTS.with(|contexts| contexts.borrow_mut().pop().flatten());
        return result.map_err(|payload| GuestPanicRecord {
            payload: panic_payload(payload),
            location,
            context,
        });
    }

    #[cfg(not(panic = "unwind"))]
    {
        Ok(operation())
    }
}

#[cfg(panic = "unwind")]
fn current_context() -> Option<GuestPanicContext> {
    PANIC_CONTEXTS
        .try_with(|contexts| contexts.borrow().last().copied())
        .ok()
        .flatten()
}

#[cfg(panic = "unwind")]
fn panic_payload(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(panic = "unwind")]
fn watching() -> bool {
    WATCHED_LOCATIONS
        .try_with(|locations| !locations.borrow().is_empty())
        .unwrap_or(false)
}

#[cfg(panic = "unwind")]
fn install_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !watching() {
                previous(info);
                return;
            }
            if let Some(location) = info.location() {
                let location =
                    GuestSourceLocation::new(location.file(), location.line(), location.column());
                let _ = WATCHED_LOCATIONS.try_with(|locations| {
                    if let Some(current) = locations.borrow_mut().last_mut() {
                        *current = Some(location);
                    }
                });
            }
            let context = current_context();
            let _ = WATCHED_CONTEXTS.try_with(|contexts| {
                if let Some(current) = contexts.borrow_mut().last_mut() {
                    *current = context;
                }
            });
        }));
    });
}

#[cfg(all(test, panic = "unwind"))]
mod tests {
    use super::*;

    #[test]
    fn capture_records_the_guest_context_and_source_location() {
        let expected_line = line!() + 3;
        let panic = capture_guest_panic(|| {
            let _scope = GuestPanicScope::new("HttpRequestButton", "build");
            panic!("boom");
        })
        .expect_err("panic should recover");

        assert_eq!(panic.payload(), "boom");
        assert_eq!(
            panic.context(),
            Some(GuestPanicContext::new("HttpRequestButton", "build"))
        );
        let location = panic
            .location()
            .expect("panic should have a source location");
        assert_eq!(location.file(), file!());
        assert_eq!(location.line(), expected_line);
    }

    #[test]
    fn capture_does_not_leak_a_previous_location() {
        let first = capture_guest_panic(|| panic!("first")).expect_err("panic should recover");
        assert!(first.location().is_some());

        let _scope = GuestPanicScope::new("Widget", "build");
        let second = capture_guest_panic(|| panic!("second")).expect_err("panic should recover");
        assert_eq!(second.payload(), "second");
        assert_eq!(second.context().unwrap().widget(), "Widget");
    }
}
