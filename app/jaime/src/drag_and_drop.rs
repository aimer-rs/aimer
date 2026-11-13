//! A three-column board whose cards are dragged between columns.
//!
//! This is the in-application half of drag and drop: `Draggable` picks a card
//! up and `InsertionList<Dragged>` puts it down *at a position*: the column
//! opens a gap where the card will go, and the card lands there, whether it is
//! one already on the board or the "New card" chip. The card marked "locked" is
//! refused by the `Done` column, so the rejection path — the feedback visibly
//! travelling back to where it was picked up — can be seen by hand.

use std::cell::Cell;
use std::rc::Rc;

use aimer::style::*;
use aimer::*;

/// The three columns of the board.
const COLUMNS: [&str; 3] = ["Todo", "Doing", "Done"];

/// The column a card cannot be dragged into while it is locked.
const LOCKED_COLUMN: usize = 2;

const CARD_HEIGHT: f32 = 56.0;
const COLUMN_WIDTH: f32 = 240.0;

/// Starts the kanban board showcase.
pub fn start_drag_and_drop_example() {
    AimerApp::start(crate::theme::provide(DragBoard::new().boxed()))
}

/// Identifies one card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CardId(u32);

/// What a drag on this board carries, and the reason an `InsertionList<Dragged>`
/// never sees an unrelated drag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dragged {
    /// A card already on the board, picked up to move or reorder.
    Move(CardId),
    /// The "New card" chip: dropping it creates a card at the slot.
    New,
}

/// The first id handed to a card created by dropping the chip.
const FIRST_NEW_CARD_ID: u32 = 100;

/// One card on the board.
#[derive(Clone)]
struct Card {
    id: CardId,
    title: &'static str,
    /// A locked card is refused by the `Done` column.
    locked: bool,
    column: usize,
}

#[derive(StatefulWidget)]
pub struct DragBoard {}

impl Default for DragBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl DragBoard {
    pub fn new() -> Self {
        Self {}
    }
}

/// What a look at a column's hover state says changed since the previous look.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HoverChange {
    /// Nothing worth feeling.
    Quiet,
    /// The drag just reached the column.
    Entered,
    /// The drag is still over the column, but the gap moved to another slot.
    SlotChanged,
}

/// Tells the moments a drag *reaches* a column and *reaches a new slot* in it
/// apart from the many rebuilds the column goes through in between.
///
/// The column's frame is rebuilt on every change of its hover state, so the
/// build itself says nothing about what changed. This remembers the previous
/// look and reports only the differences. It must outlive those rebuilds, so it
/// lives in the board state.
#[derive(Default)]
struct HoverTracker {
    over: Cell<bool>,
    slot: Cell<Option<usize>>,
}

impl HoverTracker {
    /// Records `state` as the latest look and says what it changed.
    ///
    /// Arriving reports `Entered` only, even though it also lands on a slot, so
    /// that one arrival is one haptic.
    fn observe(&self, state: InsertionState) -> HoverChange {
        let was_over = self.over.replace(state.is_hovered);
        let previous_slot = self.slot.replace(state.slot);
        if !state.is_hovered {
            HoverChange::Quiet
        } else if !was_over {
            HoverChange::Entered
        } else if state.slot.is_some() && state.slot != previous_slot {
            HoverChange::SlotChanged
        } else {
            HoverChange::Quiet
        }
    }
}

/// The haptic for `change`: arriving gets a tick when the column would take the
/// card and a warning when it would refuse it, matching the red outline; every
/// new slot the card reaches gets a light impact.
fn hover_feedback(change: HoverChange, will_accept: bool) {
    match change {
        HoverChange::Quiet => {}
        HoverChange::Entered if will_accept => haptics::Haptics::selection(),
        HoverChange::Entered => {
            haptics::Haptics::notification(haptics::NotificationStyle::Warning)
        }
        HoverChange::SlotChanged => haptics::Haptics::impact(haptics::ImpactStyle::Light),
    }
}

pub struct DragBoardState {
    /// Kept for the helper methods that call back into the state.
    updater: StateUpdater<Self>,
    /// Every card, in board order: a column shows its own cards in this order.
    cards: Vec<Card>,
    next_card_id: u32,
    last_refused: Option<&'static str>,
    /// One per column, shared with the closures that build it.
    hovers: Rc<[HoverTracker; COLUMNS.len()]>,
}

impl State<DragBoard> for DragBoardState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(widget: DragBoard) -> Self {
        DragBoardState {
            updater: StateUpdater::empty(),
            cards: vec![
                Card {
                    id: CardId(1),
                    title: "Write the plan",
                    locked: false,
                    column: 0,
                },
                Card {
                    id: CardId(2),
                    title: "Draw the overlay",
                    locked: false,
                    column: 0,
                },
                Card {
                    id: CardId(3),
                    title: "Ship the thing",
                    locked: false,
                    column: 1,
                },
                Card {
                    id: CardId(4),
                    title: "Locked: needs review",
                    locked: true,
                    column: 1,
                },
            ],
            next_card_id: FIRST_NEW_CARD_ID,
            last_refused: None,
            hovers: Rc::default(),
        }
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .children(vec![
                        heading("Drag and drop: move a card between columns", app_theme),
                        subheading(
                            match this.last_refused {
                                Some(title) => {
                                    format!("\u{201c}{title}\u{201d} was not placed")
                                }
                                None => "Drop a card at any position. The locked one cannot reach Done."
                                    .to_owned(),
                            },
                            app_theme,
                        ),
                        SizedBox::new()
                            .height(16)
                            .boxed(),
                        new_card_chip(app_theme),
                        SizedBox::new()
                            .height(16)
                            .boxed(),
                        Expanded::new()
                            .child(
                                Row::new()
                                    .vertical_alignment(BoxAlignment::Start)
                                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                                    .children(
                                        (0..COLUMNS.len())
                                            .map(|index| this.column(index, app_theme))
                                            .collect::<Vec<AnyWidget>>(),
                                    ),
                            )
                            .boxed(),
                    ]),
            )
    }
}

impl DragBoardState {
    /// One column: a drop target wrapping the cards that live in it.
    ///
    /// The child closure is called again on every hover flip, so it captures
    /// the cards as *data* and builds the tiles each time. A widget cannot be
    /// captured and reused: erased widgets are not clonable, by design — they
    /// are the throwaway side of the tree.
    fn column(&self, index: usize, app_theme: ThemeData) -> AnyWidget {
        let title = COLUMNS[index];
        let accent = match index {
            0 => app_theme.primary_color,
            1 => app_theme
                .primary_color
                .lighten(0.15),
            _ => app_theme
                .primary_color
                .lighten(0.30),
        };
        let cards: Vec<Card> = self
            .cards
            .iter()
            .filter(|card| card.column == index)
            .cloned()
            .collect();

        let locked_here = index == LOCKED_COLUMN;
        let predicate = self.updater;
        let accepter = self.updater;
        let builder = self.updater;
        let hovers = Rc::clone(&self.hovers);

        InsertionList::<Dragged>::new()
            .axis(ListAxis::Vertical)
            .spacing(10)
            .will_accept(move |dragged: &Dragged| match dragged {
                Dragged::Move(id) => !locked_here || !predicate.read(|state| state.is_locked(*id)),
                Dragged::New => true,
            })
            .on_insert(move |slot, dragged: Dragged| {
                accepter.set_state(move |state| state.insert(dragged, index, slot));
            })
            // The column's border and heading react to the drag, and wrap the
            // list, so they are the list's frame.
            .frame(move |state: InsertionState, list: AnyWidget| {
                hover_feedback(hovers[index].observe(state), state.will_accept);
                column_body(title, accent, state, list, app_theme)
            })
            // Rebuilt when the slot under the drag changes, so the tiles are
            // built from data the closure owns.
            .items(move |_state: InsertionState| {
                cards
                    .iter()
                    .map(|card| draggable_card(card, builder, app_theme))
                    .collect::<Vec<AnyWidget>>()
            })
            .boxed()
    }

    fn is_locked(&self, id: CardId) -> bool {
        self.cards
            .iter()
            .any(|card| card.id == id && card.locked)
    }

    /// Puts `dragged` into `column` at `slot`, a slot of the column as it was
    /// shown: `0` is above its first card and the card count is below its last.
    ///
    /// A card already on the board is taken out first, so a slot after its own
    /// position counts it and is one too high for the column that is left.
    fn insert(&mut self, dragged: Dragged, column: usize, slot: usize) {
        let (card, slot) = match dragged {
            Dragged::Move(id) => {
                let Some(position) = self.cards.iter().position(|card| card.id == id) else {
                    return;
                };
                let mut card = self.cards.remove(position);
                let slot = if card.column == column {
                    // Where the card was shown in this column before it left.
                    let shown_at = self.cards[..position]
                        .iter()
                        .filter(|other| other.column == column)
                        .count();
                    slot_after_removal(shown_at, slot)
                } else {
                    slot
                };
                card.column = column;
                (card, slot)
            }
            Dragged::New => {
                let card = Card {
                    id: CardId(self.next_card_id),
                    title: "New card",
                    locked: false,
                    column,
                };
                self.next_card_id += 1;
                (card, slot)
            }
        };

        // The card goes where the column's `slot`-th remaining card is, or after
        // its last one, or at the end of the board when the column is empty.
        let at = self
            .cards
            .iter()
            .enumerate()
            .filter(|(_, other)| other.column == column)
            .map(|(index, _)| index)
            .nth(slot)
            .unwrap_or_else(|| {
                self.cards
                    .iter()
                    .rposition(|other| other.column == column)
                    .map_or(self.cards.len(), |last| last + 1)
            });
        self.cards.insert(at, card);
        self.last_refused = None;
    }
}

/// One draggable card. Its feedback is the same tile, and the space it came
/// from is dimmed while it travels.
fn draggable_card(
    card: &Card,
    updater: StateUpdater<DragBoardState>,
    app_theme: ThemeData,
) -> AnyWidget {
    let id = card.id;
    let title = card.title;
    let locked = card.locked;

    Draggable::new()
        .data(Dragged::Move(id))
        .feedback(move || card_tile(title, locked, 1.0, app_theme))
        .child_when_dragging(card_tile(title, locked, 0.25, app_theme))
        .on_drag_completed(move |accepted| {
            if !accepted {
                updater.set_state(move |state| state.last_refused = Some(title));
            }
        })
        .child(card_tile(title, locked, 1.0, app_theme))
        .boxed()
}

/// The body of one column, highlighted while a card it would take hovers over
/// it and outlined in red while one it would refuse does.
fn column_body(
    title: &'static str,
    accent: Color,
    state: InsertionState,
    list: AnyWidget,
    app_theme: ThemeData,
) -> AnyWidget {
    let border = match (state.is_hovered, state.will_accept) {
        (true, true) => accent,
        (true, false) => app_theme
            .primary_color
            .darken(0.35),
        _ => crate::theme::muted_text(&app_theme),
    };

    Container::new()
        .width(Dimension::Px(COLUMN_WIDTH))
        .height(Dimension::Percent(100.0))
        .padding(LayoutSpacing::all(Spacing::Px(12)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(14)
                .border(BoxBorder::all(
                    BorderSlice::new()
                        .style(BorderStyle::Solid)
                        .stroke(Stroke::Px(2.0))
                        .color(border),
                )),
        )
        .child(
            Column::new()
                .horizontal_alignment(BoxAlignment::Start)
                .gaps(LayoutSpacing::new().bottom(10))
                .children(vec![column_heading(title, accent), list]),
        )
        .boxed()
}

fn column_heading(title: &'static str, accent: Color) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(28.0))
        .child(
            Text::new(title)
                .text_align(TextAlign::MidLeft)
                .text_style(
                    TextStyle::new()
                        .font_size(17)
                        .color(accent),
                ),
        )
        .boxed()
}

/// One card tile, at `alpha` so the same shape serves as content, feedback and
/// the dimmed placeholder left behind.
fn card_tile(title: &'static str, locked: bool, alpha: f32, app_theme: ThemeData) -> AnyWidget {
    let background = if locked {
        app_theme
            .primary_color
            .darken(0.35)
            .with_alpha(alpha)
    } else {
        app_theme
            .background_color
            .lighten(0.12)
            .with_alpha(alpha)
    };

    Container::new()
        .width(Dimension::Px(COLUMN_WIDTH - 24.0))
        .height(Dimension::Px(CARD_HEIGHT))
        .padding(LayoutSpacing::all(Spacing::Px(10)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(background)
                .border_radius(10),
        )
        .child(
            Text::new(title)
                .text_align(TextAlign::MidLeft)
                .text_style(
                    TextStyle::new()
                        .font_size(15)
                        .color(
                            app_theme
                                .on_surface_color
                                .with_alpha(alpha),
                        ),
                ),
        )
        .boxed()
}

/// The source of new cards: dropping it into a column creates a card at the slot.
fn new_card_chip(app_theme: ThemeData) -> AnyWidget {
    let chip = move || {
        Container::new()
            .width(Dimension::Px(160.0))
            .height(Dimension::Px(36.0))
            .padding(LayoutSpacing::all(Spacing::Px(8)))
            .box_decoration(
                BoxDecoration::new()
                    .background_color(app_theme.primary_color)
                    .border_radius(18),
            )
            .child(
                Text::new("+ New card")
                    .text_align(TextAlign::MidLeft)
                    .text_style(
                        TextStyle::new()
                            .font_size(15)
                            .color(app_theme.on_surface_color),
                    ),
            )
    };

    Draggable::new()
        .data(Dragged::New)
        .feedback(chip)
        .child(chip())
        .boxed()
}

fn heading(text: &'static str, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(34.0))
        .child(
            Text::new(text)
                .text_align(TextAlign::MidLeft)
                .text_style(
                    TextStyle::new()
                        .font_size(24)
                        .color(app_theme.on_background_color),
                ),
        )
        .boxed()
}

fn subheading(text: String, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(24.0))
        .child(
            Text::new(text)
                .text_align(TextAlign::MidLeft)
                .text_style(
                    TextStyle::new()
                        .font_size(15)
                        .color(crate::theme::muted_text(&app_theme)),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board's starting cards: Todo holds 1 and 2, Doing holds 3 and 4.
    fn board() -> DragBoardState {
        DragBoard::new().create_state::<DragBoardState>()
    }

    /// The ids in `column`, top to bottom.
    fn order(state: &DragBoardState, column: usize) -> Vec<u32> {
        state
            .cards
            .iter()
            .filter(|card| card.column == column)
            .map(|card| card.id.0)
            .collect()
    }

    #[test]
    fn a_new_card_lands_at_the_slot_it_was_dropped_at() {
        let mut state = board();

        state.insert(Dragged::New, 0, 1);

        let todo = order(&state, 0);
        assert_eq!(todo.len(), 3);
        assert_eq!(todo[0], 1, "the first card is still first");
        assert_eq!(todo[2], 2, "the old second card was pushed down");
        assert_eq!(state.cards.len(), 5);
        assert_eq!(order(&state, 1), vec![3, 4], "other columns are untouched");
    }

    #[test]
    fn a_new_card_can_go_first_or_last() {
        let mut state = board();

        state.insert(Dragged::New, 0, 0);
        state.insert(Dragged::New, 0, 4);

        let todo = order(&state, 0);
        assert_eq!(todo.len(), 4);
        assert_ne!(todo[0], 1);
        assert_ne!(todo[3], 2);
        assert_eq!((todo[1], todo[2]), (1, 2));
    }

    #[test]
    fn a_new_card_can_start_an_empty_column() {
        let mut state = board();

        state.insert(Dragged::New, 2, 0);

        assert_eq!(order(&state, 2).len(), 1);
    }

    #[test]
    fn every_new_card_is_a_different_card() {
        let mut state = board();

        state.insert(Dragged::New, 2, 0);
        state.insert(Dragged::New, 2, 0);

        let done = order(&state, 2);
        assert_ne!(done[0], done[1]);
    }

    #[test]
    fn moving_down_a_column_counts_the_card_itself() {
        let mut state = board();

        // The slot after the second card, as the list shows it: card 1 is still
        // in the list, so this is slot 2 and the card ends at index 1.
        state.insert(Dragged::Move(CardId(1)), 0, 2);

        assert_eq!(order(&state, 0), vec![2, 1]);
    }

    #[test]
    fn dropping_a_card_beside_itself_changes_nothing() {
        let mut state = board();

        state.insert(Dragged::Move(CardId(2)), 0, 1);
        state.insert(Dragged::Move(CardId(2)), 0, 2);

        assert_eq!(order(&state, 0), vec![1, 2]);
    }

    #[test]
    fn moving_to_another_column_inserts_at_the_slot() {
        let mut state = board();

        state.insert(Dragged::Move(CardId(1)), 1, 1);

        assert_eq!(order(&state, 0), vec![2]);
        assert_eq!(order(&state, 1), vec![3, 1, 4]);
    }

    #[test]
    fn moving_up_a_column_inserts_before_the_slot() {
        let mut state = board();

        state.insert(Dragged::Move(CardId(4)), 1, 0);

        assert_eq!(order(&state, 1), vec![4, 3]);
    }

    #[test]
    fn a_card_that_is_not_on_the_board_is_ignored() {
        let mut state = board();

        state.insert(Dragged::Move(CardId(99)), 0, 0);

        assert_eq!(state.cards.len(), 4);
    }

    fn over(slot: usize) -> InsertionState {
        InsertionState {
            is_hovered: true,
            will_accept: true,
            slot: Some(slot),
        }
    }

    #[test]
    fn a_drag_reaching_a_column_is_an_entry_once() {
        let tracker = HoverTracker::default();

        assert_eq!(tracker.observe(over(1)), HoverChange::Entered);
        // Looking again at the same state, as any other rebuild would.
        assert_eq!(tracker.observe(over(1)), HoverChange::Quiet);
    }

    #[test]
    fn a_column_that_is_not_hovered_is_never_a_change() {
        let tracker = HoverTracker::default();

        assert_eq!(tracker.observe(InsertionState::default()), HoverChange::Quiet);
        assert_eq!(tracker.observe(InsertionState::default()), HoverChange::Quiet);
    }

    #[test]
    fn leaving_and_coming_back_is_a_new_entry() {
        let tracker = HoverTracker::default();

        assert_eq!(tracker.observe(over(0)), HoverChange::Entered);
        assert_eq!(tracker.observe(InsertionState::default()), HoverChange::Quiet);
        assert_eq!(tracker.observe(over(0)), HoverChange::Entered);
    }

    #[test]
    fn reaching_another_slot_in_the_column_is_a_slot_change() {
        let tracker = HoverTracker::default();

        tracker.observe(over(0));
        assert_eq!(tracker.observe(over(1)), HoverChange::SlotChanged);
        assert_eq!(tracker.observe(over(2)), HoverChange::SlotChanged);
        // Back to where it was a moment ago is still a new slot to reach.
        assert_eq!(tracker.observe(over(1)), HoverChange::SlotChanged);
        assert_eq!(tracker.observe(over(1)), HoverChange::Quiet);
    }

    #[test]
    fn arriving_is_one_change_not_an_entry_and_a_slot_change() {
        let tracker = HoverTracker::default();

        // Arrival lands on slot 2 straight away; that is the entry, not a second
        // haptic for the slot.
        assert_eq!(tracker.observe(over(2)), HoverChange::Entered);
    }

    #[test]
    fn a_column_that_refuses_the_card_has_no_slots_to_reach() {
        let tracker = HoverTracker::default();
        let refusing = InsertionState {
            is_hovered: true,
            will_accept: false,
            slot: None,
        };

        assert_eq!(tracker.observe(refusing), HoverChange::Entered);
        assert_eq!(tracker.observe(refusing), HoverChange::Quiet);
    }

    #[test]
    fn the_slot_is_forgotten_when_the_drag_leaves() {
        let tracker = HoverTracker::default();

        tracker.observe(over(1));
        tracker.observe(InsertionState::default());
        // Slot 1 again is an arrival, not "unchanged since last time".
        assert_eq!(tracker.observe(over(1)), HoverChange::Entered);
    }

    #[test]
    fn columns_track_their_hovers_independently() {
        let hovers: Rc<[HoverTracker; COLUMNS.len()]> = Rc::default();

        assert_eq!(hovers[0].observe(over(0)), HoverChange::Entered);
        // Moving across: the next column is entered while the first is left.
        assert_eq!(hovers[0].observe(InsertionState::default()), HoverChange::Quiet);
        assert_eq!(hovers[1].observe(over(0)), HoverChange::Entered);
        assert_eq!(hovers[0].observe(InsertionState::default()), HoverChange::Quiet);
    }
}
