//! Public-boundary tests for the picker widget adapters.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_cupid::draw_cmd_v2::DrawCommand;
use aimer_events::element::{
    ElementEvent, KeyAction, Modifiers, NamedKey, ScrollDeltaKind, TouchPhase,
};
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_modal::ModalHost;
use aimer_picker::{
    Calendar, CalendarView, ColorPicker, ColorPickerView, Date, DateBounds, DatePicker,
    DatePickerView, DateSelection, DateSelectionMode, DateTimePickerView, Hsva, Swatch, SwatchId,
    TimeOfDay, TimePicker, TimePickerView,
};
use aimer_style::{AnimatedTheme, ThemeData, ThemeTokens};
use aimer_widget::base::Color;
use aimer_widget::base::{BuildContext, Vec2d, WindowHandle};
use aimer_widget::{EventDispatcher, LayoutElement, Widget};

fn context() -> BuildContext<'static> {
    context_with_max(420.0, 420.0)
}

fn context_with_max(max_width: f32, max_height: f32) -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    let mut ctx = BuildContext::new(
        canvas,
        ResolvedSize {
            width: max_width,
            height: max_height,
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
        max_width,
        max_height,
    };
    ctx
}

/// What `element` would paint, as the frame loop records it.
fn painted(element: &aimer_widget::AnyElement, ctx: &BuildContext<'_>) -> Vec<DrawCommand> {
    let recorded = aimer_widget::testing::record_retained_paint(element.as_ref(), ctx);
    aimer_widget::testing::all_commands(&recorded)
        .into_iter()
        .cloned()
        .collect()
}

fn key(key: NamedKey) -> ElementEvent {
    key_with_modifiers(key, Modifiers::default())
}

fn key_with_modifiers(key: NamedKey, modifiers: Modifiers) -> ElementEvent {
    ElementEvent::KeyInput {
        key,
        action: KeyAction::Pressed,
        modifiers,
    }
}

#[tokio::test]
async fn calendar_view_selects_through_its_public_widget_event_boundary() {
    let focused = Date::try_new(2024, 5, 15).unwrap();
    let selected = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&selected);
    let view = CalendarView::new()
        .calendar(
            Calendar::try_new(focused, DateBounds::unbounded(), DateSelectionMode::Single).unwrap(),
        )
        .on_selection(move |selection| observed.borrow_mut().push(selection));

    let ctx = context();
    let element = view.to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                Vec2d::default(),
                &key(NamedKey::ArrowRight)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        selected.borrow().as_slice(),
        &[DateSelection::Single(Some(
            Date::try_new(2024, 5, 16).unwrap()
        ))]
    );
}

#[tokio::test]
async fn calendar_view_consumes_the_provided_surface_token() {
    let custom_surface = Color::Rgba(1, 2, 3, 255);
    let mut tokens = ThemeTokens::light();
    tokens.colors.surface = custom_surface;
    let view = AnimatedTheme::new().data(tokens).child(CalendarView::new());

    let ctx = context();
    let element = view.to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);

    assert!(painted(&element, &ctx).as_slice().iter().any(
        |command| matches!(command, DrawCommand::FillRect { color, .. }
            if color.r == 1
                && color.g == 2
                && color.b == 3
                && color.a == 255)
    ));
}

#[tokio::test]
async fn calendar_range_paints_endpoints_opaque_and_interior_dates_dimmed() {
    let start = Date::try_new(2024, 5, 10).unwrap();
    let end = Date::try_new(2024, 5, 15).unwrap();
    let mut calendar = Calendar::try_new(
        start,
        DateBounds::unbounded(),
        DateSelectionMode::Range(aimer_picker::DateRangePolicy::inclusive()),
    )
    .unwrap();
    calendar.select(start).unwrap();
    calendar.select(end).unwrap();
    let ctx = context();
    let element = CalendarView::new().calendar(calendar).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);

    let selected_alphas = painted(&element, &ctx)
        .as_slice()
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { color, .. }
                if color.r == 255 && color.g == 0 && color.b == 0 =>
            {
                Some(f32::from(color.a) / 255.0)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        selected_alphas
            .iter()
            .filter(|alpha| **alpha > 0.99)
            .count(),
        2
    );
    assert_eq!(
        selected_alphas
            .iter()
            .filter(|alpha| (**alpha - 0.5).abs() < 0.01)
            .count(),
        4
    );
}

#[tokio::test]
async fn calendar_view_bridges_legacy_theme_data_into_semantic_tokens() {
    let custom_surface = Color::Rgba(4, 5, 6, 255);
    let theme = ThemeData::light().surface_color(custom_surface);
    let view = AnimatedTheme::new().data(theme).child(CalendarView::new());

    let ctx = context();
    let element = view.to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);

    assert!(painted(&element, &ctx).as_slice().iter().any(
        |command| matches!(command, DrawCommand::FillRect { color, .. }
            if color.r == 4
                && color.g == 5
                && color.b == 6
                && color.a == 255)
    ));
}

#[tokio::test]
async fn date_picker_view_confirms_a_navigated_date() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let selected = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&selected);
    let view = DatePickerView::new()
        .picker(DatePicker::new(Some(initial)))
        .on_selection(move |selection| observed.borrow_mut().push(selection));

    let ctx = context();
    let element = view.to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                Vec2d::default(),
                &key(NamedKey::ArrowRight)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        selected.borrow().as_slice(),
        &[DateSelection::Single(Some(
            Date::try_new(2024, 5, 16).unwrap()
        ))]
    );
}

#[tokio::test]
async fn date_picker_view_stays_compact_when_its_overlay_is_closed() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let view = DatePickerView::new()
        .picker(DatePicker::new(Some(initial)))
        .height(360.0);
    let ctx = context();
    let element = view.to_element(&ctx);

    let size = element.layout(&ctx);

    assert_eq!(size.height, 42.0);
    let mut dispatcher = EventDispatcher::new();
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    assert_eq!(element.layout(&ctx).height, 42.0);
}

#[tokio::test]
async fn date_time_picker_view_stays_compact_when_its_overlay_is_closed() {
    let view = DateTimePickerView::new().height(360.0);
    let ctx = context();
    let element = view.to_element(&ctx);

    assert_eq!(element.layout(&ctx).height, 42.0);
    let mut dispatcher = EventDispatcher::new();
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    assert_eq!(element.layout(&ctx).height, 42.0);
}

#[tokio::test]
async fn date_time_picker_switches_between_date_and_time_segments() {
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new()
        .child(DateTimePickerView::new().use_24_hours(false))
        .to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    ctx.canvas.begin_frame();
    element.update(&ctx);
    let (has_date, has_time, has_done) = {
        let draw_list = painted(&element, &ctx);
        (
            draw_list.as_slice().iter().any(|command| {
                matches!(
                    command,
                    DrawCommand::DrawText { text, .. } if text.as_ref() == "Date"
                )
            }),
            draw_list.as_slice().iter().any(|command| {
                matches!(
                    command,
                    DrawCommand::DrawText { text, .. } if text.as_ref() == "Time"
                )
            }),
            draw_list.as_slice().iter().any(|command| {
                matches!(
                    command,
                    DrawCommand::DrawText { text, .. } if text.as_ref() == "Done"
                )
            }),
        )
    };
    assert!(has_date);
    assert!(has_time);
    assert!(has_done);

    let time_segment = PointerInfo::mouse(Vec2d { x: 240.0, y: 60.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                time_segment.pos,
                &ElementEvent::PointerDown(time_segment),
            )
            .is_consumed()
    );
    ctx.canvas.begin_frame();
    element.update(&ctx);
    let has_am = {
        let draw_list = painted(&element, &ctx);
        draw_list.as_slice().iter().any(|command| {
            matches!(
                command,
                DrawCommand::DrawText { text, .. } if text.as_ref() == "AM"
            )
        })
    };
    assert!(has_am);

    let date_segment = PointerInfo::mouse(Vec2d { x: 40.0, y: 60.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                date_segment.pos,
                &ElementEvent::PointerDown(date_segment),
            )
            .is_consumed()
    );
    ctx.canvas.begin_frame();
    element.update(&ctx);
    let has_month = {
        let draw_list = painted(&element, &ctx);
        draw_list.as_slice().iter().any(|command| {
            matches!(
                command,
                DrawCommand::DrawText { text, .. } if text.as_ref() == "1970-01"
            )
        })
    };
    assert!(has_month);
}

#[tokio::test]
async fn date_time_picker_scrolls_the_selected_period_column() {
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&values);
    let view =
        DateTimePickerView::new().on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    element.update(&ctx);
    let time_segment = PointerInfo::mouse(Vec2d { x: 240.0, y: 56.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                time_segment.pos,
                &ElementEvent::PointerDown(time_segment),
            )
            .is_consumed()
    );
    element.update(&ctx);

    let period = PointerInfo::mouse(Vec2d { x: 280.0, y: 210.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                period.pos,
                &ElementEvent::PointerDown(period)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                period.pos,
                &ElementEvent::Scroll {
                    delta: Vec2d { x: 0.0, y: -32.0 },
                    phase: TouchPhase::Moved,
                    kind: ScrollDeltaKind::Pixel,
                    is_direct_manipulation: false,
                },
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(values.borrow().len(), 1);
    assert_eq!(values.borrow()[0].unwrap().time(), TimeOfDay::midnight());
}

#[tokio::test]
async fn color_picker_view_stays_compact_when_its_overlay_is_closed() {
    let view = ColorPickerView::new().height(260.0);
    let ctx = context();
    let element = view.to_element(&ctx);

    assert_eq!(element.layout(&ctx).height, 42.0);
    let mut dispatcher = EventDispatcher::new();
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    assert_eq!(element.layout(&ctx).height, 42.0);
}

#[tokio::test]
async fn date_time_picker_overlay_paints_a_separating_border() {
    let ctx = context();
    let element = ModalHost::new()
        .child(DateTimePickerView::new())
        .to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    ctx.canvas.begin_frame();
    element.update(&ctx);

    let outline = ThemeTokens::light().colors.outline;
    let (outline_r, outline_g, outline_b, outline_a) = outline.to_rgba();
    assert!(
        painted(&element, &ctx)
            .as_slice()
            .iter()
            .any(|command| matches!(
                command,
                DrawCommand::FillRect {
                    rect,
                    border_width,
                    border_color,
                    ..
                } if rect.height > 42.0
                    && border_width.iter().all(|width| *width > 0.0)
                    && border_color.r == outline_r
                    && border_color.g == outline_g
                    && border_color.b == outline_b
                    && border_color.a == outline_a
            ))
    );
}

#[tokio::test]
async fn color_picker_overlay_paints_a_separating_border() {
    let ctx = context();
    let element = ModalHost::new()
        .child(ColorPickerView::new())
        .to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();
    let open = PointerInfo::mouse(Vec2d { x: 10.0, y: 10.0 }, PointerButton::Primary);

    assert!(
        dispatcher
            .dispatch(element.as_ref(), open.pos, &ElementEvent::PointerDown(open))
            .is_consumed()
    );
    element.update(&ctx);

    let outline = ThemeTokens::light().colors.outline;
    let (outline_r, outline_g, outline_b, outline_a) = outline.to_rgba();
    assert!(
        painted(&element, &ctx)
            .as_slice()
            .iter()
            .any(|command| matches!(
                command,
                DrawCommand::FillRect {
                    rect,
                    border_width,
                    border_color,
                    ..
                } if rect.height > 42.0
                    && border_width.iter().all(|width| *width > 0.0)
                    && border_color.r == outline_r
                    && border_color.g == outline_g
                    && border_color.b == outline_b
                    && border_color.a == outline_a
            ))
    );
}

#[tokio::test]
async fn date_time_and_color_views_open_edit_and_confirm_through_keyboard() {
    let date_time_values = Rc::new(RefCell::new(Vec::new()));
    let observed_date_time = Rc::clone(&date_time_values);
    let date_time = DateTimePickerView::new()
        .use_24_hours(true)
        .on_selection(move |value| observed_date_time.borrow_mut().push(value));
    let color_values = Rc::new(RefCell::new(Vec::new()));
    let observed_color = Rc::clone(&color_values);
    let color = ColorPickerView::new()
        .picker(ColorPicker::new(
            Hsva::try_new(0, 100, 100, 100).unwrap(),
            true,
        ))
        .on_selection(move |value| observed_color.borrow_mut().push(value));

    let ctx = context();
    let date_time_element = date_time.to_element(&ctx);
    date_time_element.layout(&ctx);
    date_time_element.update(&ctx);
    let color_element = color.to_element(&ctx);
    color_element.layout(&ctx);
    color_element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    for event in [
        key(NamedKey::Enter),
        key_with_modifiers(
            NamedKey::ArrowRight,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ),
        key(NamedKey::ArrowUp),
        key_with_modifiers(
            NamedKey::ArrowRight,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ),
        key(NamedKey::ArrowRight),
        key_with_modifiers(
            NamedKey::ArrowRight,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ),
        key(NamedKey::ArrowUp),
        key_with_modifiers(
            NamedKey::ArrowRight,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ),
        key(NamedKey::ArrowRight),
        key(NamedKey::Enter),
    ] {
        assert!(
            dispatcher
                .dispatch(date_time_element.as_ref(), Vec2d::default(), &event)
                .is_consumed()
        );
    }
    for event in [
        key(NamedKey::Enter),
        key(NamedKey::ArrowRight),
        key(NamedKey::Enter),
    ] {
        assert!(
            dispatcher
                .dispatch(color_element.as_ref(), Vec2d::default(), &event)
                .is_consumed()
        );
    }

    assert_eq!(date_time_values.borrow().len(), 1);
    assert_eq!(
        date_time_values.borrow()[0].unwrap().date(),
        Date::try_new(1970, 1, 2).unwrap()
    );
    assert_eq!(
        date_time_values.borrow()[0].unwrap().time(),
        TimeOfDay::try_new(1, 1, 1, 0).unwrap()
    );
    assert_eq!(
        color_values.borrow().as_slice(),
        &[Hsva::try_new(1, 100, 100, 100).unwrap()]
    );
}

#[tokio::test]
async fn date_time_picker_view_selects_an_hour_from_the_time_wheel() {
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&values);
    let view =
        DateTimePickerView::new().on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    element.update(&ctx);
    let time_field = PointerInfo::mouse(Vec2d { x: 240.0, y: 56.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                time_field.pos,
                &ElementEvent::PointerDown(time_field),
            )
            .is_consumed()
    );

    let next_hour = PointerInfo::mouse(Vec2d { x: 40.0, y: 236.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                next_hour.pos,
                &ElementEvent::PointerDown(next_hour),
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        values.borrow().as_slice(),
        &[Some(
            aimer_picker::DateTime::try_new(
                Date::try_new(1970, 1, 1).unwrap(),
                TimeOfDay::try_new(1, 0, 0, 0).unwrap(),
                aimer_picker::TimeZonePolicy::Utc,
            )
            .unwrap(),
        )]
    );
}

#[tokio::test]
async fn time_picker_view_scrolls_a_24_hour_column_and_confirms_it() {
    let initial = TimeOfDay::try_new(9, 30, 0, 0).unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&values);
    let view = TimePickerView::new()
        .picker(TimePicker::new(Some(initial)))
        .height(300.0)
        .use_24_hours(true)
        .on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    element.update(&ctx);

    let draw_list = painted(&element, &ctx);
    let texts = draw_list
        .as_slice()
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { text, .. } => Some(text.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(texts.iter().any(|text| *text == "09"));
    assert!(!texts.iter().any(|text| *text == "AM" || *text == "PM"));

    let down = PointerInfo::mouse(Vec2d { x: 45.0, y: 166.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(element.as_ref(), down.pos, &ElementEvent::PointerDown(down))
            .is_consumed()
    );
    let move_to = PointerInfo::mouse(Vec2d { x: 45.0, y: 102.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                move_to.pos,
                &ElementEvent::PointerMove(move_to)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                move_to.pos,
                &ElementEvent::PointerUp(move_to)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        values.borrow().as_slice(),
        &[TimeOfDay::try_new(11, 30, 0, 0).unwrap()]
    );
}

#[tokio::test]
async fn time_picker_view_applies_a_smooth_scroll_frame_to_the_active_column() {
    let initial = TimeOfDay::try_new(9, 30, 0, 0).unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&values);
    let view = TimePickerView::new()
        .picker(TimePicker::new(Some(initial)))
        .use_24_hours(true)
        .on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    element.update(&ctx);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                Vec2d { x: 45.0, y: 166.0 },
                &ElementEvent::Scroll {
                    delta: Vec2d { x: 0.0, y: -32.0 },
                    phase: TouchPhase::Moved,
                    kind: ScrollDeltaKind::Pixel,
                    is_direct_manipulation: false,
                },
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        values.borrow().as_slice(),
        &[TimeOfDay::try_new(10, 30, 0, 0).unwrap()]
    );
}

#[tokio::test]
async fn time_picker_view_scrolls_the_column_under_the_pointer() {
    let initial = TimeOfDay::try_new(9, 30, 0, 0).unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&values);
    let view = TimePickerView::new()
        .picker(TimePicker::new(Some(initial)))
        .use_24_hours(true)
        .on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    element.update(&ctx);
    let hover = PointerInfo::mouse(Vec2d { x: 150.0, y: 166.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                hover.pos,
                &ElementEvent::PointerMove(hover)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                hover.pos,
                &ElementEvent::Scroll {
                    delta: Vec2d { x: 0.0, y: -32.0 },
                    phase: TouchPhase::Moved,
                    kind: ScrollDeltaKind::Pixel,
                    is_direct_manipulation: false,
                },
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        values.borrow().as_slice(),
        &[TimeOfDay::try_new(9, 31, 0, 0).unwrap()]
    );
}

#[tokio::test]
async fn time_picker_uses_one_period_label_and_allows_switching_to_pm() {
    let initial = TimeOfDay::try_new(1, 30, 0, 0).unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&values);
    let view = TimePickerView::new()
        .picker(TimePicker::new(Some(initial)))
        .use_24_hours(false)
        .on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context_with_max(420.0, 1000.0);
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );
    element.update(&ctx);
    let period_count = {
        let draw_list = painted(&element, &ctx);
        draw_list
            .as_slice()
            .iter()
            .filter(|command| matches!(
                command,
                DrawCommand::DrawText { text, .. } if text.as_ref() == "AM" || text.as_ref() == "PM"
            ))
            .count()
    };
    assert_eq!(period_count, 1);

    let period = PointerInfo::mouse(Vec2d { x: 280.0, y: 166.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                period.pos,
                &ElementEvent::PointerDown(period)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                period.pos,
                &ElementEvent::PointerUp(period)
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(
        values.borrow().as_slice(),
        &[TimeOfDay::try_new(13, 30, 0, 0).unwrap()]
    );
}

#[tokio::test]
async fn color_picker_view_routes_pointer_swatches_through_the_public_boundary() {
    let disabled = Swatch::new(
        SwatchId::new(1),
        Hsva::try_new(120, 100, 100, 100).unwrap(),
        true,
    );
    let enabled_color = Hsva::try_new(240, 100, 100, 100).unwrap();
    let mut picker = ColorPicker::new(Hsva::try_new(0, 100, 100, 100).unwrap(), true);
    picker.add_swatch(disabled).unwrap();
    picker
        .add_swatch(Swatch::new(SwatchId::new(2), enabled_color, false))
        .unwrap();

    let selected = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&selected);
    let view = ColorPickerView::new()
        .picker(picker)
        .on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context();
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    let open = PointerInfo::mouse(Vec2d { x: 10.0, y: 10.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(element.as_ref(), open.pos, &ElementEvent::PointerDown(open))
            .is_consumed()
    );
    element.update(&ctx);
    let disabled_press = PointerInfo::mouse(Vec2d { x: 5.0, y: 136.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                disabled_press.pos,
                &ElementEvent::PointerDown(disabled_press),
            )
            .is_consumed()
    );
    let enabled_press = PointerInfo::mouse(Vec2d { x: 37.0, y: 136.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                enabled_press.pos,
                &ElementEvent::PointerDown(enabled_press),
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(element.as_ref(), Vec2d::default(), &key(NamedKey::Enter))
            .is_consumed()
    );

    assert_eq!(selected.borrow().as_slice(), &[enabled_color]);
}

#[tokio::test]
async fn color_picker_view_selects_a_value_through_a_channel_slider_drag() {
    let selected = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&selected);
    let view = ColorPickerView::new()
        .picker(ColorPicker::new(
            Hsva::try_new(0, 100, 100, 100).unwrap(),
            true,
        ))
        .on_selection(move |value| observed.borrow_mut().push(value));
    let ctx = context();
    let element = ModalHost::new().child(view).to_element(&ctx);
    element.layout(&ctx);
    element.update(&ctx);
    let mut dispatcher = EventDispatcher::new();

    let open = PointerInfo::mouse(Vec2d { x: 10.0, y: 10.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(element.as_ref(), open.pos, &ElementEvent::PointerDown(open))
            .is_consumed()
    );
    element.update(&ctx);
    let hue_track = PointerInfo::mouse(Vec2d { x: 24.0, y: 162.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                hue_track.pos,
                &ElementEvent::PointerDown(hue_track),
            )
            .is_consumed()
    );
    let hue_drag = PointerInfo::mouse(Vec2d { x: 256.0, y: 162.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                hue_drag.pos,
                &ElementEvent::PointerMove(hue_drag),
            )
            .is_consumed()
    );
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                hue_drag.pos,
                &ElementEvent::PointerUp(hue_drag),
            )
            .is_consumed()
    );
    let confirm = PointerInfo::mouse(Vec2d { x: 240.0, y: 280.0 }, PointerButton::Primary);
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                confirm.pos,
                &ElementEvent::PointerDown(confirm),
            )
            .is_consumed()
    );

    assert_eq!(
        selected.borrow().as_slice(),
        &[Hsva::try_new(360, 100, 100, 100).unwrap()]
    );
}

#[tokio::test]
async fn calendar_view_maps_pointer_selection_to_its_clamped_layout_size() {
    let focused = Date::try_new(2024, 5, 15).unwrap();
    let selected = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&selected);
    let view = CalendarView::new()
        .calendar(
            Calendar::try_new(focused, DateBounds::unbounded(), DateSelectionMode::Single).unwrap(),
        )
        .width(320.0)
        .height(300.0)
        .on_selection(move |selection| observed.borrow_mut().push(selection));

    let ctx = context_with_max(140.0, 300.0);
    let element = view.to_element(&ctx);
    let size = element.layout(&ctx);
    element.update(&ctx);
    assert_eq!(size.width, 140.0);

    // May 15, 2024 is row 2 / column 2 in the Monday-first six-week grid.
    // These coordinates are inside the actual clamped 140px-wide layout.
    let pointer = PointerInfo::mouse(Vec2d { x: 50.0, y: 159.0 }, PointerButton::Primary);
    let mut dispatcher = EventDispatcher::new();
    assert!(
        dispatcher
            .dispatch(
                element.as_ref(),
                pointer.pos,
                &ElementEvent::PointerDown(pointer),
            )
            .is_consumed()
    );

    assert_eq!(
        selected.borrow().as_slice(),
        &[DateSelection::Single(Some(focused))]
    );
}
