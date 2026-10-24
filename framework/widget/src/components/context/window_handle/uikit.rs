use winit::dpi::{LogicalSize, PhysicalSize};

#[cfg(all(target_os = "ios", not(aimer_portable_guest)))]
pub(super) fn inner_size(window: &winit::window::Window) -> PhysicalSize<u32> {
    drawable_size(local_view_metrics(window), || window.outer_size())
}

#[cfg(all(target_os = "ios", not(aimer_portable_guest)))]
fn local_view_metrics(window: &winit::window::Window) -> Option<(LogicalSize<f64>, f64)> {
    use objc2::MainThreadMarker;
    use objc2_ui_kit::UIView;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // Winit's fallback handles dispatch for callers outside UIKit's thread.
    let _main_thread = MainThreadMarker::new()?;
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: Winit owns the UIView for the borrowed window's lifetime. The
    // marker above confines these synchronous UIKit reads to the main thread.
    let view = unsafe { &*handle.ui_view.as_ptr().cast::<UIView>() };
    let size = view.bounds().size;
    Some((LogicalSize::new(size.width, size.height), view.contentScaleFactor()))
}

fn drawable_size(
    local: Option<(LogicalSize<f64>, f64)>,
    projected_size: impl FnOnce() -> PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    if let Some((bounds, scale)) = local
        && bounds.width.is_finite() && bounds.width >= 0.0
        && bounds.height.is_finite() && bounds.height >= 0.0
        && scale.is_finite() && scale > 0.0
    {
        // The drawable lives in the view's local coordinate system. Screen
        // conversion includes UIKit's window transforms during live resize;
        // using that projected size makes the compositor scale the UI twice.
        return bounds.to_physical(scale);
    }
    projected_size()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_uses_local_bounds_instead_of_the_screen_projection() {
        for (width, height, scale) in [
            (1024.0, 768.0, 2.0),
            (512.0, 384.0, 2.0),
            (900.0, 700.0, 2.0),
            (700.0, 900.0, 3.0),
            (512.25, 384.75, 2.0),
        ] {
            // A window transform changes the screen-space rectangle without
            // changing the view's local coordinate system or content scale.
            let local = LogicalSize::new(width, height);
            let projected = LogicalSize::new(width * 0.8, height * 1.1).to_physical(scale);
            assert_eq!(drawable_size(Some((local, scale)), || projected), local.to_physical(scale));
        }
    }

    #[test]
    fn valid_local_metrics_do_not_query_screen_coordinates() {
        assert_eq!(
            drawable_size(Some((LogicalSize::new(800.0, 600.0), 2.0)), || {
                panic!("screen coordinates must not affect the drawable")
            }),
            PhysicalSize::new(1600, 1200),
        );
    }

    #[test]
    fn zero_local_bounds_do_not_fall_back_to_a_visible_window() {
        for size in [LogicalSize::new(0.0, 0.0), LogicalSize::new(0.0, 600.0), LogicalSize::new(800.0, 0.0)] {
            assert_eq!(
                drawable_size(Some((size, 2.0)), || PhysicalSize::new(1600, 1200)),
                size.to_physical(2.0),
            );
        }
    }

    #[test]
    fn unavailable_or_invalid_local_metrics_use_the_winit_fallback() {
        let fallback = PhysicalSize::new(1600, 1200);
        for local in [
            None,
            Some((LogicalSize::new(f64::NAN, 600.0), 2.0)),
            Some((LogicalSize::new(800.0, f64::INFINITY), 2.0)),
            Some((LogicalSize::new(-1.0, 600.0), 2.0)),
            Some((LogicalSize::new(800.0, -1.0), 2.0)),
            Some((LogicalSize::new(800.0, 600.0), 0.0)),
            Some((LogicalSize::new(800.0, 600.0), -1.0)),
            Some((LogicalSize::new(800.0, 600.0), f64::NAN)),
            Some((LogicalSize::new(800.0, 600.0), f64::INFINITY)),
        ] {
            assert_eq!(drawable_size(local, || fallback), fallback);
        }
    }
}
