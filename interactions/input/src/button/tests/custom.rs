use super::*;
use super::retained::{button_context, child_geometry, decoration_paint, find_element, mounted_elements};
use aimer_decoration::{BorderRadius, Decoration};

// Intentionally neither Clone nor Default: Button must own arbitrary decorations.
struct Surface(Color);

impl Decoration for Surface {
    fn paint(
        &self,
        canvas: &aimer_canvas::Canvas,
        origin: aimer_widget::base::Vec2d,
        size: ResolvedSize,
        _scale: f32,
    ) {
        canvas.fill_color_rect(origin, size, self.0);
    }

    fn background_color(&self) -> Option<Color> {
        Some(self.0)
    }
}

struct InsetSurface;

impl Decoration for InsetSurface {
    fn paint(&self, canvas: &aimer_canvas::Canvas, origin: aimer_widget::base::Vec2d,
        size: ResolvedSize, _scale: f32) {
        canvas.fill_color_rect(origin, size, Color::BLUE);
    }

    fn content_insets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        [3.0; 4]
    }

    fn child_clip_radius(&self, _size: ResolvedSize, _scale: f32) -> Option<[f32; 4]> {
        Some([9.0; 4])
    }
}

#[tokio::test]
async fn custom_decoration_geometry_follows_feedback_at_both_scales() {
    for scale in [1.0, 2.0] {
        let mut ctx = button_context();
        ctx.scale = scale;
        let builds = Rc::new(Cell::new(0));
        let root = Button::new().decoration(Surface(Color::WHITE))
            .hover_decoration(InsetSurface)
            .child(Probe { builds: builds.clone() }).to_element(&ctx);
        root.update(&ctx);
        assert_eq!(child_geometry(root.as_ref(), &ctx), (100.0, 40.0, [0.0; 4]));
        let mut dispatcher = aimer_widget::EventDispatcher::new();
        let pointer = PointerInfo::mouse((10.0, 10.0).into(), PointerButton::Primary);
        let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &ElementEvent::PointerMove(pointer));
        root.rebuild_if_dirty(&ctx);
        assert_eq!(child_geometry(root.as_ref(), &ctx),
            (100.0 - 6.0 * scale, 40.0 - 6.0 * scale, [9.0; 4]));
        assert_eq!(builds.get(), 1);
    }
}

#[tokio::test]
async fn custom_decoration_paints_and_keeps_the_child_on_feedback() {
    let ctx = button_context();
    let builds = Rc::new(Cell::new(0));
    let root = Button::new()
        .decoration(Surface(Color::BLUE))
        .child(Probe { builds: builds.clone() })
        .to_element(&ctx);
    root.update(&ctx);
    let mounted = mounted_elements(root.as_ref());
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::BLUE.into());
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    for event in [ElementEvent::PointerMove(pointer), ElementEvent::PointerDown(pointer),
        ElementEvent::PointerUp(pointer)] {
        let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
        root.rebuild_if_dirty(&ctx);
        assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::BLUE.into());
        assert_eq!(mounted_elements(root.as_ref()), mounted);
    }
    assert_eq!(builds.get(), 1);
}

#[tokio::test]
async fn builder_receives_feedback_without_replacing_input_wrappers() {
    let ctx = button_context();
    let snapshots = Rc::new(std::cell::RefCell::new(Vec::new()));
    let presses = Rc::new(Cell::new(0));
    let root = Button::new()
        .decoration(Surface(Color::BLUE)) // The builder owns all visuals.
        .on_press({
            let presses = presses.clone();
            move || presses.set(presses.get() + 1)
        })
        .build({
            let snapshots = snapshots.clone();
            move |state| {
                snapshots.borrow_mut().push((state.is_hovered, state.is_pressed, state.is_disabled));
                ErrorWidget::new("custom button")
            }
        })
        .to_element(&ctx);
    root.update(&ctx);
    assert!(find_element(root.as_ref(), "Container").is_none());
    let gesture = find_element(root.as_ref(), "GestureDetector").unwrap().id();
    let mouse = find_element(root.as_ref(), "MouseRegion").unwrap().id();
    assert_eq!(&*snapshots.borrow(), &[(false, false, false)]);
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    for (event, expected) in [
        (ElementEvent::PointerMove(pointer), (true, false, false)),
        (ElementEvent::PointerDown(pointer), (true, true, false)),
        (ElementEvent::PointerUp(pointer), (true, false, false)),
    ] {
        let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
        root.rebuild_if_dirty(&ctx);
        assert_eq!(snapshots.borrow().last(), Some(&expected));
        assert_eq!(find_element(root.as_ref(), "GestureDetector").unwrap().id(), gesture);
        assert_eq!(find_element(root.as_ref(), "MouseRegion").unwrap().id(), mouse);
    }
    assert_eq!(presses.get(), 1);
    let builds = snapshots.borrow().len();
    aimer_widget::reset_frame_work_stats();
    for _ in 0..32 {
        let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &ElementEvent::PointerMove(pointer));
        root.rebuild_if_dirty(&ctx);
    }
    assert_eq!(snapshots.borrow().len(), builds);
    assert_eq!(aimer_widget::take_frame_work_stats().state_updates, 0);
    let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &ElementEvent::PointerDown(pointer));
    root.rebuild_if_dirty(&ctx);
    let _ = dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(true, false, false)));
    assert_eq!(presses.get(), 1);
}

#[tokio::test]
async fn button_accepts_light_edge_and_inherits_background_for_contrast() {
    use aimer_cupid::draw_cmd_v2::DrawCommand;
    use aimer_cupid::material::{MaterialKind, MaterialRequest};
    use aimer_container::{Container, SizedBox};
    use aimer_decoration::LightEdge;

    for scale in [1.0, 2.0] {
        let mut ctx = button_context();
        ctx.scale = scale;
        let root = Container::new().color(Color::WHITE)
            .child(Button::new().decoration(vec![BorderRadius::from(12.0).notype(), LightEdge::new().notype()])
                .child(SizedBox::new().width(60).height(24)))
            .to_element(&ctx);
        root.update(&ctx);
        let colors: Vec<_> = aimer_widget::testing::record_retained_paint(root.as_ref(), &ctx)
            .iter().flat_map(|paint| paint.commands.iter())
            .filter_map(|command| match command {
                DrawCommand::DrawCustom { data, .. } => MaterialRequest::decode(data).ok()
                    .filter(|request| request.kind == MaterialKind::LightEdge)
                    .map(|request| request.border_color),
                _ => None,
            }).collect();
        assert_eq!(colors, vec![[0.0, 0.0, 0.0, 1.0]]);
    }
}

#[tokio::test]
async fn custom_state_decorations_follow_precedence_and_keep_the_child() {
    let ctx = button_context();
    let builds = Rc::new(Cell::new(0));
    let button = Button::new()
        .decoration(Surface(Color::WHITE))
        .hover_decoration(BoxDecoration::new().background_color(Color::BLUE))
        .press_decoration(Surface(Color::RED))
        .disable_decoration(Surface(Color::GRAY))
        .child(Probe { builds: builds.clone() });
    let (root, updater) = aimer_widget::StatefulElement::new_with_name(button, &ctx, "Button", None);
    let root = root.boxed();
    root.update(&ctx);
    let gesture = find_element(root.as_ref(), "GestureDetector").unwrap().id();
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::WHITE.into());
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    for (event, color) in [
        (ElementEvent::PointerMove(pointer), Color::BLUE),
        (ElementEvent::PointerDown(pointer), Color::RED),
        (ElementEvent::PointerUp(pointer), Color::BLUE),
    ] {
        let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
        root.rebuild_if_dirty(&ctx);
        assert_eq!(decoration_paint(root.as_ref(), &ctx), color.into());
        assert_eq!(find_element(root.as_ref(), "GestureDetector").unwrap().id(), gesture);
    }
    updater.set_state(|state| state.is_disabled = true);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::GRAY.into());
    assert!(find_element(root.as_ref(), "GestureDetector").is_none());
    updater.set_state(|state| state.is_disabled = false);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::BLUE.into());
    assert_eq!(builds.get(), 1);
}

#[tokio::test]
async fn builder_observes_disabled_and_configuration_updates() {
    let mut ctx = button_context();
    let snapshots = Rc::new(std::cell::RefCell::new(Vec::new()));
    let presses = Rc::new(Cell::new(0));
    let button = Button::new().disabled(true)
        .on_press({
            let presses = presses.clone();
            move || presses.set(presses.get() + 1)
        })
        .on_right_press({
            let presses = presses.clone();
            move || presses.set(presses.get() + 100)
        })
        .build({
            let snapshots = snapshots.clone();
            move |state| {
                snapshots.borrow_mut().push((state.is_hovered, state.is_pressed, state.is_disabled));
                ErrorWidget::new("custom button")
            }
        });
    let (root, updater) = aimer_widget::StatefulElement::new_with_name(button, &ctx, "Button", None);
    let root = root.boxed();
    root.update(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(false, false, true)));
    assert!(find_element(root.as_ref(), "GestureDetector").is_none());
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    for button in [PointerButton::Primary, PointerButton::Secondary] {
        let pointer = PointerInfo::mouse((1.0, 1.0).into(), button);
        for event in [ElementEvent::PointerDown(pointer), ElementEvent::PointerUp(pointer)] {
            let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
        }
    }
    assert_eq!(presses.get(), 0);
    updater.set_state(|state| state.is_disabled = false);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(false, false, false)));
    // The newly enabled input wrappers receive geometry during the frame.
    root.update(&ctx);
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    ctx.cursor_pos = pointer.pos;
    let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &ElementEvent::PointerDown(pointer));
    root.rebuild_if_dirty(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(true, true, false)));
    // A parent refresh while pressed must adopt the live visual updater.
    updater.set_state(|_| {});
    root.rebuild_if_dirty(&ctx);
    root.update(&ctx);
    let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &ElementEvent::PointerUp(pointer));
    root.rebuild_if_dirty(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(true, false, false)));
    assert_eq!(presses.get(), 1);
    let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &ElementEvent::PointerDown(pointer));
    root.rebuild_if_dirty(&ctx);
    updater.set_state(|state| state.is_disabled = true);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(false, false, true)));
    updater.set_state(|state| state.is_disabled = false);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(snapshots.borrow().last(), Some(&(true, false, false)));
}

#[cfg(feature = "portable-guest")]
#[test]
fn portable_button_rejects_custom_appearance_instead_of_dropping_it() {
    let source = portable_source(3);
    let mut ctx = portable_context();
    let error = Button::new().decoration(Surface(Color::BLUE)).child(PortableChild)
        .to_portable_node(&mut ctx, source).unwrap_err();
    assert!(matches!(error, PortableBuildError::UnsupportedProperty {
        widget: "Button", property: "decoration", ..
    }));
    let error = Button::new().build(|_| PortableChild)
        .to_portable_node(&mut ctx, source).unwrap_err();
    assert!(matches!(error, PortableBuildError::UnsupportedProperty {
        widget: "Button", property: "build", ..
    }));
}
