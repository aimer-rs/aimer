//! Parts of a `build` output that survive from one build to the next.

use crate::key::Key;
use crate::widget::Widget;
use crate::widget::child_builder::ChildBuilder;

use super::BuildContext;

/// The retained parts one build scope keeps between its builds.
///
/// A scope holds a handful of parts at most, so a vector scanned by key beats a
/// hash map in both lookup and allocation; the builds that matter most hold one.
#[derive(Default)]
pub(super) struct Slots {
    /// Counts builds of the owning scope, so a slot can tell whether the build
    /// that just ended asked for it.
    build: u64,
    slots: Vec<Slot>,
}

struct Slot {
    key: Key,
    /// The build that last asked for this slot.
    requested: u64,
    child: ChildBuilder,
}

impl Slots {
    /// Starts a build: slots are measured against this build from now on.
    #[inline]
    pub(super) fn begin_build(&mut self) {
        self.build += 1;
    }

    /// Ends a build: a slot this build did not ask for is released, so a part
    /// the owner stopped showing does not keep its element alive.
    #[inline]
    pub(super) fn end_build(&mut self) {
        if !self.slots.is_empty() {
            let build = self.build;
            self.slots.retain(|slot| slot.requested == build);
        }
    }

    fn get(&mut self, key: &Key) -> Option<ChildBuilder> {
        let build = self.build;
        let slot = self.slots.iter_mut().find(|slot| &slot.key == key)?;
        slot.requested = build;
        Some(slot.child.clone())
    }

    fn insert(&mut self, key: Key, child: ChildBuilder) {
        self.slots.push(Slot {
            key,
            requested: self.build,
            child: child.clone(),
        });
    }
}

impl BuildContext<'_> {
    /// Keeps a part of the widget being built alive between its builds.
    ///
    /// `build` describes the part. It runs the first time `key` is asked for in
    /// this widget, and the part it describes is turned into elements once.
    /// Every later build that asks for the same `key` gets that same subtree
    /// back without calling `build`, so the owner pays neither for describing
    /// the part nor for reconciling it: reconciliation sees the same retained
    /// subtree on both sides and walks no further.
    ///
    /// # The key is the dependency list
    ///
    /// The part is *not* rebuilt when the owner's state changes, because
    /// nothing says which of that state the part used. State the part reads
    /// therefore belongs in the key: when it changes, the key differs, `build`
    /// runs again and a new subtree replaces the old one. A part that reads no
    /// owner state at all can use a constant key.
    ///
    /// ```ignore
    /// fn build(&self, ctx: &BuildContext) -> impl Widget {
    ///     Column::new()
    ///         // Never changes with this state: built once.
    ///         .child(ctx.retain("toolbar", || Toolbar::new()))
    ///         // Depends on `revision`: rebuilt when it moves.
    ///         .child(ctx.retain(("body", self.revision), || Body::new(self.revision)))
    /// }
    /// ```
    ///
    /// # What survives
    ///
    /// The element, and with it the state of every stateful widget inside it,
    /// scroll offsets and GPU resources, as for any [`ChildBuilder`]. A
    /// stateful widget inside the part that reads inherited context is still
    /// rebuilt when the owner builds again, so a theme or provider change
    /// reaches it; one that reads none is left alone. A stateless widget
    /// inside is always rebuilt on a re-placement.
    ///
    /// # Lifetime of a slot
    ///
    /// A slot lives as long as every build of the owner asks for it. A build
    /// that does not ask for a key releases that key's part, and asking for it
    /// later declares it afresh.
    ///
    /// A key identifies a part within one widget, not across the tree. Calling
    /// this with the same key twice in one build returns the same subtree for
    /// both calls, which is a misuse: a retained subtree has one position.
    ///
    /// Outside a stateful or stateless widget's build there is no scope to keep
    /// a part in, so nothing is retained and `build` runs on every call.
    pub fn retain<W: Widget + 'static>(
        &self,
        key: impl Into<Key>,
        build: impl FnOnce() -> W,
    ) -> ChildBuilder {
        let Some(consumer) = self.current_build_consumer() else {
            return ChildBuilder::from_widget(build());
        };
        let key = key.into();
        if let Some(child) = consumer.retained.borrow_mut().get(&key) {
            return child;
        }
        // `build` runs outside the borrow: describing a part may itself reach
        // for the context of a nested widget.
        let child = ChildBuilder::slot_from_widget(build());
        consumer.retained.borrow_mut().insert(key, child.clone());
        child
    }
}
