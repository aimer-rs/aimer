//! A wrapper that tells assistive technology what its child is.

use aimer_accessibility::{NodeId, Role, SemanticAction, SemanticNode};
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_utils::callback::{CallbackExecutor, VoidCallback};

use crate::base::BuildContext;
use crate::components::event_element::{EventElement, EventResult, EventTreeRole};
use crate::components::layout_element::LayoutElement;
use crate::components::rebuildable::Rebuildable;
use crate::components::visitor_element::VisitorElement;
use crate::{AnyElement, AnyWidget, Drawable, Element, RequiredChild, Widget};

/// Describes its child to assistive technology without changing how it lays
/// out, paints or handles input.
///
/// Widgets that are *composed* from others — a button made of a gesture
/// detector, a container and some text — have no single element that knows it is
/// a button. `Semantics` is where that fact is stated: it publishes one node
/// with the given [`Role`], and its [activation](Semantics::on_activate) is what
/// a screen reader's "press" runs.
///
/// By default the node [merges](Semantics::merges) its subtree: the text the
/// child displays becomes the node's name and the child's own nodes are not
/// exposed separately, so a button containing "Save" is announced as one
/// "Save, button" and not as a button holding a loose label.
///
/// # Examples
///
/// ```
/// use aimer_accessibility::Role;
/// use aimer_widget::Semantics;
///
/// struct Icon;
/// # impl aimer_widget::PortableWidget for Icon {}
/// # impl aimer_widget::Widget for Icon {
/// #     fn to_element(self, _ctx: &aimer_widget::base::BuildContext) -> aimer_widget::AnyElement {
/// #         unreachable!("this example only builds the widget")
/// #     }
/// # }
///
/// // An icon-only control has no text to read, so it is given a name.
/// let close = Semantics::new()
///     .role(Role::Button)
///     .label("Close")
///     .on_activate(|| println!("closed"))
///     .child(Icon);
/// ```
pub struct Semantics<W = RequiredChild> {
    child: W,
    spec: Spec,
}

#[derive(Clone)]
struct Spec {
    role: Role,
    label: Option<String>,
    value: Option<String>,
    enabled: bool,
    merge: bool,
    activate: Option<VoidCallback>,
}

impl Semantics {
    /// Creates a builder describing a [`Role::Group`].
    ///
    /// Finish it with [`Semantics::child`] or [`Semantics::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            spec: Spec {
                role: Role::Group,
                label: None,
                value: None,
                enabled: true,
                merge: true,
                activate: None,
            },
        }
    }

    /// Sets the role the child is announced as.
    #[inline]
    pub fn role(mut self, role: Role) -> Self {
        self.spec.role = role;
        self
    }

    /// Sets the name assistive technology announces.
    ///
    /// Without one, a merging node takes the text its child displays.
    #[inline]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.spec.label = Some(label.into());
        self
    }

    /// Sets the current value announced after the name.
    #[inline]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.spec.value = Some(value.into());
        self
    }

    /// Sets whether the child can be interacted with. The default is `true`.
    ///
    /// A disabled node is announced as dimmed and offers no activation.
    #[inline]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.spec.enabled = enabled;
        self
    }

    /// Sets whether the child's own nodes fold into this one. The default is
    /// `true`.
    #[inline]
    pub fn merges(mut self, merge: bool) -> Self {
        self.spec.merge = merge;
        self
    }

    /// Sets what activating the node — a screen reader's "press" — runs.
    ///
    /// Activation is offered only while the node is [enabled](Semantics::enabled).
    #[inline]
    pub fn on_activate(mut self, on_activate: impl Into<VoidCallback>) -> Self {
        self.spec.activate = Some(on_activate.into());
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Semantics<W> {
        Semantics { child, spec: self.spec }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for Semantics {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> Widget for Semantics<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawSemantics { spec: self.spec, child: self.child.to_element(ctx) }.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Semantics"
    }
}

impl<W> crate::PortableWidget for Semantics<W> {}

struct RawSemantics {
    spec: Spec,
    child: AnyElement,
}

impl Rebuildable for RawSemantics {}

impl EventElement for RawSemantics {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    fn semantics(&self, id: NodeId) -> Option<SemanticNode> {
        let spec = &self.spec;
        let mut node = SemanticNode::new(id, spec.role).enabled(spec.enabled);
        if let Some(label) = &spec.label {
            node = node.with_name(label.as_str());
        }
        if let Some(value) = &spec.value {
            node = node.with_value(value.as_str());
        }
        if spec.enabled && spec.activate.is_some() {
            node = node.with_action(SemanticAction::Activate);
        }
        Some(if spec.merge { node.merge() } else { node })
    }

    fn perform_semantic_action(&self, action: &SemanticAction) -> bool {
        match (&self.spec.activate, action) {
            (Some(activate), SemanticAction::Activate) if self.spec.enabled => {
                activate.execute(());
                true
            }
            _ => false,
        }
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Drawable for RawSemantics {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }
}

impl LayoutElement for RawSemantics {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl VisitorElement for RawSemantics {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Semantics"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_accessibility::SemanticBehavior;

    use super::*;

    struct Leaf;

    impl VisitorElement for Leaf {
        fn debug_name(&self) -> &'static str {
            "Leaf"
        }
    }

    impl EventElement for Leaf {}
    impl LayoutElement for Leaf {}
    impl Rebuildable for Leaf {}

    impl Drawable for Leaf {
        fn update(&self, _ctx: &BuildContext) {}
    }

    fn raw(semantics: Semantics) -> RawSemantics {
        RawSemantics { spec: semantics.spec, child: Leaf.boxed() }
    }

    #[test]
    fn publishes_the_described_node_and_merges_its_child_by_default() {
        let element = raw(Semantics::new().role(Role::Button).label("Save").value("on"));

        let node = element.semantics(NodeId::new(3)).expect("a semantics wrapper describes itself");

        assert_eq!(node.id(), NodeId::new(3));
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.name(), Some("Save"));
        assert_eq!(node.value(), Some("on"));
        assert_eq!(node.behavior(), SemanticBehavior::Merge);
        assert!(node.state().enabled());
    }

    #[test]
    fn merging_can_be_turned_off() {
        let element = raw(Semantics::new().merges(false));

        let node = element.semantics(NodeId::new(1)).unwrap();

        assert_eq!(node.behavior(), SemanticBehavior::Normal);
    }

    #[test]
    fn activation_is_offered_and_runs_only_while_enabled() {
        let count = Rc::new(Cell::new(0));
        let counter = count.clone();
        let enabled = raw(
            Semantics::new().on_activate(move || counter.set(counter.get() + 1)),
        );

        assert_eq!(
            enabled.semantics(NodeId::new(1)).unwrap().actions(),
            [SemanticAction::Activate]
        );
        assert!(enabled.perform_semantic_action(&SemanticAction::Activate));
        assert!(!enabled.perform_semantic_action(&SemanticAction::Dismiss));
        assert_eq!(count.get(), 1);

        let counter = count.clone();
        let disabled = raw(
            Semantics::new()
                .enabled(false)
                .on_activate(move || counter.set(counter.get() + 1)),
        );

        let node = disabled.semantics(NodeId::new(1)).unwrap();
        assert!(node.actions().is_empty());
        assert!(!node.state().enabled());
        assert!(!disabled.perform_semantic_action(&SemanticAction::Activate));
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn a_node_with_nothing_to_run_offers_no_action() {
        let element = raw(Semantics::new().role(Role::Group));

        assert!(element.semantics(NodeId::new(1)).unwrap().actions().is_empty());
        assert!(!element.perform_semantic_action(&SemanticAction::Activate));
    }

    #[test]
    fn it_does_not_take_part_in_event_routing() {
        let element = raw(Semantics::new());

        assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
        assert!(!element.on_event(&ElementEvent::FocusGained).is_consumed());
    }
}
