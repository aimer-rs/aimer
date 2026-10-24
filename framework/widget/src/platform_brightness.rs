//! The light or dark appearance the platform currently asks for, and the
//! widgets that follow it.
//!
//! The operating system owns this answer: macOS, Windows, Android and the
//! browser all let the user switch appearance while an application is running,
//! and they announce the switch instead of restarting the app. The platform
//! layer feeds that announcement in here through
//! [`set_platform_brightness`]; a widget that read the appearance while it was
//! building is rebuilt, and every other widget is left alone — the same
//! contract [`crate::window_metrics`] gives readers of the window.
//!
//! Registration lasts for one build. The next build of the same widget clears
//! it and re-registers only what that build reads, so a widget that stops
//! asking about the appearance stops paying for it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::components::context::BuildConsumer;

/// The appearance the platform asks the application to draw itself in.
///
/// This is the platform's answer, not the application's decision: a theme may
/// follow it or ignore it.
///
/// # Examples
///
/// ```
/// use aimer_widget::Brightness;
///
/// assert!(Brightness::Dark.is_dark());
/// assert_eq!(Brightness::default(), Brightness::Light);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Brightness {
    /// Dark content on a light background.
    #[default]
    Light,
    /// Light content on a dark background.
    Dark,
}

impl Brightness {
    /// Returns `true` for [`Brightness::Dark`].
    #[inline]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }

    /// Returns `true` for [`Brightness::Light`].
    #[inline]
    pub const fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }
}

impl From<winit::window::Theme> for Brightness {
    #[inline]
    fn from(theme: winit::window::Theme) -> Self {
        match theme {
            winit::window::Theme::Light => Self::Light,
            winit::window::Theme::Dark => Self::Dark,
        }
    }
}

/// Address of this static is the dependency identity of the platform
/// appearance.
///
/// [`BuildConsumer::register_dependency`] keys on an address, so taking one
/// that no `Rc` can ever hand out keeps the appearance from colliding with a
/// provider or with the window metrics.
static PLATFORM_BRIGHTNESS: u8 = 0;

#[inline]
fn identity() -> usize {
    &PLATFORM_BRIGHTNESS as *const u8 as usize
}

thread_local! {
    /// The appearance last reported by the platform.
    ///
    /// Held per thread because it is only ever written by the platform layer on
    /// the thread that owns the event loop, and only ever read by the widget
    /// tree on that same thread. A platform that never reports an appearance
    /// leaves the light default in place.
    static CURRENT: Cell<Brightness> = const { Cell::new(Brightness::Light) };

    /// Every widget whose current build read the appearance.
    ///
    /// The consumer is held weakly, so a widget that has left the tree simply
    /// stops answering rather than having to be unregistered from wherever it
    /// died.
    static SUBSCRIBERS: RefCell<HashMap<u64, Weak<BuildConsumer>>> = RefCell::new(HashMap::new());
    static NEXT_SUBSCRIBER: Cell<u64> = const { Cell::new(0) };
}

/// Reads the appearance the platform last reported.
///
/// Reading through this function registers nothing: a widget that has to follow
/// later changes reads it with
/// [`BuildContext::watch_platform_brightness`](crate::base::BuildContext::watch_platform_brightness)
/// instead.
#[inline]
pub fn platform_brightness() -> Brightness {
    CURRENT.with(Cell::get)
}

/// Reports a new platform appearance and rebuilds the widgets that follow it.
///
/// Called by the platform layer when the window is created and whenever the
/// system announces a change. Returns how many widgets were marked for rebuild,
/// which tells the caller whether the change is visible at all — a change no
/// widget follows needs no frame.
///
/// Reporting the appearance that is already in effect marks nothing: platforms
/// re-announce the current appearance freely, and a repaint per announcement
/// would be a repaint for nothing.
///
/// Widgets that have since left the tree are dropped here rather than rebuilt.
pub fn set_platform_brightness(brightness: Brightness) -> usize {
    if CURRENT.with(Cell::get) == brightness {
        return 0;
    }
    CURRENT.with(|current| current.set(brightness));
    SUBSCRIBERS.with(|subscribers| {
        let mut subscribers = subscribers.borrow_mut();
        let mut notified = 0;
        subscribers.retain(|_, consumer| {
            let Some(consumer) = consumer.upgrade() else {
                return false;
            };
            consumer.mark_needs_rebuild();
            notified += 1;
            true
        });
        notified
    })
}

/// Registers the widget currently building as a follower of the platform
/// appearance, to be rebuilt whenever it changes.
///
/// Repeated reads within one build cost a hash lookup and nothing more.
pub(crate) fn subscribe(consumer: &Rc<BuildConsumer>) {
    if !consumer.register_dependency(identity()) {
        return;
    }
    let id = NEXT_SUBSCRIBER.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    SUBSCRIBERS.with(|subscribers| {
        subscribers
            .borrow_mut()
            .insert(id, Rc::downgrade(consumer));
    });
    consumer.add_cleanup(move || {
        SUBSCRIBERS.with(|subscribers| {
            subscribers.borrow_mut().remove(&id);
        });
    });
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn consumer() -> (Rc<BuildConsumer>, Rc<Cell<bool>>) {
        let dirty = Rc::new(Cell::new(false));
        (BuildConsumer::new(dirty.clone()), dirty)
    }

    /// The store is process-wide per thread, so every test starts from light.
    fn reset() {
        CURRENT.with(|current| current.set(Brightness::Light));
    }

    #[test]
    fn a_widget_that_read_the_appearance_is_rebuilt_when_it_changes() {
        reset();
        let (consumer, dirty) = consumer();
        subscribe(&consumer);

        set_platform_brightness(Brightness::Dark);

        assert!(dirty.get(), "the follower was not marked for rebuild");
        assert_eq!(platform_brightness(), Brightness::Dark);
    }

    #[test]
    fn a_widget_that_ignored_the_appearance_is_left_alone() {
        reset();
        let (_consumer, dirty) = consumer();

        set_platform_brightness(Brightness::Dark);

        assert!(
            !dirty.get(),
            "an appearance change rebuilt a widget that never asked"
        );
    }

    #[test]
    fn reporting_the_current_appearance_again_rebuilds_nothing() {
        reset();
        let (consumer, dirty) = consumer();
        subscribe(&consumer);

        let notified = set_platform_brightness(Brightness::Light);

        assert_eq!(notified, 0);
        assert!(!dirty.get(), "a repeated announcement cost a rebuild");
    }

    #[test]
    fn reading_the_appearance_twice_in_one_build_registers_once() {
        reset();
        let (consumer, _dirty) = consumer();
        let before = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());

        subscribe(&consumer);
        subscribe(&consumer);

        let after = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        assert_eq!(after - before, 1);
    }

    #[test]
    fn a_widget_that_stops_reading_the_appearance_stops_being_rebuilt() {
        reset();
        let (consumer, dirty) = consumer();
        subscribe(&consumer);

        // What a rebuild does before running `build` again: retire everything
        // the previous build depended on. This one does not read the
        // appearance.
        consumer.begin_build();
        dirty.set(false);

        set_platform_brightness(Brightness::Dark);

        assert!(!dirty.get(), "the stale registration outlived the build");
    }

    #[test]
    fn a_widget_dropped_from_the_tree_is_forgotten() {
        reset();
        let (consumer, _dirty) = consumer();
        subscribe(&consumer);
        let registered = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        drop(consumer);

        set_platform_brightness(Brightness::Dark);

        let remaining = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        assert!(remaining < registered);
    }
}
