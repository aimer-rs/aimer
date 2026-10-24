//! A node faded by its opacity still shows the backdrop effects inside it.

use std::sync::Arc;

use super::mask_group_tests::{SIZE, near, pixel, record, render};
use crate::AntiAlias;
use crate::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use crate::pipeline::material::{MATERIAL_PIPELINE_NAME, MaterialKind, MaterialRequest};
use crate::utilities::Color;

fn half(color: Color, rect: Rect) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color,
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

/// root (red on the left, blue on the right) -> a 96 x 96 node at `opacity` that
/// desaturates what is behind it.
fn faded_filter(opacity: f32) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let node = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    record(
        &tree,
        root,
        vec![
            half(Color::rgba8(255, 0, 0, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
            half(Color::rgba8(0, 0, 255, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
        ],
    );
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 96.0, 96.0]);
    filter.blur_radius = 0.0;
    filter.saturation = 0.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    record(
        &tree,
        node,
        vec![DrawCommand::DrawCustom {
            pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
            data: Arc::from(filter.encode_for_test()),
        }],
    );
    tree.set_opacity(node, opacity).unwrap();
    tree
}

/// The desaturated red is a dark grey, so fading it half way into the red under
/// it leaves a pixel with some green and blue in it; a filter that was dropped
/// leaves the red as it was. This is what a glass bar fading in must do: its
/// frosted background is there the whole way, not once it is fully shown.
#[test]
fn a_backdrop_filter_in_a_faded_node_filters_the_backdrop() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let pixels = render(antialiasing, &faded_filter(0.5));
        let over_red = pixel(&pixels, 40, 64);
        assert!(
            over_red[1] > 15 && over_red[0] < 230,
            "{antialiasing:?}: half the desaturated backdrop shows over the red: {over_red:?}"
        );
        near(pixel(&pixels, 4, 64), [255, 0, 0, 255], 2, "outside the node is untouched");
    }
}

/// The same at full opacity, where the node needs no group of its own, so the two
/// ways of drawing a node agree.
#[test]
fn a_backdrop_filter_in_an_opaque_node_filters_the_backdrop() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let pixels = render(antialiasing, &faded_filter(1.0));
        let over_red = pixel(&pixels, 40, 64);
        assert!(
            over_red[0].abs_diff(over_red[1]) <= 3 && over_red[1].abs_diff(over_red[2]) <= 3,
            "{antialiasing:?}: the red is desaturated to grey: {over_red:?}"
        );
    }
}

/// root (red on the left, blue on the right) -> a 96 x 96 node at `opacity` that
/// first draws a green fill half transparent, then a filter that changes nothing
/// of the backdrop it is given: the shape of a glass surface, a tint with a
/// filter over it.
fn tinted_then_filtered(opacity: f32) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let node = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    record(
        &tree,
        root,
        vec![
            half(Color::rgba8(255, 0, 0, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
            half(Color::rgba8(0, 0, 255, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
        ],
    );
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 96.0, 96.0]);
    filter.blur_radius = 0.0;
    filter.saturation = 1.0;
    filter.brightness = 1.0;
    filter.contrast = 1.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    record(
        &tree,
        node,
        vec![
            half(Color::rgba8(0, 255, 0, 128), Rect::new(0.0, 0.0, 96.0, 96.0)),
            DrawCommand::DrawCustom {
                pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                data: Arc::from(filter.encode_for_test()),
            },
        ],
    );
    tree.set_opacity(node, opacity).unwrap();
    tree
}

/// What a node drew before a backdrop filter is part of the backdrop the filter
/// sees, whether or not the node is faded: a glass bar fading in must show its tint
/// the whole way, not only once it is fully shown and needs no group.
#[test]
fn a_filter_in_a_faded_node_sees_what_the_node_drew_before_it() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        // Opaque: the tint over the red, passed through the filter as it is.
        let pixels = render(antialiasing, &tinted_then_filtered(1.0));
        near(pixel(&pixels, 40, 64), [127, 128, 0, 255], 6, &format!("{antialiasing:?} opaque"));

        // Half faded: that, half way into the red behind it.
        let pixels = render(antialiasing, &tinted_then_filtered(0.5));
        near(pixel(&pixels, 40, 64), [191, 64, 0, 255], 6, &format!("{antialiasing:?} half faded over red"));
        near(pixel(&pixels, 90, 64), [0, 64, 191, 255], 6, &format!("{antialiasing:?} half faded over blue"));
    }
}

/// A node at one half that draws an opaque green block, a filter over a block
/// of its own, and nothing over the rest of it. Starting such a group from the
/// frame behind it is meant to change nothing that does not sample the backdrop.
fn block_filter_and_nothing() -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let node = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    record(
        &tree,
        root,
        vec![
            half(Color::rgba8(255, 0, 0, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
            half(Color::rgba8(0, 0, 255, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
        ],
    );
    // The filter covers the right half of the node, over the blue.
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [48.0, 0.0, 48.0, 96.0]);
    filter.blur_radius = 0.0;
    filter.saturation = 0.0;
    filter.brightness = 1.0;
    filter.contrast = 1.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    record(
        &tree,
        node,
        vec![
            half(Color::rgba8(0, 255, 0, 255), Rect::new(0.0, 0.0, 48.0, 48.0)),
            DrawCommand::DrawCustom {
                pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                data: Arc::from(filter.encode_for_test()),
            },
        ],
    );
    tree.set_opacity(node, 0.5).unwrap();
    tree
}

#[test]
fn a_group_that_holds_a_filter_fades_the_rest_of_what_it_draws_as_any_group_does() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let pixels = render(antialiasing, &block_filter_and_nothing());
        // The green block, half way into the red under it.
        near(pixel(&pixels, 30, 30), [127, 128, 0, 255], 6, &format!("{antialiasing:?} block"));
        // Where the node draws nothing, the frame is as it was.
        near(pixel(&pixels, 40, 88), [255, 0, 0, 255], 2, &format!("{antialiasing:?} nothing drawn"));
        near(pixel(&pixels, 4, 64), [255, 0, 0, 255], 2, &format!("{antialiasing:?} outside the node"));
        // The filter's grey, half way into the blue under it.
        let filtered = pixel(&pixels, 90, 88);
        assert!(
            filtered[0].abs_diff(filtered[1]) <= 3 && (120..150).contains(&filtered[2]),
            "{antialiasing:?}: desaturated blue half way into the blue: {filtered:?}"
        );
    }
}

/// root (red left, blue right) -> a node at 0.8 -> a node at 0.5 inside it, which
/// draws the half-transparent green fill and then the filter that changes nothing.
fn tinted_then_filtered_in_two_groups() -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let outer = tree.add_child(root, Rect::new(8.0, 8.0, 112.0, 112.0)).unwrap();
    let inner = tree.add_child(outer, Rect::new(8.0, 8.0, 96.0, 96.0)).unwrap();
    record(
        &tree,
        root,
        vec![
            half(Color::rgba8(255, 0, 0, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
            half(Color::rgba8(0, 0, 255, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
        ],
    );
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 96.0, 96.0]);
    filter.blur_radius = 0.0;
    filter.saturation = 1.0;
    filter.brightness = 1.0;
    filter.contrast = 1.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    record(
        &tree,
        inner,
        vec![
            half(Color::rgba8(0, 255, 0, 128), Rect::new(0.0, 0.0, 96.0, 96.0)),
            DrawCommand::DrawCustom {
                pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                data: Arc::from(filter.encode_for_test()),
            },
        ],
    );
    tree.set_opacity(outer, 0.8).unwrap();
    tree.set_opacity(inner, 0.5).unwrap();
    tree
}

/// A faded node inside a faded node keeps its tint through its filter as well: the
/// reveal of a glass bar that dissolves in is drawn so, a group inside a group.
#[test]
fn a_filter_in_a_group_inside_a_group_sees_what_the_group_drew_before_it() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let pixels = render(antialiasing, &tinted_then_filtered_in_two_groups());
        // The tint over the red is half way (191, 64, 0) at the inner node's 0.5,
        // and four fifths of the way into the red at the outer node's 0.8.
        near(pixel(&pixels, 40, 64), [204, 51, 0, 255], 8, &format!("{antialiasing:?} over red"));
        near(pixel(&pixels, 90, 64), [0, 51, 204, 255], 8, &format!("{antialiasing:?} over blue"));
    }
}
