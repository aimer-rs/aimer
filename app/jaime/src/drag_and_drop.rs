//! A three-column board whose cards are dragged between columns.
//!
//! This is the in-application half of drag and drop: `Draggable` picks a card
//! up and `DragTarget<CardId>` puts it down. The card marked "locked" is
//! refused by the `Done` column, so the rejection path — the feedback visibly
//! travelling back to where it was picked up — can be seen by hand.

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

/// Identifies one card. This is the payload a drag carries, and the reason a
/// `DragTarget<CardId>` never sees an unrelated drag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CardId(u32);

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

pub struct DragBoardState {
    cards: Vec<Card>,
    last_refused: Option<&'static str>,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for DragBoard {
    type State = DragBoardState;

    fn create_state(self) -> Self::State {
        DragBoardState {
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
            last_refused: None,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<DragBoard> for DragBoardState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
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
                            match self.last_refused {
                                Some(title) => {
                                    format!("\u{201c}{title}\u{201d} was refused by Done")
                                }
                                None => "Drag a card. The locked one cannot reach Done.".to_owned(),
                            },
                            app_theme,
                        ),
                        SizedBox::new()
                            .height(24)
                            .boxed(),
                        Expanded::new()
                            .child(
                                Row::new()
                                    .vertical_alignment(BoxAlignment::Start)
                                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                                    .children(
                                        (0..COLUMNS.len())
                                            .map(|index| self.column(index, app_theme))
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

        DragTarget::<CardId>::new()
            .will_accept(move |id: &CardId| {
                !locked_here || !predicate.read(|state| state.is_locked(*id))
            })
            .on_accept(move |id: CardId| {
                accepter.set_state(move |state| state.move_card(id, index));
            })
            .child(move |state: DragTargetState| {
                let tiles = cards
                    .iter()
                    .map(|card| draggable_card(card, builder, app_theme))
                    .collect::<Vec<AnyWidget>>();
                column_body(title, accent, state, tiles, app_theme)
            })
            .boxed()
    }

    fn is_locked(&self, id: CardId) -> bool {
        self.cards
            .iter()
            .any(|card| card.id == id && card.locked)
    }

    fn move_card(&mut self, id: CardId, column: usize) {
        if let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.id == id)
        {
            card.column = column;
            self.last_refused = None;
        }
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
        .data(id)
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
    state: DragTargetState,
    cards: Vec<AnyWidget>,
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
                .children(
                    std::iter::once(column_heading(title, accent))
                        .chain(cards)
                        .collect::<Vec<AnyWidget>>(),
                ),
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
