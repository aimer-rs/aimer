//! Debug-only counters that connect input and retained-tree work to a frame.
//!
//! The counters live at the widget boundary so framework-owned modules can
//! report one vocabulary for layout, hit testing, paint, state, scroll, and
//! redraw work. They are thread-local because the retained tree is owned by
//! the UI/render thread and the frame drawer takes the counters on that same
//! thread.

#[cfg(feature = "frame-stats")]
use std::cell::Cell;

/// Work observed between two frame-drawer samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWorkStats {
    /// Calls through the retained element layout boundary.
    pub layout_calls: u64,
    /// Retained elements reached while routing pointer events.
    pub hit_test_visits: u64,
    /// Calls through the retained element paint boundary.
    pub paint_calls: u64,
    /// Calls to the application root's draw method.
    pub root_draw_calls: u64,
    /// Platform scroll input events accepted by the active application root.
    pub scroll_events: u64,
    /// Smoothed scroll portions delivered to widgets.
    pub scroll_steps: u64,
    /// Smoothing ticks that had pending scroll work to advance.
    pub smoothing_steps: u64,
    /// State mutation requests queued by StateUpdater::set_state.
    pub state_updates: u64,
    /// New coalesced element invalidation records queued during the interval.
    pub invalidations_queued: u64,
    /// Additional invalidations merged into an already queued element record.
    pub invalidations_coalesced: u64,
    /// Direct ElementId-to-element path lookups performed for invalidations.
    pub element_index_lookups: u64,
    /// Queued ElementIds that did not resolve in the current tree generation.
    pub stale_element_ids: u64,
    /// Scroll-state offset changes that altered the stored position.
    pub scroll_offset_updates: u64,
    /// Direct redraw requests sent through a WindowHandle.
    pub redraw_requests: u64,
}

#[cfg(feature = "frame-stats")]
thread_local! {
    static STATS: Cell<FrameWorkStats> = const { Cell::new(FrameWorkStats {
        layout_calls: 0,
        hit_test_visits: 0,
        paint_calls: 0,
        root_draw_calls: 0,
        scroll_events: 0,
        scroll_steps: 0,
        smoothing_steps: 0,
        state_updates: 0,
        invalidations_queued: 0,
        invalidations_coalesced: 0,
        element_index_lookups: 0,
        stale_element_ids: 0,
        scroll_offset_updates: 0,
        redraw_requests: 0,
    }) };
}

#[cfg(feature = "frame-stats")]
#[inline]
fn update(update: impl FnOnce(&mut FrameWorkStats)) {
    STATS.with(|stats| {
        let mut current = stats.get();
        update(&mut current);
        stats.set(current);
    });
}

macro_rules! record_counter {
    ($name:ident, $field:ident) => {
        #[doc(hidden)]
        #[inline]
        pub fn $name() {
            #[cfg(feature = "frame-stats")]
            update(|stats| stats.$field = stats.$field.saturating_add(1));
        }
    };
}

record_counter!(record_layout_call, layout_calls);
record_counter!(record_hit_test_visit, hit_test_visits);
record_counter!(record_paint_call, paint_calls);
record_counter!(record_root_draw_call, root_draw_calls);
record_counter!(record_scroll_event, scroll_events);
record_counter!(record_scroll_step, scroll_steps);
record_counter!(record_smoothing_step, smoothing_steps);
record_counter!(record_state_update, state_updates);
record_counter!(record_invalidation_queued, invalidations_queued);
record_counter!(record_invalidation_coalesced, invalidations_coalesced);
record_counter!(record_element_index_lookup, element_index_lookups);
record_counter!(record_stale_element_id, stale_element_ids);
record_counter!(record_scroll_offset_update, scroll_offset_updates);
record_counter!(record_redraw_request, redraw_requests);

/// Clears the counters for the next frame interval.
#[doc(hidden)]
#[inline]
pub fn reset_frame_work_stats() {
    #[cfg(feature = "frame-stats")]
    STATS.with(|stats| stats.set(FrameWorkStats::default()));
}

/// Takes and clears the counters for the completed frame interval.
#[doc(hidden)]
#[inline]
pub fn take_frame_work_stats() -> FrameWorkStats {
    #[cfg(feature = "frame-stats")]
    return STATS.with(|stats| stats.replace(FrameWorkStats::default()));

    #[cfg(not(feature = "frame-stats"))]
    FrameWorkStats::default()
}

#[cfg(all(test, feature = "frame-stats"))]
mod tests {
    use super::*;

    #[test]
    fn frame_work_stats_accumulate_and_take() {
        reset_frame_work_stats();
        record_layout_call();
        record_hit_test_visit();
        record_paint_call();
        record_root_draw_call();
        record_scroll_event();
        record_scroll_step();
        record_smoothing_step();
        record_state_update();
        record_scroll_offset_update();
        record_redraw_request();

        assert_eq!(
            take_frame_work_stats(),
            FrameWorkStats {
                layout_calls: 1,
                hit_test_visits: 1,
                paint_calls: 1,
                root_draw_calls: 1,
                scroll_events: 1,
                scroll_steps: 1,
                smoothing_steps: 1,
                state_updates: 1,
                scroll_offset_updates: 1,
                redraw_requests: 1,
                ..FrameWorkStats::default()
            }
        );
        assert_eq!(take_frame_work_stats(), FrameWorkStats::default());
    }
}
