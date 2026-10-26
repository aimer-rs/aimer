//! The device's orientation and display features, and the widgets that follow
//! them.
//!
//! The operating system owns both answers and announces changes: a phone is
//! turned, a foldable is half opened. The platform layer feeds each announcement
//! in through [`set_device_orientation`] and [`set_display_features`]; a widget
//! that read the value while it was building is rebuilt, and every other widget
//! is left alone — the same contract `aimer_widget` gives readers of the window
//! and of the platform appearance.
//!
//! The two are separate dependencies. A widget that asked which way the device
//! is turned is not rebuilt because a hinge moved, and the other way round.
//!
//! Registration lasts for one build. The next build of the same widget clears it
//! and registers only what that build reads, so a widget that stops asking stops
//! paying. The current values and the readers are per thread, like the widget
//! tree they belong to; [`crate::apply_pending`] moves values posted from other
//! threads onto the tree's own.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::thread::LocalKey;

use aimer_widget::base::{BuildConsumer, BuildContext};

use crate::{DeviceOrientation, DisplayFeature};

/// Addresses of these are the dependency identities of the two values.
///
/// [`BuildConsumer::register_dependency`] keys on an address, so taking one that
/// no `Rc` can hand out keeps them from colliding with a provider's.
static DEPENDENCY_KEYS: [u8; 2] = [0, 0];

#[inline]
fn orientation_key() -> usize {
    &DEPENDENCY_KEYS[0] as *const u8 as usize
}

#[inline]
fn features_key() -> usize {
    &DEPENDENCY_KEYS[1] as *const u8 as usize
}

/// The widgets whose current build read one of the values.
///
/// Consumers are held weakly, so a widget that has left the tree simply stops
/// answering instead of having to be unregistered from wherever it died.
struct Readers {
    consumers: RefCell<BTreeMap<u64, Weak<BuildConsumer>>>,
    next: Cell<u64>,
}

impl Readers {
    const fn new() -> Self {
        Self {
            consumers: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
        }
    }

    fn add(&self, consumer: Weak<BuildConsumer>) -> u64 {
        let id = self.next.get();
        self.next.set(id.wrapping_add(1));
        self.consumers.borrow_mut().insert(id, consumer);
        id
    }

    fn remove(&self, id: u64) {
        self.consumers.borrow_mut().remove(&id);
    }

    /// Marks every live reader for rebuild, forgets the dead ones, and returns
    /// how many were marked.
    fn notify(&self) -> usize {
        let mut notified = 0;
        self.consumers.borrow_mut().retain(|_, consumer| {
            let Some(consumer) = consumer.upgrade() else {
                return false;
            };
            consumer.mark_needs_rebuild();
            notified += 1;
            true
        });
        notified
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.consumers.borrow().len()
    }
}

thread_local! {
    static ORIENTATION: Cell<Option<DeviceOrientation>> = const { Cell::new(None) };
    static FEATURES: RefCell<Rc<[DisplayFeature]>> = RefCell::new(Rc::from(Vec::new()));
    static ORIENTATION_READERS: Readers = const { Readers::new() };
    static FEATURE_READERS: Readers = const { Readers::new() };
}

/// Registers `consumer` with `readers` for the rest of its current build.
fn subscribe(readers: &'static LocalKey<Readers>, key: usize, consumer: &Rc<BuildConsumer>) {
    if !consumer.register_dependency(key) {
        return;
    }
    let id = readers.with(|readers| readers.add(Rc::downgrade(consumer)));
    consumer.add_cleanup(move || {
        // The thread may be shutting down with the consumer still alive.
        let _ = readers.try_with(|readers| readers.remove(id));
    });
}

/// Which way the device is turned, as last reported.
///
/// `None` when the platform has not said, or cannot: a desktop computer has no
/// rotation, and a device lying flat reports none. This does not subscribe the
/// caller to anything; a widget reads it through
/// [`MediaQuery::device_orientation_of`](crate::MediaQuery::device_orientation_of).
pub fn device_orientation() -> Option<DeviceOrientation> {
    ORIENTATION.with(Cell::get)
}

/// The folds and hinges in the display, as last reported.
///
/// Empty on every device that has none. This does not subscribe the caller to
/// anything; a widget reads it through
/// [`MediaQuery::display_features_of`](crate::MediaQuery::display_features_of).
pub fn display_features() -> Rc<[DisplayFeature]> {
    FEATURES.with(|features| features.borrow().clone())
}

/// Records the device's orientation and rebuilds the widgets that follow it.
///
/// Returns how many widgets were marked, so the caller knows whether a frame is
/// needed. A value equal to the current one marks nothing: the platform repeats
/// itself, and a repaint for it would be wasted. Widgets that have since left
/// the tree are dropped here rather than rebuilt.
///
/// Call this on the thread that owns the widget tree; from any other thread use
/// [`post_device_orientation`](crate::post_device_orientation).
pub fn set_device_orientation(orientation: Option<DeviceOrientation>) -> usize {
    if ORIENTATION.with(Cell::get) == orientation {
        return 0;
    }
    ORIENTATION.with(|current| current.set(orientation));
    ORIENTATION_READERS.with(Readers::notify)
}

/// Records the display's folds and hinges and rebuilds the widgets that follow
/// them.
///
/// Returns how many widgets were marked. A list equal to the current one marks
/// nothing. See [`set_device_orientation`] for the threading rule.
pub fn set_display_features(features: Vec<DisplayFeature>) -> usize {
    let unchanged = FEATURES.with(|current| current.borrow().as_ref() == features.as_slice());
    if unchanged {
        return 0;
    }
    FEATURES.with(|current| *current.borrow_mut() = Rc::from(features));
    FEATURE_READERS.with(Readers::notify)
}

/// Reads the orientation for the widget currently building and subscribes it.
pub(crate) fn watch_device_orientation(context: &BuildContext) -> Option<DeviceOrientation> {
    if let Some(consumer) = context.current_build_consumer() {
        subscribe_orientation(&consumer);
    }
    device_orientation()
}

/// Reads the display features for the widget currently building and subscribes
/// it.
pub(crate) fn watch_display_features(context: &BuildContext) -> Rc<[DisplayFeature]> {
    if let Some(consumer) = context.current_build_consumer() {
        subscribe_features(&consumer);
    }
    display_features()
}

fn subscribe_orientation(consumer: &Rc<BuildConsumer>) {
    subscribe(&ORIENTATION_READERS, orientation_key(), consumer);
}

fn subscribe_features(consumer: &Rc<BuildConsumer>) {
    subscribe(&FEATURE_READERS, features_key(), consumer);
}

#[cfg(test)]
pub(crate) fn subscribe_features_for_test(consumer: &Rc<BuildConsumer>) {
    subscribe_features(consumer);
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::{Bounds, FoldState};

    fn consumer() -> (Rc<BuildConsumer>, Rc<Cell<bool>>) {
        let dirty = Rc::new(Cell::new(false));
        (BuildConsumer::new(dirty.clone()), dirty)
    }

    fn hinge(left: f32) -> DisplayFeature {
        DisplayFeature {
            bounds: Bounds::new(left, 0.0, left + 84.0, 1800.0),
            occludes: true,
            state: FoldState::HalfOpened,
            separating: true,
        }
    }

    #[test]
    fn nothing_is_reported_until_the_platform_says() {
        assert_eq!(device_orientation(), None);
        assert!(display_features().is_empty());
    }

    #[test]
    fn a_reported_orientation_is_readable() {
        set_device_orientation(Some(DeviceOrientation::LandscapeLeft));
        assert_eq!(device_orientation(), Some(DeviceOrientation::LandscapeLeft));
        set_device_orientation(None);
        assert_eq!(device_orientation(), None);
    }

    #[test]
    fn turning_the_device_rebuilds_a_widget_that_read_the_orientation() {
        let (consumer, dirty) = consumer();
        subscribe_orientation(&consumer);

        let notified = set_device_orientation(Some(DeviceOrientation::LandscapeRight));

        assert_eq!(notified, 1);
        assert!(dirty.get(), "the reader was not marked for rebuild");
    }

    #[test]
    fn repeating_the_orientation_rebuilds_nothing() {
        set_device_orientation(Some(DeviceOrientation::PortraitUp));
        let (consumer, dirty) = consumer();
        subscribe_orientation(&consumer);

        let notified = set_device_orientation(Some(DeviceOrientation::PortraitUp));

        assert_eq!(notified, 0);
        assert!(!dirty.get(), "an unchanged orientation rebuilt a widget");
    }

    #[test]
    fn a_hinge_does_not_rebuild_a_widget_that_only_read_the_orientation() {
        let (consumer, dirty) = consumer();
        subscribe_orientation(&consumer);

        set_display_features(vec![hinge(1350.0)]);

        assert!(!dirty.get(), "a hinge rebuilt an orientation reader");
    }

    #[test]
    fn turning_the_device_does_not_rebuild_a_widget_that_only_read_the_features() {
        let (consumer, dirty) = consumer();
        subscribe_features(&consumer);

        set_device_orientation(Some(DeviceOrientation::PortraitDown));

        assert!(!dirty.get(), "a rotation rebuilt a feature reader");
    }

    #[test]
    fn a_changed_hinge_rebuilds_its_readers_and_an_equal_one_does_not() {
        let (consumer, dirty) = consumer();
        subscribe_features(&consumer);

        assert_eq!(set_display_features(vec![hinge(1350.0)]), 1);
        assert!(dirty.get());
        assert_eq!(display_features().as_ref(), [hinge(1350.0)]);

        dirty.set(false);
        assert_eq!(set_display_features(vec![hinge(1350.0)]), 0);
        assert!(!dirty.get(), "an identical list rebuilt a widget");

        assert_eq!(set_display_features(Vec::new()), 1, "unfolding is a change");
        assert!(display_features().is_empty());
    }

    #[test]
    fn reading_twice_in_one_build_registers_once() {
        let (consumer, _dirty) = consumer();
        let before = ORIENTATION_READERS.with(Readers::len);

        subscribe_orientation(&consumer);
        subscribe_orientation(&consumer);

        assert_eq!(ORIENTATION_READERS.with(Readers::len) - before, 1);
    }

    #[test]
    fn a_widget_dropped_from_the_tree_is_forgotten() {
        let (consumer, _dirty) = consumer();
        subscribe_features(&consumer);
        assert_eq!(FEATURE_READERS.with(Readers::len), 1);

        drop(consumer);

        assert_eq!(FEATURE_READERS.with(Readers::len), 0);
        assert_eq!(set_display_features(vec![hinge(10.0)]), 0);
    }
}
