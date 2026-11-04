//! Video playback with the standard player UI: a bundled clip with captions
//! and chapters, or an HLS stream, drawn straight from the decoder's GPU
//! memory.
//!
//! [`VideoPlayerView`] brings the seek bar (cut into chapters), play/pause,
//! volume, time, captions and fullscreen; on a touch screen it adds a large
//! play/pause button in the middle. The [`VideoPlayer`] lives in the page's
//! state, so rebuilding the page never restarts it, and a paused page draws
//! nothing.

use aimer::style::{FontWeight, LayoutSpacing, TextStyle, Theme, ThemeData};
use aimer::video::{
    CaptionSource, CaptionTrack, Chapters, VideoFit, VideoPlayer, VideoPlayerView, VideoSource,
};
use aimer::{
    BuildContext, Column, Container, Row, ScrollAxis, Scrollable, SizedBox, State, StateUpdater,
    StatefulWidget, Text, Widget,
};

use crate::audio_example::{control_button, gap};
use crate::{Assets, theme};

/// Apple's public HLS test stream: adaptive bitrate, with sound.
const STREAM_URL: &str =
    "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8";
/// The size the player is shown at, in logical pixels.
const PLAYER: (f32, f32) = (640.0, 360.0);

/// Builds the video showcase page.
pub fn video_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(VideoExample::new())
}

#[derive(aimer::StatefulWidget)]
pub struct VideoExample;

impl VideoExample {
    pub fn new() -> Self {
        Self
    }
}

pub struct VideoExampleState {
    player: VideoPlayer,
    streaming: bool,
    fit: VideoFit,
    /// The bundled clip's sidecar files; `None` where they did not load.
    captions: Option<CaptionTrack>,
    chapters: Chapters,
    updater: StateUpdater<Self>,
}

fn bundled_clip() -> VideoSource {
    VideoSource::bundled(Assets::TEST_PATTERN.path())
}

impl StatefulWidget for VideoExample {
    type State = VideoExampleState;

    fn create_state(self) -> VideoExampleState {
        let player = VideoPlayer::from_source(bundled_clip());
        player.set_volume(0.6);
        let captions =
            CaptionTrack::load(&CaptionSource::Bundled(Assets::TEST_PATTERN_CAPTIONS.path().into())).ok();
        let chapters = Chapters::load(&CaptionSource::Bundled(Assets::TEST_PATTERN_CHAPTERS.path().into()))
            .unwrap_or_default();
        VideoExampleState {
            player,
            streaming: false,
            fit: VideoFit::Contain,
            captions,
            chapters,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<VideoExample> for VideoExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = self.updater;

        let mut view = VideoPlayerView::new()
            .player(self.player.clone())
            .fit(self.fit)
            .width(PLAYER.0)
            .height(PLAYER.1);
        // The sidecar files belong to the bundled clip, not the stream.
        if !self.streaming {
            if let Some(captions) = &self.captions {
                view = view.captions(captions.clone());
            }
            view = view.chapters(self.chapters.clone());
        }

        let fit = {
            let label = match self.fit {
                VideoFit::Contain => "Fit: contain",
                VideoFit::Cover => "Fit: cover",
                VideoFit::Fill => "Fit: fill",
            };
            control_button(label, app_theme, false, move || {
                updater.set_state(|state| {
                    state.fit = match state.fit {
                        VideoFit::Contain => VideoFit::Cover,
                        VideoFit::Cover => VideoFit::Fill,
                        VideoFit::Fill => VideoFit::Contain,
                    };
                });
            })
        };
        let source = {
            let player = self.player.clone();
            let label = if self.streaming { "Source: HLS stream" } else { "Source: bundled clip" };
            control_button(label, app_theme, false, move || {
                let player = player.clone();
                updater.set_state(move |state| {
                    state.streaming = !state.streaming;
                    let source = if state.streaming { VideoSource::url(STREAM_URL) } else { bundled_clip() };
                    player.load(source);
                    player.play();
                });
            })
        };

        Container::new()
            .padding(LayoutSpacing::all(28))
            .child(Column::new().children(vec![
                Text::new("Video")
                    .text_style(
                        TextStyle::new()
                            .font_size(26)
                            .font_weight(FontWeight::Bold)
                            .color(app_theme.on_surface_color),
                    )
                    .boxed(),
                SizedBox::new().height(8).boxed(),
                Text::new(
                    "The standard player: drag the chaptered seek bar, toggle captions, go \
                     fullscreen. The picture is the decoder's own GPU texture, drawn without a \
                     copy, and a paused video draws nothing.",
                )
                .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
                .boxed(),
                SizedBox::new().height(20).boxed(),
                view.boxed(),
                SizedBox::new().height(16).boxed(),
                Row::new().children(vec![fit, gap(), source]).boxed(),
                SizedBox::new().height(12).boxed(),
                Text::new("Video plays on macOS and iOS for now.")
                    .text_style(TextStyle::new().font_size(13).color(theme::muted_text(&app_theme)))
                    .boxed(),
            ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_builds_as_a_widget_without_starting_the_application() {
        fn assert_widget(_widget: impl Widget) {}
        assert_widget(video_example());
    }

    #[test]
    fn the_bundled_sidecar_files_parse() {
        let captions = CaptionTrack::parse(include_str!("../assets/test_pattern.vtt")).unwrap();
        assert_eq!(captions.cues().len(), 3);
        let chapters = Chapters::parse(include_str!("../assets/test_pattern_chapters.vtt")).unwrap();
        assert_eq!(chapters.chapters().len(), 3);
    }
}
