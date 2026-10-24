use std::any::Any;

use crate::backend::GpuBackend;

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
pub(crate) struct CustomPipelineSlotGeneric<B: GpuBackend> {
    pub pipeline: Box<dyn CustomPipelineGeneric<B>>,
}

impl<B: GpuBackend> CustomPipelineSlotGeneric<B> {
    pub fn new(pipeline: impl CustomPipelineGeneric<B>) -> Self {
        let pipeline = Box::new(pipeline);
        Self { pipeline }
    }
}
