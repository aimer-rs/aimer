//! Rebuilding the widgets that read the window metrics, and only those.
//!
//! A window drag delivers a resize event for every pixel the edge travels. The
//! frame that answers it has to lay the tree out again — the constraints
//! genuinely changed — but almost nothing has to be *built* again: a widget
//! whose `build` never looked at the window is described by exactly the same
//! configuration it was described by before the drag started.
//!
//! Widgets that do look — through `MediaQuery` or
//! [`BuildContext::watch_window_metrics`](crate::base::BuildContext::watch_window_metrics)
//! — register here while they build, exactly the way a widget watching a
//! provider registers with it. A resize then marks that handful of widgets dirty
//! instead of walking the whole tree flipping every dirty flag, which is the
//! difference between a drag that tracks the cursor and one that does not.
//!
//! Most widgets do not depend on the window itself but on one question about it
//! — "is this narrow enough to stack the columns?" — whose answer changes at a
//! single point in a whole drag.
//! [`BuildContext::select_window_metrics`](crate::base::BuildContext::select_window_metrics)
//! registers that question instead of the metrics, so the widget is rebuilt when
//! the answer changes and left alone for every other pixel.
//!
//! Registration lasts for one build. The next build of the same widget clears it
//! and re-registers whatever that build reads, so a widget that stops consulting
//! the window stops paying for it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::components::context::{BuildConsumer, WindowHandle};

/// The window metrics as a widget sees them.
///
/// # Examples
///
/// ```
/// use aimer_widget::WindowMetrics;
///
/// let metrics = WindowMetrics {
///     physical_size: winit::dpi::PhysicalSize::new(800, 600),
///     scale_factor: 2.0,
/// };
///
/// assert_eq!(metrics.logical_size().width, 400.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    /// Size of the window's client area, in physical pixels.
    pub physical_size: winit::dpi::PhysicalSize<u32>,
    /// Physical pixels per logical pixel.
    pub scale_factor: f64,
}

impl WindowMetrics {
    /// Reads the metrics a window currently reports.
    #[inline]
    pub fn of(window: &WindowHandle) -> Self {
        Self {
            physical_size: window.inner_size(),
            scale_factor: window.scale_factor(),
        }
    }

    /// The client area in logical pixels, which is what layout is expressed in.
    #[inline]
    pub fn logical_size(&self) -> aimer_attribute::size::ResolvedSize {
        let scale = self.scale_factor as f32;
        aimer_attribute::size::ResolvedSize {
            width: self.physical_size.width as f32 / scale,
            height: self.physical_size.height as f32 / scale,
        }
    }
}

/// Address of this static is the dependency identity of the window metrics.
///
/// [`BuildConsumer::register_dependency`] keys on an address, so taking one that
/// no `Rc` can ever hand out keeps the metrics from colliding with a provider.
static WINDOW_METRICS: u8 = 0;

#[inline]
fn identity() -> usize {
    &WINDOW_METRICS as *const u8 as usize
}

/// One registered reader of the window.
struct Subscriber {
    consumer: Weak<BuildConsumer>,
    /// The window the reader was built against, so the answer is recomputed
    /// from the same source it was first read from.
    window: WindowHandle,
    /// Whether the change is one this reader can see.
    should_notify: Box<dyn FnMut(&WindowMetrics) -> bool>,
}

aimer_tls::thread_local! {
    /// Every widget whose current build read the window.
    ///
    /// The consumer is held weakly, so a widget that has left the tree simply
    /// stops answering rather than having to be unregistered from wherever it
    /// died.
    static SUBSCRIBERS: RefCell<HashMap<u64, Subscriber>> = RefCell::new(HashMap::new());
    static NEXT_SUBSCRIBER: Cell<u64> = const { Cell::new(0) };
}

/// Registers `subscriber` and arranges for the next build of the same widget to
/// retire it.
fn register(consumer: &Rc<BuildConsumer>, subscriber: Subscriber) {
    let id = NEXT_SUBSCRIBER.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    SUBSCRIBERS.with(|subscribers| {
        subscribers.borrow_mut().insert(id, subscriber);
    });
    consumer.add_cleanup(move || {
        SUBSCRIBERS.with(|subscribers| {
            subscribers.borrow_mut().remove(&id);
        });
    });
}

/// Registers the widget currently building as a reader of the whole window
/// metrics, to be rebuilt whenever any of them change.
///
/// Repeated reads within one build cost a hash lookup and nothing more.
pub(crate) fn subscribe(consumer: &Rc<BuildConsumer>, window: &WindowHandle) {
    if !consumer.register_dependency(identity()) {
        return;
    }
    register(
        consumer,
        Subscriber {
            consumer: Rc::downgrade(consumer),
            window: window.clone(),
            should_notify: Box::new(|_| true),
        },
    );
}

/// Registers the widget currently building as depending on `selector` alone, and
/// returns its current answer.
///
/// Each call registers separately, because two questions about the same window
/// are two different dependencies.
pub(crate) fn subscribe_selected<T: Clone + PartialEq + 'static>(
    consumer: &Rc<BuildConsumer>,
    window: &WindowHandle,
    selector: impl Fn(&WindowMetrics) -> T + 'static,
) -> T {
    let answer = selector(&WindowMetrics::of(window));
    let mut selected = answer.clone();
    register(
        consumer,
        Subscriber {
            consumer: Rc::downgrade(consumer),
            window: window.clone(),
            should_notify: Box::new(move |metrics| {
                let next = selector(metrics);
                if next == selected {
                    false
                } else {
                    selected = next;
                    true
                }
            }),
        },
    );
    answer
}

/// Marks every widget that can see the new window metrics as needing a rebuild.
///
/// Called by the platform layer when the window is resized or moved to a display
/// with a different scale factor. Returns how many widgets were marked, which
/// tells the caller whether the frame it is about to draw differs from a pure
/// re-layout.
///
/// Widgets that have since left the tree are dropped here rather than rebuilt.
pub fn notify_window_metrics_changed() -> usize {
    notify_matching(|_| true)
}

/// Like [`notify_window_metrics_changed`], for a change to `window` alone.
///
/// Only the widgets that read `window` are asked; widgets built for any other
/// window on the thread are left exactly as they were, because nothing about
/// them changed. This is what the platform layer calls when it knows which
/// window was resized or moved to another display.
pub fn notify_window_metrics_changed_for(window: &WindowHandle) -> usize {
    let key = window.key();
    notify_matching(|subscriber| subscriber.window.key() == key)
}

fn notify_matching(mut matches: impl FnMut(&Subscriber) -> bool) -> usize {
    SUBSCRIBERS.with(|subscribers| {
        let mut subscribers = subscribers.borrow_mut();
        let mut notified = 0;
        subscribers.retain(|_, subscriber| {
            let Some(consumer) = subscriber.consumer.upgrade() else {
                return false;
            };
            if !matches(subscriber) {
                return true;
            }
            let metrics = WindowMetrics::of(&subscriber.window);
            if (subscriber.should_notify)(&metrics) {
                consumer.mark_needs_rebuild();
                notified += 1;
            }
            true
        });
        notified
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use winit::dpi::PhysicalSize;

    use super::*;

    fn window(width: u32) -> WindowHandle {
        WindowHandle::headless(PhysicalSize::new(width, 800), 1.0)
    }

    fn resize(window: &WindowHandle, width: u32) {
        window.update_headless_metrics(PhysicalSize::new(width, 800), 1.0);
    }

    fn consumer() -> (Rc<BuildConsumer>, Rc<Cell<bool>>) {
        let dirty = Rc::new(Cell::new(false));
        (BuildConsumer::new(dirty.clone()), dirty)
    }

    #[test]
    fn a_widget_that_read_the_metrics_is_rebuilt_by_a_resize() {
        let (consumer, dirty) = consumer();
        let window = window(1000);
        subscribe(&consumer, &window);

        resize(&window, 1001);
        notify_window_metrics_changed();

        assert!(dirty.get(), "the reader was not marked for rebuild");
    }

    #[test]
    fn a_widget_that_ignored_the_metrics_is_left_alone() {
        let (_consumer, dirty) = consumer();

        notify_window_metrics_changed();

        assert!(!dirty.get(), "a resize rebuilt a widget that never asked");
    }

    #[test]
    fn reading_the_metrics_twice_in_one_build_registers_once() {
        let (consumer, _dirty) = consumer();
        let window = window(1000);
        let before = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());

        subscribe(&consumer, &window);
        subscribe(&consumer, &window);

        let after = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        assert_eq!(after - before, 1);
    }

    #[test]
    fn a_widget_that_stops_reading_the_metrics_stops_being_rebuilt() {
        let (consumer, dirty) = consumer();
        let window = window(1000);
        subscribe(&consumer, &window);

        // What a rebuild does before running `build` again: retire everything
        // the previous build depended on. This one does not read the window.
        consumer.begin_build();
        dirty.set(false);

        resize(&window, 1001);
        notify_window_metrics_changed();

        assert!(!dirty.get(), "the stale registration outlived the build");
    }

    #[test]
    fn a_widget_dropped_from_the_tree_is_forgotten() {
        let (consumer, _dirty) = consumer();
        let window = window(1000);
        subscribe(&consumer, &window);
        let registered = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        drop(consumer);

        notify_window_metrics_changed();

        let remaining = SUBSCRIBERS.with(|subscribers| subscribers.borrow().len());
        assert!(remaining < registered);
    }

    #[test]
    fn a_selected_answer_survives_a_resize_that_cannot_change_it() {
        let (consumer, dirty) = consumer();
        let window = window(1000);

        let compact = subscribe_selected(&consumer, &window, |metrics| {
            metrics.logical_size().width < 600.0
        });
        assert!(!compact);

        resize(&window, 999);
        notify_window_metrics_changed();

        assert!(
            !dirty.get(),
            "a pixel of drag rebuilt a widget whose answer did not move"
        );
    }

    #[test]
    fn a_selected_answer_rebuilds_the_widget_when_it_changes() {
        let (consumer, dirty) = consumer();
        let window = window(1000);
        subscribe_selected(&consumer, &window, |metrics| {
            metrics.logical_size().width < 600.0
        });

        resize(&window, 500);
        notify_window_metrics_changed();

        assert!(dirty.get(), "the breakpoint was crossed unnoticed");
    }

    #[test]
    fn a_selected_answer_reports_every_later_change() {
        let (consumer, dirty) = consumer();
        let window = window(1000);
        subscribe_selected(&consumer, &window, |metrics| {
            metrics.logical_size().width < 600.0
        });

        resize(&window, 500);
        notify_window_metrics_changed();
        dirty.set(false);

        // Back across the breakpoint: the subscriber has to remember the answer
        // it last reported, not the one it was created with.
        resize(&window, 1000);
        notify_window_metrics_changed();

        assert!(dirty.get(), "the reader stopped following the window");
    }
}
