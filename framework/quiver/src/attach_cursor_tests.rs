use aimer_utils::cursor::CursorIcon;

use super::*;
use crate::attach_overlay::ICON_TIP_UNITS;

const EVERY_LOOK: [Look; LOOKS] = [
    Look::Arrow,
    Look::IBeam,
    Look::PointingHand,
    Look::OpenHand,
    Look::ResizeVertical,
    Look::ResizeHorizontal,
    Look::ResizeRising,
    Look::ResizeFalling,
    Look::Move,
    Look::Crosshair,
    Look::Busy,
    Look::Forbidden,
    Look::ZoomIn,
    Look::ZoomOut,
];

#[test]
fn an_icon_picks_the_look_a_person_would_expect() {
    assert_eq!(Look::of(CursorIcon::Default), Look::Arrow);
    assert_eq!(Look::of(CursorIcon::Pointer), Look::PointingHand);
    assert_eq!(Look::of(CursorIcon::Text), Look::IBeam);
    assert_eq!(Look::of(CursorIcon::Grabbing), Look::OpenHand);
    assert_eq!(Look::of(CursorIcon::NsResize), Look::ResizeVertical);
    assert_eq!(Look::of(CursorIcon::RowResize), Look::ResizeVertical);
    assert_eq!(Look::of(CursorIcon::ColResize), Look::ResizeHorizontal);
    assert_eq!(Look::of(CursorIcon::EResize), Look::ResizeHorizontal);
    assert_eq!(Look::of(CursorIcon::NeswResize), Look::ResizeRising);
    assert_eq!(Look::of(CursorIcon::SwResize), Look::ResizeRising);
    assert_eq!(Look::of(CursorIcon::NwseResize), Look::ResizeFalling);
    assert_eq!(Look::of(CursorIcon::NwResize), Look::ResizeFalling);
    assert_eq!(Look::of(CursorIcon::AllScroll), Look::Move);
    assert_eq!(Look::of(CursorIcon::Wait), Look::Busy);
    assert_eq!(Look::of(CursorIcon::NotAllowed), Look::Forbidden);
}

#[test]
fn an_icon_without_a_drawing_of_its_own_is_the_arrow() {
    for icon in [CursorIcon::ContextMenu, CursorIcon::Cell, CursorIcon::Alias, CursorIcon::Help] {
        assert_eq!(Look::of(icon), Look::Arrow, "{icon:?}");
    }
}

#[test]
fn every_look_parses_and_draws_something() {
    for look in EVERY_LOOK {
        let glyph = look.glyph();
        let drawn = glyph.scene.nodes.iter().filter(|n| n.geometry.is_some() && !n.is_definition).count();
        assert!(drawn > 0, "{look:?} draws nothing");
        assert_eq!(glyph.overrides(None, 1.0).len(), drawn, "{look:?} overrides every drawn part");
    }
}

#[test]
fn a_hotspot_is_inside_the_icon_box() {
    for look in EVERY_LOOK {
        let (x, y) = look.glyph().hotspot;
        assert!((0.0..=ICON_VIEWBOX).contains(&x) && (0.0..=ICON_VIEWBOX).contains(&y), "{look:?}: ({x}, {y})");
    }
}

#[test]
fn the_arrow_keeps_its_own_colours_and_its_tip_is_the_hotspot() {
    let arrow = Look::Arrow.glyph();
    assert_eq!(arrow.hotspot, (ICON_TIP_UNITS, ICON_TIP_UNITS));
    assert!(arrow.overrides(None, 1.0).iter().all(|o| o.fill.is_none() && o.stroke.is_none()));
}

#[test]
fn a_halo_icon_has_a_white_line_under_a_dark_one() {
    let scene = &Look::PointingHand.glyph().scene;
    let strokes: Vec<_> = scene.nodes.iter().filter(|n| n.geometry.is_some()).filter_map(|n| n.stroke.as_ref()).collect();
    // The hand is five paths, each drawn once in each colour.
    assert_eq!(strokes.len(), 10, "{strokes:?}");
    let white = strokes.iter().filter(|s| s.color.r > 0.9 && s.color.g > 0.9 && s.color.b > 0.9).count();
    let dark = strokes.iter().filter(|s| s.color.r < 0.25 && s.color.g < 0.25 && s.color.b < 0.25).count();
    assert_eq!((white, dark), (5, 5));
    let widths: Vec<f32> = strokes.iter().map(|s| s.width).collect();
    let widest_white = strokes.iter().filter(|s| s.color.r > 0.9).map(|s| s.width).fold(0.0, f32::max);
    let widest_dark = strokes.iter().filter(|s| s.color.r < 0.25).map(|s| s.width).fold(0.0, f32::max);
    assert!(widest_white > widest_dark, "the halo is the wider line: {widths:?}");
}

#[test]
fn a_resize_arrow_is_recoloured_to_read_on_any_page() {
    let overrides = Look::ResizeVertical.glyph().overrides(None, 1.0);
    for o in overrides.iter() {
        let fill = o.fill.flatten().expect("a fill");
        let stroke = o.stroke.flatten().expect("a stroke");
        assert!(fill.r < 0.25 && stroke.r > 0.9, "a dark body and a white outline");
    }
}

#[test]
fn the_resize_looks_are_the_vertical_one_turned() {
    let angle = |look: Look| look.glyph().turn;
    assert_eq!(angle(Look::ResizeVertical), 0.0);
    assert!((angle(Look::ResizeHorizontal) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert!((angle(Look::ResizeRising) - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    assert!((angle(Look::ResizeFalling) + std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    // The turned ones carry it on every drawn part, not only when swaying.
    let o = Look::ResizeHorizontal.glyph().overrides(None, 1.0);
    assert!(o.iter().all(|o| o.transform.is_some()));
}

#[test]
fn a_turn_leaves_the_hotspot_where_it_was() {
    let glyph = Look::ResizeRising.glyph();
    let unit = glyph.scene.viewport.width / ICON_VIEWBOX;
    let (px, py) = (glyph.hotspot.0 * unit, glyph.hotspot.1 * unit);
    let t = rotation_about(glyph.turn, px, py);
    let (x, y) = (t.sx * px + t.kx * py + t.tx, t.ky * px + t.sy * py + t.ty);
    assert!((x - px).abs() < 1e-3 && (y - py).abs() < 1e-3, "({x}, {y}) vs ({px}, {py})");
}

#[test]
fn a_settled_glyph_shares_its_overrides_and_a_fading_one_is_translucent() {
    let glyph = Look::Arrow.glyph();
    assert!(Arc::ptr_eq(&glyph.overrides(None, 1.0), &glyph.overrides(None, 1.0)));
    let faded = glyph.overrides(None, 0.5);
    assert!(faded.iter().all(|o| o.opacity == Some(POINTER_OPACITY * 0.5)));
}
