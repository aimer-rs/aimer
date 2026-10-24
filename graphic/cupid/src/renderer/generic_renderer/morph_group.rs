//! Morph groups: two source groups blended by one shader draw.
//!
//! The plan wraps a morph node's two children in source groups inside a host
//! group (see [`MORPH_PIPELINE_NAME`]). The host owns no target. Each source is
//! drawn into an ordinary group target and, when it ends, is *kept* instead of
//! composited into its parent. When the host ends, one draw reads both kept
//! targets and writes the blend into the parent.
//!
//! [`MORPH_PIPELINE_NAME`]: crate::draw_cmd_v2::MORPH_PIPELINE_NAME

use super::{ActiveOpacityGroup, Renderer};
use crate::backend::{
    GpuBackend, LoadOp, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
};
use crate::damage_region::DamageRect;
use crate::draw_cmd_v2::{MorphGroup, MorphParams};
use crate::pipeline::morph_pipeline::{MorphInstance, MorphPipeline};

/// Which picture of its host a source group draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MorphSource {
    From,
    To,
}

impl MorphSource {
    #[inline]
    pub(super) fn of(group: Option<MorphGroup>) -> Option<Self> {
        match group {
            Some(MorphGroup::From) => Some(Self::From),
            Some(MorphGroup::To) => Some(Self::To),
            Some(MorphGroup::Host(_)) | None => None,
        }
    }
}

/// A source group that has finished drawing and waits for its host to end.
pub(super) struct KeptSource<B: GpuBackend> {
    /// The group-target slot it occupies; no other group may draw into it until
    /// the host has composited.
    pub(super) depth: usize,
    pub(super) bounds: DamageRect,
    pub(super) sample_view: B::TextureView,
}

/// An open morph host.
pub(super) struct MorphFrame<B: GpuBackend> {
    pub(super) element: u64,
    pub(super) params: MorphParams,
    pub(super) opacity: f32,
    pub(super) bounds: DamageRect,
    /// How many groups were open when the host began; the host ends when that
    /// many are open again.
    pub(super) groups_open: usize,
    pub(super) from: Option<KeptSource<B>>,
    pub(super) to: Option<KeptSource<B>>,
}

impl<B: GpuBackend> MorphFrame<B> {
    fn kept(&self) -> usize {
        usize::from(self.from.is_some()) + usize::from(self.to.is_some())
    }
}

/// The group-target slot for the next group to open.
///
/// Open groups occupy the slots below `open`, and every kept source holds the
/// slot it drew into, so a new group goes above both.
#[inline]
pub(super) fn next_group_depth<B: GpuBackend>(open: usize, frames: &[MorphFrame<B>]) -> usize {
    open + frames.iter().map(MorphFrame::kept).sum::<usize>()
}

/// Whether ending a group now ends the host on top of `frames`.
#[inline]
pub(super) fn ends_host<B: GpuBackend>(open: usize, frames: &[MorphFrame<B>]) -> bool {
    frames.last().is_some_and(|frame| frame.groups_open == open)
}

impl<B: GpuBackend> Renderer<B> {
    /// Keeps a finished source for its host.
    pub(super) fn keep_morph_source(
        frames: &mut [MorphFrame<B>],
        role: MorphSource,
        group: &ActiveOpacityGroup<B>,
    ) {
        let frame = frames
            .last_mut()
            .expect("a morph source sits inside its host");
        let kept = Some(KeptSource {
            depth: group.depth,
            bounds: group.bounds,
            sample_view: group.sample_view.clone(),
        });
        match role {
            MorphSource::From => frame.from = kept,
            MorphSource::To => frame.to = kept,
        }
    }

    /// Blends a finished host's sources into the target `view`.
    ///
    /// `parent_origin` is the target's origin in frame pixels, `scissor` the
    /// part of the host to draw in target pixels. A host with neither source
    /// draws nothing.
    pub(super) fn composite_morph_group(
        &mut self,
        backend: &B,
        frame: &MorphFrame<B>,
        view: &B::TextureView,
        resolve_target: Option<&B::TextureView>,
        parent_origin: (u32, u32),
        size: (u32, u32),
        is_srgb: bool,
        device_scale: f32,
        scissor: DamageRect,
    ) {
        let rect = |source: &Option<KeptSource<B>>| {
            source.as_ref().map(|source| {
                [
                    (source.bounds.x - parent_origin.0) as f32,
                    (source.bounds.y - parent_origin.1) as f32,
                    source.bounds.width as f32,
                    source.bounds.height as f32,
                ]
            })
        };
        let (from_view, to_view) = match (&frame.from, &frame.to) {
            (Some(from), Some(to)) => (&from.sample_view, &to.sample_view),
            (Some(only), None) | (None, Some(only)) => (&only.sample_view, &only.sample_view),
            (None, None) => return,
        };
        let instance = MorphInstance::new(
            &frame.params,
            frame.opacity,
            device_scale,
            rect(&frame.from),
            rect(&frame.to),
        );

        let (format, antialiasing) = (self.format, self.antialiasing);
        let pipeline = self
            .morph_pipeline
            .get_or_insert_with(|| MorphPipeline::new(backend, format, antialiasing));
        let bind_group = pipeline.create_bind_group(backend, from_view, to_view);
        pipeline.begin_frame(backend, size.0, size.1, is_srgb);

        let mut encoder = backend.create_command_encoder("cupid composite morph group");
        let attachments = [RenderPassColorAttachment {
            view,
            resolve_target,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        }];
        {
            use crate::backend::GpuRenderPass;
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("cupid morph group composite pass".to_string()),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                },
            );
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
            pipeline.draw(&mut pass, &bind_group, instance);
        }
        pipeline.end_frame(backend);
        backend.submit(encoder);
    }

    /// Marks the targets a finished host read as reusable.
    pub(super) fn release_morph_sources(&mut self, frame: &MorphFrame<B>) {
        for source in [&frame.from, &frame.to].into_iter().flatten() {
            self.opacity_group_targets[source.depth].target.mark_valid();
        }
    }
}
