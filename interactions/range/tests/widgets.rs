use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_container::Container;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_range::{RangeSlider, RangeThumb, Slider, SliderThumb, SliderTrail};
use aimer_style::{LayoutSpacing, Spacing};
use aimer_widget::base::{BuildContext, Vec2d, WindowHandle};
use aimer_widget::{
    Drawable, ErrorWidget, EventDispatcher, LayoutElement, Rebuildable, State, StatefulElement,
    StatefulWidget, Widget,
};

#[test]
fn slider_builder_constructs_a_standalone_value_model() {
    let slider = Slider::new().range(0.0..100.0).step(10.0).value(20.0);

    assert_eq!(slider.current_value(), 20.0);
    assert_eq!(slider.min(), 0.0);
    assert_eq!(slider.max(), 100.0);
    assert_eq!(slider.step_value(), 10.0);
}

#[test]
fn range_slider_builder_accepts_integer_ranges() {
    let slider = RangeSlider::new()
        .range(0_u32..100_u32)
        .step(10_u32)
        .value(20_u32..70_u32);

    assert_eq!(slider.current_values(), 20_u32..70_u32);
    assert_eq!(slider.min(), 0_u32);
    assert_eq!(slider.max(), 100_u32);
}

#[tokio::test]
async fn range_controls_keep_composable_visual_slots_in_the_element_tree() {
    let ctx = context();
    let slider = Slider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(20.0)
        .track(ErrorWidget::new("track"))
        .trail(ErrorWidget::new("trail"))
        .thumb(ErrorWidget::new("thumb"))
        .to_element(&ctx);
    slider.layout(&ctx);

    let mut slot_names = Vec::new();
    slider.visit_children(&mut |surface| {
        surface.visit_children(&mut |slot| {
            slot.visit_children(&mut |visual| slot_names.push(visual.debug_name()));
        });
    });
    assert_eq!(slot_names, ["ErrorWidget", "ErrorWidget", "ErrorWidget"]);

    let range_slider = RangeSlider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(20.0..70.0)
        .track(ErrorWidget::new("track"))
        .trail(ErrorWidget::new("trail"))
        .thumbs(ErrorWidget::new("lower"), ErrorWidget::new("upper"))
        .to_element(&ctx);
    range_slider.layout(&ctx);

    let mut range_slot_names = Vec::new();
    range_slider.visit_children(&mut |surface| {
        surface.visit_children(&mut |slot| {
            slot.visit_children(&mut |visual| range_slot_names.push(visual.debug_name()));
        });
    });
    assert_eq!(
        range_slot_names,
        ["ErrorWidget", "ErrorWidget", "ErrorWidget", "ErrorWidget"]
    );
}

#[test]
fn slider_visual_parts_are_standalone_widgets() {
    fn assert_widget(_: impl Widget) {}

    assert_widget(SliderTrail::new());
    assert_widget(SliderThumb::new());
}

#[tokio::test]
async fn slider_materializes_default_trail_and_thumb_widgets() {
    let ctx = context();
    let element = Slider::<f64>::new().to_element(&ctx);
    element.layout(&ctx);

    let mut slot_names = Vec::new();
    element.visit_children(&mut |surface| {
        surface.visit_children(&mut |slot| {
            slot.visit_children(&mut |visual| slot_names.push(visual.debug_name()));
        });
    });

    assert_eq!(slot_names, ["SliderTrack", "SliderTrail", "SliderThumb"]);

    let range_element = RangeSlider::<f64>::new().to_element(&ctx);
    range_element.layout(&ctx);
    let mut range_slot_names = Vec::new();
    range_element.visit_children(&mut |surface| {
        surface.visit_children(&mut |slot| {
            slot.visit_children(&mut |visual| range_slot_names.push(visual.debug_name()));
        });
    });
    assert_eq!(
        range_slot_names,
        ["SliderTrack", "SliderTrail", "SliderThumb", "SliderThumb"]
    );
}

fn context() -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    let mut ctx = BuildContext::new(
        canvas,
        ResolvedSize {
            width: 320.0,
            height: 120.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 320.0,
        max_height: 120.0,
    };
    ctx
}

#[tokio::test]
async fn slider_widget_dispatches_pointer_drag_and_keyboard_increment() {
    let ctx = context();
    let values = Rc::new(RefCell::new(Vec::<f64>::new()));
    let observed = Rc::clone(&values);
    let element = Slider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(20.0)
        .width(200.0)
        .on_change(move |value| observed.borrow_mut().push(value))
        .to_element(&ctx);

    let size = element.layout(&ctx);
    assert_eq!(size.width, 200.0);
    assert!(size.height > 0.0);
    element.update(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let down = PointerInfo::mouse(Vec2d { x: 20.0, y: 20.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(element.as_ref(), down.pos, &ElementEvent::PointerDown(down))
            .is_consumed()
    );

    let moved = down.at(Vec2d { x: 140.0, y: 20.0 });
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                moved.pos,
                &ElementEvent::PointerMove(moved)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), moved.pos, &ElementEvent::PointerUp(moved))
            .is_consumed()
    );

    let key = ElementEvent::KeyInput {
        key: NamedKey::ArrowRight,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    };
    assert!(
        dispatcher
            .dispatch(element.as_ref(), moved.pos, &key)
            .is_consumed()
    );

    let values = values.borrow();
    assert!(
        values
            .iter()
            .any(|value| (*value - 10.0).abs() < f64::EPSILON)
    );
    assert!(
        values
            .iter()
            .any(|value| (*value - 70.0).abs() < f64::EPSILON)
    );
    assert!(
        values
            .iter()
            .any(|value| (*value - 80.0).abs() < f64::EPSILON)
    );
}

#[tokio::test]
async fn slider_widget_dispatches_pointer_drag_when_nested_in_a_padded_container() {
    let ctx = context();
    let values = Rc::new(RefCell::new(Vec::<f64>::new()));
    let observed = Rc::clone(&values);
    let slider = Slider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(20.0)
        .width(200.0)
        .on_change(move |value| observed.borrow_mut().push(value));
    let element = Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(20)))
        .child(slider)
        .to_element(&ctx);

    element.layout(&ctx);
    element.update(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let down = PointerInfo::mouse(Vec2d { x: 40.0, y: 40.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(element.as_ref(), down.pos, &ElementEvent::PointerDown(down))
            .is_consumed()
    );
    let moved = down.at(Vec2d { x: 160.0, y: 40.0 });
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                moved.pos,
                &ElementEvent::PointerMove(moved)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), moved.pos, &ElementEvent::PointerUp(moved))
            .is_consumed()
    );

    assert!(values.borrow().iter().any(|value| *value == 70.0));
}

#[tokio::test]
async fn range_slider_widget_dispatches_the_nearest_thumb_and_keyboard_increment() {
    let ctx = context();
    let values = Rc::new(RefCell::new(Vec::<(f64, f64)>::new()));
    let observed = Rc::clone(&values);
    let element = RangeSlider::new()
        .range(0.0..100.0)
        .step(10.0)
        .values(20.0..70.0)
        .width(200.0)
        .on_change(move |value| observed.borrow_mut().push(value))
        .to_element(&ctx);
    let size = element.layout(&ctx);
    assert_eq!(size.width, 200.0);
    element.update(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let down = PointerInfo::mouse(Vec2d { x: 40.0, y: 20.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(element.as_ref(), down.pos, &ElementEvent::PointerDown(down))
            .is_consumed()
    );
    let moved = down.at(Vec2d { x: 100.0, y: 20.0 });
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                moved.pos,
                &ElementEvent::PointerMove(moved)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), moved.pos, &ElementEvent::PointerUp(moved))
            .is_consumed()
    );

    let key = ElementEvent::KeyInput {
        key: NamedKey::ArrowRight,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    };
    assert!(
        dispatcher
            .dispatch(element.as_ref(), moved.pos, &key)
            .is_consumed()
    );

    let values = values.borrow();
    assert!(
        values
            .iter()
            .any(|(lower, upper)| (*lower - 50.0).abs() < f64::EPSILON
                && (*upper - 70.0).abs() < f64::EPSILON)
    );
    assert!(
        values
            .iter()
            .any(|(lower, upper)| (*lower - 60.0).abs() < f64::EPSILON
                && (*upper - 70.0).abs() < f64::EPSILON)
    );
}

#[tokio::test]
async fn slider_state_retains_runtime_press_while_adopting_a_controlled_value() {
    let ctx = context();
    let (element, updater) = StatefulElement::new(
        Slider::new()
            .range(0.0..100.0)
            .step(10.0)
            .value(20.0)
            .width(200.0),
        &ctx,
    );
    let element = element.boxed();
    element.layout(&ctx);
    element.update(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let pointer = PointerInfo::mouse(Vec2d { x: 40.0, y: 20.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                pointer.pos,
                &ElementEvent::PointerDown(pointer)
            )
            .is_consumed()
    );
    assert!(updater.read_state().is_pressed());
    assert!(updater.read_state().is_focused());

    let moved = pointer.at(Vec2d { x: 100.0, y: 20.0 });
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                moved.pos,
                &ElementEvent::PointerMove(moved)
            )
            .is_consumed()
    );
    assert_eq!(updater.read_state().current_value(), 50.0);

    updater.set_state(|state| {
        state.adopt_config_from(
            Slider::new()
                .range(0.0..100.0)
                .step(10.0)
                .value(80.0)
                .width(200.0)
                .create_state(),
        )
    });
    element.rebuild_if_dirty(&ctx);
    assert_eq!(updater.read_state().current_value(), 80.0);
    assert!(updater.read_state().is_pressed());
}

#[tokio::test]
async fn range_slider_state_retains_the_selected_thumb_while_adopting_values() {
    let ctx = context();
    let (element, updater) = StatefulElement::new(
        RangeSlider::new()
            .range(0.0..100.0)
            .step(10.0)
            .values(20.0..70.0)
            .width(200.0),
        &ctx,
    );
    let element = element.boxed();
    element.layout(&ctx);
    element.update(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let pointer = PointerInfo::mouse(Vec2d { x: 40.0, y: 20.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                pointer.pos,
                &ElementEvent::PointerDown(pointer)
            )
            .is_consumed()
    );
    assert_eq!(updater.read_state().active_thumb(), Some(RangeThumb::Lower));

    let moved = pointer.at(Vec2d { x: 100.0, y: 20.0 });
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                moved.pos,
                &ElementEvent::PointerMove(moved)
            )
            .is_consumed()
    );
    assert_eq!(updater.read_state().current_values(), 50.0..70.0);

    updater.set_state(|state| {
        state.adopt_config_from(
            RangeSlider::new()
                .range(0.0..100.0)
                .step(10.0)
                .values(30.0..80.0)
                .width(200.0)
                .create_state(),
        )
    });
    element.rebuild_if_dirty(&ctx);
    assert_eq!(updater.read_state().current_values(), 30.0..80.0);
    assert_eq!(updater.read_state().active_thumb(), Some(RangeThumb::Lower));
    assert!(updater.read_state().is_pressed());
}

/// A slider owns the gesture from the press: its thumb follows the finger at once.
/// An ancestor that arbitrates touches, such as a navigator that pops on a sideways
/// swipe, must be able to tell, so the finger is claimed.
#[tokio::test]
async fn a_slider_claims_the_finger_that_presses_it() {
    let ctx = context();
    let slider = Slider::new().range(0.0..100.0).step(10.0).value(20.0).width(200.0).to_element(&ctx);
    slider.layout(&ctx);
    slider.update(&ctx);
    let finger = aimer_widget::PointerKey::new(aimer_events::pointer::PointerSource::Touch, 5);
    let down = PointerInfo::touch(Vec2d { x: 20.0, y: 20.0 }, 5);

    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(slider.as_ref(), down.pos, &ElementEvent::PointerDown(down));

    assert!(aimer_widget::is_pointer_claimed(finger));
    aimer_widget::release_pointer(finger);
}

#[tokio::test]
async fn a_range_slider_claims_the_finger_that_presses_it() {
    let ctx = context();
    let slider = RangeSlider::new().range(0.0..100.0).step(10.0).values(20.0..70.0).width(200.0).to_element(&ctx);
    slider.layout(&ctx);
    slider.update(&ctx);
    let finger = aimer_widget::PointerKey::new(aimer_events::pointer::PointerSource::Touch, 6);
    let down = PointerInfo::touch(Vec2d { x: 40.0, y: 20.0 }, 6);

    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(slider.as_ref(), down.pos, &ElementEvent::PointerDown(down));

    assert!(aimer_widget::is_pointer_claimed(finger));
    aimer_widget::release_pointer(finger);
}

#[tokio::test]
async fn a_disabled_slider_leaves_the_finger_unclaimed() {
    let ctx = context();
    let slider = Slider::new().range(0.0..100.0).value(20.0).width(200.0).disabled(true).to_element(&ctx);
    slider.layout(&ctx);
    slider.update(&ctx);
    let finger = aimer_widget::PointerKey::new(aimer_events::pointer::PointerSource::Touch, 7);
    let down = PointerInfo::touch(Vec2d { x: 20.0, y: 20.0 }, 7);

    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(slider.as_ref(), down.pos, &ElementEvent::PointerDown(down));

    assert!(!aimer_widget::is_pointer_claimed(finger), "it does nothing with the touch, so it keeps nothing from the page");
}

#[tokio::test]
async fn disabled_slider_does_not_capture_or_propose_input() {
    let ctx = context();
    let values = Rc::new(RefCell::new(Vec::<f64>::new()));
    let observed = Rc::clone(&values);
    let element = Slider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(20.0)
        .width(200.0)
        .disabled(true)
        .on_change(move |value| observed.borrow_mut().push(value))
        .to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let pointer = PointerInfo::mouse(Vec2d { x: 100.0, y: 20.0 }, PointerButton::Primary);
    assert!(
        !dispatcher
            .dispatch(
                element.as_ref(),
                pointer.pos,
                &ElementEvent::PointerDown(pointer)
            )
            .is_consumed()
    );
    let key = ElementEvent::KeyInput {
        key: NamedKey::ArrowRight,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    };
    assert!(
        !dispatcher
            .dispatch(element.as_ref(), pointer.pos, &key)
            .is_consumed()
    );
    assert!(values.borrow().is_empty());
}

/// A build context with room for a 200 px slider.
fn roomy_context() -> BuildContext<'static> {
    let canvas = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
    let mut ctx = BuildContext::new(
        aimer_canvas::FrameCanvas::new(canvas),
        ResolvedSize {
            width: 320.0,
            height: 120.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 320.0,
        max_height: 120.0,
    };
    ctx
}

/// The visual slots of a slider surface, in paint order (track, trail, thumbs),
/// placed the way the render tree sync places them: the surface publishes where
/// each part sits when the tree asks for its bounds.
fn visual_slots<'a>(
    element: &'a aimer_widget::AnyElement,
    ctx: &BuildContext<'_>,
) -> Vec<&'a dyn aimer_widget::Element> {
    let mut slots = Vec::new();
    element.visit_children(&mut |surface| {
        let _ = surface.retained_v2_bounds(ctx);
        surface.visit_children(&mut |slot| slots.push(slot));
    });
    slots
}

/// The deepest first child of `element`: the leaf that records the paint when a
/// part is a chain of wrappers.
fn innermost(element: &dyn aimer_widget::Element) -> &dyn aimer_widget::Element {
    let mut inner = None;
    element.visit_children(&mut |child| {
        if inner.is_none() {
            inner = Some(child);
        }
    });
    inner.map_or(element, innermost)
}

/// How many rectangles `leaf` records in its own retained list.
fn recorded_fills(leaf: &dyn aimer_widget::Element, ctx: &BuildContext<'_>) -> usize {
    let leaf = innermost(leaf);
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 320.0, 120.0))
        .unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(leaf.can_paint_local_v2(ctx));
        leaf.paint_local_v2(ctx);
    });
    tree.draw_list_snapshot(root)
        .unwrap()
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { .. }
            )
        })
        .count()
}

/// The horizontal extent of one visual slot: where layout put it, and how wide
/// its content is.
fn slot_extent(slot: &dyn aimer_widget::Element, ctx: &BuildContext<'_>) -> (f32, f32) {
    let x = slot.pos().expect("a slider slot reports where it sits").x;
    (x, slot.computed_size(ctx).width)
}

#[tokio::test]
async fn slider_paints_track_active_segment_and_thumb() {
    let ctx = roomy_context();
    let element = Slider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(50.0)
        .width(200.0)
        .to_element(&ctx);
    element.layout(&ctx);

    let slots = visual_slots(&element, &ctx);
    assert_eq!(slots.len(), 3, "track, active segment and thumb");
    for slot in &slots {
        let mut leaves = 0;
        slot.visit_children(&mut |leaf| {
            leaves += 1;
            assert_eq!(
                recorded_fills(leaf, &ctx),
                1,
                "each part records one rectangle"
            );
        });
        assert_eq!(leaves, 1);
    }
    assert!(
        slots[1].retained_clip(&ctx).is_some(),
        "the active segment is clipped to the value"
    );
}

#[tokio::test]
async fn slider_minimum_thumb_stays_inside_the_visual_bounds() {
    let ctx = roomy_context();
    for value in [0.0, 100.0] {
        let element = Slider::new()
            .range(0.0..100.0)
            .step(10.0)
            .value(value)
            .width(200.0)
            .to_element(&ctx);
        let size = element.layout(&ctx);

        let slots = visual_slots(&element, &ctx);
        let (x, width) = slot_extent(slots[2], &ctx);
        assert!(x >= 0.0, "value {value}: thumb starts at {x}");
        assert!(
            x + width <= size.width,
            "value {value}: thumb ends at {}",
            x + width
        );
    }
}

#[tokio::test]
async fn range_slider_endpoint_thumbs_stay_inside_the_visual_bounds() {
    let ctx = roomy_context();
    let element = RangeSlider::new()
        .range(0.0..100.0)
        .step(10.0)
        .values(0.0..100.0)
        .width(200.0)
        .to_element(&ctx);
    let size = element.layout(&ctx);

    let slots = visual_slots(&element, &ctx);
    assert_eq!(slots.len(), 4, "track, active segment and two thumbs");
    for thumb in &slots[2..] {
        let (x, width) = slot_extent(*thumb, &ctx);
        assert!(x >= 0.0);
        assert!(x + width <= size.width);
    }
}

#[tokio::test]
async fn zero_width_slider_is_layout_safe_and_not_hit_testable() {
    let ctx = context();
    let element = Slider::new()
        .range(0.0..100.0)
        .step(10.0)
        .value(50.0)
        .width(0.0)
        .height(0.0)
        .to_element(&ctx);
    assert_eq!(element.layout(&ctx), ResolvedSize::default());

    let pointer = PointerInfo::mouse(Vec2d::default(), PointerButton::Primary);
    let mut dispatcher = EventDispatcher::new();
    assert!(
        !dispatcher
            .dispatch(
                element.as_ref(),
                pointer.pos,
                &ElementEvent::PointerDown(pointer)
            )
            .is_consumed()
    );
}
