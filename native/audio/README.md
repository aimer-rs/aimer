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
| `AudioSource::url(..)`     | `http`/`https`. The whole file downloads first; there is no streaming. |

A track opens files by path, so the system streams them from disk. A sound
reads the whole file into memory once, so keep effects short.

## Platforms

| Platform | Backend                                  |
|----------|------------------------------------------|
| macOS    | `AVAudioPlayer`                          |
| iOS      | `AVAudioPlayer`, with a playback session |
| others   | none: loads fail with `AudioError::Unsupported`; see `is_supported()` |

Android and wasm have no backend yet. The engine is platform independent and
talks to a small `Backend`/`Voice` pair, so adding one means implementing those
two traits.

### iOS notes

- The session category is *playback*, so audio plays with the silent switch on.
- App Transport Security blocks plain `http` URLs unless the app allows them.

## Behaviour worth knowing

- `position()` is refreshed about every 30 ms while a track plays.
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
