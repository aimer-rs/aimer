//! The icons in the margin before the line numbers: drawn on their lines, and pressed like buttons.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use aimer_attribute::Vec2d;
use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_cupid::svg::{SvgScene, SvgViewport};
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
use aimer_events::element::{ElementEvent, Modifiers};
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, Element, EventElement, FocusNode, LayoutElement, Rebuildable};

use super::test_support::{dummy_build_context, field_config};
use super::{RawTextField, RawTextFieldHost};
use crate::TextEditingController;
use crate::input_field::caret::CaretBlink;
use crate::input_field::gutter::{GutterMarker, GutterMarkers, LineNumbers};

const TEXT: &str = "alpha\nbeta\ngamma\ndelta";

fn scene() -> Arc<SvgScene> {
    Arc::new(SvgScene { viewport: SvgViewport { width: 24.0, height: 24.0 }, nodes: Arc::from(Vec::new()), geometries: Arc::from(Vec::new()), resources: Arc::default() })
}

/// A field with numbers and a marker on the second and fourth lines, and the number of times the
/// marker of each was pressed.
fn field(markers: bool) -> (RawTextField, Rc<Cell<u32>>, BuildContext<'static>) {
    let pressed = Rc::new(Cell::new(0));
    let counted = Rc::clone(&pressed);
    let mut config = field_config(TextEditingController::with_text(TEXT));
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    if markers {
        config.gutter_markers = Some(GutterMarkers::new(16, move |line| {
            let counted = Rc::clone(&counted);
            (line == 1 || line == 3).then(|| GutterMarker::new(scene(), move || counted.set(counted.get() + 1)))
        }));
    }
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    (field, pressed, ctx)
}

pub(super) fn commands(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 400.0, 300.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

fn svgs(commands: &[DrawCommand]) -> Vec<Rect> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::Svg { destination, .. } => Some(*destination),
            _ => None,
        })
        .collect()
}

#[test]
fn the_icons_take_their_column_from_the_margin_and_are_drawn_before_the_numbers_of_their_lines() {
    let (plain, _, _) = field(false);
    let (marked, _, ctx) = field(true);
    assert_eq!(marked.gutter_width(1.0), plain.gutter_width(1.0) + 16.0, "a column as wide as an icon");

    let commands = commands(&marked, &ctx);
    let icons = svgs(&commands);
    assert_eq!(icons.len(), 2, "one for each line that has a marker: {icons:?}");
    assert!(icons.iter().all(|icon| icon.width == 16.0 && icon.height == 16.0), "{icons:?}");
    assert_eq!(icons[0].x, icons[1].x, "in one column");
    // Before the line numbers, which are before the text.
    let number_x = commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { text, position, .. } if text.trim() == "2" => Some(position.x),
            _ => None,
        })
        .next()
        .expect("the number 2 is drawn");
    assert!(icons[0].x + icons[0].width <= number_x + 0.5, "icon {:?} then number at {number_x}", icons[0]);
    // The icon of the second line is level with its number and a row above the fourth's.
    let layout = marked.local_v2_layout(&ctx).expect("laid out");
    assert!((icons[1].y - icons[0].y - 2.0 * layout.line_height).abs() < 2.0, "{icons:?} for rows of {}", layout.line_height);
}

#[test]
fn no_markers_means_no_icons_and_no_hit() {
    let (plain, _, ctx) = field(false);
    assert!(svgs(&commands(&plain, &ctx)).is_empty());
    assert!(plain.marker_hits.borrow().is_empty());
}

#[test]
fn pressing_an_icon_calls_it_and_leaves_the_caret_and_the_release_alone() {
    let (marked, pressed, ctx) = field(true);
    let controller = marked.controller.clone();
    let _ = commands(&marked, &ctx);
    let hit = marked.marker_hits.borrow()[0].clone();
    let on_icon = Vec2d { x: hit.x + hit.size / 2.0, y: hit.y + hit.size / 2.0 };
    // A point on the text of the first line, to the right of the margin.
    let layout = marked.local_v2_layout(&ctx).expect("laid out");
    let on_text = Vec2d { x: layout.padding_left + 4.0, y: layout.padding_top + layout.line_height * 0.5 };
    let end = controller.cursor_position();
    assert_eq!(end.0, 4, "the caret starts on the last line");

    let host = RawTextFieldHost::new(marked, super::embed_tests::caret_stub());
    let root = host.boxed();
    root.sync_paint_state(&ctx);
    root.update(&ctx);
    let press = |at: Vec2d, event: ElementEvent| aimer_widget::dispatch_event(root.as_ref(), at, &event);
    let mouse = |at: Vec2d| PointerInfo::mouse(at, PointerButton::Primary);

    let down = press(on_icon, ElementEvent::PointerDown(mouse(on_icon)));
    assert!(down.is_consumed(), "the press is the icon's");
    assert_eq!(pressed.get(), 1, "the icon's handler ran once");
    let up = press(on_icon, ElementEvent::PointerUp(mouse(on_icon)));
    assert!(up.is_consumed(), "so is its release");
    root.sync_paint_state(&ctx);
    assert_eq!(pressed.get(), 1, "the release does not press it again");
    assert_eq!(controller.cursor_position(), end, "and the caret did not move");

    // A press on the text is the field's: it moves the caret and calls no icon.
    let _ = press(on_text, ElementEvent::PointerDown(mouse(on_text)));
    let _ = press(on_text, ElementEvent::PointerUp(mouse(on_text)));
    root.sync_paint_state(&ctx);
    assert_eq!(pressed.get(), 1);
    assert_ne!(controller.cursor_position(), end, "a press on the text moves the caret");
}

#[test]
fn a_press_on_what_hangs_from_the_caret_is_not_the_fields_and_leaves_the_caret_where_it_is() {
    let (plain, _, ctx) = field(false);
    let controller = plain.controller.clone();
    let _ = commands(&plain, &ctx);
    let layout = plain.local_v2_layout(&ctx).expect("laid out");
    let on_text = Vec2d { x: layout.padding_left + 4.0, y: layout.padding_top + layout.line_height * 0.5 };
    let caret_at = controller.cursor_position();
    let caret = plain.caret_context.geometry();

    // A menu that hangs 20 below the caret and begins at it, 120 wide and 60 tall.
    plain.caret_context.set_popup(Some((0.0, 20.0, 120.0, 60.0)));
    let controller_caret = plain.caret_context.clone();
    let inside = Vec2d { x: caret.x + 60.0, y: caret.y + 50.0 };
    // A field that is not at the corner of the window is told its presses in the window's coordinates: the menu is
    // where the caret is on the field, and the field is where it is on the window.
    plain.cached_bounds.save(1.0, 50.0, 30.0, 400.0, 300.0);
    // Near the far corner of the menu: 110 across and 70 down from the caret, which is inside it only once the field's
    // own position is taken off, and just out of it 20 further across.
    let near_corner = Vec2d { x: 50.0 + caret.x + 110.0, y: 30.0 + caret.y + 70.0 };
    assert!(plain.popup_at(near_corner), "on the menu, counted from the corner of the window");
    assert!(!plain.popup_at(Vec2d { x: near_corner.x + 20.0, y: near_corner.y }), "and not past its edge");
    plain.cached_bounds.save(1.0, 0.0, 0.0, 400.0, 300.0);
    let host = RawTextFieldHost::new(plain, super::embed_tests::caret_stub());
    let root = host.boxed();
    root.sync_paint_state(&ctx);
    root.update(&ctx);
    let press = |at: Vec2d, event: ElementEvent| aimer_widget::dispatch_event(root.as_ref(), at, &event);
    let mouse = |at: Vec2d| PointerInfo::mouse(at, PointerButton::Primary);

    let pressed: Rc<std::cell::RefCell<Vec<(f32, f32)>>> = Rc::default();
    let seen = Rc::clone(&pressed);
    controller_caret.on_popup_press(Some(Rc::new(move |x, y| seen.borrow_mut().push((x, y)))));
    let down = press(inside, ElementEvent::PointerDown(mouse(inside)));
    assert!(down.is_consumed(), "the press is the menu's");
    assert_eq!(*pressed.borrow(), vec![(60.0, 30.0)], "and the menu is told where on it, from its own corner: 60 across and 30 below its top");
    let up = press(inside, ElementEvent::PointerUp(mouse(inside)));
    assert!(up.is_consumed(), "so is its release");
    root.sync_paint_state(&ctx);
    assert_eq!(controller.cursor_position(), caret_at, "and the caret did not move");

    // The menu is gone: the same place is the text's, and the caret goes there.
    let _ = press(on_text, ElementEvent::PointerDown(mouse(on_text)));
    let _ = press(on_text, ElementEvent::PointerUp(mouse(on_text)));
    root.sync_paint_state(&ctx);
    assert_ne!(controller.cursor_position(), caret_at, "a press that is not on the menu moves the caret");
}

#[test]
fn the_pointer_is_an_arrow_and_not_the_text_cursor_over_an_icon_in_the_margin_and_over_what_hangs_from_the_caret() {
    use aimer_utils::cursor::{restore_thread_cursor_handler, set_thread_cursor_handler, Cursor, CursorIcon};

    let requests = Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorded = Rc::clone(&requests);
    let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

    let (marked, _, ctx) = field(true);
    let _ = commands(&marked, &ctx);
    let hit = marked.marker_hits.borrow()[0].clone();
    let on_icon = Vec2d { x: hit.x + hit.size / 2.0, y: hit.y + hit.size / 2.0 };
    let layout = marked.local_v2_layout(&ctx).expect("laid out");
    let on_text = Vec2d { x: layout.padding_left + 4.0, y: layout.padding_top + layout.line_height * 0.5 };
    let caret = marked.caret_context.geometry();
    marked.caret_context.set_popup(Some((0.0, 20.0, 120.0, 60.0)));
    let on_menu = Vec2d { x: caret.x + 60.0, y: caret.y + 50.0 };
    // Something drawn over the top-right corner of the field, 50 wide and 30 tall, a gap in from the right and the top.
    let mut marked = marked;
    marked.pointer_holes = Some(Rc::new(|width, _height| vec![(width - 60.0, 10.0, 50.0, 30.0)]));
    let on_badge = Vec2d { x: 400.0 - 35.0, y: 25.0 };

    let host = RawTextFieldHost::new(marked, super::embed_tests::caret_stub());
    let root = host.boxed();
    root.sync_paint_state(&ctx);
    root.update(&ctx);
    let last = || requests.borrow().last().cloned();
    let move_to = |at: Vec2d| {
        let _ = aimer_widget::dispatch_event(root.as_ref(), at, &ElementEvent::PointerMove(PointerInfo::mouse(at, PointerButton::Primary)));
    };

    move_to(on_text);
    assert_eq!(last(), Some(Cursor::Icon(CursorIcon::Text)), "over the text, the text cursor");
    move_to(on_icon);
    assert_eq!(last(), Some(Cursor::Icon(CursorIcon::Default)), "over an icon in the margin, an arrow");
    move_to(on_text);
    assert_eq!(last(), Some(Cursor::Icon(CursorIcon::Text)), "and the text cursor again off it");
    move_to(on_menu);
    assert_eq!(last(), Some(Cursor::Icon(CursorIcon::Default)), "over what hangs from the caret, an arrow");
    move_to(on_text);
    move_to(on_badge);
    assert_eq!(last(), Some(Cursor::Icon(CursorIcon::Default)), "over what is drawn over the field, an arrow");
    move_to(Vec2d { x: 400.0 - 5.0, y: 25.0 });
    assert_eq!(last(), Some(Cursor::Icon(CursorIcon::Text)), "and beside it, in the gap, the text cursor again");

    restore_thread_cursor_handler(previous);
}

#[test]
fn what_hangs_from_the_caret_is_dismissed_when_the_visible_part_of_the_field_changes_size_and_when_the_field_loses_focus() {
    let (plain, _, ctx) = field(false);
    let _ = commands(&plain, &ctx);
    let context = plain.caret_context.clone();
    let dismissed = Rc::new(Cell::new(0));
    let counted = Rc::clone(&dismissed);
    context.on_popup_dismiss(Some(Rc::new(move || counted.set(counted.get() + 1))));
    let (right, bottom) = context.visible_extent().expect("laid out");

    // Nothing hangs from the caret: there is nothing to dismiss.
    context.set_extent(Some((right, bottom - 40.0)));
    assert_eq!(dismissed.get(), 0);
    context.set_extent(Some((right, bottom)));

    context.set_popup(Some((0.0, 20.0, 120.0, 60.0)));
    // The same size again is the same place for it.
    context.set_extent(Some((right, bottom)));
    assert_eq!(dismissed.get(), 0);
    assert!(context.popup_at(context.geometry().x + 10.0, context.geometry().y + 30.0));
    // A smaller field, as when a panel opens under it: it is not where it was, and is gone.
    context.set_extent(Some((right, bottom - 100.0)));
    assert_eq!(dismissed.get(), 1);
    assert!(!context.popup_at(context.geometry().x + 10.0, context.geometry().y + 30.0), "and no press is its any more");
    context.set_extent(Some((right, bottom)));
    assert_eq!(dismissed.get(), 1, "there is nothing left to dismiss");

    // Losing focus is dismissal too.
    context.set_popup(Some((0.0, 20.0, 120.0, 60.0)));
    let _ = plain.on_event(&ElementEvent::FocusLost);
    assert_eq!(dismissed.get(), 2);
    assert!(!context.popup_at(context.geometry().x + 10.0, context.geometry().y + 30.0));
}

mod force_click {
    use super::*;
    use aimer_events::element::PressureStage;

    /// A field of `text` that records the bytes it is force-clicked at, painted once.
    fn field_of(text: &str) -> (RawTextField, Rc<std::cell::RefCell<Vec<usize>>>, BuildContext<'static>) {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let mut config = field_config(TextEditingController::with_text(text));
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = true;
        config.on_force_click = Some(Rc::new(move |at| record.borrow_mut().push(at)));
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        (field, seen, ctx)
    }

    fn at_line(field: &RawTextField, ctx: &BuildContext<'_>, line: f32, x: f32) -> Vec2d {
        let layout = field.local_v2_layout(ctx).expect("laid out");
        Vec2d { x, y: layout.padding_top + layout.line_height * (line + 0.5) }
    }

    fn press(field: &RawTextField, ctx: &BuildContext<'_>, at: Vec2d, stage: PressureStage) -> bool {
        let consumed = field.on_event(&ElementEvent::Pressure { pos: at, pressure: 1.0, stage }).is_consumed();
        // The offset under a position is only known to the layout, which is the next frame.
        field.update(ctx);
        let _ = commands(field, ctx);
        consumed
    }

    #[test]
    fn a_force_click_says_which_byte_of_the_text_is_under_the_pointer() {
        let (field, seen, ctx) = field_of("alpha\nbeta\ngamma");
        // Far to the right of the second line, which is past its end: the end of "beta", at byte 10.
        let at = at_line(&field, &ctx, 1.0, 380.0);
        assert!(press(&field, &ctx, at, PressureStage::Force), "the field took it");
        assert_eq!(*seen.borrow(), vec![10]);
        // On the third line, before its first letter: the start of "gamma".
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        press(&field, &ctx, at_line(&field, &ctx, 2.0, layout.padding_left + 0.5), PressureStage::None);
        press(&field, &ctx, at_line(&field, &ctx, 2.0, layout.padding_left + 0.5), PressureStage::Force);
        assert_eq!(seen.borrow()[1], 11, "after the line break that ends the second line");
    }

    #[test]
    fn the_byte_is_counted_in_bytes_and_not_in_characters() {
        // Each accent is two bytes and one character: the end of the first line is byte 6 and not 3.
        let (field, seen, ctx) = field_of("\u{e9}\u{e9}\u{e9}\nxyz");
        press(&field, &ctx, at_line(&field, &ctx, 0.0, 380.0), PressureStage::Force);
        assert_eq!(*seen.borrow(), vec![6]);
    }

    #[test]
    fn a_light_press_and_a_press_that_stays_deep_are_not_new_force_clicks() {
        let (field, seen, ctx) = field_of("alpha\nbeta");
        let at = at_line(&field, &ctx, 0.0, 380.0);
        assert!(!press(&field, &ctx, at, PressureStage::Click), "a click is the field's own to read");
        press(&field, &ctx, at, PressureStage::Force);
        press(&field, &ctx, at, PressureStage::Force);
        assert_eq!(seen.borrow().len(), 1, "one deep press, however long it is held");
        press(&field, &ctx, at, PressureStage::None);
        press(&field, &ctx, at, PressureStage::Force);
        assert_eq!(seen.borrow().len(), 2);
    }

    #[test]
    fn a_force_click_beside_the_field_is_not_the_fields() {
        let (field, seen, ctx) = field_of("alpha");
        assert!(!press(&field, &ctx, Vec2d { x: 900.0, y: 900.0 }, PressureStage::Force));
        assert!(seen.borrow().is_empty());
    }

    /// A field of `text` that records the bytes it is command-clicked at.
    fn command_field_of(text: &str) -> (RawTextField, Rc<std::cell::RefCell<Vec<usize>>>, BuildContext<'static>) {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let mut config = field_config(TextEditingController::with_text(text));
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = true;
        config.on_command_click = Some(Rc::new(move |at| record.borrow_mut().push(at)));
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        (field, seen, ctx)
    }

    /// The modifier the platform jumps with: Command on Apple systems, where Control-click is the secondary click, and
    /// Control elsewhere.
    fn command() -> Modifiers {
        if cfg!(target_vendor = "apple") { Modifiers { meta: true, ..Modifiers::default() } } else { Modifiers { ctrl: true, ..Modifiers::default() } }
    }

    fn click(field: &RawTextField, ctx: &BuildContext<'_>, at: Vec2d, button: PointerButton) -> bool {
        let consumed = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, button))).is_consumed();
        let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(at, button)));
        field.update(ctx);
        let _ = commands(field, ctx);
        consumed
    }

    #[test]
    fn a_command_click_says_which_byte_of_the_text_is_under_the_pointer_and_leaves_the_caret() {
        let (field, seen, ctx) = command_field_of("alpha\nbeta\ngamma");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        // On the second letter of "beta": the byte reported is the start of the name, wherever in it the pointer is.
        let at = at_line(&field, &ctx, 1.0, layout.padding_left + 1.0);
        Modifiers::set_current(command());
        let consumed = click(&field, &ctx, at, PointerButton::Primary);
        // Past the end of the line is on no name: the field takes the press and reports nothing.
        click(&field, &ctx, at_line(&field, &ctx, 1.0, 380.0), PointerButton::Primary);
        Modifiers::set_current(Modifiers::default());
        assert!(consumed, "the field took it");
        assert_eq!(*seen.borrow(), vec![6]);
        assert_eq!(field.cursor.offset(), 0, "the caret stays where it was: the click is not a place to type");
    }

    #[test]
    fn a_click_without_the_command_key_or_with_another_button_is_not_a_command_click() {
        let (field, seen, ctx) = command_field_of("alpha\nbeta");
        let at = at_line(&field, &ctx, 0.0, 380.0);
        click(&field, &ctx, at, PointerButton::Primary);
        assert!(seen.borrow().is_empty(), "a plain click places the caret");
        assert_eq!(field.cursor.offset(), 5);
        Modifiers::set_current(command());
        click(&field, &ctx, at, PointerButton::Secondary);
        Modifiers::set_current(Modifiers::default());
        assert!(seen.borrow().is_empty(), "a secondary click asks for the menu");
    }

    #[test]
    fn a_command_click_is_one_for_each_press_and_is_not_on_what_is_beside_the_field() {
        let (field, seen, ctx) = command_field_of("alpha");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        Modifiers::set_current(command());
        click(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 1.0), PointerButton::Primary);
        click(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 1.0), PointerButton::Primary);
        assert!(!click(&field, &ctx, Vec2d { x: 900.0, y: 900.0 }, PointerButton::Primary));
        Modifiers::set_current(Modifiers::default());
        assert_eq!(seen.borrow().len(), 2);
    }

    fn hover(field: &RawTextField, ctx: &BuildContext<'_>, at: Vec2d) {
        let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(at, PointerButton::Primary)));
        field.update(ctx);
        let _ = commands(field, ctx);
    }

    fn keys(field: &RawTextField, ctx: &BuildContext<'_>, held: Modifiers) {
        Modifiers::set_current(held);
        let _ = field.on_event(&ElementEvent::ModifiersChanged(held));
        field.update(ctx);
        let _ = commands(field, ctx);
    }

    #[test]
    fn the_name_under_the_pointer_is_a_link_while_the_jump_key_is_held() {
        let (field, _, ctx) = command_field_of("alpha beta\ngamma");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let on_alpha = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
        let on_gamma = at_line(&field, &ctx, 1.0, layout.padding_left + 6.0);
        hover(&field, &ctx, on_alpha);
        assert_eq!(field.link_range(), None, "the key is not held");
        keys(&field, &ctx, command());
        assert_eq!(field.link_range(), Some(0..5), "pressing the key with the pointer where it is makes the name a link");
        hover(&field, &ctx, on_gamma);
        assert_eq!(field.link_range(), Some(11..16), "moving to another name moves it");
        // Past the end of the line is not on a name.
        hover(&field, &ctx, at_line(&field, &ctx, 0.0, 380.0));
        assert_eq!(field.link_range(), None);
        hover(&field, &ctx, on_alpha);
        assert_eq!(field.link_range(), Some(0..5));
        keys(&field, &ctx, Modifiers::default());
        assert_eq!(field.link_range(), None, "letting go of the key takes the link away");
    }

    #[test]
    fn a_name_is_a_link_wherever_the_pointer_is_in_the_height_of_its_line() {
        let (field, seen, ctx) = command_field_of("alpha beta\ngamma delta\nzeta");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        keys(&field, &ctx, command());
        for line in 0..3 {
            for tenth in 0..10 {
                let at = at_line(&field, &ctx, line as f32 - 0.5 + (tenth as f32 + 0.5) / 10.0, layout.padding_left + 1.0);
                hover(&field, &ctx, at);
                assert!(field.link_range().is_some(), "line {line}, {tenth} tenths down it: no link");
                click(&field, &ctx, at, PointerButton::Primary);
                assert_eq!(seen.borrow().len(), line * 10 + tenth + 1, "line {line}, {tenth} tenths down it: no command click");
            }
        }
        keys(&field, &ctx, Modifiers::default());
    }

    /// A field of the same text and callbacks as `old`, built again as a rebuild of what is above does it.
    fn rebuilt(old: &RawTextField, text: &str, seen: &Rc<std::cell::RefCell<Vec<usize>>>) -> RawTextField {
        let (record_command, record_force) = (Rc::clone(seen), Rc::clone(seen));
        let mut config = field_config(TextEditingController::with_text(text));
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = true;
        config.on_command_click = Some(Rc::new(move |at| record_command.borrow_mut().push(at)));
        config.on_force_click = Some(Rc::new(move |at| record_force.borrow_mut().push(at)));
        let new = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        new.adopt_runtime_state_from(old as &dyn aimer_widget::Element);
        new
    }

    #[test]
    fn a_click_that_is_waiting_for_the_layout_is_not_lost_when_the_field_is_built_again() {
        // Any change of what is above the field builds it again, and the layout that says which byte a press was on is
        // the frame after the press: a rebuild between the two used to take the press away, and the click did nothing.
        let (field, seen, ctx) = command_field_of("alpha\nbeta");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let at = at_line(&field, &ctx, 1.0, layout.padding_left + 1.0);
        Modifiers::set_current(command());
        let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, PointerButton::Primary)));
        let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(at, PointerButton::Primary)));
        Modifiers::set_current(Modifiers::default());
        let new = rebuilt(&field, "alpha\nbeta", &seen);
        new.update(&ctx);
        let _ = commands(&new, &ctx);
        assert_eq!(*seen.borrow(), vec![6], "the click arrived");
    }

    #[test]
    fn a_force_click_that_is_waiting_for_the_layout_is_not_lost_when_the_field_is_built_again() {
        let (field, seen, ctx) = field_of("alpha\nbeta");
        let at = at_line(&field, &ctx, 0.0, 380.0);
        let _ = field.on_event(&ElementEvent::Pressure { pos: at, pressure: 1.0, stage: PressureStage::Force });
        let new = rebuilt(&field, "alpha\nbeta", &seen);
        new.update(&ctx);
        let _ = commands(&new, &ctx);
        assert_eq!(*seen.borrow(), vec![5]);
        // The press that is still held is the same deep press, and is not a second force click.
        let _ = new.on_event(&ElementEvent::Pressure { pos: at, pressure: 1.0, stage: PressureStage::Force });
        new.update(&ctx);
        let _ = commands(&new, &ctx);
        assert_eq!(seen.borrow().len(), 1, "one deep press, one force click, whatever was rebuilt meanwhile");
    }

    #[test]
    fn a_link_stays_when_the_field_is_built_again() {
        let (field, seen, ctx) = command_field_of("alpha beta");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        keys(&field, &ctx, command());
        hover(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
        assert_eq!(field.link_range(), Some(0..5));
        let new = rebuilt(&field, "alpha beta", &seen);
        assert_eq!(new.link_range(), Some(0..5), "the name is still a link, and is not drawn plain until the pointer moves");
        // The pointer is still where it was, so letting go of the key is known to take the link away.
        keys(&new, &ctx, Modifiers::default());
        assert_eq!(new.link_range(), None);
    }

    /// Runs `body` with the cursors the field asks for noted, and gives the last one it asked for.
    fn last_cursor(body: impl FnOnce()) -> Option<aimer_utils::cursor::Cursor> {
        let asked = Rc::new(std::cell::RefCell::new(None));
        let note = Rc::clone(&asked);
        let previous = aimer_utils::cursor::set_thread_cursor_handler(move |cursor| *note.borrow_mut() = Some(cursor));
        body();
        aimer_utils::cursor::restore_thread_cursor_handler(previous);
        let last = asked.borrow().clone();
        last
    }

    #[test]
    fn the_pointer_is_a_hand_over_a_link_and_the_text_cursor_elsewhere() {
        use aimer_utils::cursor::{Cursor, CursorIcon};
        let (field, _, ctx) = command_field_of("alpha beta\n  gamma");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let on_alpha = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
        let past_the_end = at_line(&field, &ctx, 0.0, 380.0);
        // Without the key it is a text cursor over a name, as it always was.
        assert_eq!(last_cursor(|| hover(&field, &ctx, on_alpha)), Some(Cursor::Icon(CursorIcon::Text)));
        // With it, the name is a link, and the cursor is a hand as soon as the layout has found the name.
        assert_eq!(last_cursor(|| keys(&field, &ctx, command())), Some(Cursor::Icon(CursorIcon::Pointer)));
        // It stays a hand as the pointer moves inside the name, and does not go back to the text cursor for a moment.
        assert_eq!(last_cursor(|| hover(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 8.0))), Some(Cursor::Icon(CursorIcon::Pointer)));
        // Off the name it is the text cursor again.
        assert_eq!(last_cursor(|| hover(&field, &ctx, past_the_end)), Some(Cursor::Icon(CursorIcon::Text)));
        hover(&field, &ctx, on_alpha);
        assert_eq!(last_cursor(|| keys(&field, &ctx, Modifiers::default())), Some(Cursor::Icon(CursorIcon::Text)), "letting go of the key");
    }

    #[test]
    fn what_is_drawn_over_the_text_is_not_a_link_and_does_not_take_a_command_click() {
        // A hole over the whole of the first line's start: a menu that hangs there.
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let mut config = field_config(TextEditingController::with_text("alpha beta"));
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = true;
        config.on_command_click = Some(Rc::new(move |at| record.borrow_mut().push(at)));
        config.pointer_holes = Some(Rc::new(|width, height| vec![(0.0, 0.0, width, height)]));
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        let _ = field.on_event(&ElementEvent::FocusGained);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let at = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
        keys(&field, &ctx, command());
        hover(&field, &ctx, at);
        assert_eq!(field.link_range(), None, "the name is under something");
        click(&field, &ctx, at, PointerButton::Primary);
        keys(&field, &ctx, Modifiers::default());
        assert!(seen.borrow().is_empty());
    }

    /// A field of "alpha" and "beta", on two lines, in which only "beta" is something to jump from, as an app that knows its syntax says.
    fn field_where_only_beta_is_a_link() -> (RawTextField, Rc<std::cell::RefCell<Vec<usize>>>, BuildContext<'static>) {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let mut config = field_config(TextEditingController::with_text("alpha\nbeta"));
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = true;
        config.on_command_click = Some(Rc::new(move |at| record.borrow_mut().push(at)));
        config.link_range = Some(Rc::new(|byte| (6..10).contains(&byte).then_some(6..10)));
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        (field, seen, ctx)
    }

    #[test]
    fn the_app_says_what_is_a_link_and_a_word_that_is_not_one_is_neither_shown_nor_clicked() {
        let (field, seen, ctx) = field_where_only_beta_is_a_link();
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let on_alpha = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
        let on_beta = at_line(&field, &ctx, 1.0, layout.padding_left + 1.0);
        keys(&field, &ctx, command());
        hover(&field, &ctx, on_alpha);
        assert_eq!(field.link_range(), None, "a word the app does not offer");
        click(&field, &ctx, on_alpha, PointerButton::Primary);
        assert!(seen.borrow().is_empty(), "and it is not jumped from");
        hover(&field, &ctx, on_beta);
        assert_eq!(field.link_range(), Some(6..10), "the range is the app's, and not the word's");
        click(&field, &ctx, on_beta, PointerButton::Primary);
        assert_eq!(seen.borrow().len(), 1);
        keys(&field, &ctx, Modifiers::default());
    }

    #[test]
    fn the_last_letter_of_a_name_is_a_link_on_its_right_half_too_and_the_space_after_it_is_not() {
        let (field, seen, ctx) = command_field_of("alpha beta\ngamma");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let letters = layout.geometry.selection_rects(0, 5);
        let [alpha] = letters.as_slice() else { panic!("one rectangle for a word: {letters:?}") };
        let last = layout.geometry.selection_rects(4, 5);
        let right_half = last[0].x + last[0].width * 0.9;
        let left_of_space = last[0].x + last[0].width + 0.5;
        let y = alpha.y + alpha.height / 2.0;
        keys(&field, &ctx, command());
        hover(&field, &ctx, Vec2d { x: layout.padding_left + right_half, y: layout.padding_top + y });
        assert_eq!(field.link_range(), Some(0..5), "the right half of the last letter is on the letter");
        click(&field, &ctx, Vec2d { x: layout.padding_left + right_half, y: layout.padding_top + y }, PointerButton::Primary);
        assert_eq!(*seen.borrow(), vec![0], "it is the click on the name, which is reported by where it starts");
        hover(&field, &ctx, Vec2d { x: layout.padding_left + left_of_space + 1.0, y: layout.padding_top + y });
        assert_eq!(field.link_range(), None, "the space after the name is not part of it");
        keys(&field, &ctx, Modifiers::default());
    }

    #[test]
    fn a_link_goes_when_the_pointer_leaves_and_when_the_text_changes() {
        let (field, _, ctx) = command_field_of("alpha beta");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        keys(&field, &ctx, command());
        hover(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
        assert_eq!(field.link_range(), Some(0..5));
        field.controller.set_text("x alpha beta");
        assert_eq!(field.link_range(), None, "the bytes are of the text that was there");
        hover(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 1.0));
        assert_eq!(field.link_range(), Some(0..1));
        let _ = field.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        assert_eq!(field.link_range(), None);
        keys(&field, &ctx, Modifiers::default());
    }

    #[test]
    fn a_field_that_did_not_ask_for_a_command_click_shows_no_link() {
        let (field, _, ctx) = field_of("alpha beta");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        keys(&field, &ctx, command());
        hover(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
        assert_eq!(field.link_range(), None);
        keys(&field, &ctx, Modifiers::default());
    }

    #[test]
    fn a_field_that_did_not_ask_for_a_command_click_takes_it_as_a_click() {
        let (field, _, ctx) = field_of("alpha\nbeta");
        Modifiers::set_current(command());
        click(&field, &ctx, at_line(&field, &ctx, 0.0, 380.0), PointerButton::Primary);
        Modifiers::set_current(Modifiers::default());
        assert_eq!(field.cursor.offset(), 5);
    }

    #[test]
    fn a_field_that_did_not_ask_for_a_force_click_leaves_it_alone() {
        let (plain, _, ctx) = super::field(false);
        let at = Vec2d { x: 100.0, y: 10.0 };
        assert!(!plain.on_event(&ElementEvent::Pressure { pos: at, pressure: 1.0, stage: PressureStage::Force }).is_consumed());
        let _ = ctx;
    }
}

mod locate {
    use super::*;
    use aimer_animation::AnimInstant;
    use std::time::Duration;

    /// 200 lines of "line N", in a field a few lines tall, painted once.
    fn tall_field() -> (RawTextField, TextEditingController, BuildContext<'static>) {
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let controller = TextEditingController::with_text(text);
        let mut config = field_config(controller.clone());
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = false;
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        field.test_clock.set(Some(AnimInstant::now()));
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        (field, controller, ctx)
    }

    fn frame(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<DrawCommand> {
        field.update(ctx);
        commands(field, ctx)
    }

    fn byte_of_line(controller: &TextEditingController, line: usize) -> usize {
        controller.value().text().split_inclusive('\n').take(line).map(str::len).sum()
    }

    #[test]
    fn a_located_range_is_shown_in_the_middle_of_the_view_and_not_at_its_edge() {
        let (field, controller, ctx) = tall_field();
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let at = byte_of_line(&controller, 100);
        controller.locate(at..at + 8);
        frame(&field, &ctx);
        let layout_after = field.local_v2_layout(&ctx).expect("laid out");
        let middle = layout.line_height * 100.5 - layout.content_height / 2.0;
        assert!((field.scroll_y.get() - middle).abs() < 1.0, "scrolled to {} and not to the middle, {middle}", field.scroll_y.get());
        // The line is where the middle of the view is.
        let line_top = layout_after.line_height * 100.0 - layout_after.scroll_y;
        assert!((line_top + layout_after.line_height / 2.0 - layout_after.content_height / 2.0).abs() < 1.0);
    }

    #[test]
    fn a_range_near_either_end_of_the_text_goes_as_near_the_middle_as_the_text_lets_it() {
        let (field, controller, ctx) = tall_field();
        controller.locate(byte_of_line(&controller, 100)..byte_of_line(&controller, 100) + 4);
        frame(&field, &ctx);
        controller.locate(byte_of_line(&controller, 1)..byte_of_line(&controller, 1) + 4);
        frame(&field, &ctx);
        assert_eq!(field.scroll_y.get(), 0.0, "the second line cannot be in the middle: the top of the text is the top of the view");
        let last = byte_of_line(&controller, 199);
        controller.locate(last..last + 4);
        frame(&field, &ctx);
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        assert!(field.scroll_y.get() + layout.content_height <= layout.line_height * 201.0 + 1.0, "not past the end of the text");
        assert!(field.scroll_y.get() > layout.line_height * 150.0, "and down at the end");
    }

    #[test]
    fn a_located_range_is_marked_for_a_moment_and_the_mark_goes_by_itself() {
        let (field, controller, ctx) = tall_field();
        frame(&field, &ctx);
        assert_eq!(field.pulse_phase(), None, "nothing was located");
        let at = byte_of_line(&controller, 20);
        controller.locate(at..at + 6);
        let early = frame(&field, &ctx);
        let first = field.pulse_phase().expect("the mark has begun");
        assert!((0.0..0.1).contains(&first), "{first}");
        field.test_clock.set(Some(field.test_clock.get().unwrap() + Duration::from_millis(300)));
        frame(&field, &ctx);
        let middle = field.pulse_phase().expect("still going");
        assert!(middle > first && middle < 1.0, "{middle}");
        field.test_clock.set(Some(field.test_clock.get().unwrap() + Duration::from_secs(5)));
        let late = frame(&field, &ctx);
        assert_eq!(field.pulse_phase(), None, "over");
        assert_eq!(early.len(), late.len() + 1, "one rectangle was drawn over the range, and none is drawn for it any more");
    }

    /// The rectangle drawn for the mark: the one with a border, as `(fill, border)`.
    fn mark_of(commands: &[DrawCommand]) -> Option<(aimer_cupid::utilities::Color, aimer_cupid::utilities::Color)> {
        commands.iter().find_map(|command| match command {
            DrawCommand::FillRect { color, border_width, border_color, .. } if border_width.iter().any(|width| *width > 0.0) => Some((*color, *border_color)),
            _ => None,
        })
    }

    #[test]
    fn the_mark_is_in_the_color_the_field_was_given_and_is_filled_with_it_and_fades_with_it() {
        let text: String = (0..60).map(|n| format!("line {n}\n")).collect();
        let controller = TextEditingController::with_text(text);
        let mut config = field_config(controller.clone());
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.locate_color = Some(aimer_widget::base::Color::Rgba(255, 92, 150, 255));
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        field.test_clock.set(Some(AnimInstant::now()));
        let ctx = dummy_build_context(400.0, 300.0);
        frame(&field, &ctx);
        let at = byte_of_line(&controller, 20);
        controller.locate(at..at + 6);
        let (fill, border) = mark_of(&frame(&field, &ctx)).expect("the mark is drawn");
        assert_eq!((border.r, border.g, border.b), (255, 92, 150), "the outline is in the color it was given");
        assert_eq!((fill.r, fill.g, fill.b), (255, 92, 150), "and so is what is inside it");
        assert!(border.a > 240, "the outline is whole as it begins: {}", border.a);
        assert!(fill.a > 0 && fill.a < border.a / 2, "what is inside is faint, and shows the text through it: {}", fill.a);
        field.test_clock.set(Some(field.test_clock.get().unwrap() + Duration::from_millis(400)));
        let (later_fill, later_border) = mark_of(&frame(&field, &ctx)).expect("still drawn");
        assert!(later_border.a < border.a && later_fill.a < fill.a, "both fade: {} {}", later_fill.a, later_border.a);
    }

    #[test]
    fn a_mark_with_no_color_given_is_in_the_color_of_the_caret_or_else_of_the_text() {
        let (field, controller, ctx) = tall_field();
        let at = byte_of_line(&controller, 20);
        controller.locate(at..at + 6);
        let (fill, border) = mark_of(&frame(&field, &ctx)).expect("the mark is drawn");
        let (red, green, blue, _) = field.caret_color.map_or(field.text_style.color, Into::into).to_rgba();
        assert_eq!((border.r, border.g, border.b), (red, green, blue));
        assert_eq!((fill.r, fill.g, fill.b), (red, green, blue));
    }

    #[test]
    fn a_jump_that_lands_while_the_press_that_asked_for_it_is_still_held_is_not_dragged_back_by_the_press() {
        // A force click is a deep press of a finger that is still on the pad: the answer of the language server comes before
        // the finger is lifted, and the finger moves a little as it presses.
        let (field, controller, ctx) = tall_field();
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let pressed = Vec2d { x: layout.padding_left + 20.0, y: layout.padding_top + layout.line_height * 2.5 };
        let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(pressed, PointerButton::Primary)));
        frame(&field, &ctx);
        let target = byte_of_line(&controller, 90);
        controller.locate(target..target + 4);
        frame(&field, &ctx);
        assert_eq!(controller.value().selection().range(), target..target, "the jump put the caret on the name");
        // The finger moves a little, then lifts.
        let moved = Vec2d { x: pressed.x + 3.0, y: pressed.y + 1.0 };
        let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(moved, PointerButton::Primary)));
        frame(&field, &ctx);
        let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(moved, PointerButton::Primary)));
        frame(&field, &ctx);
        assert_eq!(controller.value().selection().range(), target..target, "the caret is still on the name, and nothing is selected from where the press was");
    }

    #[test]
    fn a_request_that_comes_while_the_field_is_built_again_is_not_lost() {
        let (field, controller, ctx) = tall_field();
        let at = byte_of_line(&controller, 60);
        controller.locate(at..at + 5);
        // The request is the controller's, which a rebuilt field has too: whichever field lays out first takes it.
        let mut config = field_config(controller.clone());
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        let rebuilt = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        rebuilt.test_clock.set(Some(AnimInstant::now()));
        rebuilt.adopt_runtime_state_from(&field as &dyn aimer_widget::Element);
        frame(&rebuilt, &ctx);
        assert!(rebuilt.scroll_y.get() > 0.0);
        assert!(rebuilt.pulse_phase().is_some());
    }
}

mod bars {
    use super::*;
    use crate::input_field::gutter::{LineBar, LineBars};
    use aimer_cupid::draw_cmd::Color as DrawColor;
    use aimer_widget::base::Color;

    const BAR: Color = Color::Rgba(10, 200, 30, 255);

    fn bar_color() -> DrawColor {
        let (r, g, b, a) = BAR.to_rgba();
        DrawColor::rgba8(r, g, b, a)
    }

    /// A field whose second line has a bar, and that records whether each line it was asked about was
    /// asked as active.
    fn field(text: &str, wrap: bool) -> (RawTextField, BuildContext<'static>, Rc<std::cell::RefCell<Vec<(usize, bool)>>>) {
        let asked: Rc<std::cell::RefCell<Vec<(usize, bool)>>> = Rc::default();
        let recorded = Rc::clone(&asked);
        let mut config = field_config(TextEditingController::with_text(text));
        config.max_lines = Some(8);
        config.wrap = wrap;
        config.line_bars = Some(LineBars::new(3, move |line, context| {
            recorded.borrow_mut().push((line, context.is_active(&(0..usize::MAX))));
            (line == 1).then(|| LineBar::new(BAR, 5))
        }));
        let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        (field, ctx, asked)
    }

    fn bars(commands: &[DrawCommand]) -> Vec<Rect> {
        commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { rect, color, .. } if *color == bar_color() => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_stands_beside_the_rows_of_the_lines_that_have_one_and_no_others() {
        let long = "word ".repeat(60);
        let text = format!("one\n{long}\nthree");
        let (field, ctx, _) = field(&text, true);
        let rows = field.geometry_cache.latest().expect("laid out").visual_lines.len();
        let commands = commands(&field, &ctx);
        let bars = bars(&commands);
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        let line_height = layout.line_height;
        assert!(rows >= 4, "the second line wraps: {rows} rows");
        // One bar for each row of the second line, which stack into one rule.
        let wrapped = rows - 2;
        assert_eq!(bars.len(), wrapped, "{bars:?}");
        assert!(bars.iter().all(|bar| bar.width == 3.0), "{bars:?}");
        assert!(bars.iter().all(|bar| (bar.x - (layout.padding_left + 5.0)).abs() < 0.6), "5 px in from the text: {bars:?} for padding {}", layout.padding_left);
        for pair in bars.windows(2) {
            assert!((pair[1].y - (pair[0].y + pair[0].height)).abs() < 0.6, "the rows' bars touch: {pair:?}");
        }
        assert!((bars[0].y - (layout.padding_top + line_height)).abs() < 1.5, "the first starts on the second row of the field: {bars:?}");
        assert!((bars[0].height - line_height).abs() < 1.5, "a row tall: {bars:?}");
    }

    #[test]
    fn no_bars_means_no_fills_and_the_closure_sees_the_context_the_text_is_styled_with() {
        let (plain, ctx, _) = field("one\ntwo", true);
        let plain_commands = commands(&plain, &ctx);
        assert_eq!(bars(&plain_commands).len(), 1);

        let mut config = field_config(TextEditingController::with_text("one\ntwo"));
        config.max_lines = Some(8);
        let none = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        let ctx = dummy_build_context(400.0, 300.0);
        none.update(&ctx);
        assert!(bars(&commands(&none, &ctx)).is_empty());
    }

    #[test]
    fn the_closure_is_given_nothing_active_while_the_field_has_no_focus_and_the_caret_line_with_it() {
        let (field, ctx, asked) = field("one\ntwo\nthree", true);
        let _ = commands(&field, &ctx);
        assert!(asked.borrow().iter().all(|(_, active)| !active), "{:?}", asked.borrow());
        asked.borrow_mut().clear();
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.cursor.set_offset(5);
        field.update(&ctx);
        let _ = commands(&field, &ctx);
        assert!(asked.borrow().iter().any(|(_, active)| *active), "a focused field's context is active somewhere: {:?}", asked.borrow());
    }
}

/// A field of a long text laid out as a window much taller than what is on screen paints only the lines
/// in view, and asks about nothing else: a bar or a number for a row that is clipped away is work for
/// nothing, and the window has many such rows on each side.
mod offscreen {
    use super::*;
    use crate::input_field::gutter::{LineBar, LineBars};
    use aimer_widget::base::Color;

    fn windowed(lines: usize) -> (RawTextField, BuildContext<'static>, Rc<std::cell::RefCell<Vec<usize>>>, Rc<std::cell::RefCell<Vec<usize>>>) {
        let text: String = (0..lines).map(|n| format!("let value_{n} = compute({n});\n")).collect();
        let bars: Rc<std::cell::RefCell<Vec<usize>>> = Rc::default();
        let markers: Rc<std::cell::RefCell<Vec<usize>>> = Rc::default();
        let (asked_bars, asked_markers) = (Rc::clone(&bars), Rc::clone(&markers));
        let mut config = field_config(TextEditingController::with_text(text));
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(10);
        config.wrap = false;
        config.line_bars = Some(LineBars::new(3, move |line, _| {
            asked_bars.borrow_mut().push(line);
            Some(LineBar::new(Color::Rgba(10, 200, 30, 255), 0))
        }));
        config.gutter_markers = Some(GutterMarkers::new(16, move |line| {
            asked_markers.borrow_mut().push(line);
            None
        }));
        let mut field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        field.window_threshold = 0;
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(400.0, 300.0);
        field.update(&ctx);
        (field, ctx, bars, markers)
    }

    #[test]
    fn only_the_lines_in_view_are_asked_for_a_bar_or_a_marker() {
        let (field, ctx, bars, markers) = windowed(3000);
        let rows = field.geometry_cache.latest().expect("laid out").visual_lines.len();
        assert!(rows > 30, "the window is much taller than the ten lines in view: {rows} rows");
        bars.borrow_mut().clear();
        markers.borrow_mut().clear();
        let commands = commands(&field, &ctx);
        let (bars, markers) = (bars.borrow(), markers.borrow());
        assert!(!commands.is_empty());
        assert!(!bars.is_empty() && !markers.is_empty(), "what is in view is asked about");
        assert!(bars.iter().all(|line| *line < 14), "bars were asked for lines out of view: {bars:?}");
        assert!(markers.iter().all(|line| *line < 14), "markers were asked for lines out of view: {markers:?}");
    }

    #[test]
    fn a_text_of_a_few_lines_and_the_empty_text_paint_all_of_theirs() {
        let (field, ctx, bars, markers) = windowed(3);
        let _ = commands(&field, &ctx);
        // Three lines and the empty one after the last line break.
        assert_eq!(*bars.borrow().iter().max().expect("asked"), 3, "every line of a short text is in view");
        assert!(markers.borrow().contains(&3));
        let (empty, ctx, _, markers) = windowed(0);
        let _ = commands(&empty, &ctx);
        assert_eq!(*markers.borrow(), vec![0], "an empty text still has its first line");
    }
}
