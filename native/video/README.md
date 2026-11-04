# aimer_video

Video playback, drawn straight from the decoder's GPU memory.

- **[`VideoPlayer`]**: a cheap, cloneable handle with `play`, `pause`,
  `toggle`, `stop`, `seek`, `position`, `duration`, `buffered`, `size`,
  volume, mute and a loop count. The video's sound plays in sync with it.
- **[`Video`]**: the widget that shows a player's picture, with a
  [`VideoFit`] of `Contain` (letterbox), `Cover` (crop) or `Fill` (stretch).

```rust,no_run
use aimer_video::{Video, VideoFit, VideoLoops, VideoPlayer, VideoSource};

let player = VideoPlayer::from_source(VideoSource::bundled("assets/intro.mp4"));
player.set_loops(VideoLoops::Infinite);
player.play();

// In a build method; buttons get clones of the same player.
let video = Video::new().player(player.clone()).fit(VideoFit::Cover).height(240);
```

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

The widget asks for frames only while the video plays or is still opening, so
a paused video costs nothing per frame.

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
