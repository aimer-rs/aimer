//! Headless Debug-mode measurements; run with:
//! cargo run -p aimer_router --example transitions_profile
//! Measures update traversal separately from retained presentation sync.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use aimer_time::{Duration, Instant};

use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
use aimer_router::*;
use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};
use aimer_widget::{AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement,
    PortableWidget, Rebuildable, VisitorElement, Widget};

aimer_tls::thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
    static BUILDS: Cell<usize> = const { Cell::new(0) };
}

struct Counter;
fn note_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() { count.set(Some(value + 1)); }
    });
}
// Safety: forwards each allocation, its layout, and its matching deallocation
// unchanged to System. The counter uses allocation-free thread-local cells.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout); }
    }
}
#[global_allocator]
static ALLOCATOR: Counter = Counter;

#[derive(Clone)]
struct Page(u8);
impl Route for Page {
    fn parse(_: &str) -> Option<Self> { None }
    fn format(&self) -> String { format!("/{}", self.0) }
    fn view_level(&self) -> u32 { u32::from(self.0) }
}
impl Router for Page { fn build(&self, _: &BuildContext) -> AnyWidget { routes(self.clone()) } }
struct Leaf;
struct LeafElement;
impl PortableWidget for Leaf {}
impl Widget for Leaf {
    fn to_element(self, _: &BuildContext) -> AnyElement {
        BUILDS.set(BUILDS.get() + 1);
        LeafElement.boxed()
    }
}
impl VisitorElement for LeafElement { fn debug_name(&self) -> &'static str { "ProfileLeaf" } }
impl Drawable for LeafElement { fn can_paint_local_v2(&self, _: &BuildContext) -> bool { true } }
impl EventElement for LeafElement {}
impl LayoutElement for LeafElement {}
impl Rebuildable for LeafElement {}
fn routes(_: Page) -> AnyWidget { Leaf.boxed() }

fn find_host(element: &dyn Element) -> Option<&dyn Element> {
    if element.debug_name() == "NavigationTransitionHost" { return Some(element); }
    let mut found = None;
    element.visit_children(&mut |child| { if found.is_none() { found = find_host(child); } });
    found
}

fn measure(label: &str, frames: usize, mut run: impl FnMut()) {
    for _ in 0..32 { run(); }
    let builds = BUILDS.get();
    let start = Instant::now();
    ALLOCATIONS.set(Some(0));
    for _ in 0..frames { run(); }
    let allocations = ALLOCATIONS.replace(None).unwrap();
    let elapsed = start.elapsed();
    println!("{label}: {:.0} ns/frame, {allocations} allocations/{frames} frames, {} page builds",
        elapsed.as_nanos() as f64 / frames as f64, BUILDS.get() - builds);
}

fn profile(name: &str, transition: impl NavigationTransition<Page>, ctx: &BuildContext) {
    let navigator = Navigator::new(Page(0), routes).transition(transition).to_element(ctx);
    let controller = NavigatorController::<Page>::of(ctx);
    controller.push(Page(1));
    navigator.rebuild_if_dirty(ctx);
    measure(&format!("{name} full update"), 2_000, || navigator.update(ctx));
    {
        let host = find_host(navigator.as_ref()).unwrap();
        let tree = RenderTree::new();
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let root = tree.add_root(bounds).unwrap();
        let outgoing = tree.add_child(root, bounds).unwrap();
        let incoming = tree.add_child(root, bounds).unwrap();
        for node in [outgoing, incoming] {
            tree.context(node).unwrap().begin_recording().unwrap().commit(vec![]).unwrap();
        }
        let paint = ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| ctx.fork_with_state(()));
        measure(&format!("{name} retained sync"), 2_000, || assert!(host.sync_local_v2_state(&paint)));
        assert_eq!(tree.draw_list_revision(outgoing).unwrap(), 1);
        assert_eq!(tree.draw_list_revision(incoming).unwrap(), 1);
    }
    let mut page = 1;
    measure(&format!("{name} interrupted navigation"), 100, || {
        page = if page == 1 { 2 } else { 1 };
        controller.push(Page(page));
        navigator.rebuild_if_dirty(ctx);
    });
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let canvas = aimer_canvas::InnerCanvas::new();
    let ctx = BuildContext::new(aimer_canvas::FrameCanvas::new(&canvas),
        ResolvedSize { width: 800.0, height: 600.0 }, 1.0, Default::default(), Default::default(),
        WindowHandle::headless(Default::default(), 1.0), runtime.handle().clone());
    let duration = Duration::from_secs(60);
    profile("cross-fade", CrossFadeTransition::new().duration(duration), &ctx);
    profile("zoom", ZoomTransition::new().duration(duration), &ctx);
}
