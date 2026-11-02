//! A generic custom pipeline learns where its command sits in the frame.

use std::any::Any;
use std::sync::{Arc, Mutex};

use crate::AntiAlias;
use crate::backend::{metal::MetalBackend, *};
use crate::custom_pipeline::{CommandContext, CustomPipelineGeneric, RenderContextGeneric};
use crate::damage_region::DamageSet;
use crate::draw_cmd_v2::{DrawCommand, Rect, RenderFrame, RenderTree};
use crate::frame::{FramePacket, FrameRenderMetadata};
use crate::renderer::RendererImpl;

const SIZE: u32 = 64;

type Seen = Arc<Mutex<Vec<(Vec<u8>, Option<CommandContext>)>>>;

/// Records what the renderer hands it. `with_context` selects whether it
/// implements the placement-aware hook or only the plain one.
struct Probe {
    name: &'static str,
    with_context: bool,
    seen: Seen,
}

impl CustomPipelineGeneric<MetalBackend> for Probe {
    fn name(&self) -> &str {
        self.name
    }

    fn prepare(&mut self, _ctx: &RenderContextGeneric<'_, MetalBackend>) {}

    fn prepare_command(&mut self, data: &(dyn Any + Send)) -> Option<usize> {
        let bytes = data.downcast_ref::<Vec<u8>>().cloned().unwrap_or_default();
        self.seen.lock().unwrap().push((bytes, None));
        Some(0)
    }

    fn prepare_command_in(
        &mut self,
        data: &(dyn Any + Send),
        context: &CommandContext,
    ) -> Option<usize> {
        if !self.with_context {
            return self.prepare_command(data);
        }
        let bytes = data.downcast_ref::<Vec<u8>>().cloned().unwrap_or_default();
        self.seen.lock().unwrap().push((bytes, Some(*context)));
        Some(0)
    }

    fn render<'pass>(&'pass self, _pass: &mut <MetalBackend as GpuBackend>::RenderPass<'pass>) {}
}

fn custom(name: &str, payload: &[u8]) -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(name),
        data: Arc::from(payload),
    }
}

/// Renders a tree whose child sits at (16, 16) and draws one command per pipeline.
fn render_with(pipelines: Vec<Probe>, commands: Vec<DrawCommand>) {
    let backend = MetalBackend::new().expect("custom context test requires Metal");
    let format = <MetalBackend as GpuBackend>::rgba8_unorm_format();
    let target = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "custom context");
    let mut renderer =
        RendererImpl::<MetalBackend>::with_antialiasing(&backend, format, AntiAlias::Analytic);
    for probe in pipelines {
        renderer.register_custom_pipeline(probe);
    }

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let child = tree.add_child(root, Rect::new(16.0, 16.0, 32.0, 32.0)).unwrap();
    tree.context(child)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(commands)
        .unwrap();
    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(SIZE, SIZE)),
    )
    .expect("lower the custom commands");
    renderer.render_packet(&backend, &view, &packet, false);
}

#[test]
fn a_custom_command_receives_its_placement() {
    let seen = Seen::default();
    render_with(
        vec![Probe {
            name: "probe",
            with_context: true,
            seen: seen.clone(),
        }],
        vec![custom("probe", &[1, 2, 3])],
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (payload, context) = &seen[0];
    assert_eq!(payload, &[1, 2, 3]);
    let context = context.expect("the placement-aware hook was called");
    // The node sits at (16, 16) in the target; local (0, 0) must land there.
    assert_eq!(context.map_point(0.0, 0.0), (16.0, 16.0));
    assert_eq!(context.map_rect([0.0, 0.0, 8.0, 4.0]), [16.0, 16.0, 8.0, 4.0]);
    assert_eq!(context.alpha, 1.0);
    assert!(context.clip.is_none());
}

#[test]
fn a_pipeline_without_the_context_hook_is_still_called() {
    let seen = Seen::default();
    render_with(
        vec![Probe {
            name: "plain",
            with_context: false,
            seen: seen.clone(),
        }],
        vec![custom("plain", &[9])],
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, [9]);
    assert!(seen[0].1.is_none(), "reached the plain hook");
}
