//! Confining keyboard focus to one part of the widget tree.
//!
//! A dialog, a drawer or a wizard step is a *mode*: while it is up, `Tab` must
//! cycle inside it and a field behind it must not be reachable at all. Wrapping
//! that region in a [`FocusScope`] is the whole of the declaration — see the
//! type's documentation for the semantics and for what happens when the scope
//! goes away.

use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;

use crate::base::BuildContext;
use crate::components::event_element::{EventElement, EventResult, EventTreeRole};
use crate::components::layout_element::LayoutElement;
use crate::components::rebuildable::Rebuildable;
use crate::components::visitor_element::VisitorElement;
use crate::{AnyElement, AnyWidget, Drawable, Element, RequiredChild, Widget};

/// Confines keyboard focus to its subtree while it traps.
///
/// A focus scope changes nothing about layout or painting: it wraps a region and
/// declares that, while it is in the tree, only the focusable targets *inside*
/// it may own the keyboard. `Tab` and `Shift-Tab` therefore cycle within the
/// scope instead of walking out into the application behind it, and no element
/// outside it can be given focus — which is exactly what a dialog rendered
/// inline needs, and what distinguishes a mode from an ordinary panel.
///
/// Entering a scope remembers whatever owned focus outside it, and leaving the
/// scope — dismissed, navigated away from, or simply rebuilt out of the tree —
/// gives focus back to it if it is still there. Nothing has to be restored by
/// hand.
///
/// Scopes nest: the innermost trapping scope is the one that confines focus, so
/// a dialog inside a drawer traps within the dialog, and closing it hands the
/// keyboard back to the drawer.
///
/// A scope that does not [trap](FocusScope::traps) is inert, which is what makes
/// the flag worth having: a widget that is sometimes a mode — a panel that
/// becomes modal on a small screen — keeps one build path and flips a `bool`.
///
/// Overlays presented through `aimer_modal` need no scope. Their content is not
/// part of the tree they cover, so they confine focus with an
/// [`aimer_focus::FocusTrap`] instead; the effect for the application underneath
/// is the same.
///
/// # Examples
///
/// ```
/// use aimer_widget::FocusScope;
///
/// struct Dialog;
/// # impl aimer_widget::PortableWidget for Dialog {}
/// # impl aimer_widget::Widget for Dialog {
/// #     fn to_element(self, _ctx: &aimer_widget::base::BuildContext) -> aimer_widget::AnyElement {
/// #         unreachable!("this example only builds the widget")
/// #     }
/// # }
///
/// // While this is in the tree, Tab cannot leave the dialog.
/// let modal = FocusScope::new().child(Dialog);
///
/// // The same region, not confining anything.
/// let inline = FocusScope::new().traps(false).child(Dialog);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_widget::FocusScope")]
pub struct FocusScope<W = RequiredChild> {
    #[portable_child]
    child: W,
    #[portable_optional]
    traps: bool,
}

impl FocusScope {
    /// Creates a trapping scope builder.
    ///
    /// Finish the builder with [`FocusScope::child`] or
    /// [`FocusScope::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            traps: true,
        }
    }

    /// Sets whether the scope confines focus to its subtree.
    ///
    /// The default is `true`. A scope built with `false` behaves as if it were
    /// not there.
    #[inline]
    pub fn traps(mut self, traps: bool) -> Self {
        self.traps = traps;
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> FocusScope<W> {
        FocusScope {
            child,
            traps: self.traps,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for FocusScope {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> Widget for FocusScope<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawFocusScope {
            traps: self.traps,
            child: self.child.to_element(ctx),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "FocusScope"
    }
}

struct RawFocusScope {
    child: AnyElement,
    traps: bool,
}

impl Rebuildable for RawFocusScope {}

impl EventElement for RawFocusScope {
    fn event_tree_role(&self) -> EventTreeRole {
        // Focus trapping is collected from the structural tree independently.
        EventTreeRole::Transparent
    }

    #[inline]
    fn traps_focus(&self) -> bool {
        self.traps
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Drawable for RawFocusScope {
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

impl LayoutElement for RawFocusScope {
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

impl VisitorElement for RawFocusScope {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "FocusScope"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestChild;

    impl VisitorElement for TestChild {
        fn debug_name(&self) -> &'static str {
            "TestChild"
        }
    }

    impl EventElement for TestChild {}
    impl LayoutElement for TestChild {}
    impl Rebuildable for TestChild {}

    impl Drawable for TestChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    #[test]
    fn focus_scope_is_transparent_without_changing_focus_trapping() {
        let child = TestChild.boxed();
        let scope = RawFocusScope {
            child,
            traps: true,
        };

        assert_eq!(scope.event_tree_role(), EventTreeRole::Transparent);
        assert!(scope.traps_focus());
        assert!(!scope.on_event(&ElementEvent::FocusGained).is_consumed());
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use super::FocusScope;
    use crate::portable::PortableWidgetSchema;
    use crate::RequiredChild;

    #[test]
    fn focus_scope_schema_exposes_its_trap_property_and_required_child() {
        let schema = <FocusScope<RequiredChild> as PortableWidgetSchema>::SCHEMA;

        assert_eq!(
            schema.widget().canonical_name(),
            "aimer.widget:aimer_widget::FocusScope"
        );
        assert_eq!(schema.properties().len(), 1);
        assert_eq!(
            schema.properties()[0].canonical_name(),
            "aimer.property:aimer_widget::FocusScope:traps"
        );
        assert_eq!(schema.children(), aimer_anteros::ChildCardinality::exactly(1));
    }
}
