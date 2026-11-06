//! The `Lottie` widget: plays a [`LottieScene`] inside a box of the layout.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd::Rect;
use aimer_utils::AnimInstant;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement,
    Rebuildable, VisitorElement, Widget, request_isolated_animation_frame,
};

use crate::matte::{Clock, matted_element};
use crate::playback::Playback;
use crate::scene::{Block, LottieScene};

/// How the animation's canvas is fitted into the widget's box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LottieFit {
    /// The whole animation is visible, centered, keeping its aspect ratio.
    #[default]
    Contain,
    /// The animation covers the whole box, keeping its aspect ratio; what falls
    /// outside the box is clipped.
    Cover,
    /// The animation is stretched to the box.
    Fill,
}

/// Where the animation's canvas of `viewport` lands in a box of `bounds`, in the
/// box's own coordinates.
///
/// A box with no area, or a canvas with none, fits to nothing.
pub(crate) fn fit_destination(viewport: (f32, f32), bounds: (f32, f32), fit: LottieFit) -> Rect {
    let ((vw, vh), (bw, bh)) = (viewport, bounds);
    let usable = |v: f32| v.is_finite() && v > 0.0;
    if !(usable(vw) && usable(vh) && usable(bw) && usable(bh)) {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    let scale = match fit {
        LottieFit::Fill => return Rect::new(0.0, 0.0, bw, bh),
        LottieFit::Contain => (bw / vw).min(bh / vh),
        LottieFit::Cover => (bw / vw).max(bh / vh),
    };
    let (width, height) = (vw * scale, vh * scale);
    Rect::new((bw - width) / 2.0, (bh - height) / 2.0, width, height)
}

/// The box of a Lottie: a given axis is used as it is, a missing one follows
/// the canvas's aspect ratio, and with neither given the canvas's own size is
/// used. Device pixels, like every layout size.
pub(crate) fn lottie_size(
    scene: &LottieScene,
    width: Dimension,
    height: Dimension,
    ctx: &BuildContext,
) -> ResolvedSize {
    let composition = scene.composition();
    let aspect = composition.width / composition.height;
    let axis = |dimension: Dimension, max: f32| {
        (!matches!(dimension, Dimension::Auto)).then(|| dimension.resolve(max, ctx.scale))
    };
    let (width, height) = match (
        axis(width, ctx.box_constraint.max_width),
        axis(height, ctx.box_constraint.max_height),
    ) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (width, width / aspect),
        (None, Some(height)) => (height * aspect, height),
        (None, None) => (composition.width * ctx.scale, composition.height * ctx.scale),
    };
    ResolvedSize { width, height }
}

/// Whether a box with these dimensions is independent of its parent.
pub(crate) fn lottie_layout_is_stable(width: Dimension, height: Dimension) -> bool {
    !matches!(width, Dimension::Percent(_)) && !matches!(height, Dimension::Percent(_))
}

/// Records one plane of `scene` at `frame` into a box of logical `size` whose
/// top left corner is `origin`. Opens and closes nothing but its own clip.
pub(crate) fn draw_plane(
    canvas: &Canvas,
    scene: &LottieScene,
    plane: usize,
    frame: f32,
    fit: LottieFit,
    origin: Vec2d,
    size: ResolvedSize,
) {
    let composition = scene.composition();
    let fitted = fit_destination((composition.width, composition.height), (size.width, size.height), fit);
    let destination = Rect::new(origin.x + fitted.x, origin.y + fitted.y, fitted.width, fitted.height);
    let frame = scene.plane_frame(plane, frame);
    // `Cover` overflows the box on one axis; nothing may draw outside it.
    let clipped = fit == LottieFit::Cover;
    if clipped {
        canvas.push_clip(Rect::new(origin.x, origin.y, size.width, size.height), [0.0; 4]);
    }
    canvas.draw_svg(frame.scene, destination, frame.overrides);
    if clipped {
        canvas.pop_clip();
    }
}

/// The animation slot of a [`Lottie`] that has not been given one yet.
///
/// It is not a widget; finish the builder with [`Lottie::animation`].
pub struct RequiredAnimation;

/// Plays a Lottie animation inside a box of the layout.
///
/// The animation is a [`LottieScene`], which is built once from a parsed
/// [`Composition`](crate::Composition) and shared: hand the same
/// `Arc<LottieScene>` to every widget that shows it, and rebuilding a widget
/// never parses or tessellates anything again. Each frame only the nodes whose
/// transform, opacity or color changed are updated, through the same SVG
/// pipeline and mesh cache that `aimer_svg` uses.
///
/// By default the animation loops at normal speed and is as big as its own
/// canvas. [`width`](Lottie::width) and [`height`](Lottie::height) size the box;
/// when only one is given the other follows the canvas's aspect ratio, and
/// [`fit`](Lottie::fit) says how the canvas is placed in a box of another shape.
///
/// The playhead lives in the element and survives a rebuild of an ancestor, as
/// long as the new widget holds the same `Arc<LottieScene>`. The widget draws
/// only; it takes no pointer events, so it can sit under or beside interactive
/// widgets.
///
/// Anything in the file outside the supported subset is skipped, not drawn
/// wrongly: see [`Composition::diagnostics`](crate::Composition::diagnostics).
///
/// A layer cut by a track matte is drawn through the renderer's coverage mask,
/// which costs two offscreen targets the size of the box for each matted layer
/// and a full walk of the widget's subtree on every frame of the animation.
/// Animations without mattes stay one element and ask for single-element
/// frames.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use aimer_lottie::{Composition, Lottie, LottieFit, LottieScene};
///
/// let composition = Composition::parse(
///     r#"{"fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[]}"#,
/// )
/// .expect("a valid Lottie document");
/// let animation = Arc::new(LottieScene::new(composition));
///
/// let spinner = Lottie::new()
///     .width(48.0)
///     .fit(LottieFit::Contain)
///     .animation(animation);
/// ```
pub struct Lottie<A = RequiredAnimation> {
    animation: A,
    width: Dimension,
    height: Dimension,
    fit: LottieFit,
    looping: bool,
    speed: f32,
    playing: bool,
    progress: Option<f32>,
}

impl Lottie {
    /// Creates a builder that loops at normal speed and is as big as the
    /// animation's own canvas.
    ///
    /// Finish the builder with [`Lottie::animation`] or [`Lottie::box_animation`].
    #[inline]
    pub fn new() -> Self {
        Self {
            animation: RequiredAnimation,
            width: Dimension::Auto,
            height: Dimension::Auto,
            fit: LottieFit::Contain,
            looping: true,
            speed: 1.0,
            playing: true,
            progress: None,
        }
    }

    /// Attaches the required animation and completes this builder.
    ///
    /// Accepts a [`LottieScene`] or, to share one between widgets, an
    /// `Arc<LottieScene>`.
    #[inline]
    pub fn animation(self, animation: impl Into<Arc<LottieScene>>) -> Lottie<Arc<LottieScene>> {
        Lottie {
            animation: animation.into(),
            width: self.width,
            height: self.height,
            fit: self.fit,
            looping: self.looping,
            speed: self.speed,
            playing: self.playing,
            progress: self.progress,
        }
    }

    /// Attaches `animation` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_animation(self, animation: impl Into<Arc<LottieScene>>) -> AnyWidget {
        self.animation(animation).boxed()
    }
}

impl Default for Lottie {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<A> Lottie<A> {
    /// Sets the preferred width.
    ///
    /// Without a width the box is as wide as the canvas, or follows
    /// [`height`](Self::height) through the canvas's aspect ratio. Pixel values
    /// are logical pixels; percentages resolve against the parent's maximum.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the preferred height; see [`width`](Self::width).
    #[inline]
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = height.into();
        self
    }

    /// Chooses how the canvas is placed when the box has another shape. The
    /// default is [`LottieFit::Contain`].
    #[inline]
    pub fn fit(mut self, fit: LottieFit) -> Self {
        self.fit = fit;
        self
    }

    /// Whether playback starts over at the end. The default is `true`; without
    /// looping the animation stops on its last frame.
    #[inline]
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Playback rate: `1.0` is real time, `2.0` twice as fast, negative plays
    /// backwards. The default is `1.0`.
    #[inline]
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Whether the animation starts playing. The default is `true`; a paused
    /// animation shows its first frame and asks for no frames at all.
    #[inline]
    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    /// Shows one fixed point of the animation, `0.0` for its first frame to `1.0`
    /// for its last, and never plays. Out-of-range values are clamped.
    ///
    /// Rebuilding with a new progress is how a caller scrubs the animation, for
    /// example from a slider or a scroll position.
    #[inline]
    pub fn progress(mut self, progress: f32) -> Self {
        self.progress = Some(progress);
        self
    }
}

impl Widget for Lottie<Arc<LottieScene>> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let composition = self.animation.composition();
        let mut playback =
            Playback::new(composition.in_frame, composition.out_frame, composition.frame_rate)
                .looping(self.looping)
                .speed(self.speed)
                .playing(self.playing && self.progress.is_none());
        if let Some(progress) = self.progress {
            playback.set_progress(progress);
        }
        let fixed = self.progress.is_some();
        if self.animation.blocks().iter().any(|block| matches!(block, Block::Matted { .. })) {
            return matted_element(
                self.animation,
                self.width,
                self.height,
                self.fit,
                fixed,
                Rc::new(Clock::new(playback)),
                ctx,
            );
        }
        RawLottie {
            scene: self.animation,
            width: self.width,
            height: self.height,
            fit: self.fit,
            fixed,
            playback: Cell::new(playback),
            last_tick: Cell::new(None),
            paint_changed: Cell::new(true),
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Lottie"
    }
}

/// What a recording was made for; a different key means it is stale.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaintKey {
    width: u32,
    height: u32,
    scale: u32,
}

impl PaintKey {
    pub(crate) fn new(width: f32, height: f32, scale: f32) -> Self {
        Self { width: width.to_bits(), height: height.to_bits(), scale: scale.to_bits() }
    }
}

pub(crate) struct RawLottie {
    pub(crate) scene: Arc<LottieScene>,
    width: Dimension,
    height: Dimension,
    fit: LottieFit,
    /// A fixed `progress` was requested, so the clock never moves.
    fixed: bool,
    pub(crate) playback: Cell<Playback>,
    pub(crate) last_tick: Cell<Option<AnimInstant>>,
    paint_changed: Cell<bool>,
    recorded: Cell<Option<PaintKey>>,
}

impl RawLottie {
    fn paint_target(&self, ctx: &BuildContext) -> Option<(PaintKey, ResolvedSize)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = self.computed_size(ctx);
        if !size.width.is_finite() || !size.height.is_finite() || size.width < 0.0 || size.height < 0.0 {
            return None;
        }
        let key = PaintKey::new(size.width, size.height, scale);
        // Layout sizes are device-scaled; recorded commands use logical pixels.
        Some((key, ResolvedSize { width: size.width / scale, height: size.height / scale }))
    }
}

impl Drawable for RawLottie {
    #[inline]
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.paint_target(ctx).is_some()
    }

    fn sync_paint_state(&self, ctx: &BuildContext) -> bool {
        self.sync_paint_geometry(ctx);
        let now = AnimInstant::now();
        let dt = self.last_tick.get().map_or(Duration::ZERO, |previous| now.duration_since(previous));
        let mut playback = self.playback.get();
        let tick = playback.advance(dt);
        self.playback.set(playback);
        if tick.changed {
            self.paint_changed.set(true);
        }
        if tick.wants_frame {
            self.last_tick.set(Some(now));
            // A frame of the animation changes this element's own picture only.
            request_isolated_animation_frame(ctx);
        } else {
            // Idle time must not be counted when playback resumes.
            self.last_tick.set(None);
        }
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        let (key, size) = self
            .paint_target(ctx)
            .expect("Lottie support must be checked before painting");
        self.paint_changed.set(false);
        self.recorded.set(Some(key));
        let canvas = Canvas::of(ctx);
        draw_plane(
            &canvas,
            &self.scene,
            0,
            self.playback.get().frame(),
            self.fit,
            Vec2d { x: 0.0, y: 0.0 },
            size,
        );
        canvas.finish();
    }

    #[inline]
    fn paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.paint_changed.get() || self.recorded.get() != self.paint_target(ctx).map(|(key, _)| key)
    }
}

impl EventElement for RawLottie {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl VisitorElement for RawLottie {
    fn debug_name(&self) -> &'static str {
        "Lottie"
    }
}

impl Rebuildable for RawLottie {
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Keeps the playhead across a rebuild of an ancestor, so a parent that
    /// rebuilds every frame does not restart the animation. Only the same
    /// animation continues; a fixed `progress` is the caller's to decide.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old.option_any().and_then(|any| any.downcast_ref::<Self>()) else {
            return;
        };
        if self.fixed || !Arc::ptr_eq(&self.scene, &old.scene) {
            return;
        }
        let mut playback = self.playback.get();
        playback.adopt_position(&old.playback.get());
        self.playback.set(playback);
        self.last_tick.set(old.last_tick.get());
    }
}

impl LayoutElement for RawLottie {
    fn size(&self) -> Option<Size> {
        match (self.width, self.height) {
            (Dimension::Px(_), Dimension::Px(_)) => Some(Size { width: self.width, height: self.height }),
            _ => None,
        }
    }

    fn is_layout_stable(&self) -> bool {
        lottie_layout_is_stable(self.width, self.height)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        lottie_size(&self.scene, self.width, self.height, ctx)
    }
}
