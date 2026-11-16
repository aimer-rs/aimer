//! Widgets embedded in the lines of a field: the box reserved for each one, where
//! its widget is placed, and when it comes and goes.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, Drawable, Element, EventElement, FocusNode, LayoutElement, Rebuildable, VisitorElement, Widget};

use super::test_support::{dummy_build_context, field_config};
use super::{RawTextField, RawTextFieldHost};
use crate::input_field::caret::CaretBlink;
use crate::input_field::embed::{TextEmbed, TextEmbedder};
use crate::input_field::highlight::HighlightContext;
use crate::TextEditingController;

/// An embedded widget that is `height` tall at any width, notes the size it is
/// updated at, and counts the presses it receives.
#[derive(Clone)]
struct Probe {
    height: f32,
    updated_at: Rc<Cell<(f32, f32)>>,
    presses: Rc<Cell<u32>>,
    /// Whether the widget consumes a press, as a button does, or only watches it
    /// and acts on the tap, as a gesture recogniser does.
    consumes: bool,
    /// Whether the widget consumes a release, as a button does when it completes
    /// a tap.
    consumes_release: bool,
}

impl Probe {
    fn new(height: f32, updated_at: Rc<Cell<(f32, f32)>>) -> Self {
        Self { height, updated_at, presses: Rc::default(), consumes: true, consumes_release: false }
    }
}

impl Widget for Probe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        ProbeElement { height: self.height, updated_at: self.updated_at, presses: self.presses, consumes: self.consumes, consumes_release: self.consumes_release }.boxed()
    }
}

struct ProbeElement {
    height: f32,
    updated_at: Rc<Cell<(f32, f32)>>,
    presses: Rc<Cell<u32>>,
    consumes: bool,
    consumes_release: bool,
}

impl Drawable for ProbeElement {
    fn update(&self, ctx: &BuildContext) {
        self.updated_at.set((ctx.parent_size.width, ctx.parent_size.height));
    }
}

impl VisitorElement for ProbeElement {}

impl Element for ProbeElement {
    fn name(&self) -> &'static str {
        "ProbeEmbed"
    }
}

impl EventElement for ProbeElement {
    fn event_tree_role(&self) -> aimer_widget::EventTreeRole {
        aimer_widget::EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &aimer_events::element::ElementEvent) -> aimer_widget::EventResult {
        if matches!(event, aimer_events::element::ElementEvent::PointerDown(_)) {
            self.presses.set(self.presses.get() + 1);
            if self.consumes {
                return aimer_widget::EventResult::consumed();
            }
        }
        if self.consumes_release && matches!(event, aimer_events::element::ElementEvent::PointerUp(_)) {
            return aimer_widget::EventResult::consumed();
        }
        aimer_widget::EventResult::ignored()
    }
}
impl Rebuildable for ProbeElement {}

impl LayoutElement for ProbeElement {
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize { width: ctx.box_constraint.max_width, height: self.height * ctx.scale }
    }
}

const TEXT: &str = "title\n```\ncode\n```\nafter";
const BLOCK: std::ops::Range<usize> = 6..18;

/// Embeds the fenced block while the cursor is not in it.
fn block_embedder(height: f32, updated_at: Rc<Cell<(f32, f32)>>) -> TextEmbedder {
    TextEmbedder::new(move |text, context| {
        assert_eq!(&text[BLOCK], "```\ncode\n```");
        if context.is_active(&BLOCK) {
            return Vec::new();
        }
        let probe = Probe::new(height, Rc::clone(&updated_at));
        vec![TextEmbed::new(BLOCK, 7, move || probe.clone())]
    })
}

fn host(embedder: Option<TextEmbedder>, controller: TextEditingController) -> (RawTextFieldHost, BuildContext<'static>) {
    let mut config = field_config(controller);
    config.embedder = embedder;
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    // The cursor only counts as being somewhere in a field that has focus.
    let _ = field.on_event(&aimer_events::element::ElementEvent::FocusGained);
    let context = dummy_build_context(400.0, 600.0);
    let caret = ProbeElement { height: 1.0, updated_at: Rc::default(), presses: Rc::default(), consumes: true, consumes_release: false }.boxed();
    (RawTextFieldHost::new(field, caret), context)
}

fn unbounded(mut context: BuildContext<'static>) -> BuildContext<'static> {
    context.box_constraint.max_height = 1_000_000.0;
    context
}

#[test]
fn an_embed_reserves_a_full_width_box_as_tall_as_its_widget() {
    let (plain, context) = host(None, TextEditingController::with_text(TEXT));
    let (embedded, _) = host(Some(block_embedder(60.0, Rc::default())), TextEditingController::with_text(TEXT));
    let context = unbounded(context);

    let plain_height = plain.computed_size(&context).height;
    let embedded_height = embedded.computed_size(&context).height;

    // Three source lines become one 60 px line. The box stands on its baseline, so
    // the line also keeps the descent of the text under it, a few pixels of
    // margin below the widget.
    let line = (plain_height - 8.0) / 5.0;
    let expected = 8.0 + 2.0 * line + 60.0;
    assert!(
        embedded_height >= expected - 1.0 && embedded_height <= expected + 6.0,
        "{embedded_height} vs {expected}"
    );
    assert_eq!(embedded.field.controller.text(), TEXT, "the text itself is untouched");
}

#[test]
fn the_widget_is_placed_over_its_box_and_updated_at_its_size() {
    let updated_at = Rc::new(Cell::new((0.0, 0.0)));
    let (host, context) = host(Some(block_embedder(60.0, Rc::clone(&updated_at))), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    assert!(host.sync_paint_state(&context));

    let (offset, size) = host.field.embeds.placement_of(7).expect("the embedded widget exists");

    assert!((size.height - 60.0).abs() < 0.5 && (size.width - 392.0).abs() < 0.5, "{size:?}");
    assert!((offset.x - 4.0).abs() < 0.5, "it starts at the content's left edge: {offset:?}");
    assert!(offset.y > 4.0 + 10.0 && offset.y < 4.0 + 40.0, "below the title line: {offset:?}");
    host.update(&context);
    let seen = updated_at.get();
    assert!((seen.0 - 392.0).abs() < 0.5 && (seen.1 - 60.0).abs() < 0.5, "updated at {seen:?}");
}

#[test]
fn the_embed_is_a_child_of_the_host() {
    let (host, context) = host(Some(block_embedder(60.0, Rc::default())), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    assert!(host.sync_paint_state(&context));

    let mut children = 0;
    host.visit_children(&mut |_| children += 1);
    let mut hit_targets = 0;
    host.hit_test_children(&mut |_| hit_targets += 1);

    assert_eq!(children, 2, "the caret and the embed");
    assert_eq!(hit_targets, 1, "and only the embed is a target");
}

#[test]
fn an_embed_goes_when_the_cursor_enters_its_block_and_returns_when_it_leaves() {
    let (host, context) = host(Some(block_embedder(60.0, Rc::default())), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    assert!(host.sync_paint_state(&context));
    assert_eq!(host.field.embeds.len(), 1);

    let generation = aimer_widget::live_children_generation();
    host.field.cursor.set_offset(10);
    assert!(host.sync_paint_state(&context));
    assert_eq!(host.field.embeds.len(), 0, "the block shows its source under the cursor");
    assert!(aimer_widget::live_children_generation() > generation, "and the tree is told its children changed");
    let source_height = host.field.computed_size(&context).height;

    host.field.cursor.set_offset(0);
    assert!(host.sync_paint_state(&context));
    assert_eq!(host.field.embeds.len(), 1, "it comes back with the cursor elsewhere");
    assert!(host.field.computed_size(&context).height < source_height + 30.0);
}

#[test]
fn a_field_without_focus_shows_every_embed() {
    let (host, context) = host(Some(block_embedder(60.0, Rc::default())), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    host.field.cursor.set_offset(10);
    host.field.set_focused(false);

    assert!(host.sync_paint_state(&context));

    assert_eq!(host.field.embeds.len(), 1);
}

#[test]
fn a_fixed_height_needs_no_measuring() {
    let embedder = TextEmbedder::new(|_, context| {
        if context.is_active(&BLOCK) {
            return Vec::new();
        }
        vec![TextEmbed::new(BLOCK, 7, || Probe::new(999.0, Rc::default())).fixed_height(30.0)]
    });
    let (host, context) = host(Some(embedder), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    assert!(host.sync_paint_state(&context));

    let (_, size) = host.field.embeds.placement_of(7).unwrap();

    assert!((size.height - 30.0).abs() < 0.5, "the widget says 999 but the embed says 30: {size:?}");
}

#[test]
fn the_measured_height_follows_the_width_of_the_field() {
    let narrow_probe = Rc::new(Cell::new((0.0, 0.0)));
    let (narrow, context) = host(Some(block_embedder(60.0, Rc::clone(&narrow_probe))), TextEditingController::with_text(TEXT));
    let mut narrow_context = unbounded(context);
    narrow_context.box_constraint.max_width = 200.0;
    narrow_context.parent_size.width = 200.0;

    assert!(narrow.sync_paint_state(&narrow_context));
    let (_, size) = narrow.field.embeds.placement_of(7).unwrap();

    assert!((size.width - 192.0).abs() < 0.5, "the embed takes the width the field has: {size:?}");
}

#[test]
fn a_highlighted_range_under_an_embed_is_dropped_and_the_embed_wins() {
    use aimer_text::{InlineBox, SpanStyle, StyledRange};

    use super::{EmbedBox, with_embed_boxes};

    let ranges: Rc<[StyledRange]> = Rc::from(vec![
        StyledRange::new(0..3, SpanStyle::new()),
        StyledRange::new(5..9, SpanStyle::new()),
        StyledRange::new(12..14, SpanStyle::new()),
    ]);
    let boxes = [EmbedBox { key: 1, range: 4..10, width: 100.0, height: 40.0 }];

    let merged = with_embed_boxes(Some(ranges), &boxes, 1.0).unwrap();

    let spans: Vec<_> = merged.iter().map(|styled| (styled.range.clone(), styled.style.inline_box.is_some())).collect();
    assert_eq!(spans, [(0..3, false), (12..14, false), (4..10, true)]);
    let inline_box = merged.last().unwrap().style.inline_box.unwrap();
    assert_eq!(inline_box, InlineBox::new(99.99, 40.0));
}

#[test]
fn embeds_that_cut_a_character_or_overlap_are_left_out() {
    let text = "é\nline two\nthree";
    let embedder = TextEmbedder::new(|_, _| {
        vec![
            TextEmbed::new(1..2, 1, || Probe::new(10.0, Rc::default())),
            TextEmbed::new(3..11, 2, || Probe::new(10.0, Rc::default())),
            TextEmbed::new(5..13, 3, || Probe::new(10.0, Rc::default())),
            TextEmbed::new(8..99, 4, || Probe::new(10.0, Rc::default())),
        ]
    });
    let (host, context) = host(Some(embedder), TextEditingController::with_text(text));
    let context = unbounded(context);

    assert!(host.sync_paint_state(&context));

    assert!(host.field.embeds.placement_of(2).is_some(), "the valid one is kept");
    for key in [1, 3, 4] {
        assert!(host.field.embeds.placement_of(key).is_none(), "embed {key} is invalid");
    }
    let _ = (Vec2d::default(), HighlightContext::default());
}

#[test]
fn a_press_on_an_embedded_widget_reaches_it_and_not_the_text() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let presses = Rc::new(Cell::new(0));
    let probe = Probe { presses: Rc::clone(&presses), ..Probe::new(60.0, Rc::default()) };
    let embedder = TextEmbedder::new(move |_, context| {
        if context.is_active(&BLOCK) {
            return Vec::new();
        }
        let probe = probe.clone();
        vec![TextEmbed::new(BLOCK, 7, move || probe.clone())]
    });
    let (host, context) = host(Some(embedder), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    let embeds = Rc::clone(&host.field.embeds);
    let field_cursor = host.field.cursor.offset();
    let root = host.boxed();
    root.sync_paint_state(&context);
    root.update(&context);
    let (offset, size) = embeds.placement_of(7).expect("the embed is placed");

    let inside = Vec2d { x: offset.x + size.width / 2.0, y: offset.y + size.height / 2.0 };
    let result = aimer_widget::dispatch_event(
        root.as_ref(),
        inside,
        &ElementEvent::PointerDown(PointerInfo::mouse(inside, PointerButton::Primary)),
    );

    assert_eq!(presses.get(), 1, "the press reached the embedded widget (placed at {offset:?} {size:?})");
    assert!(result.is_consumed());
    let _ = field_cursor;
}

/// A copy button acts on the tap and only watches the press. The press itself must
/// not fall through to the text under the widget: it would start a selection and
/// put the cursor in the block, which swaps the widget for its source before the
/// tap completes. What a release does is covered by the click tests below.
#[test]
fn a_press_the_embedded_widget_does_not_consume_still_does_not_reach_the_text() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let presses = Rc::new(Cell::new(0));
    let probe = Probe { presses: Rc::clone(&presses), consumes: false, ..Probe::new(60.0, Rc::default()) };
    let embedder = TextEmbedder::new(move |_, context| {
        if context.is_active(&BLOCK) {
            return Vec::new();
        }
        let probe = probe.clone();
        vec![TextEmbed::new(BLOCK, 7, move || probe.clone())]
    });
    let (host, context) = host(Some(embedder), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    let embeds = Rc::clone(&host.field.embeds);
    let controller = host.field.controller.clone();
    let root = host.boxed();
    root.sync_paint_state(&context);
    root.update(&context);
    let (offset, size) = embeds.placement_of(7).expect("the embed is placed");
    let inside = Vec2d { x: offset.x + size.width / 2.0, y: offset.y + size.height / 2.0 };
    let (_, before_end) = controller.selection_graphemes();

    let result = aimer_widget::dispatch_event(
        root.as_ref(),
        inside,
        &ElementEvent::PointerDown(PointerInfo::mouse(inside, PointerButton::Primary)),
    );
    root.sync_paint_state(&context);

    assert!(result.is_consumed(), "the host claims it, so nothing above the text sees it");
    assert_eq!(presses.get(), 1, "the widget still got to see the press");
    assert_eq!(controller.selection_graphemes().1, before_end, "the cursor did not move");
    assert_eq!(embeds.len(), 1, "so the widget is still there for the tap");
}

/// Presses and releases at `y` over the embedded widget placed at key 7, as a
/// click does, and returns the cursor offset afterwards.
fn click_embed(host: RawTextFieldHost, context: &BuildContext<'static>, y_fraction: f32, release_x_shift: f32) -> (usize, usize) {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let embeds = Rc::clone(&host.field.embeds);
    let controller = host.field.controller.clone();
    let root = host.boxed();
    root.sync_paint_state(context);
    root.update(context);
    let (offset, size) = embeds.placement_of(7).expect("the embed is placed");
    let down = Vec2d { x: offset.x + size.width / 2.0, y: offset.y + size.height * y_fraction };
    let up = Vec2d { x: down.x + release_x_shift, y: down.y };

    let _ = aimer_widget::dispatch_event(root.as_ref(), down, &ElementEvent::PointerDown(PointerInfo::mouse(down, PointerButton::Primary)));
    let _ = aimer_widget::dispatch_event(root.as_ref(), up, &ElementEvent::PointerUp(PointerInfo::mouse(up, PointerButton::Primary)));
    root.sync_paint_state(context);
    (controller.selection_graphemes().1, embeds.len())
}

fn clickable_embedder(consumes_release: bool) -> TextEmbedder {
    let probe = Probe { consumes: false, consumes_release, ..Probe::new(60.0, Rc::default()) };
    TextEmbedder::new(move |_, context| {
        if context.is_active(&BLOCK) {
            return Vec::new();
        }
        let probe = probe.clone();
        vec![TextEmbed::new(BLOCK, 7, move || probe.clone())]
    })
}

/// The widget has nothing to do with a click on its plain area, so the click is
/// the user asking to edit what it shows: the cursor goes to the source line that
/// matches where they clicked, and the widget gives way to the source.
#[test]
fn a_click_the_embedded_widget_does_not_handle_enters_its_source_at_the_line_clicked() {
    let block_lines = [6, 10, 15];
    for (fraction, expected) in [(0.1, block_lines[0]), (0.5, block_lines[1]), (0.9, block_lines[2])] {
        let (host, context) = host(Some(clickable_embedder(false)), TextEditingController::with_text(TEXT));
        let context = unbounded(context);

        let (cursor, embeds) = click_embed(host, &context, fraction, 0.0);

        assert_eq!(cursor, expected, "a click {fraction} of the way down");
        assert_eq!(embeds, 0, "the source is shown under the cursor");
    }
}

#[test]
fn a_tap_the_embedded_widget_handles_does_not_enter_its_source() {
    let (host, context) = host(Some(clickable_embedder(true)), TextEditingController::with_text(TEXT));
    let context = unbounded(context);

    let (cursor, embeds) = click_embed(host, &context, 0.5, 0.0);

    assert_eq!(cursor, TEXT.chars().count(), "the cursor stayed where it was");
    assert_eq!(embeds, 1, "and the widget is still there");
}

#[test]
fn a_press_that_is_released_elsewhere_is_not_a_click() {
    let (host, context) = host(Some(clickable_embedder(false)), TextEditingController::with_text(TEXT));
    let context = unbounded(context);

    let (cursor, embeds) = click_embed(host, &context, 0.5, 5000.0);

    assert_eq!(cursor, TEXT.chars().count());
    assert_eq!(embeds, 1);
}

/// A press that landed on the text and is let go over an embedded widget belongs
/// to the text: the field must see the release, or it keeps believing the button is
/// down and extends a selection to wherever the pointer goes next.
#[test]
fn a_release_over_an_embed_ends_a_drag_that_began_on_the_text() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_widget::CaptureRequest;

    let (host, context) = host(Some(clickable_embedder(false)), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    let embeds = Rc::clone(&host.field.embeds);
    let root = host.boxed();
    root.sync_paint_state(&context);
    root.update(&context);
    let (offset, size) = embeds.placement_of(7).expect("the embed is placed");
    let on_text = Vec2d { x: 4.0, y: 4.0 };
    let over_embed = Vec2d { x: offset.x + size.width / 2.0, y: offset.y + size.height / 2.0 };

    let down = aimer_widget::dispatch_event(root.as_ref(), on_text, &ElementEvent::PointerDown(PointerInfo::mouse(on_text, PointerButton::Primary)));
    let CaptureRequest::Capture(pointer) = down.capture_request() else {
        panic!("the press on the text starts a drag: {:?}", down.capture_request());
    };
    let up = aimer_widget::dispatch_event(root.as_ref(), over_embed, &ElementEvent::PointerUp(PointerInfo::mouse(over_embed, PointerButton::Primary)));

    assert_eq!(up.capture_request(), CaptureRequest::Release(pointer), "the release ended the drag");
}

/// Dragging a selection across an embedded widget points the head at the widget's
/// box, which must resolve to an edge of the source it stands for, like any other
/// character does. A box that resolves to nothing sends the head to the end of the
/// text, so the selection runs from where it began to the end of the note.
#[test]
fn a_point_inside_an_embed_box_resolves_to_the_edge_of_its_source() {
    let (host, context) = host(Some(clickable_embedder(false)), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    host.sync_paint_state(&context);
    host.update(&context);
    let geometry = host.field.geometry_cache.latest().expect("the field laid its text out");
    let rect = geometry.embed_rects().into_iter().find(|rect| rect.key == 7).expect("the embed has a box");
    let (first, last) = (BLOCK.start, BLOCK.end);

    for (fx, fy) in [(0.5, 0.1), (0.5, 0.5), (0.5, 0.9), (0.95, 0.5), (0.05, 0.5)] {
        let (x, y) = (rect.x + rect.width * fx, rect.top + rect.height * fy);
        let offset = geometry.hit_test(x, y);
        assert!(
            offset.is_some_and(|offset| (first..=last).contains(&offset)),
            "({fx}, {fy}) of the box resolved to {offset:?}, not an edge of {first}..{last}"
        );
    }
}

/// While the button is held the head of a drag sweeps over whatever is under the
/// pointer. If an embed gave way to its source the moment the head entered it, the
/// layout would change under the pointer, the head would resolve somewhere else, the
/// embed would come back, and the selection would run back and forth for as long as
/// the button stays down. So a drag decides what shows by where it began, and the
/// embeds it passes over change when it is let go.
#[test]
fn a_drag_over_an_embed_does_not_swap_it_for_its_source_until_it_is_let_go() {
    use aimer_widget::PointerKey;

    let (host, context) = host(Some(block_embedder(60.0, Rc::default())), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    assert!(host.sync_paint_state(&context));
    assert_eq!(host.field.embeds.len(), 1);
    let end = TEXT.chars().count();

    // The press began on the last line and the head is now inside the block.
    host.field.mouse_held.set(Some(PointerKey::new(aimer_events::pointer::PointerSource::Mouse, 0)));
    host.field.cursor.set_selection_anchor(Some(end));
    host.field.cursor.set_offset(10);
    host.sync_paint_state(&context);
    assert_eq!(host.field.embeds.len(), 1, "the widget stays while the button is held");

    host.field.mouse_held.set(None);
    host.sync_paint_state(&context);
    assert_eq!(host.field.embeds.len(), 0, "and gives way once the selection is made");
}

/// The text-selection area reports a drag as an ordered range, so for a drag that
/// goes up from the last line the cursor's anchor ends up at the head, which is
/// inside the embed, and its offset at the press. What shows must still be decided
/// by where the press was, or the embed gives way to its source mid-drag, the
/// layout moves under the pointer, and the field flips between the two states.
#[test]
fn a_drag_decides_what_shows_by_the_press_even_when_the_area_swaps_anchor_and_head() {
    use aimer_widget::PointerKey;

    let (host, context) = host(Some(block_embedder(60.0, Rc::default())), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    assert!(host.sync_paint_state(&context));
    assert_eq!(host.field.embeds.len(), 1);
    let end = TEXT.chars().count();

    host.field.mouse_held.set(Some(PointerKey::new(aimer_events::pointer::PointerSource::Mouse, 0)));
    host.field.drag_origin.set(Some(end));
    host.field.cursor.set_selection_anchor(Some(10));
    host.field.cursor.set_offset(end);
    host.sync_paint_state(&context);

    assert_eq!(host.field.embeds.len(), 1, "the press was on the last line, so the widget stays");
}

/// Positions are resolved to offsets when a frame draws, so a press and the moves
/// that follow it can arrive before any frame has. The press is where the drag
/// began and must stay the anchor, whatever the pointer has done since.
#[test]
fn moves_that_arrive_before_the_press_is_drawn_do_not_replace_it() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let (host, context) = host(Some(clickable_embedder(false)), TextEditingController::with_text(TEXT));
    let context = unbounded(context);
    let embeds = Rc::clone(&host.field.embeds);
    let controller = host.field.controller.clone();
    let root = host.boxed();
    root.sync_paint_state(&context);
    root.update(&context);
    let (offset, size) = embeds.placement_of(7).expect("the embed is placed");
    // On the last line ("after", graphemes 19..24), then up in the title line.
    let press = Vec2d { x: 4.0, y: offset.y + size.height + 8.0 };
    let title = Vec2d { x: 4.0, y: 4.0 };
    let past = Vec2d { x: 30.0, y: 2.0 };

    let _ = aimer_widget::dispatch_event(root.as_ref(), press, &ElementEvent::PointerDown(PointerInfo::mouse(press, PointerButton::Primary)));
    let _ = aimer_widget::dispatch_event(root.as_ref(), title, &ElementEvent::PointerMove(PointerInfo::mouse(title, PointerButton::Primary)));
    let _ = aimer_widget::dispatch_event(root.as_ref(), past, &ElementEvent::PointerMove(PointerInfo::mouse(past, PointerButton::Primary)));
    root.sync_paint_state(&context);
    root.update(&context);

    let (anchor, focus) = controller.selection_graphemes();
    assert!(anchor >= 19, "the drag began on the last line, but is anchored at {anchor} (head {focus})");
    assert!(focus < 6, "and its head followed the pointer up to the title: {focus}");
}
