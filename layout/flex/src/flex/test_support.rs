//! Shared helpers for the flex unit tests.
//!
//! Flex layout needs a [`BuildContext`], which in turn needs a canvas, a window
//! handle, and (off the web) an async runtime handle. Building one by hand in
//! every test module is noisy, so the headless construction lives here together
//! with the probe elements used to observe how many children a pass touches.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, StatelessElement,
    VisitorElement,
};

#[cfg(not(target_arch = "wasm32"))]
fn dummy_async_handle() -> tokio::runtime::Handle {
    use std::sync::OnceLock;

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
    });
    let _guard = runtime.enter();
    tokio::runtime::Handle::current()
}

/// Builds a headless [`BuildContext`] constrained to `width` x `height`.
///
/// `visible_rect` is passed through verbatim so a test can emulate the viewport
/// a `Scrollable` hands to its child.
pub(crate) fn dummy_build_context(
    width: f32,
    height: f32,
    visible_rect: Option<(f32, f32, f32, f32)>,
) -> BuildContext<'static> {
    let canvas = {
        let leaked: &'static InnerCanvas = Box::leak(Box::new(InnerCanvas::new()));
        FrameCanvas::new(leaked)
    };

    let mut context = BuildContext::new(
        canvas,
        ResolvedSize { width, height },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(
            winit::dpi::PhysicalSize::new(width.max(1.0) as u32, height.max(1.0) as u32),
            1.0,
        ),
        #[cfg(not(target_arch = "wasm32"))]
        dummy_async_handle(),
    );
    context.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: width,
        max_height: height,
    };
    context.visible_rect = visible_rect;
    context
}

/// Replaces one generated subtree, which is what advances the element tree
/// generation.
///
/// Every cached measurement is keyed by that generation, so this is how a test
/// reproduces the frame in which a `setState` or a completed `AsyncBuilder`
/// retires what the previous frames measured.
pub(crate) fn replace_a_generated_subtree(ctx: &BuildContext) {
    let element = StatelessElement::new(
        MarkerChild.boxed(),
        |_| MarkerChild.boxed(),
        None,
        "GenerationBump",
    );
    element.mark_needs_rebuild();
    element.rebuild_if_dirty(ctx);
}

/// The smallest possible child of the element built by
/// [`replace_a_generated_subtree`].
struct MarkerChild;

impl VisitorElement for MarkerChild {
    fn debug_name(&self) -> &'static str {
        "MarkerChild"
    }
}

impl EventElement for MarkerChild {}

impl Rebuildable for MarkerChild {}

impl Drawable for MarkerChild {
    fn update(&self, _ctx: &BuildContext) {}
}

impl LayoutElement for MarkerChild {}

/// A leaf element with a fixed size that counts how often it is measured and
/// drawn.
///
/// Both counters are shared, so a test can total the work an entire child list
/// received during one pass.
pub(crate) struct CountingChild {
    size: ResolvedSize,
    measured: Rc<Cell<usize>>,
    drawn: Rc<Cell<usize>>,
}

impl CountingChild {
    /// Creates an erased counting child of `width` x `height`.
    pub(crate) fn boxed_new(
        width: f32,
        height: f32,
        measured: &Rc<Cell<usize>>,
        drawn: &Rc<Cell<usize>>,
    ) -> AnyElement {
        Self {
            size: ResolvedSize { width, height },
            measured: measured.clone(),
            drawn: drawn.clone(),
        }
        .boxed()
    }
}

impl VisitorElement for CountingChild {
    fn debug_name(&self) -> &'static str {
        "CountingChild"
    }
}

impl EventElement for CountingChild {}

impl Rebuildable for CountingChild {}

impl Drawable for CountingChild {
    fn update(&self, _ctx: &BuildContext) {
        self.drawn.set(self.drawn.get() + 1);
    }
}

impl LayoutElement for CountingChild {
    fn is_layout_stable(&self) -> bool {
        true
    }

    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.measured.set(self.measured.get() + 1);
        self.size
    }
}

/// A leaf element whose main-axis extent can change between frames, standing in
/// for an implicitly animated child.
///
/// It records the canvas translation it was drawn at, so a test can assert that
/// its siblings moved with it.
pub(crate) struct ResizingChild {
    height: Rc<Cell<f32>>,
    drawn_at: Rc<Cell<(f32, f32)>>,
}

impl ResizingChild {
    /// Creates an erased child whose height follows `height`.
    pub(crate) fn boxed_new(height: &Rc<Cell<f32>>, drawn_at: &Rc<Cell<(f32, f32)>>) -> AnyElement {
        Self {
            height: height.clone(),
            drawn_at: drawn_at.clone(),
        }
        .boxed()
    }
}

impl VisitorElement for ResizingChild {
    fn debug_name(&self) -> &'static str {
        "ResizingChild"
    }
}

impl EventElement for ResizingChild {}

impl Rebuildable for ResizingChild {}

impl Drawable for ResizingChild {
    fn update(&self, ctx: &BuildContext) {
        let translation = ctx.canvas.get_transform_translation();
        self.drawn_at.set((translation.0, translation.1));
    }
}

impl LayoutElement for ResizingChild {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: 10.0,
            height: self.height.get(),
        }
    }
}

/// A leaf element that is not layout-stable, standing in for a child whose size
/// may change without the element being replaced, such as an animated builder's
/// output.
///
/// It counts how often it is measured, so a test can tell it was asked.
pub(crate) struct VolatileChild {
    height: Rc<Cell<f32>>,
    measured: Rc<Cell<usize>>,
}

impl VolatileChild {
    /// Creates an erased child whose height follows `height`.
    pub(crate) fn boxed_new(height: &Rc<Cell<f32>>, measured: &Rc<Cell<usize>>) -> AnyElement {
        Self {
            height: height.clone(),
            measured: measured.clone(),
        }
        .boxed()
    }
}

impl VisitorElement for VolatileChild {
    fn debug_name(&self) -> &'static str {
        "VolatileChild"
    }
}

impl EventElement for VolatileChild {}

impl Rebuildable for VolatileChild {}

impl Drawable for VolatileChild {
    fn update(&self, _ctx: &BuildContext) {}
}

impl LayoutElement for VolatileChild {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.measured.set(self.measured.get() + 1);
        ResolvedSize {
            width: 10.0,
            height: self.height.get(),
        }
    }
}

/// A leaf that keeps itself reachable by the rebuild pass the way a running
/// animator does, and counts how often the pass rebuilt it.
pub(crate) struct KeptChild {
    reachable: aimer_widget::KeepReachable,
    rebuilt: Rc<Cell<usize>>,
}

impl KeptChild {
    /// Creates an erased child that adds to `rebuilt` on every rebuild.
    pub(crate) fn boxed_new(rebuilt: &Rc<Cell<usize>>) -> AnyElement {
        Self {
            reachable: aimer_widget::KeepReachable::new(),
            rebuilt: rebuilt.clone(),
        }
        .boxed()
    }
}

impl VisitorElement for KeptChild {
    fn debug_name(&self) -> &'static str {
        "KeptChild"
    }
}

impl EventElement for KeptChild {}

impl Rebuildable for KeptChild {
    fn rebuild_if_dirty(&self, _ctx: &BuildContext) {
        self.rebuilt.set(self.rebuilt.get() + 1);
        self.reachable.hold();
    }
}

impl Drawable for KeptChild {
    fn update(&self, _ctx: &BuildContext) {}
}

impl LayoutElement for KeptChild {}

/// Eager children that count how often the container asks for one by position.
///
/// A pass that decides most children need no attention should not even look at
/// them, which this makes observable.
pub(crate) struct CountingSource {
    inner: crate::flex::children_source::EagerChildren,
    asked: Rc<Cell<usize>>,
}

impl CountingSource {
    /// Wraps `children`, adding one to `asked` for every `get`.
    pub(crate) fn new(children: Vec<AnyElement>, asked: &Rc<Cell<usize>>) -> Self {
        Self {
            inner: crate::flex::children_source::EagerChildren(children),
            asked: asked.clone(),
        }
    }
}

impl crate::flex::children_source::ChildrenSource for CountingSource {
    fn len(&self) -> usize {
        self.inner.len()
    }

    fn get(&self, index: usize) -> Option<&dyn Element> {
        self.asked.set(self.asked.get() + 1);
        self.inner.get(index)
    }

    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.inner.visit(visitor);
    }

    fn live_start(&self) -> Option<usize> {
        self.inner.live_start()
    }
}
