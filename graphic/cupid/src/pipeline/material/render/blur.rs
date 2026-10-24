//! Reused Gaussian intermediates for plain backdrop filters.
//!
//! A narrow blur is one horizontal pass into the intermediate and the material's
//! own pass finishes it vertically. A wide one (see `downsample`) works on a
//! texture `factor` times smaller in each direction: a pass averages each block
//! of the capture into one texel, a horizontal and a vertical Gaussian run over
//! the small texture, and the material's pass reads it back with bilinear
//! filtering.

use super::*;
use crate::backend::*;

/// The passes a blur may need, each with a uniform of its own per request.
const LANES: usize = 3;
/// `effect.x` of the passes, negative so no material kind can be mistaken for one.
const HORIZONTAL_FULL: f32 = -1.0;
const DECIMATE: f32 = -2.0;
const HORIZONTAL_REDUCED: f32 = -3.0;
const VERTICAL_REDUCED: f32 = -4.0;

/// The second small texture a reduced blur ping-pongs through, with the bind
/// groups that read each of the two.
struct Scratch<B: GpuBackend> {
    _texture: B::Texture,
    view: B::TextureView,
    reads_first: B::BindGroup,
    reads_second: B::BindGroup,
}

pub(super) struct BackdropBlur<B: GpuBackend> {
    pipeline: B::RenderPipeline,
    _texture: B::Texture,
    pub(super) view: B::TextureView,
    bind_group: B::BindGroup,
    scratch: Option<Scratch<B>>,
    uniform_buffer: B::Buffer,
    stride: u64,
    capacity: usize,
    width: u32,
    height: u32,
    format: B::TextureFormat,
    upload: Vec<u8>,
    /// How much each request's capture is reduced; `1` is the full-resolution path.
    factors: Vec<u32>,
}

impl<B: GpuBackend> BackdropBlur<B> {
    pub(super) fn new(parent: &MaterialPipeline<B>, backend: &B) -> Self {
        let stride = parent.uniform_stride;
        let capacity = INITIAL_REQUEST_CAPACITY;
        let uniform_buffer = create_uniform_buffer_generic(backend, stride, capacity * LANES);
        let texture = create_texture(
            backend,
            parent.backdrop_format,
            parent.backdrop_width,
            parent.backdrop_height,
        );
        let view = backend.create_texture_view(&texture, "backdrop horizontal blur view");
        let bind_group = create_bind_group(
            backend,
            &parent.bind_group_layout,
            &uniform_buffer,
            stride,
            &parent._backdrop_view,
            &parent.backdrop_sampler,
            &parent._backdrop_view,
        );
        Self {
            pipeline: create_material_pipeline(
                backend,
                &parent.bind_group_layout,
                parent.backdrop_format,
                crate::AntiAlias::Analytic,
                true,
            ),
            _texture: texture,
            view,
            bind_group,
            scratch: None,
            uniform_buffer,
            stride,
            capacity,
            width: parent.backdrop_width,
            height: parent.backdrop_height,
            format: parent.backdrop_format,
            upload: Vec::new(),
            factors: Vec::new(),
        }
    }

    pub(super) fn prepare(
        &mut self,
        parent: &MaterialPipeline<B>,
        backend: &B,
        capture_changed: bool,
    ) -> bool {
        let texture_changed = self.width != parent.backdrop_width
            || self.height != parent.backdrop_height
            || self.format != parent.backdrop_format;
        if texture_changed {
            if self.format != parent.backdrop_format {
                self.pipeline = create_material_pipeline(
                    backend,
                    &parent.bind_group_layout,
                    parent.backdrop_format,
                    crate::AntiAlias::Analytic,
                    true,
                );
            }
            self._texture = create_texture(
                backend,
                parent.backdrop_format,
                parent.backdrop_width,
                parent.backdrop_height,
            );
            self.view =
                backend.create_texture_view(&self._texture, "backdrop horizontal blur view");
            self.width = parent.backdrop_width;
            self.height = parent.backdrop_height;
            self.format = parent.backdrop_format;
            self.scratch = None;
        }
        let buffer_changed = parent.requests.len() > self.capacity;
        if buffer_changed {
            self.capacity = parent
                .requests
                .len()
                .next_power_of_two()
                .min(MAX_REQUESTS_PER_FRAME);
            self.uniform_buffer =
                create_uniform_buffer_generic(backend, self.stride, self.capacity * LANES);
        }
        let needs_scratch = parent
            .requests
            .iter()
            .zip(&parent.backdrop_regions)
            .any(|(request, region)| {
                region.is_some() && request_blur_factor(request) > 1
            });
        let rebind = capture_changed || buffer_changed || texture_changed;
        let scratch_changed = needs_scratch && (self.scratch.is_none() || rebind);
        if rebind {
            self.bind_group = create_bind_group(
                backend,
                &parent.bind_group_layout,
                &self.uniform_buffer,
                self.stride,
                &parent._backdrop_view,
                &parent.backdrop_sampler,
                &parent._backdrop_view,
            );
        }
        if scratch_changed {
            self.scratch = Some(self.create_scratch(parent, backend));
        }
        let stride = self.stride as usize;
        let lane = stride * self.capacity;
        self.upload.clear();
        self.upload.resize(lane * LANES, 0);
        self.factors.clear();
        self.factors.resize(parent.requests.len(), 1);
        for (index, request) in parent.requests.iter().copied().enumerate() {
            let Some(region) = parent.backdrop_regions[index] else {
                continue;
            };
            if request.kind != MaterialKind::BackdropFilter || request.blur_radius <= 0.0 {
                continue;
            }
            // The intermediate uses capture-local device coordinates. The final
            // pass keeps the original frame coordinates and clipping.
            let mut uniform = MaterialUniform::from_request(
                request,
                self.width,
                self.height,
                false,
                Some(region),
            );
            uniform.backdrop_rect[0] = 0.0;
            uniform.backdrop_rect[1] = 0.0;
            let factor = uniform.liquid[0].max(1.0) as u32;
            self.factors[index] = factor;
            if factor <= 1 {
                uniform.bounds = [0.0, 0.0, region.width as f32, region.height as f32];
                // Negative kind is private to the blur passes, never a wire material.
                uniform.effect[0] = HORIZONTAL_FULL;
                self.write_uniform(0, index, &uniform);
                continue;
            }
            // A large blur is made on a texture `factor` times smaller: the first
            // pass reads the capture, the other two read the small texture.
            let (reduced_width, reduced_height) =
                reduced_extent(region.width, region.height, factor);
            uniform.bounds = [0.0, 0.0, reduced_width as f32, reduced_height as f32];
            uniform.effect[0] = DECIMATE;
            self.write_uniform(0, index, &uniform);
            uniform.backdrop_rect[2] = reduced_width as f32;
            uniform.backdrop_rect[3] = reduced_height as f32;
            uniform.effect[0] = HORIZONTAL_REDUCED;
            self.write_uniform(1, index, &uniform);
            uniform.effect[0] = VERTICAL_REDUCED;
            self.write_uniform(2, index, &uniform);
        }
        backend.write_buffer(&self.uniform_buffer, 0, &self.upload);
        texture_changed || scratch_changed
    }

    fn write_uniform(&mut self, lane: usize, index: usize, uniform: &MaterialUniform) {
        let start = (lane * self.capacity + index) * self.stride as usize;
        self.upload[start..start + size_of::<MaterialUniform>()]
            .copy_from_slice(bytemuck::bytes_of(uniform));
    }

    fn create_scratch(&self, parent: &MaterialPipeline<B>, backend: &B) -> Scratch<B> {
        let texture = create_texture(backend, self.format, self.width, self.height);
        let view = backend.create_texture_view(&texture, "backdrop reduced blur view");
        let reading = |source: &B::TextureView| {
            create_bind_group(
                backend,
                &parent.bind_group_layout,
                &self.uniform_buffer,
                self.stride,
                &parent._backdrop_view,
                &parent.backdrop_sampler,
                source,
            )
        };
        Scratch {
            reads_first: reading(&self.view),
            reads_second: reading(&view),
            _texture: texture,
            view,
        }
    }

    pub(super) fn encode(&self, index: usize, backend: &B, encoder: &mut B::CommandEncoder) {
        let offset = |lane: usize| ((lane * self.capacity + index) as u64 * self.stride) as u32;
        let factor = self.factors.get(index).copied().unwrap_or(1);
        let Some(scratch) = self.scratch.as_ref().filter(|_| factor > 1) else {
            self.pass(backend, encoder, &self.view, &self.bind_group, offset(0));
            return;
        };
        // Capture -> small, small -> scratch (horizontal), scratch -> small
        // (vertical). The material's pass reads the small texture.
        self.pass(backend, encoder, &self.view, &self.bind_group, offset(0));
        self.pass(backend, encoder, &scratch.view, &scratch.reads_first, offset(1));
        self.pass(backend, encoder, &self.view, &scratch.reads_second, offset(2));
    }

    fn pass(
        &self,
        backend: &B,
        encoder: &mut B::CommandEncoder,
        target: &B::TextureView,
        bind_group: &B::BindGroup,
        offset: u32,
    ) {
        let mut pass = backend.begin_render_pass(
            encoder,
            &RenderPassDescriptor {
                label: None,
                color_attachments: &[RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear([0.0; 4]),
                        store: StoreOp::Store,
                    },
                }],
                depth_stencil_attachment: None,
            },
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[offset]);
        pass.draw(0..6, 0..1);
    }
}

/// How much `request`'s blur is reduced, `1` for none or for anything that is
/// not a blurring backdrop filter.
fn request_blur_factor(request: &MaterialRequest) -> u32 {
    if request.kind != MaterialKind::BackdropFilter || request.blur_radius <= 0.0 {
        return 1;
    }
    blur_downsample(blur_radius_pixels(&request.normalized()))
}

fn create_texture<B: GpuBackend>(
    backend: &B,
    format: B::TextureFormat,
    width: u32,
    height: u32,
) -> B::Texture {
    backend.create_texture(&TextureDescriptor {
        label: Some("backdrop horizontal blur".to_string()),
        size: (width, height, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::TextureBinding, TextureUsage::RenderAttachment],
    })
}

#[cfg(all(test, feature = "wgpu"))]
mod tests {
    use super::*;
    use crate::custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
    use aimer_utils::SyncFuture;

    #[test]
    fn steady_blur_frames_reuse_gpu_resources_and_upload_storage() {
        let instance = ::wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&Default::default())
            .block()
            .expect("GPU adapter for blur resource validation");
        let (device, queue) = adapter
            .request_device(&Default::default())
            .block()
            .expect("GPU device for blur resource validation");
        let backend = crate::WgpuBackend::new(device, queue);
        let format = ::wgpu::TextureFormat::Rgba8Unorm;
        let source = create_texture(&backend, format, 640, 400);
        let ctx = RenderContextGeneric {
            backend: &backend,
            width: 640,
            height: 400,
            is_srgb: false,
            format,
            sample_count: 1,
            source_texture: Some(&source),
        };
        let mut parent = MaterialPipeline::new(&backend, format, crate::AntiAlias::Analytic);
        let request =
            MaterialRequest::new(MaterialKind::BackdropFilter, [160.0, 92.0, 320.0, 208.0]);
        parent.prepare_command(&request);
        parent.prepare(&ctx);
        let blur = parent.blur.as_ref().unwrap();
        let original = (
            blur._texture.clone(),
            blur.uniform_buffer.clone(),
            blur.bind_group.clone(),
            blur.upload.as_ptr(),
            parent.bind_group.clone(),
            parent.uniform_buffer.clone(),
        );
        for _ in 0..3 {
            parent.begin_frame();
            parent.prepare_command(&request);
            parent.prepare(&ctx);
            let blur = parent.blur.as_ref().unwrap();
            assert_eq!(blur._texture, original.0);
            assert_eq!(blur.uniform_buffer, original.1);
            assert_eq!(blur.bind_group, original.2);
            assert_eq!(blur.upload.as_ptr(), original.3);
            assert_eq!(parent.bind_group, original.4);
            assert_eq!(parent.uniform_buffer, original.5);
        }
    }
}
