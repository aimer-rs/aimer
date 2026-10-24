use super::*;

use aimer::HeadlessOptions;
use aimer_cupid::draw_cmd_v2::DrawCommand;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement};
use aimer_widget::components::element::ElementId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn transition_example() -> ExampleId {
    EXAMPLES.iter().copied()
        .find(|example| example.key() == "navigation-transitions")
        .expect("Navigation transitions must be selectable in Jaime")
}

#[test]
fn navigation_transitions_is_registered_and_renders_with_retained_paint() {
    let example = transition_example();
    assert!(EXAMPLE_CATEGORIES.iter().any(|category| {
        category.label == "Navigation" && category.examples.contains(&example)
    }));
    let mut app = AimerApp::start_headless_with(
        theme::provide(build_example(example, theme::app_theme())),
        HeadlessOptions { size: PhysicalSize::new(900, 800), scale_factor: 1.0 },
    );
    app.pump_frames(3);
    let census = app.paint_source_census().expect("a mounted transition page");
    assert!(census.declined_paint.is_empty());
    assert!(census.drawn_unmapped.is_empty());
    assert!(census.invalid_bounds.is_empty());
    assert!(census.sync_error.is_none());
}

#[derive(Debug)]
struct Label {
    text: String,
    id: ElementId,
    center: Vec2d,
}

#[derive(Default)]
struct Snapshot {
    labels: Vec<Label>,
    click_positions: HashMap<(String, usize), usize>,
}

impl Snapshot {
    fn label(&self, text: &str, index: usize) -> &Label {
        self.labels.iter().filter(|label| label.text == text).nth(index)
            .unwrap_or_else(|| panic!("missing active label {text:?} at index {index}: {:?}", self.labels))
    }

    fn count(&self, text: &str) -> usize {
        self.labels.iter().filter(|label| label.text == text).count()
    }
}

struct Captured {
    snapshot: Rc<RefCell<Snapshot>>,
    duration: std::time::Duration,
}
impl aimer::PortableWidget for Captured {}
impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        CapturedElement {
            child: crate::navigation_transitions_example::NavigationTransitionsExample::new()
                .duration(self.duration).to_element(ctx),
            snapshot: self.snapshot,
        }.boxed()
    }
}

struct CapturedElement {
    child: AnyElement,
    snapshot: Rc<RefCell<Snapshot>>,
}

impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str { "CapturedNavigationTransitions" }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl EventElement for CapturedElement {}
impl Rebuildable for CapturedElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) { self.child.rebuild_if_dirty(ctx); }
}
impl LayoutElement for CapturedElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize { self.child.computed_size(ctx) }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize { self.child.content_size(ctx) }
}
impl Drawable for CapturedElement {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool { true }
    fn paint_local_v2(&self, ctx: &BuildContext) { aimer::canvas::Canvas::of(ctx).finish(); }
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
        let mut labels = Vec::new();
        collect_labels(self.child.as_ref(), ctx, &mut labels, None);
        self.snapshot.borrow_mut().labels = labels;
    }
}

fn collect_labels(element: &dyn Element, ctx: &BuildContext, labels: &mut Vec<Label>, inherited: Option<(Vec2d, Vec2d)>) {
    let bounds = element.pos_start_end().or(inherited);
    if element.debug_name() == "RawTextWidget" {
        let paints = aimer_widget::testing::record_retained_paint(element, ctx);
        for paint in &paints {
            for command in paint.commands.iter() {
                if let DrawCommand::DrawText { text, .. } = command {
                    let (start, end) = bounds.expect("laid-out text bounds");
                    labels.push(Label {
                        text: text.to_string(), id: element.id(),
                        center: Vec2d { x: (start.x + end.x) * 0.5, y: (start.y + end.y) * 0.5 },
                    });
                }
            }
        }
    }
    // Outgoing pages may still paint, but only the incoming page can be clicked.
    let mut visit = |child: &dyn Element| collect_labels(child, ctx, labels, bounds);
    if element.debug_name() == "NavigationTransitionHost" {
        element.event_children(&mut visit);
    } else {
        element.visit_children(&mut visit);
    }
}

fn click<W: Widget + 'static>(
    app: &mut aimer::quiver::HeadlessAimerApp<W>,
    snapshot: &RefCell<Snapshot>, label: &str, index: usize,
) {
    let center = {
        let mut snapshot = snapshot.borrow_mut();
        let mut center = snapshot.label(label, index).center;
        // Separate single clicks spatially, beyond the double-tap slop, so
        // this interaction test needs no wall-clock sleeps or timing guesses.
        let position = snapshot.click_positions.entry((label.to_owned(), index)).or_default();
        center.x += (*position % 3) as f32 * 7.0 - 7.0;
        *position += 1;
        center
    };
    let scale = app.scale_factor();
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(center.x as f64 * scale, center.y as f64 * scale),
    });
    app.render_frame();
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(), state, button: MouseButton::Left,
        });
        app.render_frame();
    }
}

#[test]
fn navigation_transition_previews_support_push_pop_reset_and_opt_out() {
    for scale in [1.0, 2.0] {
        let snapshot = Rc::new(RefCell::new(Snapshot::default()));
        // Fixed zero-duration effects let every button be clicked without
        // depending on its animated position or waiting on the wall clock.
        let mut app = AimerApp::start_headless_with(theme::provide(Captured {
            snapshot: snapshot.clone(), duration: std::time::Duration::ZERO,
        }),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (2600.0 * scale) as u32),
                scale_factor: scale,
            });
        app.pump_frames(3);
        for title in ["CrossFadeTransition", "ZoomTransition", "ViewLevelTransitions", "DynamicTransitions"] {
            assert_eq!(snapshot.borrow().count(title), 1);
        }
        for index in 0..4 {
            click(&mut app, &snapshot, "Back", index);
            assert_eq!(snapshot.borrow().count("Home · level 0"), 4);
            click(&mut app, &snapshot, "Details", index);
            assert_eq!(snapshot.borrow().count("Details · level 1"), 1);
            // The current destination is disabled, so it cannot add history.
            click(&mut app, &snapshot, "Details", index);
            click(&mut app, &snapshot, "Back", index);
            assert_eq!(snapshot.borrow().count("Home · level 0"), 4);
            click(&mut app, &snapshot, "Details", index);
            click(&mut app, &snapshot, "Inspect", index);
            assert_eq!(snapshot.borrow().count("Inspect · level 2"), 1);
            click(&mut app, &snapshot, "Back", index);
            assert_eq!(snapshot.borrow().count("Details · level 1"), 1);
            click(&mut app, &snapshot, "Instant", index);
            assert_eq!(snapshot.borrow().count("Instant · animation disabled"), 1);
            click(&mut app, &snapshot, "Back", index);
            assert_eq!(snapshot.borrow().count("Details · level 1"), 1);
            click(&mut app, &snapshot, "Reset", index);
            assert_eq!(snapshot.borrow().count("Home · level 0"), 4);
        }
    }
}

#[test]
fn navigation_transition_shells_keep_frames_and_branch_history() {
    let snapshot = Rc::new(RefCell::new(Snapshot::default()));
    let mut app = AimerApp::start_headless_with(theme::provide(Captured {
        snapshot: snapshot.clone(), duration: std::time::Duration::ZERO,
    }),
        HeadlessOptions { size: PhysicalSize::new(900, 2600), scale_factor: 1.0 });
    app.pump_frames(3);
    let shell_frame = snapshot.borrow().label("Shell frame · stays in place", 0).id;
    let branch_frame = snapshot.borrow().label("Stateful shell frame · separate branch histories", 0).id;
    click(&mut app, &snapshot, "Reports", 0);
    assert_eq!(snapshot.borrow().count("Reports outlet"), 1);
    click(&mut app, &snapshot, "Overview", 0);
    assert_eq!(snapshot.borrow().count("Overview outlet"), 1);
    click(&mut app, &snapshot, "Shell back", 0);
    assert_eq!(snapshot.borrow().count("Reports outlet"), 1);
    click(&mut app, &snapshot, "Shell back", 0);
    click(&mut app, &snapshot, "Shell back", 0);
    assert_eq!(snapshot.borrow().count("Overview outlet"), 1);
    assert_eq!(snapshot.borrow().label("Shell frame · stays in place", 0).id, shell_frame);

    click(&mut app, &snapshot, "Branch back", 0);
    assert_eq!(snapshot.borrow().count("Feed · page 0"), 1);
    click(&mut app, &snapshot, "Next page", 0);
    assert_eq!(snapshot.borrow().count("Feed · page 1"), 1);
    click(&mut app, &snapshot, "Profile", 0);
    assert_eq!(snapshot.borrow().count("Profile · page 0"), 1);
    click(&mut app, &snapshot, "Next page", 0);
    click(&mut app, &snapshot, "Feed", 0);
    assert_eq!(snapshot.borrow().count("Feed · page 1"), 1);
    click(&mut app, &snapshot, "Branch back", 0);
    assert_eq!(snapshot.borrow().count("Feed · page 0"), 1);
    click(&mut app, &snapshot, "Profile", 0);
    assert_eq!(snapshot.borrow().count("Profile · page 1"), 1);
    click(&mut app, &snapshot, "Branch back", 0);
    assert_eq!(snapshot.borrow().count("Profile · page 0"), 1);
    assert_eq!(snapshot.borrow().label("Stateful shell frame · separate branch histories", 0).id, branch_frame);
}

#[test]
fn navigation_transition_effects_navigate_with_retained_paint_at_both_scales() {
    for scale in [1.0, 2.0] {
        let snapshot = Rc::new(RefCell::new(Snapshot::default()));
        let mut app = AimerApp::start_headless_with(theme::provide(Captured {
            snapshot: snapshot.clone(), duration: std::time::Duration::from_millis(450),
        }), HeadlessOptions {
            size: PhysicalSize::new((900.0 * scale) as u32, (2600.0 * scale) as u32),
            scale_factor: scale,
        });
        app.pump_frames(3);
        // Each first push starts from a stationary page. No assertions depend
        // on elapsed time or on a moving button's intermediate position.
        for index in 0..4 {
            click(&mut app, &snapshot, "Details", index);
            assert_eq!(snapshot.borrow().count("Details · level 1"), index + 1);
            let census = app.paint_source_census().expect("a navigated transition page");
            assert!(census.declined_paint.is_empty());
            assert!(census.drawn_unmapped.is_empty());
            assert!(census.invalid_bounds.is_empty());
            assert!(census.sync_error.is_none());
        }
    }
}
