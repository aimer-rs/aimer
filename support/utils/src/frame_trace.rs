//! Opt-in recording of frame-scheduling events, for finding out where a frame
//! loses time between the display's vsync and the pixels it draws.
//!
//! Nothing is recorded unless the `AIMER_FRAME_TRACE` environment variable is
//! set, and the check is made once. Events are timestamped on the same clock
//! as `CACurrentMediaTime` / `CADisplayLink::timestamp` (`CLOCK_UPTIME_RAW`)
//! so a Swift host can feed its own readings in and the two line up exactly.
//!
//! Recording is kept in memory and written out in batches, because a write per
//! event would itself disturb the frame cadence being measured. The file is
//! `$HOME/Documents/frame-trace.txt` on iOS (readable with `xcrun devicectl
//! device copy from`) and `$TMPDIR/aimer-frame-trace.txt` elsewhere. Each line is
//! `<seconds> <kind> <a> <b>`; what `a` and `b` mean depends on the kind.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Events buffered before they are written without waiting for a flush point.
const FLUSH_THRESHOLD: usize = 4096;

/// Hard cap on buffered events, so a trace that is never flushed cannot grow
/// without bound. Events past the cap are dropped.
const BUFFER_CAP: usize = 32_768;

/// Whether tracing was asked for with `AIMER_FRAME_TRACE`.
#[inline]
pub fn frame_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("AIMER_FRAME_TRACE").is_some_and(|v| v != "0"))
}

/// Seconds on the clock `CACurrentMediaTime` reads, or on a process-local
/// monotonic clock where there is no such thing.
pub fn frame_trace_now() -> f64 {
    #[cfg(target_vendor = "apple")]
    {
        unsafe extern "C" {
            fn clock_gettime_nsec_np(clock_id: u32) -> u64;
        }
        const CLOCK_UPTIME_RAW: u32 = 8;
        // SAFETY: the function takes a clock id and has no other requirements.
        unsafe { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) as f64 / 1e9 }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    }
}

/// An in-memory list of recorded events and how they print.
#[derive(Default)]
struct Recording {
    events: Vec<(f64, &'static str, f64, f64)>,
    dropped: usize,
}

impl Recording {
    fn push(&mut self, at: f64, kind: &'static str, a: f64, b: f64) {
        if self.events.len() >= BUFFER_CAP {
            self.dropped += 1;
            return;
        }
        self.events.push((at, kind, a, b));
    }

    /// Takes the buffered events as text, one line per event.
    fn drain_text(&mut self) -> String {
        let mut text = String::with_capacity(self.events.len() * 40);
        for (at, kind, a, b) in self.events.drain(..) {
            text.push_str(&format!("{at:.6} {kind} {a:.6} {b:.6}\n"));
        }
        if self.dropped > 0 {
            text.push_str(&format!("dropped {}\n", self.dropped));
            self.dropped = 0;
        }
        text
    }
}

fn recording() -> &'static Mutex<Recording> {
    static RECORDING: OnceLock<Mutex<Recording>> = OnceLock::new();
    RECORDING.get_or_init(Mutex::default)
}

fn trace_path() -> PathBuf {
    #[cfg(target_os = "ios")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join("Documents").join("frame-trace.txt")
    }
    #[cfg(not(target_os = "ios"))]
    {
        std::env::temp_dir().join("aimer-frame-trace.txt")
    }
}

/// Records one event when tracing is enabled; otherwise costs a single load.
///
/// `kind` names the event and `a` / `b` carry whatever numbers explain it
/// (a display timestamp, a flag as `0.0` / `1.0`, a touch phase).
#[inline]
pub fn frame_trace(kind: &'static str, a: f64, b: f64) {
    if !frame_trace_enabled() {
        return;
    }
    frame_trace_at(frame_trace_now(), kind, a, b);
    if recording().lock().is_ok_and(|r| r.events.len() >= FLUSH_THRESHOLD) {
        frame_trace_flush();
    }
}

#[inline]
fn frame_trace_at(at: f64, kind: &'static str, a: f64, b: f64) {
    if let Ok(mut recording) = recording().lock() {
        recording.push(at, kind, a, b);
    }
}

/// Appends everything recorded so far to the trace file.
///
/// Call it where the app is idle anyway (the display link going to sleep), not
/// in the middle of a frame. The first flush of a run replaces the old file.
pub fn frame_trace_flush() {
    if !frame_trace_enabled() {
        return;
    }
    let text = match recording().lock() {
        Ok(mut recording) => recording.drain_text(),
        Err(_) => return,
    };
    if text.is_empty() {
        return;
    }
    static FIRST: OnceLock<()> = OnceLock::new();
    let first = FIRST.set(()).is_ok();
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if first {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    if let Ok(mut file) = options.open(trace_path()) {
        let _ = file.write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_print_one_per_line_in_order() {
        let mut recording = Recording::default();
        recording.push(1.5, "tick", 1.0, 2.0);
        recording.push(2.25, "touch", 0.0, 0.0);
        assert_eq!(
            recording.drain_text(),
            "1.500000 tick 1.000000 2.000000\n2.250000 touch 0.000000 0.000000\n"
        );
        assert!(recording.events.is_empty());
    }

    #[test]
    fn events_past_the_cap_are_counted_not_stored() {
        let mut recording = Recording::default();
        for _ in 0..BUFFER_CAP + 3 {
            recording.push(0.0, "x", 0.0, 0.0);
        }
        assert_eq!(recording.events.len(), BUFFER_CAP);
        assert!(recording.drain_text().ends_with("dropped 3\n"));
        assert_eq!(recording.dropped, 0);
    }

    #[test]
    fn nothing_is_recorded_while_tracing_is_off() {
        // The test environment does not set AIMER_FRAME_TRACE.
        assert!(!frame_trace_enabled());
        let before = recording().lock().unwrap().events.len();
        frame_trace("tick", 0.0, 0.0);
        assert_eq!(recording().lock().unwrap().events.len(), before);
    }

    #[test]
    fn the_clock_does_not_run_backwards() {
        let a = frame_trace_now();
        let b = frame_trace_now();
        assert!(b >= a);
    }
}
