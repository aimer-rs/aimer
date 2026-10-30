//! Delivers GPU outcomes without moving application callbacks off the UI thread.
use std::time::Instant;

use super::AimerApplicationHandler;
use crate::aimer_app::FrameRequestKind;
use aimer_widget::Widget;
use aimer_widget::base::WindowHandle;
use winit::event_loop::ActiveEventLoop;

impl<W: Widget + 'static> AimerApplicationHandler<W> {
    /// Returns `false` when this frame would start sooner after the previous one
    /// than the display can show it. The deadline is picked up by the host's
    /// `about_to_wait`, so only a hosted window is paced: an application that
    /// owns its loop has nothing that would wake it again.
    pub(crate) fn pace_render_frame(&mut self, bypass: bool) -> bool {
        if !self.hosted {
            return true;
        }
        let now = Instant::now();
        let window = &self.window;
        let period = self.frame_pacer.period(now, || {
            window
                .as_ref()
                .and_then(WindowHandle::native_window)
                .and_then(|window| window.current_monitor().or_else(|| window.primary_monitor()))
                .and_then(|monitor| monitor.refresh_rate_millihertz())
        });
        self.frame_pacer.admit(now, period, bypass)
    }

    /// When a frame held back by [`Self::pace_render_frame`] may be tried again.
    #[inline]
    pub(crate) fn pacing_deadline(&self) -> Option<Instant> {
        self.frame_pacer.deadline()
    }

    /// Resumes a held-back frame once its deadline has been reached.
    pub(crate) fn wake_paced_frame(&mut self, now: Instant) {
        if self.frame_pacer.take_due(now) {
            self.request_deferred_render_frame();
        }
    }

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
