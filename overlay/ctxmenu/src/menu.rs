//! The menu itself: an ordinary widget, presented through the modal host.

use std::rc::Rc;

use aimer_attribute::{Bounds, Vec2d};
use aimer_macro::PortableWidget;
use aimer_modal::{AnchorHandle, Floating, ModalAnimation, ModalHandle};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{AnyElement, AnyWidget, ChildBuilder, Widget};

use crate::dismiss::ContextMenuDismiss;
use crate::item::ContextMenuItem;
use crate::panel::RawContextMenuPanel;
use crate::portable::PortableMenuItems;
use crate::rows::ContextMenuRows;
use crate::shape::ContextMenuShape;
use crate::style::ContextMenuStyle;

/// A context menu: a styled panel of verbs, pinned to what it acts on.
///
/// `ContextMenu` is a normal widget. Presenting it with
/// [`show`](ContextMenu::show) hands it to [`aimer_modal::Floating`], which
/// places it against its anchor above every clipping ancestor, closes it on a
/// press outside or on `Escape`, and keeps it following an anchor that moves.
/// Nothing about it is special-cased for text: any widget that wants to offer
/// verbs about the thing under the pointer can open one.
///
/// Its content is its [`items`](ContextMenu::items) unless it is given a
/// [`child`](ContextMenu::child), which replaces them — the style's panel and
/// padding are drawn around either. Two shapes, chosen by the pointer that
/// asked (see [`ContextMenuShape`]), and every colour, radius, row height and
/// font in [`ContextMenuStyle`].
///
/// # Examples
///
/// The usual case: a few verbs, in the shape the pointer expects.
///
/// ```no_run
/// use aimer_attribute::Vec2d;
/// use aimer_ctxmenu::{ContextMenu, ContextMenuItem};
/// use aimer_events::pointer::PointerSource;
///
/// # fn demo(source: PointerSource, at: Vec2d) {
/// let handle = ContextMenu::for_source(source)
///     .at(at)
///     .items(vec![
///         ContextMenuItem::new("Copy").on_select(|| println!("copied")),
///         ContextMenuItem::new("Select All").on_select(|| println!("all")),
///     ])
///     .show();
///
/// handle.dismiss();
/// # }
/// ```
///
/// Anything else, by giving it a child instead of items:
///
/// ```no_run
/// use aimer_container::SizedBox;
/// use aimer_ctxmenu::{ContextMenu, ContextMenuShape, ContextMenuStyle};
/// use aimer_style::BoxDecoration;
/// use aimer_widget::base::Color;
///
/// let _handle = ContextMenu::new()
///     .shape(ContextMenuShape::List)
///     .style(
///         ContextMenuStyle::list().panel(
///             BoxDecoration::new()
///                 .background_color(Color::Rgba(24, 24, 27, 250))
///                 .border_radius(12.0),
///         ),
///     )
///     .child(SizedBox::new().width(220).height(120))
///     .show();
/// ```
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_ctxmenu::ContextMenu",
    materializer = materialize_portable_context_menu
)]
pub struct ContextMenu {
    shape: ContextMenuShape,
    style: ContextMenuStyle,
    #[portable_skip]
    anchor: AnchorHandle,
    items: PortableMenuItems,
    #[portable_skip]
    on_select: Option<Rc<dyn Fn(usize)>>,
    #[portable_skip]
    dismiss: ContextMenuDismiss,
    dismiss_on_select: bool,
    barrier_color: Color,
    #[portable_skip]
    animation: Option<ModalAnimation>,
    /// Custom content, or `None` to build rows from the items. The row item
    /// callbacks remain native-only; custom content is the portable child.
    #[portable_child(optional)]
    child: Option<ChildBuilder>,
}

fn materialize_portable_context_menu(
    document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
    node: aimer_widget::portable::__anteros::WidgetNodeView<'_>,
    mut children: Vec<AnyWidget>,
) -> Result<AnyWidget, aimer_widget::portable::PortableMaterializeError> {
    if children.len() > 1 {
        return Err(
            aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                expected: 1,
                actual: children.len(),
            },
        );
    }

    let shape: ContextMenuShape = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenu:shape",
        ),
    )?;
    let style: ContextMenuStyle = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenu:style",
        ),
    )?;
    let items: PortableMenuItems = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenu:items",
        ),
    )?;
    let dismiss_on_select: bool = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenu:dismiss_on_select",
        ),
    )?;
    let barrier_color: Color = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenu:barrier_color",
        ),
    )?;

    let mut widget = ContextMenu::new()
        .shape(shape)
        .style(style)
        .items(items.into_vec())
        .dismiss_on_select(dismiss_on_select)
        .barrier_color(barrier_color);
    if let Some(child) = children.pop() {
        widget = widget.child(child);
    }
    Ok(widget.boxed())
}

impl Default for ContextMenu {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenu {
    /// Creates an empty menu in the default shape, anchored nowhere yet.
    #[inline]
    pub fn new() -> Self {
        Self {
            shape: ContextMenuShape::default(),
            style: ContextMenuStyle::default(),
            anchor: AnchorHandle::new(),
            items: PortableMenuItems::default(),
            on_select: None,
            dismiss: ContextMenuDismiss::new(),
            dismiss_on_select: true,
            barrier_color: Color::Transparent,
            animation: None,
            child: None,
        }
    }

    /// Creates a menu in the shape the pointer that asked for it expects.
    #[inline]
    pub fn for_source(source: aimer_events::pointer::PointerSource) -> Self {
        Self::new().shape(ContextMenuShape::for_source(source))
    }

    /// Sets the shape, and with it the default look of that shape.
    ///
    /// Call [`ContextMenu::style`] *after* this to keep a custom look.
    #[inline]
    pub fn shape(mut self, shape: ContextMenuShape) -> Self {
        self.shape = shape;
        self.style = ContextMenuStyle::for_shape(shape);
        self
    }

    /// Sets the look of the menu.
    #[inline]
    pub fn style(mut self, style: ContextMenuStyle) -> Self {
        self.style = style;
        self
    }

    /// Pins the menu to a point — where a right-click landed.
    #[inline]
    pub fn at(self, at: Vec2d) -> Self {
        self.around(Bounds::new(at.x, at.y, 0.0, 0.0))
    }

    /// Pins the menu clear of a rectangle — a selection, a chip, a row.
    #[inline]
    pub fn around(self, bounds: Bounds) -> Self {
        let anchor = AnchorHandle::new();
        anchor.set_bounds(bounds);
        self.anchor(anchor)
    }

    /// Pins the menu to a rectangle that keeps moving.
    ///
    /// The panel is re-placed on every frame, so a menu anchored to a selection
    /// being adjusted, or to a row inside a `Scrollable`, follows it.
    #[inline]
    pub fn anchor(mut self, anchor: AnchorHandle) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets the verbs, in the order they are drawn.
    #[inline]
    pub fn items(mut self, items: Vec<ContextMenuItem>) -> Self {
        self.items = PortableMenuItems::new(items);
        self
    }

    /// Appends one verb.
    #[inline]
    pub fn item(mut self, item: ContextMenuItem) -> Self {
        self.items.push(item);
        self
    }

    /// Sets what happens when a verb is chosen, by its index.
    ///
    /// This runs *after* the chosen item's own action, so a menu may use either
    /// or both.
    #[inline]
    pub fn on_select(mut self, on_select: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Rc::new(on_select));
        self
    }

    /// Controls whether choosing a verb closes the menu.
    ///
    /// Closing is the default, and is what `Copy` wants. A verb that only
    /// *reshapes* what the menu acts on — `Select All` — reads better with the
    /// menu left standing.
    #[inline]
    pub fn dismiss_on_select(mut self, dismiss_on_select: bool) -> Self {
        self.dismiss_on_select = dismiss_on_select;
        self
    }

    /// Sets the colour washed over the rest of the window while the menu is
    /// open.
    ///
    /// Transparent by default: a context menu is about something the user can
    /// still see.
    #[inline]
    pub fn barrier_color(mut self, barrier_color: Color) -> Self {
        self.barrier_color = barrier_color;
        self
    }

    /// Gives the menu an enter and exit transition.
    #[inline]
    pub fn animation(mut self, animation: ModalAnimation) -> Self {
        self.animation = Some(animation);
        self
    }

    /// Replaces the rows with content of one's own.
    ///
    /// The style's panel and padding are still drawn around it, and the content
    /// closes the menu through [`ContextMenu::dismiss_handle`].
    #[inline]
    pub fn child<W: Widget + 'static>(mut self, child: W) -> Self {
        self.child = Some(ChildBuilder::from_widget(child));
        self
    }

    /// The handle that closes this menu once it is open.
    ///
    /// Custom content needs it: a button inside a menu can capture the handle
    /// while the menu is still being described and close the menu from its own
    /// callback.
    #[inline]
    pub fn dismiss_handle(&self) -> ContextMenuDismiss {
        self.dismiss.clone()
    }

    /// The shape the menu is drawn in.
    #[inline]
    pub const fn shape_value(&self) -> ContextMenuShape {
        self.shape
    }

    /// The look of the menu.
    #[inline]
    pub fn style_value(&self) -> ContextMenuStyle {
        self.style.clone()
    }

    /// The rectangle the menu is pinned to.
    #[inline]
    pub fn anchor_value(&self) -> AnchorHandle {
        self.anchor.clone()
    }

    /// How many verbs the menu offers.
    #[inline]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Whether the menu was given content of its own instead of rows.
    #[inline]
    pub fn has_child(&self) -> bool {
        self.child.is_some()
    }

    /// Opens the menu above the whole application and returns its handle.
    ///
    /// The panel is placed against the anchor by [`aimer_modal::Floating`], so
    /// it is never clipped by an ancestor, and it closes on a press outside it
    /// or on `Escape` without the caller routing a single event.
    pub fn show(self) -> ModalHandle {
        let dismiss = self.dismiss.clone();
        let anchor = self.anchor.clone();
        let placement = self.shape.placement(self.style.gap);
        let barrier_color = self.barrier_color;
        let animation = self.animation.clone();

        let mut floating = Floating::new()
            .anchor(anchor)
            .placement(placement)
            .viewport_margin(self.style.screen_margin)
            .barrier_color(barrier_color);
        if let Some(animation) = animation {
            floating = floating.animation(animation);
        }
        let handle = floating.child(self).show();
        dismiss.claim(handle.clone());
        handle
    }

    /// The rows the items produce, in this menu's shape and look.
    fn rows(&self) -> ContextMenuRows {
        let mut rows = ContextMenuRows::new()
            .shape(self.shape)
            .style(self.style.clone())
            .items(self.items.clone().into_vec())
            .dismiss_with(self.dismiss.clone())
            .dismiss_on_select(self.dismiss_on_select);
        if let Some(on_select) = &self.on_select {
            let on_select = Rc::clone(on_select);
            rows = rows.on_select(move |index| on_select(index));
        }
        rows
    }
}

impl Widget for ContextMenu {
    fn to_element(mut self, ctx: &BuildContext) -> AnyElement {
        let style = self.style.clone();
        let content = match self.child.take() {
            Some(child) => child.to_element(ctx),
            None => self.rows().to_element(ctx),
        };
        RawContextMenuPanel::element(content, style)
    }

    fn debug_name(&self) -> &'static str {
        "ContextMenu"
    }
}

#[cfg(test)]
mod tests {
    use aimer_container::ZeroSizedBox;
    use aimer_events::pointer::PointerSource;
    use aimer_modal::{FloatingAlign, FloatingSide};

    use super::*;

    #[test]
    fn a_touch_menu_is_a_pill_and_a_mouse_menu_is_a_list() {
        assert_eq!(
            ContextMenu::for_source(PointerSource::Touch).shape_value(),
            ContextMenuShape::Pill
        );
        assert_eq!(
            ContextMenu::for_source(PointerSource::Mouse).shape_value(),
            ContextMenuShape::List
        );
    }

    #[test]
    fn choosing_a_shape_brings_that_shapes_look_with_it() {
        let pill = ContextMenu::new().shape(ContextMenuShape::Pill);
        let list = ContextMenu::new().shape(ContextMenuShape::List);

        assert_eq!(
            pill.style_value().row_height,
            ContextMenuStyle::pill().row_height
        );
        assert_eq!(
            list.style_value().row_height,
            ContextMenuStyle::list().row_height
        );
    }

    #[test]
    fn a_custom_look_survives_being_given_a_child() {
        let menu = ContextMenu::new()
            .shape(ContextMenuShape::List)
            .style(ContextMenuStyle::list().row_height(44.0))
            .child(ZeroSizedBox);

        assert!(menu.has_child());
        assert_eq!(menu.style_value().row_height, 44.0);
        assert_eq!(menu.shape_value(), ContextMenuShape::List);
    }

    #[test]
    fn a_menu_shows_its_items_until_it_is_given_a_child() {
        let items = ContextMenu::new().item(ContextMenuItem::new("Copy"));

        assert_eq!(items.item_count(), 1);
        assert!(!items.has_child(), "so the rows are the content");
        assert!(items.child(ZeroSizedBox).has_child());
    }

    #[test]
    fn a_point_anchor_is_the_click_itself() {
        let menu = ContextMenu::new().at(Vec2d { x: 40.0, y: 60.0 });

        assert_eq!(
            menu.anchor_value().bounds(),
            Some(Bounds::new(40.0, 60.0, 0.0, 0.0))
        );
    }

    #[test]
    fn a_rectangle_anchor_is_kept_clear_of() {
        let menu = ContextMenu::new().around(Bounds::new(10.0, 20.0, 100.0, 16.0));

        assert_eq!(
            menu.anchor_value().bounds(),
            Some(Bounds::new(10.0, 20.0, 100.0, 16.0))
        );
    }

    #[test]
    fn a_tracked_anchor_is_followed_rather_than_copied() {
        let anchor = AnchorHandle::new();
        let menu = ContextMenu::new().anchor(anchor.clone());

        anchor.set_bounds(Bounds::new(1.0, 2.0, 3.0, 4.0));

        assert_eq!(
            menu.anchor_value().bounds(),
            Some(Bounds::new(1.0, 2.0, 3.0, 4.0)),
            "the menu reads the handle, it does not snapshot it"
        );
    }

    #[test]
    fn the_shape_decides_where_the_panel_hangs() {
        let pill = ContextMenu::new().shape(ContextMenuShape::Pill);
        let placement = pill.shape_value().placement(pill.style_value().gap);

        assert_eq!(placement.side_value(), FloatingSide::Top);
        assert_eq!(placement.align_value(), FloatingAlign::Center);

        let list = ContextMenu::new().shape(ContextMenuShape::List);
        let placement = list.shape_value().placement(list.style_value().gap);

        assert_eq!(placement.side_value(), FloatingSide::Bottom);
        assert_eq!(placement.gap_value(), 0.0);
    }

    #[test]
    fn showing_a_menu_claims_its_dismissal_handle() {
        let menu = ContextMenu::new()
            .at(Vec2d { x: 10.0, y: 10.0 })
            .item(ContextMenuItem::new("Copy"));
        let dismiss = menu.dismiss_handle();

        assert!(!dismiss.is_claimed(), "nothing is open yet");

        let handle = menu.show();

        assert!(
            dismiss.is_claimed(),
            "so a row chosen inside the menu can close it"
        );
        assert!(handle.dismiss());
        assert!(!dismiss.dismiss(), "and it closes only once");
    }

    #[test]
    fn a_menu_with_custom_content_is_presented_the_same_way() {
        let menu = ContextMenu::new()
            .at(Vec2d { x: 10.0, y: 10.0 })
            .child(ZeroSizedBox);
        let dismiss = menu.dismiss_handle();

        let handle = menu.show();

        assert!(dismiss.is_claimed());
        assert!(handle.dismiss());
    }

    #[test]
    fn context_menu_publishes_a_derived_optional_child_schema() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <ContextMenu as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::new(0, 1)
        );
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use aimer_container::ZeroSizedBox;
    use aimer_widget::portable::{
        __anteros::WidgetDocumentView, PortableBuildContext, PortableLimits, PortableNativeWidget,
        PortableWidgetLimits, PortableWidgetSchema, SourceFingerprint, StableId128,
        linked_portable_native_widget_registrations,
    };
    use aimer_widget::{PortableWidget, Widget};

    use super::{ContextMenu, ContextMenuItem, ContextMenuShape, ContextMenuStyle};

    fn source(value: u8) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([value; 16]))
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            7,
            11,
            PortableWidgetLimits::new(16, 32, 16, 16, 2_048, 16_384).with_max_blob_bytes(8_192),
            PortableLimits::new(16, 512, 2_048, 8_192, 16_384),
        )
        .unwrap()
    }

    #[test]
    fn menu_lower_round_trips_style_items_policy_and_optional_child() {
        let mut context = context();
        let root = ContextMenu::new()
            .shape(ContextMenuShape::List)
            .style(ContextMenuStyle::list().row_height(32.0))
            .items(vec![ContextMenuItem::new("Copy")])
            .dismiss_on_select(false)
            .barrier_color(aimer_widget::base::Color::Rgba(1, 2, 3, 4))
            .child(ZeroSizedBox)
            .to_portable_node(&mut context, source(3))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();

        assert_eq!(node.children().count(), 1);
        assert_eq!(node.properties().count(), 5);
        let materialized = <ContextMenu as PortableNativeWidget>::materialize_widget(
            &view,
            node,
            vec![ZeroSizedBox.boxed()],
        )
        .unwrap();
        assert_eq!(materialized.debug_name(), "ContextMenu");

        let schema = <ContextMenu as PortableWidgetSchema>::SCHEMA;
        assert!(
            linked_portable_native_widget_registrations()
                .iter()
                .any(|registration| registration
                    .supports(schema.widget().id(), schema.widget().min_version()))
        );
    }

    #[test]
    fn menu_materialization_rejects_more_than_one_child() {
        let mut context = context();
        let root = ContextMenu::new()
            .to_portable_node(&mut context, source(4))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();

        assert!(matches!(
            <ContextMenu as PortableNativeWidget>::materialize_widget(
                &view,
                node,
                vec![ZeroSizedBox.boxed(), ZeroSizedBox.boxed()],
            ),
            Err(
                aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                    expected: 1,
                    actual: 2,
                }
            )
        ));
    }
}
