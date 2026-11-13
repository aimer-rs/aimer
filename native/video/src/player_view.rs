use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use aimer_attribute::Dimension;
use aimer_container::Container;
use aimer_modal::{Modal, ModalHandle};
use aimer_space::{Align, Alignment, Stack};
use aimer_style::{BoxDecoration, LayoutSpacing};
use aimer_widget::base::{BuildContext, Color, WindowHandle};
use aimer_widget::{AnyElement, State, StateView, StateUpdater, StatefulElement, Widget};

use crate::controls::BAR_HEIGHT;
use crate::{CaptionStyle, CaptionTrack, Captions, Chapters, Video, VideoControls, VideoFit, VideoPlayer};

/// How far above the bottom captions sit: clear of the control bar, with a
/// small gap.
const CAPTION_LIFT: i32 = BAR_HEIGHT as i32 + 8;

/// A complete video player: the picture, its captions and the standard
/// controls.
///
/// This is [`Video`], [`Captions`] and [`VideoControls`] stacked the usual
/// way, plus fullscreen: the fullscreen button puts the window into
/// fullscreen and shows the same player over the whole window, then returns
/// it to its place. The video keeps playing throughout; nothing is reopened.
///
/// Captions come from a sidecar [`CaptionTrack`] (WebVTT or SRT) and show
/// while the captions button is on. [`Chapters`] cut the seek bar into
/// segments and name the current one.
///
/// # Examples
///
/// ```no_run
/// use aimer_video::{CaptionTrack, Chapters, VideoPlayer, VideoPlayerView, VideoSource};
///
/// let player = VideoPlayer::from_source(VideoSource::bundled("assets/talk.mp4"));
/// let captions = CaptionTrack::parse(include_str!("../../../app/jaime/assets/test_pattern.vtt")).unwrap();
/// let view = VideoPlayerView::new()
///     .player(player)
///     .captions(captions)
///     .height(360);
/// ```
#[derive(Clone)]
pub struct VideoPlayerView {
    player: Option<VideoPlayer>,
    fit: VideoFit,
    captions: Option<Arc<CaptionTrack>>,
    caption_style: CaptionStyle,
    chapters: Arc<Chapters>,
    accent: Color,
    fullscreen_button: bool,
    width: Dimension,
    height: Dimension,
    /// Set on the copy shown in fullscreen, shared with the original.
    session: Option<Rc<Session>>,
}

impl Default for VideoPlayerView {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoPlayerView {
    /// Creates a view with no player; it draws a black box until given one.
    #[inline]
    pub fn new() -> Self {
        Self {
            player: None,
            fit: VideoFit::Contain,
            captions: None,
            caption_style: CaptionStyle::default(),
            chapters: Arc::new(Chapters::default()),
            accent: Color::Rgb(255, 0, 51),
            fullscreen_button: true,
            width: Dimension::Auto,
            height: Dimension::Auto,
            session: None,
        }
    }

    /// Plays `player`.
    #[inline]
    pub fn player(mut self, player: VideoPlayer) -> Self {
        self.player = Some(player);
        self
    }

    /// Sets how the picture fits; the default is [`VideoFit::Contain`].
    #[inline]
    pub fn fit(mut self, fit: VideoFit) -> Self {
        self.fit = fit;
        self
    }

    /// Shows `captions`, with a button to turn them off.
    #[inline]
    pub fn captions(mut self, captions: CaptionTrack) -> Self {
        self.captions = Some(Arc::new(captions));
        self
    }

    /// Sets how the captions look.
    #[inline]
    pub fn caption_style(mut self, style: CaptionStyle) -> Self {
        self.caption_style = style;
        self
    }

    /// Cuts the seek bar into `chapters` and names the current one.
    #[inline]
    pub fn chapters(mut self, chapters: Chapters) -> Self {
        self.chapters = Arc::new(chapters);
        self
    }

    /// Sets the colour of the seek bar's played part and thumb.
    #[inline]
    pub fn accent(mut self, accent: Color) -> Self {
        self.accent = accent;
        self
    }

    /// Shows or hides the fullscreen button; shown by default.
    #[inline]
    pub fn fullscreen_button(mut self, shown: bool) -> Self {
        self.fullscreen_button = shown;
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

/// What the inline view and its fullscreen copy share.
struct Session {
    captions_on: Cell<bool>,
    /// The fullscreen presentation, while there is one.
    modal: RefCell<Option<ModalHandle>>,
    window: RefCell<Option<WindowHandle>>,
}

impl Session {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            captions_on: Cell::new(true),
            modal: RefCell::new(None),
            window: RefCell::new(None),
        })
    }

    fn enter(self: &Rc<Self>, view: &VideoPlayerView, window: &WindowHandle) {
        if self.modal.borrow().is_some() {
            return;
        }
        window.set_fullscreen(true);
        *self.window.borrow_mut() = Some(window.clone());
        let mut copy = view.clone();
        copy.session = Some(self.clone());
        copy.width = Dimension::Auto;
        copy.height = Dimension::Auto;
        let modal = Modal::new()
            .barrier_color(Color::Rgb(0, 0, 0))
            .barrier_dismissible(false)
            // Leaving goes through `exit`, which also restores the window.
            .escape_dismissible(false)
            .child(copy)
            .show();
        *self.modal.borrow_mut() = Some(modal);
    }

    fn exit(&self) {
        if let Some(window) = self.window.borrow_mut().take() {
            window.set_fullscreen(false);
        }
        if let Some(modal) = self.modal.borrow_mut().take() {
            modal.dismiss();
        }
    }
}

pub struct VideoPlayerViewState {
    config: VideoPlayerView,
    session: Rc<Session>,
}

impl State<VideoPlayerView> for VideoPlayerViewState {
    fn create(widget: VideoPlayerView) -> Self {
        let session = widget.session.clone().unwrap_or_else(Session::new);
        VideoPlayerViewState {
            config: widget,
            session,
        }
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.config = new.config;
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let config = &this.config;
        let mut layers = Stack::new();
        if let Some(player) = config.player.clone() {
            layers = layers.add_child(Video::new().player(player.clone()).fit(config.fit));

            let captions_on = this.session.captions_on.get();
            if let Some(track) = config.captions.clone()
                && captions_on
            {
                // A layer as large as the picture, so the box is centred in it;
                // a `Positioned` would hug the box and pin it to one edge.
                layers = layers.add_child(
                    Container::new()
                        .width(Dimension::Percent(100.0))
                        .height(Dimension::Percent(100.0))
                        .padding(LayoutSpacing::new().bottom(CAPTION_LIFT))
                        .child(
                            Align::new().alignment(Alignment::BotCenter).child(
                                Captions::new()
                                    .player(player.clone())
                                    .shared_track(track)
                                    .style(config.caption_style),
                            ),
                        ),
                );
            }

            let mut controls = VideoControls::new()
                .player(player)
                .shared_chapters(config.chapters.clone())
                .accent(config.accent);
            if config.captions.is_some() {
                let session = this.session.clone();
                let updater = this.updater();
                controls = controls.captions_button(captions_on, move |on| {
                    session.captions_on.set(on);
                    updater.set_state(|_| {});
                });
            }
            if config.fullscreen_button {
                let in_fullscreen = config.session.is_some();
                let session = this.session.clone();
                let view = config.clone();
                let window = ctx.window().clone();
                controls = controls.fullscreen_button(in_fullscreen, move |enter| {
                    if enter {
                        session.enter(&view, &window);
                    } else {
                        session.exit();
                    }
                });
            }
            layers = layers.add_child(controls);
        }
        Container::new()
            .width(config.width)
            .height(config.height)
            .box_decoration(BoxDecoration::new().background_color(Color::Rgb(0, 0, 0)))
            .child(layers)
    }
}

impl Widget for VideoPlayerView {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, VideoPlayerViewState>(self, ctx, "VideoPlayerView", None).0.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "VideoPlayerView"
    }
}
