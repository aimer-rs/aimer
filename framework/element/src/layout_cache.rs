use std::any::Any;
use std::cell::{RefCell, UnsafeCell};

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;

use crate::components::element::{element_tree_generation, layout_invalidation_generation};

/// TEMPORARY diagnostic counters for the layout-cache hit rate.
pub mod probe {
    use std::cell::Cell;

    aimer_tls::thread_local! {
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
///
/// An empty slot is a measurement with `valid` unset rather than an `Option`:
/// the flag sits in padding the struct already has, while an `Option` around
/// it would add a whole word to each of the two slots of every element.
#[derive(Clone, Copy)]
struct Measurement {
    constraint: BoxConstraint,
    scale_bits: u32,
    valid: bool,
    tree_generation: u64,
    layout_generation: u64,
    size: ResolvedSize,
}

impl Measurement {
    /// A slot that describes nothing.
    const EMPTY: Self = Self {
        constraint: BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 0.0,
            max_height: 0.0,
        },
        scale_bits: 0,
        valid: false,
        tree_generation: 0,
        layout_generation: 0,
        size: ResolvedSize {
            width: 0.0,
            height: 0.0,
        },
    };

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
        self.valid
            && self.constraint == constraint
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
/// use aimer_element::LayoutCache;
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
    computed: UnsafeCell<Measurement>,
    content: UnsafeCell<Measurement>,
    // Doubly boxed so the slot is a thin pointer: most elements never use it,
    // and a `Box<dyn Any>` would cost them two words for nothing.
    extra: RefCell<Option<Box<Box<dyn Any>>>>,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self {
            computed: UnsafeCell::new(Measurement::EMPTY),
            content: UnsafeCell::new(Measurement::EMPTY),
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
            .is_none_or(|value| !(***value).is::<Option<T>>());
        if replace {
            *extra = Some(Box::new(Box::new(None::<T>)));
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
        *guard = Self::measurement(constraint, scale_bits, size);
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
        *guard = Self::measurement(constraint, scale_bits, size);
    }

    /// Clears the memoized measurement values (call when layout inputs change).
    /// Auxiliary state from [`Self::with_extra`] is retained and owns its own
    /// invalidation policy.
    pub fn invalidate(&self) {
        // Clearing the flag is enough: an invalid slot describes nothing, and
        // the stale fields are overwritten whole by the next `set_*`.
        unsafe {
            (*self.computed.get()).valid = false;
            (*self.content.get()).valid = false;
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
            valid: true,
            tree_generation: element_tree_generation(),
            layout_generation: layout_invalidation_generation(),
            size,
        }
    }

    #[inline]
    fn read(
        measurement: &Measurement,
        constraint: BoxConstraint,
        scale_bits: u32,
    ) -> Option<ResolvedSize> {
        if !measurement.valid {
            probe::miss();
            return None;
        }
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

    /// Every container, text, flex and sized-box element embeds one of these, so
    /// its size is paid once per element and copied with the element. Two
    /// measurements and the auxiliary slot need no more than this.
    #[test]
    fn a_layout_cache_stays_within_its_size_budget() {
        assert!(
            size_of::<LayoutCache>() <= 112,
            "a layout cache grew past its size budget: {} bytes",
            size_of::<LayoutCache>()
        );
    }

    #[test]
    fn invalidation_empties_both_measurements() {
        let cache = LayoutCache::new();
        let constraint = BoxConstraint::new();
        let scale_bits = 1.0_f32.to_bits();
        assert_eq!(cache.get_computed(constraint, scale_bits), None);
        assert_eq!(cache.get_content(constraint, scale_bits), None);

        cache.set_computed(constraint, scale_bits, size(10.0));
        cache.set_content(constraint, scale_bits, size(20.0));
        assert_eq!(cache.get_computed(constraint, scale_bits), Some(size(10.0)));
        assert_eq!(cache.get_content(constraint, scale_bits), Some(size(20.0)));

        cache.invalidate();
        assert_eq!(cache.get_computed(constraint, scale_bits), None);
        assert_eq!(cache.get_content(constraint, scale_bits), None);
    }

    #[test]
    fn a_nested_auxiliary_borrow_still_panics_instead_of_aliasing() {
        let cache = LayoutCache::new();
        let nested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.with_extra(|_: &mut Option<u32>| {
                cache.with_extra(|_: &mut Option<u32>| {});
            });
        }));
        assert!(nested.is_err());
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
