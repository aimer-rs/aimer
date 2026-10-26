use std::time::Duration;

use aimer::Dimension::Percent;
use aimer::animation::{AnimatedSwitcher, curves};
use aimer::screen::MediaQuery;
use aimer::style::{
    FontWeight, LayoutSpacing, Spacing, TextDecoration, TextStyle, Theme, ThemeData,
};
use aimer::{
    AnyWidget, AssetImage, BoxAlignment, BuildContext, Column, Container, Dimension, Row, SizedBox,
    State, StateUpdater, StatefulWidget, Text, TextButton, Widget, widget,
};

#[widget(Stateful)]
pub struct TestFadingAnimation;

pub struct SameLookingSectionState {
    current_index: usize,
    state: StateUpdater<Self>,
}

impl StatefulWidget for TestFadingAnimation {
    type State = SameLookingSectionState;

    fn create_state(self) -> Self::State {
        // The framework preserves the live state across parent rebuilds
        // (e.g. a window resize) by adopting it during reconciliation, so the
        // selected tab survives without any manual persistence — this only
        // needs to provide the initial value.
        SameLookingSectionState {
            current_index: 0,
            state: StateUpdater::new(),
        }
    }
}

const PLATFORMS: &[&str] = &["macOS", "iOS", "Web", "Android"];
const PLATFORM_IMAGE: &[&str] = &[
    "assets/macos_screenshot.png",
    "assets/ios_screenshot.png",
    "assets/web_screenshot.png",
    "assets/android_screenshot.png",
];
const PLATFORM_IMAGE_SWITCHER_KEY: &str = "platform-image-switcher";

fn platform_image(index: usize) -> &'static str {
    PLATFORM_IMAGE[index % PLATFORM_IMAGE.len()]
}

fn platform_image_switcher(index: usize) -> AnimatedSwitcher<AssetImage> {
    AnimatedSwitcher::new(
        Duration::from_millis(350),
        curves::FAST_OUT_SLOW_IN,
        AssetImage::new(platform_image(index)),
    )
    .child_key(PLATFORMS[index % PLATFORMS.len()])
    .key(PLATFORM_IMAGE_SWITCHER_KEY)
}

impl State<TestFadingAnimation> for SameLookingSectionState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.state = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);

        Container::new()
            .color(app_theme.background_color)
            .padding(app_padding(ctx))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .children(vec![
                        Container::new()
                            .height(100)
                            .child(
                                Text::new("Consistence Looking").text_style(
                                    TextStyle::new()
                                        .font_size(mobile_title(ctx))
                                        .color(app_theme.on_background_color)
                                        .font_weight(FontWeight::Bolder)
                                        .text_decoration(TextDecoration::Underline),
                                ),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(24)
                            .boxed(),
                        Container::new()
                            .height(if is_mobile(ctx) { 250 } else { 450 })
                            .child(platform_image_switcher(self.current_index))
                            .boxed(),
                        SizedBox::new()
                            .height(40)
                            .boxed(),
                        Row::new()
                            .horizontal_alignment(BoxAlignment::Center)
                            .vertical_alignment(BoxAlignment::Center)
                            .gaps(LayoutSpacing::horizontal(Spacing::Px(8)))
                            .children(self.build_platform_button_list(app_theme))
                            .boxed(),
                        SizedBox::new()
                            .height(40)
                            .boxed(),
                    ]),
            )
    }
}

impl SameLookingSectionState {
    fn build_platform_button_list(&self, app_theme: ThemeData) -> Vec<AnyWidget> {
        let selected = self.current_index;
        PLATFORMS
            .iter()
            .enumerate()
            .map({
                let updater = self.state;
                move |(i, l)| {
                    let index = i;
                    let is_selected = index == selected;
                    let font_weight = if selected == index {
                        FontWeight::Bolder
                    } else {
                        FontWeight::Normal
                    };

                    TextButton::new(*l)
                        .style(
                            TextStyle::new()
                                .font_size(20)
                                .color(if is_selected {
                                    app_theme.primary_color
                                } else {
                                    app_theme.on_background_color
                                })
                                .font_weight(font_weight)
                                .text_decoration(if is_selected {
                                    TextDecoration::Underline
                                } else {
                                    TextDecoration::None
                                }),
                        )
                        .hover_style(
                            TextStyle::new()
                                .font_size(20)
                                .color(if is_selected {
                                    app_theme.primary_color
                                } else {
                                    app_theme
                                        .primary_color
                                        .lighten(0.22)
                                })
                                .font_weight(font_weight)
                                .text_decoration(TextDecoration::Underline),
                        )
                        .on_press({
                            let updater = updater;
                            move || {
                                println!("animation demo: tab {index} pressed");
                                if updater
                                    .read_state()
                                    .current_index
                                    != index
                                {
                                    updater.set_state(move |s| s.current_index = index);
                                }
                            }
                        })
                        .boxed()
                }
            })
            .collect()
    }
}

pub fn app_padding(_: &BuildContext) -> LayoutSpacing {
    let horizontal_padding = 20f64;
    LayoutSpacing::new()
        .left(horizontal_padding)
        .right(horizontal_padding)
        .top(Spacing::Px(20))
        .bottom(Spacing::Px(20))
}

pub fn is_mobile(ctx: &BuildContext) -> bool {
    let window_size = MediaQuery::of(ctx).size;
    window_size.width < 600f32
}

pub fn resp_position(ctx: &BuildContext, wide: f32, slim: f32) -> Dimension {
    if is_mobile(ctx) {
        Percent(slim)
    } else {
        Percent(wide)
    }
}

pub fn mobile_title(ctx: &BuildContext) -> u32 {
    if is_mobile(ctx) { 30 } else { 44 }
}
