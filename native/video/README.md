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

Availability: macOS, iOS, Android 8+, Windows 10+, Linux, Web

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
| Windows       | `MediaPlayer` (Media Foundation), frames as shared Direct3D textures |
| Android       | `MediaPlayer` into a `SurfaceTexture`, converted to RGBA on the GPU, shared as hardware buffers |
| Linux         | `ffmpeg` decodes (software), pictures uploaded once each; sound through `aimer_audio` |
| web           | `HTMLVideoElement`, its picture copied into a texture on the GPU |
| others        | none yet: loads fail with `VideoError::Unsupported`; see `is_supported()` |

Hardware-decoded zero-copy video on Linux is planned.
The player core is platform independent and talks to a small `PlatformVideo`
trait; a backend also needs its renderer to import its texture kind, through
`GpuBackend::import_native_texture`.

The macOS and iOS backend has been compiled but not yet run on a device.

### On Android

`MediaPlayer` decodes into a `Surface` made from a `SurfaceTexture`, which shows
the picture as an OpenGL ES external texture in the decoder's own (often YUV)
format. The image pipeline samples plain RGBA, so a small GL pass draws that
texture into an `AHardwareBuffer` of RGBA pixels, once per picture, on the GPU;
the Vulkan backend imports the buffer and samples it where it lies. No pixel
reaches the CPU.

- One worker thread per player owns the EGL context, the `SurfaceTexture` and
  the `MediaPlayer`; the handle only sends commands and reads a snapshot, so
  nothing on the UI thread waits on JNI or the GPU. EGL, OpenGL ES and
  `AHardwareBuffer` are opened with `dlopen`, so building needs nothing from
  the NDK.
- No Java listener can be installed (a Rust library cannot ship a Java class),
  so the worker polls: a new picture shows as the `SurfaceTexture`'s timestamp
  changing, and the end as `isPlaying()` turning false near the end. A stream
  that stops early is reported as failed. A stream's buffered range is unknown.
- Pictures are drawn into a ring of six buffers; a buffer is drawn into again
  only when no frame holds it. When the renderer is slower than the video the
  newest pictures are dropped.
- It needs the Vulkan renderer with `VK_ANDROID_external_memory_android_hardware_buffer`
  (every device with Android 8 and Vulkan 1.0 has it). On the OpenGL renderer
  the video draws nothing.
- A bundled video is read from the APK in place when it is stored uncompressed
  (the usual for `.mp4`) and copied to the cache directory otherwise.
- This backend has been compiled for `aarch64-linux-android` but not yet run on
  a device.

### On Linux

There is no system player, so the backend runs the `ffmpeg` program (and
`ffprobe` to learn the size and length) and reads raw RGBA pictures from its
output. Installing FFmpeg is the app's or the user's job; without it a load
fails with `VideoError::Playback` saying so. A program, not a linked library,
keeps this independent of which FFmpeg version the system has. FFmpeg streams
`http(s)` and HLS itself.

- The soundtrack plays through `aimer_audio` and is the clock when it runs:
  pictures are shown when the sound reaches them. A video without sound, or on
  a machine with no audio device, runs on the wall clock.
- FFmpeg decodes on the GPU where it can drive one (`-hwaccel auto`: VAAPI,
  VDPAU, CUDA...), and falls back to software by itself. If the GPU decoder
  makes a source fail before it shows a picture, the player retries once in
  software, from where it was, and stays there. Set `AIMER_VIDEO_HWACCEL=0` to
  never try the GPU, and `AIMER_FFMPEG` / `AIMER_FFPROBE` to name the programs.
- FFmpeg downloads the decoded pictures to system memory, converts them to RGBA
  there, and each is uploaded to the GPU once (`NativeTexture::Rgba8`). So the
  GPU does the decoding but the picture still crosses the CPU: this is not
  zero-copy. Four pictures are decoded ahead and no more, so a paused video
  decodes nothing.
- Seeking restarts FFmpeg at the new position; it is accurate, not instant.
- Handing the decoder's GPU surfaces to the renderer without the trip through
  system memory (VAAPI to a Vulkan image over DMA-BUF) is not implemented: it
  needs FFmpeg's C structures, whose layout changes between versions, and a
  machine with a VAAPI device to test on.

### On Windows

`MediaPlayer` runs in frame-server mode: it plays the sound and raises an event
per picture, and each picture is copied (converted from YUV on the GPU) into a
Direct3D 11 texture shared by NT handle. The renderer's Direct3D 12 backend
opens the handle, waits on a shared fence for the copy to finish, and samples
the texture as it is, so the picture is never copied through the CPU. Both
sides use the system's default hardware adapter; on a machine with several GPUs
where the renderer chose another, the video draws nothing. This backend has
been compiled but not yet run on Windows.

### On the web

The browser fetches, streams, decodes and plays the sound, so the backend is a
detached `<video>` element. A browser offers no video texture to share, so the
renderer's backend (WebGL2 or WebGPU) copies the element's current picture into
a texture of its own, on the GPU, once per picture: no pixel reaches the CPU.
Things to know:

- Playback must start from an input handler; a browser refuses it otherwise,
  and the player then reports `VideoError::Playback`.
- Elements are created with `crossorigin="anonymous"`, because a picture the
  page may not read cannot be copied into a texture. A video from another
  origin must be served with CORS headers.
- A bundled asset is fetched from the site root; `VideoSource::file` is
  `Unsupported`.
- Sidecar captions and chapters cannot be read from files on the web; parse
  their text (`CaptionTrack::parse`, say from `include_str!`).

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
