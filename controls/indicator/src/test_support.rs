//! Helpers shared by the tests of this crate.

use std::sync::OnceLock;

use aimer_attribute::BoxConstraint;
use aimer_cupid::draw_cmd_v2::{Rect, RenderItem, RenderNodeId, RenderOp, RenderTree};
use aimer_widget::base::{BuildContext, ResolvedSize, Vec2d, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, PortableWidget, Rebuildable,
    VisitorElement, Widget,
};

pub(crate) const FRAME_SIZE: f32 = 400.0;
/// The size of the stand-in child, which is also the size of a slot.
pub(crate) const SLOT: f32 = 10.0;

fn runtime_handle() -> tokio::runtime::Handle {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("indicator layer test runtime should build")
        })
        .handle()
        .clone()
}

pub(crate) fn context() -> BuildContext<'static> {
    let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
    let mut context = BuildContext::new(
        aimer_canvas::FrameCanvas::new(inner),
        ResolvedSize { width: FRAME_SIZE, height: FRAME_SIZE },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        runtime_handle(),
    );
    context.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: FRAME_SIZE,
        max_height: FRAME_SIZE,
    };
    context
}

/// A leaf that is one slot large.
pub(crate) struct Slot;

impl Widget for Slot {
    fn to_element(self, _context: &BuildContext) -> AnyElement {
        SlotElement.boxed()
    }
}

impl PortableWidget for Slot {}

struct SlotElement;

impl Drawable for SlotElement {
    fn update(&self, _context: &BuildContext) {}
}

impl EventElement for SlotElement {}

impl LayoutElement for SlotElement {
    fn computed_size(&self, _context: &BuildContext) -> ResolvedSize {
        ResolvedSize { width: SLOT, height: SLOT }
    }
}

impl Rebuildable for SlotElement {}

impl VisitorElement for SlotElement {
    fn debug_name(&self) -> &'static str {
        "SlotElement"
    }
}

/// A retained tree whose root holds the layer's child, kept alive so a test can
/// read back what synchronization did to that child.
pub(crate) struct Retained {
    tree: RenderTree,
    root: RenderNodeId,
    child: RenderNodeId,
}

impl Retained {
    pub(crate) fn new() -> Self {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, FRAME_SIZE, FRAME_SIZE)).unwrap();
        let child = tree.add_child(root, Rect::new(0.0, 0.0, SLOT, SLOT)).unwrap();
        Self { tree, root, child }
    }

    pub(crate) fn sync(&self, element: &dyn Element, context: &BuildContext<'_>) -> bool {
        let retained = self.tree.context(self.root).unwrap();
        context.with_local_v2_paint_context(retained, |context| element.sync_local_v2_state(context))
    }

    /// The child's draw item, or `None` when the renderer would not draw it.
    pub(crate) fn drawn_child(&self) -> Option<RenderItem> {
        self.tree.render_all().into_iter().find_map(|op| match op {
            RenderOp::Draw(item) if item.element == self.child => Some(item),
            _ => None,
        })
    }
}

