//! A button on fire, drawn by a [`ShaderEffectDecoration`].
//!
//! The flames are one `.aimer` shader (`fire_button_example/fire_button.aimer`)
//! used as a decoration behind a button. Pressing the button lets the fire burn
//! out: the flames shorten, tear into separate licks and vanish. Pressing it
//! again relights it. Nothing here animates the button itself; the decoration
//! is rebuilt each frame with the next fuel level and time, while the button is
//! built once and kept.
//!
//! When the fire is out the animation stops, so a cold button costs no frames.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer::animation::{AnimatedBuilder, AnimationController, curves};
use aimer::decoration::{DecorationBox, ShaderEffectDecoration};
use aimer::effects::{ParamWriter, ShaderEffect};
use aimer::input::gesture::gesture_detector::GestureDetector;
use aimer::style::{
    BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, Theme,
    ThemeData,
};
use aimer::{
    AimerApp, Center, Column, Container, Positioned, ScrollAxis, Scrollable, Stack, State, StateView, StateUpdater,
    StatefulWidget, Text, Widget,
};
use aimer_time::Instant;

use crate::theme;

/// Builds the fire-button page without starting the application.
pub fn fire_button_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(FireButtonExample::new())
}

/// Starts the standalone fire-button showcase.
pub fn start_fire_button_example() {
    AimerApp::start(theme::provide(fire_button_example()));
}

// ── Layout ───────────────────────────────────────────────────────────────────

/// The fire's box, in logical pixels. It has no background of its own: the
/// flames are the only thing drawn in it, and they rise from the button inside.
const BOX_WIDTH: f32 = 360.0;
const BOX_HEIGHT: f32 = 320.0;
/// The button, at the center of the box.
const BUTTON_WIDTH: f32 = 200.0;
const BUTTON_HEIGHT: f32 = 64.0;
const BUTTON_LEFT: f32 = (BOX_WIDTH - BUTTON_WIDTH) / 2.0;
const BUTTON_TOP: f32 = (BOX_HEIGHT - BUTTON_HEIGHT) / 2.0;

/// The button's center and half size as fractions of the box, which is how the
/// shader receives them so the picture is independent of the display scale.
const BUTTON_FRACTIONS: [f32; 4] = [
    0.5,
    (BUTTON_TOP + BUTTON_HEIGHT / 2.0) / BOX_HEIGHT,
    BUTTON_WIDTH / 2.0 / BOX_WIDTH,
    BUTTON_HEIGHT / 2.0 / BOX_HEIGHT,
];

// ── The effect ───────────────────────────────────────────────────────────────

/// One frame of fire: the shader's parameters, in the order its `Frame` struct
/// declares them after the shared header.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fire {
    /// Seconds of animation.
    time: f32,
    /// Fuel, `0.0` (out) to `1.0` (full blaze).
    level: f32,
}

impl ShaderEffect for Fire {
    const NAME: &'static str = "jaime-fire-button";
    const SOURCE: &'static str = include_str!("fire_button_example/fire_button.aimer");

    fn write_params(&self, params: &mut ParamWriter) {
        params
            .f32(self.time)
            .f32(self.level)
            // A pill: the corner radius is the button's half height.
            .f32(1.0)
            // The shader's `_pad0`.
            .pad(4)
            .vec4(BUTTON_FRACTIONS);
    }
}

// ── The fuel ─────────────────────────────────────────────────────────────────

/// Seconds a full fire takes to burn out.
const EXHAUST_SECONDS: f64 = 3.5;
/// Seconds an empty fire takes to reach a full blaze.
const IGNITE_SECONDS: f64 = 0.6;
/// The longest step one tick may add, so a stalled frame (a drag, a debugger)
/// resumes where it left off instead of leaping.
const MAX_STEP: f64 = 0.1;

/// What the fire is doing, for the text under the button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Burning,
    /// Pressed while burning: the fuel is running out.
    Exhausting,
    Out,
}

/// The fire's fuel and clock.
///
/// The fuel moves toward full while the fire is wanted and toward empty
/// otherwise, so a press while the flames are half gone relights them from
/// there instead of snapping to a new state.
struct Fuel {
    level: Cell<f32>,
    wanted: Cell<bool>,
    time: Cell<f64>,
    last: Cell<Option<Instant>>,
}

impl Fuel {
    /// A full fire, already burning.
    fn new() -> Self {
        Self {
            level: Cell::new(1.0),
            wanted: Cell::new(true),
            time: Cell::new(0.0),
            last: Cell::new(None),
        }
    }

    fn stage(&self) -> Stage {
        if self.wanted.get() {
            Stage::Burning
        } else if self.level.get() > 0.0 {
            Stage::Exhausting
        } else {
            Stage::Out
        }
    }

    /// Whether there is nothing left to draw or animate.
    fn is_out(&self) -> bool {
        self.stage() == Stage::Out
    }

    /// Flips between burning and burning out. Forgets the last tick, because
    /// the clock may have been stopped for any length of time.
    fn toggle(&self) {
        self.wanted.set(!self.wanted.get());
        self.last.set(None);
    }

    /// Adds `seconds` of real time and returns the effect to draw.
    ///
    /// A step that is not a plain forward amount of time adds nothing.
    fn advance(&self, seconds: f64) -> Fire {
        if seconds.is_finite() && seconds > 0.0 {
            self.time.set(self.time.get() + seconds);
            let level = f64::from(self.level.get());
            let next = if self.wanted.get() {
                level + seconds / IGNITE_SECONDS
            } else {
                level - seconds / EXHAUST_SECONDS
            };
            self.level.set(next.clamp(0.0, 1.0) as f32);
        }
        Fire {
            time: self.time.get() as f32,
            level: self.level.get(),
        }
    }

    /// Advances by the real time since the previous tick.
    fn tick(&self, now: Instant) -> Fire {
        let step = self
            .last
            .replace(Some(now))
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64().min(MAX_STEP));
        self.advance(step)
    }
}

/// One animation frame: the effect to draw now. Stops the ticker once the fire
/// is out, so a cold button asks for no more frames.
fn frame(fuel: &Fuel, ticker: &AnimationController, now: Instant) -> Fire {
    let fire = fuel.tick(now);
    if fuel.is_out() {
        ticker.reset();
    }
    fire
}

/// What a press does: lets a burning fire die, or relights one.
fn press(fuel: &Fuel, ticker: &AnimationController) {
    fuel.toggle();
    if !ticker.is_animating() {
        ticker.forward_from_first_tick();
    }
}

// ── The page ─────────────────────────────────────────────────────────────────

/// The fire-button showcase page.
#[derive(StatefulWidget)]
pub struct FireButtonExample {}

impl FireButtonExample {
    /// Creates the page with the fire burning.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for FireButtonExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FireButtonExampleState {
    /// Kept for the helper methods that call back into the state.
    updater: StateUpdater<Self>,
    /// Shared with the animation closure and the press handler, both of which
    /// outlive any one page rebuild.
    fuel: Rc<Fuel>,
    /// Only drives frames: its value is never read.
    ticker: AnimationController,
}

impl State<FireButtonExample> for FireButtonExampleState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(widget: FireButtonExample) -> Self {
        let ticker = AnimationController::new(Duration::from_secs(1), curves::LINEAR);
        ticker.set_repeat(true);
        ticker.forward_from_first_tick();
        FireButtonExampleState {
            updater: StateUpdater::empty(),
            fuel: Rc::new(Fuel::new()),
            ticker,
        }
    }

    fn build(this: &StateView<Self>, ctx: &aimer::BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);

        let on_tap = {
            let fuel = Rc::clone(&this.fuel);
            let ticker = this.ticker.clone();
            let updater = this.updater();
            move || {
                press(&fuel, &ticker);
                // The fire reads `fuel` itself; this refreshes the caption.
                updater.set_state(|_| {});
            }
        };
        let button = GestureDetector::new().on_tap(on_tap).child(
            Container::new()
                .width(BUTTON_WIDTH)
                .height(BUTTON_HEIGHT)
                .box_decoration(
                    BoxDecoration::new()
                        .background_color(Color::Rgb(34, 20, 16))
                        .border_radius(BUTTON_HEIGHT / 2.0),
                )
                .child(
                    Text::new("Fire")
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(24)
                                .font_weight(FontWeight::Bold)
                                .color(Color::Rgb(255, 190, 110)),
                        ),
                ),
        );
        // The button is the same at every frame, so the animation builds it
        // once and only rebuilds the decoration around it.
        let stage = Container::new().width(BOX_WIDTH).height(BOX_HEIGHT).child(
            Stack::new().children([Positioned::new()
                .left(BUTTON_LEFT)
                .top(BUTTON_TOP)
                .child(button)
                .boxed()]),
        );
        let fire = {
            let fuel = Rc::clone(&this.fuel);
            let ticker = this.ticker.clone();
            AnimatedBuilder::with_child(this.ticker.clone(), stage, move |_, stage| {
                let effect = frame(&fuel, &ticker, Instant::now());
                DecorationBox::new()
                    .decoration(ShaderEffectDecoration::new(effect))
                    .child(stage)
            })
        };

        let caption = match this.fuel.stage() {
            Stage::Burning => "Burning. Press the button to let it burn out.",
            Stage::Exhausting | Stage::Out => "Burning out. Press the button to light it again.",
        };

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                    .children([
                        Text::new("Fire button")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                        Text::new(
                            "The flames are a ShaderEffectDecoration: a .aimer shader the app \
                             registered, drawn behind the button. Each frame the decoration is rebuilt with the next fuel \
                             level; the button is built once. When the fire is out the animation \
                             stops. It needs a backend that takes run-time shaders (Metal, Vulkan, \
                             Direct3D 12, OpenGL or WebGPU); elsewhere no flames appear.",
                        )
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                        Center::new().child(fire).boxed(),
                        Text::new(caption)
                            .text_style(
                                TextStyle::new()
                                    .font_size(15)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests;
