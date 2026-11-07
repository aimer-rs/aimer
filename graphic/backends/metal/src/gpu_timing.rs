//! Measuring how long the GPU spends on one frame.
//!
//! A frame is usually several command buffers. Each gets a completion handler
//! that reports its GPU start and end time here, and the frame's GPU time is the
//! span from the earliest start to the latest end, reported exactly once after
//! the renderer has finished submitting and every buffer has completed.

use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct GpuFrameCapture {
    report: fn(Duration),
    state: Mutex<GpuFrameCaptureState>,
}

#[derive(Debug, Default)]
struct GpuFrameCaptureState {
    pending_submissions: usize,
    encoding_finished: bool,
    invalid_timestamp: bool,
    has_interval: bool,
    gpu_start_seconds: f64,
    gpu_end_seconds: f64,
    reported: bool,
}

impl GpuFrameCapture {
    pub(crate) fn new(report: fn(Duration)) -> Self {
        Self {
            report,
            state: Mutex::new(GpuFrameCaptureState::default()),
        }
    }

    pub(crate) fn register_submission(&self) {
        let mut state = self
            .state
            .lock()
            .expect("Metal GPU frame timing state is not poisoned");
        state.pending_submissions = state.pending_submissions.saturating_add(1);
    }

    pub(crate) fn complete_submission(&self, gpu_start_seconds: f64, gpu_end_seconds: f64) {
        let duration = {
            let mut state = self
                .state
                .lock()
                .expect("Metal GPU frame timing state is not poisoned");
            if gpu_start_seconds.is_finite()
                && gpu_end_seconds.is_finite()
                && gpu_start_seconds > 0.0
                && gpu_end_seconds > gpu_start_seconds
            {
                if state.has_interval {
                    state.gpu_start_seconds = state.gpu_start_seconds.min(gpu_start_seconds);
                    state.gpu_end_seconds = state.gpu_end_seconds.max(gpu_end_seconds);
                } else {
                    state.gpu_start_seconds = gpu_start_seconds;
                    state.gpu_end_seconds = gpu_end_seconds;
                    state.has_interval = true;
                }
            } else {
                state.invalid_timestamp = true;
            }
            state.pending_submissions = state.pending_submissions.saturating_sub(1);
            ready_gpu_duration(&mut state)
        };
        if let Some(duration) = duration {
            (self.report)(duration);
        }
    }

    pub(crate) fn finish_encoding(&self) {
        let duration = {
            let mut state = self
                .state
                .lock()
                .expect("Metal GPU frame timing state is not poisoned");
            state.encoding_finished = true;
            ready_gpu_duration(&mut state)
        };
        if let Some(duration) = duration {
            (self.report)(duration);
        }
    }
}

fn ready_gpu_duration(state: &mut GpuFrameCaptureState) -> Option<Duration> {
    if !state.encoding_finished || state.pending_submissions != 0 || state.reported {
        return None;
    }
    state.reported = true;
    if state.invalid_timestamp || !state.has_interval {
        return None;
    }
    let seconds = state.gpu_end_seconds - state.gpu_start_seconds;
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static REPORTED: RefCell<Vec<Duration>> = const { RefCell::new(Vec::new()) };
    }

    fn report(elapsed: Duration) {
        REPORTED.with(|reported| reported.borrow_mut().push(elapsed));
    }

    fn reported() -> Vec<Duration> {
        REPORTED.with(|reported| reported.borrow().clone())
    }

    #[test]
    fn a_frame_reports_the_span_of_all_its_submissions_exactly_once() {
        let capture = GpuFrameCapture::new(report);
        capture.register_submission();
        capture.register_submission();
        capture.finish_encoding();
        capture.complete_submission(10.000, 10.004);
        assert!(reported().is_empty(), "one submission is still running");
        capture.complete_submission(10.002, 10.009);
        assert_eq!(reported().len(), 1);
        assert!((reported()[0].as_secs_f64() - 0.009).abs() < 1e-9, "{:?}", reported());
        capture.finish_encoding();
        assert_eq!(reported().len(), 1, "a frame is reported once");
    }

    #[test]
    fn nothing_is_reported_before_encoding_finishes() {
        let capture = GpuFrameCapture::new(report);
        capture.register_submission();
        capture.complete_submission(5.0, 5.002);
        assert!(reported().is_empty(), "later submissions may still arrive");
        capture.finish_encoding();
        assert_eq!(reported().len(), 1);
    }

    #[test]
    fn an_invalid_timestamp_discards_the_whole_frame() {
        for (start, end) in [
            (0.0, 1.0),
            (2.0, 2.0),
            (3.0, 2.0),
            (f64::NAN, 2.0),
            (1.0, f64::INFINITY),
        ] {
            REPORTED.with(|reported| reported.borrow_mut().clear());
            let capture = GpuFrameCapture::new(report);
            capture.register_submission();
            capture.register_submission();
            capture.complete_submission(10.0, 10.002);
            capture.complete_submission(start, end);
            capture.finish_encoding();
            assert!(reported().is_empty(), "({start}, {end}) must not be reported");
        }
    }

    #[test]
    fn a_frame_with_no_submission_reports_nothing() {
        let capture = GpuFrameCapture::new(report);
        capture.finish_encoding();
        assert!(reported().is_empty());
    }
}
