// Element tests for texts taking part in a shared [`SelectionSession`], the
// multi-widget behaviour a [`SelectionArea`](crate::SelectionArea)
// establishes.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::{Bounds, Vec2d};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
use aimer_style::{TextAlign, TextOverflow, TextStyle};
use aimer_utils::AnimInstant;
use aimer_widget::base::WindowHandle;
use aimer_widget::{
    CaptureRequest, Element, EventDispatcher, EventElement, PointerKey, Rebuildable,
    is_pointer_claimed, release_all_pointers,
};

use super::selected;
use crate::paragraph::Paragraph;
use crate::rich_text::{DEFAULT_SELECTION_COLOR, LinkCallback, RawRichText, SelectionBinding};
use crate::selection::TextHitRegion;
use crate::selection::cursor::HoverCursor;
use crate::selection::selectable::{SelectionCoordinator, TextGeometry};
use crate::selection::session::SelectionSession;
use crate::selection::touch_hold::{TOUCH_SELECTION_HOLD, TouchHoldGate};
use crate::text::selectable_text::RawSelectableText;
use crate::text_span::ResolvedTextSpan;

/// Builds one selectable element per string, all sharing a single session,
/// stacked vertically thirty pixels apart and each already carrying the
/// geometry of a painted frame: one ten-by-twenty box per character.
fn region_texts(texts: &[&str]) -> (Rc<SelectionSession>, Vec<RawRichText>) {
    let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0);
    let session = SelectionSession::new(
        window.clone(),
        Rc::new(SelectionCoordinator::default()),
        DEFAULT_SELECTION_COLOR,
    );
    let elements = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let plain: Rc<str> = Rc::from(*text);
            let top = index as f32 * 30.0;
            let geometry = Rc::new(TextGeometry::new(window.clone()));
            let slot = session.register(Rc::clone(&plain), Rc::downgrade(&geometry) as _);
            slot.stamp();
            let element = RawRichText {
                paragraph: Paragraph::new(
                    vec![ResolvedTextSpan::plain(
                        Rc::clone(&plain),
                        TextStyle::default(),
                    )],
                    TextAlign::TopLeft,
                    TextOverflow::Clip,
                ),
                plain_text: Rc::clone(&plain),
                on_link: LinkCallback::default(),
                link_hover_color: None,
                selectable: true,
                selection_color: DEFAULT_SELECTION_COLOR,
                binding: RefCell::new(SelectionBinding {
                    geometry: Rc::clone(&geometry),
                    session: Rc::clone(&session),
                    slot,
                    owns_session: false,
                }),
                link_regions: RefCell::new(Vec::new()),
                pressed_link: RefCell::new(None),
                hovered_link: RefCell::new(None),
                hover_cursor: HoverCursor::new(),
                touch_hold: TouchHoldGate::new(),
                focus_node: aimer_widget::FocusNode::new(),
            };
            paint_character_grid(&geometry, &plain, top);
            element
        })
        .collect();
    // The participants painted, so the region opens the next frame and their
    // draw order becomes the document order.
    session.begin_frame();
    (session, elements)
}

/// One ten-by-twenty hit box per character, on the line starting at `top`.
fn character_regions(text: &str, top: f32) -> Vec<TextHitRegion> {
    text.char_indices()
        .enumerate()
        .map(|(column, (offset, character))| {
            TextHitRegion::new(
                offset..offset + character.len_utf8(),
                Bounds::new(column as f32 * 10.0, top, 10.0, 20.0),
            )
        })
        .collect()
}

/// The bounds those boxes add up to.
fn character_bounds(text: &str, top: f32) -> Bounds {
    Bounds::new(0.0, top, text.chars().count() as f32 * 10.0, 20.0)
}

/// Fills `geometry` as if the text had painted that grid.
fn paint_character_grid(geometry: &Rc<TextGeometry>, text: &Rc<str>, top: f32) {
    let bounds = character_bounds(text, top);
    *geometry.regions.borrow_mut() = character_regions(text, top);
    geometry
        .bounds
        .save(1.0, bounds.x, bounds.y, bounds.width, bounds.height);
}

fn mouse_at(x: f32, y: f32) -> PointerInfo {
    PointerInfo::new(
        Vec2d { x, y },
        PointerSource::Mouse,
        0,
        PointerButton::Primary,
    )
}

fn touch_at(x: f32, y: f32, id: u64) -> PointerInfo {
    PointerInfo::new(
        Vec2d { x, y },
        PointerSource::Touch,
        id,
        PointerButton::Primary,
    )
}

fn shortcut(key: &str) -> ElementEvent {
    ElementEvent::KeyInput {
        key: NamedKey::Other(key.into()),
        action: KeyAction::Pressed,
        modifiers: Modifiers {
            ctrl: true,
            ..Default::default()
        },
    }
}

#[test]
fn a_drag_across_three_texts_selects_suffix_all_and_prefix() {
    let (_session, texts) = region_texts(&["first", "second", "third"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(25.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(25.0, 70.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(25.0, 70.0)));

    assert_eq!(selected(&texts[0]), Some(3..5));
    assert_eq!(selected(&texts[1]), Some(0..6));
    assert_eq!(selected(&texts[2]), Some(0..3));
}

#[test]
fn a_reversed_drag_selects_the_same_ranges() {
    let (_session, texts) = region_texts(&["first", "second", "third"]);

    let _ = texts[2].on_event(&ElementEvent::PointerDown(mouse_at(25.0, 70.0)));
    let _ = texts[2].on_event(&ElementEvent::PointerMove(mouse_at(25.0, 10.0)));
    let _ = texts[2].on_event(&ElementEvent::PointerUp(mouse_at(25.0, 10.0)));

    assert_eq!(selected(&texts[0]), Some(3..5));
    assert_eq!(selected(&texts[1]), Some(0..6));
    assert_eq!(selected(&texts[2]), Some(0..3));
}

#[test]
fn a_drag_that_moves_through_the_gap_keeps_extending() {
    let (_session, texts) = region_texts(&["first", "second"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(55.0, 25.0)));

    assert_eq!(selected(&texts[0]).map(|range| range.end), Some(5));
    assert_eq!(selected(&texts[1]).map(|range| range.start), Some(0));
}

#[test]
fn a_drag_far_below_the_region_clamps_to_the_last_participant() {
    let (_session, texts) = region_texts(&["first", "second"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(500.0, 900.0)));

    assert_eq!(selected(&texts[0]), Some(0..5));
    assert_eq!(selected(&texts[1]), Some(0..6));
}

#[test]
fn copied_text_follows_draw_order_and_joins_participants_with_a_newline() {
    let (session, texts) = region_texts(&["first", "second"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(500.0, 900.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(500.0, 900.0)));

    assert_eq!(session.selected_text(), "first\nsecond");
}

#[test]
fn copy_order_follows_the_draw_stamp_not_the_registration_order() {
    let (session, texts) = region_texts(&["first", "second"]);

    texts[1].slot().stamp();
    texts[0].slot().stamp();
    session.begin_frame();
    session.select_all();

    assert_eq!(session.selected_text(), "second\nfirst");
}

#[test]
fn a_second_pointer_over_another_participant_does_not_steal_the_drag() {
    let (_session, texts) = region_texts(&["first", "second"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[1].on_event(&ElementEvent::PointerMove(touch_at(35.0, 40.0, 1)));

    assert_eq!(selected(&texts[0]), Some(0..0));
    assert_eq!(selected(&texts[1]), None);
}

/// Presses `info` and rewinds the press past the hold, which is where a finger
/// that rested on the text stands.
fn hold(text: &RawRichText, info: PointerInfo) {
    let _ = text.on_event(&ElementEvent::PointerDown(info));
    text.touch_hold.backdate(TOUCH_SELECTION_HOLD);
}

#[test]
fn a_touch_press_selects_nothing_until_it_has_been_held() {
    let (_session, texts) = region_texts(&["first", "second"]);

    let result = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));

    assert_eq!(selected(&texts[0]), None, "a finger must rest first");
    assert!(
        !result.is_consumed(),
        "an unconsumed press is what leaves the scroll gesture possible"
    );
}

#[test]
fn a_touch_that_moves_away_before_the_hold_was_a_scroll() {
    let (_session, texts) = region_texts(&["first", "second"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    let result = texts[0].on_event(&ElementEvent::PointerMove(touch_at(25.0, 60.0, 0)));

    assert_eq!(selected(&texts[0]), None);
    assert!(!result.is_consumed());
}

#[test]
fn a_short_touch_tap_selects_nothing() {
    let (_session, texts) = region_texts(&["first", "second"]);

    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));

    assert_eq!(selected(&texts[0]), None);
}

#[test]
fn a_completed_touch_hold_selects_the_word_under_the_finger() {
    let (session, texts) = region_texts(&["first", "second"]);

    hold(&texts[0], touch_at(25.0, 10.0, 0));
    let result = texts[0].on_event(&ElementEvent::PointerMove(touch_at(25.0, 10.0, 0)));

    assert_eq!(selected(&texts[0]), Some(0..5));
    assert_eq!(session.selected_text(), "first");
    assert!(
        result.is_consumed(),
        "the finger now owns the gesture, so the scrollable must not have it"
    );
}

#[test]
fn a_touch_hold_released_where_it_started_still_selects_the_word() {
    let (session, texts) = region_texts(&["first", "second"]);

    hold(&texts[0], touch_at(25.0, 10.0, 0));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));

    assert_eq!(selected(&texts[0]), Some(0..5));
    assert_eq!(session.selected_text(), "first");
}

#[test]
fn a_finger_that_kept_moving_after_the_hold_extends_across_participants() {
    let (session, texts) = region_texts(&["first", "second"]);

    hold(&texts[0], touch_at(25.0, 10.0, 0));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(touch_at(25.0, 10.0, 0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(touch_at(35.0, 40.0, 0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(touch_at(35.0, 40.0, 0)));

    assert_eq!(selected(&texts[0]), Some(0..5));
    assert_eq!(selected(&texts[1]), Some(0..4));
    assert_eq!(session.selected_text(), "first\nseco");
}

#[test]
fn a_cancelled_gesture_forgets_a_touch_press_that_never_became_a_selection() {
    let (_session, texts) = region_texts(&["first", "second"]);

    hold(&texts[0], touch_at(25.0, 10.0, 0));
    let _ = texts[0].on_event(&ElementEvent::Cancel);
    let _ = texts[0].on_event(&ElementEvent::PointerMove(touch_at(25.0, 10.0, 0)));

    assert_eq!(selected(&texts[0]), None);
}

#[test]
fn the_region_selects_and_copies_every_participant_on_the_keyboard() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(2.0, 10.0)));

    session.select_all();

    assert_eq!(selected(&texts[0]), Some(0..5));
    assert_eq!(selected(&texts[1]), Some(0..6));
    assert_eq!(session.selected_text(), "first\nsecond");
}

#[test]
fn a_participant_inside_a_region_does_not_answer_the_select_all_shortcut_itself() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(2.0, 10.0)));

    assert!(!texts[0].on_event(&shortcut("a")).is_consumed());
    assert_eq!(selected(&texts[1]), None);
    assert!(session.is_focused());
}

#[test]
fn the_region_clears_the_selection_when_a_click_hits_no_participant() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(500.0, 900.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(500.0, 900.0)));

    session.clear();

    assert_eq!(selected(&texts[0]), None);
    assert_eq!(selected(&texts[1]), None);
    assert_eq!(session.selected_text(), "");
}

#[test]
fn cancelling_a_drag_restores_the_previous_multi_widget_selection() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(500.0, 900.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(500.0, 900.0)));

    let _ = texts[1].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 40.0)));
    let _ = texts[1].on_event(&ElementEvent::PointerMove(mouse_at(35.0, 40.0)));
    let _ = texts[1].on_event(&ElementEvent::Cancel);

    assert_eq!(session.selected_text(), "first\nsecond");
}

#[test]
fn a_participant_rebuilt_with_shorter_text_clamps_the_live_selection() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(500.0, 900.0)));

    texts[1].slot().set_text(Rc::from("no"));

    assert_eq!(selected(&texts[1]), Some(0..2));
    assert_eq!(session.selected_text(), "first\nno");
}

#[test]
fn a_drag_from_a_plain_text_into_a_rich_text_selects_across_both() {
    let (session, texts) = region_texts(&["second"]);
    let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0);
    let plain = "first";
    let text = RawSelectableText::painted(
        &session,
        &window,
        Rc::from(plain),
        character_regions(plain, 60.0),
        character_bounds(plain, 60.0),
    );
    // The plain text paints last, so it comes last in document order.
    texts[0].slot().stamp();
    text.slot().stamp();
    session.begin_frame();

    let _ = text.on_event(&ElementEvent::PointerDown(mouse_at(25.0, 70.0)));
    let _ = text.on_event(&ElementEvent::PointerMove(mouse_at(25.0, 10.0)));
    let _ = text.on_event(&ElementEvent::PointerUp(mouse_at(25.0, 10.0)));

    assert_eq!(selected(&texts[0]), Some(3..6));
    assert_eq!(text.slot().selected_range(), Some(0..3));
    assert_eq!(session.selected_text(), "ond\nfir");
}

#[test]
fn a_plain_text_also_waits_for_the_hold_before_selecting() {
    let (session, _texts) = region_texts(&[]);
    let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0);
    let plain = "first second";
    let text = RawSelectableText::painted(
        &session,
        &window,
        Rc::from(plain),
        character_regions(plain, 0.0),
        character_bounds(plain, 0.0),
    );
    text.slot().stamp();
    session.begin_frame();

    let _ = text.on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    assert_eq!(
        text.slot().selected_range(),
        None,
        "a finger must rest before a plain text selects either"
    );

    text.touch_hold.backdate(TOUCH_SELECTION_HOLD);
    let _ = text.on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));

    assert_eq!(session.selected_text(), "first");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_stationary_touch_hold_selects_without_another_pointer_event() {
    use std::cell::Cell;

    use aimer_attribute::ResolvedSize;
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_widget::Drawable;
    use aimer_widget::base::BuildContext;

    let redraws = Rc::new(Cell::new(0));
    let counted = Rc::clone(&redraws);
    let previous = aimer_events::window::set_thread_redraw_requester(move || {
        counted.set(counted.get() + 1);
    });
    let inner = InnerCanvas::new();
    let canvas = FrameCanvas::new(&inner);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0);
    let context = BuildContext::new(
        canvas,
        ResolvedSize {
            width: 200.0,
            height: 100.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        window.clone(),
        runtime.handle().clone(),
    );
    let session = SelectionSession::new(
        window.clone(),
        Rc::new(SelectionCoordinator::default()),
        DEFAULT_SELECTION_COLOR,
    );
    let plain = "first second";
    let text = RawSelectableText::painted(
        &session,
        &window,
        Rc::from(plain),
        character_regions(plain, 0.0),
        character_bounds(plain, 0.0),
    );

    let _ = text.on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    text.update(&context);
    assert_eq!(session.selected_text(), "");
    text.touch_hold.backdate(TOUCH_SELECTION_HOLD);

    text.update(&context);

    assert_eq!(session.selected_text(), "first");
    let _ = text.on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));
    assert_eq!(
        session.selected_text(),
        "first",
        "lifting a stationary hold must preserve the selected word"
    );
    assert!(
        redraws.get() >= 2,
        "the press and each waiting frame must keep the hold timer advancing"
    );
    aimer_events::window::restore_thread_redraw_requester(previous);
}

#[test]
fn dragging_a_knob_that_sits_over_the_glyphs_adjusts_the_selection_instead_of_restarting_it() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    texts[0].touch_hold.backdate(TOUCH_SELECTION_HOLD);
    let _ = texts[0].on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));
    assert_eq!(selected(&texts[0]), Some(0..5), "the hold selected the word");

    let (_, end) = session.handle_circles().expect("a touch selection has knobs");
    let grab = texts[0].on_event(&ElementEvent::PointerDown(touch_at(
        end.center_x,
        end.center_y,
        0,
    )));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(touch_at(25.0, 40.0, 0)));

    assert!(grab.is_consumed(), "the knob takes the press");
    assert_eq!(selected(&texts[0]), Some(0..5), "the start stayed put");
    assert_eq!(selected(&texts[1]), Some(0..3), "the end moved on");
}

/// A finger press is ambiguous, so the text takes the *pointer* without taking
/// the *gesture*: owning the pointer is the only way it hears an enclosing
/// scrollable take the gesture away, while leaving the gesture unclaimed is what
/// lets that scrollable take it at all.
#[test]
fn a_finger_press_takes_the_pointer_but_not_the_gesture() {
    // Nothing is on the glass when a gesture begins, whatever earlier gestures
    // of this thread left behind.
    release_all_pointers();
    let (_session, texts) = region_texts(&["first", "second"]);
    let pointer = PointerKey::new(PointerSource::Touch, 0);

    let press = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));

    assert!(
        !press.is_consumed(),
        "the press must go on reaching the region behind the text"
    );
    assert_eq!(
        press.capture_request(),
        CaptureRequest::Capture(pointer),
        "owning the pointer is how the text is told the gesture was taken from it"
    );
    assert!(
        !is_pointer_claimed(pointer),
        "a press that may still turn out to be a scroll must not speak for the gesture"
    );
}

/// The gesture a scroll view takes over.
///
/// A press inside a `Scrollable` could be a tap, a hold, or the start of a
/// scroll, so the view arms a pending drag and — once the finger has travelled
/// past the drag threshold — takes the gesture by cancelling the pointer of
/// whichever element owns it. That call reaches the owner of the pointer and
/// nobody else, so a text that recorded the press without taking the pointer is
/// never told: its hold goes on ripening and turns into a selection on some
/// later frame, from a finger that left the glass long ago.
#[test]
fn a_scroll_view_that_takes_the_gesture_puts_the_hold_out() {
    let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0);
    let session = SelectionSession::new(
        window.clone(),
        Rc::new(SelectionCoordinator::default()),
        DEFAULT_SELECTION_COLOR,
    );
    let plain = "first second";
    // Erased, as the tree stores it: an element carries the identity a capture
    // is recorded against only once it has been.
    let root = RawSelectableText::painted(
        &session,
        &window,
        Rc::from(plain),
        character_regions(plain, 0.0),
        character_bounds(plain, 0.0),
    )
    .boxed();
    let text = root
        .option_any()
        .and_then(|element| element.downcast_ref::<RawSelectableText>())
        .expect("the erased element is the selectable text");
    text.slot().stamp();
    session.begin_frame();
    let finger = touch_at(25.0, 10.0, 0);
    let pointer = PointerKey::new(finger.source, finger.id);

    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(root.as_ref(), finger.pos, &ElementEvent::PointerDown(finger));
    // The finger travelled: the scroll view wins the drag and takes the gesture
    // from whoever owns the pointer, exactly as `aimer_scroll` does.
    let _ = dispatcher.cancel_pointer(root.as_ref(), pointer);

    text.touch_hold.backdate(TOUCH_SELECTION_HOLD);
    assert_eq!(
        // The page itself never moved: this text paints where it did when the
        // finger landed, so the cancelled capture is the whole of the evidence.
        text.touch_hold
            .poll_stationary(AnimInstant::now(), Vec2d::ZERO),
        None,
        "the gesture is the scroll view's, so no frame may turn the hold into a selection"
    );
    assert_eq!(session.selected_text(), "");
}

/// The page that slid out from under the finger.
///
/// A scroll view revokes the press it took the gesture from — but only when the
/// gesture it took was the finger's *drag*. A page also travels under a finger
/// that never reported a move: a touch browser scrolls by handing the
/// application scroll deltas, momentum carries a page on by itself, and an
/// animation moves a paragraph for reasons of its own. The frames of a text
/// carried out of view stop coming, too, so the release is the next thing to
/// judge the press — and by elapsed time alone it is a completed hold, which is
/// how a scroll ends with a word selected in text the user can no longer see.
#[test]
fn a_finger_whose_paragraph_slid_away_selects_nothing_when_it_lifts() {
    let (session, texts) = region_texts(&["first second", "third"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    texts[0].touch_hold.backdate(TOUCH_SELECTION_HOLD);

    // The page scrolled: this paragraph is painted somewhere else now. Nothing
    // told the text so — no finger move, no cancelled capture — and no frame of
    // its own judged the press, because a text carried out of view is not drawn.
    let geometry = Rc::clone(&texts[0].binding.borrow().geometry);
    paint_character_grid(&geometry, &texts[0].plain_text, -40.0);

    let _ = texts[0].on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));

    assert_eq!(
        session.selected_text(),
        "",
        "the finger was scrolling, so its release selects nothing"
    );
    assert!(
        !is_pointer_claimed(PointerKey::new(PointerSource::Touch, 0)),
        "and no gesture was opened for a release to leave behind"
    );
}

/// The gesture that never ends.
///
/// A release is not guaranteed to arrive: a scroll view that has taken over one
/// finger swallows the lift of another, and an element torn down mid-gesture
/// never hears one at all. Touch identities are reused, so the next press of
/// "finger 0" would look like the press that opened that gesture — and a press
/// mistaken for its own gesture dismisses nothing, which is a selection that can
/// never be got rid of. The claim the framework drops the moment a pointer goes
/// up tells the two apart.
#[test]
fn a_gesture_whose_finger_left_the_glass_no_longer_owns_the_pointer() {
    let (session, texts) = region_texts(&["first", "second"]);
    let pointer = PointerKey::new(PointerSource::Touch, 0);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    texts[0].touch_hold.backdate(TOUCH_SELECTION_HOLD);
    let _ = texts[0].on_event(&ElementEvent::PointerMove(touch_at(25.0, 10.0, 0)));
    assert_eq!(
        session.active_pointer(),
        Some(pointer),
        "the hold opened a gesture the finger owns"
    );

    release_all_pointers();

    assert_eq!(
        session.active_pointer(),
        None,
        "a gesture whose claim is gone is a gesture whose finger is gone"
    );
}

/// Tapping is how a touch platform dismisses a selection — and inside a scroll
/// view the text is the only thing a tap ever lands on, so waiting for a press
/// elsewhere would leave the highlight up for good.
#[test]
fn a_finger_press_dismisses_the_selection_it_lands_on() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));
    texts[0].touch_hold.backdate(TOUCH_SELECTION_HOLD);
    let _ = texts[0].on_event(&ElementEvent::PointerUp(touch_at(25.0, 10.0, 0)));
    assert_eq!(session.selected_text(), "first", "the hold selected a word");

    let _ = texts[0].on_event(&ElementEvent::PointerDown(touch_at(25.0, 10.0, 0)));

    assert_eq!(
        session.selected_text(),
        "",
        "the tap that follows a selection drops it"
    );
    assert_eq!(selected(&texts[0]), None);
}

#[test]
fn a_right_click_selects_the_word_under_it_and_offers_the_callout() {
    let (session, texts) = region_texts(&["first second", "third"]);

    let taken = texts[0].on_event(&ElementEvent::PointerDown(PointerInfo::new(
        Vec2d { x: 75.0, y: 10.0 },
        PointerSource::Mouse,
        0,
        PointerButton::Secondary,
    )));

    assert!(taken.is_consumed());
    assert_eq!(session.selected_text(), "second");
    assert!(session.ui.is_menu_open());
    assert!(
        session.handle_circles().is_none(),
        "a right-click earns the callout but not the knobs"
    );
}

#[test]
fn a_right_click_on_an_existing_selection_keeps_it() {
    let (session, texts) = region_texts(&["first", "second"]);
    let _ = texts[0].on_event(&ElementEvent::PointerDown(mouse_at(2.0, 10.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerMove(mouse_at(500.0, 900.0)));
    let _ = texts[0].on_event(&ElementEvent::PointerUp(mouse_at(500.0, 900.0)));

    let _ = texts[1].on_event(&ElementEvent::PointerDown(PointerInfo::new(
        Vec2d { x: 25.0, y: 40.0 },
        PointerSource::Mouse,
        0,
        PointerButton::Secondary,
    )));

    assert_eq!(session.selected_text(), "first\nsecond");
    assert!(session.ui.is_menu_open());
}
