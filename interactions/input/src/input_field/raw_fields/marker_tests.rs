//! The icons in the margin before the line numbers: drawn on their lines, and pressed like buttons.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use aimer_attribute::Vec2d;
use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_cupid::svg::{SvgScene, SvgViewport};
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_events::element::ElementEvent;
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

fn commands(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<DrawCommand> {
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

    #[test]
    fn a_field_that_did_not_ask_for_a_force_click_leaves_it_alone() {
        let (plain, _, ctx) = super::field(false);
        let at = Vec2d { x: 100.0, y: 10.0 };
        assert!(!plain.on_event(&ElementEvent::Pressure { pos: at, pressure: 1.0, stage: PressureStage::Force }).is_consumed());
        let _ = ctx;
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
