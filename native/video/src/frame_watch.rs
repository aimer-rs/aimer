//! A zero-size element that runs a cheap check on every frame it is walked.
//!
//! The player UI uses it to notice, without rebuilding anything, when its
//! picture of the player is stale: a new caption cue, controls that should
//! hide, a playhead that moved. The check returns whether more frames are
//! needed, so a paused player stops asking for them.

use std::rc::Rc;

use aimer_attribute::size::ResolvedSize;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable,
    VisitorElement, Widget,
};

/// Runs `check` each frame; asks for the next frame while it returns `true`.
pub(crate) struct FrameWatch {
    check: Rc<dyn Fn() -> bool>,
}

impl FrameWatch {
    pub(crate) fn new(check: impl Fn() -> bool + 'static) -> Self {
        Self { check: Rc::new(check) }
    }
}

impl Widget for FrameWatch {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawFrameWatch { check: self.check }.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "FrameWatch"
    }
}

struct RawFrameWatch {
    check: Rc<dyn Fn() -> bool>,
}

impl Drawable for RawFrameWatch {
    fn update(&self, _ctx: &BuildContext) {
        if (self.check)() {
            aimer_events::window::request_animation_frame();
        }
    }

    // No pixels of its own, but `false` would report it as an element that declined
    // to paint; an empty paint is how a watcher says it has nothing to draw.
    #[inline]
    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }
}

impl EventElement for RawFrameWatch {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl VisitorElement for RawFrameWatch {
    fn debug_name(&self) -> &'static str {
        "FrameWatch"
    }
}

impl Rebuildable for RawFrameWatch {}

impl LayoutElement for RawFrameWatch {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize::default()
    }
}
