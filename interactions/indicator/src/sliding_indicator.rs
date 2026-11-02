//! The widget that wires a controller, a track and a style around some content.

use std::rc::Rc;

use aimer_space::Stack;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, PortableWidget, RequiredChild, Widget};

use crate::controller::IndicatorController;
use crate::layer::IndicatorLayerWidget;
use crate::style::{AnyIndicatorStyle, IndicatorLayer, IndicatorStyle};
use crate::surface::IndicatorSurfaceElement;
use crate::track::{AnyIndicatorTrack, IndicatorTrack};

/// Marks the selected slot of its content with an indicator that slides.
///
/// The content is laid out as usual. The indicator is drawn in up to two layers
/// around it, as the [`IndicatorStyle`] decides: a *rest* layer beneath the
/// content and a *lift* layer over it. The [`IndicatorTrack`] says where the
/// slots are, and the [`IndicatorController`] says where the indicator is. The
/// content has to put its own items where the track expects them.
///
/// Moving the indicator never rebuilds the content or the layers: each frame
/// only presents the layers' render nodes at their new position, scale and
/// opacity.
///
/// Pass the controller in and keep it across rebuilds, since it owns the
/// motion. A `SlidingIndicator` without one starts a fresh controller each time
/// it is built, which can only ever show the first slot.
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_flex::Row;
/// use aimer_indicator::{IndicatorController, SlidingIndicator, indicator_styles, tracks};
///
/// let controller = IndicatorController::new(0.0);
/// let bar = SlidingIndicator::new()
///     .controller(controller.clone())
///     .track(tracks::row(3, 80.0, 48.0))
///     .style(indicator_styles::LIQUID_GLASS)
///     .child(
///         Row::new()
///             .add_child(SizedBox::new().width(80.0).height(48.0))
///             .add_child(SizedBox::new().width(80.0).height(48.0))
///             .add_child(SizedBox::new().width(80.0).height(48.0)),
///     );
///
/// // Later, when the selection changes:
/// controller.select(2.0);
/// ```
pub struct SlidingIndicator<W = RequiredChild> {
    controller: IndicatorController,
    track: AnyIndicatorTrack,
    style: AnyIndicatorStyle,
    on_select: Option<Rc<dyn Fn(usize)>>,
    child: W,
}

impl SlidingIndicator {
    /// Creates an indicator with the default liquid glass style, a single
    /// 48 by 32 slot, and a controller resting on the first slot.
    ///
    /// Finish the builder with [`SlidingIndicator::child`] or
    /// [`SlidingIndicator::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            controller: IndicatorController::default(),
            track: AnyIndicatorTrack::default(),
            style: AnyIndicatorStyle::default(),
            on_select: None,
            child: RequiredChild,
        }
    }

    /// Sets the controller that owns the indicator's motion.
    #[inline]
    pub fn controller(mut self, controller: IndicatorController) -> Self {
        self.controller = controller;
        self
    }

    /// Sets where the slots are.
    #[inline]
    pub fn track(mut self, track: impl IndicatorTrack) -> Self {
        self.track = track.into_any();
        self
    }

    /// Sets how the indicator looks.
    #[inline]
    pub fn style(mut self, style: impl IndicatorStyle) -> Self {
        self.style = style.into_any();
        self
    }

    /// Makes the indicator respond to the pointer, and sets the callback that
    /// receives the slot the user chose.
    ///
    /// Pressing the content lifts the indicator and brings it under the
    /// pointer, and dragging carries it along the track, kept within its slots,
    /// for as long as the pointer is held, wherever it goes. Letting go
    /// proposes the nearest slot, unless that is the slot it started on. The
    /// pointer gesture is only handled when this is set.
    ///
    /// The indicator is *controlled*: nothing moves it to the chosen slot but
    /// the caller passing it back through the controller, with
    /// [`IndicatorController::select`]. A choice that is not passed back leaves
    /// the indicator where the drag let go until that happens.
    #[inline]
    pub fn on_select(mut self, on_select: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Rc::new(on_select));
        self
    }

    /// Attaches the required content and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> SlidingIndicator<W> {
        SlidingIndicator {
            controller: self.controller,
            track: self.track,
            style: self.style,
            on_select: self.on_select,
            child,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for SlidingIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> Widget for SlidingIndicator<W> {
    fn debug_name(&self) -> &'static str {
        "SlidingIndicator"
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        use crate::track::IndicatorTrack as _;

        let slot = self.track.slot_size();
        let layer = |layer: IndicatorLayer, visual: AnyWidget| IndicatorLayerWidget {
            controller: self.controller.clone(),
            track: self.track.clone(),
            style: self.style.clone(),
            layer,
            child: visual,
        };

        // Layers are painted in the order they are added: the rest layer under
        // the content, the lift layer over it, so that a lens magnifies the
        // content it passes over.
        let mut stack = Stack::new();
        if let Some(visual) = self.style.layer(IndicatorLayer::Rest, slot) {
            stack = stack.add_child(layer(IndicatorLayer::Rest, visual));
        }
        stack = stack.add_child(self.child);
        if let Some(visual) = self.style.layer(IndicatorLayer::Lift, slot) {
            stack = stack.add_child(layer(IndicatorLayer::Lift, visual));
        }
        let stack = stack.to_element(ctx);
        match self.on_select {
            Some(on_select) => IndicatorSurfaceElement::new(
                stack,
                self.controller,
                self.track,
                on_select,
            )
            .boxed(),
            None => stack,
        }
    }
}

impl<W: Widget + 'static> PortableWidget for SlidingIndicator<W> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::indicator_styles;
    use crate::test_support::{Slot, context};
    use crate::track::tracks;

    /// The debug names of the stack's children, in paint order.
    fn children(element: &AnyElement) -> Vec<&'static str> {
        let mut names = Vec::new();
        element.visit_children(&mut |child| names.push(child.debug_name()));
        names
    }

    #[test]
    fn liquid_glass_paints_the_pill_under_the_content_and_the_lens_over_it() {
        let element = SlidingIndicator::new()
            .track(tracks::row(3, 10.0, 10.0))
            .style(indicator_styles::LIQUID_GLASS)
            .child(Slot)
            .to_element(&context());

        assert_eq!(
            children(&element),
            ["IndicatorLayerElement", "SlotElement", "IndicatorLayerElement"]
        );
    }

    #[test]
    fn a_flat_pill_has_no_layer_over_the_content() {
        let element = SlidingIndicator::new()
            .track(tracks::row(3, 10.0, 10.0))
            .style(indicator_styles::FLAT_PILL)
            .child(Slot)
            .to_element(&context());

        assert_eq!(children(&element), ["IndicatorLayerElement", "SlotElement"]);
    }

    #[test]
    fn an_invalid_style_leaves_only_the_content() {
        let element = SlidingIndicator::new()
            .style(indicator_styles::LIQUID_GLASS.lift_scale(f32::NAN))
            .child(Slot)
            .to_element(&context());

        assert_eq!(children(&element), ["SlotElement"]);
    }

    #[test]
    fn a_default_indicator_is_a_valid_widget() {
        let element = SlidingIndicator::default().child(Slot).to_element(&context());
        assert_eq!(children(&element).len(), 3);
    }
}
