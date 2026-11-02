//! RadioGroup, Select, and Autocomplete through their public API.

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_container::SizedBox;
use aimer_events::element::NamedKey;
use aimer_flex::Column;
use aimer_selection::{
    Autocomplete, ChoiceOption, OptionError, RadioGroup, Select, SemanticRole,
};
use aimer_widget::{LayoutElement, PortableWidget, Widget};
use support::{Driver, context};

fn plans() -> Vec<ChoiceOption<&'static str>> {
    vec![
        ChoiceOption::new("basic", "Basic", "basic"),
        ChoiceOption::new("pro", "Pro", "pro"),
        ChoiceOption::new("team", "Team", "team").disabled(true),
        ChoiceOption::new("enterprise", "Enterprise", "enterprise"),
    ]
}

fn height_in_column(ctx: &aimer_widget::base::BuildContext, widget: impl Widget + 'static) -> f32 {
    Column::new()
        .children([widget.boxed()])
        .to_element(ctx)
        .layout(ctx)
        .height
}

#[tokio::test]
async fn list_controls_are_named_widgets_with_the_default_portable_contract() {
    fn assert_portable<T: PortableWidget>() {}
    assert_portable::<RadioGroup<i32>>();
    assert_portable::<Select<i32>>();
    assert_portable::<Autocomplete<&'static str>>();

    let ctx = context();
    assert_eq!(RadioGroup::<i32>::new().to_element(&ctx).debug_name(), "RadioGroup");
    assert_eq!(Select::<i32>::new().to_element(&ctx).debug_name(), "Select");
    assert_eq!(
        Autocomplete::<i32>::new().to_element(&ctx).debug_name(),
        "Autocomplete"
    );
}

#[test]
fn option_keys_must_be_unique_and_non_empty() {
    let duplicate = RadioGroup::new().try_options([
        ChoiceOption::new("a", "One", 1),
        ChoiceOption::new("a", "Two", 2),
    ]);
    assert_eq!(duplicate.err(), Some(OptionError::DuplicateKey("a".to_owned())));

    let empty = Select::new().try_options([ChoiceOption::new("", "Nameless", 1)]);
    assert_eq!(empty.err(), Some(OptionError::EmptyKey));

    // Duplicate labels are presentation, not identity.
    assert!(
        Autocomplete::new()
            .try_options([ChoiceOption::new("a", "Same", 1), ChoiceOption::new("b", "Same", 2)])
            .is_ok()
    );
}

#[test]
#[should_panic(expected = "duplicate choice option key: a")]
fn the_infallible_options_setter_panics_on_a_bad_key() {
    let _ = Select::new().options([
        ChoiceOption::new("a", "One", 1),
        ChoiceOption::new("a", "Two", 2),
    ]);
}

#[tokio::test]
async fn a_radio_group_lays_out_every_option() {
    let ctx = context();
    let one = RadioGroup::new().options([ChoiceOption::new("a", "A", 1)]);
    let three = RadioGroup::new().options([
        ChoiceOption::new("a", "A", 1),
        ChoiceOption::new("b", "B", 2),
        ChoiceOption::new("c", "C", 3),
    ]);
    let one = height_in_column(&ctx, one);
    let three = height_in_column(&ctx, three);
    assert!(
        three > one * 1.5,
        "three options must not clip to one row: one={one}, three={three}"
    );
}

#[tokio::test]
async fn tapping_a_radio_row_proposes_its_value() {
    let ctx = context();
    let proposed = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&proposed);
    // Fixed-height rows make the row centers computable from the theme's
    // padding and gap.
    let tokens = aimer_style::ThemeTokens::light();
    let (padding, gap) = (tokens.spacing.x_small, tokens.spacing.small);
    let row_center = |index: usize| padding + index as f32 * (40.0 + gap) + 20.0;
    let group = RadioGroup::new()
        .options(plans())
        .selected(Some("basic"))
        .builder(|_| SizedBox::new().width(200.0).height(40.0))
        .on_change(move |plan| observed.borrow_mut().push(plan));
    let mut driver = Driver::new(&ctx, group.to_element(&ctx));

    // Row 0 is the selection, so tapping it proposes nothing.
    driver.click(10.0, row_center(0));
    assert!(proposed.borrow().is_empty());

    driver.click(10.0, row_center(1));
    assert_eq!(proposed.borrow().as_slice(), &["pro"]);

    // Row 2 is disabled.
    driver.click(10.0, row_center(2));
    assert_eq!(proposed.borrow().as_slice(), &["pro"]);
}

#[tokio::test]
async fn a_radio_group_navigates_by_keyboard_skipping_disabled_options() {
    let ctx = context();
    let proposed = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&proposed);
    let group = RadioGroup::new()
        .options(plans())
        .selected(Some("pro"))
        .on_change(move |plan| observed.borrow_mut().push(plan));
    let mut driver = Driver::new(&ctx, group.to_element(&ctx));
    // Focus the group without activating a row.
    let _ = driver.pointer_down(399.0, 4.0);
    let _ = driver.pointer_up(399.0, 4.0);

    assert!(driver.key(NamedKey::ArrowDown).is_consumed());
    assert_eq!(proposed.borrow().as_slice(), &["enterprise"], "Team is disabled and skipped");

    // The owner has not adopted "enterprise", so "pro" is still the selection
    // and landing on it again proposes nothing.
    let before = proposed.borrow().len();
    let _ = driver.key(NamedKey::ArrowUp);
    assert_eq!(proposed.borrow().len(), before);

    let before = proposed.borrow().len();
    let _ = driver.key(NamedKey::Home);
    assert_eq!(proposed.borrow().len(), before, "Home moves the ring without proposing");
    let _ = driver.key(NamedKey::Enter);
    assert_eq!(proposed.borrow().last(), Some(&"basic"), "Enter proposes the ringed option");
}

#[tokio::test]
async fn a_disabled_radio_group_ignores_pointer_and_keys() {
    let ctx = context();
    let calls = Rc::new(Cell::new(0_u32));
    let observed = Rc::clone(&calls);
    let group = RadioGroup::new()
        .options(plans())
        .disabled(true)
        .on_change(move |_| observed.set(observed.get() + 1));
    let mut driver = Driver::new(&ctx, group.to_element(&ctx));
    driver.click(10.0, 10.0);
    let _ = driver.key(NamedKey::ArrowDown);
    assert_eq!(calls.get(), 0);
}

#[tokio::test]
async fn the_radio_group_builder_sees_selection_per_option() {
    let ctx = context();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&seen);
    let group = RadioGroup::new()
        .options(plans())
        .selected(Some("pro"))
        .builder(move |state| {
            observed
                .borrow_mut()
                .push((state.option().key().to_owned(), state.is_selected()));
            SizedBox::new().height(40.0)
        });
    let _driver = Driver::new(&ctx, group.to_element(&ctx));
    let seen = seen.borrow();
    assert!(seen.contains(&("pro".to_owned(), true)));
    assert!(seen.contains(&("basic".to_owned(), false)));
}

#[tokio::test]
async fn a_select_trigger_opens_and_closes_on_tap() {
    let ctx = context();
    let opens = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&opens);
    let select = Select::new()
        .options(plans())
        .selected(Some("pro"))
        .builder(move |state| {
            observed.borrow_mut().push((state.is_open(), state.text().to_owned()));
            SizedBox::new().width(120.0).height(44.0)
        });
    let mut driver = Driver::new(&ctx, select.to_element(&ctx));

    assert_eq!(opens.borrow().last(), Some(&(false, "Pro".to_owned())));
    driver.click(8.0, 8.0);
    assert_eq!(opens.borrow().last().map(|entry| entry.0), Some(true));
    driver.click(8.0, 8.0);
    assert_eq!(opens.borrow().last().map(|entry| entry.0), Some(false));
}

#[tokio::test]
async fn a_select_trigger_shows_the_placeholder_until_something_is_selected() {
    let ctx = context();
    let seen = Rc::new(RefCell::new(String::new()));
    let observed = Rc::clone(&seen);
    let select = Select::<&str>::new()
        .options(plans())
        .placeholder("Choose a plan")
        .builder(move |state| {
            *observed.borrow_mut() = state.text().to_owned();
            SizedBox::new().width(10.0).height(10.0)
        });
    let _ = select.to_element(&ctx).layout(&ctx);
    assert_eq!(&*seen.borrow(), "Choose a plan");
}

#[tokio::test]
async fn a_closed_select_is_one_row_tall_however_many_options_it_has() {
    let ctx = context();
    let few = Select::new().options((0..3).map(|i| ChoiceOption::new(format!("k{i}"), "x", i)));
    let many = Select::new().options((0..3000).map(|i| ChoiceOption::new(format!("k{i}"), "x", i)));
    let few = height_in_column(&ctx, few);
    let many = height_in_column(&ctx, many);
    assert_eq!(few, many, "options live in the popup, not in the layout");
}

#[tokio::test]
async fn a_disabled_select_does_not_open() {
    let ctx = context();
    let opened = Rc::new(Cell::new(false));
    let observed = Rc::clone(&opened);
    let select = Select::new()
        .options(plans())
        .disabled(true)
        .builder(move |state| {
            observed.set(observed.get() || state.is_open());
            SizedBox::new().width(120.0).height(44.0)
        });
    let mut driver = Driver::new(&ctx, select.to_element(&ctx));
    driver.click(8.0, 8.0);
    let _ = driver.key(NamedKey::Enter);
    assert!(!opened.get());
}

#[tokio::test]
async fn a_select_opens_from_the_keyboard() {
    let ctx = context();
    let open = Rc::new(Cell::new(false));
    let observed = Rc::clone(&open);
    let select = Select::new().options(plans()).builder(move |state| {
        observed.set(state.is_open());
        SizedBox::new().width(120.0).height(44.0)
    });
    let mut driver = Driver::new(&ctx, select.to_element(&ctx));
    let _ = driver.pointer_down(8.0, 8.0);
    let _ = driver.pointer_move(399.0, 399.0);
    let _ = driver.pointer_up(399.0, 399.0);
    assert!(!open.get());

    assert!(driver.key(NamedKey::ArrowDown).is_consumed());
    assert!(open.get());
}

#[tokio::test]
async fn an_autocomplete_builds_loading_and_error_states() {
    let ctx = context();
    let widget = Autocomplete::new()
        .options(plans())
        .query("pr")
        .loading(true)
        .error("offline");
    assert_eq!(Widget::debug_name(&widget), "Autocomplete");
    let _ = widget.to_element(&ctx).layout(&ctx);
}

#[test]
fn semantics_describe_the_list_controls() {
    let group = RadioGroup::<u8>::new().label("Plan").semantics();
    assert_eq!(group.role(), SemanticRole::RadioGroup);
    assert_eq!(group.label(), Some("Plan"));

    assert_eq!(Select::<u8>::new().semantics().role(), SemanticRole::Select);

    let autocomplete = Autocomplete::<u8>::new().loading(true).error("bad").semantics();
    assert_eq!(autocomplete.role(), SemanticRole::Autocomplete);
    assert!(autocomplete.is_busy());
    assert_eq!(autocomplete.error(), Some("bad"));
}
