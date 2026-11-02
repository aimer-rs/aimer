# aimer_time

Time utilities for the Aimer framework and for applications built on it.

Everything here works on native targets and in the browser: the clocks use
`std::time` natively and the browser's `performance.now()` on wasm, where
`std::time` would panic.

| Module | What it gives you |
| --- | --- |
| `Instant`, `Clock` | A monotonic instant, plus a `SystemClock` and a hand-driven `ManualClock` so time-dependent code is testable without sleeping. |
| `FrameClock` | Per-frame delta, total elapsed time, frame count and a smoothed FPS estimate. |
| `Stopwatch`, `Countdown`, `Debounce`, `Throttle` | Poll-based timers. They never spawn threads or allocate; each takes an explicit `now` so they are deterministic. |
| `Date`, `DateTime`, `Weekday` | A proleptic Gregorian UTC calendar: unix conversion, weekday, ordinal, arithmetic, ISO 8601 parsing and `strftime`-style formatting. |
| `human`, `DurationExt` | Compact duration display (`1h 02m 03s`) and frequency helpers. |

```rust
use std::time::Duration;
use aimer_time::{Clock, Debounce, ManualClock};

let clock = ManualClock::new();
let mut search = Debounce::new(Duration::from_millis(300));

search.trigger_at(clock.now());
clock.advance(Duration::from_millis(100));
assert!(!search.poll_at(clock.now()));
clock.advance(Duration::from_millis(250));
assert!(search.poll_at(clock.now()));
```

```rust
use aimer_time::DateTime;

let moment = DateTime::from_unix_timestamp(0, 0).unwrap();
assert_eq!(moment.to_string(), "1970-01-01T00:00:00Z");
assert_eq!(moment.format("%a %d %b %Y"), "Thu 01 Jan 1970");
```

Local time zones are intentionally out of scope: calendar values are UTC.
