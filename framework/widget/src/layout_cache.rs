use std::any::Any;
use std::cell::{RefCell, UnsafeCell};

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;

use crate::components::element::{element_tree_generation, layout_invalidation_generation};

/// TEMPORARY diagnostic counters for the layout-cache hit rate.
pub mod probe {
    use std::cell::Cell;

    thread_local! {
        pub static HITS: Cell<u64> = const { Cell::new(0) };
        pub static MISSES: Cell<u64> = const { Cell::new(0) };
        pub static SETS: Cell<u64> = const { Cell::new(0) };
    }

    #[inline]
    pub fn hit() {
        HITS.with(|counter| counter.set(counter.get() + 1));
    }

    #[inline]
    pub fn miss() {
        MISSES.with(|counter| counter.set(counter.get() + 1));
    }

    /// One completed (uncached) measurement being stored.
    #[inline]
    pub fn set() {
        SETS.with(|counter| counter.set(counter.get() + 1));
    }

    /// Returns `(hits, misses, sets)` and resets all counters.
    pub fn take() -> (u64, u64, u64) {
        let hits = HITS.with(|counter| counter.replace(0));
        let misses = MISSES.with(|counter| counter.replace(0));
        let sets = SETS.with(|counter| counter.replace(0));
        (hits, misses, sets)
    }
}

/// One memoized measurement together with everything that decides whether it
/// still describes the element.
#[derive(Clone, Copy)]
struct Measurement {
    constraint: BoxConstraint,
    scale_bits: u32,
    tree_generation: u64,
    layout_generation: u64,
    size: ResolvedSize,
}

impl Measurement {
    /// `true` when the measurement was taken under `constraint` and
    /// `scale_bits`, and nothing has replaced a generated subtree since.
    #[inline]
    fn describes(
        &self,
        constraint: BoxConstraint,
        scale_bits: u32,
        tree_generation: u64,
        layout_generation: u64,
    ) -> bool {
        self.constraint == constraint
            && self.scale_bits == scale_bits
            && self.tree_generation == tree_generation
            && self.layout_generation == layout_generation
    }
}

/// Caches the result of `computed_size` and `content_size` between frames.
///
/// The cache is keyed by `(BoxConstraint, scale)` so that if the same element
/// is queried multiple times with the same inputs, the result is returned
/// instantly. yeah it saves the CPU and GPU and reduces power consuming :))
///
/// The key also carries the [element-tree
/// generation](crate::element_tree_generation) and layout-invalidation
/// generation the measurement was taken in. An element measures its children,
/// so a measurement only describes the element for as long as the subtree below
/// it is the one that was measured. Replacing a generated subtree — a
/// `setState`, or an
/// [`AsyncBuilder`](crate::AsyncBuilder) swapping its loading state for the data
/// it waited on — advances that generation, which retires every measurement
/// taken before it. Without this an ancestor that itself never rebuilt, such as
/// the `Container` between a `Scrollable` and its content, would keep handing
/// out the height the content had while it was still loading, and the scroll
/// view would decide there is nothing to scroll.
///
/// The tree generation only moves when the element tree changes shape, while
/// the layout generation moves when a caller explicitly invalidates layout. A
/// frame that merely animates or scrolls still reads straight from the cache.
///
/// # Examples
///
/// ```
/// use aimer_attribute::BoxConstraint;
/// use aimer_attribute::size::ResolvedSize;
/// use aimer_widget::LayoutCache;
///
/// let cache = LayoutCache::new();
/// let constraint = BoxConstraint::new();
/// let scale_bits = 1.0_f32.to_bits();
/// let size = ResolvedSize { width: 100.0, height: 40.0 };
///
/// cache.set_computed(constraint, scale_bits, size);
///
/// assert_eq!(cache.get_computed(constraint, scale_bits), Some(size));
/// ```
pub struct LayoutCache {
    computed: UnsafeCell<Option<Measurement>>,
    content: UnsafeCell<Option<Measurement>>,
    extra: RefCell<Option<Box<dyn Any>>>,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self {
            computed: UnsafeCell::new(None),
            content: UnsafeCell::new(None),
            extra: RefCell::new(None),
        }
    }

    /// Runs `callback` with one auxiliary cache slot owned by this layout
    /// cache.
    ///
    /// The slot is useful for a layout element that has an expensive retained
    /// helper whose type belongs to another crate. It is deliberately separate
    /// from the measurement entries: invalidating a measurement does not drop
    /// the helper, while the helper can invalidate its own derived state. One
    /// cache owns at most one auxiliary type; changing the requested type
    /// replaces the previous value.
    pub fn with_extra<T: 'static, R>(
        &self,
        callback: impl FnOnce(&mut Option<T>) -> R,
    ) -> R {
        let mut extra = self.extra.borrow_mut();
        let replace = extra
            .as_ref()
            .is_none_or(|value| !value.is::<Option<T>>());
        if replace {
            *extra = Some(Box::new(None::<T>));
        }
        let slot = extra
            .as_mut()
            .and_then(|value| value.downcast_mut::<Option<T>>())
            .expect("the auxiliary layout-cache slot has the requested type");
        callback(slot)
    }

    /// Returns cached computed_size if constraint, scale, element tree, and
    /// layout invalidation generation are unchanged, otherwise None.
    pub fn get_computed(&self, constraint: BoxConstraint, scale_bits: u32) -> Option<ResolvedSize> {
        let guard = unsafe { &*self.computed.get() };
        Self::read(guard, constraint, scale_bits)
    }

    /// Stores computed_size result.
    pub fn set_computed(&self, constraint: BoxConstraint, scale_bits: u32, size: ResolvedSize) {
        probe::set();
        let guard = unsafe { &mut *self.computed.get() };
        *guard = Some(Self::measurement(constraint, scale_bits, size));
    }

    /// Returns cached content_size if constraint, scale, element tree, and
    /// layout invalidation generation are unchanged, otherwise None.
    pub fn get_content(&self, constraint: BoxConstraint, scale_bits: u32) -> Option<ResolvedSize> {
        let guard = unsafe { &*self.content.get() };
        Self::read(guard, constraint, scale_bits)
    }

    /// Stores content_size result.
    pub fn set_content(&self, constraint: BoxConstraint, scale_bits: u32, size: ResolvedSize) {
        probe::set();
        let guard = unsafe { &mut *self.content.get() };
        *guard = Some(Self::measurement(constraint, scale_bits, size));
    }

    /// Clears the memoized measurement values (call when layout inputs change).
    /// Auxiliary state from [`Self::with_extra`] is retained and owns its own
    /// invalidation policy.
    pub fn invalidate(&self) {
        unsafe {
            *self.computed.get() = None;
            *self.content.get() = None;
        }
    }

    #[inline]
    fn measurement(
        constraint: BoxConstraint,
        scale_bits: u32,
        size: ResolvedSize,
    ) -> Measurement {
        Measurement {
            constraint,
            scale_bits,
            tree_generation: element_tree_generation(),
            layout_generation: layout_invalidation_generation(),
            size,
        }
    }

    #[inline]
    fn read(
        slot: &Option<Measurement>,
        constraint: BoxConstraint,
        scale_bits: u32,
    ) -> Option<ResolvedSize> {
        let Some(measurement) = slot.as_ref() else {
            probe::miss();
            return None;
        };
        let result = measurement
            .describes(
                constraint,
                scale_bits,
                element_tree_generation(),
                layout_invalidation_generation(),
            )
            .then_some(measurement.size);
        if result.is_some() {
            probe::hit();
        } else {
            probe::miss();
        }
        result
    }
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::BuildContext;
    use crate::components::element::reconcile_generated_tree;
    use crate::{Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement};

    struct Leaf;

    impl VisitorElement for Leaf {
        fn debug_name(&self) -> &'static str {
            "Leaf"
        }
    }

    impl Drawable for Leaf {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for Leaf {}
    impl LayoutElement for Leaf {}
    impl Rebuildable for Leaf {}

    fn size(height: f32) -> ResolvedSize {
        ResolvedSize {
            width: 100.0,
            height,
        }
    }

    #[test]
    fn a_measurement_taken_under_other_inputs_is_not_reused() {
        let cache = LayoutCache::new();
        let constraint = BoxConstraint::new();
        let wider = BoxConstraint {
            max_width: 200.0,
            ..constraint
        };
        cache.set_computed(constraint, 1.0_f32.to_bits(), size(40.0));

        assert_eq!(cache.get_computed(wider, 1.0_f32.to_bits()), None);
        assert_eq!(cache.get_computed(constraint, 2.0_f32.to_bits()), None);
    }

    #[test]
    fn an_auxiliary_value_survives_measurement_invalidation() {
        let cache = LayoutCache::new();
        cache.with_extra(|slot: &mut Option<u32>| *slot = Some(42));
        cache.invalidate();

        assert_eq!(cache.with_extra(|slot: &mut Option<u32>| *slot), Some(42));
    }

    #[test]
    fn replacing_a_generated_subtree_retires_a_cached_measurement() {
        let cache = LayoutCache::new();
        let constraint = BoxConstraint::new();
        let scale_bits = 1.0_f32.to_bits();
        cache.set_computed(constraint, scale_bits, size(40.0));
        cache.set_content(constraint, scale_bits, size(40.0));

        // What an `AsyncBuilder` does when the future it waited on completes.
        reconcile_generated_tree(&Leaf, &Leaf);

        assert_eq!(cache.get_computed(constraint, scale_bits), None);
        assert_eq!(cache.get_content(constraint, scale_bits), None);
    }

    #[test]
    fn layout_invalidation_retires_a_cached_measurement() {
        let cache = LayoutCache::new();
        let constraint = BoxConstraint::new();
        let scale_bits = 1.0_f32.to_bits();
        cache.set_computed(constraint, scale_bits, size(40.0));
        assert_eq!(cache.get_computed(constraint, scale_bits), Some(size(40.0)));

        crate::components::element::advance_layout_invalidation_generation();

        assert_eq!(cache.get_computed(constraint, scale_bits), None);
    }
}
