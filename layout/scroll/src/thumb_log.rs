/// Logs one line about a finger's hold on the scroll bar thumb.
///
/// Every line carries the wall-clock time it was written at, so the line for
/// the moment the finger landed can be told apart in time from the one that
/// says the thumb was taken, and both from what the person was doing. The
/// `thumb-hold` tag is what to filter on.
macro_rules! thumb_log {
    ($($arg:tt)*) => {
        aimer_utils::debug!(
            "[thumb-hold {}] {}",
            aimer_utils::wall_clock_label(),
            format_args!($($arg)*)
        )
    };
}
