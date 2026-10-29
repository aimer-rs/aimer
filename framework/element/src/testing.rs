//! Helpers that let a unit test see what the frame loop would paint, without a
//! window.
//!
//! Painting no longer goes through the canvas an element is updated with: each
//! element records its own retained list, and the frame loop collects them from
//! the render tree. [`record_retained_paint`] does the same walk for a test, so
//! it can assert on what an element tree would show.

use std::sync::Arc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};

use crate::base::BuildContext;
use crate::Element;

/// The retained list one element recorded.
#[derive(Clone, Debug)]
pub struct RecordedPaint {
    /// The element's `debug_name`.
    pub element: &'static str,
    /// What it recorded, in its own local coordinates.
    pub commands: Arc<[DrawCommand]>,
}

/// Records the retained list of every element under (and including) `root`
/// that can paint locally, in paint order, the way the frame loop does.
///
/// Recording only reads: it does not run `sync_local_v2_state`, which saves hit
/// bounds from the canvas transform and would overwrite the positions an
/// earlier `update` stored. Run `layout`/`update` first when the paint depends
/// on state they establish.
///
/// Each element is given the context the render-tree sync would give it: its
/// parent's, narrowed to the parent's content size unless the parent is a
/// transparent wrapper around a single child of that size. Elements that do not
/// paint locally contribute nothing.
///
/// `ctx` is the root's own context.
#[doc(hidden)]
pub fn record_retained_paint(root: &dyn Element, ctx: &BuildContext<'_>) -> Vec<RecordedPaint> {
    let mut recorded = Vec::new();
    record(root, ctx, &mut recorded);
    recorded
}

/// Every command in `recorded`, flattened in paint order.
#[doc(hidden)]
pub fn all_commands(recorded: &[RecordedPaint]) -> Vec<&DrawCommand> {
    recorded
        .iter()
        .flat_map(|paint| paint.commands.iter())
        .collect()
}

fn record(element: &dyn Element, ctx: &BuildContext<'_>, recorded: &mut Vec<RecordedPaint>) {
    let size = element
        .retained_v2_bounds(ctx)
        .unwrap_or_else(|| element.content_size(ctx));
    let position = element.pos().unwrap_or_default();

    if element.can_paint_local_v2(ctx) {
        let tree = RenderTree::new();
        let node = tree
            .add_root(Rect::new(0.0, 0.0, 1_000_000.0, 1_000_000.0))
            .expect("a finite root rectangle");
        let node_context = tree.context(node).expect("a fresh node has a context");
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            element.paint_local_v2(ctx);
        });
        if let Ok(snapshot) = tree.draw_list_snapshot(node) {
            recorded.push(RecordedPaint {
                element: element.debug_name(),
                commands: snapshot.commands,
            });
        }
    }

    let mut base = ctx.clone();
    base.parent_size = size;
    base.parent_pos = Vec2d {
        x: ctx.parent_pos.x + position.x,
        y: ctx.parent_pos.y + position.y,
    };
    let mut retained_children = 0;
    let mut only_child_size = None;
    element.visit_retained_v2_children(&mut |_, child| {
        retained_children += 1;
        if retained_children == 1 {
            only_child_size = Some(child.content_size(ctx));
        }
    });
    if retained_children == 1 && only_child_size.is_some_and(|child| child == size) {
        base.box_constraint = ctx.box_constraint;
        base.parent_size = ctx.parent_size;
    } else {
        base.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
    }
    element.visit_retained_v2_children(&mut |index, child| {
        let child_ctx = element
            .retained_v2_child_context_at(ctx, child, index)
            .unwrap_or_else(|| base.clone());
        record(child, &child_ctx, recorded);
    });
}
