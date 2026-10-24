use aimer_attribute::ResolvedSize;
use aimer_widget::Widget;
use winit::event::WindowEvent;

use super::AimerApplicationHandler;
use super::event_handler::WindowEventHandler;

impl WindowEventHandler {
    pub(crate) fn defer_uikit_frame_event<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
        event: &WindowEvent,
    ) -> bool {
        match event {
            WindowEvent::RedrawRequested => {
                // drawRect can run during a UIKit layout/commit. Keep it short
                // and draw only from FrameReady, just like a resize request.
                app.request_animation_frame();
                true
            }
            WindowEvent::Resized(size) => {
                // winit projects resize bounds into screen coordinates. The
                // drawable uses the local UIView bounds, even during a drag.
                let size = app
                    .window
                    .as_ref()
                    .map_or(*size, |window| window.inner_size());
                Self::apply_resize(size, app);
                app.request_animation_frame();
                true
            }
            _ => false,
        }
    }

    pub(crate) fn refresh_uikit_viewport<W: Widget + 'static>(
        app: &mut AimerApplicationHandler<W>,
    ) {
        let Some(window) = app.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        let scale = window.scale_factor();
        let physical = ResolvedSize {
            width: size.width as f32,
            height: size.height as f32,
        };
        if app.native_window_size != Some(physical) || app.window_scale != scale {
            app.window_scale = scale;
            Self::apply_resize(size, app);
        }
    }
}
