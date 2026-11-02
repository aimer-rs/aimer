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
pub struct Column<W = RequiredChild> {
    vertical_alignment: BoxAlignment,
    horizontal_alignment: BoxAlignment,
    justify_content: Option<JustifyContent>,
    gaps: LayoutSpacing,
    overflow: OverflowBehavior,
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
pub struct Row<W: Widget + 'static = AnyWidget> {
    vertical_alignment: BoxAlignment,
    horizontal_alignment: BoxAlignment,
    justify_content: Option<JustifyContent>,
    gaps: LayoutSpacing,
    overflow: OverflowBehavior,
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

}
