//! Dropping at a position in a list.
//!
//! A [`DragTarget`](crate::DragTarget) answers *whether* something was dropped
//! on it. A list needs to know *where*: between which two rows the new item
//! belongs. [`InsertionList`] answers that. It owns the layout of one row or
//! column of items, works out which slot the pointer is nearest to, opens a gap
//! at that slot so the user can see it before letting go, and reports the slot
//! with the dropped value.
//!
//! # Slots
//!
//! A list of `n` items has `n + 1` slots, numbered `0..=n`: slot `0` is before
//! the first item and slot `n` is after the last. The pointer chooses the slot
//! by the *middle* of each item, not its edge: past the middle of an item the
//! slot is after it, short of the middle it is before. That is what makes the
//! choice change at the point a person expects, halfway across a row.
//!
//! Slots are numbered against the list **as it is displayed**. Dragging an item
//! of the same list to another place in it therefore names a slot that still
//! counts the item itself; [`slot_after_removal`] converts that into the index
//! to insert at once the item has been taken out.
//!
//! # Where the items are
//!
//! The list does not measure its items itself. Each is wrapped in a transparent
//! probe that reports its own rectangle through
//! [`InteractionBounds`](aimer_widget::InteractionBounds), the same source hit
//! testing uses, so a list inside a scrolling region is measured where it is
//! drawn now and not where it was last laid out.
//!
//! The gap is a real child of the row or column, not an overlay. That cannot
//! make the slot flicker: opening a gap moves only the items after it, and those
//! are the ones the pointer has not yet passed the middle of, so moving them
//! further away leaves the answer unchanged.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use aimer_attribute::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_color::prelude::Color;
use aimer_container::{Container, ZeroSizedBox};
use aimer_events::element::ElementEvent;
use aimer_flex::{Column, Row};
use aimer_style::{LayoutSpacing, Spacing};
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, EventTreeRole,
    LayoutElement, PointerKey, Rebuildable, RequiredChild, State, StateUpdater, StatefulElement,
    StatefulWidget, VisitorElement, Widget,
};

use crate::DragSession;
use crate::target::{AcceptPredicate, HasChild, clear_hover, enter_hover, next_target_id};

/// Receives an accepted value together with the slot it was dropped at.
type InsertHandler<T> = Rc<dyn Fn(usize, T)>;

/// Builds the items of a list from its drag state.
type ItemsBuilder = Rc<dyn Fn(InsertionState) -> Vec<AnyWidget>>;

/// Builds the marker shown at the slot a drop would land in.
type IndicatorBuilder = Rc<dyn Fn() -> AnyWidget>;

/// Wraps the list in whatever should change with the drag over it.
type FrameBuilder = Rc<dyn Fn(InsertionState, AnyWidget) -> AnyWidget>;

/// How thick the default slot marker is, in logical pixels.
const DEFAULT_INDICATOR_THICKNESS: u32 = 3;

/// Which way a list lays its items out.
///
/// # Examples
///
/// ```
/// use aimer_dnd::ListAxis;
///
/// assert_eq!(ListAxis::default(), ListAxis::Vertical);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ListAxis {
    /// Items are stacked top to bottom, as in a column.
    #[default]
    Vertical,
    /// Items are placed left to right, as in a row.
    Horizontal,
}

impl ListAxis {
    /// The coordinate of `point` along this axis.
    #[inline]
    fn along(self, point: Vec2d) -> f32 {
        match self {
            Self::Vertical => point.y,
            Self::Horizontal => point.x,
        }
    }
}

/// Chooses the slot a pointer is nearest to among `items`.
///
/// `items` are the rectangles of the list's items as top-left and bottom-right
/// corners, in the order they are displayed. The slot is the number of items
/// whose middle the pointer has passed, so a pointer exactly on a middle is
/// still before that item.
///
/// # Examples
///
/// ```
/// use aimer_attribute::position::Vec2d;
/// use aimer_dnd::{ListAxis, insertion_slot};
///
/// let row = |top: f32, bottom: f32| {
///     (Vec2d { x: 0.0, y: top }, Vec2d { x: 90.0, y: bottom })
/// };
/// // Three rows, 40 px tall.
/// let rows = [row(0.0, 40.0), row(40.0, 80.0), row(80.0, 120.0)];
/// let at = |y: f32| insertion_slot(ListAxis::Vertical, Vec2d { x: 10.0, y }, rows);
///
/// assert_eq!(at(10.0), 0); // above the middle of the first row
/// assert_eq!(at(50.0), 1); // past the first row's middle, short of the second's
/// assert_eq!(at(110.0), 3); // past the last row's middle: the end
/// ```
pub fn insertion_slot(
    axis: ListAxis,
    pointer: Vec2d,
    items: impl IntoIterator<Item = (Vec2d, Vec2d)>,
) -> usize {
    let pointer = axis.along(pointer);
    items
        .into_iter()
        .filter(|(start, end)| (axis.along(*start) + axis.along(*end)) * 0.5 < pointer)
        .count()
}

/// Converts a slot of the list as displayed into the index to insert at once
/// the item at `from` has been removed.
///
/// Taking an item out shifts every later item down by one, so a slot after the
/// item's own position is one too high for the list that is left.
///
/// # Examples
///
/// ```
/// use aimer_dnd::slot_after_removal;
///
/// // Moving item 0 to the slot after item 1 puts it at index 1.
/// assert_eq!(slot_after_removal(0, 2), 1);
/// // Moving item 2 to the front is unaffected.
/// assert_eq!(slot_after_removal(2, 0), 0);
/// // The slots on either side of the item itself leave it where it is.
/// assert_eq!(slot_after_removal(1, 1), 1);
/// assert_eq!(slot_after_removal(1, 2), 1);
/// ```
#[inline]
pub const fn slot_after_removal(from: usize, slot: usize) -> usize {
    if slot > from { slot - 1 } else { slot }
}

/// What a list knows about the drag currently over it.
///
/// Handed to the items builder when it changes, and only then: a pointer moving
/// within one slot does not rebuild the list.
///
/// # Examples
///
/// ```
/// use aimer_dnd::InsertionState;
///
/// let idle = InsertionState::default();
///
/// assert!(!idle.is_hovered);
/// assert_eq!(idle.slot, None);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InsertionState {
    /// Whether a drag of the list's payload type is over it right now.
    pub is_hovered: bool,
    /// Whether releasing here would be accepted.
    pub will_accept: bool,
    /// The slot a release would insert at: `Some` only while the drag is over
    /// the list and would be accepted.
    pub slot: Option<usize>,
}

/// Where each item of one list currently is.
///
/// Filled by the items themselves as they are laid out, so it describes the
/// elements that are on screen and never one a rebuild has discarded.
#[derive(Default)]
struct Slots {
    items: RefCell<Vec<Option<Rc<InteractionBounds>>>>,
}

impl Slots {
    /// Keeps the first `len` entries, which belong to the items about to be
    /// built, and drops the rest.
    fn reset(&self, len: usize) {
        let mut items = self.items.borrow_mut();
        items.truncate(len);
        items.resize_with(len, || None);
    }

    /// Records that item `index` measures itself with `bounds`.
    fn register(&self, index: usize, bounds: &Rc<InteractionBounds>) {
        let mut items = self.items.borrow_mut();
        if items.len() <= index {
            items.resize_with(index + 1, || None);
        }
        if !items[index]
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, bounds))
        {
            items[index] = Some(Rc::clone(bounds));
        }
    }

    /// The slot `pointer` is nearest to.
    fn slot_at(&self, axis: ListAxis, pointer: Vec2d) -> usize {
        let items = self.items.borrow();
        insertion_slot(
            axis,
            pointer,
            items
                .iter()
                .filter_map(|item| item.as_ref()?.pos_start_end()),
        )
    }
}

/// A list that accepts dropped values at a position.
///
/// `new()` takes nothing and [`InsertionList::items`] comes last: the items
/// builder is what turns this into a [`Widget`].
///
/// The list lays its items out itself, in a column or a row, so that it can open
/// a gap at the slot the pointer is over. Everything about the items other than
/// their order is up to the builder. What surrounds the list and should react to
/// a drag is built by [`InsertionList::frame`].
///
/// # Examples
///
/// ```no_run
/// use aimer_container::{Container, ZeroSizedBox};
/// use aimer_dnd::{InsertionList, InsertionState, ListAxis, slot_after_removal};
/// use aimer_widget::Widget;
///
/// #[derive(Clone)]
/// struct Card(u32);
///
/// let list = InsertionList::<Card>::new()
///     .axis(ListAxis::Vertical)
///     .spacing(8)
///     .on_insert(|slot, card: Card| {
///         // `slot` counts the rows as shown; insert there.
///         let _ = (slot, card);
///     })
///     .items(|_state: InsertionState| {
///         (0..3).map(|_| Container::new().height(40).child(ZeroSizedBox).boxed())
///     });
///
/// // Moving a row that is already in the list: the slot still counts the row.
/// let _index = slot_after_removal(0, 2);
/// ```
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_dnd::InsertionList", schema_only)]
pub struct InsertionList<T, C = RequiredChild> {
    #[portable_skip]
    axis: ListAxis,
    #[portable_skip]
    spacing: u32,
    #[portable_skip]
    will_accept: Option<AcceptPredicate<T>>,
    #[portable_skip]
    on_insert: Option<InsertHandler<T>>,
    #[portable_skip]
    indicator: Option<IndicatorBuilder>,
    #[portable_skip]
    frame: Option<FrameBuilder>,
    // The items are rebuilt from live drag state, so they have no single static
    // portable child node. Keep the callback on the native path.
    #[portable_skip]
    items: Option<ItemsBuilder>,
    #[portable_skip]
    _child: PhantomData<C>,
    #[portable_skip]
    _payload: PhantomData<T>,
}

impl<T> Default for InsertionList<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> InsertionList<T> {
    /// Creates an incomplete vertical list that accepts every `T`.
    #[inline]
    pub fn new() -> Self {
        Self {
            axis: ListAxis::Vertical,
            spacing: 0,
            will_accept: None,
            on_insert: None,
            indicator: None,
            frame: None,
            items: None,
            _child: PhantomData,
            _payload: PhantomData,
        }
    }
}

impl<T, C> InsertionList<T, C> {
    /// Sets which way the items are laid out. The default is
    /// [`ListAxis::Vertical`].
    #[inline]
    pub fn axis(mut self, axis: ListAxis) -> Self {
        self.axis = axis;
        self
    }

    /// Sets the space between items, in logical pixels.
    ///
    /// The marker for a slot is spaced like an item, so the room it opens is its
    /// own size plus this on either side.
    #[inline]
    pub fn spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Decides, per drag, whether this list would take it.
    ///
    /// Called on hover, not on every move. A list with no predicate accepts every
    /// payload of its type, and offers no slot to one it refuses.
    #[inline]
    pub fn will_accept<F: Fn(&T) -> bool + 'static>(mut self, will_accept: F) -> Self {
        self.will_accept = Some(Rc::new(will_accept));
        self
    }

    /// Receives the dropped value and the slot it was dropped at, exactly once
    /// per accepted drop.
    ///
    /// The slot counts the items as displayed; see [`slot_after_removal`] for an
    /// item that is moving within the list.
    #[inline]
    pub fn on_insert<F: Fn(usize, T) + 'static>(mut self, on_insert: F) -> Self {
        self.on_insert = Some(Rc::new(on_insert));
        self
    }

    /// Replaces the marker shown at the slot a drop would land in.
    ///
    /// The marker is laid out as an item of the list, so its own size is the
    /// room that opens. The default is a thin line across the list.
    #[inline]
    pub fn indicator<F, V>(mut self, indicator: F) -> Self
    where
        F: Fn() -> V + 'static,
        V: Widget + 'static,
    {
        self.indicator = Some(Rc::new(move || indicator().boxed()));
        self
    }

    /// Wraps the list in a widget that is rebuilt with the drag state, for the
    /// parts of a list's surroundings that react to a drag: a border that
    /// highlights, a heading that changes.
    ///
    /// Only one target is hovered at a time, so nesting a second
    /// [`DragTarget`](crate::DragTarget) around the list to do this would leave
    /// one of the two never hovered. The frame is built with the same
    /// [`InsertionState`] the items are, and receives the list to place inside
    /// itself.
    #[inline]
    pub fn frame<F, V>(mut self, frame: F) -> Self
    where
        F: Fn(InsertionState, AnyWidget) -> V + 'static,
        V: Widget + 'static,
    {
        self.frame = Some(Rc::new(move |state, list| frame(state, list).boxed()));
        self
    }

    /// Builds the list's items from its drag state, completing the builder.
    ///
    /// Called again when the slot under the drag changes, so a closure must build
    /// every item from data it owns, as a [`DragTarget`](crate::DragTarget)'s
    /// child builder must.
    #[inline]
    pub fn items<F, I>(self, items: F) -> InsertionList<T, HasChild>
    where
        F: Fn(InsertionState) -> I + 'static,
        I: IntoIterator<Item = AnyWidget>,
    {
        InsertionList {
            axis: self.axis,
            spacing: self.spacing,
            will_accept: self.will_accept,
            on_insert: self.on_insert,
            indicator: self.indicator,
            frame: self.frame,
            items: Some(Rc::new(move |state| items(state).into_iter().collect())),
            _child: PhantomData,
            _payload: PhantomData,
        }
    }
}

impl<T: 'static> Widget for InsertionList<T, HasChild> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "InsertionList", None)
    }

    fn debug_name(&self) -> &'static str {
        "InsertionList"
    }
}

/// The drag state one list keeps between frames.
pub struct InsertionLiveState<T> {
    id: u64,
    hover: InsertionState,
    slots: Rc<Slots>,
    axis: ListAxis,
    spacing: u32,
    will_accept: Option<AcceptPredicate<T>>,
    on_insert: Option<InsertHandler<T>>,
    indicator: Option<IndicatorBuilder>,
    frame: Option<FrameBuilder>,
    items: Option<ItemsBuilder>,
    updater: StateUpdater<Self>,
}

impl<T: 'static> StatefulWidget for InsertionList<T, HasChild> {
    type State = InsertionLiveState<T>;

    fn create_state(self) -> Self::State {
        InsertionLiveState {
            id: next_target_id(),
            hover: InsertionState::default(),
            slots: Rc::new(Slots::default()),
            axis: self.axis,
            spacing: self.spacing,
            will_accept: self.will_accept,
            on_insert: self.on_insert,
            indicator: self.indicator,
            frame: self.frame,
            items: self.items,
            updater: StateUpdater::empty(),
        }
    }
}

impl<T> InsertionLiveState<T> {
    /// The child that occupies `slot` between items: the space between two items,
    /// nothing at either end of the list, or the marker while the slot is the one
    /// a drop would land in.
    ///
    /// A marked slot is the marker plus the spacing an item would have had on the
    /// side that has an item, so opening it moves the items after it by exactly
    /// the marker's size and the spacing around it does not change.
    fn slot_filler(&self, slot: usize, count: usize, marked: bool) -> AnyWidget {
        let gap = self.spacing;
        let (before, after) = match (slot == 0, slot == count) {
            (true, true) => (0, 0),
            (true, false) => (0, gap),
            (false, true) => (gap, 0),
            (false, false) => (gap / 2, gap - gap / 2),
        };

        if !marked {
            // Unmarked, only the space between two items is occupied.
            let extent = if slot == 0 || slot == count { 0.0 } else { gap as f32 };
            return match self.axis {
                ListAxis::Vertical => Container::new()
                    .width(Dimension::Percent(100.0))
                    .height(extent)
                    .child(ZeroSizedBox)
                    .boxed(),
                ListAxis::Horizontal => Container::new()
                    .width(extent)
                    .height(Dimension::Percent(100.0))
                    .child(ZeroSizedBox)
                    .boxed(),
            };
        }

        let marker = match self.indicator.as_ref() {
            Some(indicator) => indicator(),
            None => default_indicator(self.axis),
        };
        match self.axis {
            ListAxis::Vertical => Container::new()
                .width(Dimension::Percent(100.0))
                .padding(
                    LayoutSpacing::new()
                        .top(Spacing::Px(before))
                        .bottom(Spacing::Px(after)),
                )
                .child(marker)
                .boxed(),
            ListAxis::Horizontal => Container::new()
                .height(Dimension::Percent(100.0))
                .padding(
                    LayoutSpacing::new()
                        .left(Spacing::Px(before))
                        .right(Spacing::Px(after)),
                )
                .child(marker)
                .boxed(),
        }
    }
}

impl<T: 'static> State<InsertionList<T, HasChild>> for InsertionLiveState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.axis = new.axis;
        self.spacing = new.spacing;
        self.will_accept = new.will_accept;
        self.on_insert = new.on_insert;
        self.indicator = new.indicator;
        self.frame = new.frame;
        self.items = new.items;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let items = self
            .items
            .as_ref()
            .map(|build| build(self.hover))
            .unwrap_or_default();
        let count = items.len();
        self.slots.reset(count);

        // A slot is only marked while it would be accepted.
        let marked = self.hover.slot.filter(|_| self.hover.will_accept);

        // Every slot always has a filler, so the children keep their positions
        // whether or not a marker is showing: `[slot 0, item 0, slot 1, item 1,
        // ..., item n-1, slot n]`. Elements are reconciled by position, and a
        // marker inserted *between* children would move every later item to the
        // position of its neighbour, handing a carried item's drag to the wrong
        // row. The marker replaces a filler in place instead, and the fillers
        // carry the spacing between items.
        let mut children: Vec<AnyWidget> = Vec::with_capacity(count * 2 + 1);
        for (index, item) in items.into_iter().enumerate() {
            children.push(self.slot_filler(index, count, marked == Some(index)));
            children.push(
                SlotProbe {
                    index,
                    child: item,
                    slots: Rc::clone(&self.slots),
                }
                .boxed(),
            );
        }
        children.push(self.slot_filler(count, count, marked == Some(count)));

        let list = match self.axis {
            ListAxis::Vertical => Column::new().children(children).boxed(),
            ListAxis::Horizontal => Row::new().children(children).boxed(),
        };

        let content = match self.frame.as_ref() {
            Some(frame) => frame(self.hover, list),
            None => list,
        };

        InsertionGate {
            child: content,
            logic: Rc::new(InsertionLogic {
                id: self.id,
                axis: self.axis,
                slots: Rc::clone(&self.slots),
                will_accept: self.will_accept.clone(),
                on_insert: self.on_insert.clone(),
                updater: self.updater,
            }),
        }
    }
}

/// The marker used when the application supplies none: a thin line across the
/// list.
fn default_indicator(axis: ListAxis) -> AnyWidget {
    let thickness = DEFAULT_INDICATOR_THICKNESS as f32;
    let line = Container::new().color(Color::Rgba(64, 128, 255, 255));
    match axis {
        ListAxis::Vertical => line
            .width(Dimension::Percent(100.0))
            .height(thickness)
            .child(ZeroSizedBox)
            .boxed(),
        ListAxis::Horizontal => line
            .width(thickness)
            .height(Dimension::Percent(100.0))
            .child(ZeroSizedBox)
            .boxed(),
    }
}

/// The event-handling half of a list, kept apart from the state so a rebuild
/// does not have to recreate the element that owns the bounds.
struct InsertionLogic<T> {
    id: u64,
    axis: ListAxis,
    slots: Rc<Slots>,
    will_accept: Option<AcceptPredicate<T>>,
    on_insert: Option<InsertHandler<T>>,
    updater: StateUpdater<InsertionLiveState<T>>,
}

impl<T: 'static> InsertionLogic<T> {
    fn hover(&self) -> InsertionState {
        self.updater.read(|state| state.hover)
    }

    /// Rebuilds only if the state actually changed.
    fn set_hover(&self, hover: InsertionState) {
        if self.hover() == hover {
            return;
        }
        self.updater.set_state(move |state| state.hover = hover);
    }

    /// Whether this list would take the drag in flight, and whether the drag is
    /// even addressed to it.
    fn evaluate(&self) -> Option<bool> {
        DragSession::with_payload::<T, bool>(|payload| {
            self.will_accept
                .as_ref()
                .is_none_or(|predicate| predicate(payload))
        })
    }
}

/// Where a drop is delivered. Lays out as its child and takes its events.
struct InsertionGate<T> {
    child: AnyWidget,
    logic: Rc<InsertionLogic<T>>,
}

impl<T: 'static> Widget for InsertionGate<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawInsertionGate {
            child: self.child.to_element(ctx),
            logic: self.logic.clone(),
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "InsertionGate"
    }
}

impl<T: 'static> aimer_widget::PortableWidget for InsertionGate<T> {}

struct RawInsertionGate<T> {
    child: AnyElement,
    logic: Rc<InsertionLogic<T>>,
    bounds: InteractionBounds,
}

impl<T: 'static> RawInsertionGate<T> {
    /// Registers this list as the hovered one and remembers how to unregister it.
    fn enter(&self) {
        let logic = self.logic.clone();
        enter_hover(
            self.logic.id,
            Rc::new(move || logic.set_hover(InsertionState::default())),
        );
    }
}

impl<T: 'static> VisitorElement for RawInsertionGate<T> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "InsertionList"
    }
}

impl<T: 'static> EventElement for RawInsertionGate<T> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::DragOver { pos, .. } => {
                // Somebody else's payload: stay out of its way entirely, so a
                // target underneath that does understand it still hears.
                let Some(will_accept) = self.logic.evaluate() else {
                    return EventResult::ignored();
                };
                self.enter();
                self.logic.set_hover(InsertionState {
                    is_hovered: true,
                    will_accept,
                    slot: will_accept.then(|| self.logic.slots.slot_at(self.logic.axis, *pos)),
                });
                EventResult::consumed().with_redraw()
            }

            ElementEvent::DragDrop { pos, source, id } => {
                let Some(will_accept) = self.logic.evaluate() else {
                    return EventResult::ignored();
                };
                // The slot is read before the hover is cleared: clearing it
                // rebuilds the list without its marker.
                let slot = self.logic.slots.slot_at(self.logic.axis, *pos);
                clear_hover();
                if !will_accept {
                    // Refused: the payload stays in the session, which is how
                    // the release is reported as unaccepted and the feedback
                    // knows to spring back.
                    return EventResult::consumed().with_redraw();
                }

                let pointer = PointerKey::new(*source, *id);
                if let Some(payload) = DragSession::take(pointer)
                    && let Ok(value) = payload.downcast::<T>()
                    && let Some(on_insert) = self.logic.on_insert.as_ref()
                {
                    on_insert(slot, value);
                }
                EventResult::consumed().with_redraw()
            }

            ElementEvent::DragLeave { .. } | ElementEvent::Cancel => {
                self.logic.set_hover(InsertionState::default());
                EventResult::ignored()
            }

            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl<T: 'static> LayoutElement for RawInsertionGate<T> {
    #[inline]
    fn size(&self) -> Option<Size> {
        None
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.child.layout(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl<T: 'static> Drawable for RawInsertionGate<T> {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("InsertionList")
    }

    fn update(&self, ctx: &BuildContext) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = self.child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = self.child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.child.sync_paint_geometry(ctx);
    }
}

impl<T: 'static> Rebuildable for RawInsertionGate<T> {}

/// One item of a list, reporting where it is and otherwise standing aside.
struct SlotProbe {
    index: usize,
    child: AnyWidget,
    slots: Rc<Slots>,
}

impl Widget for SlotProbe {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawSlotProbe {
            index: self.index,
            child: self.child.to_element(ctx),
            slots: self.slots,
            bounds: Rc::new(InteractionBounds::new()),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SlotProbe"
    }
}

impl aimer_widget::PortableWidget for SlotProbe {}

struct RawSlotProbe {
    index: usize,
    child: AnyElement,
    slots: Rc<Slots>,
    bounds: Rc<InteractionBounds>,
}

impl RawSlotProbe {
    /// Measures the item and tells the list it is the one at `index`.
    ///
    /// Registered here, from the element that is laid out, rather than when it
    /// is built: a rebuild builds elements it then throws away, and only the
    /// ones that are laid out are on screen.
    fn measure(&self, ctx: &BuildContext, size: ResolvedSize) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.slots.register(self.index, &self.bounds);
    }
}

impl VisitorElement for RawSlotProbe {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "SlotProbe"
    }
}

impl EventElement for RawSlotProbe {
    /// The probe handles nothing; the item inside it may.
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawSlotProbe {
    #[inline]
    fn size(&self) -> Option<Size> {
        None
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.child.layout(ctx);
        self.measure(ctx, size);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Drawable for RawSlotProbe {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
        self.slots.register(self.index, &self.bounds);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("InsertionList item")
    }

    fn update(&self, ctx: &BuildContext) {
        self.measure(ctx, self.child.computed_size(ctx));
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.measure(ctx, self.child.computed_size(ctx));
        self.child.sync_paint_geometry(ctx);
    }
}

impl Rebuildable for RawSlotProbe {}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(top: f32, bottom: f32) -> (Vec2d, Vec2d) {
        (Vec2d { x: 0.0, y: top }, Vec2d { x: 90.0, y: bottom })
    }

    fn column(left: f32, right: f32) -> (Vec2d, Vec2d) {
        (Vec2d { x: left, y: 0.0 }, Vec2d { x: right, y: 30.0 })
    }

    fn at_y(y: f32) -> Vec2d {
        Vec2d { x: 10.0, y }
    }

    fn three_rows() -> [(Vec2d, Vec2d); 3] {
        [row(0.0, 40.0), row(40.0, 80.0), row(80.0, 120.0)]
    }

    #[test]
    fn an_empty_list_has_exactly_one_slot() {
        assert_eq!(
            insertion_slot(ListAxis::Vertical, at_y(500.0), std::iter::empty()),
            0
        );
        assert_eq!(
            insertion_slot(ListAxis::Vertical, at_y(-500.0), std::iter::empty()),
            0
        );
    }

    #[test]
    fn the_slot_changes_at_the_middle_of_each_item() {
        let slot = |y| insertion_slot(ListAxis::Vertical, at_y(y), three_rows());

        assert_eq!(slot(0.0), 0);
        assert_eq!(slot(19.9), 0);
        assert_eq!(slot(20.1), 1);
        assert_eq!(slot(59.9), 1);
        assert_eq!(slot(60.1), 2);
        assert_eq!(slot(99.9), 2);
        assert_eq!(slot(100.1), 3);
    }

    #[test]
    fn a_pointer_exactly_on_a_middle_is_still_before_that_item() {
        assert_eq!(
            insertion_slot(ListAxis::Vertical, at_y(20.0), three_rows()),
            0
        );
    }

    #[test]
    fn a_pointer_outside_the_list_clamps_to_its_ends() {
        let slot = |y| insertion_slot(ListAxis::Vertical, at_y(y), three_rows());

        assert_eq!(slot(-1000.0), 0);
        assert_eq!(slot(1000.0), 3);
    }

    #[test]
    fn spacing_between_items_does_not_move_the_decision() {
        // A 10 px gap between rows: the middles are what count.
        let rows = [row(0.0, 40.0), row(50.0, 90.0), row(100.0, 140.0)];

        // 45 is in the gap, past the first middle (20), short of the second (70).
        assert_eq!(insertion_slot(ListAxis::Vertical, at_y(45.0), rows), 1);
        assert_eq!(insertion_slot(ListAxis::Vertical, at_y(75.0), rows), 2);
    }

    #[test]
    fn a_horizontal_list_is_read_along_x() {
        let items = [column(0.0, 40.0), column(40.0, 80.0)];
        let slot = |x| {
            insertion_slot(
                ListAxis::Horizontal,
                Vec2d { x, y: 9999.0 },
                items,
            )
        };

        assert_eq!(slot(10.0), 0);
        assert_eq!(slot(30.0), 1);
        assert_eq!(slot(70.0), 2);
    }

    #[test]
    fn opening_a_gap_does_not_change_the_slot_it_opened_for() {
        // The pointer sits in slot 2. Opening a 5 px gap there moves only the
        // third row, down and away; the answer must not change, or the list
        // would flicker between two layouts.
        let pointer = at_y(80.0);
        let before = insertion_slot(ListAxis::Vertical, pointer, three_rows());
        let with_gap = [row(0.0, 40.0), row(40.0, 80.0), row(85.0, 125.0)];
        let after = insertion_slot(ListAxis::Vertical, pointer, with_gap);

        assert_eq!(before, 2);
        assert_eq!(after, before);
    }

    #[test]
    fn removal_shifts_only_the_slots_after_the_item() {
        assert_eq!(slot_after_removal(0, 0), 0);
        assert_eq!(slot_after_removal(0, 1), 0);
        assert_eq!(slot_after_removal(0, 2), 1);
        assert_eq!(slot_after_removal(0, 3), 2);
        assert_eq!(slot_after_removal(2, 0), 0);
        assert_eq!(slot_after_removal(2, 2), 2);
        assert_eq!(slot_after_removal(2, 3), 2);
    }

    #[test]
    fn slots_register_the_item_that_is_on_screen() {
        let slots = Slots::default();
        slots.reset(2);
        let first = Rc::new(InteractionBounds::new());
        let second = Rc::new(InteractionBounds::new());
        first.save(1.0, 0.0, 0.0, 90.0, 40.0);
        second.save(1.0, 0.0, 40.0, 90.0, 40.0);
        slots.register(0, &first);
        slots.register(1, &second);

        assert_eq!(slots.slot_at(ListAxis::Vertical, at_y(50.0)), 1);

        // A rebuild replaces the second row; the list must ask the new one.
        let replacement = Rc::new(InteractionBounds::new());
        replacement.save(1.0, 0.0, 100.0, 90.0, 40.0);
        slots.register(1, &replacement);

        assert_eq!(slots.slot_at(ListAxis::Vertical, at_y(50.0)), 1);
        assert_eq!(slots.slot_at(ListAxis::Vertical, at_y(130.0)), 2);
    }

    #[test]
    fn rebuilding_with_fewer_items_forgets_the_extra_slots() {
        let slots = Slots::default();
        slots.reset(3);
        for index in 0..3 {
            let bounds = Rc::new(InteractionBounds::new());
            bounds.save(1.0, 0.0, 40.0 * index as f32, 90.0, 40.0);
            slots.register(index, &bounds);
        }
        assert_eq!(slots.slot_at(ListAxis::Vertical, at_y(500.0)), 3);

        slots.reset(1);

        assert_eq!(slots.slot_at(ListAxis::Vertical, at_y(500.0)), 1);
    }

    #[test]
    fn completed_insertion_list_publishes_a_derived_schema() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <InsertionList<String, HasChild> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::none()
        );
    }
}
