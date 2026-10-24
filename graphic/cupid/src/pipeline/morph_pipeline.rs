//! Composites two offscreen pictures with a transition effect.
//!
//! The renderer draws the two sources of a morph group into group targets and
//! calls [`MorphPipeline::draw`] once, over the union of their rectangles. One
//! draw reads both targets, so the effect can look at a pixel in the outgoing
//! and the incoming picture at once, which blending them separately cannot.

use bytemuck::{Pod, Zeroable};

use crate::draw_cmd_v2::{MorphParams, MorphStyle};

/// One morph composite, as the shader reads it.
///
/// Rectangles are `x, y, width, height` in pixels of the target being drawn
/// into; a rectangle with no area marks an absent source. The layout is shared
/// with `morph.wgsl` and its Metal, HLSL, GLSL and SPIR-V siblings, so every
/// field is a `vec4`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub(crate) struct MorphInstance {
    pub(crate) rect_from: [f32; 4],
    pub(crate) rect_to: [f32; 4],
    /// Style, progress, opacity, seed.
    pub(crate) timing: [f32; 4],
    /// Direction x, direction y, softness, amplitude.
    pub(crate) shape: [f32; 4],
    /// Frequency, blur radius in target pixels, then two unused lanes.
    pub(crate) noise: [f32; 4],
}

impl MorphInstance {
    /// Builds the instance for `params`, drawn at `opacity` into a target whose
    /// pixels are `device_scale` times the logical ones the blur is given in.
    pub(crate) fn new(
        params: &MorphParams,
        opacity: f32,
        device_scale: f32,
        rect_from: Option<[f32; 4]>,
        rect_to: Option<[f32; 4]>,
    ) -> Self {
        let style = match params.style {
            MorphStyle::Dissolve => 0.0,
            MorphStyle::Wipe => 1.0,
            MorphStyle::Liquid => 2.0,
            MorphStyle::Bounds => 3.0,
            MorphStyle::Blur => 4.0,
            MorphStyle::Slide => 5.0,
            MorphStyle::Goo => 6.0,
        };
        Self {
            rect_from: rect_from.unwrap_or([0.0; 4]),
            rect_to: rect_to.unwrap_or([0.0; 4]),
            timing: [style, params.progress, opacity, params.seed],
            shape: [
                params.direction[0],
                params.direction[1],
                params.softness,
                params.amplitude,
            ],
            noise: [params.frequency, params.blur * device_scale, 0.0, 0.0],
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 5] = [
        Self::attribute(std::mem::offset_of!(Self, rect_from), 0),
        Self::attribute(std::mem::offset_of!(Self, rect_to), 1),
        Self::attribute(std::mem::offset_of!(Self, timing), 2),
        Self::attribute(std::mem::offset_of!(Self, shape), 3),
        Self::attribute(std::mem::offset_of!(Self, noise), 4),
    ];

    const fn attribute(offset: usize, shader_location: u32) -> crate::backend::VertexAttribute {
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: offset as u64,
            shader_location,
        }
    }
}

/// The pipeline that blends a morph's two source targets.
///
/// It has no per-frame instance batching: a morph is composited with its own
/// submission, like an opacity group, and there is only ever one instance.
pub(crate) struct MorphPipeline<B: crate::backend::GpuBackend> {
    pipeline: B::RenderPipeline,
    viewport_buffer: B::Buffer,
    viewport_bind_group: B::BindGroup,
    texture_bind_group_layout: B::BindGroupLayout,
    sampler: B::Sampler,
    instance_buffer: B::Buffer,
    instance: MorphInstance,
    /// Whether the pass wrote the instance itself, so `end_frame` need not.
    instance_written_in_pass: bool,
    last_viewport: Option<(u32, u32, bool)>,
}

impl<B: crate::backend::GpuBackend> MorphPipeline<B> {
    /// Creates the pipeline for targets of `format`.
    pub(crate) fn new(
        backend: &B,
        format: B::TextureFormat,
        antialiasing: crate::AntiAlias,
    ) -> Self {
        use crate::backend::*;

        let shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::Morph),
            "morph shader",
        );

        let viewport_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("morph viewport uniform".to_string()),
            size: 16,
            usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
        });
        let viewport_layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
            binding: 0,
            visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]);
        let viewport_bind_group = backend.create_bind_group(
            &viewport_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&viewport_buffer),
            }],
        );

        let texture_entry = |binding| BindGroupLayoutEntry {
            binding,
            visibility: vec![ShaderStage::Fragment],
            ty: BindingType::Texture {
                multisampled: false,
                view_dimension: TextureViewDimension::D2,
                sample_type: TextureSampleType::Float { filterable: true },
            },
            count: None,
        };
        let texture_bind_group_layout = backend.create_bind_group_layout(&[
            texture_entry(0),
            texture_entry(1),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ]);
        let pipeline_layout =
            backend.create_pipeline_layout(&[&viewport_layout, &texture_bind_group_layout]);

        let vertex_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<MorphInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &MorphInstance::GENERIC_ATTRIBUTES,
        })];
        let pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("morph pipeline".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &vertex_buffers,
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWriteMask::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: crate::pipeline::multisample_state(antialiasing),
        });

        // Group targets are sampled at fractional positions as the morph moves
        // and warps them, and the edge of a picture must not bleed in.
        let sampler = backend.create_sampler(&SamplerDescriptor {
            label: Some("morph sampler".to_string()),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            max_anisotropy: 1,
        });

        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("morph instance buffer".to_string()),
            size: size_of::<MorphInstance>() as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        Self {
            pipeline,
            viewport_buffer,
            viewport_bind_group,
            texture_bind_group_layout,
            sampler,
            instance_buffer,
            instance: MorphInstance::zeroed(),
            instance_written_in_pass: false,
            last_viewport: None,
        }
    }

    /// Binds the two source targets. `from` and `to` may be the same view when
    /// one source is absent: its rectangle then has no area and is not read.
    pub(crate) fn create_bind_group(
        &self,
        backend: &B,
        from: &B::TextureView,
        to: &B::TextureView,
    ) -> B::BindGroup {
        use crate::backend::*;
        backend.create_bind_group(
            &self.texture_bind_group_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(from),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(to),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        )
    }

    /// Sets the size of the target the next draw lands in.
    pub(crate) fn begin_frame(&mut self, backend: &B, width: u32, height: u32, is_srgb: bool) {
        // The shader only reads the size; the sRGB lane keeps the layout shared
        // with the image pipeline's viewport.
        if self.last_viewport != Some((width, height, is_srgb)) {
            self.last_viewport = Some((width, height, is_srgb));
            backend.write_buffer(
                &self.viewport_buffer,
                0,
                bytemuck::cast_slice(&[width as f32, height as f32, f32::from(is_srgb), 0.0]),
            );
        }
        self.instance_written_in_pass = false;
    }

    /// Draws `instance` into the pass.
    pub(crate) fn draw<'a>(
        &mut self,
        pass: &mut B::RenderPass<'a>,
        bind_group: &B::BindGroup,
        instance: MorphInstance,
    ) where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        self.instance = instance;
        self.instance_written_in_pass =
            pass.write_buffer_before_draw(&self.instance_buffer, 0, bytemuck::bytes_of(&instance));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.viewport_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_vertex_buffer(0, &self.instance_buffer, 0);
        pass.draw(0..6, 0..1);
    }

    /// Uploads the instance for backends that cannot write inside the pass.
    pub(crate) fn end_frame(&mut self, backend: &B) {
        if !self.instance_written_in_pass {
            backend.write_buffer(&self.instance_buffer, 0, bytemuck::bytes_of(&self.instance));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_is_five_vec4_lanes() {
        assert_eq!(size_of::<MorphInstance>(), 5 * 16);
        for (index, attribute) in MorphInstance::GENERIC_ATTRIBUTES.iter().enumerate() {
            assert_eq!(attribute.offset, index as u64 * 16);
            assert_eq!(attribute.shader_location, index as u32);
        }
    }

    #[test]
    fn instance_carries_the_params_in_shader_order() {
        let mut params = MorphParams::new(MorphStyle::Liquid, 0.25);
        params.direction = [0.0, 1.0];
        params.softness = 0.5;
        params.amplitude = 0.1;
        params.frequency = 4.0;
        params.seed = 2.0;
        params.blur = 6.0;

        let instance = MorphInstance::new(&params, 0.5, 2.0, Some([1.0, 2.0, 3.0, 4.0]), None);

        assert_eq!(instance.rect_from, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(instance.rect_to, [0.0; 4], "an absent source has no area");
        assert_eq!(instance.timing, [2.0, 0.25, 0.5, 2.0]);
        assert_eq!(instance.shape, [0.0, 1.0, 0.5, 0.1]);
        assert_eq!(instance.noise[0], 4.0);
        assert_eq!(instance.noise[1], 12.0, "the blur is given in logical pixels");
    }

    #[test]
    fn every_shader_reads_the_same_interface() {
        let wgsl = include_str!("shaders/wgsl/morph.wgsl");
        let metal = include_str!("shaders/metal/morph.metal");
        let hlsl = include_str!("shaders/dx12/morph.hlsl");
        for name in ["rect_from", "rect_to", "timing", "shape", "noise"] {
            for (language, source) in [("wgsl", wgsl), ("metal", metal), ("hlsl", hlsl)] {
                assert!(source.contains(name), "{language} shader lacks `{name}`");
            }
        }
        // The four styles are numbered the same way everywhere.
        for source in [wgsl, metal, hlsl] {
            for branch in ["style == 0u", "style == 1u", "style == 2u", "style == 4u", "style == 5u", "style == 6u"] {
                assert!(source.contains(branch));
            }
        }
    }
}
