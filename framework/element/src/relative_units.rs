//! CSS context-relative length units, taken from the build in progress.
//!
//! [`Dimension`] has no variant for `em`, `rem`, `vw`, `vh`, `vmin` or `vmax`,
//! because none of them is a size until something says what a font size or the
//! viewport is. [`RelativeUnitExt`] says it by asking the build running on this
//! thread ([`BuildContext::current`]), and returns the ordinary
//! [`Dimension::Px`] that results. The pure arithmetic lives in
//! [`UnitContext`], which can also be built by hand.

use aimer_attribute::{Dimension, UnitContext};

use crate::components::context::BuildContext;

/// Builds a [`Dimension`] from a number in a unit measured against the build
/// that is running.
///
/// Every method reads the window of the current build through
/// [`BuildContext::watch_window_metrics`], so a widget that uses `50.vw()` is
/// rebuilt when the window is resized and its width follows. The result is a
/// plain [`Dimension::Px`]; layout never sees the unit.
///
/// `em` and `rem` use [`UnitContext::DEFAULT_FONT_SIZE`]: a build has no
/// inherited font size to read yet, so they are a fixed 16 pixels per unit.
///
/// # Panics
///
/// Every method panics outside a widget `build`, since there is no viewport to
/// measure against. Use [`UnitContext`] directly where a build may not be
/// running.
///
/// # Examples
///
/// ```ignore
/// use aimer_element::RelativeUnitExt;
///
/// fn build(&self, _ctx: &BuildContext) -> impl Widget {
///     Container::new().width(50.vw()).height(1.5.em()).child(content)
/// }
/// ```
pub trait RelativeUnitExt: Sized {
    /// This many font sizes.
    fn em(self) -> Dimension;

    /// This many root font sizes.
    fn rem(self) -> Dimension;

    /// This percent of the viewport width.
    fn vw(self) -> Dimension;

    /// This percent of the viewport height.
    fn vh(self) -> Dimension;

    /// This percent of the smaller viewport extent.
    fn vmin(self) -> Dimension;

    /// This percent of the larger viewport extent.
    fn vmax(self) -> Dimension;
}

/// Snapshots the units of the build running on this thread, subscribing it to
/// window changes.
#[track_caller]
fn current_units() -> UnitContext {
    let active = BuildContext::current();
    let size = active.get().watch_window_metrics().logical_size();
    UnitContext::new(size.width, size.height)
}

macro_rules! impl_relative_unit_ext {
    ($($number:ty),* $(,)?) => {$(
        impl RelativeUnitExt for $number {
            #[inline]
            #[track_caller]
            fn em(self) -> Dimension {
                current_units().em(self as f32)
            }

            #[inline]
            #[track_caller]
            fn rem(self) -> Dimension {
                current_units().rem(self as f32)
            }

            #[inline]
            #[track_caller]
            fn vw(self) -> Dimension {
                current_units().vw(self as f32)
            }

            #[inline]
            #[track_caller]
            fn vh(self) -> Dimension {
                current_units().vh(self as f32)
            }

            #[inline]
            #[track_caller]
            fn vmin(self) -> Dimension {
                current_units().vmin(self as f32)
            }

            #[inline]
            #[track_caller]
            fn vmax(self) -> Dimension {
                current_units().vmax(self as f32)
            }
        }
    )*};
}

impl_relative_unit_ext!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::components::context::{BuildConsumer, WindowHandle};

    fn runtime() -> tokio::runtime::Handle {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime always builds")
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    /// A context whose window is `width` x `height` logical pixels at 2x.
    fn context(width: u32, height: u32) -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            Default::default(),
            2.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(width * 2, height * 2), 2.0),
            runtime(),
        )
    }

    fn build<R>(context: &BuildContext<'static>, consumer: &Rc<BuildConsumer>, f: impl FnOnce() -> R) -> R {
        context.with_build_consumer(consumer.clone(), |_| f())
    }

    #[test]
    fn viewport_units_measure_the_logical_window_of_the_build() {
        let context = context(800, 600);
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        build(&context, &consumer, || {
            assert_eq!(50.vw(), Dimension::Px(400.0));
            assert_eq!(10.vh(), Dimension::Px(60.0));
            assert_eq!(10.vmin(), Dimension::Px(60.0));
            assert_eq!(10.vmax(), Dimension::Px(80.0));
            assert_eq!(1.5f32.vw(), Dimension::Px(12.0));
        });
    }

    #[test]
    fn font_relative_units_use_the_default_font_size() {
        let context = context(800, 600);
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        build(&context, &consumer, || {
            assert_eq!(2.em(), Dimension::Px(32.0));
            assert_eq!(0.5f32.rem(), Dimension::Px(8.0));
        });
    }

    #[test]
    fn a_widget_using_viewport_units_is_rebuilt_when_the_window_resizes() {
        let context = context(800, 600);
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());

        let before = build(&context, &consumer, || 50.vw());
        assert_eq!(before, Dimension::Px(400.0));
        assert!(!dirty.get());

        context
            .window
            .update_headless_metrics(winit::dpi::PhysicalSize::new(1000, 1200), 2.0);
        crate::notify_window_metrics_changed();

        assert!(dirty.get(), "the widget read the viewport, so a resize must dirty it");
        dirty.set(false);
        let after = build(&context, &consumer, || 50.vw());
        assert_eq!(after, Dimension::Px(250.0));
    }

    #[test]
    fn a_widget_that_never_reads_the_viewport_is_left_alone() {
        let context = context(800, 600);
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());

        build(&context, &consumer, || {});
        crate::notify_window_metrics_changed();

        assert!(!dirty.get());
    }

    #[test]
    #[should_panic(expected = "no widget build is running")]
    fn units_outside_a_build_panic_with_the_reason() {
        let _ = 50.vw();
    }
}
