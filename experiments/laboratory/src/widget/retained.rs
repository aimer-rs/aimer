//! A child a parent can hand to the tree more than once.
//!
//! A consuming [`Widget::to_element`] serves exactly one build, which is all a
//! container needs: a container is rebuilt by *its* parent, so it receives a
//! fresh child widget every time. A widget that rebuilds *itself* is the hard
//! case — a button on hover, a viewport on a new offset, a theme on every tick
//! of its transition — because its child widget was already consumed by the
//! first build and cannot answer a second one.
//!
//! Reproducing the widget is not available: cloning it would require a `Clone`
//! bound the tree does not have (an erased child is not `Clone`), and asking
//! the caller for a factory would change `child(widget)` into
//! `child(|| widget)` at every call site.
//!
//! So the child is *retained* instead. The first build consumes the widget and
//! keeps the element it produced; every later build hands the tree a thin proxy
//! over that same element. The subtree is therefore not rebuilt at all when its
//! parent rebuilds itself, which is strictly cheaper than the borrowing
//! conversion it replaces — that one re-ran the whole subtree on every hover.

use std::cell::{Cell, UnsafeCell};
use std::rc::Rc;

use crate::element::{AnyElement, Element};
use crate::widget::Widget;
use crate::widget::erased::AnyWidget;

/// A child subtree its parent may place into the tree any number of times.
///
/// # Invariant
///
/// One slot describes **one position** in the tree. A slot is cheap to clone —
/// a reference-count bump — so a parent can carry it across its own rebuilds,
/// but cloning it into two different positions would place the same element
/// twice and is a misuse; [`RetainedChild::build`] asserts against the
/// re-entrant half of that mistake in debug builds.
///
/// # Example
///
/// ```
/// use aimer_laboratory::{AnyElement, Element, RetainedChild, Widget};
///
/// struct Leaf;
///
/// struct LeafElement;
///
/// impl Element for LeafElement {
///     fn debug_name(&self) -> &'static str {
///         "LeafElement"
///     }
/// }
///
/// impl Widget for Leaf {
///     fn to_element(self) -> AnyElement {
///         AnyElement::new(LeafElement)
///     }
/// }
///
/// let slot = RetainedChild::new(Leaf);
///
/// // Both builds name the same retained element.
/// assert_eq!(slot.build().debug_name(), "LeafElement");
/// assert_eq!(slot.build().debug_name(), "LeafElement");
/// ```
pub struct RetainedChild(Rc<Slot>);

/// The cell a [`RetainedChild`] and all of its proxies share.
struct Slot {
    /// The child widget, until the first build takes it.
    widget: Cell<Option<AnyWidget>>,
    /// The element the first build produced.
    ///
    /// An `UnsafeCell` rather than a `RefCell`: the tree is single threaded and
    /// a proxy hands out a borrow of the element on every read, so a `RefCell`
    /// would add a counter to the hot path for an invariant the tree already
    /// guarantees structurally. `active` records the one violation that could
    /// still happen by accident — a child reaching back into its own slot.
    element: UnsafeCell<Option<AnyElement>>,
    /// Set while a borrow of `element` is outstanding.
    active: Cell<bool>,
    /// The retained child's diagnostic name, readable without a borrow.
    debug_name: &'static str,
}

impl RetainedChild {
    /// Retains `child`, to be built on first use.
    ///
    /// The widget is stored, not built: a parent is constructed long before it
    /// reaches the tree, and a widget that never reaches the tree must never
    /// produce an element.
    #[inline]
    pub fn new<W: Widget>(child: W) -> Self {
        let debug_name = child.debug_name();
        Self(Rc::new(Slot {
            widget: Cell::new(Some(child.boxed())),
            element: UnsafeCell::new(None),
            active: Cell::new(false),
            debug_name,
        }))
    }

    /// Produces the element for this child's position.
    ///
    /// The first call consumes the retained widget; every later call returns a
    /// proxy over the element that first call produced, so the subtree itself
    /// is built exactly once no matter how often its parent rebuilds.
    pub fn build(&self) -> AnyElement {
        debug_assert!(
            !self.0.active.get(),
            "a retained child cannot be built from inside itself"
        );

        if let Some(widget) = self.0.widget.take() {
            let element = widget.into_element();
            // SAFETY: No proxy exists yet for this slot on the first build, and
            // `active` is clear, so no borrow of the cell is outstanding.
            unsafe { *self.0.element.get() = Some(element) };
        }

        AnyElement::new(RetainedChildElement(Rc::clone(&self.0)))
    }

    /// Returns the address of the retained element, once it exists.
    ///
    /// Exposed so an experiment can assert *reuse* rather than merely a build
    /// count: the address is the element's identity.
    #[inline]
    pub fn retained_address(&self) -> Option<*const ()> {
        self.0
            .with_element(|element| element.map(AnyElement::address))
    }

    /// Returns the retained child's diagnostic name.
    #[inline]
    pub fn debug_name(&self) -> &'static str {
        self.0.debug_name
    }
}

impl Clone for RetainedChild {
    #[inline]
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl Slot {
    /// Runs `read` against the retained element, guarding re-entry.
    #[inline]
    fn with_element<R>(&self, read: impl FnOnce(Option<&AnyElement>) -> R) -> R {
        debug_assert!(!self.active.get(), "a retained child cannot alias itself");
        self.active.set(true);
        // SAFETY: The tree is single threaded and `active` proves no other
        // borrow of this cell is live, so this reference is exclusive for the
        // duration of `read`.
        let result = read(unsafe { (*self.element.get()).as_ref() });
        self.active.set(false);
        result
    }

    /// Runs `write` against the retained element, guarding re-entry.
    #[inline]
    fn with_element_mut<R>(&self, write: impl FnOnce(Option<&mut AnyElement>) -> R) -> R {
        debug_assert!(!self.active.get(), "a retained child cannot alias itself");
        self.active.set(true);
        // SAFETY: As above, and the exclusivity `&mut` requires is what
        // `active` is tracking.
        let result = write(unsafe { (*self.element.get()).as_mut() });
        self.active.set(false);
        result
    }
}

/// The element a parent places into the tree for a retained child.
///
/// It owns no subtree of its own: every question about the child is forwarded
/// to the one retained element, so identity, diagnostics, and rebuild
/// propagation are the child's own and not the proxy's.
struct RetainedChildElement(Rc<Slot>);

impl Element for RetainedChildElement {
    fn debug_name(&self) -> &'static str {
        self.0
            .with_element(|element| element.map(AnyElement::debug_name))
            .unwrap_or("RetainedChild")
    }

    fn rebuild(&mut self) {
        self.0.with_element_mut(|element| {
            if let Some(element) = element {
                element.rebuild();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// A child that counts how often it was converted and where it landed.
    struct Probe {
        builds: Rc<Cell<usize>>,
    }

    struct ProbeElement {
        rebuilds: usize,
    }

    impl Element for ProbeElement {
        fn debug_name(&self) -> &'static str {
            "ProbeElement"
        }

        fn rebuild(&mut self) {
            self.rebuilds += 1;
        }
    }

    impl Widget for Probe {
        fn to_element(self) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            AnyElement::new(ProbeElement { rebuilds: 0 })
        }

        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    #[test]
    fn a_retained_child_is_built_once_however_often_it_is_placed() {
        let builds = Rc::new(Cell::new(0));
        let slot = RetainedChild::new(Probe {
            builds: Rc::clone(&builds),
        });

        for _ in 0..8 {
            drop(slot.build());
        }

        assert_eq!(
            builds.get(),
            1,
            "a parent rebuilding itself must not rebuild its child subtree"
        );
    }

    #[test]
    fn every_placement_names_the_same_element() {
        let slot = RetainedChild::new(Probe {
            builds: Rc::new(Cell::new(0)),
        });

        let first = slot.build();
        let address = slot.retained_address();
        let second = slot.build();

        assert_eq!(first.debug_name(), "ProbeElement");
        assert_eq!(second.debug_name(), "ProbeElement");
        assert_eq!(
            slot.retained_address(),
            address,
            "the retained element is the same element on every placement"
        );
    }

    #[test]
    fn a_slot_that_never_reaches_the_tree_builds_nothing() {
        let builds = Rc::new(Cell::new(0));
        drop(RetainedChild::new(Probe {
            builds: Rc::clone(&builds),
        }));

        assert_eq!(builds.get(), 0);
    }

    #[test]
    fn a_placement_reports_the_childs_name_before_it_is_built() {
        let slot = RetainedChild::new(Probe {
            builds: Rc::new(Cell::new(0)),
        });

        assert_eq!(slot.debug_name(), "Probe");
    }

    #[test]
    fn a_rebuild_reaches_the_retained_element() {
        let slot = RetainedChild::new(Probe {
            builds: Rc::new(Cell::new(0)),
        });

        let mut placement = slot.build();
        placement.rebuild();

        assert_eq!(placement.debug_name(), "ProbeElement");
    }
}
