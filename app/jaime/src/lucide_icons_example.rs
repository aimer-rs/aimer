//! The complete Lucide collection, with labeled, windowed preview rows.
use aimer::style::{BoxDecoration, LayoutSpacing, Spacing, TextAlign, TextStyle, ThemeData};
use aimer::{
    Align, Alignment, AnyWidget, Column, Container, Dimension, Expanded, Icons, Row, ScrollAxis,
    ScrollController, Scrollable, SizedBox, StaticSvg, Svg, Text, Widget,
};
use aimer::{JustifyContent, SvgStyle};

use crate::theme;

const COLUMNS: usize = 3;
const ROW_EXTENT: f32 = 144.0;
type Entry = (&'static str, &'static StaticSvg);

/// Browses every bundled icon in filename order without eagerly building them.
pub fn lucide_icons_example(app_theme: ThemeData) -> impl Widget {
    gallery(Icons::ALL, app_theme, ScrollController::new())
}

fn gallery(
    entries: &'static [Entry],
    app_theme: ThemeData,
    controller: ScrollController,
) -> impl Widget {
    Column::new().children([
        Text::new(format!("{} icons", entries.len()))
            .text_style(
                TextStyle::new()
                    .font_size(14)
                    .color(theme::muted_text(&app_theme)),
            )
            .boxed(),
        SizedBox::new()
            .height(12)
            .boxed(),
        Expanded::new().box_child(
            Scrollable::new()
                .axis(ScrollAxis::Vertical)
                .controller(controller)
                .child(
                    Column::new()
                        .list(entries.chunks(COLUMNS))
                        .item_extent(ROW_EXTENT)
                        .builder(move |entries| {
                            Container::new()
                                .height(ROW_EXTENT)
                                .padding(LayoutSpacing::new().bottom(12))
                                .child(
                                    Row::new()
                                        .gaps(LayoutSpacing::all(Spacing::Px(12)))
                                        .children(std::array::from_fn::<_, COLUMNS, _>(|index| {
                                            Expanded::new().box_child(match entries.get(index) {
                                                Some(entry) => tile(*entry, app_theme),
                                                None => SizedBox::new().boxed(),
                                            })
                                        })),
                                )
                        }),
                ),
        ),
    ])
}

fn tile((label, icon): Entry, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(ROW_EXTENT - 12.0)
        .padding(LayoutSpacing::all(12))
        .box_decoration(
            BoxDecoration::new()
                .background_color(theme::recessed_surface(&app_theme))
                .border_radius(10),
        )
        .box_child(
            Column::new()
                .justify_content(JustifyContent::SpaceEvenly)
                .horizontal_alignment(aimer::BoxAlignment::Center)
                .children([
                    Container::new()
                        .width(48)
                        .height(48)
                        .box_child(
                            Align::new()
                                .alignment(Alignment::MidCenter)
                                .child(
                                    Svg::new(icon)
                                        .style(
                                            "path",
                                            SvgStyle::new().stroke(app_theme.primary_color),
                                        )
                                        .width(32)
                                        .height(32),
                                ),
                        ),
                    SizedBox::new()
                        .height(8)
                        .boxed(),
                    Container::new()
                        .width(Dimension::Percent(100.0))
                        .box_child(
                            Text::new(label)
                                .wrapped()
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(12)
                                        .color(app_theme.on_background_color),
                                ),
                        ),
                ]),
        )
}

#[cfg(test)]
mod tests;
