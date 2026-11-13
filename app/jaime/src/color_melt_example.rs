//! A color-melting shader effect with live controls.
//!
//! The picture is one application-defined [`ShaderEffect`]: a `.aimer` shader
//! (`color_melt_example/color_melt.aimer`) that the renderer translates for
//! whichever backend is running. This page shows the three pieces an app needs:
//! the effect type that names the shader and writes its parameters, the one-time
//! registration, and a painter that sends a draw command for the box it is in.
//! The sliders and palette buttons change the parameters the next frame sends.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer::animation::{AnimatedBuilder, AnimationController, curves};
use aimer::effects::{self, EffectCommand, ParamWriter, ShaderEffect};
use aimer::range::{Slider, SliderThumb, SliderTrail};
use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextDecoration, TextStyle, Theme, ThemeData,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, CanvasPainter, Column, Container, CustomPaint, Row,
    ScrollAxis, Scrollable, State, StateView, StateUpdater, StatefulWidget, Text, TextButton, Widget,
};
use aimer::ResolvedSize;
use aimer::canvas::Canvas;
use aimer_time::Instant;

use crate::theme;

/// Builds the color-melt page without starting the application.
///
/// The picture and the six controls are taller than a small window, so the page
/// scrolls vertically.
pub fn color_melt_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(ColorMeltExample::new())
}

/// Starts the standalone color-melt showcase.
pub fn start_color_melt_example() {
    AimerApp::start(theme::provide(color_melt_example()));
}

// ── The effect ───────────────────────────────────────────────────────────────

/// One frame of the melting colors: the shader's parameters, in the order its
/// `Frame` struct declares them after the shared header.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Melt {
    /// Seconds of animation, already multiplied by the speed control.
    time: f32,
    scale: f32,
    melt: f32,
    contrast: f32,
    radius: f32,
    grain: f32,
    colors: [[f32; 4]; 4],
}

impl ShaderEffect for Melt {
    const NAME: &'static str = "jaime-color-melt";
    const SOURCE: &'static str = include_str!("color_melt_example/color_melt.aimer");

    fn write_params(&self, params: &mut ParamWriter) {
        params
            .f32(self.time)
            .f32(self.scale)
            .f32(self.melt)
            .f32(self.contrast)
            .f32(self.radius)
            .f32(self.grain)
            // The shader's `_pad0` and `_pad1`.
            .pad(8);
        for color in self.colors {
            params.vec4(color);
        }
    }
}

/// Registers the shader once; further calls do nothing.
///
/// A failure is the shader or this file being wrong, not the user's machine, so
/// it is reported loudly and the page then shows an empty box.
fn register_effect() {
    if let Err(error) = effects::register::<Melt>() {
        eprintln!("color melt: {error}");
    }
}

/// Sends the effect's draw command for the whole box.
struct MeltPainter(Melt);

impl CanvasPainter for MeltPainter {
    fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
        if let Ok(command) = EffectCommand::new(&self.0, [0.0, 0.0, size.width, size.height]) {
            canvas.draw_custom(command.name, command.payload);
        }
    }
}

// ── What the controls change ─────────────────────────────────────────────────

/// A named set of four colors, authored in sRGB.
struct Palette {
    name: &'static str,
    colors: [[f32; 4]; 4],
}

const PALETTES: [Palette; 4] = [
    Palette {
        name: "Sunset",
        colors: [
            [1.00, 0.55, 0.20, 1.0],
            [0.96, 0.20, 0.42, 1.0],
            [0.52, 0.18, 0.78, 1.0],
            [0.10, 0.12, 0.45, 1.0],
        ],
    },
    Palette {
        name: "Lava",
        colors: [
            [1.00, 0.85, 0.25, 1.0],
            [0.95, 0.35, 0.10, 1.0],
            [0.60, 0.08, 0.10, 1.0],
            [0.12, 0.03, 0.06, 1.0],
        ],
    },
    Palette {
        name: "Ocean",
        colors: [
            [0.55, 0.95, 0.90, 1.0],
            [0.10, 0.65, 0.85, 1.0],
            [0.08, 0.30, 0.65, 1.0],
            [0.03, 0.08, 0.30, 1.0],
        ],
    },
    Palette {
        name: "Candy",
        colors: [
            [1.00, 0.80, 0.90, 1.0],
            [0.75, 0.55, 1.00, 1.0],
            [0.40, 0.85, 0.95, 1.0],
            [1.00, 0.95, 0.60, 1.0],
        ],
    },
];

/// The inclusive range of each slider.
const SPEED: (f32, f32) = (0.0, 3.0);
const SCALE: (f32, f32) = (0.4, 3.0);
const MELT: (f32, f32) = (0.0, 2.0);
const CONTRAST: (f32, f32) = (1.0, 8.0);
const RADIUS: (f32, f32) = (0.0, 0.5);
const GRAIN: (f32, f32) = (0.0, 1.0);

/// Everything the controls set.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Props {
    /// Animation speed; `0` freezes the picture.
    speed: f32,
    scale: f32,
    melt: f32,
    contrast: f32,
    radius: f32,
    grain: f32,
    /// Index into [`PALETTES`].
    palette: usize,
}

impl Props {
    const DEFAULT: Self = Self {
        speed: 1.0,
        scale: 1.2,
        melt: 1.0,
        contrast: 2.5,
        radius: 0.06,
        grain: 0.3,
        palette: 0,
    };

    /// Forces every value into its slider's range.
    ///
    /// A value that is not a number falls back to its default, so a bad input
    /// can never reach the shader.
    fn clamped(self) -> Self {
        let fit = |value: f32, (low, high): (f32, f32), default: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                default
            }
        };
        Self {
            speed: fit(self.speed, SPEED, Self::DEFAULT.speed),
            scale: fit(self.scale, SCALE, Self::DEFAULT.scale),
            melt: fit(self.melt, MELT, Self::DEFAULT.melt),
            contrast: fit(self.contrast, CONTRAST, Self::DEFAULT.contrast),
            radius: fit(self.radius, RADIUS, Self::DEFAULT.radius),
            grain: fit(self.grain, GRAIN, Self::DEFAULT.grain),
            palette: self.palette.min(PALETTES.len() - 1),
        }
    }

    /// The effect to draw at `time` seconds of animation.
    fn effect(&self, time: f32) -> Melt {
        Melt {
            time,
            scale: self.scale,
            melt: self.melt,
            contrast: self.contrast,
            radius: self.radius,
            grain: self.grain,
            colors: PALETTES[self.palette.min(PALETTES.len() - 1)].colors,
        }
    }
}

/// Animation time that keeps running when the speed changes.
///
/// Time is the integral of speed, not `elapsed * speed`: dragging the speed
/// slider must slow the picture down from where it is, not jump it to a
/// different point of the animation.
struct MeltClock {
    time: Cell<f64>,
    last: Cell<Option<Instant>>,
}

/// The longest step one tick may add. A stalled frame (a drag, a debugger)
/// resumes where it left off instead of leaping forward.
const MAX_STEP: f64 = 0.1;

impl MeltClock {
    fn new() -> Self {
        Self {
            time: Cell::new(0.0),
            last: Cell::new(None),
        }
    }

    /// Adds `seconds` of real time at `speed`, and returns the animation time.
    fn advance(&self, seconds: f64, speed: f32) -> f32 {
        // A step that is not a plain forward amount of time adds nothing.
        if seconds.is_finite() && seconds > 0.0 && speed.is_finite() && speed > 0.0 {
            self.time.set(self.time.get() + seconds * f64::from(speed));
        }
        self.time.get() as f32
    }

    /// Advances by the real time since the previous tick.
    fn tick(&self, now: Instant, speed: f32) -> f32 {
        let step = self
            .last
            .replace(Some(now))
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64().min(MAX_STEP));
        self.advance(step, speed)
    }
}

// ── The page ─────────────────────────────────────────────────────────────────

/// The color-melt showcase page.
#[derive(StatefulWidget)]
pub struct ColorMeltExample {}

impl ColorMeltExample {
    /// Creates the page with the default controls.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for ColorMeltExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ColorMeltExampleState {
    /// Shared with the animation closure, which outlives any one page rebuild,
    /// so the picture always reads the latest controls.
    props: Rc<Cell<Props>>,
    clock: Rc<MeltClock>,
    /// Only drives frames: its value is never read.
    ticker: AnimationController,
}

impl State<ColorMeltExample> for ColorMeltExampleState {
    fn create(widget: ColorMeltExample) -> Self {
        register_effect();
        let ticker = AnimationController::new(Duration::from_secs(1), curves::LINEAR);
        ticker.set_repeat(true);
        ticker.forward_from_first_tick();
        ColorMeltExampleState {
            props: Rc::new(Cell::new(Props::DEFAULT)),
            clock: Rc::new(MeltClock::new()),
            ticker,
        }
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let props = this.props.get();

        let picture = {
            let props = Rc::clone(&this.props);
            let clock = Rc::clone(&this.clock);
            AnimatedBuilder::new(this.ticker.clone(), move |_| {
                let current = props.get();
                let time = clock.tick(Instant::now(), current.speed);
                CustomPaint::new()
                    .height(360)
                    .painter(MeltPainter(current.effect(time)))
            })
        };

        let slider = |label: &'static str,
                      value: f32,
                      (low, high): (f32, f32),
                      step: f32,
                      set: fn(&mut Props, f32)| {
            let shared = Rc::clone(&this.props);
            let updater = this.updater();
            let control = Slider::new()
                .range(f64::from(low)..f64::from(high))
                .step(f64::from(step))
                .value(f64::from(value))
                .width(320.0)
                .trail(
                    SliderTrail::new()
                        .height(6.0)
                        .radius(3.0)
                        .color(app_theme.primary_color),
                )
                .thumb(
                    SliderThumb::new()
                        .size(20.0)
                        .radius(10.0)
                        .color(app_theme.primary_color),
                )
                .on_change(move |next| {
                    let mut props = shared.get();
                    set(&mut props, next as f32);
                    shared.set(props.clamped());
                    // The picture reads `shared` itself; this refreshes the
                    // readouts and slider positions.
                    updater.set_state(|_| {});
                });
            labelled(label, format!("{value:.2}"), control.boxed(), app_theme)
        };

        let palette_buttons = PALETTES
            .iter()
            .enumerate()
            .map(|(index, palette)| {
                let selected = index == props.palette;
                let shared = Rc::clone(&this.props);
                let updater = this.updater();
                let style = TextStyle::new()
                    .font_size(16)
                    .font_weight(if selected { FontWeight::Bolder } else { FontWeight::Normal })
                    .color(if selected {
                        app_theme.primary_color
                    } else {
                        app_theme.on_background_color
                    })
                    .text_decoration(if selected {
                        TextDecoration::Underline
                    } else {
                        TextDecoration::None
                    });
                TextButton::new(palette.name)
                    .style(style.clone())
                    .hover_style(style.color(app_theme.primary_color.lighten(0.22)))
                    .on_press(move || {
                        let mut props = shared.get();
                        props.palette = index;
                        shared.set(props);
                        updater.set_state(|_| {});
                    })
                    .boxed()
            })
            .collect::<Vec<_>>();

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                    .children(vec![
                        Text::new("Color melt")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                        Text::new(
                            "A custom shader effect: the picture below is drawn by a .aimer \
                             shader the app registered, translated at run time for the \
                             running backend. It needs one that takes run-time shaders \
                             (Metal, Vulkan, Direct3D 12, OpenGL or WebGPU); elsewhere the \
                             box stays empty.",
                        )
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                        picture.boxed(),
                        Row::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(16)))
                            .children(palette_buttons)
                            .boxed(),
                        slider("Speed", props.speed, SPEED, 0.05, |p, v| p.speed = v),
                        slider("Scale", props.scale, SCALE, 0.05, |p, v| p.scale = v),
                        slider("Melt", props.melt, MELT, 0.05, |p, v| p.melt = v),
                        slider("Edge contrast", props.contrast, CONTRAST, 0.1, |p, v| {
                            p.contrast = v
                        }),
                        slider("Corner radius", props.radius, RADIUS, 0.01, |p, v| p.radius = v),
                        slider("Grain", props.grain, GRAIN, 0.05, |p, v| p.grain = v),
                    ]),
            )
    }
}

/// A row: the label and its value above a control.
fn labelled(label: &'static str, value: String, control: AnyWidget, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(12)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(12),
        )
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                .children([
                    Text::new(format!("{label}: {value}"))
                        .text_style(
                            TextStyle::new()
                                .font_size(14)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                    control,
                ]),
        )
        .boxed()
}

#[cfg(test)]
mod tests;
