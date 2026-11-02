use super::*;
use super::retained::{button_context, find_element, mounted_elements};
use aimer_decoration::{BackdropFilter, LightEdge};

#[tokio::test]
async fn no_decor_ignores_state_decorations_and_retains_the_child_on_feedback() {
    for before_configuration in [true, false] {
        let ctx = button_context();
        let builds = Rc::new(Cell::new(0));
        let presses = Rc::new(Cell::new(0));
        let mut button = Button::new();
        if before_configuration {
            button = button.no_decor();
        }
        let button = button.decoration(LightEdge::new())
            .hover_decoration(BoxDecoration::new().background_color(Color::BLUE))
            .press_decoration(BackdropFilter::new())
            .disable_decoration(BoxDecoration::new().background_color(Color::RED))
            .on_press({
                let presses = presses.clone();
                move || presses.set(presses.get() + 1)
            }).child(Probe { builds: builds.clone() });
        let button = if before_configuration { button } else { button.no_decor() };
        let root = button.to_element(&ctx);
        root.update(&ctx);
        assert!(find_element(root.as_ref(), "Container").is_none());
        assert!(find_element(root.as_ref(), "GestureDetector").is_some());
        let mounted = mounted_elements(root.as_ref());
        aimer_widget::reset_frame_work_stats();
        let mut dispatcher = aimer_widget::EventDispatcher::new();
        let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
        for event in [ElementEvent::PointerMove(pointer), ElementEvent::PointerDown(pointer),
            ElementEvent::PointerUp(pointer)] {
            let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
            root.rebuild_if_dirty(&ctx);
            assert!(find_element(root.as_ref(), "Container").is_none());
            assert_eq!(mounted_elements(root.as_ref()), mounted);
        }
        assert_eq!(presses.get(), 1);
        assert_eq!(builds.get(), 1);
        assert_eq!(aimer_widget::take_frame_work_stats().state_updates, 0);
    }
}

#[tokio::test]
async fn no_decor_disabled_button_keeps_the_child_without_input_or_background() {
    let ctx = button_context();
    let presses = Rc::new(Cell::new(0));
    let builds = Rc::new(Cell::new(0));
    let root = Button::new().no_decor().disabled(true)
        .on_press({
            let presses = presses.clone();
            move || presses.set(presses.get() + 1)
        }).on_right_press({
            let presses = presses.clone();
            move || presses.set(presses.get() + 1)
        }).child(Probe { builds: builds.clone() }).to_element(&ctx);
    root.update(&ctx);
    assert!(find_element(root.as_ref(), "Container").is_none());
    assert!(find_element(root.as_ref(), "MouseRegion").is_none());
    assert!(find_element(root.as_ref(), "GestureDetector").is_none());
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    for button in [PointerButton::Primary, PointerButton::Secondary] {
        let pointer = PointerInfo::mouse((1.0, 1.0).into(), button);
        for event in [ElementEvent::PointerDown(pointer), ElementEvent::PointerUp(pointer)] {
            let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
        }
    }
    assert_eq!(presses.get(), 0);
    assert_eq!(builds.get(), 1);
}

#[cfg(feature = "portable-guest")]
#[test]
fn portable_button_rejects_no_decor_instead_of_adding_a_decoration() {
    let mut ctx = portable_context();
    let error = Button::new().no_decor().child(PortableChild)
        .to_portable_node(&mut ctx, portable_source(4)).unwrap_err();
    assert!(matches!(error, PortableBuildError::UnsupportedProperty {
        widget: "Button", property: "no_decor", ..
    }));
}
