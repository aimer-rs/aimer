use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_container::{Container, ZeroSizedBox};
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_macro::Rebuildable;
use aimer_space::Alignment;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, LayoutElement,
    RequiredChild, VisitorElement, Widget,
};

use crate::ModalAnimation;
use crate::animated_content::wrap_animated_content;
use crate::host::{self, ModalHandle, ModalId, ModalTimeline};
use crate::paint::{contains, overlay_child_context};

/// Displays content above the entire application render tree.
///
/// Complete the builder with [`Modal::child`], then call [`Modal::show`] from a
/// callback to present it immediately through the framework-level overlay.
/// `AimerApp` installs the required host automatically.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::curves;
/// use aimer_container::SizedBox;
/// use aimer_modal::{Modal, ModalAnimation};
///
/// let handle =
///     Modal::new().animation(ModalAnimation::new().enter_duration(Duration::from_millis(200))
///                                                 .enter_curve(curves::EASE_OUT))
///                 .child(SizedBox::new().width(320).height(180))
///                 .show();
///
/// handle.dismiss();
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_modal::Modal", schema_only)]
pub struct Modal<W = RequiredChild> {
    #[portable_child]
    child: W,
    #[portable_skip]
    barrier_color: Color,
    #[portable_skip]
    alignment: Alignment,
    #[portable_skip]
    animation: Option<ModalAnimation>,
    #[portable_skip]
    barrier_dismissible: bool,
    #[portable_skip]
    escape_dismissible: bool,
}

impl Default for Modal {
    fn default() -> Self {
        Self::new()
    }
}

impl Modal {
    /// Creates a centered modal with a 45%-opaque black barrier.
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            barrier_color: Color::BLACK.with_opacity(115),
            alignment: Alignment::MidCenter,
            animation: None,
            barrier_dismissible: true,
            escape_dismissible: true,
        }
    }

    /// Sets the viewport-wide barrier color.
    pub fn barrier_color(mut self, barrier_color: Color) -> Self {
        self.barrier_color = barrier_color;
        self
    }

    /// Sets the content alignment within the viewport.
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Enables a paint-only enter and exit transition.
    pub fn animation(mut self, animation: ModalAnimation) -> Self {
        self.animation = Some(animation);
        self
    }

    /// Controls whether pressing outside the content dismisses the modal.
    pub fn barrier_dismissible(mut self, dismissible: bool) -> Self {
        self.barrier_dismissible = dismissible;
        self
    }

    /// Controls whether a pressed Escape key dismisses the modal.
    pub fn escape_dismissible(mut self, dismissible: bool) -> Self {
        self.escape_dismissible = dismissible;
        self
    }

    /// Attaches the required modal content and completes this builder.
    pub fn child<W: Widget>(self, child: W) -> Modal<W> {
        Modal {
            child,
            barrier_color: self.barrier_color,
            alignment: self.alignment,
            animation: self.animation,
            barrier_dismissible: self.barrier_dismissible,
            escape_dismissible: self.escape_dismissible,
        }
    }

    /// Attaches and erases the required modal content.
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Modal<W> {
    /// Presents this modal through the application-wide host immediately.
    ///
    /// Calls made before the first application frame are queued and presented
    /// as soon as the root host is built.
    pub fn show(self) -> ModalHandle {
        let animation = self.animation.clone();
        host::show(
            animation,
            Box::new(move |ctx, id, timeline| self.to_raw_element(ctx, Some(id), timeline)),
        )
    }

    fn to_raw_element(
        self,
        ctx: &BuildContext,
        id: Option<ModalId>,
        timeline: Rc<RefCell<ModalTimeline>>,
    ) -> AnyElement {
        let barrier = Container::new()
            .color(self.barrier_color)
            .child(ZeroSizedBox)
            .to_element(ctx);
        let barrier = wrap_animated_content(
            barrier,
            (self.barrier_color.to_rgba().3 != 0)
                .then(|| self.animation.clone())
                .flatten(),
            timeline.clone(),
            false,
        );
        let child = wrap_animated_content(
            self.child.to_element(ctx),
            self.animation.clone(),
            timeline.clone(),
            true,
        );
        RawModal {
            barrier,
            child,
            alignment: self.alignment,
            id,
            barrier_dismissible: self.barrier_dismissible,
            escape_dismissible: self.escape_dismissible,
            child_bounds: RefCell::new(None),
        }
        .boxed()
    }

    #[cfg(test)]
    pub(crate) fn animation_config(&self) -> Option<ModalAnimation> {
        self.animation.clone()
    }

    #[cfg(test)]
    pub(crate) fn alignment_value(&self) -> Alignment {
        self.alignment
    }

    #[cfg(test)]
    pub(crate) fn barrier_color_value(&self) -> Color {
        self.barrier_color
    }
}

impl<W: Widget + 'static> Widget for Modal<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.to_raw_element(
            ctx,
            None,
            Rc::new(RefCell::new(ModalTimeline::new_static())),
        )
    }

    fn debug_name(&self) -> &'static str {
        "Modal"
    }
}

#[derive(Rebuildable)]
struct RawModal {
    barrier: AnyElement,
    child: AnyElement,
    alignment: Alignment,
    id: Option<ModalId>,
    barrier_dismissible: bool,
    escape_dismissible: bool,
    child_bounds: RefCell<Option<(Vec2d, Vec2d)>>,
}

impl Drawable for RawModal {
    fn update(&self, ctx: &BuildContext) {
        self.barrier.update(ctx);
        let child_size = self.child.computed_size(ctx);
        let (offset_x, offset_y) = alignment_offset(self.alignment, ctx.parent_size, child_size);
        let child_ctx = overlay_child_context(
            ctx,
            child_size,
            Vec2d {
                x: offset_x,
                y: offset_y,
            },
            &self.child_bounds,
        );
        ctx.canvas.save();
        ctx.canvas.translate(Vec2d {
            x: offset_x,
            y: offset_y,
        });
        self.child.update(&child_ctx);
        ctx.canvas.restore();
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        if child_index != 1 || !std::ptr::eq(child, self.child.as_ref()) {
            return None;
        }
        self.retained_child_layout(ctx)
            .map(|(_, _, child_context)| child_context)
    }

    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        if child_index != 1 || !std::ptr::eq(child, self.child.as_ref()) {
            return None;
        }
        let (size, offset, _) = self.retained_child_layout(ctx)?;
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(
                offset.x / ctx.scale,
                offset.y / ctx.scale,
                size.width / ctx.scale,
                size.height / ctx.scale,
            ),
            None,
        ))
    }
}

impl EventElement for RawModal {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        let dismiss = match event {
            ElementEvent::PointerDown(pointer)
                if self.barrier_dismissible && !self.contains_child(pointer.pos) =>
            {
                true
            }
            ElementEvent::KeyInput {
                key: NamedKey::Escape,
                action: KeyAction::Pressed,
                ..
            } if self.escape_dismissible => true,
            _ => false,
        };
        if dismiss && let Some(id) = self.id {
            host::dismiss(id);
        }
        EventResult::consumed()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.barrier.as_ref());
        visitor(self.child.as_ref());
    }
}

impl RawModal {
    fn contains_child(&self, position: Vec2d) -> bool {
        contains(&self.child_bounds, position)
    }

    fn retained_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
    ) -> Option<(ResolvedSize, Vec2d, BuildContext<'a>)> {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return None;
        }

        let child_size = self.child.computed_size(ctx);
        if !child_size.width.is_finite()
            || !child_size.height.is_finite()
            || child_size.width < 0.0
            || child_size.height < 0.0
        {
            return None;
        }
        let (offset_x, offset_y) = alignment_offset(self.alignment, ctx.parent_size, child_size);
        if !offset_x.is_finite() || !offset_y.is_finite() {
            return None;
        }
        let offset = Vec2d {
            x: offset_x,
            y: offset_y,
        };

        let mut child_context = ctx.clone();
        child_context.parent_size = child_size;
        child_context.parent_pos = Vec2d {
            x: ctx.parent_pos.x + offset.x,
            y: ctx.parent_pos.y + offset.y,
        };
        child_context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: child_size.width,
            max_height: child_size.height,
        };
        child_context.visible_rect = ctx
            .visible_rect
            .map(|(x, y, width, height)| (x - offset.x, y - offset.y, width, height));
        Some((child_size, offset, child_context))
    }
}

impl LayoutElement for RawModal {
    fn size(&self) -> Option<Size> {
        None
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }
}

impl VisitorElement for RawModal {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.barrier.as_ref());
        visitor(self.child.as_ref());
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        visitor(0, self.barrier.as_ref());
        visitor(1, self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Modal"
    }
}

fn alignment_offset(alignment: Alignment, parent: ResolvedSize, child: ResolvedSize) -> (f32, f32) {
    let remaining_width = (parent.width - child.width).max(0.0);
    let remaining_height = (parent.height - child.height).max(0.0);
    let x = match alignment {
        Alignment::TopLeft | Alignment::MidLeft | Alignment::BotLeft => 0.0,
        Alignment::TopCenter | Alignment::MidCenter | Alignment::BotCenter => remaining_width / 2.0,
        Alignment::TopRight | Alignment::MidRight | Alignment::BotRight => remaining_width,
    };
    let y = match alignment {
        Alignment::TopLeft | Alignment::TopCenter | Alignment::TopRight => 0.0,
        Alignment::MidLeft | Alignment::MidCenter | Alignment::MidRight => remaining_height / 2.0,
        Alignment::BotLeft | Alignment::BotCenter | Alignment::BotRight => remaining_height,
    };
    (x, y)
}
