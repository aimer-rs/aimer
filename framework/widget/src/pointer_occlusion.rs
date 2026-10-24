use aimer_attribute::Vec2d;

use crate::Element;

/// Returns whether something inside `element` hides what is painted behind it
/// at `pos` from the pointer.
///
/// A layered parent asks this of the layers above a point to decide whether the
/// layers below may still see the pointer there. The walk follows each element's
/// [`EventElement::occlusion_children_at`](crate::EventElement::occlusion_children_at),
/// so it descends only through children whose bounds hold `pos` and stops at the
/// first occluder. An element that reports no bounds is taken to be everywhere,
/// as the event dispatcher does, but never occludes on that account: only an
/// element that knows where it is can cover anything.
pub fn pointer_occluded_at(element: &dyn Element, pos: Vec2d) -> bool {
    let bounded = match element.pos_start_end() {
        Some((start, end)) => {
            if pos.x < start.x || pos.x > end.x || pos.y < start.y || pos.y > end.y {
                return false;
            }
            true
        }
        None => false,
    };
    if bounded && element.occludes_pointer_at(pos) {
        return true;
    }
    let mut occluded = false;
    element.occlusion_children_at(pos, &mut |child| {
        occluded = occluded || pointer_occluded_at(child, pos);
    });
    occluded
}

/// Asks for a frame so regions repaint against a changed view of the pointer.
///
/// A layer that starts or stops covering the pointer changes what the layers
/// below it may hover, which they only learn about while painting.
#[doc(hidden)]
#[inline]
pub fn request_pointer_view_frame() {
    aimer_events::window::request_animation_frame();
}
