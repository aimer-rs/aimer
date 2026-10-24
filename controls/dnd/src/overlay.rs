//! The feedback that follows the pointer, painted above everything.
//!
//! A dragged card has to be visible outside the column it came from, outside
//! the scrollable that clips that column, and above any modal on screen. No
//! position in the widget tree satisfies all three, so the feedback is not in
//! the tree at all: it is hosted by the modal host's overlay, above every
//! modal, for exactly as long as the drag lasts.
//!
//! The hosted element never receives an event, which matters because it sits
//! directly under the pointer it is chasing and would otherwise intercept the
//! very moves that drive it. An [`OverlayLayer`] installed alongside it only
//! advances the drag's state each frame.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{AnimInstant, Curve, curves};
use aimer_attribute::position::Vec2d;
use aimer_events::window::request_animation_frame;
use aimer_modal::{OverlayContentHandle, OverlayLayer, OverlayLayerHandle};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyWidget, Widget};

use crate::DragSession;
use crate::target::clear_hover;

/// How long a refused drop takes to travel back to where it came from.
const SPRING_BACK_DURATION: Duration = Duration::from_millis(180);

/// Which way a drag is allowed to move.
///
/// Constraining an axis is what makes a reorderable list feel like a list
/// rather than a free canvas: the card follows the finger along the list and
/// ignores it across.
///
/// # Examples
///
/// ```
/// use aimer_attribute::position::Vec2d;
/// use aimer_dnd::DragAxis;
///
/// let start = Vec2d { x: 10.0, y: 10.0 };
/// let now = Vec2d { x: 40.0, y: 90.0 };
///
/// assert_eq!(DragAxis::Free.constrain(start, now), now);
/// assert_eq!(DragAxis::Vertical.constrain(start, now).x, start.x);
/// assert_eq!(DragAxis::Horizontal.constrain(start, now).y, start.y);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DragAxis {
    /// The feedback follows the pointer in both directions.
    #[default]
    Free,
    /// Only vertical movement is applied; the horizontal position is frozen at
    /// the point the drag started.
    Vertical,
    /// Only horizontal movement is applied.
    Horizontal,
}

impl DragAxis {
    /// Projects `pos` onto the axis, relative to where the drag started.
    #[inline]
    pub fn constrain(self, start: Vec2d, pos: Vec2d) -> Vec2d {
        match self {
            Self::Free => pos,
            Self::Vertical => Vec2d {
                x: start.x,
                y: pos.y,
            },
            Self::Horizontal => Vec2d {
                x: pos.x,
                y: start.y,
            },
        }
    }
}

/// A refused drop travelling back to its origin.
struct SpringBack {
    started: Cell<Option<AnimInstant>>,
    from: Vec2d,
}

/// Everything the painter needs to draw one drag.
struct OverlayState {
    /// Rebuilt lazily, once, on the first frame that paints it: constructing an
    /// element needs a [`BuildContext`], and the drag begins in an event
    /// handler, which has none.
    ///
    /// `None` for a drag with no feedback. The state still exists, because it
    /// is also what settles the drop.
    build: Option<Rc<dyn Fn() -> AnyWidget>>,
    /// The feedback, once built: hosted by the overlay and taken down with the
    /// drag.
    hosted: Option<OverlayContentHandle>,
    /// The device scale the follower converts the logical position by.
    scale: Rc<Cell<f32>>,
    /// The last position the follower reported, kept for the frame after a
    /// spring-back arrives and the feedback is about to go.
    last_position: Rc<Cell<Vec2d>>,
    /// Where inside the feedback the pointer grabbed it, so the card does not
    /// jump to have its corner under the cursor.
    grab_offset: Vec2d,
    /// Where the pointer was when the drag started, and where a refused drop
    /// returns to.
    start: Vec2d,
    axis: DragAxis,
    spring: Option<SpringBack>,
    /// Set by the release that asked for a drop, cleared by the frame that
    /// settles it.
    resolving: bool,
    on_completed: Option<Rc<dyn Fn(bool)>>,
}

thread_local! {
    static OVERLAY: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
    static HANDLE: RefCell<Option<OverlayLayerHandle>> = const { RefCell::new(None) };
}

/// The drag feedback layer.
///
/// This is a namespace rather than a widget: there is one drag at a time and
/// the feedback belongs to it, not to a position in the tree.
pub struct DragOverlay;

impl DragOverlay {
    /// Starts painting `build`'s output at the pointer.
    ///
    /// `grab_offset` is the vector from the feedback's top-left corner to the
    /// pointer, and `start` is where the pointer was when the drag began — the
    /// point a refused drop returns to.
    pub fn show(
        build: Option<Rc<dyn Fn() -> AnyWidget>>,
        grab_offset: Vec2d,
        start: Vec2d,
        axis: DragAxis,
    ) {
        OVERLAY.with_borrow_mut(|overlay| {
            *overlay = Some(OverlayState {
                build,
                hosted: None,
                scale: Rc::new(Cell::new(1.0)),
                last_position: Rc::new(Cell::new(start)),
                grab_offset,
                start,
                axis,
                spring: None,
                resolving: false,
                on_completed: None,
            });
        });
        HANDLE.with_borrow_mut(|handle| {
            if handle.is_none() {
                *handle = Some(OverlayLayer::install(Rc::new(|ctx, _canvas| tick(ctx))));
            }
        });
        request_animation_frame();
    }

    /// Removes the feedback immediately, as an accepted drop does.
    pub fn hide() {
        clear_hover();
        OVERLAY.with_borrow_mut(|overlay| *overlay = None);
        request_animation_frame();
    }

    /// Sends the feedback back to where the drag started, then removes it.
    ///
    /// This is the only feedback a refused drop gives: the card visibly does
    /// not stay where it was let go.
    pub fn spring_back() {
        clear_hover();
        let springing = OVERLAY.with_borrow_mut(|overlay| {
            let Some(state) = overlay.as_mut() else {
                return false;
            };
            let from = state.painted_position();
            state.spring = Some(SpringBack {
                started: Cell::new(None),
                from,
            });
            true
        });
        if springing {
            request_animation_frame();
        }
    }

    /// Records that a drop was requested and must be settled next frame.
    ///
    /// Whether a target accepted the payload is only knowable *after* the
    /// routed drop pass, which runs inside the dispatcher once the releasing
    /// element has already returned. Rather than invent a second callback from
    /// the dispatcher back into this crate, the answer is read one frame later
    /// from the session itself: if the payload is gone, somebody took it.
    pub fn resolve_on_next_frame(on_completed: Option<Rc<dyn Fn(bool)>>) {
        OVERLAY.with_borrow_mut(|overlay| {
            if let Some(state) = overlay.as_mut() {
                state.resolving = true;
                state.on_completed = on_completed;
            }
        });
        request_animation_frame();
    }

    /// Returns whether feedback is currently being painted.
    pub fn is_showing() -> bool {
        OVERLAY.with_borrow(|overlay| overlay.is_some())
    }
}

impl Drop for OverlayState {
    fn drop(&mut self) {
        if let Some(hosted) = self.hosted.take() {
            hosted.remove();
        }
    }
}

impl OverlayState {
    /// Where the feedback's top-left corner is right now, in logical window
    /// pixels: under the pointer, or part-way along a spring-back. `None` once
    /// a spring-back has arrived.
    fn position_now(&self) -> Option<Vec2d> {
        if self.spring.is_some() {
            self.spring_position(AnimInstant::now())
        } else {
            Some(self.painted_position())
        }
    }

    /// Where the feedback's top-left corner is this frame.
    fn painted_position(&self) -> Vec2d {
        let pointer = DragSession::position().unwrap_or(self.start);
        let constrained = self.axis.constrain(self.start, pointer);
        Vec2d {
            x: constrained.x - self.grab_offset.x,
            y: constrained.y - self.grab_offset.y,
        }
    }

    /// Where a spring-back has reached, or `None` once it has arrived.
    fn spring_position(&self, now: AnimInstant) -> Option<Vec2d> {
        let target = Vec2d {
            x: self.start.x - self.grab_offset.x,
            y: self.start.y - self.grab_offset.y,
        };
        let spring = self.spring.as_ref()?;
        let started = match spring.started.get() {
            Some(started) => started,
            None => {
                spring.started.set(Some(now));
                now
            }
        };
        let elapsed = now.duration_since(started).as_secs_f32();
        let t = (elapsed / SPRING_BACK_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        if t >= 1.0 {
            return None;
        }
        let eased = curves::EASE_OUT.transform(t);
        Some(Vec2d {
            x: spring.from.x + (target.x - spring.from.x) * eased,
            y: spring.from.y + (target.y - spring.from.y) * eased,
        })
    }
}

/// Advances the drag one frame: settles a requested drop, carries a refused
/// one back, and makes sure the feedback is hosted.
///
/// Returns whether the layer stays installed. It stays for as long as the
/// application lives once a first drag has happened, because installing and
/// removing it per drag would cost a modal-host command each time and it does
/// nothing at all when no drag is in flight.
fn tick(ctx: &BuildContext) -> bool {
    OVERLAY.with_borrow_mut(|overlay| {
        let Some(state) = overlay.as_mut() else {
            return true;
        };
        state.scale.set(ctx.scale);

        if state.resolving {
            state.resolving = false;
            // The drag is over either way, so no target may keep believing it
            // is hovered.
            clear_hover();
            let accepted = !DragSession::is_active();
            if let Some(completed) = state.on_completed.take() {
                completed(accepted);
            }
            if accepted {
                *overlay = None;
                request_animation_frame();
                return true;
            }
            // Refused: the payload is still in flight and has to be dropped,
            // and the feedback has to visibly fail to stay where it was let go.
            DragSession::cancel_any();
            let from = state.painted_position();
            state.spring = Some(SpringBack {
                started: Cell::new(None),
                from,
            });
        }

        if state.spring.is_some() {
            if state.spring_position(AnimInstant::now()).is_some() {
                request_animation_frame();
            } else {
                *overlay = None;
                request_animation_frame();
                return true;
            }
        }

        if state.hosted.is_none()
            && let Some(build) = state.build.clone()
        {
            let element = build().to_element(ctx);
            state.hosted = Some(OverlayLayer::host_element(
                element,
                follower_position(&state.scale, &state.last_position),
            ));
        }
        true
    })
}

/// The place the hosted feedback asks for each time the render tree syncs, in
/// device pixels.
///
/// The drag's position comes from pointer events and cached element bounds,
/// which are logical, while the tree places elements in device pixels — so it
/// is scaled here, at the one place the two spaces meet. Skipping this puts the
/// feedback at `1 / scale` of its distance from the window origin, which is
/// invisible on a 1:1 display and half a screen out on a retina one.
fn follower_position(
    scale: &Rc<Cell<f32>>,
    last_position: &Rc<Cell<Vec2d>>,
) -> Rc<dyn Fn() -> Vec2d> {
    let scale = Rc::clone(scale);
    let last_position = Rc::clone(last_position);
    Rc::new(move || {
        let logical = OVERLAY
            .with_borrow(|overlay| overlay.as_ref().and_then(OverlayState::position_now))
            .unwrap_or_else(|| last_position.get());
        last_position.set(logical);
        Vec2d {
            x: logical.x * scale.get(),
            y: logical.y * scale.get(),
        }
    })
}

#[cfg(test)]
mod tests {
    use aimer_container::{Container, ZeroSizedBox};
    use aimer_events::pointer::PointerSource;
    use aimer_widget::PointerKey;

    use super::*;
    use crate::DragPayload;
    use crate::test_support::headless_context;

    /// The feedback's top-left corner, in logical window coordinates.
    fn painted_corner() -> Vec2d {
        OVERLAY.with_borrow(|overlay| {
            overlay
                .as_ref()
                .expect("the overlay is showing")
                .position_now()
                .expect("the feedback is not springing back")
        })
    }

    /// Where the hosted feedback asks to be placed, in device pixels.
    fn device_corner() -> Vec2d {
        let (scale, last) = OVERLAY.with_borrow(|overlay| {
            let state = overlay.as_ref().expect("the overlay is showing");
            (Rc::clone(&state.scale), Rc::clone(&state.last_position))
        });
        follower_position(&scale, &last)()
    }

    /// Coordinates arrive from events in logical units and the canvas draws in
    /// device pixels, so a display that is not 1:1 is the only place the
    /// difference shows: at scale 2 an unconverted position lands at half the
    /// distance from the origin, and the card floats away from the pointer.
    #[test]
    fn the_feedback_sits_under_the_pointer_on_a_scaled_display() {
        DragSession::cancel_any();
        DragOverlay::hide();

        let pointer = PointerKey::new(PointerSource::Mouse, 0);
        let start = Vec2d { x: 100.0, y: 100.0 };
        let grab_offset = Vec2d { x: 20.0, y: 10.0 };

        assert!(DragSession::begin(pointer, DragPayload::new(7u32), start));
        DragOverlay::show(
            Some(Rc::new(|| {
                Container::new()
                    .width(120)
                    .height(60)
                    .child(ZeroSizedBox)
                    .boxed()
            })),
            grab_offset,
            start,
            DragAxis::Free,
        );
        DragSession::update(pointer, Vec2d { x: 260.0, y: 190.0 });

        let mut ctx = headless_context(800.0, 600.0);
        ctx.scale = 2.0;
        assert!(tick(&ctx), "the layer stays installed");

        assert_eq!(
            painted_corner(),
            Vec2d { x: 240.0, y: 180.0 },
            "the corner is the pointer minus the grab offset, whatever the scale"
        );
        assert_eq!(
            device_corner(),
            Vec2d { x: 480.0, y: 360.0 },
            "the tree places it in device pixels"
        );

        DragSession::cancel_any();
        DragOverlay::hide();
    }

    #[test]
    fn a_free_axis_follows_the_pointer_exactly() {
        let start = Vec2d { x: 3.0, y: 4.0 };
        let now = Vec2d { x: 30.0, y: 40.0 };

        assert_eq!(DragAxis::Free.constrain(start, now), now);
    }

    #[test]
    fn a_constrained_axis_freezes_the_other_one() {
        let start = Vec2d { x: 3.0, y: 4.0 };
        let now = Vec2d { x: 30.0, y: 40.0 };

        assert_eq!(
            DragAxis::Vertical.constrain(start, now),
            Vec2d { x: 3.0, y: 40.0 }
        );
        assert_eq!(
            DragAxis::Horizontal.constrain(start, now),
            Vec2d { x: 30.0, y: 4.0 }
        );
    }
}
