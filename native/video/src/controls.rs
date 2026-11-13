use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use aimer_attribute::Dimension;
use aimer_container::{Container, SizedBox};
use aimer_events::pointer::PointerSource;
use aimer_flex::{BoxAlignment, Column, Expanded, Row};
use aimer_input::button::Button;
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::mouse_region::MouseRegion;
use aimer_range::{Slider, SliderThumb, SliderTrail};
use aimer_space::{Align, Alignment, Positioned, Stack};
use aimer_style::{BoxDecoration, LayoutSpacing, TextStyle};
use aimer_svg::{StaticSvg, Svg, SvgStyle};
use aimer_text::Text;
use aimer_time::Instant;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{AnyElement, AnyWidget, State, StateView, StateUpdater, StatefulElement, Widget};

use crate::controls_state::ControlsVisibility;
use crate::frame_watch::FrameWatch;
use crate::rail::Rail;
use crate::track_geometry::fraction;
use crate::{Chapters, VideoPlayer};

/// The seek bar's resolution, in seconds.
const SEEK_STEP: f64 = 0.05;
/// The seek thumb's diameter, in logical pixels.
const THUMB: f32 = 12.0;
/// The rail's thickness.
const RAIL: f32 = 4.0;
/// The width of the volume slider.
const VOLUME_WIDTH: f32 = 72.0;
/// The side of a bar button.
const BUTTON: f32 = 36.0;
/// The bar's padding: horizontal, then vertical.
const BAR_PADDING: (i32, i32) = (12, 8);
/// The gap between the seek bar and the buttons.
const BAR_GAP: f32 = 4.0;
/// The seek bar's height: the thumb plus room for it to grow under a press.
const SEEK_HEIGHT: f32 = THUMB + 6.0;
/// The bar's full height. It is fixed because a placed child that declares no
/// size of its own fills its parent, which would spread the bar (and its
/// background) over the whole picture.
pub(crate) const BAR_HEIGHT: f32 = BAR_PADDING.1 as f32 * 2.0 + SEEK_HEIGHT + BAR_GAP + BUTTON;

/// A toggle the controls show a button for: its state and what to call.
#[derive(Clone)]
struct Toggle {
    on: bool,
    on_toggle: Rc<dyn Fn(bool)>,
}

/// The standard controls for a [`VideoPlayer`], drawn over its picture.
///
/// - A seek bar along the bottom showing what has played and downloaded, cut
///   into chapters when [`chapters`](VideoControls::chapters) are given. It
///   follows a drag and seeks once, where the drag ends.
/// - Play/pause, mute and a volume slider, the time, and the current
///   chapter's title.
/// - Optional captions and fullscreen buttons, reporting to callbacks.
/// - On a touch screen, a large play/pause button in the middle.
///
/// While the video plays, the controls hide three seconds after the last
/// pointer activity and come back on the next. With a mouse, clicking the
/// picture plays or pauses; on a touch screen a tap shows or hides the
/// controls. Paused, they stay up.
///
/// [`VideoPlayerView`](crate::VideoPlayerView) combines these controls with
/// the picture and captions; use `VideoControls` to arrange your own.
pub struct VideoControls {
    player: Option<VideoPlayer>,
    chapters: Arc<Chapters>,
    captions: Option<Toggle>,
    fullscreen: Option<Toggle>,
    accent: Color,
}

impl Default for VideoControls {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoControls {
    /// Creates controls with no player; they draw nothing until given one.
    #[inline]
    pub fn new() -> Self {
        Self {
            player: None,
            chapters: Arc::new(Chapters::default()),
            captions: None,
            fullscreen: None,
            accent: Color::Rgb(255, 0, 51),
        }
    }

    /// Controls `player`.
    #[inline]
    pub fn player(mut self, player: VideoPlayer) -> Self {
        self.player = Some(player);
        self
    }

    /// Cuts the seek bar into `chapters` and shows the current one's title.
    #[inline]
    pub fn chapters(self, chapters: Chapters) -> Self {
        self.shared_chapters(Arc::new(chapters))
    }

    /// Like [`chapters`](Self::chapters), sharing them without a copy.
    #[inline]
    pub fn shared_chapters(mut self, chapters: Arc<Chapters>) -> Self {
        self.chapters = chapters;
        self
    }

    /// Shows a captions button in state `on`; a press calls `on_toggle` with
    /// the new state.
    #[inline]
    pub fn captions_button(mut self, on: bool, on_toggle: impl Fn(bool) + 'static) -> Self {
        self.captions = Some(Toggle {
            on,
            on_toggle: Rc::new(on_toggle),
        });
        self
    }

    /// Shows a fullscreen button in state `on`; a press calls `on_toggle`
    /// with the new state.
    #[inline]
    pub fn fullscreen_button(mut self, on: bool, on_toggle: impl Fn(bool) + 'static) -> Self {
        self.fullscreen = Some(Toggle {
            on,
            on_toggle: Rc::new(on_toggle),
        });
        self
    }

    /// Sets the colour of the played part of the seek bar and its thumb.
    #[inline]
    pub fn accent(mut self, accent: Color) -> Self {
        self.accent = accent;
        self
    }
}

pub struct VideoControlsState {
    /// Kept for the helper methods that call back into the state.
    updater: StateUpdater<Self>,
    config: VideoControls,
    visibility: Rc<Cell<ControlsVisibility>>,
    /// Whether the last build showed the controls.
    shown: Rc<Cell<bool>>,
    /// Where the seek bar is being dragged to, in seconds.
    scrub: Rc<Cell<Option<f64>>>,
}

impl State<VideoControls> for VideoControlsState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(widget: VideoControls) -> Self {
        VideoControlsState {
            updater: StateUpdater::empty(),
            config: widget,
            visibility: Rc::new(Cell::new(ControlsVisibility::default())),
            shown: Rc::new(Cell::new(false)),
            scrub: Rc::new(Cell::new(None)),
        }
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.config = new.config;
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let mut layers = Stack::new();
        let Some(player) = this.config.player.clone() else {
            return layers;
        };
        let updater = this.updater();
        // Whether the player is active, from its own state: no platform call,
        // so a paused video costs nothing even while this rebuilds.
        let playing = player.wants_frames();
        let now = Instant::now();
        let visibility = this.visibility.get();
        let visible = visibility.visible(now, playing) || this.scrub.get().is_some();
        this.shown.set(visible);

        // The picture: a tap toggles the controls on a touch screen, and
        // plays or pauses with a mouse.
        let tap_area = {
            let player = player.clone();
            let visibility = this.visibility.clone();
            GestureDetector::new()
                .on_tap(move || {
                    let mut state = visibility.get();
                    if state.is_touch() {
                        state.tap(Instant::now(), player.wants_frames());
                    } else {
                        player.toggle();
                        state.note_activity(Instant::now(), false);
                    }
                    visibility.set(state);
                    updater.set_state(|_| {});
                })
                .child(
                    SizedBox::new()
                        .width(Dimension::Percent(100.0))
                        .height(Dimension::Percent(100.0)),
                )
        };
        layers = layers.add_child(tap_area);

        // Each frame: rebuild while the controls show a moving playhead, and
        // once more when they should hide.
        let watch = {
            let player = player.clone();
            let visibility = this.visibility.clone();
            let shown = this.shown.clone();
            let scrub = this.scrub.clone();
            FrameWatch::new(move || {
                let playing = player.wants_frames();
                let now = Instant::now();
                let state = visibility.get();
                let visible = state.visible(now, playing) || scrub.get().is_some();
                if visible != shown.get() || (visible && playing) {
                    updater.set_state(|_| {});
                }
                (visible && playing) || state.hides_at(playing).is_some_and(|at| at > now)
            })
        };
        layers = layers.add_child(watch);

        if visible {
            let width = (ctx.parent_size.width / ctx.scale.max(f32::EPSILON)).max(0.0);
            layers = layers.add_child(
                Positioned::new()
                    .left(0)
                    .right(0)
                    .bottom(0)
                    .child(this.bottom_bar(&player, width)),
            );
            if visibility.is_touch() {
                layers = layers.add_child(
                    Align::new()
                        .alignment(Alignment::MidCenter)
                        .child(this.center_button(&player)),
                );
            }
        }

        let visibility = this.visibility.clone();
        let shown = this.shown.clone();
        Stack::new().add_child(MouseRegion::new()
            .on_pointer_activity(move |source| {
                let mut state = visibility.get();
                state.note_activity(Instant::now(), source == PointerSource::Touch);
                visibility.set(state);
                // Only hidden controls need a rebuild to appear.
                if !shown.get() {
                    updater.set_state(|_| {});
                }
            })
            .child(layers))
    }
}

impl VideoControlsState {
    fn bottom_bar(&self, player: &VideoPlayer, width: f32) -> AnyWidget {
        let duration = player.duration();
        let position = self
            .scrub
            .get()
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .unwrap_or_else(|| player.position());

        let mut row: Vec<AnyWidget> = vec![
            self.play_button(player),
            self.mute_button(player),
            self.volume_slider(player),
            gap(10.0),
            label(format!(
                "{} / {}",
                format_time(position),
                duration.map_or_else(|| "--:--".to_string(), format_time)
            )),
        ];
        if let Some(chapter) = self.config.chapters.at(position) {
            row.push(gap(10.0));
            row.push(chapter_pill(chapter.title()));
        }
        row.push(Expanded::new().child(SizedBox::new()).boxed());
        if let Some(captions) = &self.config.captions {
            let icon = if captions.on { aimer_lucide::Icons::CAPTIONS } else { aimer_lucide::Icons::CAPTIONS_OFF };
            let toggle = captions.clone();
            row.push(icon_button(icon, move || (toggle.on_toggle)(!toggle.on)));
        }
        if let Some(fullscreen) = &self.config.fullscreen {
            let icon = if fullscreen.on { aimer_lucide::Icons::MINIMIZE } else { aimer_lucide::Icons::MAXIMIZE };
            let toggle = fullscreen.clone();
            row.push(icon_button(icon, move || (toggle.on_toggle)(!toggle.on)));
        }

        let (pad_x, pad_y) = BAR_PADDING;
        Container::new()
            .width(Dimension::Percent(100.0))
            .height(BAR_HEIGHT)
            .padding(LayoutSpacing::new().left(pad_x).right(pad_x).top(pad_y).bottom(pad_y))
            .box_decoration(BoxDecoration::new().background_color(Color::Rgb(0, 0, 0).with_opacity(140)))
            .child(
                Column::new().children(vec![
                    self.seek_bar(player, position, duration, (width - pad_x as f32 * 2.0).max(0.0)),
                    SizedBox::new().height(BAR_GAP).boxed(),
                    Row::new()
                        .vertical_alignment(BoxAlignment::Center)
                        .children(row)
                        .boxed(),
                ]),
            )
            .boxed()
    }

    fn seek_bar(&self, player: &VideoPlayer, position: Duration, duration: Option<Duration>, width: f32) -> AnyWidget {
        let total = duration.map_or(0.0, |total| total.as_secs_f64());
        let rail = |color: [u8; 4], fill: Option<(f32, [u8; 4])>| Rail {
            chapters: self.config.chapters.clone(),
            duration,
            inset: THUMB / 2.0,
            height: RAIL,
            color,
            fill,
        };
        let accent = rgba(self.config.accent);
        let buffered = fraction(player.buffered(), duration);
        let dragged = self.scrub.clone();
        let settled = self.scrub.clone();
        let visibility = self.visibility.clone();
        let updater = self.updater;
        let seek = player.clone();
        Slider::new()
            // The range must not be empty while the length is unknown.
            .range(0.0..total.max(SEEK_STEP))
            .step(SEEK_STEP)
            .value(position.as_secs_f64())
            .disabled(duration.is_none())
            .width(width)
            .height(SEEK_HEIGHT)
            .track(rail([255, 255, 255, 70], Some((buffered, [255, 255, 255, 130]))))
            .trail(rail(accent, None))
            .thumb(SliderThumb::new().size(THUMB).radius(THUMB / 2.0).color(self.config.accent))
            .on_change(move |seconds| {
                dragged.set(Some(seconds));
                updater.set_state(|_| {});
            })
            .on_change_end(move |seconds| {
                seek.seek(Duration::from_secs_f64(seconds.max(0.0)));
                settled.set(None);
                let mut state = visibility.get();
                state.note_activity(Instant::now(), state.is_touch());
                visibility.set(state);
                updater.set_state(|_| {});
            })
            .boxed()
    }

    fn play_button(&self, player: &VideoPlayer) -> AnyWidget {
        let icon = if player.wants_frames() { aimer_lucide::Icons::PAUSE } else { aimer_lucide::Icons::PLAY };
        let player = player.clone();
        let updater = self.updater;
        icon_button(icon, move || {
            player.toggle();
            updater.set_state(|_| {});
        })
    }

    fn mute_button(&self, player: &VideoPlayer) -> AnyWidget {
        let icon = if player.is_muted() || player.volume() == 0.0 {
            aimer_lucide::Icons::VOLUME_X
        } else if player.volume() < 0.5 {
            aimer_lucide::Icons::VOLUME_1
        } else {
            aimer_lucide::Icons::VOLUME_2
        };
        let player = player.clone();
        let updater = self.updater;
        icon_button(icon, move || {
            player.set_muted(!player.is_muted());
            updater.set_state(|_| {});
        })
    }

    fn volume_slider(&self, player: &VideoPlayer) -> AnyWidget {
        let value = if player.is_muted() { 0.0 } else { f64::from(player.volume()) };
        let player = player.clone();
        let updater = self.updater;
        Slider::new()
            .range(0.0..1.0)
            .step(0.05)
            .value(value)
            .width(VOLUME_WIDTH)
            .height(SEEK_HEIGHT)
            .trail(SliderTrail::new().height(3.0).radius(1.5).color(Color::Rgb(255, 255, 255)))
            .thumb(SliderThumb::new().size(10.0).radius(5.0).color(Color::Rgb(255, 255, 255)))
            .on_change(move |volume| {
                player.set_volume(volume as f32);
                if player.is_muted() {
                    player.set_muted(false);
                }
                updater.set_state(|_| {});
            })
            .boxed()
    }

    fn center_button(&self, player: &VideoPlayer) -> AnyWidget {
        let icon = if player.wants_frames() { aimer_lucide::Icons::PAUSE } else { aimer_lucide::Icons::PLAY };
        let player = player.clone();
        let updater = self.updater;
        Button::new()
            .on_press(move || {
                player.toggle();
                updater.set_state(|_| {});
            })
            .decoration(BoxDecoration::new().background_color(Color::Rgb(0, 0, 0).with_opacity(130)).border_radius(32))
            .child(
                Container::new()
                    .width(64)
                    .height(64)
                    .child(Align::new().alignment(Alignment::MidCenter).child(icon_image(icon, 30.0))),
            )
            .boxed()
    }
}

fn rgba(color: Color) -> [u8; 4] {
    let (r, g, b, a) = color.to_rgba();
    [r, g, b, a]
}

fn icon_image(icon: &'static StaticSvg, size: f32) -> AnyWidget {
    let white = Color::Rgb(255, 255, 255);
    Svg::new(icon)
        .style("path", SvgStyle::new().stroke(white))
        .style("rect", SvgStyle::new().stroke(white))
        .width(size)
        .height(size)
        .boxed()
}

fn icon_button(icon: &'static StaticSvg, on_press: impl Fn() + 'static) -> AnyWidget {
    Button::new()
        .on_press(on_press)
        .decoration(BoxDecoration::new().background_color(Color::Transparent).border_radius(18))
        .child(
            Container::new()
                .width(BUTTON)
                .height(BUTTON)
                .child(Align::new().alignment(Alignment::MidCenter).child(icon_image(icon, 20.0))),
        )
        .boxed()
}

fn label(text: String) -> AnyWidget {
    Text::new(text)
        .text_style(TextStyle::new().font_size(13).color(Color::Rgb(255, 255, 255)))
        .boxed()
}

fn chapter_pill(title: &str) -> AnyWidget {
    // Fitted, or the row would stretch it to the buttons' height.
    Container::new()
        .fit_content()
        .padding(LayoutSpacing::new().left(10).right(10).top(3).bottom(3))
        .box_decoration(BoxDecoration::new().background_color(Color::Rgb(255, 255, 255).with_opacity(40)).border_radius(12))
        .child(label(title.to_string()))
        .boxed()
}

fn gap(width: f32) -> AnyWidget {
    SizedBox::new().width(width).boxed()
}

/// `m:ss`, or `h:mm:ss` from an hour on, as players show time.
pub(crate) fn format_time(time: Duration) -> String {
    let total = time.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total / 60) % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

impl Widget for VideoControls {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, VideoControlsState>(self, ctx, "VideoControls", None).0.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "VideoControls"
    }
}

#[cfg(test)]
mod tests {
    use super::format_time;
    use std::time::Duration;

    #[test]
    fn times_read_like_a_player_with_hours_from_an_hour_on() {
        assert_eq!(format_time(Duration::ZERO), "0:00");
        assert_eq!(format_time(Duration::from_millis(16_900)), "0:16");
        assert_eq!(format_time(Duration::from_secs(3_599)), "59:59");
        assert_eq!(format_time(Duration::from_secs(3_600)), "1:00:00");
        assert_eq!(format_time(Duration::from_secs(10_810)), "3:00:10");
    }
}
