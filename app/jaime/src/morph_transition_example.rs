use std::time::Duration;

use aimer::animation::{MorphStyle, MorphTransition, curves};
use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextStyle, Theme, ThemeData,
};
use aimer::{
    AnyWidget, BuildContext, Color, Column, Container, Dimension, Liquid, Positioned, Row,
    SizedBox, Stack, State, StateUpdater, StatefulWidget, Text, Widget,
};

use crate::theme;

/// One card the morph can land on. Each differs in size and color, so every
/// style has something to show: the shader blends what the card paints, not a
/// description of it.
struct Card {
    key: &'static str,
    title: &'static str,
    detail: &'static str,
    width: f32,
    height: f32,
    color: Color,
}

const CARDS: [Card; 3] = [
    Card {
        key: "compact",
        title: "Compact",
        detail: "A small chip.",
        width: 180.0,
        height: 72.0,
        color: Color::Rgba(46, 112, 219, 255),
    },
    Card {
        key: "tall",
        title: "Tall",
        detail: "Grows downward, new color.",
        width: 220.0,
        height: 200.0,
        color: Color::Rgba(217, 84, 79, 255),
    },
    Card {
        key: "wide",
        title: "Wide",
        detail: "Stretches across the stage.",
        width: 380.0,
        height: 110.0,
        color: Color::Rgba(51, 158, 107, 255),
    },
];

const STYLES: [(MorphStyle, &str); 5] = [
    (MorphStyle::Dissolve, "Dissolve"),
    (MorphStyle::Wipe, "Wipe"),
    (MorphStyle::Liquid, "Liquid"),
    (MorphStyle::Bounds, "Bounds"),
    (MorphStyle::Blur, "Blur"),
];

/// The two states of the navigation bar demo: a title, and the controls beside
/// it. The first state has one pill of two buttons; the second has the same pill
/// of two other buttons and, apart from it, a round third control, the way a
/// toolbar's buttons divide when the screen changes.
struct Header {
    key: &'static str,
    label: &'static str,
    title: &'static str,
    /// The glyphs of the buttons inside the pill.
    pill: &'static [&'static str],
    /// The initials of the round control beside the pill, if it has one.
    avatar: Option<&'static str>,
}

const HEADERS: [Header; 2] = [
    Header {
        key: "songs",
        label: "Songs",
        title: "Songs",
        pill: &["\u{2261}", "\u{22EF}"],
        avatar: None,
    },
    Header {
        key: "library",
        label: "Library",
        title: "Library",
        pill: &["\u{2295}", "\u{2611}"],
        avatar: Some("LS"),
    },
];

const HEADER_WIDTH: f32 = 480.0;
const HEADER_HEIGHT: f32 = 64.0;
/// Where the title's text begins, clear of the back button that sits over the
/// bar's left end.
const TITLE_INSET: f32 = 68.0;
const BACK_SIZE: f32 = 48.0;
const PILL_ICON: f32 = 60.0;
const PILL_PADDING: f32 = 8.0;
const PILL_HEIGHT: f32 = 48.0;
const CONTROLS_WIDTH: f32 = 300.0;
const CONTROLS_GAP: f32 = 12.0;

/// How long a card takes to morph into the next.
pub(crate) const MORPH_MS: u64 = 800;

/// Tall enough for the tallest card, so switching never moves the controls.
const STAGE_HEIGHT: f32 = 220.0;

/// Builds the morph-transition example without starting an application.
pub fn morph_transition_example() -> impl Widget {
    MorphTransitionExample
}

#[widget(Stateful)]
struct MorphTransitionExample;

struct MorphTransitionState {
    card: usize,
    style: usize,
    header: usize,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for MorphTransitionExample {
    type State = MorphTransitionState;

    fn create_state(self) -> Self::State {
        MorphTransitionState {
            card: 0,
            style: 0,
            header: 0,
            updater: StateUpdater::empty(),
        }
    }
}

/// The content handed to `MorphTransition`: the card paints its own color, so
/// what the shader blends is exactly what the card looks like.
fn card_content(card: &Card) -> AnyWidget {
    let white = Color::Rgba(255, 255, 255, 255);
    Container::new()
        .width(Dimension::Px(card.width))
        .height(Dimension::Px(card.height))
        .color(card.color)
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .child(Column::new().children(vec![
            Text::new(card.title)
                .text_style(
                    TextStyle::new()
                        .font_size(22)
                        .font_weight(FontWeight::Bold)
                        .color(white),
                )
                .boxed(),
            SizedBox::new().height(6).boxed(),
            Text::new(card.detail)
                .text_style(TextStyle::new().font_size(14).color(white))
                .boxed(),
        ]))
        .boxed()
}

/// The pill's width for a header: room for each of its buttons.
fn pill_width(header: &Header) -> f32 {
    PILL_ICON * header.pill.len() as f32 + PILL_PADDING
}

/// How wide a header's controls are together, the pill and the round control
/// beside it if there is one.
fn controls_width(header: &Header) -> f32 {
    pill_width(header)
        + if header.avatar.is_some() {
            CONTROLS_GAP + PILL_HEIGHT
        } else {
            0.0
        }
}

/// The title as a picture of the whole bar. Both titles fill the same box, so a
/// slide carries each one clear across it, under the back button, instead of
/// moving a box the size of its text.
fn title_content(header: &Header) -> AnyWidget {
    let white = Color::Rgba(255, 255, 255, 255);
    Container::new()
        .width(Dimension::Px(HEADER_WIDTH))
        .height(Dimension::Px(HEADER_HEIGHT))
        .child(Row::new().children(vec![
            SizedBox::new().width(TITLE_INSET as u32).boxed(),
            Column::new()
                .children(vec![
                    SizedBox::new().height(10).boxed(),
                    Text::new(header.title)
                        .text_style(
                            TextStyle::new()
                                .font_size(30)
                                .font_weight(FontWeight::Bold)
                                .color(white),
                        )
                        .boxed(),
                ])
                .boxed(),
        ]))
        .boxed()
}

/// A header's controls as a picture of one fixed region, with the pill and the
/// round control against its right edge. Both states fill the same region, so
/// the morph only has to turn one silhouette into the other: the pill keeps its
/// place while a round control buds off its right end.
fn controls_content(header: &Header) -> AnyWidget {
    let white = Color::Rgba(255, 255, 255, 255);
    let surface = Color::Rgba(48, 48, 54, 255);
    let icons = header
        .pill
        .iter()
        .map(|icon| {
            Container::new()
                .width(Dimension::Px(PILL_ICON))
                .height(Dimension::Px(PILL_HEIGHT - 8.0))
                .padding(LayoutSpacing::all(Spacing::Px(10)))
                .child(Text::new(*icon).text_style(TextStyle::new().font_size(20).color(white)))
                .boxed()
        })
        .collect::<Vec<_>>();
    let mut row = vec![
        SizedBox::new()
            .width((CONTROLS_WIDTH - controls_width(header)) as u32)
            .boxed(),
        Container::new()
            .width(Dimension::Px(pill_width(header)))
            .height(Dimension::Px(PILL_HEIGHT))
            .box_decoration(
                BoxDecoration::new()
                    .background_color(surface)
                    .border_radius(24),
            )
            .padding(LayoutSpacing::all(Spacing::Px(4)))
            .child(Row::new().children(icons))
            .boxed(),
    ];
    if let Some(initials) = header.avatar {
        row.push(SizedBox::new().width(CONTROLS_GAP as u32).boxed());
        row.push(
            Container::new()
                .width(Dimension::Px(PILL_HEIGHT))
                .height(Dimension::Px(PILL_HEIGHT))
                .box_decoration(
                    BoxDecoration::new()
                        .background_color(Color::Rgba(62, 52, 110, 255))
                        .border_radius(24),
                )
                .padding(LayoutSpacing::all(Spacing::Px(12)))
                .child(
                    Text::new(initials)
                        .text_style(TextStyle::new().font_size(17).font_weight(FontWeight::Bold).color(white)),
                )
                .boxed(),
        );
    }
    Container::new()
        .width(Dimension::Px(CONTROLS_WIDTH))
        .height(Dimension::Px(HEADER_HEIGHT - 8.0))
        .padding(LayoutSpacing::all(Spacing::Px(0)))
        .child(Row::new().children(row))
        .boxed()
}

/// The back button: a glass lens over the bar's left end, so it refracts the
/// title as it slides underneath.
fn back_button() -> impl Widget {
    let white = Color::Rgba(255, 255, 255, 255);
    Liquid::new()
        .tint(Color::Rgba(70, 70, 78, 255))
        .opacity(0.35)
        .border_color(Color::Rgba(255, 255, 255, 90))
        .border_radius(BACK_SIZE / 2.0)
        .distortion_strength(0.7)
        .magnification(0.7)
        .edge_lighting(0.6)
        .specular_highlight(0.5)
        .child(
            Container::new()
                .width(Dimension::Px(BACK_SIZE))
                .height(Dimension::Px(BACK_SIZE))
                .padding(LayoutSpacing::all(Spacing::Px(10)))
                .child(Text::new("\u{2039}").text_style(TextStyle::new().font_size(26).color(white))),
        )
}

/// The navigation bar: a title that is pushed in, controls that divide, and a
/// glass back button above both.
fn navigation_bar(header: &'static Header) -> impl Widget {
    // The old title leaves to the right and the new one comes in from the left,
    // across the whole bar and under the back button.
    let title = MorphTransition::new(
        Duration::from_millis(MORPH_MS),
        curves::EASE_IN_OUT,
        title_content(header),
    )
    .style(MorphStyle::Slide)
    .direction(1.0, 0.0)
    .child_key(header.key);

    // The controls divide: a round third control buds off the pill's right end
    // through a neck that thins and parts, while the buttons' glyphs blur from
    // the old ones to the new. Going back, the two shapes merge again.
    let controls = MorphTransition::new(
        Duration::from_millis(MORPH_MS),
        curves::LINEAR,
        controls_content(header),
    )
    .style(MorphStyle::Goo)
    .child_key(header.key);

    Container::new()
        .width(Dimension::Px(HEADER_WIDTH))
        .height(Dimension::Px(HEADER_HEIGHT))
        .color(Color::Rgba(8, 8, 10, 255))
        .child(Stack::new().children([
            Positioned::new().left(0).top(0).child(title).boxed(),
            Positioned::new().right(8).top(8).layer(1).child(controls).boxed(),
            Positioned::new().left(8).top(8).layer(2).child(back_button()).boxed(),
        ]))
}

impl MorphTransitionState {
    /// A row of buttons that sets one field of the state.
    fn choices(
        &self,
        theme: &ThemeData,
        labels: impl Iterator<Item = &'static str>,
        selected: usize,
        select: impl Fn(&mut Self, usize) + Copy + 'static,
    ) -> Row {
        let mut row = Vec::new();
        for (index, label) in labels.enumerate() {
            if index > 0 {
                row.push(SizedBox::new().width(8).boxed());
            }
            let updater = self.updater;
            let (background, text) = if index == selected {
                (theme.primary_color, theme.on_primary_color)
            } else {
                (theme.surface_color, theme.on_background_color)
            };
            row.push(
                Button::new()
                    .on_press(move || updater.set_state(move |state| select(state, index)))
                    .decoration(
                        BoxDecoration::new()
                            .background_color(background)
                            .border_radius(8),
                    )
                    .child(
                        Container::new()
                            .padding(LayoutSpacing::all(Spacing::Px(10)))
                            .child(
                                Text::new(label)
                                    .text_style(TextStyle::new().font_size(15).color(text)),
                            ),
                    )
                    .boxed(),
            );
        }
        Row::new().children(row)
    }
}

impl State<MorphTransitionExample> for MorphTransitionState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let card = &CARDS[self.card];

        // The child key is what starts a morph: it changes with the card, so
        // every switch keeps the outgoing card and blends it into the new one.
        // Changing only the style does not start one.
        // A linear curve keeps the noise threshold moving at a steady pace; an
        // ease-in-out one spends its first and last third nearly still.
        let morph = MorphTransition::new(
            Duration::from_millis(MORPH_MS),
            curves::LINEAR,
            card_content(card),
        )
        .style(STYLES[self.style].0)
        .direction(1.0, 0.0)
        .child_key(card.key);

        let header = &HEADERS[self.header];

        let heading = |text: &'static str| {
            Text::new(text)
                .text_style(
                    TextStyle::new()
                        .font_size(15)
                        .font_weight(FontWeight::Bold)
                        .color(theme.on_background_color),
                )
                .boxed()
        };

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(28)))
            .child(Column::new().children(vec![
                Text::new("Morph transition")
                    .text_style(
                        TextStyle::new()
                            .font_size(26)
                            .font_weight(FontWeight::Bold)
                            .color(theme.on_background_color),
                    )
                    .boxed(),
                SizedBox::new().height(8).boxed(),
                Text::new(
                    "Pick a card. Both cards are drawn to textures and one shader reads \
                     them together, so the blend follows what they actually paint.",
                )
                .wrapped()
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(theme::muted_text(&theme)),
                )
                .boxed(),
                SizedBox::new().height(16).boxed(),
                heading("Card"),
                SizedBox::new().height(6).boxed(),
                self.choices(
                    &theme,
                    CARDS.iter().map(|card| card.title),
                    self.card,
                    |state, index| state.card = index,
                )
                .boxed(),
                SizedBox::new().height(14).boxed(),
                heading("Style"),
                SizedBox::new().height(6).boxed(),
                self.choices(
                    &theme,
                    STYLES.iter().map(|(_, label)| *label),
                    self.style,
                    |state, index| state.style = index,
                )
                .boxed(),
                SizedBox::new().height(14).boxed(),
                heading("Header"),
                SizedBox::new().height(6).boxed(),
                self.choices(
                    &theme,
                    HEADERS.iter().map(|header| header.label),
                    self.header,
                    |state, index| state.header = index,
                )
                .boxed(),
                SizedBox::new().height(24).boxed(),
                SizedBox::new()
                    .height(STAGE_HEIGHT as u32)
                    .child(morph)
                    .boxed(),
                SizedBox::new().height(20).boxed(),
                SizedBox::new()
                    .height(HEADER_HEIGHT as u32)
                    .child(navigation_bar(header))
                    .boxed(),
            ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_have_distinct_keys_so_every_switch_starts_a_morph() {
        for (i, a) in CARDS.iter().enumerate() {
            for b in &CARDS[i + 1..] {
                assert_ne!(a.key, b.key);
            }
        }
    }

    #[test]
    fn cards_differ_in_size_and_color_so_the_morph_is_visible() {
        for (i, a) in CARDS.iter().enumerate() {
            for b in &CARDS[i + 1..] {
                assert!(a.width != b.width || a.height != b.height);
                assert_ne!(a.color, b.color);
            }
        }
    }

    #[test]
    fn the_stage_fits_the_tallest_card() {
        assert!(CARDS.iter().all(|card| card.height <= STAGE_HEIGHT));
    }

    #[test]
    fn every_style_is_offered_once() {
        for (i, (a, _)) in STYLES.iter().enumerate() {
            assert!(STYLES[i + 1..].iter().all(|(b, _)| a != b));
        }
        assert_eq!(STYLES.len(), 5);
    }
}
