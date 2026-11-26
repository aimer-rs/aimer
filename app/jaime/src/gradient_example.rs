//! Linear, radial and conic gradients, plus decorations painted over and through a child.

use aimer::decoration::{
    BackdropFilter, BorderRadius, Decoration, DecorationBox, ForegroundDecorationBox, Gradient,
    LightEdge, MaskedDecorationBox,
};
use std::rc::Rc;

use aimer::range::{RangeChangeCallback, Slider};
use aimer::style::{BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing, TextStyle};
use aimer::{
    AimerApp, AnyWidget, BuildContext, BuildContextRetain, Column, Container, Row, ScrollAxis,
    Scrollable, SizedBox, State, StateView, StateUpdater, StatefulWidget, Text, Widget,
};

use crate::theme;

const CARD_WIDTH: u32 = 220;
const CARD_HEIGHT: u32 = 120;

/// Starts the standalone gradient example.
pub fn start_gradient_example() {
    AimerApp::start(theme::provide(gradient_example()));
}

/// Builds adjustable gradients and the behind/over/through decoration comparison.
///
/// The gallery is taller than a small window, so it scrolls vertically.
pub fn gradient_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(GradientExample::new())
}

/// Demonstrates GPU-evaluated gradients and where a decoration can paint.
#[derive(StatefulWidget)]
pub struct GradientExample {}

impl GradientExample {
    /// Creates a gradient gallery with a 135° sunset, a spotlight and a color wheel.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for GradientExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct GradientExampleState {
    angle: f64,
    radius: f64,
    start: f64,
    corners: f64,
    /// The sliders' callbacks, made once when the state mounts. A closure written
    /// in `build` is a new callback on every build, and a slider holding it could
    /// never be recognised as unchanged; these keep their identity.
    on_change: Option<SliderCallbacks>,
}

struct SliderCallbacks {
    angle: RangeChangeCallback,
    radius: RangeChangeCallback,
    start: RangeChangeCallback,
    corners: RangeChangeCallback,
}

impl State<GradientExample> for GradientExampleState {
    fn create(_widget: GradientExample) -> Self {
        GradientExampleState {
            angle: 135.0,
            radius: 1.0,
            start: 0.0,
            corners: 20.0,
            on_change: None,
        }
    }

    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.on_change = Some(SliderCallbacks {
            angle: Rc::new(move |value| updater.set_state(move |state| state.angle = value)),
            radius: Rc::new(move |value| updater.set_state(move |state| state.radius = value)),
            start: Rc::new(move |value| updater.set_state(move |state| state.start = value)),
            corners: Rc::new(move |value| updater.set_state(move |state| state.corners = value)),
        });
    }

    fn build(this: StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let callbacks = this
            .on_change
            .as_ref()
            .expect("the callbacks are made when the state mounts, before the first build");
        // Everything is described again on every slider tick, which is cheap;
        // `retain_hashed` is what keeps the unchanged parts from being turned into
        // elements and walked again. A control or card follows its own value, so
        // it is rebuilt — with its state carried across, which keeps a drag alive
        // — only when its hash changes.
        Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
            ctx.retain_hashed("gpu-note", note("Gradients are evaluated per pixel on the GPU, so they stay smooth at any size and scale.")).boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                ctx.retain_hashed("angle-control", control(format!("Linear angle: {:.0}°", this.angle), this.angle, 0.0..360.0, 5.0,
                    Rc::clone(&callbacks.angle))).boxed(),
                ctx.retain_hashed("radius-control", control(format!("Radial radius: {:.2}", this.radius), this.radius, 0.1..1.5, 0.05,
                    Rc::clone(&callbacks.radius))).boxed(),
            ]).boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                ctx.retain_hashed("start-control", control(format!("Conic start: {:.0}°", this.start), this.start, 0.0..360.0, 5.0,
                    Rc::clone(&callbacks.start))).boxed(),
                ctx.retain_hashed("corners-control", control(format!("Corner radius: {:.0} px", this.corners), this.corners, 0.0..60.0, 1.0,
                    Rc::clone(&callbacks.corners))).boxed(),
            ]).boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
                this.card(ctx, "sunset-card", Gradient::linear()
                    .angle(this.angle as f32)
                    .stop(0.0, Color::Rgb(255, 94, 98))
                    .stop(1.0, Color::Rgb(255, 195, 113))),
                this.card(ctx, "spotlight-card", Gradient::radial()
                    .radius(this.radius as f32)
                    .colors([Color::Rgba(255, 255, 255, 230), Color::Rgb(60, 120, 255)])),
                this.card(ctx, "wheel-card", Gradient::conic()
                    .from_angle(this.start as f32)
                    .colors([Color::RED, Color::Rgb(255, 220, 0), Color::Rgb(0, 200, 90),
                        Color::Rgb(0, 140, 255), Color::RED])),
            ]).boxed(),
            ctx.retain_hashed("edge-note", note("Behind or over: the same light edge on an opaque card. Behind it, the card covers the rim; over it, the rim stays visible.")).boxed(),
            ctx.retain_hashed("edge-cards", Container::new().padding(LayoutSpacing::all(Spacing::Px(24)))
                .box_decoration(BoxDecoration::new().background_color(Color::Rgb(9, 16, 30)).border_radius(16))
                .child(Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                    DecorationBox::new()
                        .decoration(vec![
                            BorderRadius::from(16).notype(),
                            LightEdge::new().width(10.0).notype(),
                        ])
                        .child(artwork())
                        .boxed(),
                    ForegroundDecorationBox::new()
                        .decoration(vec![
                            BorderRadius::from(16).notype(),
                            LightEdge::new().width(10.0).notype(),
                        ])
                        .child(artwork())
                        .boxed(),
                ]))).boxed(),
            ctx.retain_hashed("through-note", note("Through: a decoration shown only where the child has coverage, so a rainbow becomes rainbow text.")).boxed(),
            ctx.retain_hashed("rainbow-text", MaskedDecorationBox::new()
                .decoration(Gradient::rainbow())
                .child(caption("Rainbow text", 44))).boxed(),
            ctx.retain_hashed("glass-note", note("Glass text: a backdrop filter masked by text blurs and brightens what is behind only inside the glyphs.")).boxed(),
            ctx.retain_hashed("glass-text", DecorationBox::new()
                .decoration(vec![
                    BorderRadius::from(16).notype(),
                    Gradient::conic()
                        .colors([Color::RED, Color::Rgb(255, 220, 0), Color::Rgb(0, 200, 90),
                            Color::Rgb(0, 140, 255), Color::RED])
                        .notype(),
                ])
                .child(Container::new().padding(LayoutSpacing::all(Spacing::Px(24)))
                    .child(MaskedDecorationBox::new()
                        .decoration(BackdropFilter::new().blur_radius(10.0).brightness(1.8))
                        .child(caption("Glass text", 56))))).boxed(),
        ])
    }
}

impl GradientExampleState {
    /// A gradient behind an empty card, rounded by the shared corner radius.
    fn card(&self, ctx: &BuildContext, key: &'static str, gradient: Gradient) -> AnyWidget {
        ctx.retain_hashed(
            key,
            DecorationBox::new()
                .decoration(vec![
                    BorderRadius::from(self.corners as f32).notype(),
                    gradient.notype(),
                ])
                .child(SizedBox::new().width(CARD_WIDTH).height(CARD_HEIGHT)),
        )
        .boxed()
    }
}

/// The opaque card the decoration comparison paints behind and over.
fn artwork() -> Container<SizedBox> {
    Container::new()
        .box_decoration(
            BoxDecoration::new()
                .background_color(Color::Rgb(24, 42, 76))
                .border_radius(16),
        )
        .child(SizedBox::new().width(CARD_WIDTH).height(CARD_HEIGHT))
}

fn caption(text: &'static str, size: u32) -> Text {
    Text::new(text).text_style(
        TextStyle::new()
            .font_size(size)
            .font_weight(FontWeight::Bold)
            .color(Color::WHITE),
    )
}

fn note(text: &'static str) -> AnyWidget {
    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(14)
                .color(theme::app_theme().on_background_color),
        )
        .wrapped()
        .boxed()
}

fn control(
    label: String,
    value: f64,
    range: std::ops::Range<f64>,
    step: f64,
    on_change: RangeChangeCallback,
) -> impl Widget {
    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(8)))
        .children([
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(theme::app_theme().on_background_color),
                )
                .boxed(),
            Slider::new()
                .range(range)
                .step(step)
                .value(value)
                .width(280.0)
                .on_change_shared(on_change)
                .boxed(),
        ])
}

#[cfg(test)]
mod rebuild_profile {
    //! What one slider tick costs the gradient page: `set_state` plus the rebuild
    //! it schedules, without layout or paint.
    //!
    //! A measurement, not a check. Run unoptimised, one test at a time:
    //!
    //! ```text
    //! cargo nextest run -p jaime -j 1 -E 'binary_id(jaime)' gradient_rebuild_profile \
    //!     --run-ignored only --no-capture
    //! ```

    use std::sync::OnceLock;

    use aimer::{BuildContext, Rebuildable, StatefulElement};
    use aimer_time::Instant;

    use super::{GradientExample, GradientExampleState};

    const WARMUP: usize = 50;
    const ROUNDS: usize = 600;

    fn context() -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer::canvas::InnerCanvas::new()));
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("profile runtime should build")
        });
        let _guard = runtime.enter();
        let mut context = aimer::BuildContext::new(
            aimer::canvas::FrameCanvas::new(inner),
            aimer::ResolvedSize { width: 1280.0, height: 800.0 },
            1.0,
            aimer::Vec2d::default(),
            aimer::Vec2d::default(),
            aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
            runtime.handle().clone(),
        );
        context.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 1280.0,
            max_height: f32::MAX,
        };
        context
    }

    #[test]
    #[ignore = "profile harness"]
    fn gradient_rebuild_profile() {
        let ctx = context();
        let (element, updater) = StatefulElement::new_with_name::<GradientExample, GradientExampleState>(
            GradientExample::new(),
            &ctx,
            "GradientExample",
            None,
        );
        let mut samples = Vec::with_capacity(ROUNDS);
        for round in 0..WARMUP + ROUNDS {
            // A different angle every time, so the state really changes.
            let angle = f64::from((round % 72) as u32) * 5.0;
            let started = Instant::now();
            updater.set_state(move |state| state.angle = angle);
            element.rebuild_if_dirty(&ctx);
            if round >= WARMUP {
                samples.push(started.elapsed());
            }
        }
        samples.sort();
        let at = |fraction: f64| samples[((samples.len() - 1) as f64 * fraction) as usize];
        eprintln!(
            "gradient page set_state + rebuild: p50 {:?}, p95 {:?}, min {:?}",
            at(0.5),
            at(0.95),
            samples[0]
        );
    }
}
