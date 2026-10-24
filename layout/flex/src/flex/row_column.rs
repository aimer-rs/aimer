use aimer_macro::PortableWidget;
use aimer_style::LayoutSpacing;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, RequiredChild, Widget};

use crate::flex::children_source::EagerChildren;
use crate::flex::flex_list::FlexList;
use crate::flex::raw_flex::RawFlex;
use crate::flex::{BoxAlignment, FlexDirection, JustifyContent, OverflowBehavior};

/// A flex container that arranges a homogeneous collection vertically.
///
/// Children run from top to bottom. Vertical alignment controls the main axis,
/// horizontal alignment controls the cross axis, and overflow defaults to
/// clipping. `Column::new()` is not a valid [`Widget`] until
/// [`Column::children`] supplies the terminal child collection.
///
/// # Example
///
/// ```rust
/// use aimer_container::SizedBox;
/// use aimer_flex::{Column, Row};
/// use aimer_widget::Widget;
///
/// let column = Column::new()
///                 .children([
///                     Row::new().children([
///                         SizedBox::new().width(40).boxed(),
///                         SizedBox::new().width(60).boxed()
///                     ]).boxed(),
///                     Row::new().children([
///                         SizedBox::new().width(100).boxed(),  // |
///                         SizedBox::new().width(20).boxed()    // | Same Widget no need to boxed
///                     ]).boxed()
///                 ]);
/// ```
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_flex::flex::Column", schema_only)]
pub struct Column<W = RequiredChild> {
    #[portable_optional]
    vertical_alignment: BoxAlignment,
    #[portable_optional]
    horizontal_alignment: BoxAlignment,
    justify_content: Option<JustifyContent>,
    #[portable_optional]
    gaps: LayoutSpacing,
    #[portable_optional]
    overflow: OverflowBehavior,
    #[portable_children]
    children: Vec<W>,
}

impl Default for Column {
    fn default() -> Self {
        Self::new()
    }
}

impl Column {
    /// Creates an empty column builder.
    ///
    /// Both alignments default to [`BoxAlignment::Start`], gaps to zero, and
    /// overflow to [`OverflowBehavior::Hidden`]. Finish with
    /// [`Column::children`] to obtain a valid [`Widget`].
    #[inline]
    pub fn new() -> Self {
        Self {
            vertical_alignment: Default::default(),
            horizontal_alignment: Default::default(),
            justify_content: None,
            gaps: Default::default(),
            overflow: Default::default(),
            children: Default::default(),
        }
    }

    /// Sets main-axis alignment for the column's children.
    ///
    /// The default is [`BoxAlignment::Start`].
    #[inline]
    pub fn vertical_alignment(mut self, alignment: BoxAlignment) -> Self {
        self.vertical_alignment = alignment;
        self
    }

    /// Sets horizontal cross-axis alignment for the column's children.
    ///
    /// The default is [`BoxAlignment::Start`].
    #[inline]
    pub fn horizontal_alignment(mut self, alignment: BoxAlignment) -> Self {
        self.horizontal_alignment = alignment;
        self
    }

    /// Sets placement along the column's vertical main axis.
    ///
    /// The default preserves [`Column::vertical_alignment`]. Use any of the
    /// six [`JustifyContent`] variants to place or distribute the children.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use aimer_flex::{Column, JustifyContent};
    ///
    /// let column = Column::new()
    ///     .justify_content(JustifyContent::SpaceAround)
    ///     .children(std::iter::empty::<aimer_container::SizedBox>());
    /// ```
    #[inline]
    pub fn justify_content(mut self, justify_content: JustifyContent) -> Self {
        self.justify_content = Some(justify_content);
        self
    }

    /// Sets logical-pixel spacing between adjacent children.
    ///
    /// The default is zero. For a column, the top and bottom components of the
    /// converted [`LayoutSpacing`] determine the inter-child gap.
    #[inline]
    pub fn gaps(mut self, gaps: impl Into<LayoutSpacing>) -> Self {
        self.gaps = gaps.into();
        self
    }

    /// Sets clipping or wrapping behavior when children exceed constraints.
    ///
    /// The default is [`OverflowBehavior::Hidden`]; use
    /// [`OverflowBehavior::Visible`] to paint beyond the bounds or
    /// [`OverflowBehavior::Wrap`] to continue in additional columns.
    #[inline]
    pub fn overflow(mut self, overflow: OverflowBehavior) -> Self {
        self.overflow = overflow;
        self
    }

    /// Replaces the children and returns an erased vertical layout.
    ///
    /// This is equivalent to [`Column::children`] followed by
    /// [`Widget::boxed`]. Use it when different branches need to return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_children<W: Widget + 'static>(
        self,
        children: impl IntoIterator<Item = W>,
    ) -> AnyWidget {
        Column {
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow: self.overflow,
            children: children.into_iter().collect(),
        }
        .boxed()
    }

    /// Supplies a data source instead of a widget collection.
    ///
    /// The returned [`FlexList`] is not yet a widget: pair it with
    /// [`FlexList::builder`] to map each datum to a child. Prefer this over
    /// [`Column::children`] for long lists — the column then retains the data
    /// rather than one widget per row.
    ///
    /// # Example
    ///
    /// ```rust
    /// use aimer_container::SizedBox;
    /// use aimer_flex::Column;
    ///
    /// let column = Column::new().list(0..120_000)
    ///                           .builder(|i| SizedBox::new().height(*i % 40));
    /// ```
    #[inline]
    pub fn list<T>(self, items: impl IntoIterator<Item = T>) -> FlexList<T> {
        FlexList::new(
            FlexDirection::Column,
            self.vertical_alignment,
            self.horizontal_alignment,
            self.justify_content,
            self.gaps,
            self.overflow,
            items,
        )
    }

    /// Replaces the child collection and completes this builder.
    ///
    /// All iterator items have one concrete widget type. This terminal
    /// operation returns a valid [`Column`], including for an empty
    /// iterator.
    #[inline]
    pub fn children<W: Widget>(self, children: impl IntoIterator<Item = W>) -> Column<W> {
        Column {
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow: self.overflow,
            children: children.into_iter().collect(),
        }
    }
}

impl<W: Widget + 'static> Widget for Column<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let children = EagerChildren(
            self.children
                .into_iter()
                .map(|c| c.to_element(ctx))
                .collect(),
        );
        RawFlex {
            direction: FlexDirection::Column,
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow_behavior: self.overflow,
            children: Box::new(children),
            cache: Default::default(),
            layout: Default::default(),
            item_extent: None,
            debug_name: "Column",
        }
        .boxed()
    }
}

/// A flex container that arranges its children horizontally.
///
/// Children run from left to right. Horizontal alignment controls the main
/// axis, vertical alignment controls the cross axis, and overflow defaults to
/// clipping. Unlike [`Column`], an empty `Row::new()` is already a valid erased
/// widget and supports incremental insertion.
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_flex::flex::Row", schema_only)]
pub struct Row<W: Widget + 'static = AnyWidget> {
    #[portable_optional]
    vertical_alignment: BoxAlignment,
    #[portable_optional]
    horizontal_alignment: BoxAlignment,
    justify_content: Option<JustifyContent>,
    #[portable_optional]
    gaps: LayoutSpacing,
    #[portable_optional]
    overflow: OverflowBehavior,
    #[portable_children]
    children: Vec<W>,
}

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

impl Row {
    /// Creates an empty row.
    ///
    /// Both alignments default to [`BoxAlignment::Start`], gaps to zero, and
    /// overflow to [`OverflowBehavior::Hidden`].
    #[inline]
    pub fn new() -> Self {
        Self {
            vertical_alignment: Default::default(),
            horizontal_alignment: Default::default(),
            justify_content: None,
            gaps: Default::default(),
            overflow: Default::default(),
            children: Default::default(),
        }
    }

    /// Sets vertical cross-axis alignment for the row's children.
    ///
    /// The default is [`BoxAlignment::Start`].
    #[inline]
    pub fn vertical_alignment(mut self, alignment: BoxAlignment) -> Self {
        self.vertical_alignment = alignment;
        self
    }

    /// Sets main-axis alignment for the row's children.
    ///
    /// The default is [`BoxAlignment::Start`].
    #[inline]
    pub fn horizontal_alignment(mut self, alignment: BoxAlignment) -> Self {
        self.horizontal_alignment = alignment;
        self
    }

    /// Sets placement along the row's horizontal main axis.
    ///
    /// The default preserves [`Row::horizontal_alignment`]. Use any of the
    /// six [`JustifyContent`] variants to place or distribute the children.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use aimer_flex::{JustifyContent, Row};
    ///
    /// let row = Row::new().justify_content(JustifyContent::SpaceBetween);
    /// ```
    #[inline]
    pub fn justify_content(mut self, justify_content: JustifyContent) -> Self {
        self.justify_content = Some(justify_content);
        self
    }

    /// Sets logical-pixel spacing between adjacent children.
    ///
    /// The default is zero. For a row, the left and right components of the
    /// converted [`LayoutSpacing`] determine the inter-child gap.
    #[inline]
    pub fn gaps(mut self, gaps: impl Into<LayoutSpacing>) -> Self {
        self.gaps = gaps.into();
        self
    }

    /// Sets clipping or wrapping behavior when children exceed constraints.
    ///
    /// The default is [`OverflowBehavior::Hidden`]; use
    /// [`OverflowBehavior::Visible`] to paint beyond the bounds or
    /// [`OverflowBehavior::Wrap`] to continue in additional rows.
    #[inline]
    pub fn overflow(mut self, overflow: OverflowBehavior) -> Self {
        self.overflow = overflow;
        self
    }

    /// Supplies a data source instead of a widget collection.
    ///
    /// The returned [`FlexList`] is not yet a widget: pair it with
    /// [`FlexList::builder`] to map each datum to a child.
    ///
    /// # Example
    ///
    /// ```rust
    /// use aimer_container::SizedBox;
    /// use aimer_flex::Row;
    ///
    /// let row = Row::new().list([40, 60, 80])
    ///                     .builder(|width| SizedBox::new().width(*width));
    /// ```
    #[inline]
    pub fn list<T>(self, items: impl IntoIterator<Item = T>) -> FlexList<T> {
        FlexList::new(
            FlexDirection::Row,
            self.vertical_alignment,
            self.horizontal_alignment,
            self.justify_content,
            self.gaps,
            self.overflow,
            items,
        )
    }

    /// Replaces all children with a homogeneous collection.
    ///
    /// This is not an append operation. The returned row adopts the iterator's
    /// concrete item type and remains valid when the iterator is empty.
    #[inline]
    pub fn children<W: Widget>(self, children: impl IntoIterator<Item = W>) -> Row<W> {
        Row {
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow: self.overflow,
            children: children.into_iter().collect(),
        }
    }

    /// Replaces the children and erases the completed row's concrete type.
    ///
    /// This is equivalent to [`Row::children`] followed by [`Widget::boxed`].
    /// Use it when different branches need to return one [`AnyWidget`] type.
    #[inline]
    pub fn box_children<W: Widget + 'static>(
        self,
        children: impl IntoIterator<Item = W>,
    ) -> AnyWidget {
        Row {
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow: self.overflow,
            children: children.into_iter().collect(),
        }
        .boxed()
    }

    /// Appends a child to the erased child collection.
    ///
    /// The child is boxed internally, so successive calls may use different
    /// concrete widget types. Existing children are retained.
    #[inline]
    pub fn add_child<W: Widget + 'static>(mut self, child: W) -> Self {
        self.children.push(child.boxed());
        self
    }

    /// Inserts a child at `index` in the erased child collection.
    ///
    /// Existing children at and after `index` move one position to the right.
    /// This method panics when `index` is greater than the current length, just
    /// like [`Vec::insert`].
    #[inline]
    pub fn insert_child<W: Widget + 'static>(mut self, index: usize, child: W) -> Self {
        self.children.insert(index, child.boxed());
        self
    }
}
//
// impl<W: Widget + 'static> Iterator for Row<W> {
//     type Item = W;
//
//     fn next(&mut self) -> Option<Self::Item> {
//         self.children.pop()
//     }
// }

impl<W: Widget + 'static> Widget for Row<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let children = EagerChildren(
            self.children
                .into_iter()
                .map(|c| c.to_element(ctx))
                .collect(),
        );
        RawFlex {
            direction: FlexDirection::Row,
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow_behavior: self.overflow,
            children: Box::new(children),
            cache: Default::default(),
            layout: Default::default(),
            item_extent: None,
            debug_name: "Row",
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use aimer_style::{LayoutSpacing, Spacing};

    use super::{Column, Row};
    use crate::flex::{BoxAlignment, JustifyContent, OverflowBehavior};

    #[test]
    fn native_builders_retain_their_layout_configuration() {
        let column = Column::new()
            .vertical_alignment(BoxAlignment::End)
            .horizontal_alignment(BoxAlignment::Center)
            .justify_content(JustifyContent::SpaceAround)
            .gaps(LayoutSpacing::vertical(8))
            .overflow(OverflowBehavior::Wrap)
            .children(std::iter::empty::<Row>());
        assert!(column.children.is_empty());
        assert!(column.vertical_alignment == BoxAlignment::End);
        assert!(column.horizontal_alignment == BoxAlignment::Center);
        assert!(column.justify_content == Some(JustifyContent::SpaceAround));
        assert!(column.gaps.top == Spacing::Px(8));
        assert!(column.gaps.bottom == Spacing::Px(8));
        assert!(column.overflow == OverflowBehavior::Wrap);

        let row = Row::new()
            .vertical_alignment(BoxAlignment::Center)
            .horizontal_alignment(BoxAlignment::End)
            .justify_content(JustifyContent::SpaceEvenly)
            .gaps(LayoutSpacing::horizontal(Spacing::Px(13)))
            .overflow(OverflowBehavior::Visible);
        assert!(row.children.is_empty());
        assert!(row.vertical_alignment == BoxAlignment::Center);
        assert!(row.horizontal_alignment == BoxAlignment::End);
        assert!(row.justify_content == Some(JustifyContent::SpaceEvenly));
        assert!(row.gaps.left == Spacing::Px(13));
        assert!(row.gaps.right == Spacing::Px(13));
        assert!(row.overflow == OverflowBehavior::Visible);
    }

    #[cfg(feature = "portable-guest")]
    mod portable {
        use std::cell::RefCell;
        use std::rc::Rc;

        use aimer_anteros::{
            PROPERTY_COLUMN_GAPS, PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT,
            PROPERTY_COLUMN_JUSTIFY_CONTENT, PROPERTY_COLUMN_OVERFLOW,
            PROPERTY_COLUMN_VERTICAL_ALIGNMENT, PROPERTY_ROW_GAPS,
            PROPERTY_ROW_HORIZONTAL_ALIGNMENT, PROPERTY_ROW_JUSTIFY_CONTENT, PROPERTY_ROW_OVERFLOW,
            PROPERTY_ROW_VERTICAL_ALIGNMENT, PROPERTY_SIZED_BOX_WIDTH, PropertyValue, Version,
            WIDGET_COLUMN, WIDGET_ROW, WIDGET_SIZED_BOX, WidgetDocumentView, WidgetProperty,
        };
        use aimer_widget::base::BuildContext;
        use aimer_widget::portable::{
            PortableBuildContext, PortableBuildError, PortableLimits, PortableNodeId,
            PortableWidgetLimits, PortableWidgetResource, PortableWidgetSchema, SourceFingerprint,
            StableId128,
        };
        use aimer_widget::{AnyElement, Key, PortableWidget, Widget};

        use super::{BoxAlignment, Column, JustifyContent, LayoutSpacing, OverflowBehavior, Row};

        const SCHEMA_V1: Version = Version::new(1, 0);

        struct PortableLeaf {
            key: Option<Key>,
            width: Option<f64>,
            seen_sources: Rc<RefCell<Vec<SourceFingerprint>>>,
        }

        impl PortableLeaf {
            fn new(seen_sources: Rc<RefCell<Vec<SourceFingerprint>>>) -> Self {
                Self {
                    key: None,
                    width: None,
                    seen_sources,
                }
            }

            fn keyed(mut self, key: Key) -> Self {
                self.key = Some(key);
                self
            }

            fn width(mut self, width: f64) -> Self {
                self.width = Some(width);
                self
            }
        }

        impl Widget for PortableLeaf {
            fn key(&self) -> Option<Key> {
                self.key.clone()
            }

            fn to_element(self, _ctx: &BuildContext) -> AnyElement {
                panic!("portable test leaf must not enter native construction")
            }
        }

        impl PortableWidget for PortableLeaf {
            fn to_portable_node(
                self,
                ctx: &mut PortableBuildContext,
                source: SourceFingerprint,
            ) -> Result<PortableNodeId, PortableBuildError> {
                self.seen_sources.borrow_mut().push(source);
                let properties: Vec<_> = self
                    .width
                    .map(|width| {
                        WidgetProperty::new(PROPERTY_SIZED_BOX_WIDTH, PropertyValue::F64(width))
                    })
                    .into_iter()
                    .collect();
                ctx.push_node(
                    WIDGET_SIZED_BOX,
                    SCHEMA_V1,
                    self.key.as_ref(),
                    source,
                    &properties,
                    &[],
                )
            }
        }

        fn source(byte: u8) -> SourceFingerprint {
            SourceFingerprint::new(StableId128::from_bytes([byte; 16]))
        }

        fn context(limits: PortableWidgetLimits) -> PortableBuildContext {
            PortableBuildContext::new(7, 11, limits, PortableLimits::new(8, 16, 64, 128, 1_024))
                .unwrap()
        }

        fn limits() -> PortableWidgetLimits {
            PortableWidgetLimits::new(16, 16, 16, 16, 64, 4_096).with_max_blob_bytes(64)
        }

        #[test]
        fn reflected_flex_schemas_retain_the_built_in_collection_contract() {
            let column = <Column<PortableLeaf> as PortableWidgetSchema>::SCHEMA;
            assert_eq!(column.widget().id(), WIDGET_COLUMN);
            assert_eq!(column.children().minimum(), 0);
            assert_eq!(column.children().maximum(), u32::MAX);
            assert_eq!(column.properties().len(), 5);

            let row = <Row<PortableLeaf> as PortableWidgetSchema>::SCHEMA;
            assert_eq!(row.widget().id(), WIDGET_ROW);
            assert_eq!(row.children().minimum(), 0);
            assert_eq!(row.children().maximum(), u32::MAX);
            assert_eq!(row.properties().len(), 5);
        }

        #[test]
        fn column_lowers_to_exact_bounded_widget_ir_with_child_keys() {
            let seen_sources = Rc::new(RefCell::new(Vec::new()));
            let child_key = Key::fixed([0x31; 16]);
            let mut ctx = context(limits());
            let root = Column::new()
                .children([
                    PortableLeaf::new(Rc::clone(&seen_sources)).keyed(child_key.clone()),
                    PortableLeaf::new(Rc::clone(&seen_sources)),
                ])
                .to_portable_node(&mut ctx, source(9))
                .unwrap();
            let child_sources = seen_sources.borrow();
            assert_eq!(child_sources.len(), 2);
            assert_ne!(child_sources[0], child_sources[1]);
            assert_ne!(child_sources[0], source(9));
            let keyed_slot = ctx.slot_for(Some(&child_key), child_sources[0]);

            let document = ctx.finish_document(root).unwrap();
            let bytes = document.encode().unwrap();
            let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
            assert_eq!(view.root_node(), 2);
            assert_eq!(view.node_count(), 3);
            assert_eq!(view.node(0).unwrap().widget_type(), WIDGET_SIZED_BOX);
            assert_eq!(
                view.node(0).unwrap().key().unwrap().as_bytes(),
                &keyed_slot.to_bytes()
            );
            assert_eq!(view.node(1).unwrap().widget_type(), WIDGET_SIZED_BOX);
            let root = view.node(2).unwrap();
            assert_eq!(root.widget_type(), WIDGET_COLUMN);
            assert_eq!(root.widget_schema(), SCHEMA_V1);
            assert_eq!(root.properties().count(), 0);
            assert_eq!(root.children().collect::<Vec<_>>(), vec![0, 1]);
        }

        #[test]
        fn row_lowers_erased_children_and_derives_sources_deterministically() {
            fn lower() -> (Vec<SourceFingerprint>, Vec<u32>) {
                let seen_sources = Rc::new(RefCell::new(Vec::new()));
                let mut ctx = context(limits());
                let root = Row::new()
                    .add_child(PortableLeaf::new(Rc::clone(&seen_sources)))
                    .add_child(PortableLeaf::new(Rc::clone(&seen_sources)))
                    .to_portable_node(&mut ctx, source(17))
                    .unwrap();
                let sources = seen_sources.borrow().clone();
                let document = ctx.finish_document(root).unwrap();
                let bytes = document.encode().unwrap();
                let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
                let root = view.node(2).unwrap();
                assert_eq!(root.widget_type(), WIDGET_ROW);
                assert_eq!(root.widget_schema(), SCHEMA_V1);
                assert_eq!(root.properties().count(), 0);
                (sources, root.children().collect())
            }

            let first = lower();
            let second = lower();
            assert_eq!(first, second);
            assert_eq!(first.1, vec![0, 1]);
        }

        #[test]
        fn portable_row_and_column_honor_child_and_property_limits() {
            let seen_sources = Rc::new(RefCell::new(Vec::new()));
            let mut child_limited = context(limits().with_max_children(1));
            let error = Row::new()
                .children([
                    PortableLeaf::new(Rc::clone(&seen_sources)),
                    PortableLeaf::new(Rc::clone(&seen_sources)),
                ])
                .to_portable_node(&mut child_limited, source(21))
                .unwrap_err();
            assert!(matches!(
                error,
                PortableBuildError::LimitExceeded {
                    resource: PortableWidgetResource::Children,
                    max: 1,
                    actual: 2,
                }
            ));

            let mut property_limited = context(limits().with_max_properties(0));
            let error = Column::new()
                .children([PortableLeaf::new(seen_sources).width(24.0)])
                .to_portable_node(&mut property_limited, source(22))
                .unwrap_err();
            assert!(matches!(
                error,
                PortableBuildError::LimitExceeded {
                    resource: PortableWidgetResource::Properties,
                    max: 0,
                    actual: 1,
                }
            ));
        }

        #[test]
        fn flex_properties_lower_and_preserve_their_canonical_values() {
            let mut column_context = context(limits());
            let column = Column::new()
                .vertical_alignment(BoxAlignment::Center)
                .horizontal_alignment(BoxAlignment::End)
                .justify_content(JustifyContent::SpaceEvenly)
                .gaps(LayoutSpacing::vertical(2_u32))
                .overflow(OverflowBehavior::Visible)
                .children(std::iter::empty::<PortableLeaf>())
                .to_portable_node(&mut column_context, source(31))
                .unwrap();
            let column_document = column_context.finish_document(column).unwrap();
            let column_bytes = column_document.encode().unwrap();
            let column_view =
                WidgetDocumentView::decode(&column_bytes, column_document.model_limits()).unwrap();
            let column = column_view.node(0).unwrap();
            assert_eq!(column.properties().count(), 5);
            assert!(column.properties().any(|property| {
                property.property_id() == PROPERTY_COLUMN_VERTICAL_ALIGNMENT
                    && property.value() == PropertyValue::I64(1)
            }));
            assert!(column.properties().any(|property| {
                property.property_id() == PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT
                    && property.value() == PropertyValue::I64(2)
            }));
            assert!(column.properties().any(|property| {
                property.property_id() == PROPERTY_COLUMN_JUSTIFY_CONTENT
                    && property.value() == PropertyValue::I64(5)
            }));
            assert!(column.properties().any(|property| {
                property.property_id() == PROPERTY_COLUMN_OVERFLOW
                    && property.value() == PropertyValue::I64(2)
            }));
            assert!(column.properties().any(|property| {
                property.property_id() == PROPERTY_COLUMN_GAPS
                    && matches!(property.value(), PropertyValue::BlobRef(_))
            }));

            let mut row_context = context(limits());
            let row = Row::new()
                .vertical_alignment(BoxAlignment::End)
                .horizontal_alignment(BoxAlignment::Center)
                .justify_content(JustifyContent::SpaceAround)
                .gaps(LayoutSpacing::horizontal(2_u32.into()))
                .overflow(OverflowBehavior::Wrap)
                .to_portable_node(&mut row_context, source(32))
                .unwrap();
            let row_document = row_context.finish_document(row).unwrap();
            let row_bytes = row_document.encode().unwrap();
            let row_view =
                WidgetDocumentView::decode(&row_bytes, row_document.model_limits()).unwrap();
            let row = row_view.node(0).unwrap();
            assert!(row.properties().any(|property| {
                property.property_id() == PROPERTY_ROW_VERTICAL_ALIGNMENT
                    && property.value() == PropertyValue::I64(2)
            }));
            assert!(row.properties().any(|property| {
                property.property_id() == PROPERTY_ROW_HORIZONTAL_ALIGNMENT
                    && property.value() == PropertyValue::I64(1)
            }));
            assert!(row.properties().any(|property| {
                property.property_id() == PROPERTY_ROW_JUSTIFY_CONTENT
                    && property.value() == PropertyValue::I64(4)
            }));
            assert!(row.properties().any(|property| {
                property.property_id() == PROPERTY_ROW_OVERFLOW
                    && property.value() == PropertyValue::I64(1)
            }));
            assert!(row.properties().any(|property| {
                property.property_id() == PROPERTY_ROW_GAPS
                    && matches!(property.value(), PropertyValue::BlobRef(_))
            }));
        }
    }
}
