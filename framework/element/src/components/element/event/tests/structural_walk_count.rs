//! How many children the structural walks visit per focus-directed key event.
//!
//! Measurement only: it asserts nothing about performance. It counts the
//! visitor calls a list container receives while one key event is dispatched to
//! the focused row at the end of the list, for a container that overrides
//! `structural_children` (as `RawFlex` does) and for one that keeps the default.
//!
//! ```text
//! CARGO_TARGET_DIR="agents/target" cargo nextest run -p aimer_element \
//!     structural_walk_count --run-ignored only --no-capture
//! ```

use super::focus::FocusKeyElement;
use super::*;

#[derive(Default)]
struct Visits {
    visual: Cell<u64>,
    event: Cell<u64>,
    structural: Cell<u64>,
    focus_scan_rows: Cell<u64>,
}

impl Visits {
    fn snapshot(&self) -> [u64; 3] {
        [self.visual.get(), self.event.get(), self.structural.get()]
    }
}

struct CountingList {
    rows: Vec<AnyElement>,
    overrides_structural: bool,
    visits: Rc<Visits>,
}

impl VisitorElement for CountingList {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for row in &self.rows {
            self.visits.visual.set(self.visits.visual.get() + 1);
            visitor(row.as_ref());
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for row in &self.rows {
            self.visits.event.set(self.visits.event.get() + 1);
            visitor(row.as_ref());
        }
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if self.overrides_structural {
            for row in &self.rows {
                self.visits.structural.set(self.visits.structural.get() + 1);
                visitor(row.as_ref());
            }
            return;
        }
        // The trait default, copied so that its two inner walks are counted
        // through `event_children` and `visit_children` above.
        let mut children: smallvec::SmallVec<[&'a dyn Element; 8]> = smallvec::SmallVec::new();
        self.event_children(&mut |child| children.push(child));
        let event_child_count = children.len();
        let mut visual_index = 0;
        self.visit_children(&mut |child| {
            if visual_index < event_child_count
                && std::ptr::eq(children[visual_index], child)
            {
                visual_index += 1;
                return;
            }
            visual_index += 1;
            if !children.iter().any(|existing| std::ptr::eq(*existing, child)) {
                children.push(child);
            }
        });
        for child in children {
            visitor(child);
        }
    }
}

impl Element for CountingList {
    fn name(&self) -> &'static str {
        "CountingList"
    }
}

impl EventElement for CountingList {}
impl LayoutElement for CountingList {}
impl Drawable for CountingList {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for CountingList {}

fn key(named: NamedKey) -> ElementEvent {
    ElementEvent::KeyInput {
        key: named,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    }
}

fn run(rows: usize, overrides_structural: bool, focus_first: bool) {
    let visits = Rc::new(Visits::default());
    let focused = FocusNode::new();
    let mut elements = Vec::with_capacity(rows);
    for index in 0..rows {
        let y = index as f32 * 10.0;
        elements.push(
            FocusKeyElement {
                node: if index == if focus_first { 0 } else { rows - 1 } { focused.clone() } else { FocusNode::new() },
                bounds: (Vec2d { x: 0.0, y }, Vec2d { x: 10.0, y: y + 10.0 }),
                key_events: Rc::new(Cell::new(0)),
                consume_keys: Rc::new(Cell::new(index == if focus_first { 0 } else { rows - 1 })),
            }
            .boxed(),
        );
    }
    let root = CountingList { rows: elements, overrides_structural, visits: visits.clone() }.boxed();
    let mut dispatcher = EventDispatcher::new();
    focused.request_focus();

    let before = visits.snapshot();
    let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    let settle = visits.snapshot();

    let first = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &key(NamedKey::Enter));
    let after_first = visits.snapshot();
    let second = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &key(NamedKey::Enter));
    let after_second = visits.snapshot();
    assert!(first.is_consumed() && second.is_consumed(), "the focused row must take the key");
    // The free function a `RawFocusable` calls for each key event it forwards.
    let focused_a = dispatch_focused_event(root.as_ref(), &key(NamedKey::Enter));
    let after_focused_a = visits.snapshot();
    let focused_b = dispatch_focused_event(root.as_ref(), &key(NamedKey::Enter));
    let after_focused_b = visits.snapshot();
    assert!(focused_a.is_consumed() && focused_b.is_consumed());

    let delta = |a: [u64; 3], b: [u64; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    println!(
        "rows={rows:5} override={overrides_structural:5} focus_first={focus_first:5} \
         settle(vis,evt,str)={:?} key#1={:?} key#2(steady)={:?} \
         dispatch_focused_event#1={:?} #2={:?}",
        delta(before, settle),
        delta(settle, after_first),
        delta(after_first, after_second),
        delta(after_second, after_focused_a),
        delta(after_focused_a, after_focused_b),
    );
}

#[test]
#[ignore = "measurement harness; run explicitly"]
fn structural_walk_count_per_key_event() {
    for rows in [10, 100, 1000] {
        for focus_first in [false, true] {
            run(rows, true, focus_first);
            run(rows, false, focus_first);
        }
    }
}
