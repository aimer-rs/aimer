//! Lottie playback: real After Effects exports, each with what Aimer skipped.
//!
//! The animations are parsed and prepared once and shared, so rebuilding the
//! page (a play/pause press) never parses or tessellates anything again, and the
//! playheads carry on where they were.

use std::sync::{Arc, OnceLock};

use aimer::input::button::Button;
use aimer::lottie::{Composition, Diagnostic, Lottie, LottieScene};
use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontWeight, LayoutSpacing, TextAlign,
    TextStyle, Theme, ThemeData,
};
use aimer::{
    AnyWidget, BuildContext, Column, Container, Dimension, Row, SizedBox, State, StateView, StateUpdater,
    StatefulWidget, Text, Widget,
};

use crate::theme;

/// Cards per row of the gallery.
const PER_ROW: usize = 3;

/// An original fingerprint scan, then test animations from the Lottie for
/// Android project (Apache-2.0); see `assets/lottie/NOTICE.md`. The last one has
/// a solid background layer, which is not drawn yet.
const FILES: [(&str, &str); 8] = [
    ("Fingerprint scan", include_str!("../assets/lottie/FingerprintScan.json")),
    ("EllipseDirection", include_str!("../assets/lottie/EllipseDirection.json")),
    ("Rect4", include_str!("../assets/lottie/Rect4.json")),
    ("SplitPathTransform", include_str!("../assets/lottie/SplitPathTransform.json")),
    ("Scale0", include_str!("../assets/lottie/Scale0.json")),
    ("Polygon", include_str!("../assets/lottie/Polygon.json")),
    ("MaskNone", include_str!("../assets/lottie/MaskNone.json")),
    ("Rect1", include_str!("../assets/lottie/Rect1.json")),
];

struct Entry {
    name: &'static str,
    scene: Arc<LottieScene>,
    /// How many things in the file were skipped or replaced, and the first.
    skipped: usize,
    first_skipped: Option<String>,
}

fn gallery() -> &'static [Entry] {
    static GALLERY: OnceLock<Vec<Entry>> = OnceLock::new();
    GALLERY.get_or_init(|| {
        FILES
            .iter()
            .map(|(name, source)| {
                let composition = Composition::parse(source)
                    .unwrap_or_else(|error| panic!("bundled animation {name}: {error}"));
                let scene = LottieScene::new(composition);
                let diagnostics: Vec<_> = scene.composition().diagnostics.iter().collect();
                Entry {
                    name,
                    skipped: diagnostics.len(),
                    first_skipped: diagnostics.first().map(|d| short(d)),
                    scene: Arc::new(scene),
                }
            })
            .collect()
    })
}

/// A diagnostic in few enough words to fit under a card.
fn short(diagnostic: &Diagnostic) -> String {
    match diagnostic {
        Diagnostic::Unsupported { feature, .. } => format!("no {feature}"),
        Diagnostic::Expression { .. } => "expression ignored".to_string(),
        Diagnostic::Malformed { what, .. } => format!("bad {what}"),
        other => other.to_string(),
    }
}

/// Builds the Lottie showcase page.
pub fn lottie_example() -> impl Widget {
    LottieExample::new()
}

#[derive(aimer::StatefulWidget)]
pub struct LottieExample;

impl LottieExample {
    pub fn new() -> Self {
        Self
    }
}

pub struct LottieExampleState {
    /// Kept for the helper methods that call back into the state.
    updater: StateUpdater<Self>,
    playing: bool,
    slow: bool,
}

impl State<LottieExample> for LottieExampleState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(widget: LottieExample) -> Self {
        LottieExampleState { playing: true, slow: false, updater: StateUpdater::empty() }
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = this.updater();
        let (playing, slow) = (this.playing, this.slow);

        let toggle = control_button(if playing { "Pause" } else { "Play" }, app_theme, true, move || {
            updater.set_state(|state| state.playing = !state.playing);
        });
        let speed = control_button(
            if slow { "Speed: 0.25x" } else { "Speed: 1x" },
            app_theme,
            false,
            move || updater.set_state(|state| state.slow = !state.slow),
        );

        let cards: Vec<AnyWidget> = gallery()
            .chunks(PER_ROW)
            .flat_map(|row| {
                let mut cells: Vec<AnyWidget> = Vec::new();
                for entry in row {
                    cells.push(card(entry, app_theme, playing, slow));
                    cells.push(SizedBox::new().width(16).boxed());
                }
                [Row::new().children(cells).boxed(), SizedBox::new().height(16).boxed()]
            })
            .collect();

        let mut page: Vec<AnyWidget> = vec![
            Text::new("Cards list what each file uses that is not drawn yet.")
                .text_style(TextStyle::new().font_size(14).color(theme::muted_text(&app_theme)))
                .boxed(),
            SizedBox::new().height(16).boxed(),
            Row::new()
                .children(vec![toggle, SizedBox::new().width(12).boxed(), speed])
                .boxed(),
            SizedBox::new().height(20).boxed(),
        ];
        page.extend(cards);

        Container::new()
            .padding(LayoutSpacing::all(28))
            .child(Column::new().children(page))
    }
}

fn card(entry: &Entry, app_theme: ThemeData, playing: bool, slow: bool) -> AnyWidget {
    let note = match (&entry.first_skipped, entry.skipped) {
        (None, _) => "nothing skipped".to_string(),
        (Some(first), 1) => format!("1 skipped: {first}"),
        (Some(first), n) => format!("{n} skipped: {first}"),
    };
    Container::new()
        .width(Dimension::Px(196.0))
        .padding(LayoutSpacing::all(12))
        .decoration(
            BoxDecoration::new()
                .background_color(theme::raised_surface(&app_theme))
                .border_radius(12),
        )
        .child(Column::new().children(vec![
            Lottie::new()
                .width(172.0)
                .playing(playing)
                .speed(if slow { 0.25 } else { 1.0 })
                .animation(Arc::clone(&entry.scene))
                .boxed(),
            SizedBox::new().height(8).boxed(),
            Text::new(entry.name)
                .text_style(
                    TextStyle::new()
                        .font_size(13)
                        .font_weight(FontWeight::Bold)
                        .color(app_theme.on_surface_color),
                )
                .boxed(),
            Text::new(note)
                .text_style(TextStyle::new().font_size(11).color(theme::muted_text(&app_theme)))
                .boxed(),
        ]))
        .boxed()
}

fn control_button(
    label: &'static str,
    app_theme: ThemeData,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let (background, foreground) = if primary {
        (app_theme.primary_color, app_theme.on_primary_color)
    } else {
        (aimer::Color::Transparent, app_theme.primary_color)
    };
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border(if primary {
                    BoxBorder::default()
                } else {
                    BoxBorder::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(1.0)
                            .color(app_theme.primary_color),
                    )
                })
                .border_radius(8),
        )
        .child(
            Container::new()
                .height(Dimension::Px(32.0))
                .padding(LayoutSpacing::new().left(14).right(14))
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_builds_as_a_widget_without_starting_the_application() {
        fn assert_widget(_widget: impl Widget) {}
        assert_widget(lottie_example());
    }

    #[test]
    fn every_bundled_animation_loads() {
        assert_eq!(gallery().len(), FILES.len());
    }
}
