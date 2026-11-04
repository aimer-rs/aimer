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

## Sources

| Source                     | Notes                                                                  |
|----------------------------|------------------------------------------------------------------------|
| `AudioSource::bundled(..)` | The key from `[assets]` in `Aimer.toml`, e.g. `Assets::THEME.path()`.   |
| `AudioSource::file(..)`    | A path on the host file system.                                        |
| `AudioSource::url(..)`     | `http`/`https`. The whole file downloads first, then seeks and loops are instant. |
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

| Platform | Backend                                  |
|----------|------------------------------------------|
| macOS    | `AVAudioPlayer`; `AVPlayer` for streams  |
| iOS      | `AVAudioPlayer`, with a playback session; `AVPlayer` for streams |
| others   | none: loads fail with `AudioError::Unsupported`; see `is_supported()` |

Android and wasm have no backend yet. The engine is platform independent and
talks to a small `Backend`/`Voice` pair, so adding one means implementing those
two traits.

### iOS notes

- The session category is *playback*, so audio plays with the silent switch on.
- App Transport Security blocks plain `http` URLs unless the app allows them.

## Behaviour worth knowing

- `position()` is refreshed about every 30 ms while a track plays.
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
