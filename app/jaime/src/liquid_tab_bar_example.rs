//! A tab bar whose selection indicator turns into a liquid glass lens while it
//! moves, as in the iOS 26 tab bar.
//!
//! Tap a tab and the indicator lifts into a lens that magnifies the icon and
//! label it passes over, overshoots a little, and settles back into a flat pill.
//! Press a tab and drag to carry the lens along the bar with the pointer: it
//! stays lifted while it is held, and lands on the nearest tab when let go.
//!
//! Both bars below are the same widget. What differs is the two things passed
//! to it, each an implementation of a trait: how the indicator looks
//! (`IndicatorStyle`) and how it travels (`Motion`).

use aimer::animation::{Motion, motions};
use aimer::indicator::{IndicatorStyle, TabBar, TabItem, indicator_styles};
use aimer::macros::widget;
use aimer::style::{Color, FontWeight, LayoutSpacing, Spacing, TextStyle};
use aimer::svg::{Svg, SvgColor, SvgDocument, SvgStyle};
use aimer::{
    AimerApp, AnyWidget, BuildContext, Column, State, StateUpdater, StatefulWidget, Text, Widget,
};
use std::time::Duration;

use crate::theme;

const CLOCK: &str = r##"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
  <circle cx="12" cy="12" r="10"/>
  <path fill="#ffffff" d="M11 6.5h2v5.1l3.5 2.1-1 1.7L11 12.7z"/>
</svg>"##;

const PEOPLE: &str = r##"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
  <circle cx="9" cy="8" r="3.6"/>
  <circle cx="17.2" cy="9" r="2.8"/>
  <path d="M2 19.5c0-3.4 3-5.6 7-5.6s7 2.2 7 5.6z"/>
  <path d="M15.6 14.2c3.5.1 6.4 1.7 6.4 5.3h-4.2c0-2.1-.8-3.9-2.2-5.3z"/>
</svg>"##;

const FOLDER: &str = r##"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
  <path d="M2 6.5A2.5 2.5 0 0 1 4.5 4h4.2c.6 0 1.1.2 1.5.6L11.6 6h7.9A2.5 2.5 0 0 1 22 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 2 17.5z"/>
</svg>"##;

const TITLES: [&str; 3] = ["Recents", "Shared", "Browse"];

/// Starts the standalone liquid tab bar example.
pub fn start_liquid_tab_bar_example() {
    AimerApp::start(theme::provide(liquid_tab_bar_example()));
}

/// Builds two tab bars that share one selection but differ in style and motion.
pub fn liquid_tab_bar_example() -> impl Widget {
    LiquidTabBarExample::new()
}

/// A 24-pixel icon painted in `tint`. `selector` picks the shapes that take the
/// tint, so the clock's white hands keep their colour.
fn icon(svg: &'static str, selector: &'static str, tint: Color) -> Svg {
    let (red, green, blue, alpha) = tint.to_rgba();
    let document = SvgDocument::from_svg(svg.as_bytes()).expect("the bundled icon is valid SVG");
    Svg::new(document)
        .width(24.0)
        .height(24.0)
        .style(
            selector,
            SvgStyle::new().fill(SvgColor::rgba8(red, green, blue, alpha)),
        )
}

/// Demonstrates the sliding indicator through a tab bar.
#[derive(StatefulWidget)]
pub struct LiquidTabBarExample {}

impl LiquidTabBarExample {
    /// Creates the example with the second tab selected.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for LiquidTabBarExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LiquidTabBarExampleState {
    selected: usize,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for LiquidTabBarExample {
    type State = LiquidTabBarExampleState;

    fn create_state(self) -> Self::State {
        LiquidTabBarExampleState {
            selected: 1,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<LiquidTabBarExample> for LiquidTabBarExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let app_theme = theme::app_theme();
        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(20)))
            .children([
                Text::new(
                    "Tap a tab, or press one and drag. The indicator lifts into a lens while it \
                     moves and settles back when you let go.",
                )
                .text_style(
                    TextStyle::new()
                        .font_size(15)
                        .color(app_theme.on_background_color),
                )
                .wrapped()
                .boxed(),
                caption("Liquid glass indicator, bouncy spring", &app_theme),
                self.bar(indicator_styles::LIQUID_GLASS, motions::BOUNCY),
                caption("Flat pill indicator, smooth spring", &app_theme),
                self.bar(indicator_styles::FLAT_PILL, motions::SMOOTH),
                Text::new(format!("Selected: {}", TITLES[self.selected]))
                    .text_style(
                        TextStyle::new()
                            .font_size(14)
                            .font_weight(FontWeight::Value(500))
                            .color(theme::muted_text(&app_theme)),
                    )
                    .boxed(),
            ])
    }
}

fn caption(text: &'static str, app_theme: &aimer::style::ThemeData) -> AnyWidget {
    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(13)
                .font_weight(FontWeight::Value(600))
                .color(theme::muted_text(app_theme)),
        )
        .boxed()
}

impl LiquidTabBarExampleState {
    /// A bar over the shared selection, looking and moving as the arguments say.
    fn bar(&self, style: impl IndicatorStyle, motion: impl Motion) -> AnyWidget {
        let updater = self.updater;
        TabBar::new()
            .selected(self.selected)
            .on_select(move |index| updater.set_state(move |state| state.selected = index))
            .style(style)
            .fade_duration(Duration::from_millis(120))
            .motion(motion)
            .item(
                TabItem::new()
                    .label(TITLES[0])
                    .icon(|tint| icon(CLOCK, "path", tint)),
            )
            .item(
                TabItem::new()
                    .label(TITLES[1])
                    .icon(|tint| icon(PEOPLE, "path", tint)),
            )
            .item(
                TabItem::new()
                    .label(TITLES[2])
                    .icon(|tint| icon(FOLDER, "path", tint)),
            )
            .boxed()
    }
}
