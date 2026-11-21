//! A rebuilt root the walk has yet to reach keeps the subtrees that do not hold it from being left alone.
//!
//! A subtree that was rebuilt is remembered with the context it was given, so that its next rebuild can
//! be handled by revisiting it alone. A full walk marks every such root stale and wants each reached
//! again; leaving a subtree alone is only wrong for the roots inside it, which the walk would then
//! never reach. The walk reaches the roots in tree order, so a root late in the tree used to turn off
//! the skipping of everything before it: the frame that follows a rebuild walked the whole window.

use super::*;

use std::cell::RefCell;
use aimer_widget::StateUpdater;

const LEAVES: usize = 24;

/// A leaf whose update depends on nothing but its context, and that counts its updates.
struct Leaf {
    updates: Rc<Cell<usize>>,
}

impl Widget for Leaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        LeafElement { updates: self.updates }.boxed()
    }
}

struct LeafElement {
    updates: Rc<Cell<usize>>,
}

impl VisitorElement for LeafElement {}

impl Element for LeafElement {
    fn name(&self) -> &'static str {
        "StaleRootsLeaf"
    }
}

impl EventElement for LeafElement {}
impl Rebuildable for LeafElement {}

impl LayoutElement for LeafElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(40.0), Dimension::Px(10.0)))
    }
}

impl Drawable for LeafElement {
    fn update(&self, _ctx: &BuildContext) {
        self.updates.set(self.updates.get() + 1);
    }

    fn reusable_walk(&self) -> bool {
        true
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        canvas.fill_rect(Rect::new(0.0, 0.0, 40.0, 10.0), [200, 200, 200, 255]);
        canvas.finish();
    }
}

type Updater = Rc<RefCell<Option<StateUpdater<Counter>>>>;

/// A widget that is built again whenever its counter is bumped: the root of a rebuilt subtree.
struct Rebuilt {
    updater: Updater,
}

struct Counter {
    value: u8,
    updater: Updater,
}

impl State<Rebuilt> for Counter {
    fn create(widget: Rebuilt) -> Self {
        Counter { value: 0, updater: widget.updater }
    }

    fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        *this.updater.borrow_mut() = Some(this.updater());
        SizedBox::new()
            .width(Dimension::Px(48.0))
            .height(Dimension::Px(48.0))
            .color(Color::Rgb(this.value.wrapping_mul(40), 0, 0))
    }
}

impl Widget for Rebuilt {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, Counter>(self, ctx, "StaleRootsRebuilt", None)
            .0
            .boxed()
    }
}

/// A page of leaves followed by a widget that is rebuilt on request: the leaves come first in tree
/// order, the rebuilt root after them.
fn start(updates: Rc<Cell<usize>>, updater: Updater) -> HeadlessAimerApp<impl Widget + 'static> {
    aimer_widget::set_clean_subtree_skip(true);
    // The leaves have a column of their own: a subtree is left alone only if all of it may be.
    let leaves = Column::new()
        .children(
            (0..LEAVES)
                .map(|_| Leaf { updates: updates.clone() }.boxed())
                .collect::<Vec<_>>(),
        )
        .boxed();
    let builder = Rebuilt { updater }.boxed();
    let mut app = AimerApp::start_headless_with(
        Column::new().children(vec![leaves, builder]),
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    app
}

fn move_pointer<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, y: f32) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(150.0, y as f64),
    });
    app.render_frame();
}

#[test]
fn a_rebuilt_root_late_in_the_tree_does_not_keep_the_leaves_before_it_from_being_left_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let updates = Rc::new(Cell::new(0));
    let updater: Updater = Rc::new(RefCell::new(None));
    let mut app = start(updates.clone(), updater.clone());
    let rebuild = || {
        updater
            .borrow()
            .as_ref()
            .expect("the widget was built")
            .set_state(|state| state.value = state.value.wrapping_add(1));
    };
    // The subtree is rebuilt, which the tree remembers; the walks after it give the leaves a record
    // again (a subtree is left alone after it was walked twice alike).
    rebuild();
    for step in 0..5 {
        move_pointer(&mut app, 400.0 + step as f32);
    }
    let before = updates.get();

    move_pointer(&mut app, 410.0);

    assert_eq!(
        updates.get(),
        before,
        "the leaves were walked again for a pointer that is nowhere near them; blockers {:?}",
        aimer_widget::clean_skip_blockers()
    );
}
