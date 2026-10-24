//! A runnable showcase for the W14 Glass and Liquid surface containers.
//!
//! W17 wires the public exports into the root workspace. The example uses only
//! their public builders and keeps the bounded fallback available when Cupid
//! cannot capture the current backdrop.

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::shape::{FillStyle, ShapeColor, ShapeFit, ShapePath, ShapePathBuilder};
use aimer::style::{BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle};
use aimer::{
    AimerApp, AnyWidget, BoxAlignment, BuildContext, Colors, Column, Container, CustomShape,
    Dimension, Glass, Liquid, MaterialMotionPolicy, Positioned, Row, Stack, State, StateUpdater,
    StatefulWidget, Text, Widget, ZeroSizedBox,
};

const SURFACE_WIDTH: f32 = 720.0;
const SURFACE_HEIGHT: f32 = 660.0;
const PATTERN_START_Y: f32 = 128.0;
const INITIAL_BLUR_INTENSITY: f32 = 0.72;
const BLUR_INTENSITY_STEP: f32 = 0.10;

fn zig_zag_path() -> ShapePath {
    let mut builder = ShapePathBuilder::new();
    let row_height = 80.0;
    let amplitude = 48.0;
    let ribbon_half_width = 11.0;

    for row in 0..7 {
        let base_y = PATTERN_START_Y + row as f32 * row_height;
        let mut points = Vec::new();
        let mut x = -48.0;
        while x <= SURFACE_WIDTH + 48.0 {
            let center_y = base_y
                + if ((x + 48.0) / 80.0).round() as i32 % 2 == 0 {
                    0.0
                } else {
                    amplitude
                };
            points.push((x, center_y));
            x += 80.0;
        }
        let &(first_x, first_center_y) = points
            .first()
            .expect("each static zig-zag row must have a first point");
        builder = builder.move_to(first_x, first_center_y - ribbon_half_width);
        for &(point_x, center_y) in points
            .iter()
            .skip(1)
        {
            builder = builder.line_to(point_x, center_y - ribbon_half_width);
        }
        for &(point_x, center_y) in points.iter().rev() {
            builder = builder.line_to(point_x, center_y + ribbon_half_width);
        }
        builder = builder.close();
    }

    builder
        .build()
        .expect("the static zig-zag backdrop must remain finite")
}

fn zig_zag_backdrop() -> aimer::AnyWidget {
    CustomShape::new()
        .path(zig_zag_path())
        .fill(FillStyle::solid(ShapeColor::BLACK))
        .fit(ShapeFit::None)
        .child(
            Container::new()
                .width(Dimension::Px(SURFACE_WIDTH))
                .height(Dimension::Px(SURFACE_HEIGHT))
                .child(ZeroSizedBox),
        )
        .boxed()
}
/// Builds the Glass/Liquid showcase without starting an application.
pub fn glass_liquid_example() -> impl Widget {
    GlassLiquidExample::new()
}

/// Starts the standalone Glass/Liquid showcase.
pub fn start_glass_liquid_example() {
    AimerApp::start(glass_liquid_example());
}

/// A small page that exercises static, dynamic, and reduced-motion surfaces.
#[widget(Stateful)]
pub struct GlassLiquidExample {}

impl GlassLiquidExample {
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }

    fn card(title: &'static str, body: &'static str) -> aimer::AnyWidget {
        Container::new()
            .width(Dimension::Px(320.0))
            .height(Dimension::Px(128.0))
            .padding(LayoutSpacing::all(Spacing::Px(20)))
            .child(
                Column::new().children([
                    Text::new(title)
                        .text_style(
                            TextStyle::new()
                                .font_size(20)
                                .font_weight(FontWeight::Bold)
                                .color(Colors::White),
                        )
                        .boxed(),
                    Text::new(body)
                        .text_style(
                            TextStyle::new()
                                .font_size(14)
                                .color(aimer::Color::Rgba(238, 246, 255, 232)),
                        )
                        .wrapped()
                        .boxed(),
                ]),
            )
            .boxed()
    }
}

impl Default for GlassLiquidExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct GlassLiquidExampleState {
    blur_intensity: f32,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for GlassLiquidExample {
    type State = GlassLiquidExampleState;

    fn create_state(self) -> Self::State {
        GlassLiquidExampleState {
            blur_intensity: INITIAL_BLUR_INTENSITY,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<GlassLiquidExample> for GlassLiquidExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let glass = Glass::new()
            .tint(aimer::Color::Rgba(64, 148, 214, 255))
            .opacity(0.78)
            .blur_intensity(self.blur_intensity)
            .saturation(1.15)
            .brightness(1.08)
            .elevation(1.0)
            // .contrast(0.9)
            .border_color(aimer::Color::Rgba(198, 238, 255, 214))
            // .border_radius(22.0)
            .quality(3)
            .child(GlassLiquidExample::card(
                "Glass",
                "Blue frosted backdrop with adjustable diffusion of the zig-zag scene.",
            ));

        let liquid = Liquid::new()
            .tint(aimer::Color::Rgba(210, 236, 250, 255))
            .opacity(0.5)
            .border_color(aimer::Color::Rgba(255, 255, 255, 160))
            .distortion_strength(0.15)
            .edge_lighting(0.55)
            .specular_highlight(0.55)
            .magnification(0.6)
            .chromatic_aberration(0.4)
            .animation_speed(0.5)
            .animation_time(1.25)
            .interaction(0.4)
            .child(GlassLiquidExample::card(
                "Liquid",
                "Apple Liquid Glass style: refraction concentrated at the curved rim, an adaptive tint, a crisp edge sheen, and a chromatic-aberration fringe — no blob wobble, no bokeh.",
            ));

        let teardrop = Liquid::new()
            .tint(aimer::Color::Rgba(214, 236, 250, 255))
            .opacity(0.46)
            .border_color(aimer::Color::Rgba(255, 255, 255, 150))
            .distortion_strength(0.2)
            .edge_lighting(0.7)
            .specular_highlight(0.6)
            .magnification(0.5)
            .blob_amount(0.55)
            .blob_seed(0.85)
            .tip_pull(0.7)
            .child(GlassLiquidExample::card(
                "Liquid · teardrop",
                "blob_amount and tip_pull bend the rounded-rect outline into an organic, pointed droplet.",
            ));

        let reduced = Liquid::new()
            .tint(aimer::Color::Rgba(0, 82, 150, 255))
            .opacity(0.6)
            .border_color(aimer::Color::Rgba(166, 232, 255, 192))
            .distortion_strength(0.9)
            .animation_speed(3.0)
            .motion_policy(MaterialMotionPolicy::Reduced)
            .child(GlassLiquidExample::card(
                "Liquid · reduced motion",
                "Motion policy removes refraction ripple and animation while preserving the child.",
            ));

        let content = Container::new()
            .width(Dimension::Px(SURFACE_WIDTH))
            .height(Dimension::Px(SURFACE_HEIGHT))
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::all(4
                    ))
                    .children([
                    Text::new("Glass and Liquid materials")
                        .text_style(
                            TextStyle::new()
                                .font_size(28)
                                .font_weight(FontWeight::Bold)
                                .color(aimer::Colors::Black),
                        )
                        .boxed(),
                    Row::new()
                        .vertical_alignment(BoxAlignment::Center)
                        .gaps(LayoutSpacing::all(Spacing::Px(12)))
                        .children([
                            Container::new()
                                .width(Dimension::Px(400.0))
                                .child(
                                    Text::new(
                                        "Glass diffuses the zig-zag scene behind it. Adjust the blur intensity; Liquid still refracts it with a water-like ripple.",
                                    )
                                    .wrapped()
                                    .boxed(),
                                )
                                .boxed(),
                            self.blur_control(),
                        ])
                        .boxed(),
                    glass.boxed(),
                    liquid.boxed(),
                    teardrop.boxed(),
                    reduced.boxed(),
                ]),
            );

        Container::new()
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .color(aimer::Color::Rgba(246, 246, 246, 255))
            .child(
                Container::new()
                    .width(Dimension::Px(SURFACE_WIDTH))
                    .height(Dimension::Px(SURFACE_HEIGHT))
                    .child(
                        Stack::new()
                            .add_child(
                                Container::new()
                                    .width(Dimension::Px(SURFACE_WIDTH))
                                    .height(Dimension::Px(SURFACE_HEIGHT))
                                    .color(aimer::Color::Rgba(255, 255, 255, 255))
                                    .child(ZeroSizedBox),
                            )
                            .add_child(
                                Positioned::new()
                                    .left(0.0)
                                    .top(0.0)
                                    .layer(1)
                                    .child(zig_zag_backdrop()),
                            )
                            .add_child(
                                Positioned::new()
                                    .left(0.0)
                                    .top(0.0)
                                    .layer(2)
                                    .child(content),
                            ),
                    ),
            )
    }
}

impl GlassLiquidExampleState {
    fn blur_control(&self) -> AnyWidget {
        let decrease = self.updater;
        let increase = self.updater;
        let reset = self.updater;

        Row::new()
            .vertical_alignment(BoxAlignment::Center)
            .gaps(LayoutSpacing::all(Spacing::Px(6)))
            .children([
                Text::new(format!("Blur: {:.0}%", self.blur_intensity * 100.0))
                    .text_style(
                        TextStyle::new()
                            .font_size(13)
                            .color(Colors::Black),
                    )
                    .boxed(),
                blur_button("−", move || {
                    decrease.set_state(|state| {
                        state.blur_intensity =
                            (state.blur_intensity - BLUR_INTENSITY_STEP).max(0.0);
                    });
                }),
                blur_button("+", move || {
                    increase.set_state(|state| {
                        state.blur_intensity =
                            (state.blur_intensity + BLUR_INTENSITY_STEP).min(1.0);
                    });
                }),
                blur_button("Reset", move || {
                    reset.set_state(|state| state.blur_intensity = INITIAL_BLUR_INTENSITY);
                }),
            ])
            .boxed()
    }
}

fn blur_button(label: &'static str, on_press: impl Fn() + 'static) -> AnyWidget {
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(aimer::Color::Rgba(25, 86, 128, 230))
                .border_radius(8),
        )
        .child(
            Container::new()
                .height(Dimension::Px(32.0))
                .padding(
                    LayoutSpacing::new()
                        .left(8)
                        .right(8),
                )
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .font_weight(FontWeight::Bold)
                                .color(Colors::White),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer::LiquidMaterial;

    #[test]
    fn showcase_constructor_is_a_widget() {
        fn assert_widget(_: impl Widget) {}
        assert_widget(glass_liquid_example());
    }

    #[test]
    fn showcase_uses_the_reduced_motion_policy_publicly() {
        let material = LiquidMaterial::new()
            .distortion_strength(0.8)
            .animation_speed(2.0)
            .motion_policy(MaterialMotionPolicy::Reduced);
        assert_eq!(material.effective_distortion(), 0.0);
        assert_eq!(material.effective_animation_speed(), 0.0);
    }

    #[test]
    fn zig_zag_backdrop_path_is_finite_and_bounded() {
        let bounds = zig_zag_path().bounds();
        assert!(
            bounds
                .min
                .x
                .is_finite()
        );
        assert!(
            bounds
                .min
                .y
                .is_finite()
        );
        assert!(
            bounds
                .max
                .x
                .is_finite()
        );
        assert!(
            bounds
                .max
                .y
                .is_finite()
        );
        assert!(bounds.max.x > bounds.min.x);
        assert!(bounds.max.y > bounds.min.y);
    }
}
