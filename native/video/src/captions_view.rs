use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use aimer_container::Container;
use aimer_flex::{BoxAlignment, Column};
use aimer_style::{BoxDecoration, LayoutSpacing, TextAlign, TextStyle};
use aimer_text::Text;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{AnyElement, AnyWidget, Element, State, StateUpdater, StatefulElement, StatefulWidget, Widget};

use crate::frame_watch::FrameWatch;
use crate::{CaptionTrack, VideoPlayer};

/// How captions look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionStyle {
    /// Logical pixels, whole numbers (the text style takes whole sizes).
    font_size: f32,
    color: Color,
    background: Color,
}

impl Default for CaptionStyle {
    /// White text on a translucent black box, as most players draw them.
    fn default() -> Self {
        Self {
            font_size: 18.0,
            color: Color::Rgb(255, 255, 255),
            background: Color::Rgb(0, 0, 0).with_opacity(190),
        }
    }
}

impl CaptionStyle {
    /// The default style.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the text size in logical pixels.
    #[inline]
    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// Sets the text colour.
    #[inline]
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Sets the colour of the box behind the text.
    #[inline]
    pub fn background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }
}

/// Shows the caption cue a [`VideoPlayer`] is at, from a [`CaptionTrack`].
///
/// It follows the playhead while the video plays and rebuilds only when the
/// cue changes, not every frame; a paused video costs it nothing. Between
/// cues it draws nothing. [`VideoPlayerView`](crate::VideoPlayerView) places
/// it over the picture; use it directly in a layout of your own.
///
/// # Examples
///
/// ```no_run
/// use aimer_video::{CaptionStyle, CaptionTrack, Captions, VideoPlayer, VideoSource};
///
/// let player = VideoPlayer::from_source(VideoSource::bundled("assets/talk.mp4"));
/// let track = CaptionTrack::parse("WEBVTT\n\n00:01.000 --> 00:04.000\nHello\n").unwrap();
/// let captions = Captions::new()
///     .player(player)
///     .track(track)
///     .style(CaptionStyle::new().font_size(22.0));
/// ```
pub struct Captions {
    player: Option<VideoPlayer>,
    track: Option<Arc<CaptionTrack>>,
    style: CaptionStyle,
}

impl Default for Captions {
    fn default() -> Self {
        Self::new()
    }
}

impl Captions {
    /// Creates a view with no player or track; it shows nothing until given
    /// both.
    #[inline]
    pub fn new() -> Self {
        Self {
            player: None,
            track: None,
            style: CaptionStyle::default(),
        }
    }

    /// Follows `player`'s playhead.
    #[inline]
    pub fn player(mut self, player: VideoPlayer) -> Self {
        self.player = Some(player);
        self
    }

    /// Shows cues from `track`.
    #[inline]
    pub fn track(self, track: CaptionTrack) -> Self {
        self.shared_track(Arc::new(track))
    }

    /// Shows cues from a track shared with other views (a fullscreen copy of
    /// the same player, say), without copying it.
    #[inline]
    pub fn shared_track(mut self, track: Arc<CaptionTrack>) -> Self {
        self.track = Some(track);
        self
    }

    /// Sets how the captions look.
    #[inline]
    pub fn style(mut self, style: CaptionStyle) -> Self {
        self.style = style;
        self
    }
}

/// The cues showing, as `CaptionTrack::showing_at` identifies them; `None`
/// until the first check.
type Showing = Rc<Cell<Option<(usize, usize)>>>;

pub struct CaptionsState {
    config: Captions,
    text: Option<String>,
    showing: Showing,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for Captions {
    type State = CaptionsState;

    fn create_state(self) -> CaptionsState {
        CaptionsState {
            config: self,
            text: None,
            showing: Rc::new(Cell::new(None)),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<Captions> for CaptionsState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        let same_track = match (&self.config.track, &new.config.track) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.config = new.config;
        if !same_track {
            // A different track: find its cue afresh.
            self.showing.set(None);
            self.text = None;
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let watch = {
            let player = self.config.player.clone();
            let track = self.config.track.clone();
            let showing = self.showing.clone();
            let updater = self.updater;
            FrameWatch::new(move || {
                let (Some(player), Some(track)) = (player.as_ref(), track.as_ref()) else {
                    return false;
                };
                let active = player.wants_frames();
                // A resting player's cue cannot change: skip even reading the
                // playhead, once the first check has been made.
                if !active && showing.get().is_some() {
                    return false;
                }
                let at = player.position();
                let now = track.showing_at(at);
                if showing.get() != Some(now) {
                    showing.set(Some(now));
                    let text = track.text_at(at);
                    updater.set_state(move |state| state.text = text);
                }
                active
            })
        };
        let mut children: Vec<AnyWidget> = vec![watch.boxed()];
        if let Some(text) = &self.text {
            children.push(caption_box(text, &self.config.style));
        }
        Column::new()
            .horizontal_alignment(BoxAlignment::Center)
            .children(children)
    }
}

fn caption_box(text: &str, style: &CaptionStyle) -> AnyWidget {
    Container::new()
        .padding(LayoutSpacing::new().left(10).right(10).top(4).bottom(4))
        .box_decoration(BoxDecoration::new().background_color(style.background).border_radius(4))
        .child(
            Text::new(text.to_string())
                .text_align(TextAlign::MidCenter)
                .text_style(TextStyle::new().font_size(style.font_size.round().max(1.0) as u32).color(style.color)),
        )
        .boxed()
}

impl Widget for Captions {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Captions", None).0.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Captions"
    }
}
