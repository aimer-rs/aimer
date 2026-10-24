#![deny(missing_docs)]

//! Platform-neutral feedback state and overlay lifecycle contracts.
//!
//! The crate deliberately contains no renderer, modal host, or process-global
//! state. Applications connect the models to their existing overlay and
//! accessibility systems through [`OverlayHost`] and [`Announcer`].

mod clock;
mod overlay;
mod progress;
mod status;
mod toast;
mod tooltip;

pub use clock::{Clock, ManualClock, SystemClock};
pub use overlay::{
    Announcement, AnnouncementPriority, Announcer, DismissReason, FocusTarget, OverflowPolicy,
    OverlayHost, OverlayId, OverlayKind, OverlayLifecycle, OverlayModality, OverlayRequest,
    OverlaySide, PlacementAlign, PlacementSpec, Rect, ResolvedPlacement, Size, resolve_placement,
};
pub use progress::{
    MotionPolicy, ProgressError, ProgressIndicator, ProgressSemantics, ProgressState, Spinner,
    SpinnerError,
};
pub use status::{FeedbackSlot, StatusBanner, StatusKind};
pub use toast::{Snackbar, Toast, ToastAction, ToastId, ToastKind, ToastQueue, ToastQueueEvent};
pub use tooltip::{Tooltip, TooltipController, TooltipEvent, TooltipTouch, TouchPolicy};

#[cfg(test)]
mod tests {
    use super::{Clock, ManualClock};
    use std::time::Duration;

    #[test]
    fn manual_clock_can_be_advanced_without_sleeping() {
        let clock = ManualClock::new();
        assert_eq!(clock.now(), Duration::ZERO);

        clock.advance(Duration::from_millis(25));

        assert_eq!(clock.now(), Duration::from_millis(25));
    }
}
