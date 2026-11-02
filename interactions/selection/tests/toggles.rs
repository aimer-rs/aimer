//! Checkbox, switch, and radio through their public API and the real event
//! dispatcher.

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_container::SizedBox;
use aimer_events::element::NamedKey;
use aimer_flex::Row;
use aimer_selection::{Checkbox, CheckboxValue, Radio, SemanticRole, Switch};
use aimer_text::Text;
use aimer_widget::{LayoutElement, PortableWidget, State, StatefulWidget, Widget};
use support::{Driver, context};

fn counter() -> (Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    (Rc::clone(&calls), calls)
}

#[tokio::test]
async fn toggle_controls_are_named_widgets() {
    let ctx = context();
    assert_eq!(Widget::debug_name(&Checkbox::new()), "Checkbox");
    assert_eq!(Checkbox::new().to_element(&ctx).debug_name(), "Checkbox");
    assert_eq!(Switch::new().to_element(&ctx).debug_name(), "Switch");
    assert_eq!(Radio::new(1).to_element(&ctx).debug_name(), "Radio");
}

#[tokio::test]
async fn toggle_controls_use_the_default_portable_contract() {
    fn assert_portable<T: PortableWidget>() {}
    assert_portable::<Checkbox>();
    assert_portable::<Switch>();
    assert_portable::<Radio<&'static str>>();
}

#[tokio::test]
async fn disabled_toggle_controls_still_build() {
    let ctx = context();
    Checkbox::new().disabled(true).label("Locked").to_element(&ctx);
    Switch::new().disabled(true).to_element(&ctx);
    Radio::new(1).disabled(true).to_element(&ctx);
}

#[tokio::test]
async fn checkbox_hit_target_meets_density_minimum() {
    let ctx = context();
    let size = Checkbox::new().label("Hit").to_element(&ctx).layout(&ctx);
    assert!(
        size.width >= 44.0 && size.height >= 44.0,
        "choice controls must honor the style density minimum target, got {size:?}"
    );
}

#[tokio::test]
async fn a_checkbox_tap_proposes_the_toggled_value_and_keeps_its_own() {
    let ctx = context();
    let proposed = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&proposed);
    let element = Checkbox::new()
        .label("Accept")
        .on_change(move |value| observed.borrow_mut().push(value))
        .to_element(&ctx);
    let mut driver = Driver::new(&ctx, element);

    assert!(driver.pointer_down(8.0, 8.0).is_consumed());
    assert!(driver.pointer_up(8.0, 8.0).is_consumed());

    assert_eq!(proposed.borrow().as_slice(), &[CheckboxValue::Checked]);
}

#[tokio::test]
async fn an_indeterminate_checkbox_proposes_checked() {
    let ctx = context();
    let proposed = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&proposed);
    let element = Checkbox::new()
        .value(CheckboxValue::Indeterminate)
        .on_change(move |value| observed.borrow_mut().push(value))
        .to_element(&ctx);
    let mut driver = Driver::new(&ctx, element);

    driver.click(8.0, 8.0);

    assert_eq!(proposed.borrow().as_slice(), &[CheckboxValue::Checked]);
}

#[tokio::test]
async fn a_switch_tap_proposes_the_opposite_value() {
    let ctx = context();
    let proposed = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&proposed);
    let element = Switch::new()
        .value(true)
        .on_change(move |value| observed.borrow_mut().push(value))
        .to_element(&ctx);
    let mut driver = Driver::new(&ctx, element);

    driver.click(8.0, 8.0);

    assert_eq!(proposed.borrow().as_slice(), &[false]);
}

#[tokio::test]
async fn an_unselected_radio_proposes_its_value_and_a_selected_one_does_not() {
    let ctx = context();
    let (calls, observed) = counter();
    let unselected = Radio::new("pro")
        .on_change(move |value| {
            assert_eq!(value, "pro");
            observed.set(observed.get() + 1);
        })
        .to_element(&ctx);
    Driver::new(&ctx, unselected).click(8.0, 8.0);
    assert_eq!(calls.get(), 1);

    let (calls, observed) = counter();
    let selected = Radio::new("pro")
        .selected(true)
        .on_change(move |_| observed.set(observed.get() + 1))
        .to_element(&ctx);
    Driver::new(&ctx, selected).click(8.0, 8.0);
    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn a_disabled_control_proposes_nothing() {
    let ctx = context();
    let (calls, observed) = counter();
    let element = Checkbox::new()
        .disabled(true)
        .on_change(move |_| observed.set(observed.get() + 1))
        .to_element(&ctx);
    let mut driver = Driver::new(&ctx, element);

    driver.click(8.0, 8.0);
    let _ = driver.key(NamedKey::Enter);

    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn a_focused_switch_activates_from_the_keyboard() {
    let ctx = context();
    let (calls, observed) = counter();
    let element = Switch::new()
        .on_change(move |_| observed.set(observed.get() + 1))
        .to_element(&ctx);
    let mut driver = Driver::new(&ctx, element);

    // Focus it with a press that leaves the control without a tap.
    let _ = driver.pointer_down(8.0, 8.0);
    let _ = driver.pointer_move(399.0, 399.0);
    let _ = driver.pointer_up(399.0, 399.0);
    let before = calls.get();

    assert!(driver.key(NamedKey::Enter).is_consumed());
    assert_eq!(calls.get(), before + 1);
    assert!(
        !driver.key(NamedKey::ArrowDown).is_consumed(),
        "a switch must not swallow keys it does not use"
    );
}

#[tokio::test]
async fn the_builder_sees_the_pressed_flag_while_a_press_is_in_progress() {
    let ctx = context();
    let pressed = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&pressed);
    let element = Switch::new()
        .builder(move |state| {
            observed.borrow_mut().push(state.is_pressed());
            SizedBox::new().width(60.0).height(60.0)
        })
        .to_element(&ctx);
    let mut driver = Driver::new(&ctx, element);

    let _ = driver.pointer_down(8.0, 8.0);
    assert_eq!(pressed.borrow().last(), Some(&true));
    let _ = driver.pointer_up(8.0, 8.0);
    assert_eq!(pressed.borrow().last(), Some(&false));
}

#[tokio::test]
async fn the_builder_receives_the_controlled_value_after_a_rebuild() {
    let ctx = context();
    let values = Rc::new(RefCell::new(Vec::new()));
    let make = |value: CheckboxValue| {
        let observed = Rc::clone(&values);
        Checkbox::new()
            .value(value)
            .builder(move |visual| {
                observed.borrow_mut().push(visual.value());
                SizedBox::new().width(24.0).height(24.0)
            })
            .create_state()
    };
    let mut state = make(CheckboxValue::Unchecked);

    state.build(&ctx).to_element(&ctx);
    state.adopt_config_from(make(CheckboxValue::Checked));
    state.build(&ctx).to_element(&ctx);

    assert_eq!(
        values.borrow().as_slice(),
        &[CheckboxValue::Unchecked, CheckboxValue::Checked]
    );
}

#[tokio::test]
async fn a_checkbox_can_wrap_an_explicit_child() {
    let ctx = context();
    let element = Checkbox::new()
        .child(Row::new().children([Text::new("□").boxed(), Text::new("Accept").boxed()]))
        .to_element(&ctx);
    let size = element.layout(&ctx);
    assert!(size.width > 0.0 && size.height > 0.0);
}

#[tokio::test]
async fn semantics_describe_each_control() {
    let checkbox = Checkbox::new()
        .value(CheckboxValue::Indeterminate)
        .label("All")
        .error("pick one")
        .semantics();
    assert_eq!(checkbox.role(), SemanticRole::Checkbox);
    assert_eq!(checkbox.checked_value(), Some(CheckboxValue::Indeterminate));
    assert_eq!(checkbox.label(), Some("All"));
    assert_eq!(checkbox.error(), Some("pick one"));

    let switch = Switch::new().value(true).disabled(true).semantics();
    assert_eq!(switch.role(), SemanticRole::Switch);
    assert_eq!(switch.checked_value(), Some(CheckboxValue::Checked));
    assert!(!switch.enabled());

    let radio = Radio::new(1).selected(true).semantics();
    assert_eq!(radio.role(), SemanticRole::Radio);
    assert_eq!(radio.is_selected(), Some(true));
}
