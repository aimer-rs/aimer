use aimer_attribute::{Dimension, Size};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable,
    VisitorElement, Widget,
};
/// A leaf widget that occupies no space and paints nothing.
///
/// `ZeroSizedBox` has no child and can be constructed with [`ZeroSizedBox::new`]
/// or by instantiating the unit struct directly. It is useful as an empty
/// placeholder where a valid [`Widget`] or element is required, and its layout
/// size remains the default zero size.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_container::single_child::ZeroSizedBox")]
pub struct ZeroSizedBox;

impl ZeroSizedBox {
    /// Creates an empty widget with zero layout size.
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    #[inline]
    pub fn boxed() -> Box<Self> {
        Box::new(Self)
    }
}

impl Drawable for ZeroSizedBox {
    fn update(&self, _: &BuildContext) {}

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }
}

impl VisitorElement for ZeroSizedBox {
    fn debug_name(&self) -> &'static str {
        "ZeroSizedBox"
    }
}

impl EventElement for ZeroSizedBox {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl LayoutElement for ZeroSizedBox {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        true
    }

    fn size(&self) -> Option<Size> {
        Some(Size {
            width: Dimension::Px(0.0),
            height: Dimension::Px(0.0),
        })
    }
}

impl Rebuildable for ZeroSizedBox {}

impl Widget for ZeroSizedBox {
    fn to_element(self, _: &BuildContext) -> AnyElement {
        Element::boxed(ZeroSizedBox)
    }
}

#[cfg(test)]
mod tests {
    use super::ZeroSizedBox;
    use aimer_widget::EventElement;

    #[test]
    fn zero_sized_box_is_transparent_to_event_routing() {
        assert_eq!(
            ZeroSizedBox.event_tree_role(),
            aimer_widget::EventTreeRole::Transparent
        );
    }
}
