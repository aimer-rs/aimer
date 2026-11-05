use std::cell::RefCell;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::base::BuildContext;

/// Builds the child context for overlay content at `offset` inside the viewport.
///
/// `offset` is relative to `ctx.parent_pos`. `child_bounds` receives the
/// unscaled content rectangle used by the barrier hit test, matching the
/// overlay's stable layout geometry throughout its animation.
pub(crate) fn overlay_child_context<'a>(
    ctx: &BuildContext<'a>,
    child_size: ResolvedSize,
    offset: Vec2d,
    child_bounds: &RefCell<Option<(Vec2d, Vec2d)>>,
) -> BuildContext<'a> {
    let (origin, end) = overlay_content_bounds(ctx.parent_pos, child_size, offset);
    *child_bounds.borrow_mut() = Some((origin, end));

    // The content is a child of the viewport, so a size it is given as a share
    // of its parent is a share of the viewport: both `parent_size` and the
    // constraint stay the viewport's, as for any child of the root. Handing the
    // content its own size for either would resolve a percentage against
    // itself, a second time, after layout resolved it against the viewport.
    let mut child_ctx = ctx.clone();
    child_ctx.parent_pos = origin;
    child_ctx.box_constraint = viewport_constraint(ctx);
    child_ctx.visible_rect = ctx
        .visible_rect
        .map(|(x, y, width, height)| (x - offset.x, y - offset.y, width, height));
    child_ctx
}

/// The room the viewport offers its content: all of it, at most.
pub(crate) fn viewport_constraint(ctx: &BuildContext) -> BoxConstraint {
    BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: ctx.parent_size.width,
        max_height: ctx.parent_size.height,
    }
}

fn overlay_content_bounds(
    parent_pos: Vec2d,
    child_size: ResolvedSize,
    offset: Vec2d,
) -> (Vec2d, Vec2d) {
    let origin = Vec2d {
        x: parent_pos.x + offset.x,
        y: parent_pos.y + offset.y,
    };
    (
        origin,
        Vec2d {
            x: origin.x + child_size.width,
            y: origin.y + child_size.height,
        },
    )
}

/// Returns whether `position` falls inside the last painted content rectangle.
pub(crate) fn contains(child_bounds: &RefCell<Option<(Vec2d, Vec2d)>>, position: Vec2d) -> bool {
    child_bounds.borrow().is_some_and(|(start, end)| {
        position.x >= start.x && position.x <= end.x && position.y >= start.y && position.y <= end.y
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use aimer_attribute::position::Vec2d;
    use aimer_attribute::size::ResolvedSize;

    use super::{contains, overlay_content_bounds};

    #[test]
    fn overlay_hit_bounds_remain_at_unscaled_layout_geometry() {
        let bounds = RefCell::new(None);
        *bounds.borrow_mut() = Some(overlay_content_bounds(
            Vec2d { x: 100.0, y: 50.0 },
            ResolvedSize {
                width: 80.0,
                height: 40.0,
            },
            Vec2d { x: 12.0, y: 8.0 },
        ));

        assert!(contains(&bounds, Vec2d { x: 140.0, y: 70.0 }));
        assert!(!contains(&bounds, Vec2d { x: 195.0, y: 70.0 }));
    }
}
