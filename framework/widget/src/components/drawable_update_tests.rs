//! `Drawable::update` is the per-frame traversal that replaces `draw`.
//!
//! An element may implement only `update` (the target shape) or only the
//! legacy `draw` (not yet migrated); both must be driven by the same frame
//! traversal, and the deprecated `draw` entry point must still reach `update`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::OnceLock;

use aimer_attribute::size::ResolvedSize;

use crate::base::BuildContext;
use crate::components::context::WindowHandle;
use crate::{Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement};

fn context() -> BuildContext<'static> {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime builds")
    });
    let _guard = runtime.enter();
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        ResolvedSize {
            width: 8.0,
            height: 8.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone(),
    )
}

/// Implements only `update`: no `draw` at all.
struct UpdateOnly {
    updates: Rc<Cell<u32>>,
}

impl VisitorElement for UpdateOnly {
    fn debug_name(&self) -> &'static str {
        "UpdateOnly"
    }
}
impl EventElement for UpdateOnly {}
impl LayoutElement for UpdateOnly {}
impl Rebuildable for UpdateOnly {}

impl Drawable for UpdateOnly {
    fn update(&self, _ctx: &BuildContext) {
        self.updates.set(self.updates.get() + 1);
    }
}

/// Implements only the legacy `draw`, as every widget does today.
struct DrawOnly {
    draws: Rc<Cell<u32>>,
}

impl VisitorElement for DrawOnly {
    fn debug_name(&self) -> &'static str {
        "DrawOnly"
    }
}
impl EventElement for DrawOnly {}
impl LayoutElement for DrawOnly {}
impl Rebuildable for DrawOnly {}

impl Drawable for DrawOnly {
    fn update(&self, _ctx: &BuildContext) {
        self.draws.set(self.draws.get() + 1);
    }
}

#[test]
fn an_element_can_implement_only_update() {
    let updates = Rc::new(Cell::new(0));
    let element = UpdateOnly {
        updates: updates.clone(),
    }
    .boxed();

    element.update(&context());

    assert_eq!(updates.get(), 1);
}

#[test]
fn a_legacy_draw_only_element_is_driven_by_update() {
    let draws = Rc::new(Cell::new(0));
    let element = DrawOnly {
        draws: draws.clone(),
    }
    .boxed();

    element.update(&context());

    assert_eq!(draws.get(), 1, "update falls back to the legacy draw");
}


#[test]
fn update_through_a_boxed_drawable_reaches_the_inner_element() {
    let updates = Rc::new(Cell::new(0));
    let boxed: Box<dyn Drawable> = Box::new(UpdateOnly {
        updates: updates.clone(),
    });

    boxed.update(&context());

    assert_eq!(updates.get(), 1);
}

/// Records its own paint locally.
struct LocalPainter {
    painted: Rc<Cell<u32>>,
}

impl VisitorElement for LocalPainter {
    fn debug_name(&self) -> &'static str {
        "LocalPainter"
    }
}
impl EventElement for LocalPainter {}
impl LayoutElement for LocalPainter {}
impl Rebuildable for LocalPainter {}

impl Drawable for LocalPainter {
    fn update(&self, _ctx: &BuildContext) {}

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {
        self.painted.set(self.painted.get() + 1);
    }
}

/// A wrapper that asks its erased child whether it paints locally, as the
/// animation and scroll wrappers do. The answer must come from the child, not
/// from the erased handle's default.
#[test]
fn an_erased_element_reports_its_childs_local_paint_support() {
    let ctx = context();
    let painted = Rc::new(Cell::new(0));
    let erased: crate::AnyElement = LocalPainter {
        painted: painted.clone(),
    }
    .boxed();

    assert!(erased.can_paint_local_v2(&ctx));
    assert!(erased.as_ref().can_paint_local_v2(&ctx));
    erased.paint_local_v2(&ctx);
    assert_eq!(painted.get(), 1, "the erased handle forwards the paint call");
    let boxed: Box<dyn Element> = Box::new(LocalPainter {
        painted: painted.clone(),
    });
    assert!(boxed.can_paint_local_v2(&ctx));
    boxed.paint_local_v2(&ctx);
    assert_eq!(painted.get(), 2);
}
