//! A record is built only for a subtree given the same context twice in a row.

use aimer_attribute::size::ResolvedSize;
use aimer_cupid::draw_cmd::RenderTree;

use super::*;
use crate::base::WindowHandle;

#[cfg(not(target_arch = "wasm32"))]
fn async_handle() -> tokio::runtime::Handle {
    use std::sync::OnceLock;

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the test")
        })
        .handle()
        .clone()
}

fn context(width: f32) -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        ResolvedSize { width, height: 40.0 },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        #[cfg(not(target_arch = "wasm32"))]
        async_handle(),
    )
}

/// Walks a subtree of `MIN_SUBTREE` elements that all promised a repeatable walk.
fn walk(element: ElementId, ctx: &BuildContext) {
    let entry = enter();
    for _ in 1..MIN_SUBTREE {
        enter();
    }
    leave(element, ctx, entry);
}

fn records() -> u64 {
    clean_subtree_skip_stats().records
}

#[test]
fn a_record_needs_the_same_context_twice_in_a_row() {
    set_clean_subtree_skip(true);
    reset_clean_subtree_skip_stats();
    let element = ElementId::next();
    let (narrow, wide) = (context(100.0), context(140.0));
    with_v2_render_tree_context(RenderTree::new(), Rc::new(ElementNodeMap::default()), None, || {
        walk(element, &narrow);
        assert_eq!(records(), 0, "the first sight of a subtree is not a record");

        walk(element, &narrow);
        assert_eq!(records(), 1, "the same context twice is");

        // A context that changes every walk never gives it one.
        for context in [&wide, &narrow, &wide, &narrow] {
            walk(element, context);
        }
        assert_eq!(records(), 1, "a subtree that kept changing was recorded");

        // Held still again, it is recorded again.
        walk(element, &narrow);
        assert_eq!(records(), 2);
    });
}

#[test]
fn two_subtrees_that_share_a_slot_do_not_make_each_other_steady() {
    set_clean_subtree_skip(true);
    reset_clean_subtree_skip_stats();
    let first = ElementId::next();
    // Ids are consecutive, so one a whole table length on lands in the same slot.
    let mut second = first;
    for _ in 0..SLOTS {
        second = ElementId::next();
    }
    assert_eq!(slot(first), slot(second));
    let ctx = context(100.0);

    with_v2_render_tree_context(RenderTree::new(), Rc::new(ElementNodeMap::default()), None, || {
        // They take turns with the slot, each with the same context as the other.
        for _ in 0..3 {
            walk(first, &ctx);
            walk(second, &ctx);
        }
        assert_eq!(records(), 0, "one element's walk made the other look steady");
    });
}
