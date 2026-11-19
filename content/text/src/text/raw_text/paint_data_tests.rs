//! What a paragraph-laid-out text derives to paint from, which every frame asks again: whether it can
//! paint at all, and what it spills outside its box. The walk asks of each text on each frame, and the
//! answer is the same until its layout, its scale or its box change.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_cupid::font::FontStyle;
use aimer_style::{LineHeight, TextAlign, TextOverflow, TextStyle};
use aimer_widget::base::{BuildContext, Color, WindowHandle};
use aimer_widget::{Drawable, LayoutCache, LayoutElement};

use crate::{SpanStyle, StyledRange, TextSource};

use super::{RawTextWidget, paragraph_paint_computations, reset_paragraph_paint_computations};

fn context(runtime: &tokio::runtime::Runtime, width: f32, height: f32, scale: f32) -> BuildContext<'static> {
    // The canvas outlives the test; leaking keeps the borrow `'static`.
    let inner: &'static InnerCanvas = Box::leak(Box::new(InnerCanvas::new()));
    let mut context = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize { width, height },
        scale,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(winit::dpi::PhysicalSize::new(width as u32, height as u32), 1.0),
        runtime.handle().clone(),
    );
    context.box_constraint = aimer_attribute::BoxConstraint { min_width: 0.0, min_height: 0.0, max_width: width, max_height: height };
    context
}

fn ranged(text: &'static str) -> RawTextWidget {
    // A colour on the first word, and a slanted face, whose overhang spills outside the box.
    let ranges: Vec<StyledRange> = vec![
        StyledRange::new(0..3, SpanStyle::new().color(Color::Rgba(200, 30, 30, 255))),
        StyledRange::new(4..9, SpanStyle::new().color(Color::Rgba(30, 200, 30, 255))),
    ];
    RawTextWidget {
        text: TextSource::Static(text),
        text_style: TextStyle::default().text_overflow(TextOverflow::Wrap).font_style(FontStyle::Italic),
        text_align: TextAlign::TopLeft,
        line_height: LineHeight::Normal,
        text_indent: 0.0,
        ranges: Some(Rc::from(ranges)),
        cache: LayoutCache::new(),
        _typeface: Cell::new(None),
    }
}

const TEXT: &str = "let value = compute(1, 2) and more words that wrap around";

#[test]
fn the_same_layout_in_the_same_box_is_derived_once_however_often_it_is_asked() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime, 140.0, 400.0, 1.0);
    let widget = ranged(TEXT);
    reset_paragraph_paint_computations();

    for _ in 0..10 {
        assert!(widget.can_paint(&ctx));
        let _ = widget.paint_outsets(&ctx);
    }

    assert_eq!(paragraph_paint_computations(), 1);
}

#[test]
fn another_width_height_scale_or_layout_is_derived_again() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let widget = ranged(TEXT);
    let derive = |ctx: &BuildContext| {
        let before = paragraph_paint_computations();
        assert!(widget.can_paint(ctx));
        paragraph_paint_computations() - before
    };
    reset_paragraph_paint_computations();

    assert_eq!(derive(&context(&runtime, 140.0, 400.0, 1.0)), 1, "the first time");
    assert_eq!(derive(&context(&runtime, 140.0, 400.0, 1.0)), 0, "the same again");
    assert_eq!(derive(&context(&runtime, 90.0, 400.0, 1.0)), 1, "a narrower box wraps it again");
    assert_eq!(derive(&context(&runtime, 90.0, 300.0, 1.0)), 1, "a box as wide and not as tall places it again");
    assert_eq!(derive(&context(&runtime, 90.0, 300.0, 2.0)), 1, "a scale lays it out again");
    assert_eq!(derive(&context(&runtime, 90.0, 300.0, 2.0)), 0);
    widget.invalidate_layout();
    assert_eq!(derive(&context(&runtime, 90.0, 300.0, 2.0)), 1, "a layout that was retired is made again");
}

/// Whatever it remembers, it says what a text that was never asked before says.
#[test]
fn what_is_remembered_is_what_a_fresh_text_says() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let remembering = ranged(TEXT);
    for (width, height, scale) in [(140.0, 400.0, 1.0), (60.0, 400.0, 1.0), (140.0, 30.0, 2.0), (140.0, 400.0, 1.0), (60.0, 400.0, 1.0)] {
        let ctx = context(&runtime, width, height, scale);
        let fresh = ranged(TEXT);
        assert_eq!(remembering.can_paint(&ctx), fresh.can_paint(&ctx), "{width} x {height} at {scale}");
        assert_eq!(remembering.paint_outsets(&ctx), fresh.paint_outsets(&ctx), "{width} x {height} at {scale}");
    }
    let outsets = remembering.paint_outsets(&context(&runtime, 140.0, 400.0, 1.0)).expect("the slant spills");
    assert!(outsets.iter().any(|outset| *outset > 0.0), "{outsets:?}");
}

#[test]
fn a_box_that_cannot_hold_text_is_declined_and_does_not_poison_the_next_ask() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let widget = ranged(TEXT);

    assert!(!widget.can_paint(&context(&runtime, 140.0, f32::NAN, 1.0)), "no height to place it in");
    assert!(!widget.can_paint(&context(&runtime, 140.0, f32::NAN, 1.0)), "and still none");
    assert!(widget.can_paint(&context(&runtime, 140.0, 400.0, 1.0)), "a box that can");
    assert!(!widget.can_paint(&context(&runtime, 140.0, 400.0, 0.0)), "a scale of nothing");
    assert!(widget.can_paint(&context(&runtime, 140.0, 400.0, 1.0)));
}

#[test]
fn the_empty_text_and_a_text_without_ranges_are_asked_as_before() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime, 140.0, 400.0, 1.0);
    let mut empty = ranged("");
    empty.ranges = None;
    reset_paragraph_paint_computations();

    // Plain text takes the glyph recorder, not the paragraph's.
    let _ = empty.can_paint(&ctx);
    let _ = empty.paint_outsets(&ctx);

    assert_eq!(paragraph_paint_computations(), 0, "nothing of the paragraph was derived");
}
