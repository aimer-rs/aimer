//! Delivers GPU outcomes without moving application callbacks off the UI thread.
use super::AimerApplicationHandler;
use crate::aimer_app::FrameRequestKind;
use aimer_widget::Widget;
use winit::event_loop::ActiveEventLoop;

impl<W: Widget + 'static> AimerApplicationHandler<W> {
    pub(crate) fn admit_render_frame(&mut self) -> bool {
        if let Some(size) = self.pending_resize.take() {
            self.render_ctx.resize(size);
        }
        if self.render_ctx.can_begin_frame() {
            return true;
        }
        self.render_ctx.defer_frame();
        false
    }

    pub(crate) fn request_deferred_render_frame(&self) {
        // Resuming deferred work is not a new invalidation. Preserve its
        // scoped/scroll reason through the window's redraw observer.
        let kind = self
            .frame_request_reason
            .get()
            .unwrap_or(FrameRequestKind::Full);
        crate::aimer_app::with_frame_request_kind(kind, || {
            #[cfg(target_os = "ios")]
            aimer_events::window::request_animation_frame();
            #[cfg(not(target_os = "ios"))]
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        });
    }

    pub(crate) fn deliver_render_completions(&mut self, event_loop: &ActiveEventLoop) {
        let completed = self.render_ctx.poll_completions();
        crate::first_frame::notify_first_frame_presented(completed.presented);
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        if completed.presented && self.show_window_after_first_frame {
            if let Some(window) = self.native_window() {
                window.set_visible(true);
            }
            self.show_window_after_first_frame = false;
        }
        if self.render_ctx.is_closing() {
            if self.render_ctx.is_stopped() {
                self.window_finished(event_loop);
            }
            return;
        }
        if completed.retry {
            crate::render_ctx::note_frame_not_presented();
            self.note_frame_request(FrameRequestKind::Full);
        }
        if completed.retry || completed.wake_frame {
            self.request_deferred_render_frame();
        }
    }
}
