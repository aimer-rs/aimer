use crate::{BorderRadius, Decoration, ShaderEffectDecoration};
use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_cupid::effects::{self, ParamWriter, ShaderEffect, decode_payload};

/// A flat-color rectangle: the smallest shader that satisfies the contract.
const SOLID: &str = "\
shader decoration_solid;

import aimer::clip::clip_alpha;

uniform struct Frame {
    target_size: vec2,
    surface_is_srgb: f32,
    opacity: f32,
    rect: vec4,
    clip_rect: vec4,
    clip_radii: vec4,
    color: vec4,
}

#[bind(0, 0)]
uniform frame: Frame;

#[varying]
struct VertexOutput {
    @position position: vec4,
    pixel_pos: vec2,
}

const CORNERS: [vec2; 6] = [
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
];

#[vertex]
fn vs_main(vi: @vertex_index) -> VertexOutput {
    let pixel = frame.rect.xy + CORNERS[vi] * frame.rect.zw;
    let ndc = vec2(
        pixel.x / frame.target_size.x * 2.0 - 1.0,
        1.0 - pixel.y / frame.target_size.y * 2.0,
    );
    VertexOutput { position: vec4(ndc, 0.0, 1.0), pixel_pos: pixel }
}

#[fragment]
fn fs_main(input: VertexOutput) -> @frag_color {
    let a = frame.color.a * frame.opacity * clip_alpha(input.pixel_pos, frame.clip_rect, frame.clip_radii);
    vec4(frame.color.rgb * a, a)
}
";

#[derive(Clone, Copy)]
struct Solid([f32; 4]);

impl ShaderEffect for Solid {
    const NAME: &'static str = "decoration_solid";
    const SOURCE: &'static str = SOLID;

    fn write_params(&self, params: &mut ParamWriter) {
        params.vec4(self.0);
    }
}

/// Writes 4 bytes, which the 16-byte row rule rejects.
struct Misaligned;

impl ShaderEffect for Misaligned {
    const NAME: &'static str = "decoration_misaligned";
    const SOURCE: &'static str = SOLID;

    fn write_params(&self, params: &mut ParamWriter) {
        params.f32(1.0);
    }
}

const SIZE: ResolvedSize = ResolvedSize { width: 80.0, height: 40.0 };

fn record<D: Decoration + ?Sized>(
    decoration: &D, size: ResolvedSize, scale: f32, prelude: impl FnOnce(&Canvas),
) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let canvas = Canvas::of(&tree.context(root).unwrap());
    prelude(&canvas);
    decoration.paint(&canvas, Vec2d { x: 3.0, y: 4.0 }, size, scale);
    canvas.finish();
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

fn payloads(commands: &[DrawCommand]) -> Vec<&[u8]> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => Some(&data[..]),
            _ => None,
        })
        .collect()
}

#[test]
fn paints_one_effect_command_over_the_box() {
    let commands = record(&ShaderEffectDecoration::new(Solid([0.1, 0.2, 0.3, 1.0])), SIZE, 2.0, |_| {});
    let [DrawCommand::DrawCustom { pipeline_name, data }] = commands.as_slice() else {
        panic!("one effect command, got {commands:?}")
    };
    assert_eq!(&**pipeline_name, "decoration_solid");
    // The rectangle is the logical box; the render tree's transform scales it.
    let (rect, params) = decode_payload(data).unwrap();
    assert_eq!(rect, [3.0, 4.0, 80.0, 40.0]);
    let expected: Vec<u8> = [0.1f32, 0.2, 0.3, 1.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(params, &expected[..]);
}

#[test]
fn new_registers_the_effect_so_the_renderer_can_find_it() {
    let _ = ShaderEffectDecoration::new(Solid([0.0; 4]));
    assert!(effects::registered().iter().any(|spec| spec.name() == "decoration_solid"));
}

#[test]
fn an_empty_box_or_a_bad_scale_paints_nothing() {
    let decoration = ShaderEffectDecoration::new(Solid([1.0; 4]));
    for size in [
        ResolvedSize { width: 0.0, height: 40.0 },
        ResolvedSize { width: 80.0, height: -1.0 },
        ResolvedSize { width: f32::NAN, height: 40.0 },
    ] {
        assert!(record(&decoration, size, 2.0, |_| {}).is_empty(), "{size:?}");
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(record(&decoration, SIZE, scale, |_| {}).is_empty(), "scale {scale}");
    }
}

#[test]
fn an_effect_that_breaks_the_contract_paints_nothing_and_does_not_panic() {
    let decoration = ShaderEffectDecoration::new(Misaligned);
    // Twice: the failure is reported once, and every later frame still skips.
    assert!(payloads(&record(&decoration, SIZE, 2.0, |_| {})).is_empty());
    assert!(payloads(&record(&decoration, SIZE, 2.0, |_| {})).is_empty());
}

#[test]
fn a_preceding_border_radius_rounds_the_effect_with_a_clip() {
    let decoration = ShaderEffectDecoration::new(Solid([1.0; 4]));
    let radius = BorderRadius::from(12);
    let commands = record(&decoration, SIZE, 2.0, |canvas| {
        radius.paint(canvas, Vec2d { x: 3.0, y: 4.0 }, SIZE, 2.0);
    });
    let [DrawCommand::PushClip { rect, border_radius }, DrawCommand::DrawCustom { .. }, DrawCommand::PopClip] =
        commands.as_slice()
    else {
        panic!("clip, effect, unclip; got {commands:?}")
    };
    assert_eq!((rect.x, rect.y, rect.width, rect.height), (3.0, 4.0, 80.0, 40.0));
    assert_eq!(*border_radius, [12.0; 4]);
}

#[test]
fn square_corners_add_no_clip() {
    let decoration = ShaderEffectDecoration::new(Solid([1.0; 4]));
    let commands = record(&decoration, SIZE, 2.0, |_| {});
    assert!(matches!(commands.as_slice(), [DrawCommand::DrawCustom { .. }]));
}

#[test]
fn a_radius_larger_than_half_the_short_side_is_limited() {
    let decoration = ShaderEffectDecoration::new(Solid([1.0; 4]));
    let radius = BorderRadius::from(500);
    let commands = record(&decoration, SIZE, 2.0, |canvas| {
        radius.paint(canvas, Vec2d { x: 3.0, y: 4.0 }, SIZE, 2.0);
    });
    let Some(DrawCommand::PushClip { border_radius, .. }) = commands.first() else {
        panic!("clip first; got {commands:?}")
    };
    assert_eq!(*border_radius, [20.0; 4]);
}

#[test]
fn the_decoration_declares_no_geometry() {
    let decoration = ShaderEffectDecoration::new(Solid([1.0; 4]));
    assert_eq!(decoration.paint_outsets(SIZE, 2.0), [0.0; 4]);
    assert_eq!(decoration.content_insets(SIZE, 2.0), [0.0; 4]);
    assert_eq!(decoration.child_clip_radius(SIZE, 2.0), None);
    assert_eq!(decoration.background_color(), None);
    assert!(!decoration.is_empty());
}

#[test]
fn revision_defaults_to_zero_and_is_set_by_the_caller() {
    let decoration = ShaderEffectDecoration::new(Solid([1.0; 4]));
    assert_eq!(decoration.revision(), 0);
    assert_eq!(decoration.with_revision(7).revision(), 7);
}

#[test]
fn it_stacks_with_other_decorations() {
    let stack = [BorderRadius::from(8).notype(), ShaderEffectDecoration::new(Solid([1.0; 4])).notype()];
    let commands = record(&stack[..], SIZE, 2.0, |_| {});
    assert_eq!(payloads(&commands).len(), 1);
    assert!(matches!(commands.first(), Some(DrawCommand::PushClip { .. })));
}
