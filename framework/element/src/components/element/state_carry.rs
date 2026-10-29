//! Moves live state from a replaced subtree into the subtree that replaces it.
//!
//! A rebuild produces a fresh subtree. Before it takes the old one's place, the
//! walk here pairs the two and hands over whatever the old elements were
//! holding: the state of stateful elements, and the runtime state other
//! elements keep beside their children.
//!
//! The walk is generic over the elements; what it hands over is reached through
//! [`StateCarrier`], which the layer that defines stateful elements implements.

use std::any::Any;

use super::*;

/// The state-owning part of an element, as the state carry sees it.
///
/// Reached through [`Rebuildable::state_carrier`]. Two carriers describe the
/// same state when their name and key agree.
#[doc(hidden)]
pub trait StateCarrier {
    /// The key the state is looked up by wherever it reappears, if it has one.
    fn carrier_key(&self) -> Option<&Key>;

    /// The name of the widget the state belongs to.
    fn carrier_name(&self) -> &'static str;

    /// How many times the state has been updated; the larger one is the newer.
    fn carrier_revision(&self) -> u64;

    /// Takes over the live state of `old`, a carrier of the same kind.
    fn adopt_state_from(&self, old: &dyn StateCarrier, ctx: &BuildContext);

    /// This carrier as the element it is.
    fn as_element(&self) -> &dyn Element;

    /// This carrier as `Any`, so an implementation can reach its concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// If both elements are stateful with the same `debug_name`, adopt
/// the live state from `old` into `new`. This is the rescue path that runs
/// even when the element tree shape changed, it ensures nested stateful
/// widgets (e.g. tab buttons, form inputs) keep their runtime state across
/// a parent rebuild.
///
/// Safe to call on any pair: when both sides aren't matching
/// `StatefulElement`s, it's a no-op.
#[doc(hidden)]
pub fn carry_stateful(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    if !old.is_stateful_element() && !new.is_stateful_element() {
        return;
    }

    let Some(old_ele) = old.state_carrier() else {
        return;
    };
    let Some(new_ele) = new.state_carrier() else {
        return;
    };
    if old_ele.carrier_name() != new_ele.carrier_name()
        || old_ele.carrier_key() != new_ele.carrier_key()
    {
        return;
    }
    new_ele.adopt_state_from(old_ele, ctx);
}

struct KeyedStateIndex<'a> {
    states: HashMap<(&'a Key, &'static str), &'a dyn StateCarrier>,
}

impl<'a> KeyedStateIndex<'a> {
    fn from_subtree(root: &'a dyn Element) -> Self {
        let mut states = HashMap::new();
        let mut pending = vec![root];

        while let Some(current) = pending.pop() {
            if let Some(stateful) = current.state_carrier()
                && let Some(key) = stateful.carrier_key()
            {
                let identity = (key, stateful.carrier_name());
                let should_replace = states
                    .get(&identity)
                    .is_none_or(|existing: &&dyn StateCarrier| {
                        existing.carrier_revision() < stateful.carrier_revision()
                    });
                if should_replace {
                    states.insert(identity, stateful);
                }
            }

            pending.extend(element_children(current).into_iter().rev());
        }

        Self { states }
    }

    #[inline]
    fn get(&self, key: &Key, debug_name: &'static str) -> Option<&'a dyn StateCarrier> {
        self.states.get(&(key, debug_name)).copied()
    }
}

/// Reports whether two element references name the same element.
///
/// Compares the data addresses only: a retained child is reached through two
/// different proxies, so the vtable halves of the two fat pointers are the ones
/// that may differ while the element behind them is one and the same.
#[inline]
fn same_element(left: &dyn Element, right: &dyn Element) -> bool {
    std::ptr::eq(
        left as *const dyn Element as *const (),
        right as *const dyn Element as *const (),
    ) || crate::components::element::same_retained_child(left, right)
}

fn element_children(element: &dyn Element) -> SmallVec<[&dyn Element; 8]> {
    structural_children(element)
}

/// Recurse into the matched children of an old and new element tree, letting
/// each nested `StatefulElement` carry its runtime state from the old subtree
/// into the new one.
///
/// Children are enumerated through the canonical structural-child accessor so
/// elements such as scrollable containers can expose their layout children
/// even when those children are hidden from event dispatch.
#[doc(hidden)]
pub fn carry_child_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    if same_element(old, new) {
        return;
    }
    carry_keyed_child_state(old, new, ctx);
    carry_unkeyed_child_state(old, new, ctx);
}

fn carry_keyed_child_state(old_root: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    new.with_rebuild_context(ctx, &mut |ctx| {
        let mut keyed_states = None;
        carry_keyed_child_state_in_context(old_root, new, ctx, &mut keyed_states)
    });
}

fn carry_keyed_child_state_in_context<'a>(
    old_root: &'a dyn Element,
    new: &dyn Element,
    ctx: &BuildContext,
    keyed_states: &mut Option<KeyedStateIndex<'a>>,
) {
    if new.is_stateful_element()
        && let Some(new_stateful) = new.state_carrier()
        && let Some(key) = new_stateful.carrier_key()
    {
        let old_stateful = keyed_states
            .get_or_insert_with(|| KeyedStateIndex::from_subtree(old_root))
            .get(key, new_stateful.carrier_name());
        if let Some(old_stateful) = old_stateful
            && old_stateful.carrier_revision() >= new_stateful.carrier_revision()
        {
            new_stateful.adopt_state_from(old_stateful, ctx);
            carry_state_below_keyed(old_stateful, new_stateful, ctx);
        }
        return;
    }

    for child in element_children(new) {
        // A descendant may publish inherited state (for example, a provider).
        // Keep entering each element's rebuild scope while sharing the keyed
        // index, otherwise a keyed state rebuild can see the bare frame context.
        child.with_rebuild_context(ctx, &mut |ctx| {
            carry_keyed_child_state_in_context(old_root, child, ctx, keyed_states);
        });
    }
}

/// Hands the subtree below a keyed stateful element over to the element that
/// replaced it.
///
/// A keyed element keeps its own state wherever it reappears, it is looked up
/// by key, not by position, but the subtree it rebuilds from that state is
/// constructed anew, and everything below it is ordinary unkeyed state: a
/// request an `AsyncBuilder` already completed, a scroll offset, an animation
/// halfway through. Without this hand-over all of it starts over whenever an
/// ancestor rebuilds, which for a route transition wrapping a page means every
/// tick of an unrelated animation above it.
///
/// Unlike the walk below an unkeyed element, this one cannot assume the two
/// subtrees line up. A keyed element deliberately outlives changes to what it
/// contains, one route transition keeps its identity while the page inside it
/// is replaced, so a pair is only descended into once both sides agree on what
/// they are. Everything else keeps the state it was built with, which is how a
/// navigation still gets its new page.
///
/// Keys nested below this one are resolved first, and by key rather than by
/// position: the walk that follows deliberately steps over keyed elements, and
/// the pass that would otherwise claim them stopped at this element. A section
/// that names itself inside a page inside a keyed route transition, the shape
/// `website/src/screen/home_screen.rs` builds, would otherwise be the one
/// thing an ancestor's rebuild resets.
fn carry_state_below_keyed(old: &dyn StateCarrier, new: &dyn StateCarrier, ctx: &BuildContext) {
    if std::ptr::eq(
        old as *const dyn StateCarrier as *const (),
        new as *const dyn StateCarrier as *const (),
    ) {
        return;
    }
    let (old, new) = (old.as_element(), new.as_element());

    let mut keyed_states = None;
    for child in element_children(new) {
        child.with_rebuild_context(ctx, &mut |ctx| {
            carry_keyed_child_state_in_context(old, child, ctx, &mut keyed_states);
        });
    }

    carry_matching_child_state(old, new, ctx);
}

/// Pairs the children of `old` and `new` by position and hands over the state of
/// every pair that describes the same widget.
fn carry_matching_child_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    let old_children = element_children(old);
    if old_children.is_empty() {
        return;
    }

    let new_children = element_children(new);
    for (old_child, new_child) in old_children.iter().zip(new_children.iter()) {
        if !identities_are_compatible(*old_child, *new_child) {
            continue;
        }

        carry_matching_state(*old_child, *new_child, ctx);
    }
}

/// Hands one compatible pair over, then continues into their children.
fn carry_matching_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    // A retained child appears in both trees, so a pair can be one element
    // twice over; it has nothing to hand to itself.
    if same_element(old, new) {
        return;
    }
    new.with_rebuild_context(ctx, &mut |ctx| {
        adopt_runtime_state(old, new);

        if old.is_stateful_element()
            && new.is_stateful_element()
            && let Some(new_stateful) = new.state_carrier()
        {
            // A keyed element is the keyed pass's business, not this walk's.
            if new_stateful.carrier_key().is_some() {
                return;
            }

            carry_stateful(old, new, ctx);
        }

        carry_matching_child_state(old, new, ctx);
    });
}

/// Lets `new` take over the runtime state `old` was holding beside its children.
///
/// The positional walk below reaches every child an element owns, which covers
/// almost everything. What it cannot reach is a container that materializes its
/// children on demand, a freshly built one has none, so there is no pair to walk
///, or state a container keeps beside them, such as a measurement of a list too
/// long to measure again. Both are handed over here instead; see
/// [`Rebuildable::adopt_runtime_state_from`].
///
/// Restricted to elements of the same concrete type, so an element is never
/// offered state it cannot interpret.
fn adopt_runtime_state(old: &dyn Element, new: &dyn Element) {
    if old.element_type_id() != new.element_type_id() || old.debug_name() != new.debug_name() {
        return;
    }

    new.adopt_runtime_state_from(old);
}

fn carry_unkeyed_child_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    // A retained child is placed into the old and the new tree alike, so the
    // pairing walk can arrive with the same element on both sides. Nothing can
    // be carried from an element into itself, and descending would carry
    // everything below it twice.
    if same_element(old, new) || different_retained_children(old, new)
    {
        return;
    }
    new.with_rebuild_context(ctx, &mut |ctx| {
        carry_unkeyed_child_state_in_context(old, new, ctx)
    });
}

fn carry_unkeyed_child_state_in_context(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    adopt_runtime_state(old, new);

    if old.is_stateful_element() && new.is_stateful_element() {
        if new
            .state_carrier()
            .is_some_and(|stateful| stateful.carrier_key().is_some())
        {
            return;
        }

        carry_stateful(old, new, ctx);
    }

    let old_children = element_children(old);
    if old_children.is_empty() {
        return;
    }

    let new_children = element_children(new);

    for (old_child, new_child) in old_children.iter().zip(new_children.iter()) {
        carry_unkeyed_child_state(*old_child, *new_child, ctx);
    }
}

