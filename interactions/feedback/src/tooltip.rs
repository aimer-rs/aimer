use std::time::Duration;

use aimer_text::Text;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, PortableWidget, Widget};

use super::clock::Clock;
use super::overlay::{
    Announcement, Announcer, DismissReason, OverlayHost, OverlayId, OverlayKind, OverlayRequest,
    PlacementSpec, Rect,
};

/// Policy controlling whether a tooltip may be opened by touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPolicy {
    /// Never open the tooltip from a touch gesture.
    Never,
    /// Open on touch-down without waiting for the hover delay.
    Immediate,
    /// Open after the supplied touch-and-hold duration.
    LongPress(Duration),
}

/// A touch lifecycle event supplied by the input adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipTouch {
    /// A touch began on the tooltip trigger.
    Started,
    /// A touch ended on or near the trigger.
    Ended,
    /// A touch was cancelled by the input system.
    Cancelled,
}

/// The caller-owned configuration for one tooltip trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct Tooltip {
    text: String,
    delay: Duration,
    placement: PlacementSpec,
    touch_policy: TouchPolicy,
    show_on_hover: bool,
    show_on_focus: bool,
}

impl Tooltip {
    /// Creates a tooltip that appears after a 500ms hover or focus delay.
    #[inline]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            delay: Duration::from_millis(500),
            placement: PlacementSpec::default(),
            touch_policy: TouchPolicy::Never,
            show_on_hover: true,
            show_on_focus: true,
        }
    }

    /// Sets the hover/focus delay.
    #[inline]
    pub const fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Sets anchored placement preferences.
    #[inline]
    pub const fn placement(mut self, placement: PlacementSpec) -> Self {
        self.placement = placement;
        self
    }

    /// Sets the touch-opening policy.
    #[inline]
    pub const fn touch_policy(mut self, policy: TouchPolicy) -> Self {
        self.touch_policy = policy;
        self
    }

    /// Enables or disables opening while the trigger is hovered.
    #[inline]
    pub const fn show_on_hover(mut self, show: bool) -> Self {
        self.show_on_hover = show;
        self
    }

    /// Enables or disables opening while the trigger owns keyboard focus.
    #[inline]
    pub const fn show_on_focus(mut self, show: bool) -> Self {
        self.show_on_focus = show;
        self
    }

    /// Returns the display text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the hover/focus delay.
    #[inline]
    pub const fn delay_value(&self) -> Duration {
        self.delay
    }

    /// Returns the placement preferences.
    #[inline]
    pub const fn placement_value(&self) -> PlacementSpec {
        self.placement
    }

    /// Returns the touch policy.
    #[inline]
    pub const fn touch_policy_value(&self) -> TouchPolicy {
        self.touch_policy
    }

    /// Returns whether hover opens the tooltip.
    #[inline]
    pub const fn show_on_hover_value(&self) -> bool {
        self.show_on_hover
    }

    /// Returns whether focus opens the tooltip.
    #[inline]
    pub const fn show_on_focus_value(&self) -> bool {
        self.show_on_focus
    }

    /// Builds the host request for this tooltip and optional trigger anchor.
    #[inline]
    pub fn overlay_request(&self, anchor: Option<Rect>) -> OverlayRequest {
        let mut request = OverlayRequest::new(OverlayKind::Tooltip, self.text.clone())
            .placement(self.placement)
            .non_modal()
            .z_index(200)
            .dismiss_on_escape(true)
            .dismiss_on_outside_press(true);
        if let Some(anchor) = anchor {
            request = request.anchor(anchor);
        }
        request
    }

    /// Creates the polite accessibility announcement for this tooltip.
    #[inline]
    pub fn announcement(&self) -> Announcement {
        Announcement::new(self.text.clone())
    }
}

impl Widget for Tooltip {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // A tooltip's popup presentation is host-owned. Rendering its text as
        // an inline widget is the portable fallback for trees without a host.
        Text::new(self.text).to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "Tooltip"
    }
}

impl PortableWidget for Tooltip {}

/// The observable transition produced by [`TooltipController::pump`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipEvent {
    /// No lifecycle change occurred.
    Idle,
    /// A trigger is active but its delay has not elapsed.
    Pending,
    /// The tooltip was presented by the overlay host.
    Presented(OverlayId),
    /// The tooltip was dismissed by the overlay host.
    Dismissed {
        /// The host-owned overlay handle.
        id: OverlayId,
        /// Why the trigger stopped showing the tooltip.
        reason: DismissReason,
    },
}

/// Clock-driven tooltip lifecycle for one hover/focus/touch trigger.
pub struct TooltipController<C> {
    tooltip: Tooltip,
    clock: C,
    anchor: Option<Rect>,
    hovered: bool,
    focused: bool,
    touch_active: bool,
    touch_ready: bool,
    pending_since: Option<Duration>,
    active: Option<OverlayId>,
    needs_update: bool,
    end_reason: Option<DismissReason>,
}

impl<C: Clock> TooltipController<C> {
    /// Creates a controller using the supplied deterministic or production clock.
    #[inline]
    pub fn new(tooltip: Tooltip, clock: C) -> Self {
        Self {
            tooltip,
            clock,
            anchor: None,
            hovered: false,
            focused: false,
            touch_active: false,
            touch_ready: false,
            pending_since: None,
            active: None,
            needs_update: false,
            end_reason: None,
        }
    }

    /// Sets the trigger rectangle used by the overlay host for placement.
    #[inline]
    pub fn set_anchor(&mut self, anchor: Rect) {
        if self.anchor != Some(anchor) {
            self.anchor = Some(anchor);
            self.needs_update |= self.active.is_some();
        }
    }

    /// Marks the trigger as hovered or not hovered.
    #[inline]
    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
        if hovered {
            self.end_reason = None;
        } else if !self.focused && !self.touch_active {
            self.end_reason = Some(DismissReason::TriggerExit);
        }
    }

    /// Marks the trigger as focused or not focused.
    #[inline]
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if focused {
            self.end_reason = None;
        } else if !self.hovered && !self.touch_active {
            self.end_reason = Some(DismissReason::TriggerExit);
        }
    }

    /// Supplies a touch lifecycle event from the input adapter.
    #[inline]
    pub fn touch(&mut self, event: TooltipTouch) {
        match event {
            TooltipTouch::Started => {
                self.touch_active = !matches!(self.tooltip.touch_policy, TouchPolicy::Never);
                self.touch_ready = matches!(self.tooltip.touch_policy, TouchPolicy::Immediate);
                if self.touch_active {
                    self.end_reason = None;
                }
            }
            TooltipTouch::Ended | TooltipTouch::Cancelled => {
                self.touch_active = false;
                self.touch_ready = false;
                if !self.hovered && !self.focused {
                    self.end_reason = Some(if matches!(event, TooltipTouch::Ended) {
                        DismissReason::TouchEnd
                    } else {
                        DismissReason::TriggerExit
                    });
                }
            }
        }
    }

    /// Returns whether an overlay is currently visible.
    #[inline]
    pub const fn is_visible(&self) -> bool {
        self.active.is_some()
    }

    /// Returns whether the tooltip is waiting for its opening delay.
    #[inline]
    pub const fn is_pending(&self) -> bool {
        self.pending_since.is_some()
    }

    /// Returns the host-owned visible overlay handle.
    #[inline]
    pub const fn active_overlay(&self) -> Option<OverlayId> {
        self.active
    }

    /// Presents or dismisses the tooltip through the supplied overlay host.
    pub fn pump<H: OverlayHost>(&mut self, host: &mut H) -> TooltipEvent {
        self.pump_inner(host, None)
    }

    /// Presents or dismisses the tooltip and announces it when shown.
    pub fn pump_with_announcer<H: OverlayHost>(
        &mut self,
        host: &mut H,
        announcer: Option<&mut dyn Announcer>,
    ) -> TooltipEvent {
        self.pump_inner(host, announcer)
    }

    fn pump_inner<H: OverlayHost>(
        &mut self,
        host: &mut H,
        mut announcer: Option<&mut dyn Announcer>,
    ) -> TooltipEvent {
        let now = self.clock.now();
        let should_show = self.should_show();

        if let Some(id) = self.active
            && !should_show
        {
            let reason = self.end_reason.unwrap_or(DismissReason::TriggerExit);
            if !host.dismiss(id, reason) {
                return TooltipEvent::Idle;
            }
            self.active = None;
            self.pending_since = None;
            self.needs_update = false;
            self.end_reason = None;
            return TooltipEvent::Dismissed { id, reason };
        }

        if !should_show {
            self.pending_since = None;
            return TooltipEvent::Idle;
        }

        if let Some(id) = self.active
            && self.needs_update
        {
            let request = self.tooltip.overlay_request(self.anchor);
            if host.update(id, request.clone()) {
                self.needs_update = false;
                return TooltipEvent::Idle;
            }
            if !host.dismiss(id, DismissReason::Replaced) {
                return TooltipEvent::Idle;
            }
            let replacement = host.present(request);
            self.active = Some(replacement);
            self.needs_update = false;
            return TooltipEvent::Presented(replacement);
        }

        let pending_since = *self.pending_since.get_or_insert(now);
        let elapsed = now.saturating_sub(pending_since);
        if elapsed < self.required_delay() {
            return TooltipEvent::Pending;
        }

        if self.active.is_none() {
            let id = host.present(self.tooltip.overlay_request(self.anchor));
            if let Some(announcer) = announcer.as_deref_mut() {
                announcer.announce(self.tooltip.announcement());
            }
            self.active = Some(id);
            self.pending_since = None;
            self.needs_update = false;
            return TooltipEvent::Presented(id);
        }

        TooltipEvent::Idle
    }

    fn should_show(&self) -> bool {
        (self.hovered && self.tooltip.show_on_hover)
            || (self.focused && self.tooltip.show_on_focus)
            || self.touch_active
    }

    fn required_delay(&self) -> Duration {
        if self.touch_active && self.touch_ready {
            Duration::ZERO
        } else if self.touch_active {
            match self.tooltip.touch_policy {
                TouchPolicy::LongPress(duration) => duration,
                TouchPolicy::Never | TouchPolicy::Immediate => self.tooltip.delay,
            }
        } else {
            self.tooltip.delay
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_press_touch_policy_does_not_use_the_hover_delay() {
        let clock = super::super::ManualClock::new();
        let mut controller = TooltipController::new(
            Tooltip::new("Hold")
                .delay(Duration::from_secs(10))
                .touch_policy(TouchPolicy::LongPress(Duration::from_millis(400))),
            clock.clone(),
        );
        let mut host = TestHost::default();
        controller.touch(TooltipTouch::Started);

        assert!(matches!(controller.pump(&mut host), TooltipEvent::Pending));
        clock.advance(Duration::from_millis(400));
        assert!(matches!(
            controller.pump(&mut host),
            TooltipEvent::Presented(_)
        ));
    }

    #[derive(Default)]
    struct TestHost {
        next_id: u64,
    }

    impl OverlayHost for TestHost {
        fn present(&mut self, _request: OverlayRequest) -> OverlayId {
            self.next_id += 1;
            OverlayId::new(self.next_id)
        }

        fn dismiss(&mut self, _id: OverlayId, _reason: DismissReason) -> bool {
            true
        }
    }
}
