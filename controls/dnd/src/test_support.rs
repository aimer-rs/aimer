//! Headless scaffolding shared by the drag tests.
//!
//! An element only knows where it is once it has been laid out, and laying out
//! needs a [`BuildContext`] — which in turn needs a canvas, a window handle and
//! (off the web) an async runtime handle. Building one by hand in every test
//! module is noisy, so the headless construction lives here.

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_widget::base::{BuildContext, WindowHandle};

#[cfg(not(target_arch = "wasm32"))]
fn dummy_async_handle() -> tokio::runtime::Handle {
    use std::sync::OnceLock;

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime is available in tests")
    });
    let _guard = runtime.enter();
    tokio::runtime::Handle::current()
}

/// Builds a headless [`BuildContext`] constrained to `width` x `height`.
pub(crate) fn headless_context(width: f32, height: f32) -> BuildContext<'static> {
    let canvas = {
        let leaked: &'static InnerCanvas = Box::leak(Box::new(InnerCanvas::new()));
        FrameCanvas::new(leaked)
    };

    let mut context = BuildContext::new(
        canvas,
        ResolvedSize { width, height },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(
            winit::dpi::PhysicalSize::new(width.max(1.0) as u32, height.max(1.0) as u32),
            1.0,
        ),
        #[cfg(not(target_arch = "wasm32"))]
        dummy_async_handle(),
    );
    context.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: width,
        max_height: height,
    };
    context
}
