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

fn with_visible(width: f32, visible: (f32, f32, f32, f32)) -> BuildContext<'static> {
    let mut ctx = context(width);
    ctx.set_visible_rect(Some(visible));
    ctx
}

/// A 100 by 40 subtree has a margin of its own extent along each axis: it is wholly
/// inside a visible rectangle that reaches from -100 to 200 across and from -40 to
/// 80 down.
const INSIDE: (f32, f32, f32, f32) = (-500.0, -500.0, 2000.0, 2000.0);
const INSIDE_SHIFTED: (f32, f32, f32, f32) = (-520.0, -480.0, 2100.0, 1900.0);
const AT_THE_EDGE: (f32, f32, f32, f32) = (-60.0, -500.0, 2000.0, 2000.0);
/// Wide enough across to be inside, a vertical scroll away from being so down.
const SAME_ACROSS: (f32, f32, f32, f32) = (-500.0, -500.0, 2000.0, 2000.0);
const MOVED_DOWN: (f32, f32, f32, f32) = (-500.0, -300.0, 2000.0, 2000.0);
const CUT_BELOW: (f32, f32, f32, f32) = (-500.0, -500.0, 2000.0, 520.0);

#[test]
fn a_subtree_wholly_inside_the_visible_rectangle_does_not_depend_on_its_edges() {
    set_clean_subtree_skip(true);
    reset_clean_subtree_skip_stats();
    let element = ElementId::next();
    let (a, b) = (with_visible(100.0, INSIDE), with_visible(100.0, INSIDE_SHIFTED));

    // The same key by every route a skip asks it.
    assert!(ContextKey::of(&a, Reads::NONE).matches(&b, Reads::NONE));
    assert!(ContextKey::of(&b, Reads::NONE).matches(&a, Reads::NONE));
    assert_eq!(context_hash(&a, Reads::NONE), context_hash(&b, Reads::NONE));

    with_v2_render_tree_context(RenderTree::new(), Rc::new(ElementNodeMap::default()), None, || {
        walk(element, &a);
        walk(element, &b);
        assert_eq!(records(), 1, "a window that only moved was taken for a different context");
    });
}

#[test]
fn a_subtree_near_the_edge_of_the_visible_rectangle_does_depend_on_it() {
    let (inside, at_the_edge) = (with_visible(100.0, INSIDE), with_visible(100.0, AT_THE_EDGE));

    assert!(!ContextKey::of(&inside, Reads::NONE).matches(&at_the_edge, Reads::NONE));
    assert!(!ContextKey::of(&at_the_edge, Reads::NONE).matches(&inside, Reads::NONE));
    assert_ne!(context_hash(&inside, Reads::NONE), context_hash(&at_the_edge, Reads::NONE));

    // Two windows that both cut across the subtree differ when their edges do.
    let (cut, cut_elsewhere) =
        (with_visible(100.0, (10.0, 0.0, 50.0, 40.0)), with_visible(100.0, (20.0, 0.0, 50.0, 40.0)));
    assert!(!ContextKey::of(&cut, Reads::NONE).matches(&cut_elsewhere, Reads::NONE));
}

#[test]
fn an_unbounded_window_is_not_a_wholly_visible_one() {
    // No rectangle at all means nothing limits it, which is not the same as a
    // rectangle that happens to contain it: the two are told apart.
    let (none, inside) = (context(100.0), with_visible(100.0, INSIDE));
    assert!(!ContextKey::of(&none, Reads::NONE).matches(&inside, Reads::NONE));
}

/// A list scrolls along one axis, and its viewport may be exactly as wide as its
/// rows: along that axis the rows may move wholly inside the window, and across it
/// they need no room.
#[test]
fn the_axes_are_told_apart() {
    // Moved down, still wholly inside along both axes: the same.
    let (a, moved) = (with_visible(100.0, SAME_ACROSS), with_visible(100.0, MOVED_DOWN));
    assert!(ContextKey::of(&a, Reads::NONE).matches(&moved, Reads::NONE));

    // A window exactly as wide as the subtree has no margin across, so its edges
    // across stay exact; moving it down still changes nothing.
    let exact_across_a = with_visible(100.0, (0.0, -500.0, 100.0, 2000.0));
    let exact_across_b = with_visible(100.0, (0.0, -300.0, 100.0, 2000.0));
    assert!(ContextKey::of(&exact_across_a, Reads::NONE).matches(&exact_across_b, Reads::NONE));

    // Moved sideways across the same window, it is not the same.
    let sideways = with_visible(100.0, (10.0, -500.0, 100.0, 2000.0));
    assert!(!ContextKey::of(&exact_across_a, Reads::NONE).matches(&sideways, Reads::NONE));

    // A window that stops short below the subtree cuts across it along that axis.
    let cut = with_visible(100.0, CUT_BELOW);
    assert!(!ContextKey::of(&a, Reads::NONE).matches(&cut, Reads::NONE));
}

fn moved(width: f32, by: (f32, f32)) -> BuildContext<'static> {
    let ctx = context(width);
    ctx.canvas.translate(aimer_attribute::position::Vec2d { x: by.0, y: by.1 });
    ctx
}

/// A scrolled list moves its rows on the canvas and changes nothing else about
/// them: a row that never looked at where it is drawn is the same row.
#[test]
fn a_subtree_that_never_reads_the_canvas_transform_may_move() {
    let (here, there) = (moved(100.0, (0.0, 0.0)), moved(100.0, (0.0, 240.0)));
    assert!(ContextKey::of(&here, Reads::NONE).matches(&there, Reads::NONE));
    assert!(ContextKey::of(&there, Reads::NONE).matches(&here, Reads::NONE));
    assert_eq!(context_hash(&here, Reads::NONE), context_hash(&there, Reads::NONE));
}

#[test]
fn a_subtree_that_reads_the_canvas_transform_may_not() {
    let reads = Reads { cursor: false, transform: true, window: false };
    let (here, there) = (moved(100.0, (0.0, 0.0)), moved(100.0, (0.0, 240.0)));
    assert!(!ContextKey::of(&here, reads).matches(&there, reads));
    assert_ne!(context_hash(&here, reads), context_hash(&there, reads));
    // Where it is drawn is the same, so is the subtree.
    let again = moved(100.0, (0.0, 0.0));
    assert!(ContextKey::of(&here, reads).matches(&again, reads));
}

/// The framework compares transforms to build a key; that must not look like the
/// subtree asked for it.
#[test]
fn building_a_key_does_not_read_the_transform() {
    let ctx = moved(100.0, (3.0, 4.0));
    let before = aimer_canvas::transform_reads();
    let reads = Reads { cursor: false, transform: true, window: false };
    let key = ContextKey::of(&ctx, reads);
    let _ = key.matches(&ctx, reads);
    let _ = context_hash(&ctx, reads);
    assert_eq!(aimer_canvas::transform_reads(), before);
    let _ = ctx.canvas.get_transform();
    assert_eq!(aimer_canvas::transform_reads(), before + 1);
}

/// What a walk that read the transform leaves in its record, and what a walk that
/// did not leaves.
#[test]
fn a_record_remembers_whether_its_walk_read_the_transform() {
    set_clean_subtree_skip(true);
    reset_clean_subtree_skip_stats();
    let (reading, blind) = (ElementId::next(), ElementId::next());
    let ctx = context(100.0);
    with_v2_render_tree_context(RenderTree::new(), Rc::new(ElementNodeMap::default()), None, || {
        for _ in 0..2 {
            let entry = enter();
            let _ = ctx.canvas.get_transform_translation();
            for _ in 1..MIN_SUBTREE {
                enter();
            }
            leave(reading, &ctx, entry);
            walk(blind, &ctx);
        }
        let transform_read = |element: ElementId| {
            RECORDS.with(|records| {
                records.borrow()[slot(element)]
                    .as_ref()
                    .filter(|record| record.element == element.get())
                    .map(|record| record.reads.transform)
            })
        };
        assert_eq!(transform_read(reading), Some(true));
        assert_eq!(transform_read(blind), Some(false));
    });
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Option<[u32; 4]> {
    Some([x.to_bits(), y.to_bits(), width.to_bits(), height.to_bits()])
}

/// A pointer that is outside the subtree's place before and after it moved is told
/// nothing either time: the hover it decided, it decides again.
#[test]
fn a_pointer_outside_both_places_does_not_tell_them_apart() {
    let (before, after) = (rect(0.0, 100.0, 80.0, 40.0), rect(0.0, 60.0, 80.0, 40.0));
    assert!(same_for_the_pointer(before, after, (400.0, 300.0)));
    assert!(same_for_the_pointer(before, after, (-1.0, -1.0)));
    // Not moving is the same whatever the pointer does.
    assert!(same_for_the_pointer(before, before, (40.0, 120.0)));
}

#[test]
fn a_pointer_inside_either_place_does_tell_them_apart() {
    let (before, after) = (rect(0.0, 100.0, 80.0, 40.0), rect(0.0, 60.0, 80.0, 40.0));
    assert!(!same_for_the_pointer(before, after, (40.0, 120.0)), "inside where it was");
    assert!(!same_for_the_pointer(before, after, (40.0, 70.0)), "inside where it is");
    // The edges count as inside, as they do for a hit test.
    assert!(!same_for_the_pointer(before, after, (80.0, 140.0)));
    // Appearing or vanishing is a difference.
    assert!(!same_for_the_pointer(before, None, (400.0, 300.0)));
    assert!(!same_for_the_pointer(None, after, (400.0, 300.0)));
    assert!(same_for_the_pointer(None, None, (400.0, 300.0)));
}

/// A subtree that is left alone reads nothing, but what contains it depends on
/// what it would have read: a container walked on its own, with a child skipped,
/// must not be recorded as if it had not looked at the pointer or at its place.
#[test]
fn leaving_a_subtree_alone_counts_as_reading_what_its_last_walk_read() {
    set_clean_subtree_skip(true);
    set_clean_skip_verify(false);
    reset_clean_subtree_skip_stats();
    let element = ElementId::next();
    let ctx = context(100.0);
    with_v2_render_tree_context(RenderTree::new(), Rc::new(ElementNodeMap::default()), None, || {
        for _ in 0..2 {
            crate::components::element::animation_only::begin_full_traversal();
            let entry = enter();
            let _ = ctx.canvas.get_transform_translation();
            let _ = ctx.cursor_pos();
            for _ in 1..MIN_SUBTREE {
                enter();
            }
            leave(element, &ctx, entry);
        }
        crate::components::element::animation_only::begin_full_traversal();

        let (transform, cursor) = (aimer_canvas::transform_reads(), CURSOR_READS.with(Cell::get));
        assert!(matches!(try_skip(element, &ctx), Decision::Skip), "the subtree was not left alone");
        assert_eq!(aimer_canvas::transform_reads(), transform + 1, "its place was not counted");
        assert_eq!(CURSOR_READS.with(Cell::get), cursor + 1, "the pointer was not counted");
    });
}
