//! An effect registered by the application draws through the real renderer.

use std::sync::Arc;

use super::mask_group_tests::{SIZE, disc, near, pixel, record, render};
use crate::AntiAlias;
use crate::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use crate::effects::{EffectCommand, ParamWriter, ShaderEffect, register};
use crate::utilities::Color;

const SOLID: &str = include_str!("../../../pipelines/effect/tests/shaders/solid.aimer");

struct Solid([f32; 4]);

impl ShaderEffect for Solid {
    const NAME: &'static str = "test-solid";
    const SOURCE: &'static str = SOLID;

    fn write_params(&self, params: &mut ParamWriter) {
        params.vec4(self.0);
    }
}

struct Water {
    amplitude: f32,
}

impl ShaderEffect for Water {
    const NAME: &'static str = "test-water";
    const SOURCE: &'static str = include_str!("../../../pipelines/effect/tests/shaders/water.aimer");
    const SAMPLES_BACKDROP: bool = true;

    fn write_params(&self, params: &mut ParamWriter) {
        params.f32(0.0).f32(self.amplitude).f32(12.0).pad(4);
    }
}

fn custom<E: ShaderEffect>(effect: &E, rect: [f32; 4]) -> DrawCommand {
    let command = EffectCommand::new(effect, rect).unwrap();
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(command.name),
        data: command.payload,
    }
}

/// root -> node at (16, 16), 64x64, recording `commands` in local space.
fn scene(commands: Vec<DrawCommand>) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let node = tree.add_child(root, Rect::new(16.0, 16.0, 64.0, 64.0)).unwrap();
    record(&tree, node, commands);
    tree
}

#[test]
fn a_registered_effect_draws_at_its_placement() {
    register::<Solid>().unwrap();
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let tree = scene(vec![custom(&Solid([0.0, 1.0, 0.0, 1.0]), [0.0, 0.0, 32.0, 32.0])]);
        let pixels = render(antialiasing, &tree);

        // Local (0..32) lands at (16..48) because the node sits at (16, 16).
        near(pixel(&pixels, 30, 30), [0, 255, 0, 255], 2, "inside the effect");
        near(pixel(&pixels, 8, 8), [0, 0, 0, 0], 0, "above and left of it");
        near(pixel(&pixels, 60, 30), [0, 0, 0, 0], 0, "right of it");
        near(pixel(&pixels, 30, 60), [0, 0, 0, 0], 0, "below it");
    }
}

#[test]
fn an_effect_is_clipped_and_ordered_with_the_commands_around_it() {
    register::<Solid>().unwrap();
    let tree = scene(vec![
        disc(Rect::new(0.0, 0.0, 64.0, 64.0), Color::rgba8(255, 0, 0, 255)),
        custom(&Solid([0.0, 0.0, 1.0, 1.0]), [16.0, 16.0, 32.0, 32.0]),
        DrawCommand::PushClip {
            rect: Rect::new(0.0, 0.0, 20.0, 64.0),
            border_radius: [0.0; 4],
        },
        custom(&Solid([1.0, 1.0, 0.0, 1.0]), [0.0, 40.0, 64.0, 16.0]),
        DrawCommand::PopClip,
    ]);
    let pixels = render(AntiAlias::Analytic, &tree);

    near(pixel(&pixels, 40, 40), [0, 0, 255, 255], 2, "the blue effect covers the red disc");
    near(pixel(&pixels, 26, 62), [255, 255, 0, 255], 2, "yellow inside its clip");
    near(pixel(&pixels, 70, 62), [255, 0, 0, 255], 2, "outside the clip only the red disc is there");
}

#[test]
fn an_effect_can_sample_what_was_drawn_behind_it() {
    register::<Water>().unwrap();
    let tree = scene(vec![
        disc(Rect::new(0.0, 0.0, 64.0, 64.0), Color::rgba8(255, 0, 0, 255)),
        // Zero amplitude: no refraction, so the result is the red mixed with
        // the shader's fixed tint at 35%.
        custom(&Water { amplitude: 0.0 }, [16.0, 16.0, 32.0, 32.0]),
    ]);
    let pixels = render(AntiAlias::Analytic, &tree);

    near(pixel(&pixels, 48, 48), [170, 27, 49, 255], 4, "red seen through the tint");
    near(pixel(&pixels, 70, 48), [255, 0, 0, 255], 4, "the disc beside the effect is untouched");
}
