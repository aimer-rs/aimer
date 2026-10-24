use std::cell::Cell;

thread_local! {
    static DEFERRED_HOVER_RECONCILIATION_DEPTH: Cell<usize> = const { Cell::new(0) };
    static HOVER_RECONCILIATION_PENDING: Cell<bool> = const { Cell::new(false) };
    static POINTER_CAPTURE_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Records whether some element holds a pointer capture, as of the last event
/// the application delivered.
///
/// The application keeps this current after every dispatch; widgets read it
/// through [`pointer_capture_active`].
#[doc(hidden)]
#[inline]
pub fn set_pointer_capture_active(active: bool) {
    POINTER_CAPTURE_ACTIVE.with(|flag| flag.set(active));
}

/// Reports whether a drag or press currently owns the pointer.
///
/// While it does, the pointer belongs to the capturing element, so a region
/// that merely finds the cursor over it while painting must not start hovering:
/// nothing would end that hover, because the moves that leave it go to the
/// capturing element and not to the region.
#[doc(hidden)]
#[inline]
pub fn pointer_capture_active() -> bool {
    POINTER_CAPTURE_ACTIVE.with(Cell::get)
}

/// Reports whether a scrolling container is collecting hover work for one
/// reconciliation pass after its content has moved.
#[doc(hidden)]
#[inline]
pub fn mouse_region_hover_reconciliation_deferred() -> bool {
    DEFERRED_HOVER_RECONCILIATION_DEPTH.with(|depth| depth.get() != 0)
}

/// Requests one hover reconciliation at the end of the outermost deferred
/// scope. Multiple nested scrollables share that single request.
#[doc(hidden)]
#[inline]
pub fn request_mouse_region_hover_reconciliation() {
    HOVER_RECONCILIATION_PENDING.with(|pending| pending.set(true));
}

/// Defers per-region hover checks until a scrolling container can reconcile
/// the current pointer path once against the moved content.
///
/// The returned flag is true only for the outermost scope, and only if any
/// nested scope requested reconciliation. The scope restores its thread-local
/// state if `callback` unwinds.
#[doc(hidden)]
pub fn with_deferred_mouse_region_hover_reconciliation<R>(
    callback: impl FnOnce() -> R,
) -> (R, bool) {
    let mut scope = DeferredHoverScope::enter();
    let result = callback();
    let needs_reconciliation = scope.finish();
    (result, needs_reconciliation)
}

struct DeferredHoverScope {
    outermost: bool,
    active: bool,
}

impl DeferredHoverScope {
    fn enter() -> Self {
        let outermost = DEFERRED_HOVER_RECONCILIATION_DEPTH.with(|depth| {
            let previous = depth.get();
            depth.set(previous + 1);
            previous == 0
        });
        if outermost {
            HOVER_RECONCILIATION_PENDING.with(|pending| pending.set(false));
        }
        Self {
            outermost,
            active: true,
        }
    }

    fn finish(&mut self) -> bool {
        let needs_reconciliation = self.outermost
            && HOVER_RECONCILIATION_PENDING.with(|pending| pending.replace(false));
        self.leave();
        needs_reconciliation
    }

    fn leave(&mut self) {
        if !self.active {
            return;
        }
        DEFERRED_HOVER_RECONCILIATION_DEPTH.with(|depth| {
            depth.set(depth.get().saturating_sub(1));
        });
        if self.outermost {
            HOVER_RECONCILIATION_PENDING.with(|pending| pending.set(false));
        }
        self.active = false;
    }
}

impl Drop for DeferredHoverScope {
    fn drop(&mut self) {
        self.leave();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_scroll_scopes_request_only_one_hover_reconciliation() {
        let ((), should_reconcile) = with_deferred_mouse_region_hover_reconciliation(|| {
            assert!(mouse_region_hover_reconciliation_deferred());
            let ((), nested_reconciliation) = with_deferred_mouse_region_hover_reconciliation(|| {
                request_mouse_region_hover_reconciliation();
            });
            assert!(!nested_reconciliation);
        });

        assert!(should_reconcile);
        assert!(!mouse_region_hover_reconciliation_deferred());
    }

    #[test]
    fn a_deferred_scope_without_scrolling_does_not_reconcile() {
        let ((), should_reconcile) =
            with_deferred_mouse_region_hover_reconciliation(|| {});

        assert!(!should_reconcile);
        assert!(!mouse_region_hover_reconciliation_deferred());
    }
}
