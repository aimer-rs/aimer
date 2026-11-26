//! Parts of a `build` output that survive from one build to the next.

use crate::components::context::BuildContext;
use crate::content_hash::ContentHash;
use crate::key::Key;
use crate::widget::Widget;
use crate::widget::child_builder::ChildBuilder;

/// Keeps parts of a widget's `build` output alive between its builds.
///
/// Implemented for [`BuildContext`]; bring the trait into scope to call
/// [`retain`](BuildContextRetain::retain) on a context.
pub trait BuildContextRetain {
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
    fn retain<W: Widget + 'static>(
        &self,
        key: impl Into<Key>,
        build: impl FnOnce() -> W,
    ) -> ChildBuilder;

    /// Keeps a part of the widget being built alive for as long as its content
    /// hash does not change.
    ///
    /// Like [`retain`](BuildContextRetain::retain), but the dependency list is
    /// the widget's own [`Widget::hash`] instead of a key the caller maintains.
    /// The owner describes `widget` on every build, which is the cheap half; the
    /// slot under `key` remembers the hash of the widget it was built from, and
    /// when the new widget reports the same one the slot's subtree is returned
    /// and `widget` is dropped without being turned into elements. A different
    /// hash builds the slot again from the new widget.
    ///
    /// A widget whose hash is `None` is not retained: it is placed like any other
    /// child. That is what a widget holding a closure must report, since a
    /// closure cannot be hashed.
    ///
    /// # What the caller must know
    ///
    /// - The hash must cover everything the widget's elements are built from,
    ///   its children's hashes included. A field left out is a change that goes
    ///   unseen.
    /// - It cannot cover what the part reads from the build context: theme,
    ///   provider state, scale, constraints. The part is then retained under the
    ///   same limits as [`retain`](BuildContextRetain::retain), and a widget
    ///   that reads such state either belongs under a key that includes it or
    ///   should report `None`.
    /// - A changed hash is the same part reconfigured, not a different one: it is
    ///   built from the new widget and reconciled against the old subtree like
    ///   any rebuild, so the state below it is carried across and handed the new
    ///   configuration (a slider being dragged keeps its drag while its value
    ///   changes on every tick). Only the *unchanged* case skips the work. The
    ///   old hash is not resurrected later, and equal hashes are taken as equal
    ///   widgets, so a hash that collides shows the old part.
    /// - Hashing walks the part's description, so this costs time proportional
    ///   to its size on every build, unlike `retain`, whose closure is not run
    ///   at all. It is for parts whose dependencies are hard to list, not for
    ///   the largest ones.
    ///
    /// The slot's lifetime is the one `retain` documents: it lives as long as
    /// every build asks for `key`, and calling with one key twice in a build is
    /// a misuse. Outside a build there is no scope to keep it in, so nothing is
    /// retained.
    ///
    /// ```ignore
    /// fn build(&self, ctx: &BuildContext) -> impl Widget {
    ///     // Rebuilt only when `Badge`'s own configuration changes.
    ///     ctx.retain_hashed("badge", Badge::new(self.count, self.tone))
    /// }
    /// ```
    fn retain_hashed<W: Widget + 'static>(&self, key: impl Into<Key>, widget: W) -> ChildBuilder;
}

impl BuildContextRetain for BuildContext<'_> {
    fn retain<W: Widget + 'static>(
        &self,
        key: impl Into<Key>,
        build: impl FnOnce() -> W,
    ) -> ChildBuilder {
        let Some(consumer) = self.current_build_consumer() else {
            return ChildBuilder::from_widget(build());
        };
        let key = key.into();
        if let Some(child) = consumer.retained_part::<ChildBuilder>(&key) {
            return child;
        }
        // `build` runs outside the borrow: describing a part may itself reach
        // for the context of a nested widget.
        let child = ChildBuilder::slot_from_widget(build());
        consumer.retain_part(key, child.clone());
        child
    }

    fn retain_hashed<W: Widget + 'static>(&self, key: impl Into<Key>, widget: W) -> ChildBuilder {
        let (Some(hash), Some(consumer)) = (widget.hash(), self.current_build_consumer()) else {
            return ChildBuilder::from_widget(widget);
        };
        let key = key.into();
        if let Some((held, child)) = consumer.retained_part::<(ContentHash, ChildBuilder)>(&key)
            && held == hash
        {
            return child;
        }
        // The part is described outside the borrow, as `retain` does. Not a slot:
        // a changed hash is this part reconfigured, so reconciliation carries the
        // state below it across, as it does for any rebuild.
        let child = ChildBuilder::from_widget(widget);
        consumer.replace_part(key, (hash, child.clone()));
        child
    }
}
