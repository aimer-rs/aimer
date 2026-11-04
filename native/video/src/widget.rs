use std::cell::Cell;

use aimer_attribute::Dimension;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::Canvas;
use aimer_cupid::utilities::Rect;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable,
    VisitorElement, Widget,
};

use crate::VideoPlayer;

/// How a video's picture fits the widget's box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoFit {
    /// Show the whole picture, as large as fits, centred; the rest of the box
    /// stays empty (letterboxing).
    #[default]
    Contain,
    /// Fill the box, keeping the aspect ratio and cropping what overflows.
    Cover,
    /// Stretch the picture to the box, ignoring its aspect ratio.
    Fill,
}

impl VideoFit {
    /// Where a `video` sized picture lands in a `area` sized box, in the
    /// box's coordinates.
    fn place(self, video: (f32, f32), area: (f32, f32)) -> Rect {
        let (video_width, video_height) = video;
        let (width, height) = area;
        if self == Self::Fill || video_width <= 0.0 || video_height <= 0.0 {
            return Rect::new(0.0, 0.0, width, height);
        }
        let scale_x = width / video_width;
        let scale_y = height / video_height;
        let scale = match self {
            Self::Contain => scale_x.min(scale_y),
            Self::Cover => scale_x.max(scale_y),
            Self::Fill => unreachable!("handled above"),
        };
        let (drawn_width, drawn_height) = (video_width * scale, video_height * scale);
        Rect::new(
            (width - drawn_width) / 2.0,
            (height - drawn_height) / 2.0,
            drawn_width,
            drawn_height,
        )
    }
}

/// Shows a [`VideoPlayer`]'s picture.
///
/// The widget draws the decoder's own GPU texture: no frame is copied through
/// the CPU or uploaded, so a playing video costs one textured quad per frame.
/// It repaints only when a new frame arrives, and asks for frames only while
/// the video plays (or is still opening), so a paused video costs nothing.
///
/// Its size follows the incoming constraints like an image box; set
/// [`width`](Video::width) and [`height`](Video::height) to fix it. Until the
/// first frame is decoded, and without a player, it draws nothing.
///
/// Controls live on the player, not the widget: keep the player in your state
/// and hand clones to buttons.
///
/// # Examples
///
/// ```no_run
/// use aimer_video::{Video, VideoFit, VideoPlayer, VideoSource};
///
/// let player = VideoPlayer::from_source(VideoSource::url("https://example.com/clip.m3u8"));
/// player.play();
/// let video = Video::new().player(player).fit(VideoFit::Cover).height(240);
/// ```
pub struct Video {
    player: Option<VideoPlayer>,
    fit: VideoFit,
    width: Dimension,
    height: Dimension,
}

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

impl Video {
    /// Creates a widget with no player; it draws nothing until given one.
    #[inline]
    pub fn new() -> Self {
        Self {
            player: None,
            fit: VideoFit::Contain,
            width: Dimension::Auto,
            height: Dimension::Auto,
        }
    }

    /// Shows `player`'s picture.
    #[inline]
    pub fn player(mut self, player: VideoPlayer) -> Self {
        self.player = Some(player);
        self
    }

    /// Sets how the picture fits the box. The default is
    /// [`VideoFit::Contain`].
    #[inline]
    pub fn fit(mut self, fit: VideoFit) -> Self {
        self.fit = fit;
        self
    }

    /// Fixes the width; the default follows the incoming constraints.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Fixes the height; the default follows the incoming constraints.
    #[inline]
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = height.into();
        self
    }
}

impl Widget for Video {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawVideo {
            player: self.player,
            fit: self.fit,
            width: self.width,
            height: self.height,
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Video"
    }
}

/// What a recording was made for; a different key means it is stale.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintKey {
    width: u32,
    height: u32,
    scale: u32,
    generation: u64,
    video: Option<(u32, u32)>,
}

struct RawVideo {
    player: Option<VideoPlayer>,
    fit: VideoFit,
    width: Dimension,
    height: Dimension,
    recorded: Cell<Option<PaintKey>>,
}

impl RawVideo {
    /// The key and logical extent to record for, or `None` while the scale or
    /// size is not a usable number.
    fn paint_target(&self, ctx: &BuildContext) -> Option<(PaintKey, ResolvedSize)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = self.computed_size(ctx);
        if !size.width.is_finite() || !size.height.is_finite() || size.width < 0.0 || size.height < 0.0 {
            return None;
        }
        // From the player's own state: checking for a repaint must not call
        // the platform every frame.
        let (generation, video) = match &self.player {
            Some(player) => player.paint_state(),
            None => (0, None),
        };
        let key = PaintKey {
            width: size.width.to_bits(),
            height: size.height.to_bits(),
            scale: scale.to_bits(),
            generation,
            video,
        };
        // Layout sizes are device-scaled; recorded commands use logical pixels.
        let logical = ResolvedSize {
            width: size.width / scale,
            height: size.height / scale,
        };
        Some((key, logical))
    }
}

impl Drawable for RawVideo {
    fn update(&self, _ctx: &BuildContext) {
        let Some(player) = &self.player else {
            return;
        };
        // Frames are needed while the video plays, opens, or fetches the
        // picture a seek produced. A paused or finished video asks for none,
        // and this check does not call the platform.
        if player.wants_frames() {
            aimer_events::window::request_animation_frame();
        }
    }

    fn sync_paint_state(&self, _ctx: &BuildContext) -> bool {
        // Runs before the paint is replayed or recorded, so the frame taken
        // here is the one this frame shows: a new frame changes the
        // generation, which makes `paint_needs_recording` record it now
        // rather than a frame late.
        if let Some(player) = &self.player {
            player.poll();
        }
        true
    }

    #[inline]
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.paint_target(ctx).is_some()
    }

    fn paint(&self, ctx: &BuildContext) {
        let Some((key, size)) = self.paint_target(ctx) else {
            return;
        };
        let canvas = Canvas::of(ctx);
        if let Some(player) = &self.player
            && player.has_frame()
        {
            let video = key.video.map_or((size.width, size.height), |(width, height)| {
                (width as f32, height as f32)
            });
            let rect = self.fit.place(video, (size.width, size.height));
            canvas.draw_external_texture(rect, player.texture());
        }
        canvas.finish();
        self.recorded.set(Some(key));
    }

    #[inline]
    fn paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.recorded.get() != self.paint_target(ctx).map(|(key, _)| key)
    }

    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd::Rect> {
        // Only a cropped picture can reach outside the box.
        if self.fit != VideoFit::Cover {
            return None;
        }
        let (_, size) = self.paint_target(ctx)?;
        Some(aimer_cupid::draw_cmd::Rect::new(0.0, 0.0, size.width, size.height))
    }
}

impl EventElement for RawVideo {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl VisitorElement for RawVideo {
    fn debug_name(&self) -> &'static str {
        "Video"
    }
}

impl Rebuildable for RawVideo {}

impl LayoutElement for RawVideo {
    fn size(&self) -> Option<Size> {
        match (self.width, self.height) {
            (Dimension::Px(_), Dimension::Px(_)) => Some(Size {
                width: self.width,
                height: self.height,
            }),
            _ => None,
        }
    }

    fn is_layout_stable(&self) -> bool {
        matches!((self.width, self.height), (Dimension::Px(_), Dimension::Px(_)))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let axis = |dimension: Dimension, max: f32, parent: f32| match dimension {
            Dimension::Auto => parent,
            dimension => dimension.resolve(max, ctx.scale),
        };
        ResolvedSize {
            width: axis(self.width, ctx.box_constraint.max_width, ctx.parent_size.width),
            height: axis(self.height, ctx.box_constraint.max_height, ctx.parent_size.height),
        }
    }
}

// These build a `BuildContext` with a tokio runtime handle, which wasm lacks.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "widget_tests.rs"]
mod tests;
