//! How a wake from outside the frame loop reaches a frame.
//!
//! Work that finishes off the UI thread — an async task, a worker — asks for a
//! frame, and the platform decides when that frame may run. Where a display
//! link paces frames, an immediate wake renders in the middle of a refresh
//! interval, on top of the frame the link is about to start: two renders where
//! the display shows one, and the early one carries the very rebuild work the
//! wake exists for. Routing such a wake through the link makes it ride the next
//! tick like every other request.

/// Where a wake from outside the frame loop is delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameWake {
    /// Raise the pending-frame flag and let the display link start the frame on
    /// its next tick.
    ///
    /// The frame that follows is the one the link starts for every request, so
    /// it is not told why the wake happened.
    DisplayLink,
    /// Post a `FrameReady` user event, which the loop renders as soon as it
    /// gets to it.
    EventLoop,
}

impl FrameWake {
    /// The route this build takes: iOS paces frames with a display link, every
    /// other native target renders when the event loop is woken.
    pub(crate) const PLATFORM: Self = Self::for_target(cfg!(target_os = "ios"));

    /// The route for a target that does (`ios`) or does not pace frames with a
    /// display link.
    pub(crate) const fn for_target(ios: bool) -> Self {
        if ios { Self::DisplayLink } else { Self::EventLoop }
    }
}

/// Runs the delivery `wake` names and nothing else.
#[inline]
pub(crate) fn wake_frame(wake: FrameWake, display_link: impl FnOnce(), event_loop: impl FnOnce()) {
    match wake {
        FrameWake::DisplayLink => display_link(),
        FrameWake::EventLoop => event_loop(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn ios_wakes_ride_the_display_link_and_other_targets_post_to_the_event_loop() {
        assert_eq!(FrameWake::for_target(true), FrameWake::DisplayLink);
        assert_eq!(FrameWake::for_target(false), FrameWake::EventLoop);
    }

    #[test]
    fn the_platform_route_matches_the_target() {
        assert_eq!(
            FrameWake::PLATFORM,
            FrameWake::for_target(cfg!(target_os = "ios"))
        );
    }

    #[test]
    fn a_display_link_wake_never_posts_to_the_event_loop() {
        let link = Cell::new(0);
        let posted = Cell::new(0);
        wake_frame(
            FrameWake::DisplayLink,
            || link.set(link.get() + 1),
            || posted.set(posted.get() + 1),
        );
        assert_eq!((link.get(), posted.get()), (1, 0));
    }

    #[test]
    fn an_event_loop_wake_never_touches_the_display_link() {
        let link = Cell::new(0);
        let posted = Cell::new(0);
        wake_frame(
            FrameWake::EventLoop,
            || link.set(link.get() + 1),
            || posted.set(posted.get() + 1),
        );
        assert_eq!((link.get(), posted.get()), (0, 1));
    }
}
