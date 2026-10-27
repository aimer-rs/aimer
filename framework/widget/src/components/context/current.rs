//! Global access to the context of the build in progress.
//!
//! A `build` is handed its [`BuildContext`] as an argument, but code it calls
//! into — a helper, an extension method on a future — has no argument to carry
//! it. Every build runs inside [`BuildContext::with_build_consumer`], which
//! registers its context here for exactly the duration of the build, so that
//! code can ask for it instead of threading it through.
//!
//! The registry is a per-thread stack. Builds nest (a build places children
//! whose own builds run inside it), so the innermost build is the current one
//! and leaving it makes the enclosing one current again.

use std::cell::RefCell;
use std::fmt;

use super::BuildContext;

/// One build in progress.
#[derive(Clone, Copy)]
struct Entry {
    /// Distinguishes this build from every later one that reuses its stack slot.
    id: u64,
    /// The context of the build, with its canvas lifetime erased.
    ///
    /// Only dereferenced while the entry is on the stack: [`BuildScope`] pops
    /// it before the borrow it was taken from can end.
    context: *const BuildContext<'static>,
}

struct Scopes {
    next_id: u64,
    stack: Vec<Entry>,
}

aimer_tls::thread_local! {
    static SCOPES: RefCell<Scopes> = const {
        RefCell::new(Scopes { next_id: 0, stack: Vec::new() })
    };
}

/// Keeps a build's context registered for as long as the build runs.
///
/// Dropping it, including by unwinding, unregisters the build and everything
/// still registered above it.
pub(super) struct BuildScope {
    depth: usize,
}

impl BuildScope {
    #[inline]
    pub(super) fn enter(context: &BuildContext<'_>) -> Self {
        let context = context as *const BuildContext<'_> as *const BuildContext<'static>;
        SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            let id = scopes.next_id;
            scopes.next_id = id.wrapping_add(1);
            let depth = scopes.stack.len();
            scopes.stack.push(Entry { id, context });
            Self { depth }
        })
    }
}

impl Drop for BuildScope {
    #[inline]
    fn drop(&mut self) {
        // The thread may already be tearing its locals down; nothing is left to
        // unregister then.
        let _ = SCOPES.try_with(|scopes| scopes.borrow_mut().stack.truncate(self.depth));
    }
}

/// A handle to the [`BuildContext`] of the build running on this thread.
///
/// Obtained from [`BuildContext::current`] or [`BuildContext::try_current`].
/// It is neither `Send` nor `Sync`, and it does not keep the build alive: the
/// context it refers to belongs to the build, which borrows the frame canvas
/// and ends with it.
///
/// Reaching the context goes through [`ActiveBuildContext::get`], which checks
/// that the build is still running and panics if it is not. A handle kept past
/// the end of its build therefore fails loudly instead of touching a canvas
/// that is gone. There is deliberately no `Deref`: that would let a caller
/// clone the context into a type with no lifetime bound, which could outlive
/// the build undetected.
///
/// # Examples
///
/// ```ignore
/// fn build(&self, ctx: &BuildContext) -> impl Widget {
///     let scale = BuildContext::current().get().scale;
///     // ...
/// }
/// ```
pub struct ActiveBuildContext {
    entry: Entry,
    depth: usize,
}

impl ActiveBuildContext {
    /// Returns whether the build this handle was taken in is still running.
    ///
    /// A build that has ended, even if another build now occupies its place,
    /// is not running.
    #[inline]
    pub fn is_live(&self) -> bool {
        SCOPES.with(|scopes| {
            scopes
                .borrow()
                .stack
                .get(self.depth)
                .is_some_and(|entry| entry.id == self.entry.id)
        })
    }

    /// Borrows the context of the build.
    ///
    /// The borrow lasts no longer than this handle, and a clone taken from it
    /// is bound to that borrow. A clone must still not be kept after the build
    /// returns: the canvas it refers to is only valid for the frame being built.
    ///
    /// # Panics
    ///
    /// Panics if the build that created this handle has ended.
    #[inline]
    #[track_caller]
    pub fn get(&self) -> &BuildContext<'_> {
        assert!(
            self.is_live(),
            "the build that created it has ended; an ActiveBuildContext must not outlive its build"
        );
        // SAFETY: the entry is still on the stack (checked above), so the
        // build's `&BuildContext` is still borrowed by `with_build_consumer`,
        // and the pointee outlives this call. Shrinking the erased `'static`
        // to the handle's borrow only narrows it.
        unsafe { &*self.entry.context }
    }
}

/// No build is running on this thread, so there is no current context.
///
/// Returned by [`BuildContext::try_current`] when called from an event
/// handler, a task, layout, painting, or any other code outside a `build`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NoActiveBuildContext;

impl fmt::Display for NoActiveBuildContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no widget build is running on this thread")
    }
}

impl std::error::Error for NoActiveBuildContext {}

impl BuildContext<'_> {
    /// Returns the context of the build running on this thread.
    ///
    /// Use it where threading `ctx` through is impractical. Inside nested
    /// builds this is the innermost one.
    ///
    /// # Panics
    ///
    /// Panics if no build is running. Code that may run outside one should
    /// call [`BuildContext::try_current`].
    #[inline]
    #[track_caller]
    pub fn current() -> ActiveBuildContext {
        match Self::try_current() {
            Ok(current) => current,
            Err(error) => panic!("{error}"),
        }
    }

    /// Returns the context of the build running on this thread, or an error if
    /// no build is running.
    #[inline]
    pub fn try_current() -> Result<ActiveBuildContext, NoActiveBuildContext> {
        SCOPES.with(|scopes| {
            let scopes = scopes.borrow();
            let depth = scopes.stack.len().checked_sub(1).ok_or(NoActiveBuildContext)?;
            Ok(ActiveBuildContext {
                entry: scopes.stack[depth],
                depth,
            })
        })
    }

    /// Returns the context of the build running on this thread without
    /// checking that there is one or that it stays valid.
    ///
    /// This is the zero-cost form of [`BuildContext::current`] for code that
    /// has already established it is inside a build.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that:
    ///
    /// - a build is running on this thread now, and
    /// - that build outlives `'a`, so the returned reference, and anything
    ///   cloned from it, is not used after the build returns.
    #[inline]
    pub unsafe fn unchecked_current<'a>() -> &'a BuildContext<'a> {
        let entry = SCOPES.with(|scopes| scopes.borrow().stack.last().copied());
        debug_assert!(entry.is_some(), "unchecked_current called outside a build");
        // SAFETY: the caller guarantees a build is running, so the stack is
        // non-empty, and that it outlives `'a`, so the pointer is valid for it.
        unsafe { &*entry.unwrap_unchecked().context }
    }
}
