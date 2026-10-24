use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::draw_cmd_v2::DrawCommand;
use aimer_cupid::svg::SvgScene;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{
    AimerApp, AnyElement, BuildContext, Drawable, Element, EventElement, HeadlessOptions,
    LayoutElement, Rebuildable, ResolvedSize, Vec2d, VisitorElement,
};

use super::*;

#[derive(Default)]
struct Capture {
    labels: Vec<String>,
    scenes: Vec<Arc<SvgScene>>,
}

struct Captured<W> {
    child: W,
    capture: Rc<RefCell<Capture>>,
}
impl<W: Widget> aimer::PortableWidget for Captured<W> {}
impl<W: Widget> Widget for Captured<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        CapturedElement { child: self.child.to_element(ctx), capture: self.capture }.boxed()
    }
}
struct CapturedElement {
    child: AnyElement,
    capture: Rc<RefCell<Capture>>,
}
impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str { "CapturedLucideGallery" }
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
        let paints = aimer_widget::testing::record_retained_paint(self.child.as_ref(), ctx);
        let mut capture = Capture::default();
        for command in paints.iter().flat_map(|paint| paint.commands.iter()) {
            match command {
                DrawCommand::DrawText { text, .. } if !text.ends_with(" icons") => {
                    capture.labels.push(text.to_string());
                }
                DrawCommand::Svg { scene, .. } => capture.scenes.push(scene.clone()),
                _ => {}
            }
        }
        *self.capture.borrow_mut() = capture;
    }
}

fn scene(entries: &'static [Entry]) -> (
    aimer::quiver::HeadlessAimerApp<impl Widget>, ScrollController, Rc<RefCell<Capture>>,
) {
    let controller = ScrollController::new();
    let capture = Rc::new(RefCell::new(Capture::default()));
    let mut app = AimerApp::start_headless_with(
        Captured { child: gallery(entries, theme::app_theme(), controller.clone()), capture: capture.clone() },
        HeadlessOptions { size: PhysicalSize::new(960, 480), scale_factor: 1.0 },
    );
    app.pump_frames(4);
    (app, controller, capture)
}

#[test]
fn gallery_scrolls_to_first_middle_and_last_labels_without_building_every_icon() {
    let (mut app, controller, capture) = scene(Icons::ALL);
    for index in [0, Icons::ALL.len() / 2, Icons::ALL.len() - 1] {
        controller.jump_to(Vec2d { x: 0.0, y: (index / COLUMNS) as f32 * ROW_EXTENT });
        app.pump_frames(4);
        app.render_frame();
        let capture = capture.borrow();
        assert!(capture.labels.iter().any(|label| label == Icons::ALL[index].0),
            "missing {} among {:?}", Icons::ALL[index].0, capture.labels);
        assert!(capture.scenes.len() < 60, "only viewport/overscan icons should be materialized");
        assert_eq!(capture.labels.len(), capture.scenes.len());
        for (label, scene) in capture.labels.iter().zip(&capture.scenes) {
            let (_, icon) = Icons::ALL.iter().find(|(name, _)| *name == label).unwrap();
            assert!(Arc::ptr_eq(scene, icon.document().scene()), "wrong icon beside {label}");
        }
    }
}

#[test]
fn gallery_partial_final_row_keeps_each_icon_and_label_once() {
    static ENTRIES: &[Entry] = &[
        ("house", Icons::HOUSE), ("search", Icons::SEARCH), ("circle", Icons::CIRCLE),
        ("chart-scatter", Icons::CHART_SCATTER), ("square", Icons::SQUARE),
    ];
    let (_app, _controller, capture) = scene(ENTRIES);
    let capture = capture.borrow();
    assert_eq!(capture.labels, ["house", "search", "circle", "chart-scatter", "square"]);
    assert_eq!(capture.scenes.len(), 5);
    for (scene, (_, icon)) in capture.scenes.iter().zip(ENTRIES) {
        assert!(Arc::ptr_eq(scene, icon.document().scene()));
    }
}
