//! The experiments this laboratory exists for.
//!
//! Two claims are under test, and both are about cost rather than behaviour:
//!
//! 1. **A build copies nothing.** A decorated node owns a `Vec` of shadows.
//!    With a borrowing conversion that vector is cloned into the element and
//!    the original is thrown away one line later; with a consuming conversion
//!    it is moved, so the buffer the widget allocated is the buffer the
//!    element keeps.
//! 2. **A build allocates nothing.** Once the pool is warm, turning a widget
//!    tree into an element tree must not reach the system allocator at all.
//!    The test below counts allocations through a recording global allocator
//!    and asserts a hard zero.
//!
//! A third property falls out of the design rather than being measured: a
//! composing widget is *stored* in its element instead of being cloned into a
//! rebuild closure, so nothing in the tree needs a `Clone` bound. `Composable`
//! deliberately has none.

use crate::element::{AnyElement, Element};
use crate::widget::Widget;
use crate::widget::erased::AnyWidget;
use crate::widget::retained::RetainedChild;

/// One entry of a decoration's shadow list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shadow {
    /// Blur radius, in logical pixels.
    pub blur: f32,
    /// Spread radius, in logical pixels.
    pub spread: f32,
}

/// A node that owns heap data and one child, modelling `Container`.
pub struct Decorated {
    shadows: Vec<Shadow>,
    child: AnyWidget,
}

impl Decorated {
    /// Creates a decorated node around `child`.
    #[inline]
    pub fn new(shadows: Vec<Shadow>, child: AnyWidget) -> Self {
        Self { shadows, child }
    }
}

/// The element built from [`Decorated`], holding the widget's own vector.
///
/// The shadow list is never read here — a real element would paint it — but it
/// is the value whose ownership the experiment tracks, so it has to live for
/// as long as the element does.
struct DecoratedElement {
    _shadows: Vec<Shadow>,
    child: AnyElement,
}

impl Element for DecoratedElement {
    fn debug_name(&self) -> &'static str {
        "DecoratedElement"
    }

    fn rebuild(&mut self) {
        self.child.rebuild();
    }
}

impl Widget for Decorated {
    #[inline]
    fn to_element(self) -> AnyElement {
        let child = self.child.into_element();
        AnyElement::new(DecoratedElement {
            _shadows: self.shadows,
            child,
        })
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        "Decorated"
    }
}

/// A leaf node, used to terminate an experiment's tree.
pub struct Leaf;

struct LeafElement;

impl Element for LeafElement {
    fn debug_name(&self) -> &'static str {
        "LeafElement"
    }
}

impl Widget for Leaf {
    #[inline]
    fn to_element(self) -> AnyElement {
        AnyElement::new(LeafElement)
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        "Leaf"
    }
}

/// A node that describes its subtree instead of rendering it, modelling a
/// stateless widget.
///
/// Note the absence of a `Clone` bound: the element keeps the composable
/// itself, so a rebuild re-runs the original value.
pub trait Composable: 'static {
    /// Describes this node's subtree.
    fn build(&self) -> AnyWidget;
}

/// Adapts a [`Composable`] into a [`Widget`].
pub struct Composed<C: Composable> {
    composable: C,
}

impl<C: Composable> Composed<C> {
    /// Wraps `composable` so it can take part in a widget tree.
    #[inline]
    pub fn new(composable: C) -> Self {
        Self { composable }
    }
}

/// The element built from [`Composed`], which owns the composable and can
/// therefore rebuild without any copy of it.
struct ComposedElement<C: Composable> {
    composable: C,
    child: AnyElement,
}

impl<C: Composable> Element for ComposedElement<C> {
    fn debug_name(&self) -> &'static str {
        "ComposedElement"
    }

    fn rebuild(&mut self) {
        self.child = self.composable.build().into_element();
    }
}

impl<C: Composable> Widget for Composed<C> {
    #[inline]
    fn to_element(self) -> AnyElement {
        let child = self.composable.build().into_element();
        AnyElement::new(ComposedElement {
            composable: self.composable,
            child,
        })
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        "Composed"
    }
}

/// A node that rebuilds *itself*, modelling `Button`, `Scrollable`, and every
/// other widget whose own state changes without its parent noticing.
///
/// This is the case a consuming conversion cannot serve with a plain child
/// value: the child widget was consumed by the first build, so the second one
/// has nothing left to convert. The child therefore arrives as a
/// [`RetainedChild`], and every rebuild places the element that first build
/// produced instead of producing another one.
pub struct SelfRebuilding {
    child: RetainedChild,
}

impl SelfRebuilding {
    /// Creates a self-rebuilding node around a retained child.
    #[inline]
    pub fn new(child: RetainedChild) -> Self {
        Self { child }
    }
}

/// The element built from [`SelfRebuilding`].
///
/// It keeps the slot rather than the child element directly, because a rebuild
/// re-runs the node's own description and therefore asks the slot for the
/// child's position again.
struct SelfRebuildingElement {
    child: RetainedChild,
    placement: AnyElement,
}

impl Element for SelfRebuildingElement {
    fn debug_name(&self) -> &'static str {
        "SelfRebuildingElement"
    }

    fn rebuild(&mut self) {
        self.placement = self.child.build();
    }
}

impl Widget for SelfRebuilding {
    #[inline]
    fn to_element(self) -> AnyElement {
        let placement = self.child.build();
        AnyElement::new(SelfRebuildingElement {
            child: self.child,
            placement,
        })
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        "SelfRebuilding"
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// Counts every allocation performed by the current thread.
    ///
    /// The counter is thread local and const initialized, so recording it
    /// costs one non-atomic increment and never allocates itself, which is
    /// what keeps the allocator from recursing into its own bookkeeping.
    struct RecordingAllocator;

    thread_local! {
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    // SAFETY: Every method forwards to `System` unchanged; the counter only
    // observes the call.
    unsafe impl GlobalAlloc for RecordingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record();
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) };
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            record();
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            record();
            unsafe { System.realloc(pointer, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: RecordingAllocator = RecordingAllocator;

    fn record() {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }

    fn allocations() -> usize {
        ALLOCATIONS.with(Cell::get)
    }

    fn shadows() -> Vec<Shadow> {
        vec![
            Shadow {
                blur: 4.0,
                spread: 1.0,
            },
            Shadow {
                blur: 8.0,
                spread: 2.0,
            },
        ]
    }

    fn tree() -> AnyWidget {
        Decorated::new(shadows(), Leaf.boxed()).boxed()
    }

    /// A widget that reports where its shadow buffer ended up, so a test can
    /// tell a move apart from a copy.
    struct Probe {
        shadows: Vec<Shadow>,
    }

    struct ProbeElement {
        _shadows: Vec<Shadow>,
    }

    impl Element for ProbeElement {
        fn debug_name(&self) -> &'static str {
            "ProbeElement"
        }
    }

    impl Widget for Probe {
        fn to_element(self) -> AnyElement {
            let element = ProbeElement {
                _shadows: self.shadows,
            };
            RECEIVED_BUFFER.with(|buffer| buffer.set(element._shadows.as_ptr()));
            RECEIVED_LENGTH.with(|length| length.set(element._shadows.len()));
            AnyElement::new(element)
        }
    }

    thread_local! {
        /// Address of the shadow buffer the last element received.
        static RECEIVED_BUFFER: Cell<*const Shadow> = const { Cell::new(std::ptr::null()) };
        /// Length of the shadow list the last element received.
        static RECEIVED_LENGTH: Cell<usize> = const { Cell::new(0) };
    }

    /// A composable that counts how often it described its subtree, and is
    /// deliberately not `Clone`.
    struct CountingComposable {
        builds: Rc<Cell<usize>>,
    }

    impl Composable for CountingComposable {
        fn build(&self) -> AnyWidget {
            self.builds.set(self.builds.get() + 1);
            Leaf.boxed()
        }
    }

    #[test]
    fn a_build_keeps_the_widgets_own_buffer() {
        let shadows = shadows();
        let buffer = shadows.as_ptr();
        let length = shadows.len();

        let element = Probe { shadows }.boxed().into_element();

        assert_eq!(element.debug_name(), "ProbeElement");
        assert_eq!(
            RECEIVED_BUFFER.with(Cell::get),
            buffer,
            "the shadow list must be moved into the element, never cloned"
        );
        assert_eq!(RECEIVED_LENGTH.with(Cell::get), length);
    }

    #[test]
    fn the_recording_allocator_observes_real_allocations() {
        // Without this guard the zero above could pass because nothing is
        // being counted at all.
        let start = allocations();
        let buffer = shadows();
        let spent = allocations() - start;

        assert_eq!(buffer.len(), 2);
        assert!(spent >= 1, "a vector allocation must be visible");
    }

    #[test]
    fn a_warm_build_reaches_the_allocator_zero_times() {
        // Warm the pool: the first tree pays for its blocks, later trees reuse
        // them. A real application is in this steady state after one frame.
        for _ in 0..4 {
            drop(tree().into_element());
        }

        let widget = tree();
        let start = allocations();
        let element = widget.into_element();
        let spent = allocations() - start;

        assert_eq!(element.debug_name(), "DecoratedElement");
        assert_eq!(
            spent, 0,
            "turning a warm widget tree into an element tree must not allocate"
        );
    }

    #[test]
    fn a_composable_rebuilds_without_ever_being_cloned() {
        let builds = Rc::new(Cell::new(0));
        let widget = Composed::new(CountingComposable {
            builds: Rc::clone(&builds),
        });

        let mut element = widget.boxed().into_element();
        assert_eq!(element.debug_name(), "ComposedElement");
        assert_eq!(builds.get(), 1);

        element.rebuild();
        element.rebuild();

        assert_eq!(
            builds.get(),
            3,
            "the element re-runs the original composable it was given"
        );
    }

    /// A child that counts its conversions and reports when its data dies.
    struct Tracked {
        builds: Rc<Cell<usize>>,
        drops: Rc<Cell<usize>>,
        shadows: Vec<Shadow>,
    }

    struct TrackedElement {
        drops: Rc<Cell<usize>>,
        _shadows: Vec<Shadow>,
    }

    impl Element for TrackedElement {
        fn debug_name(&self) -> &'static str {
            "TrackedElement"
        }
    }

    impl Drop for TrackedElement {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    impl Widget for Tracked {
        fn to_element(self) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            AnyElement::new(TrackedElement {
                drops: self.drops,
                _shadows: self.shadows,
            })
        }

        fn debug_name(&self) -> &'static str {
            "Tracked"
        }
    }

    #[test]
    fn a_self_rebuilding_parent_builds_its_child_once() {
        let builds = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let child = RetainedChild::new(Tracked {
            builds: Rc::clone(&builds),
            drops: Rc::clone(&drops),
            shadows: shadows(),
        });

        let mut element = SelfRebuilding::new(child).boxed().into_element();
        assert_eq!(element.debug_name(), "SelfRebuildingElement");

        for _ in 0..16 {
            element.rebuild();
        }

        assert_eq!(
            builds.get(),
            1,
            "hover, scroll, and theme ticks must not rebuild the subtree below them"
        );
    }

    #[test]
    fn a_self_rebuilding_parent_places_the_same_child_element() {
        let child = RetainedChild::new(Tracked {
            builds: Rc::new(Cell::new(0)),
            drops: Rc::new(Cell::new(0)),
            shadows: shadows(),
        });

        let mut element = SelfRebuilding::new(child.clone()).boxed().into_element();
        let address = child.retained_address();
        assert!(address.is_some());

        for _ in 0..4 {
            element.rebuild();

            assert_eq!(
                child.retained_address(),
                address,
                "every rebuild must place the element the first build produced"
            );
        }
    }

    #[test]
    fn a_retained_child_is_destroyed_exactly_once() {
        let drops = Rc::new(Cell::new(0));
        let child = RetainedChild::new(Tracked {
            builds: Rc::new(Cell::new(0)),
            drops: Rc::clone(&drops),
            shadows: shadows(),
        });

        let mut element = SelfRebuilding::new(child.clone()).boxed().into_element();
        for _ in 0..4 {
            element.rebuild();
        }

        assert_eq!(drops.get(), 0, "a placement going away is not the child");

        drop(element);
        drop(child);

        assert_eq!(
            drops.get(),
            1,
            "the retained element is destroyed once, when the last holder goes"
        );
    }

    #[test]
    fn a_rebuild_with_a_retained_child_reaches_the_allocator_zero_times() {
        let child = RetainedChild::new(Tracked {
            builds: Rc::new(Cell::new(0)),
            drops: Rc::new(Cell::new(0)),
            shadows: shadows(),
        });
        let mut element = SelfRebuilding::new(child).boxed().into_element();

        // Warm the pool the same way a running application does: the first
        // frames pay for their blocks, the steady state reuses them.
        for _ in 0..4 {
            element.rebuild();
        }

        let start = allocations();
        element.rebuild();
        let spent = allocations() - start;

        assert_eq!(
            spent, 0,
            "a parent rebuilding itself over a retained child must not allocate"
        );
    }

    #[test]
    fn a_rebuild_walks_into_the_subtree() {
        let builds = Rc::new(Cell::new(0));
        let composed = Composed::new(CountingComposable {
            builds: Rc::clone(&builds),
        });

        let mut element = Decorated::new(shadows(), composed.boxed())
            .boxed()
            .into_element();
        assert_eq!(builds.get(), 1);

        element.rebuild();

        assert_eq!(builds.get(), 2, "the decorated element forwards rebuilds");
    }
}
