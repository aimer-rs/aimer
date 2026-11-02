use std::any::Any;

use aimer_cupid_gpu::GpuBackend;

/// Backend-agnostic render context passed to stable generic custom pipelines.
pub struct RenderContextGeneric<'a, B: GpuBackend> {
    pub backend: &'a B,
    pub width: u32,
    pub height: u32,
    pub is_srgb: bool,
    pub format: B::TextureFormat,
    /// Sample count used by the renderer's color attachment. Custom render
    /// pipelines must use the same count.
    pub sample_count: u32,
    /// The frame's source texture when the renderer can snapshot the current
    /// backdrop. Custom pipelines should treat this as unavailable on targets
    /// whose presentation surface cannot be copied.
    pub source_texture: Option<&'a B::Texture>,
}

/// Where a custom draw command sits in the frame when the renderer resolves it.
///
/// A payload is authored in the local space of the element that recorded it.
/// The renderer knows the transform, clip and opacity in force at that point of
/// the draw list; the pipeline does not, so they are passed alongside the
/// payload instead of every pipeline re-deriving them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandContext {
    /// Local to target-pixel transform, column-major: `[m00, m10, m20, m01,
    /// m11, m21, m02, m12, m22]`, the last row being `0, 0, 1` for the affine
    /// transforms the renderer produces.
    pub transform: [f32; 9],
    /// Scale of the transform along x and y, for lengths such as radii.
    pub scale: (f32, f32),
    /// The active clip as `(rect [x, y, w, h], corner radii)` in target
    /// pixels, or `None` when nothing clips.
    pub clip: Option<([f32; 4], [f32; 4])>,
    /// Inherited opacity in `0.0..=1.0`.
    pub alpha: f32,
}

impl CommandContext {
    /// The context of a command at the origin, unscaled, unclipped and opaque.
    pub const IDENTITY: Self = Self {
        transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        scale: (1.0, 1.0),
        clip: None,
        alpha: 1.0,
    };

    /// Maps a local point to target pixels.
    #[inline]
    pub fn map_point(&self, x: f32, y: f32) -> (f32, f32) {
        let m = &self.transform;
        (m[0] * x + m[3] * y + m[6], m[1] * x + m[4] * y + m[7])
    }

    /// Maps a local `[x, y, width, height]` rectangle to the axis-aligned
    /// rectangle between its transformed corners.
    ///
    /// A rotation or skew is approximated by the box through the first and
    /// opposite corner, as the material pipeline does; effects are rectangles.
    pub fn map_rect(&self, rect: [f32; 4]) -> [f32; 4] {
        let (x1, y1) = self.map_point(rect[0], rect[1]);
        let (x2, y2) = self.map_point(rect[0] + rect[2], rect[1] + rect[3]);
        [x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs()]
    }
}

/// Backend-agnostic custom pipeline trait used by the generic renderer.
pub trait CustomPipelineGeneric<B: GpuBackend>: 'static {
    /// A unique name identifying this pipeline.
    fn name(&self) -> &str;

    /// Called once per frame before the render pass begins.
    fn prepare(&mut self, ctx: &RenderContextGeneric<'_, B>);

    /// Starts a new ordered draw-command frame.
    fn begin_frame(&mut self) {}

    /// Accepts the payload attached to one custom draw command.
    fn prepare_command(&mut self, _data: &(dyn Any + Send)) -> Option<usize> {
        None
    }

    /// Accepts the payload attached to one custom draw command together with
    /// where the command sits in the frame.
    ///
    /// The renderer calls this, not [`prepare_command`](Self::prepare_command).
    /// The default forwards to it, so a pipeline that works in local space and
    /// does not care about placement implements only the latter.
    fn prepare_command_in(
        &mut self,
        data: &(dyn Any + Send),
        _context: &CommandContext,
    ) -> Option<usize> {
        self.prepare_command(data)
    }

    /// Called during the render pass to issue draw calls via the backend.
    fn render<'pass>(&'pass self, pass: &mut B::RenderPass<'pass>);

    /// Renders one prepared command at its original position in the draw list.
    fn render_command<'pass>(
        &'pass self,
        _command_index: Option<usize>,
        pass: &mut B::RenderPass<'pass>,
    ) {
        self.render(pass);
    }

    /// Whether the command needs the already-rendered backdrop copied.
    fn needs_backdrop(&self, _command_index: Option<usize>) -> bool {
        false
    }

    /// Copies the current frame into the pipeline's backdrop texture.
    fn capture_backdrop(
        &self,
        _backend: &B,
        _encoder: &mut B::CommandEncoder,
        _source_texture: &B::Texture,
        _width: u32,
        _height: u32,
    ) {
    }

    /// Copies the backdrop needed by one prepared command.
    ///
    /// `width` and `height` are the size of the target being drawn into, and
    /// `source_offset` is that target's origin inside `source_texture`. The
    /// two only differ for an offscreen target, such as an opacity group, whose
    /// draw coordinates are local to its bounds while `source_texture` is the
    /// whole frame. Regions the command prepared are target-local, so they must
    /// be shifted by `source_offset` to be read from the source.
    fn capture_backdrop_command(
        &self,
        _command_index: Option<usize>,
        backend: &B,
        encoder: &mut B::CommandEncoder,
        source_texture: &B::Texture,
        _source_offset: (u32, u32),
        width: u32,
        height: u32,
    ) {
        self.capture_backdrop(backend, encoder, source_texture, width, height);
    }

    /// Whether this pipeline has any work to do this frame.
    fn has_work(&self) -> bool {
        true
    }
}

/// Backend-agnostic wrapper that holds a generic custom pipeline instance.
pub struct CustomPipelineSlotGeneric<B: GpuBackend> {
    pub pipeline: Box<dyn CustomPipelineGeneric<B>>,
}

impl<B: GpuBackend> CustomPipelineSlotGeneric<B> {
    pub fn new(pipeline: impl CustomPipelineGeneric<B>) -> Self {
        let pipeline = Box::new(pipeline);
        Self { pipeline }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate_scale(tx: f32, ty: f32, sx: f32, sy: f32) -> CommandContext {
        CommandContext {
            transform: [sx, 0.0, 0.0, 0.0, sy, 0.0, tx, ty, 1.0],
            scale: (sx, sy),
            ..CommandContext::IDENTITY
        }
    }

    #[test]
    fn identity_leaves_a_rect_alone() {
        assert_eq!(
            CommandContext::IDENTITY.map_rect([1.0, 2.0, 30.0, 40.0]),
            [1.0, 2.0, 30.0, 40.0]
        );
    }

    #[test]
    fn a_rect_is_translated_and_scaled() {
        let ctx = translate_scale(16.0, 8.0, 2.0, 3.0);
        assert_eq!(ctx.map_rect([1.0, 1.0, 10.0, 10.0]), [18.0, 11.0, 20.0, 30.0]);
        assert_eq!(ctx.map_point(0.0, 0.0), (16.0, 8.0));
    }

    #[test]
    fn a_mirrored_transform_still_yields_a_positive_box() {
        let ctx = translate_scale(100.0, 0.0, -1.0, 1.0);
        assert_eq!(ctx.map_rect([10.0, 0.0, 20.0, 5.0]), [70.0, 0.0, 20.0, 5.0]);
    }
}
