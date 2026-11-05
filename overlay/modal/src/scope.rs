//! What a modal inherits from where it is requested.

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventTreeRole, InheritedScope, LayoutElement,
    Rebuildable, VisitorElement,
};

/// The providers, and every other inherited value, in scope at one place in the
/// application, kept to be given to a [`crate::Modal`].
///
/// A modal is presented by the window's overlay, above the whole application and
/// outside of everything the application provides, so what it shows cannot find
/// the theme, the store or the router its requester can. Take the scope in the
/// `build` of the widget that requests the modal, with [`ModalScope::capture`],
/// keep it for the callback that presents it, and hand it to the modal with
/// [`crate::Modal::scope`]: the content then reads, and follows, exactly what a
/// child of that widget would.
///
/// A scope is cheap to keep and to clone. What it holds is shared with the
/// application, a provider's handle being the handle of that provider, so the
/// modal sees the provider's value as it is when it reads it, not as it was when
/// the scope was taken.
#[derive(Clone, Default)]
pub struct ModalScope(InheritedScope);

impl ModalScope {
    /// The scope at `ctx`.
    pub fn capture(ctx: &BuildContext) -> Self {
        Self(InheritedScope::capture(ctx))
    }

    /// Runs `callback` with the scope in place over `ctx`'s own, and puts `ctx`
    /// back as it was afterwards: for building what a modal, or the overlay,
    /// will show, where the context at hand is the overlay's.
    pub fn with<'a, R>(
        &self,
        ctx: &BuildContext<'a>,
        callback: impl FnOnce(&BuildContext<'a>) -> R,
    ) -> R {
        self.0.with(ctx, callback)
    }

    pub(crate) fn wrap(&self, child: AnyElement) -> AnyElement {
        ScopedElement {
            child,
            scope: self.clone(),
        }
        .boxed()
    }
}

/// Puts a [`ModalScope`] over the content of a modal each time the content is
/// visited, since a rebuild or a repaint of a part of it starts from the
/// overlay, which has none of the application's providers.
struct ScopedElement {
    child: AnyElement,
    scope: ModalScope,
}

impl ScopedElement {
    fn scoped<'a, R>(
        &self,
        ctx: &BuildContext<'a>,
        callback: impl FnOnce(&BuildContext<'a>) -> R,
    ) -> R {
        self.scope.with(ctx, callback)
    }
}

impl VisitorElement for ScopedElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "ModalScope"
    }
}

impl Drawable for ScopedElement {
    fn update(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.update(ctx));
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, _ctx: &BuildContext) {}

    fn child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        Some(self.scope.0.fork(ctx))
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.sync_paint_geometry(ctx));
    }
}

impl LayoutElement for ScopedElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.layout(ctx))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.computed_size(ctx))
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.content_size(ctx))
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

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

impl EventElement for ScopedElement {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl Rebuildable for ScopedElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.rebuild_if_dirty(ctx));
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.scoped(ctx, callback);
    }

    fn is_carry_state(&self) -> bool {
        true
    }

    fn mark_needs_rebuild(&self) {
        self.child.mark_needs_rebuild();
    }
}
