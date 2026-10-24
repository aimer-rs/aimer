//! Child storage behind [`RawFlex`](crate::flex::raw_flex::RawFlex).
//!
//! A flex container used to own a `Vec<AnyElement>` outright, which meant the
//! whole child list had to exist before the container did. For a list of a
//! hundred thousand rows that is half a million allocations paid before the
//! first frame — and paid again on every rebuild, because
//! [`Widget::to_element`] runs on every rebuild.
//!
//! Indirecting through [`ChildrenSource`] lets the container ask for children by
//! index instead of owning them. Two implementations cover the two regimes:
//!
//! - [`EagerChildren`] holds the vector, exactly as before. Everything reached
//!   through [`Flex::children`](crate::Flex::children) uses it, so its behaviour
//!   is unchanged down to the allocation count.
//! - [`WindowedChildren`] holds the *data* plus the mapper and materializes only
//!   the index range a frame asks for. A hundred-thousand-row list then builds a
//!   couple of dozen elements instead of a hundred thousand.
//!
//! # The sparse-children contract
//!
//! A windowed source deliberately exposes fewer children than it has. That is
//! visible to every tree walk, so the meaning of each one is fixed here:
//!
//! - [`ChildrenSource::len`] is the *logical* count and is always exact. It is
//!   what the layout table and the scroll extent are derived from.
//! - [`ChildrenSource::visit`] yields the *materialized* children only. Tree
//!   walks that reach it — structural inspection, broadcasts, lifecycle
//!   delivery — therefore see the live window, not the whole list. An element
//!   outside the window does not exist yet, so there is nothing to deliver to.
//! - Identity *is* preserved while a row stays inside the window, overscan
//!   included, so a one-pixel scroll never rebuilds what is on screen. It also
//!   survives leaving the window: a bounded pool keeps the elements of the
//!   [`RECYCLED`] most recently dropped rows, so scrolling a screen away and back
//!   returns the very same element, with its state, focus, and layout cache.
//! - Beyond that pool per-child state is **not** durable. State that has to
//!   outlive it belongs above the list, in the data the mapper reads.
//! - A rebuild of the container *does* preserve it: the replacement claims every
//!   row the old source held through
//!   [`Rebuildable::adopt_runtime_state_from`](aimer_widget::Rebuildable::adopt_runtime_state_from),
//!   and the live state of a claimed row is carried into the row it builds for
//!   the same identity. Identity is the row's index, or the key
//!   [`FlexList::key`](crate::FlexList::key) derives from the datum, which is
//!   what makes state follow an item that moved.
//! - A freshly built windowed container has an *empty* window until its first
//!   frame. Reconciliation walks children by sibling position, so an empty
//!   window is what keeps a rebuild from pairing row 12 of the old window with
//!   row 12 of a differently positioned new one; the hand-off above replaces
//!   that pairing with an explicit one.
//!
//! Windowing is only sound when the container can state its total extent
//! without measuring it child by child. That holds when the extent is declared
//! through [`FlexList::item_extent`](crate::FlexList::item_extent), and when it
//! is predicted from a single probed child — see
//! [`RawFlex::estimated_layout`](crate::flex::raw_flex::RawFlex). A prediction
//! that turns out wrong is corrected child by child rather than abandoned, so a
//! list of varying rows stays windowed. Only a shape no per-child correction can
//! describe — a flex child, a wrapped line, a bounded main axis — makes a source
//! stop windowing through [`ChildrenSource::materialize_all`]; that decision is
//! final, which keeps a container from alternating between the two regimes frame
//! after frame.

use std::cell::{Cell, UnsafeCell};
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, Element, Key, Widget, carry_element_state};

/// Derives the identity of a datum, so a row can be recognised after the data
/// moved it — see [`FlexList::key`](crate::FlexList::key).
pub(crate) type KeyMapper<T> = Rc<dyn Fn(&T) -> Key>;

/// One live child a source hands to the source that replaces it.
///
/// A rebuild of the container produces an empty source, so the rows the old one
/// held are passed across explicitly and their live state is carried into the
/// rows the new one builds.
pub(crate) struct RetainedRow {
    /// Position the row held in the source it came from.
    index: usize,
    /// Identity its datum defined, when the caller supplied a key mapper.
    key: Option<Key>,
    /// The live element, boxed because [`AnyElement`] stores small elements
    /// inline and the row has to keep its address across the hand-off.
    element: Box<AnyElement>,
}

/// Extra children materialized on each side of the requested range.
///
/// Scrolling moves the range by one row at a time, so a margin absorbs a frame
/// of velocity and keeps the window edges from rebuilding on every tick.
const OVERSCAN: usize = 4;

/// How many rows keep their element after leaving the window.
///
/// Dropping an element throws away everything it held — the state of a
/// [`StatefulWidget`](aimer_widget::StatefulWidget) inside the row, the caret of
/// an input field, a measured layout — so a row that is scrolled out and back
/// would come back blank. Holding on to the most recently dropped ones covers the
/// distance a user actually scrolls back over, at the cost of a few dozen
/// elements. The pool is searched linearly, which is why it stays small.
const RECYCLED: usize = 64;

/// Supplies a flex container's children by index.
///
/// See the [module documentation](self) for what a sparse implementation is
/// allowed to hide.
#[allow(clippy::len_without_is_empty)]
pub(crate) trait ChildrenSource {
    /// Logical number of children, materialized or not.
    fn len(&self) -> usize;

    /// Borrows child `index`, or `None` when it is not materialized.
    fn get(&self, index: usize) -> Option<&dyn Element>;

    /// Visits every materialized child in index order.
    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element));

    /// Visits every materialized child in reverse index order.
    ///
    /// The default preserves the contract for custom sources. The built-in
    /// eager and windowed sources override it so routed hit testing does not
    /// allocate a temporary reverse-order buffer.
    fn visit_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let mut children = Vec::new();
        self.visit(&mut |child| children.push(child));
        for child in children.into_iter().rev() {
            visitor(child);
        }
    }

    /// Index of the first materialized child, when any exists.
    ///
    /// A container that wants to measure one representative child asks for this
    /// so it can probe a row that already exists rather than build one it would
    /// immediately drop.
    fn live_start(&self) -> Option<usize>;

    /// Index to use as a representative child when rebuilding a predicted
    /// layout, when one is known from the source's previous window.
    ///
    /// This is separate from [`ChildrenSource::live_start`]: a freshly built
    /// window is intentionally empty, but a replacement can still remember
    /// which part of the old window was representative without installing
    /// those old elements by position.
    #[inline]
    fn probe_index(&self) -> Option<usize> {
        self.live_start()
    }

    /// Whether children are materialized on demand rather than up front.
    ///
    /// A container that measures its children cannot use a windowed source, so
    /// it consults this before trusting [`ChildrenSource::get`] to answer. It
    /// turns `false` for good once [`ChildrenSource::materialize_all`] was
    /// called.
    #[inline]
    fn is_windowed(&self) -> bool {
        false
    }

    /// Materializes every child and stops windowing for good.
    ///
    /// A pass that derives its result from the whole list — measuring an
    /// undeclared extent, resolving a wrap line break — cannot work with a
    /// partial window. Windowing is disabled rather than merely widened, so the
    /// next [`ChildrenSource::window`] call cannot drop what that pass depends
    /// on. Children already materialized keep their identity.
    #[inline]
    fn materialize_all(&self, _ctx: &BuildContext) {}

    /// Materializes exactly `range`, plus a small overscan, and drops the rest.
    ///
    /// Children whose index stays in range keep their identity. Implementations
    /// that already hold every child ignore this.
    ///
    /// # Safety contract
    ///
    /// Elements outside the new window are dropped, so no borrow handed out by
    /// [`ChildrenSource::get`] may outlive a call to this method.
    #[inline]
    #[allow(unused_variables)]
    fn window(&self, range: Range<usize>, ctx: &BuildContext) {}

    /// Surrenders every materialized child, emptying the source.
    ///
    /// Called on the source of a container being replaced. An eager source has
    /// nothing to say here: reconciliation already pairs its children by sibling
    /// position.
    #[inline]
    fn take_retained(&self) -> Vec<RetainedRow> {
        Vec::new()
    }

    /// Accepts the children surrendered by the source this one replaces.
    ///
    /// They are held, not installed: a row is only worth reviving once this
    /// source builds the same identity, and it may never build it at all.
    #[inline]
    #[allow(unused_variables)]
    fn adopt_retained(&self, children: Vec<RetainedRow>) {}
}

/// Children that were all built up front.
///
/// This is the storage every widget-collection API produces, and it is a thin
/// wrapper: `len`, `get`, and `visit` are the vector's own operations.
pub(crate) struct EagerChildren(pub(crate) Vec<AnyElement>);

impl ChildrenSource for EagerChildren {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    fn get(&self, index: usize) -> Option<&dyn Element> {
        self.0.get(index).map(|child| child.as_ref())
    }

    #[inline]
    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.0 {
            visitor(child.as_ref());
        }
    }

    #[inline]
    fn visit_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in self.0.iter().rev() {
            visitor(child.as_ref());
        }
    }

    #[inline]
    fn live_start(&self) -> Option<usize> {
        (!self.0.is_empty()).then_some(0)
    }
}

/// The contiguous run of materialized children.
///
/// Elements are boxed because [`AnyElement`] stores small elements inline: the
/// deque reallocates as the window slides, and a borrow handed out earlier must
/// keep pointing at the same element.
struct Window {
    /// Index of the child held by the front of `elements`.
    start: usize,
    elements: VecDeque<Box<AnyElement>>,
}

impl Window {
    /// One past the index of the last materialized child.
    #[inline]
    fn end(&self) -> usize {
        self.start + self.elements.len()
    }
}

/// Children built from data on demand, one viewport at a time.
///
/// The container retains `items` and `builder` rather than elements, so the
/// resident cost is the size of the data — four bytes per item for a range of
/// `u32` — plus the couple of dozen elements a frame can actually show.
pub(crate) struct WindowedChildren<T, F> {
    /// Shared with the widget that produced it: [`Widget::to_element`] takes
    /// `&self`, so the data cannot be moved out of the widget.
    items: Rc<Vec<T>>,
    builder: Rc<F>,
    /// Derives a row's identity from its datum, when the caller supplied one.
    ///
    /// Without it a row is identified by its index, which is wrong the moment
    /// the data is inserted into or reordered: the state of row 3 would move to
    /// whatever datum lands at index 3.
    keyed: Option<KeyMapper<T>>,
    /// Rows surrendered by the source this one replaced, still unclaimed.
    ///
    /// Held rather than installed: a row is worth reviving only once this source
    /// builds the same identity, and the window it will build is not known yet.
    inherited: UnsafeCell<Vec<RetainedRow>>,
    /// Representative row from the source this one replaced, if any.
    probe: Cell<Option<usize>>,
    /// Only ever borrowed from the single render thread, and never across a
    /// call that can resize the window.
    live: UnsafeCell<Window>,
    /// Elements of rows that left the window, most recently dropped first.
    ///
    /// Bounded by [`RECYCLED`]. A separate cell from `live` so the two can be
    /// held at once while a row moves between them.
    recycled: UnsafeCell<VecDeque<(usize, Box<AnyElement>)>>,
    /// Set once the container gave up on windowing, after which every child
    /// exists and [`ChildrenSource::window`] does nothing.
    eager: Cell<bool>,
}

impl<T, F> WindowedChildren<T, F> {
    /// Creates a source over `items` that maps each datum through `builder`.
    ///
    /// Nothing is materialized yet: the first [`ChildrenSource::window`] call
    /// decides what exists.
    #[inline]
    pub(crate) fn new(items: Rc<Vec<T>>, builder: Rc<F>, keyed: Option<KeyMapper<T>>) -> Self {
        Self {
            items,
            builder,
            keyed,
            inherited: UnsafeCell::new(Vec::new()),
            probe: Cell::new(None),
            live: UnsafeCell::new(Window {
                start: 0,
                elements: VecDeque::new(),
            }),
            recycled: UnsafeCell::new(VecDeque::new()),
            eager: Cell::new(false),
        }
    }

    /// Borrows the window.
    ///
    /// # Safety
    ///
    /// The caller must not hold the returned borrow across anything that can
    /// mutate the window.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn window_mut(&self) -> &mut Window {
        unsafe { &mut *self.live.get() }
    }

    /// Borrows the pool of rows that left the window.
    ///
    /// # Safety
    ///
    /// Same contract as [`WindowedChildren::window_mut`], over a distinct field:
    /// the two may be held at once, but neither across anything that mutates it.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn recycled_mut(&self) -> &mut VecDeque<(usize, Box<AnyElement>)> {
        unsafe { &mut *self.recycled.get() }
    }

    /// Reclaims the element row `index` had before it left the window.
    ///
    /// Returning it instead of building a new one is what makes a row's state
    /// survive being scrolled past.
    fn take_recycled(&self, index: usize) -> Option<Box<AnyElement>> {
        let pool = unsafe { self.recycled_mut() };
        let slot = pool.iter().position(|(held, _)| *held == index)?;
        pool.remove(slot).map(|(_, element)| element)
    }

    /// Keeps the element of row `index` now that it left the window.
    ///
    /// The oldest entry is dropped once the pool is full, which is the only place
    /// a row's state is ever discarded while its container lives.
    fn retire(&self, index: usize, element: Box<AnyElement>) {
        let pool = unsafe { self.recycled_mut() };
        pool.push_front((index, element));
        while pool.len() > RECYCLED {
            pool.pop_back();
        }
    }

    /// Empties the pool.
    ///
    /// Called once every row is materialized: nothing can leave the window
    /// afterwards, so anything still held would be leaked until the container is
    /// dropped.
    fn clear_recycled(&self) {
        unsafe { self.recycled_mut() }.clear();
    }

    /// Borrows the rows inherited from the replaced source.
    ///
    /// # Safety
    ///
    /// Same contract as [`WindowedChildren::window_mut`], over a distinct field.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn inherited_mut(&self) -> &mut Vec<RetainedRow> {
        unsafe { &mut *self.inherited.get() }
    }

    /// Identity of row `index`, as its datum defines it.
    #[inline]
    fn key_of(&self, index: usize) -> Option<Key> {
        let keyed = self.keyed.as_ref()?;
        Some(keyed(self.items.get(index)?))
    }

    /// Describes a row this source is giving up.
    #[inline]
    fn surrender(&self, index: usize, element: Box<AnyElement>) -> RetainedRow {
        RetainedRow {
            index,
            key: self.key_of(index),
            element,
        }
    }

    /// Carries the live state of an inherited row into the freshly built
    /// `element`, when one of them is the same row.
    ///
    /// This is what makes per-row state survive a rebuild of the container: the
    /// row's element is new — the mapper may well have produced a different
    /// widget from changed data — but everything the old element *held* moves
    /// across. The match is by key when the caller supplied one, so state follows
    /// a datum that moved to another index, and by index otherwise.
    ///
    /// A claimed row is removed, so two rows can never adopt the same state.
    fn revive(&self, index: usize, element: &AnyElement, ctx: &BuildContext) {
        let claimed = {
            let inherited = unsafe { self.inherited_mut() };
            if inherited.is_empty() {
                return;
            }
            let slot = match self.key_of(index) {
                Some(key) => inherited
                    .iter()
                    .position(|row| row.key.as_ref() == Some(&key)),
                None => inherited
                    .iter()
                    .position(|row| row.key.is_none() && row.index == index),
            };
            match slot {
                Some(slot) => inherited.remove(slot),
                None => return,
            }
        };

        carry_element_state(claimed.element.as_ref(), element.as_ref(), ctx);
    }
}

impl<T, W, F> WindowedChildren<T, F>
where
    W: Widget + 'static,
    F: Fn(&T) -> W,
{
    /// Materializes row `index`.
    ///
    /// A row reclaimed from the recycle pool is returned as it was, state
    /// included. A row that has to be built adopts the state of the matching
    /// inherited row, so a rebuild of the container is not visible to it either.
    fn build_row(&self, index: usize, ctx: &BuildContext) -> Box<AnyElement> {
        if let Some(element) = self.take_recycled(index) {
            return element;
        }
        let element = Box::new((self.builder)(&self.items[index]).to_element(ctx));
        self.revive(index, &element, ctx);
        element
    }
}

impl<T, W, F> ChildrenSource for WindowedChildren<T, F>
where
    T: 'static,
    W: Widget + 'static,
    F: Fn(&T) -> W + 'static,
{
    #[inline]
    fn len(&self) -> usize {
        self.items.len()
    }

    #[inline]
    fn get(&self, index: usize) -> Option<&dyn Element> {
        // Shared read of a window that only `window` and `clear` mutate.
        let live = unsafe { &*self.live.get() };
        let element = live.elements.get(index.checked_sub(live.start)?)?;
        Some(AnyElement::as_ref(element))
    }

    #[inline]
    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let live = unsafe { &*self.live.get() };
        for element in &live.elements {
            visitor(AnyElement::as_ref(element));
        }
    }

    #[inline]
    fn visit_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let live = unsafe { &*self.live.get() };
        for element in live.elements.iter().rev() {
            visitor(AnyElement::as_ref(element));
        }
    }

    #[inline]
    fn live_start(&self) -> Option<usize> {
        let live = unsafe { &*self.live.get() };
        (!live.elements.is_empty()).then_some(live.start)
    }

    #[inline]
    fn probe_index(&self) -> Option<usize> {
        self.live_start().or(self.probe.get())
    }

    #[inline]
    fn is_windowed(&self) -> bool {
        !self.eager.get()
    }

    fn materialize_all(&self, ctx: &BuildContext) {
        if self.eager.get() {
            return;
        }
        let len = self.items.len();
        let build = |index: usize| -> Box<AnyElement> { self.build_row(index, ctx) };

        let live = unsafe { self.window_mut() };
        let previous_range = live.start..live.end();
        // Grow around what is already there, so the rows currently on screen
        // keep their element — and with it their state and layout cache.
        if live.elements.is_empty() {
            live.start = 0;
            live.elements.reserve(len);
            for index in 0..len {
                live.elements.push_back(build(index));
            }
        } else {
            while live.start > 0 {
                live.start -= 1;
                let element = build(live.start);
                live.elements.push_front(element);
            }
            while live.end() < len {
                let element = build(live.end());
                live.elements.push_back(element);
            }
        }
        if previous_range != (0..len) {
            aimer_widget::notify_live_children_changed();
        }
        self.clear_recycled();
        self.eager.set(true);
    }

    fn window(&self, range: Range<usize>, ctx: &BuildContext) {
        if self.eager.get() {
            return;
        }
        let len = self.items.len();
        let wanted = range.start.saturating_sub(OVERSCAN).min(len)
            ..range.end.saturating_add(OVERSCAN).min(len);

        // A row that was here before is reclaimed rather than rebuilt, so
        // building is the only allocation this method can perform.
        let build = |index: usize| -> Box<AnyElement> { self.build_row(index, ctx) };

        let live = unsafe { self.window_mut() };
        if live.start == wanted.start && live.end() == wanted.end {
            return;
        }

        // A jump — the scroll bar was dragged, or this is the first frame —
        // shares nothing with the live run, so rebuilding it outright is both
        // cheaper and simpler than sliding across the gap.
        if live.elements.is_empty() || wanted.start >= live.end() || wanted.end <= live.start {
            let leaving = live.start;
            for (offset, element) in live.elements.drain(..).enumerate() {
                self.retire(leaving + offset, element);
            }
            live.start = wanted.start;
            live.elements.reserve(wanted.len());
            for index in wanted {
                live.elements.push_back(build(index));
            }
            aimer_widget::notify_live_children_changed();
            return;
        }

        // Grow first, then trim, so a row that stays in range is never dropped
        // and rebuilt — that is what keeps its state and layout cache alive.
        while live.start > wanted.start {
            live.start -= 1;
            let element = build(live.start);
            live.elements.push_front(element);
        }
        while live.end() < wanted.end {
            let element = build(live.end());
            live.elements.push_back(element);
        }
        while live.start < wanted.start {
            if let Some(element) = live.elements.pop_front() {
                self.retire(live.start, element);
            }
            live.start += 1;
        }
        while live.end() > wanted.end {
            let leaving = live.end() - 1;
            if let Some(element) = live.elements.pop_back() {
                self.retire(leaving, element);
            }
        }
        aimer_widget::notify_live_children_changed();
    }

    fn take_retained(&self) -> Vec<RetainedRow> {
        let live = unsafe { self.window_mut() };
        let pool = unsafe { self.recycled_mut() };
        let mut retained = Vec::with_capacity(live.elements.len() + pool.len());

        // Most relevant first: what was on screen, then what was scrolled past,
        // then whatever this source itself inherited and never claimed.
        let start = live.start;
        for (offset, element) in live.elements.drain(..).enumerate() {
            retained.push(self.surrender(start + offset, element));
        }
        for (index, element) in pool.drain(..) {
            retained.push(self.surrender(index, element));
        }
        retained.append(unsafe { self.inherited_mut() });
        retained
    }

    fn adopt_retained(&self, children: Vec<RetainedRow>) {
        if self.probe.get().is_none() {
            self.probe.set(children.first().map(|row| row.index));
        }
        let inherited = unsafe { self.inherited_mut() };
        inherited.extend(children);
        // Rebuilds can chain without a frame in between, so the unclaimed tail
        // would otherwise grow without bound. The order is by relevance, so the
        // rows dropped here are the ones furthest from any viewport.
        inherited.truncate(RECYCLED);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::size::ResolvedSize;

    use super::*;
    use crate::flex::test_support::{CountingChild, dummy_build_context};

    /// A widget whose element records how many times it was built.
    struct Counting(Rc<Cell<usize>>);

    impl Widget for Counting {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            self.0.set(self.0.get() + 1);
            let unused = Rc::new(Cell::new(0));
            CountingChild::boxed_new(10.0, 20.0, &unused, &unused)
        }
    }

    impl aimer_widget::PortableWidget for Counting {}

    fn source(built: &Rc<Cell<usize>>) -> WindowedChildren<u32, impl Fn(&u32) -> Counting + use<>> {
        sized_source(100_000, built)
    }

    fn sized_source(
        len: u32,
        built: &Rc<Cell<usize>>,
    ) -> WindowedChildren<u32, impl Fn(&u32) -> Counting + use<>> {
        let built = built.clone();
        WindowedChildren::new(
            Rc::new((0..len).collect()),
            Rc::new(move |_: &u32| Counting(built.clone())),
            None,
        )
    }

    /// Nothing exists until a frame asks for a range: that is what removes the
    /// cold-start cost.
    #[test]
    fn nothing_is_built_before_the_first_window() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);

        assert_eq!(children.len(), 100_000);
        assert_eq!(built.get(), 0);
        assert!(children.get(0).is_none());
    }

    #[test]
    fn only_the_windowed_range_is_built() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        children.window(10..14, &ctx);

        assert_eq!(built.get(), 4 + 2 * OVERSCAN);
        assert!(children.get(10).is_some());
        assert!(children.get(13).is_some());
        assert!(children.get(10 - OVERSCAN - 1).is_none());
        assert!(children.get(13 + OVERSCAN + 1).is_none());
    }

    /// Every change to the live window reaches the event index, whose pointers
    /// to dropped rows would otherwise dangle: a jump, a slide and a full
    /// materialization all drop or build rows in place.
    #[test]
    fn every_window_change_reports_the_live_children_change() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);
        let mut seen = aimer_widget::live_children_generation();
        let mut advanced = |what: &str| {
            let now = aimer_widget::live_children_generation();
            assert_ne!(now, seen, "{what} must report the change");
            seen = now;
        };

        children.window(100..110, &ctx);
        advanced("the first window");
        children.window(101..111, &ctx);
        advanced("a slide");
        children.window(50_000..50_004, &ctx);
        advanced("a jump");
        children.materialize_all(&ctx);
        advanced("materializing every row");
    }

    /// Only the materialized children are visited, which is the sparse half of
    /// the contract.
    #[test]
    fn visit_yields_the_live_window_only() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);
        children.window(500..504, &ctx);

        let mut visited = 0;
        children.visit(&mut |_| visited += 1);

        assert_eq!(visited, 4 + 2 * OVERSCAN);
    }

    /// A row that stays in range must keep the very same element, otherwise a
    /// scroll of one pixel would reset every visible row's state.
    #[test]
    fn a_row_that_stays_in_range_keeps_its_identity() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        children.window(100..110, &ctx);
        let before = children.get(105).unwrap() as *const dyn Element;
        let built_once = built.get();

        children.window(101..111, &ctx);
        let after = children.get(105).unwrap() as *const dyn Element;

        assert!(std::ptr::eq(before, after), "row 105 was rebuilt");
        assert_eq!(
            built.get(),
            built_once + 1,
            "sliding by one row must build exactly one row"
        );
    }

    /// Scrolling far away shares nothing with the live run, so the window is
    /// rebuilt rather than slid across the gap.
    #[test]
    fn a_distant_window_is_rebuilt_from_scratch() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        children.window(0..4, &ctx);
        built.set(0);
        children.window(50_000..50_004, &ctx);

        assert_eq!(built.get(), 4 + 2 * OVERSCAN);
        assert!(children.get(0).is_none());
        assert!(children.get(50_000).is_some());
    }

    #[test]
    fn a_window_is_clamped_to_the_item_count() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        children.window(99_998..100_400, &ctx);

        assert!(children.get(99_999).is_some());
        assert!(children.get(100_000).is_none());
    }

    /// Giving up on windowing has to be final: every child exists from then on,
    /// and a later window can no longer drop any of them.
    #[test]
    fn materializing_everything_stops_windowing_for_good() {
        let built = Rc::new(Cell::new(0));
        let children = sized_source(20, &built);
        let ctx = dummy_build_context(100.0, 100.0, None);
        children.window(8..10, &ctx);
        let live = children.get(9).unwrap() as *const dyn Element;

        children.materialize_all(&ctx);

        assert!(!children.is_windowed());
        assert_eq!(built.get(), 20, "every row has to exist afterwards");
        assert!(
            std::ptr::eq(children.get(9).unwrap() as *const dyn Element, live),
            "a row that was already live must keep its element"
        );

        children.window(0..1, &ctx);

        assert_eq!(built.get(), 20, "nothing may be rebuilt");
        assert!(children.get(19).is_some(), "nothing may be dropped");
        assert_eq!(children.live_start(), Some(0));
    }

    /// Nothing is materialized yet, so there is no row to probe.
    #[test]
    fn an_empty_window_has_no_live_start() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        assert_eq!(children.live_start(), None);

        children.window(700..704, &ctx);

        assert_eq!(children.live_start(), Some(700 - OVERSCAN));
    }

    /// A replacement starts with an empty live window, but it still needs the
    /// old window's representative row when it has to predict its extent.
    #[test]
    fn an_adopted_window_keeps_its_previous_probe_index() {
        let built = Rc::new(Cell::new(0));
        let old = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        old.window(700..704, &ctx);
        let new = source(&built);
        new.adopt_retained(old.take_retained());

        assert_eq!(new.live_start(), None, "the new window must stay empty");
        assert_eq!(new.probe_index(), Some(700 - OVERSCAN));
    }

    /// A row that was scrolled out and back has to come back as the *same*
    /// element: that is what keeps the state of a stateful row, and the caret of
    /// an input field, alive across a scroll.
    #[test]
    fn a_row_scrolled_out_and_back_keeps_its_element() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);

        children.window(0..4, &ctx);
        let before = children.get(2).unwrap() as *const dyn Element;
        let built_once = built.get();

        // Far enough that nothing is shared with the previous window.
        children.window(400..404, &ctx);
        assert!(children.get(2).is_none(), "row 2 must leave the window");
        children.window(0..4, &ctx);

        assert!(
            std::ptr::eq(children.get(2).unwrap() as *const dyn Element, before),
            "row 2 was rebuilt instead of reclaimed"
        );
        assert_eq!(
            built.get(),
            built_once + 4 + 2 * OVERSCAN,
            "only the rows that were never seen may be built"
        );
    }

    /// Keeping every row that ever left the window would grow without bound, so
    /// the pool has a ceiling and the oldest row past it is rebuilt.
    #[test]
    fn the_recycle_pool_is_bounded() {
        let built = Rc::new(Cell::new(0));
        let children = source(&built);
        let ctx = dummy_build_context(100.0, 100.0, None);
        children.window(0..1, &ctx);

        // Slide far enough that the first row falls out of the pool.
        for start in 1..2 * RECYCLED {
            children.window(start..start + 1, &ctx);
        }
        let built_before = built.get();
        children.window(0..1, &ctx);

        assert!(
            built.get() > built_before,
            "the pool kept every row it was ever handed"
        );
    }

    /// An eager source is not allowed to hide anything, and `window` must be a
    /// no-op on it.
    #[test]
    fn eager_children_expose_everything() {
        let unused = Rc::new(Cell::new(0));
        let ctx = dummy_build_context(10.0, 10.0, None);
        let children = EagerChildren(
            (0..3)
                .map(|_| CountingChild::boxed_new(10.0, 20.0, &unused, &unused))
                .collect(),
        );

        children.window(1..2, &ctx);

        assert_eq!(children.len(), 3);
        assert!(!children.is_windowed());
        assert_eq!(
            children.get(2).unwrap().computed_size(&ctx),
            ResolvedSize {
                width: 10.0,
                height: 20.0
            }
        );

        let mut visited = 0;
        children.visit(&mut |_| visited += 1);
        assert_eq!(visited, 3);
        assert_eq!(children.live_start(), Some(0));
    }
}
