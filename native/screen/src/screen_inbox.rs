//! Delivering platform announcements to the thread that owns the widget tree.
//!
//! Android reports a fold on the JVM's UI thread; the widget tree and the
//! readers in [`crate::screen_state`] live on the event-loop thread, and a
//! widget is not something another thread may touch. A platform thread therefore
//! only *posts* here, and the event loop *applies* what was posted on its own
//! thread, the same way links reach it.
//!
//! Only the latest value of each kind is kept: a device that is rotated and
//! rotated back before the loop wakes is, to the widgets, never rotated at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::{
    DeviceOrientation, DisplayFeature, HingeState, set_device_orientation, set_display_features,
    set_hinge,
};

#[derive(Default)]
struct Pending {
    /// `Some(None)` is a posted "unknown", distinct from nothing posted.
    orientation: Option<Option<DeviceOrientation>>,
    features: Option<Vec<DisplayFeature>>,
    /// `Some(None)` is a hinge that went away, distinct from nothing posted.
    hinge: Option<Option<HingeState>>,
}

/// The mailbox. A type rather than two statics so a test owns its own.
pub(crate) struct Inbox {
    pending: Mutex<Pending>,
    /// Whether `pending` may hold something, so an idle poll is one load.
    has_pending: AtomicBool,
}

impl Inbox {
    pub(crate) const fn new() -> Self {
        Self {
            pending: Mutex::new(Pending {
                orientation: None,
                features: None,
                hinge: None,
            }),
            has_pending: AtomicBool::new(false),
        }
    }

    pub(crate) fn post_orientation(&self, orientation: Option<DeviceOrientation>) {
        self.post(|pending| pending.orientation = Some(orientation));
    }

    pub(crate) fn post_features(&self, features: Vec<DisplayFeature>) {
        self.post(|pending| pending.features = Some(features));
    }

    pub(crate) fn post_hinge(&self, hinge: Option<HingeState>) {
        self.post(|pending| pending.hinge = Some(hinge));
    }

    fn post(&self, store: impl FnOnce(&mut Pending)) {
        store(&mut self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        // After the unlock: a taker that sees the flag finds the value.
        self.has_pending.store(true, Ordering::Release);
    }

    /// Moves what was posted onto the calling thread's tree, and returns how
    /// many widgets it marked.
    pub(crate) fn apply(&self) -> usize {
        if !self.has_pending.swap(false, Ordering::Acquire) {
            return 0;
        }
        let Pending {
            orientation,
            features,
            hinge,
        } = std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        // Applied outside the lock: marking widgets runs arbitrary rebuild
        // bookkeeping, and a platform thread must never wait on that.
        orientation.map_or(0, set_device_orientation)
            + features.map_or(0, set_display_features)
            + hinge.map_or(0, set_hinge)
    }
}

static INBOX: Inbox = Inbox::new();

/// Posts the device's orientation from any thread.
///
/// Nothing happens to a widget until the event-loop thread calls
/// [`apply_pending`]; the caller is responsible for waking it.
pub fn post_device_orientation(orientation: Option<DeviceOrientation>) {
    INBOX.post_orientation(orientation);
}

/// Posts the display's folds and hinges from any thread.
///
/// See [`post_device_orientation`] for when it takes effect.
pub fn post_display_features(features: Vec<DisplayFeature>) {
    INBOX.post_features(features);
}

/// Posts the state of the hinge from any thread; `None` when there is none.
///
/// See [`post_device_orientation`] for when it takes effect.
pub fn post_hinge(hinge: Option<HingeState>) {
    INBOX.post_hinge(hinge);
}

static WAKER: OnceLock<fn()> = OnceLock::new();

/// Registers how to wake the event loop when the platform announces a change
/// from inside UIKit's own callbacks.
///
/// On iOS the platform tells this crate about a hinge from a callback of its
/// own, not from the event loop, so after posting there is nobody to call
/// [`apply_pending`]. The host registers a function that wakes its loop (and so
/// arrives at [`apply_pending`]) once, at start-up; a second registration is
/// ignored. Platforms whose host wakes the loop itself never need this.
pub fn set_change_waker(waker: fn()) {
    let _ = WAKER.set(waker);
}

/// Wakes the event loop through the registered waker, if there is one.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn wake() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}

/// Applies everything posted since the last call, on the calling thread, and
/// returns how many widgets it marked for rebuild.
///
/// Call it from the event-loop thread, when woken for a screen change. When
/// nothing was posted it costs one atomic load, and returns `0`.
pub fn apply_pending() -> usize {
    INBOX.apply()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_widget::base::BuildConsumer;

    use super::*;
    use crate::{Bounds, FoldState, HingeStatus, device_orientation, display_features, hinge as current_hinge};

    fn hinge() -> DisplayFeature {
        DisplayFeature {
            bounds: Bounds::new(1350.0, 0.0, 1434.0, 1800.0),
            occludes: true,
            state: FoldState::HalfOpened,
            separating: true,
        }
    }

    #[test]
    fn an_empty_inbox_applies_nothing() {
        assert_eq!(Inbox::new().apply(), 0);
    }

    #[test]
    fn a_value_posted_from_another_thread_lands_on_this_one() {
        let inbox = Inbox::new();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                inbox.post_orientation(Some(DeviceOrientation::LandscapeRight));
                inbox.post_features(vec![hinge()]);
            });
        });
        assert_eq!(device_orientation(), None, "nothing applies before apply");

        inbox.apply();

        assert_eq!(device_orientation(), Some(DeviceOrientation::LandscapeRight));
        assert_eq!(display_features().as_ref(), [hinge()]);
    }

    #[test]
    fn only_the_latest_post_of_a_kind_survives() {
        let inbox = Inbox::new();
        inbox.post_orientation(Some(DeviceOrientation::LandscapeLeft));
        inbox.post_orientation(Some(DeviceOrientation::PortraitDown));

        inbox.apply();

        assert_eq!(device_orientation(), Some(DeviceOrientation::PortraitDown));
    }

    #[test]
    fn a_posted_unknown_clears_the_orientation() {
        let inbox = Inbox::new();
        inbox.post_orientation(Some(DeviceOrientation::LandscapeLeft));
        inbox.apply();
        inbox.post_orientation(None);

        inbox.apply();

        assert_eq!(device_orientation(), None);
    }

    #[test]
    fn a_post_is_applied_once() {
        let inbox = Inbox::new();
        inbox.post_features(vec![hinge()]);
        inbox.apply();

        // Cleared by something else since; a second apply must not resurrect it.
        set_display_features(Vec::new());
        assert_eq!(inbox.apply(), 0);
        assert!(display_features().is_empty());
    }

    #[test]
    fn applying_reports_the_widgets_it_marked() {
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());
        crate::screen_state::subscribe_features_for_test(&consumer);
        let inbox = Inbox::new();
        inbox.post_features(vec![hinge()]);

        assert_eq!(inbox.apply(), 1);
        assert!(dirty.get());
    }

    #[test]
    fn the_global_entry_points_share_one_mailbox() {
        post_device_orientation(Some(DeviceOrientation::PortraitUp));
        post_display_features(vec![hinge()]);

        apply_pending();

        assert_eq!(device_orientation(), Some(DeviceOrientation::PortraitUp));
        assert_eq!(display_features().len(), 1);
    }

    #[test]
    fn a_hinge_posted_from_another_thread_lands_on_this_one() {
        let inbox = Inbox::new();
        let state = HingeState::new(1.2, HingeStatus::PartiallyOpen);
        std::thread::scope(|scope| {
            scope.spawn(|| inbox.post_hinge(Some(state)));
        });
        assert_eq!(current_hinge(), None, "nothing applies before apply");

        inbox.apply();

        assert_eq!(current_hinge(), Some(state));
    }

    #[test]
    fn a_posted_absent_hinge_clears_the_hinge() {
        let inbox = Inbox::new();
        inbox.post_hinge(Some(HingeState::new(3.0, HingeStatus::FullyOpen)));
        inbox.apply();
        inbox.post_hinge(None);

        inbox.apply();

        assert_eq!(current_hinge(), None);
    }
}
