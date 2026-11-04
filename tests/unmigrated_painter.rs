//! An element with no retained paint paints nothing.
//!
//! The legacy canvas path is gone. An element that does not implement
//! `paint` has nothing to show and stays unresolved, so the frame
//! asks it again next time.

use aimer::{
    AimerApp, AnyElement, BuildContext, Drawable, Element, EventElement, LayoutElement,
    Rebuildable, ResolvedSize, Size, VisitorElement, Widget,
};

struct LegacyPainter;

impl Widget for LegacyPainter {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }
}

impl VisitorElement for LegacyPainter {
    fn debug_name(&self) -> &'static str {
        "LegacyPainter"
    }
}

impl EventElement for LegacyPainter {}
impl Rebuildable for LegacyPainter {}

impl LayoutElement for LegacyPainter {
    fn size(&self) -> Option<Size> {
        Some(Size::new(40.0, 40.0))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: 40.0 * ctx.scale,
            height: 40.0 * ctx.scale,
        }
    }
}

impl Drawable for LegacyPainter {
    fn update(&self, _ctx: &BuildContext) {}
}

#[test]
fn an_element_without_retained_paint_paints_nothing_and_is_asked_again() {
    let mut app = AimerApp::start_headless(LegacyPainter);
    for _ in 0..3 {
        app.render_frame();
    }

    let census = app
        .paint_source_census()
        .expect("a mounted page has a root");
    assert!(
        census
            .unresolved_roots
            .iter()
            .any(|element| element.debug_name == "LegacyPainter"),
        "the element has no paint to show: {:?}",
        census.unresolved_roots
    );
    assert!(
        census.drawn_unmapped.is_empty(),
        "{:?}",
        census.drawn_unmapped
    );
    assert_eq!(census.sync_error, None);
}
