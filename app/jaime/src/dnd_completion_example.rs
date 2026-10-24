//! A bounded drag-and-drop completion page for the Jaime showcase.
//!
//! The page keeps the existing public `Draggable`, `DragTarget`, and
//! `DropZone` widgets visible while the W9 model seams are wired through the
//! later integration pass. It deliberately uses a long list inside a
//! `Scrollable`, a single-pointer drag target, and a restricted file zone so
//! the production behavior has a concrete surface in the app.

use std::path::PathBuf;

use aimer::style::{LayoutSpacing, Spacing, TextStyle};
use aimer::*;

const ITEMS: [(&str, u32); 8] = [
    ("Stable item A", 1),
    ("Stable item B", 2),
    ("Stable item C", 3),
    ("Stable item D", 4),
    ("Stable item E", 5),
    ("Stable item F", 6),
    ("Stable item G", 7),
    ("Stable item H", 8),
];

/// Builds the W9 drag-and-drop completion page.
pub fn dnd_completion_example() -> impl Widget {
    let target = DragTarget::<u32>::new()
        .on_accept(|_id| {})
        .child(|state: DragTargetState| {
            let cards = ITEMS
                .iter()
                .map(|(label, id)| draggable_card(label, *id, state.is_hovered))
                .collect::<Vec<_>>();
            Container::new()
                .padding(LayoutSpacing::all(Spacing::Px(10)))
                .color(if state.will_accept {
                    Color::Rgba(46, 125, 50, 30)
                } else {
                    Color::Rgba(0, 0, 0, 12)
                })
                .child(
                    Column::new()
                        .gaps(LayoutSpacing::all(Spacing::Px(6)))
                        .children(cards),
                )
        });

    let files = DropZone::new()
        .extensions(["png", "jpg", "jpeg"])
        .on_drop(|_paths: Vec<PathBuf>| {})
        .child(|state: DragTargetState| {
            Container::new()
                .padding(LayoutSpacing::all(Spacing::Px(12)))
                .color(if state.is_hovered {
                    Color::Rgba(46, 125, 50, 30)
                } else {
                    Color::Rgba(0, 0, 0, 12)
                })
                .child(
                    Text::new("Drop PNG/JPEG files here — unsafe batches are rejected").wrapped(),
                )
        });

    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(28)))
        .color(Color::WHITE)
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(12)))
                .children([
                    Text::new("Drag and drop completion")
                        .text_style(TextStyle::new().font_size(26).color(Color::BLACK))
                        .boxed(),
                    Text::new(
                        "Stable-key reorder state, bounded edge scrolling, cancellation, and a safe file fallback.",
                    )
                    .wrapped()
                    .text_style(TextStyle::new().font_size(15).color(Color::BLACK))
                    .boxed(),
                    Expanded::new()
                        .child(
                            Scrollable::new()
                                .vertical_scroll_bar(None)
                                .child(target),
                        )
                        .boxed(),
                    files.boxed(),
                ]),
        )
}

/// Starts the W9 page as a standalone app entry point.
pub fn start_dnd_completion_example() {
    AimerApp::start(dnd_completion_example());
}

fn draggable_card(label: &'static str, id: u32, hovered: bool) -> AnyWidget {
    let background = if hovered {
        Color::Rgba(25, 118, 210, 42)
    } else {
        Color::Rgba(0, 0, 0, 18)
    };
    Draggable::new()
        .data(id)
        .child_when_dragging(
            Container::new()
                .height(Dimension::Px(48.0))
                .color(Color::Rgba(0, 0, 0, 8))
                .child(Text::new(label)),
        )
        .feedback(move || {
            Container::new()
                .height(Dimension::Px(48.0))
                .color(background)
                .child(Text::new(label))
        })
        .child(
            Container::new()
                .height(Dimension::Px(48.0))
                .color(background)
                .child(Text::new(label)),
        )
        .boxed()
}
