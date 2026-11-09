use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_cupid::draw_cmd::{DrawCommand, RenderNodeId, RenderTree};
use aimer_platform::attachment::{MarkerPhase, SyntheticPointer};
use aimer_utils::AnimInstant;
use aimer_widget::SafeAreaInsets;

use super::*;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn pointer(touch: bool, id: u64, phase: MarkerPhase, x: f32, y: f32) -> SyntheticPointer {
    SyntheticPointer { touch, id, phase, pos: Vec2d { x, y } }
}

fn commands(tree: &RenderTree, node: RenderNodeId) -> Vec<DrawCommand> {
    tree.draw_list_snapshot(node).expect("a recorded node").commands.to_vec()
}

fn prepare(overlay: &mut AttachOverlay, tree: &RenderTree, now: AnimInstant) {
    overlay.prepare(tree, 1000, 600, 1.0, SafeAreaInsets::ZERO, now).expect("prepared");
}

/// Where a node's own origin is in the window: its commands are local to it.
fn origin(tree: &RenderTree, node: RenderNodeId) -> (f32, f32) {
    let bounds = tree.element_bounds(node).expect("bounds");
    (bounds.x, bounds.y)
}

fn svg_in(commands: &[DrawCommand]) -> Option<(aimer_cupid::draw_cmd::Rect, Arc<[aimer_cupid::svg::SvgNodeStyleOverride]>)> {
    commands.iter().find_map(|c| match c {
        DrawCommand::Svg { destination, overrides, .. } => Some((*destination, overrides.clone())),
        _ => None,
    })
}

/// The angle, in radians, the recorded pointer is rotated by: read back from the
/// transform override on its path, whose rotation part is `[cos -sin; sin cos]`.
fn tilt_of(commands: &[DrawCommand]) -> f32 {
    let (_, overrides) = svg_in(commands).expect("the pointer icon");
    match overrides.iter().find_map(|o| o.transform) {
        Some(t) => t.ky.atan2(t.sx),
        None => 0.0,
    }
}

fn rings_in(commands: &[DrawCommand]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, DrawCommand::FillRect { border_width, .. } if border_width[0] > 0.0))
        .count()
}

fn texts(commands: &[DrawCommand]) -> Vec<String> {
    commands
        .iter()
        .filter_map(|c| if let DrawCommand::DrawText { text, .. } = c { Some(text.to_string()) } else { None })
        .collect()
}

// ---- the badge's shape and place

#[test]
fn the_badge_is_a_wide_pill_in_the_top_right_corner_inside_the_window_and_its_safe_area() {
    let rect = badge_rect(1000.0, SafeAreaInsets::ZERO);
    assert!(rect.width > rect.height, "a pill is wider than it is tall: {rect:?}");
    assert!((rect.x + rect.width - (1000.0 - BADGE_MARGIN)).abs() < 1e-3, "{rect:?}");
    assert!((rect.y - BADGE_MARGIN).abs() < 1e-3, "top, not the middle: {rect:?}");

    // Clear of a notch or status bar on top and of a cutout on the right.
    let inset = badge_rect(1000.0, SafeAreaInsets::new(0.0, 47.0, 20.0, 34.0));
    assert!((inset.y - (rect.y + 47.0)).abs() < 1e-3, "clear of the top inset");
    assert!((inset.x + inset.width - (rect.x + rect.width - 20.0)).abs() < 1e-3, "clear of the right inset");
}

#[test]
fn a_window_too_narrow_for_the_badge_never_pushes_it_off_the_left() {
    let rect = badge_rect(40.0, SafeAreaInsets::ZERO);
    assert!(rect.x >= 0.0 && rect.y >= 0.0, "{rect:?}");
}

#[test]
fn the_badge_icon_is_white_not_the_theme_colour_lucide_ships_it_in() {
    assert!(BADGE_SVG.contains("currentColor"), "premise: the file is in the theme colour");
    let markup = badge_markup();
    assert!(!markup.contains("currentColor"), "a retained scene has no text colour to inherit");
    assert!(markup.contains("#ffffff") && markup.contains("<path"));
}

// ---- markers

#[test]
fn a_mouse_marker_follows_the_injected_mouse_until_it_leaves() {
    let t = AnimInstant::now();
    let mut overlay = AttachOverlay::default();
    assert!(overlay.markers().is_empty());
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 10.0, 20.0), t);
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 30.0, 40.0), t);
    let [marker] = overlay.markers() else { panic!("one mouse: {:?}", overlay.markers()) };
    assert_eq!((marker.pos.x, marker.pos.y), (30.0, 40.0));
    assert!(!marker.touch);
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Left, 0.0, 0.0), t);
    assert!(overlay.markers().is_empty());
}

#[test]
fn a_press_marks_the_marker_down_and_a_release_lifts_it() {
    let t = AnimInstant::now();
    let mut overlay = AttachOverlay::default();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Pressed, 5.0, 5.0), t);
    assert!(overlay.markers()[0].down);
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Released, 5.0, 5.0), t);
    assert!(!overlay.markers()[0].down);
}

#[test]
fn each_finger_has_its_own_marker_and_a_cancel_clears_every_finger_but_not_the_mouse() {
    let t = AnimInstant::now();
    let mut overlay = AttachOverlay::default();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 1.0, 1.0), t);
    overlay.note_pointer(pointer(true, 1, MarkerPhase::Pressed, 10.0, 10.0), t);
    overlay.note_pointer(pointer(true, 2, MarkerPhase::Pressed, 20.0, 20.0), t);
    assert_eq!(overlay.markers().len(), 3);
    overlay.note_pointer(pointer(true, 0, MarkerPhase::Left, 0.0, 0.0), t);
    let [mouse] = overlay.markers() else { panic!("{:?}", overlay.markers()) };
    assert!(!mouse.touch, "the cancel was for fingers");
}

#[test]
fn a_finger_that_has_lifted_fades_and_is_then_forgotten() {
    let t = AnimInstant::now();
    let mut overlay = AttachOverlay::default();
    let tree = RenderTree::new();
    overlay.note_pointer(pointer(true, 1, MarkerPhase::Pressed, 10.0, 10.0), t);
    overlay.note_pointer(pointer(true, 1, MarkerPhase::Released, 10.0, 10.0), t);
    prepare(&mut overlay, &tree, t + FADE / 2);
    assert_eq!(overlay.markers().len(), 1, "still fading");
    prepare(&mut overlay, &tree, t + FADE + Duration::from_millis(1));
    assert!(overlay.markers().is_empty(), "faded out");
}

#[test]
fn nobody_attached_means_no_markers_even_if_a_late_event_arrives() {
    let t = AnimInstant::now();
    let mut overlay = AttachOverlay::default();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 1.0, 1.0), t);
    overlay.set_names(Vec::new());
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 2.0, 2.0), t);
    assert_eq!(overlay.markers().len(), 1, "a late event after detaching is ignored");
    let tree = RenderTree::new();
    prepare(&mut overlay, &tree, t);
    prepare(&mut overlay, &tree, t + PRESENCE_FADE + Duration::from_millis(1));
    assert!(overlay.markers().is_empty(), "and what the driver left behind is forgotten once it has faded");
}

// ---- when the overlay needs frames

#[test]
fn it_asks_for_frames_only_while_something_animates_and_a_resting_pointer_starts_to_idle() {
    let t = AnimInstant::now();
    let mut overlay = AttachOverlay::default();
    assert_eq!(overlay.wake_in(t), None, "an overlay with no pointer wakes nothing");
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 1.0, 1.0), t);
    assert_eq!(overlay.wake_in(t), Some(FRAME), "a pointer that has just appeared is fading in");
    // Past the fade, what is left is waiting for the idle animation to begin.
    assert_eq!(overlay.wake_in(t + PRESENCE_FADE + Duration::from_millis(1)), Some(IDLE_AFTER - PRESENCE_FADE - Duration::from_millis(1)), "once, for when the idle animation is due to begin");
    assert_eq!(overlay.wake_in(t + IDLE_AFTER), Some(IDLE_FRAME), "then at the idle animation's own pace");
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Pressed, 1.0, 1.0), t + Duration::from_secs(5));
    let pressed = t + Duration::from_secs(5);
    assert_eq!(overlay.wake_in(pressed + Duration::from_millis(10)), Some(FRAME), "the ripple is playing");
    assert_eq!(overlay.wake_in(pressed + RIPPLE + Duration::from_millis(1)), None, "a held button rests without idling");
}

// ---- what is recorded

#[test]
fn with_nobody_attached_and_no_markers_nothing_is_added_to_the_tree() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    prepare(&mut overlay, &tree, AnimInstant::now());
    assert!(overlay.badge_node().is_none() && overlay.pointer_node().is_none());
}

#[test]
fn the_badge_is_an_indigo_pill_holding_a_white_square_mouse_pointer_icon() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    prepare(&mut overlay, &tree, t);
    // Once it has faded in.
    prepare(&mut overlay, &tree, t + PRESENCE_FADE + Duration::from_millis(1));
    let node = overlay.badge_node().expect("a badge node");
    let recorded = commands(&tree, node);
    assert!(texts(&recorded).is_empty(), "an icon, not words");

    let (pill, radius, fill) = recorded
        .iter()
        .find_map(|c| if let DrawCommand::FillRect { rect, border_radius, color, .. } = c { Some((*rect, border_radius[0], *color)) } else { None })
        .expect("the pill");
    assert_eq!(radius, pill.height / 2.0, "fully rounded ends");
    assert_eq!(fill, Color::rgba8(BADGE_COLOR.0, BADGE_COLOR.1, BADGE_COLOR.2, 255), "solid, not see-through");
    assert!(pill.x > 500.0 && pill.y < 100.0, "the pill is in the top-right in window coordinates: {pill:?}");

    let (icon, _) = svg_in(&recorded).expect("the icon");
    let scene = recorded.iter().find_map(|c| if let DrawCommand::Svg { scene, .. } = c { Some(scene) } else { None }).unwrap();
    assert!(Arc::ptr_eq(scene, badge_scene()));
    assert_eq!((icon.width, icon.height), (BADGE_ICON, BADGE_ICON));
    // Centred in the pill.
    assert!(((icon.x + icon.width / 2.0) - (pill.x + pill.width / 2.0)).abs() < 0.01);
    assert!(((icon.y + icon.height / 2.0) - (pill.y + pill.height / 2.0)).abs() < 0.01);

    let bounds = tree.element_bounds(node).expect("bounds");
    assert!(bounds.x + bounds.width <= 1000.0 && bounds.y + bounds.height <= 600.0);
    // The commands are in window coordinates, which only draw in the right place
    // if the node's own origin is the window's: a node hugging the badge would
    // shift them by its position, off the window.
    assert_eq!((bounds.x, bounds.y), (0.0, 0.0));
}

#[test]
fn the_mouse_marker_is_the_lucide_pointer_with_its_tip_on_the_injected_position() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 300.0, 200.0), t);
    prepare(&mut overlay, &tree, t);
    let node = overlay.pointer_node().expect("a pointer node");
    let recorded = commands(&tree, node);
    let (destination, overrides) = svg_in(&recorded).expect("the pointer icon is drawn");
    let (ox, oy) = origin(&tree, node);
    // Commands are local to the node, so the icon's world position adds its origin.
    assert!((ox + destination.x - (300.0 - ICON_TIP)).abs() < 0.01 && (oy + destination.y - (200.0 - ICON_TIP)).abs() < 0.01);
    assert_eq!((destination.width, destination.height), (ICON_SIZE, ICON_SIZE));
    let scene = recorded.iter().find_map(|c| if let DrawCommand::Svg { scene, .. } = c { Some(scene) } else { None }).unwrap();
    assert!(Arc::ptr_eq(scene, pointer_scene()));
    assert!(!overrides.is_empty());
}

#[test]
fn the_pointer_is_drawn_at_its_configured_size_and_opacity_with_its_tip_scaled_to_match() {
    // Numbers that are tuned live in the constants, and the test follows them: what
    // is pinned is how they relate, so retuning the cursor does not break it.
    assert!((ICON_SIZE - ICON_VIEWBOX * ICON_SCALE).abs() < 1e-4, "the icon's box scaled");
    assert!((ICON_TIP - ICON_TIP_UNITS * ICON_SCALE).abs() < 1e-4, "and the tip moves with it");
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 100.0, 100.0), t);
    // Once it has faded in.
    prepare(&mut overlay, &tree, t + PRESENCE_FADE + Duration::from_millis(1));
    let (icon, overrides) = svg_in(&commands(&tree, overlay.pointer_node().unwrap())).expect("the icon");
    assert_eq!((icon.width, icon.height), (ICON_SIZE, ICON_SIZE));
    assert!(!overrides.is_empty());
    for node in overrides.iter() {
        assert_eq!(node.opacity, Some(POINTER_OPACITY), "every drawn part of the pointer has the configured opacity");
    }
}

#[test]
fn the_pointer_is_the_cursor_icon_from_the_assets_folder_with_its_own_colours() {
    // The file opens with a DOCTYPE, which the SVG parser does not accept; the
    // overlay parses from the `<svg` element on.
    assert!(POINTER_SVG.starts_with("<svg"), "{:?}", &POINTER_SVG[..30.min(POINTER_SVG.len())]);
    let scene = pointer_scene();
    assert_eq!(scene.geometries.len(), 1, "one arrow");
    let drawn: Vec<_> = scene.nodes.iter().filter(|n| n.geometry.is_some()).collect();
    let [arrow] = drawn[..] else { panic!("one drawn node") };
    let fill = arrow.fill.as_ref().expect("the icon's own fill");
    assert!(fill.color.r < 0.25 && fill.color.g < 0.25 && fill.color.b < 0.25, "a dark body: {:?}", fill.color);
    let stroke = arrow.stroke.as_ref().expect("the icon's own outline");
    assert!(stroke.color.r > 0.9 && stroke.color.g > 0.9 && stroke.color.b > 0.9, "a white outline: {:?}", stroke.color);
    // Recolouring would override the colours; only the opacity and the sway do.
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 100.0, 100.0, t);
    prepare(&mut overlay, &tree, t);
    let (_, overrides) = svg_in(&pointer_commands(&overlay, &tree)).unwrap();
    assert!(overrides.iter().all(|o| o.fill.is_none() && o.stroke.is_none()), "the icon keeps its colours");
}

#[test]
fn a_touch_marker_is_a_circle_centered_on_the_finger_and_a_press_adds_a_ripple() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    overlay.note_pointer(pointer(true, 1, MarkerPhase::Pressed, 100.0, 120.0), t);
    prepare(&mut overlay, &tree, t + Duration::from_millis(50));
    let node = overlay.pointer_node().expect("a node");
    let recorded = commands(&tree, node);
    assert!(!recorded.iter().any(|c| matches!(c, DrawCommand::Svg { .. })), "a finger is not a mouse pointer");
    let (ox, oy) = origin(&tree, node);
    let circles: Vec<_> = recorded
        .iter()
        .filter_map(|c| if let DrawCommand::FillRect { rect, .. } = c { Some(*rect) } else { None })
        .collect();
    assert!(circles.len() >= 2, "the finger and the ripple: {circles:?}");
    for rect in circles {
        let (cx, cy) = (ox + rect.x + rect.width / 2.0, oy + rect.y + rect.height / 2.0);
        assert!((cx - 100.0).abs() < 0.01 && (cy - 120.0).abs() < 0.01, "{rect:?} around ({cx}, {cy})");
    }
}

#[test]
fn an_unchanged_overlay_does_not_damage_the_window_again() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 10.0, 10.0), t);
    prepare(&mut overlay, &tree, t);
    // Once it has faded in, which repaints it as it goes.
    let shown = t + PRESENCE_FADE + Duration::from_millis(1);
    prepare(&mut overlay, &tree, shown);
    tree.take_damage();
    prepare(&mut overlay, &tree, shown + Duration::from_millis(5));
    assert!(tree.take_damage().is_empty(), "nothing changed, nothing repainted");
}

#[test]
fn detaching_empties_both_nodes_and_damages_what_they_covered() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, 10.0, 10.0), t);
    let shown = t + PRESENCE_FADE * 2;
    prepare(&mut overlay, &tree, t);
    prepare(&mut overlay, &tree, shown);
    let (badge, pointers) = (overlay.badge_node().unwrap(), overlay.pointer_node().unwrap());
    tree.take_damage();
    overlay.set_names(Vec::new());
    prepare(&mut overlay, &tree, shown);
    prepare(&mut overlay, &tree, shown + PRESENCE_FADE + Duration::from_millis(1));
    assert!(commands(&tree, badge).is_empty() && commands(&tree, pointers).is_empty());
    assert!(!tree.take_damage().is_empty(), "the pixels under them are restored");
}

#[test]
fn unusable_window_metrics_create_nothing() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    for (w, h, scale) in [(0, 600, 1.0), (1000, 0, 1.0), (1000, 600, 0.0), (1000, 600, f32::NAN), (1000, 600, -1.0)] {
        overlay.prepare(&tree, w, h, scale, SafeAreaInsets::ZERO, AnimInstant::now()).expect("skipped");
        assert!(overlay.badge_node().is_none(), "{w}x{h}@{scale}");
    }
}

// ---- idle

fn mouse_at(overlay: &mut AttachOverlay, x: f32, y: f32, at: AnimInstant) {
    overlay.note_pointer(pointer(false, 0, MarkerPhase::Moved, x, y), at);
}

fn pointer_commands(overlay: &AttachOverlay, tree: &RenderTree) -> Vec<DrawCommand> {
    commands(tree, overlay.pointer_node().expect("a pointer node"))
}

#[test]
fn a_pointer_that_has_just_moved_is_drawn_upright() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 300.0, 200.0, t);
    prepare(&mut overlay, &tree, t + IDLE_AFTER / 2);
    let recorded = pointer_commands(&overlay, &tree);
    assert_eq!(tilt_of(&recorded), 0.0, "not swaying yet");
    assert_eq!(svg_in(&recorded).unwrap().0.width, ICON_SIZE);
}

#[test]
fn an_idle_pointer_sways_a_few_degrees_either_side_about_its_tip() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 300.0, 200.0, t);
    let tilt_at = |overlay: &mut AttachOverlay, after: Duration| {
        prepare(overlay, &tree, t + IDLE_AFTER + after);
        tilt_of(&pointer_commands(overlay, &tree))
    };
    let limit = SWAY_DEGREES.to_radians();
    assert!((tilt_at(&mut overlay, IDLE_PERIOD / 4) - limit).abs() < 0.005, "a quarter in: leaning one way, as far as it goes");
    assert!((tilt_at(&mut overlay, IDLE_PERIOD * 3 / 4) + limit).abs() < 0.005, "three quarters in: leaning the other way");
    assert!(limit < 0.12, "slight: {SWAY_DEGREES} degrees");
}

#[test]
fn the_sway_pivots_on_the_tip_so_the_tip_never_moves() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 300.0, 200.0, t);
    prepare(&mut overlay, &tree, t + IDLE_AFTER + IDLE_PERIOD / 4);
    let (_, overrides) = svg_in(&pointer_commands(&overlay, &tree)).unwrap();
    let swayed = overrides.iter().find_map(|o| o.transform).expect("swaying");
    let node = pointer_scene().nodes.iter().find(|n| n.geometry.is_some()).unwrap();
    // The tip in the icon's own 24-unit space, and where the node's transform puts it.
    let apply = |t: aimer_cupid::svg::SvgTransform, (x, y): (f32, f32)| (t.sx * x + t.kx * y + t.tx, t.ky * x + t.sy * y + t.ty);
    let tip = (ICON_TIP / ICON_SCALE, ICON_TIP / ICON_SCALE);
    let (before, after) = (apply(node.transform, tip), apply(swayed, tip));
    assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3, "{before:?} vs {after:?}");
    // And a point away from the tip does move.
    let far = (20.0, 20.0);
    let (a, b) = (apply(node.transform, far), apply(swayed, far));
    assert!((a.0 - b.0).abs() + (a.1 - b.1).abs() > 1.0, "the rest of the arrow swings");
}

#[test]
fn an_idle_pointer_draws_nothing_but_the_arrow_no_bubble_no_ring() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 300.0, 200.0, t);
    for after in [IDLE_AFTER, IDLE_AFTER + IDLE_PERIOD / 4, IDLE_AFTER + IDLE_PERIOD / 2, IDLE_AFTER + IDLE_PERIOD * 3 / 4] {
        prepare(&mut overlay, &tree, t + after);
        let recorded = pointer_commands(&overlay, &tree);
        assert_eq!(recorded.len(), 1, "only the arrow at {after:?}: {recorded:?}");
        assert!(matches!(recorded[0], DrawCommand::Svg { .. }));
        assert_eq!(rings_in(&recorded), 0);
    }
}

#[test]
fn the_sway_is_a_loop_that_starts_and_ends_upright() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 50.0, 50.0, t);
    let tilt_at = |overlay: &mut AttachOverlay, after: Duration| {
        prepare(overlay, &tree, t + after);
        tilt_of(&pointer_commands(overlay, &tree))
    };
    assert!(tilt_at(&mut overlay, IDLE_AFTER).abs() < 1e-4, "begins upright");
    assert!(tilt_at(&mut overlay, IDLE_AFTER + IDLE_PERIOD / 2).abs() < 1e-3, "passes through upright half way");
    assert!(tilt_at(&mut overlay, IDLE_AFTER + IDLE_PERIOD).abs() < 1e-3, "and ends upright, so the loop has no seam");
}

#[test]
fn moving_the_pointer_ends_the_sway_and_a_held_button_or_a_finger_never_sways() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 50.0, 50.0, t);
    let later = t + IDLE_AFTER + IDLE_PERIOD / 4;
    prepare(&mut overlay, &tree, later);
    assert!(tilt_of(&pointer_commands(&overlay, &tree)).abs() > 0.05, "swaying");
    mouse_at(&mut overlay, 60.0, 60.0, later);
    prepare(&mut overlay, &tree, later + Duration::from_millis(5));
    assert_eq!(tilt_of(&pointer_commands(&overlay, &tree)), 0.0, "the move straightened it");

    let mut held = AttachOverlay::default();
    held.note_pointer(pointer(false, 0, MarkerPhase::Pressed, 50.0, 50.0), t);
    prepare(&mut held, &tree, t + RIPPLE + IDLE_AFTER + IDLE_PERIOD / 4);
    assert_eq!(tilt_of(&pointer_commands(&held, &tree)), 0.0, "a held button does not sway");

    let mut finger = AttachOverlay::default();
    finger.note_pointer(pointer(true, 1, MarkerPhase::Pressed, 50.0, 50.0), t);
    prepare(&mut finger, &tree, t + RIPPLE + IDLE_AFTER + IDLE_PERIOD / 4);
    assert_eq!(finger.wake_in(t + RIPPLE + IDLE_AFTER * 2), None, "a finger on the glass does not idle");
}

// ---- the pointer's node only covers the pointer

#[test]
fn the_pointer_node_hugs_the_markers_so_the_idle_animation_repaints_a_small_area() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 500.0, 300.0, t);
    prepare(&mut overlay, &tree, t);
    let bounds = tree.element_bounds(overlay.pointer_node().unwrap()).unwrap();
    assert!(bounds.width < 120.0 && bounds.height < 120.0, "not the whole window: {bounds:?}");
    assert!(bounds.x < 500.0 && bounds.x + bounds.width > 500.0 && bounds.y < 300.0 && bounds.y + bounds.height > 300.0);
}

#[test]
fn moving_a_pointer_damages_where_it_was_and_where_it_is() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 100.0, 100.0, t);
    prepare(&mut overlay, &tree, t);
    tree.take_damage();
    mouse_at(&mut overlay, 800.0, 500.0, t + Duration::from_millis(20));
    prepare(&mut overlay, &tree, t + Duration::from_millis(20));
    let damage = tree.take_damage();
    let covers = |x: f32, y: f32| damage.iter().any(|r| r.x <= x && x <= r.x + r.width && r.y <= y && y <= r.y + r.height);
    assert!(covers(100.0, 100.0) && covers(800.0, 500.0), "the old place is restored and the new one painted: {damage:?}");
}

#[test]
fn a_glide_step_or_a_sway_frame_repaints_a_small_square_not_the_window() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 500.0, 300.0, t);
    prepare(&mut overlay, &tree, t);
    tree.take_damage();
    let area = |damage: &[aimer_cupid::draw_cmd::Rect]| damage.iter().map(|r| r.width * r.height).sum::<f32>();

    // One step of a glide.
    mouse_at(&mut overlay, 520.0, 308.0, t + Duration::from_millis(16));
    prepare(&mut overlay, &tree, t + Duration::from_millis(16));
    let step = area(&tree.take_damage());
    assert!(step > 0.0 && step < 20_000.0, "a step damaged {step} px² of a 600,000 px² window");

    // One frame of the sway.
    prepare(&mut overlay, &tree, t + IDLE_AFTER + IDLE_PERIOD / 4);
    let breath = area(&tree.take_damage());
    assert!(breath > 0.0 && breath < 10_000.0, "a sway frame damaged {breath} px²");
}

#[test]
fn a_pointer_at_the_window_edge_keeps_its_node_inside_the_window() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    for (x, y) in [(0.0, 0.0), (1000.0, 600.0), (-30.0, 700.0)] {
        mouse_at(&mut overlay, x, y, t);
        prepare(&mut overlay, &tree, t);
        let b = tree.element_bounds(overlay.pointer_node().unwrap()).unwrap();
        assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.width <= 1000.0 + 0.01 && b.y + b.height <= 600.0 + 0.01, "({x}, {y}): {b:?}");
    }
}


// ---- fading in and out when a driver attaches and leaves

/// How opaque the badge's pill is painted, from 0 to 1.
fn pill_alpha(commands: &[DrawCommand]) -> f32 {
    commands
        .iter()
        .find_map(|c| if let DrawCommand::FillRect { color, .. } = c { Some(color.a as f32 / 255.0) } else { None })
        .expect("the pill")
}

/// How opaque the badge's icon is painted: no override means fully.
fn icon_alpha(commands: &[DrawCommand]) -> f32 {
    let (_, overrides) = svg_in(commands).expect("the icon");
    overrides.iter().find_map(|o| o.opacity).unwrap_or(1.0)
}

fn badge_commands(overlay: &AttachOverlay, tree: &RenderTree) -> Vec<DrawCommand> {
    commands(tree, overlay.badge_node().expect("a badge node"))
}

#[test]
fn the_badge_fades_in_when_a_driver_attaches() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    overlay.set_names(names(&["aimer-cli"]));
    prepare(&mut overlay, &tree, t);
    assert!(pill_alpha(&badge_commands(&overlay, &tree)) < 0.05, "it starts invisible");

    prepare(&mut overlay, &tree, t + PRESENCE_FADE / 2);
    let half = badge_commands(&overlay, &tree);
    let (pill, icon) = (pill_alpha(&half), icon_alpha(&half));
    assert!(pill > 0.2 && pill < 0.8, "half way: {pill}");
    assert!((pill - icon).abs() < 0.02, "the icon fades with the pill: {pill} vs {icon}");

    prepare(&mut overlay, &tree, t + PRESENCE_FADE + Duration::from_millis(1));
    let done = badge_commands(&overlay, &tree);
    assert_eq!(pill_alpha(&done), 1.0);
    assert_eq!(icon_alpha(&done), 1.0);
}

#[test]
fn the_badge_fades_out_when_the_last_driver_leaves_and_is_then_removed() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    overlay.set_names(names(&["aimer-cli"]));
    prepare(&mut overlay, &tree, t);
    let shown = t + PRESENCE_FADE * 2;
    prepare(&mut overlay, &tree, shown);
    let node = overlay.badge_node().unwrap();
    assert_eq!(pill_alpha(&commands(&tree, node)), 1.0, "premise: fully shown");

    overlay.set_names(Vec::new());
    prepare(&mut overlay, &tree, shown);
    prepare(&mut overlay, &tree, shown + PRESENCE_FADE / 2);
    let half = commands(&tree, node);
    assert!(!half.is_empty(), "still on screen while it fades");
    let pill = pill_alpha(&half);
    assert!(pill > 0.2 && pill < 0.8, "half way down: {pill}");

    tree.take_damage();
    prepare(&mut overlay, &tree, shown + PRESENCE_FADE + Duration::from_millis(1));
    assert!(commands(&tree, node).is_empty(), "gone once it has faded");
    assert!(!tree.take_damage().is_empty(), "and the pixels under it are restored");
}

#[test]
fn a_driver_that_returns_mid_fade_fades_back_in_from_where_it_was() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    overlay.set_names(names(&["aimer-cli"]));
    prepare(&mut overlay, &tree, t);
    prepare(&mut overlay, &tree, t + PRESENCE_FADE * 2);
    let leaving = t + PRESENCE_FADE * 2;
    overlay.set_names(Vec::new());
    prepare(&mut overlay, &tree, leaving);
    prepare(&mut overlay, &tree, leaving + PRESENCE_FADE / 2);
    let low = pill_alpha(&badge_commands(&overlay, &tree));

    let back = leaving + PRESENCE_FADE / 2;
    overlay.set_names(names(&["aimer-cli"]));
    prepare(&mut overlay, &tree, back);
    let at_return = pill_alpha(&badge_commands(&overlay, &tree));
    assert!((at_return - low).abs() < 0.02, "no jump back to 0 or 1: {low} then {at_return}");
    prepare(&mut overlay, &tree, back + PRESENCE_FADE + Duration::from_millis(1));
    assert_eq!(pill_alpha(&badge_commands(&overlay, &tree)), 1.0);
}

#[test]
fn the_pointer_fades_in_when_it_first_appears() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 300.0, 200.0, t);
    prepare(&mut overlay, &tree, t);
    let start = icon_alpha(&pointer_commands(&overlay, &tree));
    assert!(start < 0.05, "it starts invisible: {start}");

    prepare(&mut overlay, &tree, t + PRESENCE_FADE / 2);
    let half = icon_alpha(&pointer_commands(&overlay, &tree));
    assert!(half > 0.1 * POINTER_OPACITY && half < 0.9 * POINTER_OPACITY, "half way: {half}");

    prepare(&mut overlay, &tree, t + PRESENCE_FADE + Duration::from_millis(1));
    assert!((icon_alpha(&pointer_commands(&overlay, &tree)) - POINTER_OPACITY).abs() < 1e-4, "then as opaque as it is configured to be");
}

#[test]
fn the_pointers_fade_out_with_the_driver_and_are_forgotten_after() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    overlay.set_names(names(&["aimer-cli"]));
    let t = AnimInstant::now();
    mouse_at(&mut overlay, 300.0, 200.0, t);
    overlay.note_pointer(pointer(true, 4, MarkerPhase::Pressed, 50.0, 60.0), t);
    let shown = t + PRESENCE_FADE * 2;
    prepare(&mut overlay, &tree, shown);
    let node = overlay.pointer_node().unwrap();

    overlay.set_names(Vec::new());
    prepare(&mut overlay, &tree, shown);
    prepare(&mut overlay, &tree, shown + PRESENCE_FADE / 2);
    let half = commands(&tree, node);
    assert!(svg_in(&half).is_some(), "the mouse is still drawn while it fades");
    let opacity = icon_alpha(&half);
    assert!(opacity > 0.1 * POINTER_OPACITY && opacity < 0.9 * POINTER_OPACITY, "{opacity}");
    assert_eq!(overlay.markers().len(), 2, "the driver's pointers are kept until they are gone");

    prepare(&mut overlay, &tree, shown + PRESENCE_FADE + Duration::from_millis(1));
    assert!(commands(&tree, node).is_empty(), "gone");
    assert!(overlay.markers().is_empty(), "and forgotten");
}

#[test]
fn the_overlay_asks_for_frames_while_it_fades_and_then_stops() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    overlay.set_names(names(&["aimer-cli"]));
    prepare(&mut overlay, &tree, t);
    assert_eq!(overlay.wake_in(t + PRESENCE_FADE / 2), Some(FRAME), "fading in");
    prepare(&mut overlay, &tree, t + PRESENCE_FADE * 2);
    assert_eq!(overlay.wake_in(t + PRESENCE_FADE * 2), None, "a shown badge with no pointer needs nothing");

    let gone = t + PRESENCE_FADE * 2;
    overlay.set_names(Vec::new());
    prepare(&mut overlay, &tree, gone);
    assert_eq!(overlay.wake_in(gone + PRESENCE_FADE / 2), Some(FRAME), "fading out");
    prepare(&mut overlay, &tree, gone + PRESENCE_FADE + Duration::from_millis(1));
    assert_eq!(overlay.wake_in(gone + PRESENCE_FADE * 2), None, "and done");
}

#[test]
fn a_settled_overlay_does_not_repaint() {
    let tree = RenderTree::new();
    let mut overlay = AttachOverlay::default();
    let t = AnimInstant::now();
    overlay.set_names(names(&["aimer-cli"]));
    prepare(&mut overlay, &tree, t);
    prepare(&mut overlay, &tree, t + PRESENCE_FADE + Duration::from_millis(1));
    tree.take_damage();
    prepare(&mut overlay, &tree, t + PRESENCE_FADE * 2);
    assert!(tree.take_damage().is_empty(), "fully shown and unchanged: nothing to repaint");
}
