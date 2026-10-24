//! The part of the window the system keeps for itself, and the widgets that
//! stay out of it.
//!
//! A window is not always fully usable. iOS draws the status bar, the notch and
//! the home indicator *over* the application's surface; Android does the same
//! with its status and navigation bars and a display cutout; a browser tab on a
//! notched phone exposes the same region through the `env(safe-area-inset-*)`
//! CSS variables. Content painted there is still visible, but a touch landing
//! on it belongs to the system — which is why a panel placed under the status
//! bar cannot be pressed at all.
//!
//! The platform layer reports the region through [`set_safe_area_insets`], in
//! logical pixels measured inwards from each edge of the window. Everything
//! that positions itself against the window rather than against a parent —
//! [`crate::window_metrics`]' readers, floating panels, context menus — asks for
//! it here.
//!
//! Widgets that read the insets while building are rebuilt when they change,
//! and only those; the contract, the registration lifetime and the cost are
//! exactly [`crate::platform_brightness`]'.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use aimer_events::window::WindowKey;

use crate::components::context::{BuildConsumer, WindowHandle};

/// The window edges the system reserves, in logical pixels.
///
/// Each field is the distance from that edge of the window to the first pixel
/// an application may safely place something interactive on. All-zero — the
/// default — means the whole window is usable, which is the answer on a desktop
/// window and on a platform that reports nothing.
///
/// # Examples
///
/// ```
/// use aimer_widget::SafeAreaInsets;
///
/// // An iPhone in portrait: status bar on top, home indicator at the bottom.
/// let insets = SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0);
///
/// assert!(!insets.is_zero());
/// assert_eq!(insets.top, 59.0);
/// assert_eq!(SafeAreaInsets::ZERO, SafeAreaInsets::default());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SafeAreaInsets {
    /// Reserved width along the left edge.
    pub left: f32,
    /// Reserved height along the top edge.
    pub top: f32,
    /// Reserved width along the right edge.
    pub right: f32,
    /// Reserved height along the bottom edge.
    pub bottom: f32,
}

impl SafeAreaInsets {
    /// A window with nothing reserved.
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// Creates insets from the four edges, in logical pixels.
    ///
    /// Negative and non-finite values are treated as `0.0`: a platform that
    /// reports nonsense must not push content off the screen.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_widget::SafeAreaInsets;
    ///
    /// let insets = SafeAreaInsets::new(-4.0, f32::NAN, 0.0, 34.0);
    ///
    /// assert_eq!(insets.left, 0.0);
    /// assert_eq!(insets.top, 0.0);
    /// assert_eq!(insets.bottom, 34.0);
    /// ```
    #[inline]
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left: sanitize(left),
            top: sanitize(top),
            right: sanitize(right),
            bottom: sanitize(bottom),
        }
    }

    /// Creates the same inset on all four edges.
    ///
    /// Useful as a comfort margin folded into the system's own reservation.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_widget::SafeAreaInsets;
    ///
    /// assert_eq!(SafeAreaInsets::all(8.0).right, 8.0);
    /// ```
    #[inline]
    pub fn all(inset: f32) -> Self {
        Self::new(inset, inset, inset, inset)
    }

    /// Returns whether the whole window is usable.
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.left == 0.0 && self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0
    }

    /// Returns the larger inset of each edge.
    ///
    /// This is how a comfort margin is combined with the system's reservation:
    /// the margin applies where the system asks for less, and is absorbed where
    /// it asks for more.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_widget::SafeAreaInsets;
    ///
    /// let combined = SafeAreaInsets::new(0.0, 59.0, 0.0, 0.0).max(SafeAreaInsets::all(8.0));
    ///
    /// assert_eq!(combined.top, 59.0);
    /// assert_eq!(combined.left, 8.0);
    /// ```
    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Returns the insets multiplied by `factor`.
    ///
    /// The paint pass works in scaled units while the platform reports logical
    /// ones, so a painter converts with the frame's scale factor.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_widget::SafeAreaInsets;
    ///
    /// assert_eq!(SafeAreaInsets::all(10.0).scaled(2.0).top, 20.0);
    /// ```
    #[inline]
    pub fn scaled(self, factor: f32) -> Self {
        if !factor.is_finite() || factor <= 0.0 {
            return self;
        }
        Self {
            left: self.left * factor,
            top: self.top * factor,
            right: self.right * factor,
            bottom: self.bottom * factor,
        }
    }
}

#[inline]
fn sanitize(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// Address of this static is the dependency identity of the safe area.
///
/// [`BuildConsumer::register_dependency`] keys on an address, so taking one
/// that no `Rc` can ever hand out keeps the insets from colliding with a
/// provider, with the window metrics or with the appearance.
static SAFE_AREA: u8 = 0;

#[inline]
fn identity() -> usize {
    &SAFE_AREA as *const u8 as usize
}

thread_local! {
    /// The insets the platform last reported for the application as a whole.
    ///
    /// Held per thread because it is only ever written by the platform layer on
    /// the thread that owns the event loop, and only ever read by the widget
    /// tree on that same thread. A window the platform has reported insets for
    /// on its own (see [`set_safe_area_insets_for`]) uses those instead; this
    /// is what every other window falls back to.
    static CURRENT: Cell<SafeAreaInsets> = const { Cell::new(SafeAreaInsets::ZERO) };

    /// The insets reported for one window alone: two windows of an application
    /// need not be reserved the same region, as on a tablet where one scene
    /// sits under the status bar and another does not.
    static PER_WINDOW: RefCell<HashMap<WindowKey, SafeAreaInsets>> = RefCell::new(HashMap::new());

    /// Every widget whose current build read the insets, with the window it
    /// was built for (`None` for a reader that follows every window).
    static SUBSCRIBERS: RefCell<HashMap<u64, (Option<WindowKey>, Weak<BuildConsumer>)>> =
        RefCell::new(HashMap::new());
    static NEXT_SUBSCRIBER: Cell<u64> = const { Cell::new(0) };
}

/// Reads the insets the platform last reported.
///
/// Reading through this function registers nothing: a widget that has to follow
/// later changes reads it with
/// [`BuildContext::watch_safe_area_insets`](crate::base::BuildContext::watch_safe_area_insets)
/// instead.
///
/// # Examples
///
/// ```
/// use aimer_widget::{SafeAreaInsets, safe_area_insets};
///
/// // A platform that reports nothing leaves the whole window usable.
/// let _insets: SafeAreaInsets = safe_area_insets();
/// ```
///
/// With several windows on the thread this answers for the active one (see
/// [`aimer_events::window::active_window`]); a widget that knows its window
/// should ask [`safe_area_insets_for`] instead.
#[inline]
pub fn safe_area_insets() -> SafeAreaInsets {
    match aimer_events::window::active_window() {
        Some(window) => insets_of(window),
        None => CURRENT.with(Cell::get),
    }
}

/// Reads the insets the platform last reported for `window`, or the
/// application's when it reported none for that window alone.
///
/// # Examples
///
/// ```
/// use aimer_widget::base::WindowHandle;
/// use aimer_widget::safe_area::{SafeAreaInsets, safe_area_insets_for, set_safe_area_insets_for};
///
/// let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(800, 600), 1.0);
/// let other = WindowHandle::headless(winit::dpi::PhysicalSize::new(800, 600), 1.0);
///
/// set_safe_area_insets_for(&window, SafeAreaInsets::new(0.0, 24.0, 0.0, 0.0));
///
/// assert_eq!(safe_area_insets_for(&window).top, 24.0);
/// assert_eq!(safe_area_insets_for(&other).top, 0.0);
/// # aimer_widget::safe_area::forget_safe_area_of(&window);
/// ```
#[inline]
pub fn safe_area_insets_for(window: &WindowHandle) -> SafeAreaInsets {
    insets_of(window.key())
}

fn insets_of(window: WindowKey) -> SafeAreaInsets {
    PER_WINDOW
        .with(|per_window| per_window.borrow().get(&window).copied())
        .unwrap_or_else(|| CURRENT.with(Cell::get))
}

/// Reports the region the system reserves in one window and rebuilds the
/// widgets of that window that follow it.
///
/// The counterpart of [`set_safe_area_insets`] for a platform that knows which
/// window an announcement is about. Widgets built for any other window are not
/// asked to rebuild: nothing changed for them. Returns how many widgets were
/// marked.
pub fn set_safe_area_insets_for(window: &WindowHandle, insets: SafeAreaInsets) -> usize {
    let key = window.key();
    if insets_of(key) == insets {
        return 0;
    }
    PER_WINDOW.with(|per_window| per_window.borrow_mut().insert(key, insets));
    notify(|subscribed| subscribed.is_none_or(|subscribed| subscribed == key))
}

/// Forgets what was reported for `window`, once it is gone.
///
/// A window that closes leaves nothing behind; one that stays gets the
/// application's insets again.
pub fn forget_safe_area_of(window: &WindowHandle) {
    let key = window.key();
    PER_WINDOW.with(|per_window| per_window.borrow_mut().remove(&key));
}

/// Reports the region the system reserves and rebuilds the widgets that follow
/// it.
///
/// Called by the platform layer when the window is created and whenever the
/// reservation changes — a rotation, a keyboard, a status bar that grows during
/// a call. Returns how many widgets were marked for rebuild, which tells the
/// caller whether the change is visible at all.
///
/// Reporting insets that are already in effect marks nothing: platforms
/// re-announce them on every layout pass, and a repaint per announcement would
/// be a repaint for nothing.
///
/// # Examples
///
/// ```
/// use aimer_widget::{SafeAreaInsets, safe_area_insets, set_safe_area_insets};
///
/// set_safe_area_insets(SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0));
/// assert_eq!(safe_area_insets().top, 59.0);
///
/// assert_eq!(set_safe_area_insets(safe_area_insets()), 0);
/// # set_safe_area_insets(SafeAreaInsets::ZERO);
/// ```
pub fn set_safe_area_insets(insets: SafeAreaInsets) -> usize {
    if CURRENT.with(Cell::get) == insets {
        return 0;
    }
    CURRENT.with(|current| current.set(insets));
    notify(|_| true)
}

/// Marks every live follower whose window `wanted` accepts, and drops the
/// followers that have left the tree.
fn notify(wanted: impl Fn(Option<WindowKey>) -> bool) -> usize {
    SUBSCRIBERS.with(|subscribers| {
        let mut subscribers = subscribers.borrow_mut();
        let mut notified = 0;
        subscribers.retain(|_, (window, consumer)| {
            let Some(consumer) = consumer.upgrade() else {
                return false;
            };
            if wanted(*window) {
                consumer.mark_needs_rebuild();
                notified += 1;
            }
            true
        });
        notified
    })
}

/// Registers the widget currently building as a follower of the safe area, to
/// be rebuilt whenever it changes.
///
/// Repeated reads within one build cost a hash lookup and nothing more.
pub(crate) fn subscribe(consumer: &Rc<BuildConsumer>) {
    subscribe_in(consumer, None);
}

/// Registers the widget currently building as a follower of the safe area of
/// `window` alone.
pub(crate) fn subscribe_for(consumer: &Rc<BuildConsumer>, window: &WindowHandle) {
    subscribe_in(consumer, Some(window.key()));
}

fn subscribe_in(consumer: &Rc<BuildConsumer>, window: Option<WindowKey>) {
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
            .insert(id, (window, Rc::downgrade(consumer)));
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

    /// The store is per thread and shared by every test on it.
    fn reset() {
        CURRENT.with(|current| current.set(SafeAreaInsets::ZERO));
        PER_WINDOW.with(|per_window| per_window.borrow_mut().clear());
    }

    #[test]
    fn nonsense_from_the_platform_reserves_nothing() {
        let insets = SafeAreaInsets::new(-10.0, f32::NAN, f32::INFINITY, 34.0);

        assert_eq!(insets.left, 0.0);
        assert_eq!(insets.top, 0.0);
        assert_eq!(insets.right, 0.0);
        assert_eq!(insets.bottom, 34.0);
    }

    #[test]
    fn a_comfort_margin_only_applies_where_the_system_asks_for_less() {
        let combined = SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0).max(SafeAreaInsets::all(8.0));

        assert_eq!(combined.top, 59.0);
        assert_eq!(combined.bottom, 34.0);
        assert_eq!(combined.left, 8.0);
        assert_eq!(combined.right, 8.0);
    }

    #[test]
    fn scaling_converts_logical_insets_into_painted_ones() {
        let scaled = SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0).scaled(3.0);

        assert_eq!(scaled.top, 177.0);
        assert_eq!(scaled.bottom, 102.0);
    }

    #[test]
    fn a_meaningless_scale_leaves_the_insets_alone() {
        let insets = SafeAreaInsets::all(8.0);

        assert_eq!(insets.scaled(0.0), insets);
        assert_eq!(insets.scaled(f32::NAN), insets);
    }

    #[test]
    fn a_widget_that_read_the_insets_is_rebuilt_when_they_change() {
        reset();
        let (consumer, dirty) = consumer();
        subscribe(&consumer);

        set_safe_area_insets(SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0));

        assert!(dirty.get(), "the follower was not marked for rebuild");
        assert_eq!(safe_area_insets().top, 59.0);
        reset();
    }

    #[test]
    fn a_widget_that_ignored_the_insets_is_left_alone() {
        reset();
        let (_consumer, dirty) = consumer();

        set_safe_area_insets(SafeAreaInsets::all(8.0));

        assert!(!dirty.get(), "an inset change rebuilt a widget that never asked");
        reset();
    }

    #[test]
    fn reporting_the_current_insets_again_rebuilds_nothing() {
        reset();
        let (consumer, dirty) = consumer();
        subscribe(&consumer);

        let notified = set_safe_area_insets(SafeAreaInsets::ZERO);

        assert_eq!(notified, 0);
        assert!(!dirty.get(), "a repeated announcement cost a rebuild");
    }

    #[test]
    fn reading_the_insets_twice_in_one_build_registers_once() {
        let (consumer, _dirty) = consumer();
        let before = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());

        subscribe(&consumer);
        subscribe(&consumer);

        let after = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        assert_eq!(after - before, 1);
    }

    fn window() -> WindowHandle {
        WindowHandle::headless(winit::dpi::PhysicalSize::new(800, 600), 1.0)
    }

    #[test]
    fn a_window_reported_on_its_own_keeps_the_others_as_they_were() {
        reset();
        let (first, second) = (window(), window());

        set_safe_area_insets_for(&first, SafeAreaInsets::new(0.0, 24.0, 0.0, 0.0));

        assert_eq!(safe_area_insets_for(&first).top, 24.0);
        assert_eq!(safe_area_insets_for(&second), SafeAreaInsets::ZERO);
        reset();
    }

    #[test]
    fn a_window_without_insets_of_its_own_follows_the_application() {
        reset();
        let window = window();

        set_safe_area_insets(SafeAreaInsets::all(6.0));

        assert_eq!(safe_area_insets_for(&window), SafeAreaInsets::all(6.0));
        reset();
    }

    #[test]
    fn only_the_followers_of_the_reported_window_are_rebuilt() {
        reset();
        let (first, second) = (window(), window());
        let (follower_a, dirty_a) = consumer();
        let (follower_b, dirty_b) = consumer();
        subscribe_for(&follower_a, &first);
        subscribe_for(&follower_b, &second);

        let notified = set_safe_area_insets_for(&first, SafeAreaInsets::all(10.0));

        assert_eq!(notified, 1);
        assert!(dirty_a.get(), "the follower of the reported window was missed");
        assert!(!dirty_b.get(), "another window's follower was rebuilt");
        reset();
    }

    #[test]
    fn reporting_a_windows_insets_again_rebuilds_nothing() {
        reset();
        let window = window();
        let (follower, dirty) = consumer();
        subscribe_for(&follower, &window);
        set_safe_area_insets_for(&window, SafeAreaInsets::all(10.0));
        dirty.set(false);

        let notified = set_safe_area_insets_for(&window, SafeAreaInsets::all(10.0));

        assert_eq!(notified, 0);
        assert!(!dirty.get());
        reset();
    }

    #[test]
    fn a_forgotten_window_returns_to_the_applications_insets() {
        reset();
        let window = window();
        set_safe_area_insets_for(&window, SafeAreaInsets::all(10.0));

        forget_safe_area_of(&window);

        assert_eq!(safe_area_insets_for(&window), SafeAreaInsets::ZERO);
        reset();
    }

    #[test]
    fn the_active_window_decides_what_the_unqualified_read_answers() {
        reset();
        let window = window();
        set_safe_area_insets_for(&window, SafeAreaInsets::all(10.0));
        let registration = aimer_events::window::register_window_scope(window.key(), || {}, || {});

        let inside = aimer_events::window::with_window_scope(window.key(), safe_area_insets);

        assert_eq!(inside, SafeAreaInsets::all(10.0));
        assert_eq!(safe_area_insets(), SafeAreaInsets::ZERO);
        drop(registration);
        reset();
    }

    #[test]
    fn a_widget_dropped_from_the_tree_is_forgotten() {
        reset();
        let (consumer, _dirty) = consumer();
        subscribe(&consumer);
        let registered = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        drop(consumer);

        set_safe_area_insets(SafeAreaInsets::all(4.0));

        let remaining = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        assert!(remaining < registered);
        reset();
    }
}
