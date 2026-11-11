# aimer_audio

Audio playback for Aimer, with two handles for two jobs:

- **[`AudioPlayer`]**: one long track (music, a podcast, a voice message) with
  a playhead: `play`, `pause`, `toggle`, `stop`, `seek`, `position`,
  `duration`, volume, mute and a loop count.
- **[`Sound`]**: a short effect (a tap, a chime) that is decoded once and can
  overlap itself, up to eight instances at a time.

Both are cheap, cloneable handles that are safe to call from any thread and
never block. Loading, decoding and playback run on one internal audio thread,
so a widget can keep a handle in its state and call it from a button callback.

```rust,no_run
use aimer_audio::{AudioLoops, AudioPlayer, AudioSource, Sound};

let music = AudioPlayer::from_source(AudioSource::bundled("assets/theme.mp3"));
music.set_loops(AudioLoops::Infinite);
music.play(); // starts as soon as the file is ready

let tap = Sound::from_source(AudioSource::bundled("assets/tap.wav"));
tap.play();
```

Availability: macOS, iOS, Android 7+, Windows 10+, Linux, Web

## Sources

| Source                     | Notes                                                                  |
|----------------------------|------------------------------------------------------------------------|
| `AudioSource::bundled(..)` | The key from `[assets]` in `Aimer.toml`, e.g. `Assets::THEME.path()`.   |
| `AudioSource::file(..)`    | A path on the host file system.                                        |
| `AudioSource::url(..)`     | `http`/`https`. The whole file downloads first, then seeks and loops are instant (the web streams it). |
| `AudioSource::stream(..)`  | `http`/`https`, played while it downloads; see [Streaming](#streaming). |

A track opens files by path, so the system streams them from disk. A sound
reads the whole file into memory once, so keep effects short.

## Streaming

`AudioSource::stream(url)` starts playing once enough has arrived instead of
waiting for the whole file, which suits podcasts and long tracks.

```rust,no_run
use aimer_audio::{AudioPlayer, AudioSource, PlaybackState};

let player = AudioPlayer::from_source(AudioSource::stream("https://example.com/episode.mp3"));
player.play();

// From a frame callback:
match player.state() {
    PlaybackState::Buffering => { /* show a spinner */ }
    _ => { /* progress bar: player.position() of player.duration(), */
           /* with player.buffered() as the loaded part */ }
}
```

- `duration()` is `None` until the first bytes reveal it. `position()` and
  `buffered()` update with the same 30 ms tick as a local track.
- `PlaybackState::Buffering` means "asked to play, waiting for data"; it
  resumes by itself and `is_playing()` stays `true`.
- A dropped connection fails the track (`AudioError::Network`) instead of
  finishing it.
- Seeking past the buffered part waits for the network, then resumes.
- Loops are driven by the engine by rewinding the same player, so a looping
  stream does not download again.
- Only an `AudioPlayer` streams. A `Sound` that is given a stream source
  downloads it like `AudioSource::url`.
- Streams are not cached: a new `load` of the same URL downloads it again.

## Platforms

| Platform | Backend |
|----------|---------|
| macOS    | `AVAudioPlayer`; `AVPlayer` for streams |
| iOS      | `AVAudioPlayer`, with a playback session; `AVPlayer` for streams |
| Android  | `android.media.MediaPlayer` over JNI |
| Windows  | WinRT `Windows.Media.Playback.MediaPlayer` |
| Linux    | Symphonia decoding, mixed and played through ALSA |
| Web      | `HTMLAudioElement` |
| others   | none: loads fail with `AudioError::Unsupported`; see `is_supported()` |

The Linux backend's tests run anywhere, against ALSA's built-in `null` device.
The Android, Windows and web backends, and streaming on Apple platforms, are
new and have only been compiled so far, not run on a device.

Every backend sits behind a small `Backend`/`Voice` pair, and the engine that
drives them is platform independent.

### iOS notes

- The session category is *playback*, so audio plays with the silent switch on.
- App Transport Security blocks plain `http` URLs unless the app allows them.

### Android notes

- Bundled assets are read from the APK. An asset stored compressed is copied to
  the cache directory once; audio files are stored uncompressed by default.
- In-memory audio (a downloaded `AudioSource::url`, a `Sound`) is written to the
  cache directory once, because `MediaPlayer` cannot read a byte array.
- `MediaPlayer` reports buffering and errors only to Java listeners, which a
  Rust library cannot install. So `buffered()` is unknown for a stream, and
  `Buffering` covers only the wait before a stream starts. A stream that stops
  more than 1.5 s before its end is reported as `AudioError::Network`.
- Plain `http` URLs need `android:usesCleartextTraffic` or a network security
  config.

### Windows notes

- Players opt out of the system media controls (the media keys and the volume
  flyout).
- A source opens asynchronously, so `duration()` appears a tick after `Ready`,
  and a file that is not audio fails with `AudioError::Decode` shortly after
  it loads rather than during the load.

### Linux notes

- Decoding covers MP3, AAC (ADTS and MP4/M4A), FLAC, Ogg Vorbis, WAV and ADPCM.
- Output goes to ALSA's `default` device (PipeWire or PulseAudio on a desktop).
  Set `AIMER_AUDIO_ALSA_DEVICE` to use another one. `libasound.so.2` is loaded
  at run time, so building needs no ALSA headers. Without it, loads fail with
  `AudioError::Playback`.
- A `Sound` is decoded once into memory and mixed sample-accurately. A track is
  decoded about half a second ahead by a thread of its own.
- A stream keeps everything it has downloaded, so seeking back is instant.
  Seeking ahead waits for the download to get there. A server that sends no
  `Content-Length` gives a stream that cannot be seeked or looped.
- Debug builds decode noticeably slower than release builds. Effects decode
  once, but a long compressed track in a debug build costs some CPU.

### Web notes

- Bundled assets are fetched from the site root (`/assets/...`), like bundled
  images. `AudioSource::file` fails with `AudioError::Unsupported`.
- `AudioSource::url` streams, like `AudioSource::stream`: the browser decides
  how much to download ahead.
- Browsers refuse to start audio before the user has interacted with the page.
  The refusal fails the track with `AudioError::Playback`, so call `play()`
  from an input handler.
- The engine runs on the page's thread and ticks from a `setInterval` timer,
  only while something plays or is still loading.

## Behaviour worth knowing

- `position()` is refreshed about every 30 ms while a track plays.
- `seek()` publishes the new position at once. Seeks that queue up for one
  track (a seek bar dragged faster than the audio thread applies them) collapse
  into the latest. A seek bar is still best built with `Slider::on_change_end`:
  show the dragged value while the finger is down and seek once on release.
- A streamed track is also ticked while it is paused until its length is known
  and all of it is downloaded, so `duration()` and `buffered()` keep updating.
- `play()` before `load()` does nothing; `play()` while loading starts the
  track the moment it is ready.
- A finished track reads `PlaybackState::Finished`, and `play()` restarts it.
- `AudioLoops::Count(n)` plays the track `n` times in total. `Infinite` is
  looped by the system without a gap. A finite count is driven by the engine,
  which opens a fresh player at the end of each pass, so expect a short silence
  (up to about 30 ms plus the open time) between passes. Two probed
  `AVAudioPlayer` quirks are behind this: a finite `numberOfLoops` repeats a
  0.3 s clip one time too few, and a player that has played to its end ignores
  `play` afterwards.
- The first sound of a process takes 0.5 to 3 s in a debug build while the
  system starts its audio stack. The engine starts it as soon as the first
  handle is created; call `aimer_audio::warm_up()` earlier if you want that
  paid at app start-up.
- Dropping the last clone of a handle stops it.
