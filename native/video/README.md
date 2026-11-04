# aimer_video

Video playback, drawn straight from the decoder's GPU memory.

- **[`VideoPlayer`]**: a cheap, cloneable handle with `play`, `pause`,
  `toggle`, `stop`, `seek`, `position`, `duration`, `buffered`, `size`,
  volume, mute and a loop count. The video's sound plays in sync with it.
- **[`Video`]**: the widget that shows a player's picture, with a
  [`VideoFit`] of `Contain` (letterbox), `Cover` (crop) or `Fill` (stretch).
- **[`VideoPlayerView`]**: the standard player: the picture, captions and
  controls, with fullscreen. Its parts are public too: [`VideoControls`] and
  [`Captions`].

```rust,no_run
use aimer_video::{Video, VideoFit, VideoLoops, VideoPlayer, VideoSource};

let player = VideoPlayer::from_source(VideoSource::bundled("assets/intro.mp4"));
player.set_loops(VideoLoops::Infinite);
player.play();

// In a build method; buttons get clones of the same player.
let video = Video::new().player(player.clone()).fit(VideoFit::Cover).height(240);
```

## The standard player

```rust,no_run
use aimer_video::{CaptionSource, CaptionTrack, Chapters, VideoPlayer, VideoPlayerView, VideoSource};

let player = VideoPlayer::from_source(VideoSource::bundled("assets/talk.mp4"));
let captions = CaptionTrack::load(&CaptionSource::Bundled("assets/talk.vtt".into())).unwrap();
let chapters = Chapters::load(&CaptionSource::Bundled("assets/talk_chapters.vtt".into())).unwrap();
let view = VideoPlayerView::new()
    .player(player)
    .captions(captions)
    .chapters(chapters)
    .height(360);
```

`VideoPlayerView` lays out:

- **The seek bar**: what has played (in the accent colour), what has
  downloaded, and a thumb you can drag. It shows the dragged position while
  the pointer is down and seeks once, on release. With chapters, it is cut
  into one segment per chapter.
- **Play/pause, mute and volume**, the **time** (`m:ss`, or `h:mm:ss` from
  an hour on), and the **current chapter's title**.
- **A captions button** when there are captions, and **a fullscreen button**.
- **On a touch screen**, a large play/pause button in the middle.

While the video plays, the controls hide three seconds after the last pointer
activity and come back on the next. Clicking the picture with a mouse plays or
pauses; on a touch screen a tap shows or hides the controls instead. Paused,
the controls stay up.

Fullscreen puts the window into fullscreen
(`WindowHandle::set_fullscreen`) and shows the same player over the whole
window; the button leaves it again. The video plays on throughout.

### Captions and chapters

Captions come from sidecar files, WebVTT (`.vtt`) or SubRip (`.srt`), parsed
in Rust and drawn as framework text, so they look the same on every platform.
Styling tags (`<i>`, `<c.yellow>`, `<v Speaker>`, `{\an8}`) are removed,
entities decoded, overlapping cues shown together, and a malformed cue skipped
rather than the whole file. Cue positions and styles from the file are not
applied yet; set the look with `CaptionStyle`.

Chapters come from a WebVTT chapters file (each cue's start and text are a
chapter's start and title) or from a list of `Chapter`s.

`CaptionSource` is a bundled asset, a file, or text the app already holds (one
it downloaded itself, say). Captions are not read from inside the media yet.

## Sources

| Source                     | Notes                                                                 |
|----------------------------|-----------------------------------------------------------------------|
| `VideoSource::bundled(..)` | The key from `[assets]` in `Aimer.toml`, e.g. `Assets::INTRO.path()`. |
| `VideoSource::file(..)`    | A path on the host file system.                                       |
| `VideoSource::url(..)`     | `http`/`https`, streamed while it downloads; HLS (`.m3u8`) adapts.    |

## How frames reach the screen

No frame is copied through the CPU. The platform decoder writes each frame
into GPU memory, and the renderer samples that memory directly:

1. The player's output hands out decoded frames as GPU surfaces (on Apple,
   IOSurface-backed `CVPixelBuffer`s in BGRA, already converted from YUV by
   the hardware decoder).
2. Each frame is wrapped as a renderer texture (`CVMetalTextureCache`), with
   no copy.
3. The `Video` widget records one `DrawExternalTexture` command. The render
   tree repaints the widget only when a new frame arrives, and the renderer
   keeps each frame alive until the GPU has finished drawing it.

The widget asks for frames only while the video plays, is still opening, or
is fetching the one picture a seek, load or stop produced while paused. A
paused video costs nothing per frame: while it rests, the player answers the
widget from its own state and makes no platform call, even when something
else keeps the window redrawing. Check `VideoPlayer::wants_frames` to drive
your own frame-by-frame UI (a position readout, say) the same way.

## Platforms

| Platform      | Backend                                                         |
|---------------|-----------------------------------------------------------------|
| macOS         | `AVPlayer` + `AVPlayerItemVideoOutput`, frames as Metal textures |
| iOS           | the same, with a *playback* audio session                       |
| others        | none yet: loads fail with `VideoError::Unsupported`; see `is_supported()` |

Linux (FFmpeg loaded at run time), Android, Windows and the web are planned.
The player core is platform independent and talks to a small `PlatformVideo`
trait; a backend also needs its renderer to import its texture kind, through
`GpuBackend::import_native_texture`.

The macOS and iOS backend has been compiled but not yet run on a device.

## Behaviour worth knowing

- `duration()` and `size()` are `None` until the source has opened; a live
  stream has no duration.
- `PlaybackState::Buffering` means "asked to play, waiting for data". It
  resumes by itself, and `is_playing()` stays `true`.
- A failed stream reports an error instead of finishing.
- `play()` after the end starts again from the top. Loops are driven by the
  player, which seeks back to the start at the end of each pass, so expect a
  short gap between passes.
- Seeking is not frame-exact: the platform lands on the nearest point it can
  decode from cheaply.
- Dropping the last clone of a player stops it.
- iOS: App Transport Security blocks plain `http` URLs unless the app allows
  them.
