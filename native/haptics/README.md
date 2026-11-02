# aimer_haptics

Haptic feedback for Aimer.

- **[`Haptics`]** — a `cfg`-free function-style API for widgets
  (`impact`, `selection`, `notification`, `prepare_impact`, `play_pattern`).
  Every call is safe on every target and does nothing where there is no
  hardware or no backend, so call sites never need a platform guard.

## Platforms

| Platform | Canned effects                                    | `play_pattern`                     |
|----------|---------------------------------------------------|------------------------------------|
| iOS      | UIKit feedback generators                         | Core Haptics (intensity + sharpness) |
| macOS    | `NSHapticFeedbackManager` (trackpad)              | not supported, does nothing        |
| Android  | `Vibrator` / `VibrationEffect` over JNI           | amplitude waveform (intensity only) |
| others   | no-op                                             | no-op                              |

### macOS

AppKit exposes three feedbacks — generic, alignment and level change — and no
way to vary or schedule them. `Light`/`Soft` map to generic, `Medium`/`Rigid`
and `selection` to alignment, `Heavy` to level change. A success notification
is alignment; warnings and errors are level change. Feedback is only felt on a
Force Touch trackpad, and only from the main thread; elsewhere it is skipped.

### Android

The app's manifest must declare the normal (install-time) permission:

```xml
<uses-permission android:name="android.permission.VIBRATE" />
```

Without it the vibrator refuses with a `SecurityException`, which is written to
the log and otherwise ignored. Ticks and clicks use the device's predefined
effects on API 29+ and an amplitude one-shot on API 26+; API 24–25 fall back to
plain on/off timing. `Soft` shares the tick with `Light` and `Rigid` the click
with `Medium`. Devices with no vibrator are detected once and then ignored.
