//! When a frame may be presented by the raster thread alone.
//!
//! A `CAMetalLayer` that presents with the Core Animation transaction ties every
//! frame to the main thread: the raster thread must wait for the GPU to schedule
//! the frame, the main thread joins a transaction, and the frame appears only
//! when that transaction commits. It is the right way to change the layer's
//! size, because content and geometry reach the screen together. It is the
//! wrong way to show a frame whose size has not changed: each one costs a
//! main-thread transaction and the jitter of whatever else the main thread is
//! doing, such as handling touches during a drag.
//!
//! [`PresentGate`] is the UI host's record of which of the two the next frames
//! may use. It starts transactional, goes direct once the layer has shown a few
//! frames at an unchanged size, and goes back the moment the size or scale
//! moves, so the frame that follows a geometry change is always transactional.

/// Transactional presents at one unchanged size before frames may go direct.
///
/// More than one, so that the first frames after a geometry change have
/// settled the layer's frame and scale before the transaction is dropped.
pub(super) const STABLE_PRESENTS: u32 = 3;

/// Which kind of presentation the raster thread should use for new frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PresentMode {
    /// Hand the frame to the main thread, which presents it in a transaction.
    Transactional,
    /// Present from the raster thread as soon as the frame is encoded.
    Direct,
}

pub(super) struct PresentGate {
    /// Whether this platform and configuration may ever go direct.
    allowed: bool,
    mode: PresentMode,
    stable: u32,
}

impl PresentGate {
    /// Whether this build may present directly at all.
    ///
    /// Only iOS, where the layer has no live resize to keep in step with and a
    /// geometry change always arrives as a size change the host sees first.
    /// `AIMER_TRANSACTIONAL_PRESENT` switches it off for a device that
    /// misbehaves with it, without a rebuild.
    pub(super) fn platform_allows_direct() -> bool {
        cfg!(target_os = "ios")
    }

    /// A gate that may go direct only if `allowed`.
    pub(super) fn new(allowed: bool) -> Self {
        Self {
            allowed,
            mode: PresentMode::Transactional,
            stable: 0,
        }
    }

    /// The mode new frames should be encoded for.
    pub(super) fn mode(&self) -> PresentMode {
        self.mode
    }

    /// The layer's size or scale is about to change. Returns the new mode when
    /// that is a change.
    pub(super) fn geometry_changed(&mut self) -> Option<PresentMode> {
        self.stable = 0;
        self.switch_to(PresentMode::Transactional)
    }

    /// A transactional frame reached the screen without the geometry moving.
    /// Returns the new mode when this settles the layer enough to go direct.
    pub(super) fn transactional_frame_presented(&mut self) -> Option<PresentMode> {
        if !self.allowed || self.mode == PresentMode::Direct {
            return None;
        }
        self.stable += 1;
        if self.stable >= STABLE_PRESENTS {
            return self.switch_to(PresentMode::Direct);
        }
        None
    }

    fn switch_to(&mut self, mode: PresentMode) -> Option<PresentMode> {
        if self.mode == mode {
            return None;
        }
        self.mode = mode;
        Some(mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(gate: &mut PresentGate) {
        for _ in 0..STABLE_PRESENTS {
            gate.transactional_frame_presented();
        }
    }

    #[test]
    fn frames_are_transactional_until_the_layer_has_settled() {
        let mut gate = PresentGate::new(true);
        assert_eq!(gate.mode(), PresentMode::Transactional);
        for _ in 0..STABLE_PRESENTS - 1 {
            assert_eq!(gate.transactional_frame_presented(), None);
            assert_eq!(gate.mode(), PresentMode::Transactional);
        }
        assert_eq!(
            gate.transactional_frame_presented(),
            Some(PresentMode::Direct)
        );
        assert_eq!(gate.mode(), PresentMode::Direct);
    }

    #[test]
    fn a_geometry_change_returns_to_transactional_and_starts_counting_again() {
        let mut gate = PresentGate::new(true);
        settled(&mut gate);
        assert_eq!(gate.geometry_changed(), Some(PresentMode::Transactional));
        assert_eq!(gate.mode(), PresentMode::Transactional);

        // The count restarts: one stable frame short of settled is not enough.
        for _ in 0..STABLE_PRESENTS - 1 {
            assert_eq!(gate.transactional_frame_presented(), None);
        }
        assert_eq!(gate.mode(), PresentMode::Transactional);
    }

    #[test]
    fn a_change_that_interrupts_the_count_discards_it() {
        let mut gate = PresentGate::new(true);
        for _ in 0..STABLE_PRESENTS - 1 {
            gate.transactional_frame_presented();
        }
        assert_eq!(gate.geometry_changed(), None, "already transactional");
        assert_eq!(gate.transactional_frame_presented(), None);
        assert_eq!(gate.mode(), PresentMode::Transactional);
    }

    #[test]
    fn a_gate_that_is_not_allowed_never_goes_direct() {
        let mut gate = PresentGate::new(false);
        for _ in 0..STABLE_PRESENTS * 4 {
            assert_eq!(gate.transactional_frame_presented(), None);
        }
        assert_eq!(gate.mode(), PresentMode::Transactional);
        assert_eq!(gate.geometry_changed(), None);
    }

    #[test]
    fn staying_direct_reports_no_further_change() {
        let mut gate = PresentGate::new(true);
        settled(&mut gate);
        assert_eq!(gate.transactional_frame_presented(), None);
        assert_eq!(gate.mode(), PresentMode::Direct);
    }

    #[test]
    fn only_ios_may_go_direct_by_default() {
        assert_eq!(PresentGate::platform_allows_direct(), cfg!(target_os = "ios"));
    }
}
