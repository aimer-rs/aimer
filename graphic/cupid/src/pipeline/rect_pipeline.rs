use bytemuck::{Pod, Zeroable};

use super::frame_upload::FrameUpload;
use super::image_pipeline::InstanceBufferPolicy;
use crate::utilities::Rgba8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct RectInstance {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub color: Rgba8,
    /// Per-corner border radius: [top-left, top-right, bottom-right,
    /// bottom-left]
    pub border_radius: [f32; 4],
    /// Per-side border width: [top, right, bottom, left]
    pub border_width: [f32; 4],
    pub border_color: Rgba8,
    /// Per-side outline width: [top, right, bottom, left]
    pub outline_width: [f32; 4],
    pub outline_color: Rgba8,
    /// Clip rect: [x, y, width, height]. If width < 0, no clip is applied.
    /// A non-negative width of ~0 fully clips (shader returns alpha 0).
    pub clip_rect: [f32; 4],
    /// Border radius for the clip rect: [top-left, top-right, bottom-right,
    /// bottom-left].
    pub clip_border_radius: [f32; 4],
    /// Shadow parameters: [offset_x, offset_y, blur, spread]
    pub shadow_params: [f32; 4],
    /// Shadow color in straight-alpha RGBA bytes.
    pub shadow_color: Rgba8,
    /// Shadow flags: [inset (0.0 or 1.0), 0, 0, 0]
    pub shadow_flags: [f32; 4],
}

impl RectInstance {
    #[cfg(feature = "wgpu")]
    const ATTRIBS: [wgpu::VertexAttribute; 13] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Unorm8x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Unorm8x4,
        6 => Float32x4,
        7 => Unorm8x4,
        8 => Float32x4,
        9 => Float32x4,
        10 => Float32x4,
        11 => Unorm8x4,
        12 => Float32x4,
    ];

    #[cfg(feature = "wgpu")]
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<RectInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 13] = [
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, position) as u64,
            shader_location: 0,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, size) as u64,
            shader_location: 1,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, color) as u64,
            shader_location: 2,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, border_radius) as u64,
            shader_location: 3,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, border_width) as u64,
            shader_location: 4,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, border_color) as u64,
            shader_location: 5,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, outline_width) as u64,
            shader_location: 6,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, outline_color) as u64,
            shader_location: 7,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_rect) as u64,
            shader_location: 8,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_border_radius) as u64,
            shader_location: 9,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, shadow_params) as u64,
            shader_location: 10,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, shadow_color) as u64,
            shader_location: 11,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, shadow_flags) as u64,
            shader_location: 12,
        },
    ];
}

pub struct RectPipeline<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    pipeline: B::RenderPipeline,
    clear_pipeline: B::RenderPipeline,
    viewport_buffer: B::Buffer,
    viewport_bind_group: B::BindGroup,
    instance_buffer: B::Buffer,
    instance_policy: InstanceBufferPolicy,
    instances: Vec<RectInstance>,
    frame_instance_offset: usize,
    upload: FrameUpload<RectInstance>,
    immediate_uploads: bool,
    last_viewport: Option<(u32, u32, bool)>,
}

// ── Backend-agnostic generic path ──────────────────────────────────────────
//
// Pipeline resources and draw operations use the selected backend's associated types.
impl<B: crate::backend::GpuBackend> RectPipeline<B> {
    const INITIAL_CAPACITY: usize = 256;

    #[inline]
    pub fn push(&mut self, instance: RectInstance) {
        self.instances.push(instance);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.instances.clear();
        self.frame_instance_offset = 0;
        self.immediate_uploads = false;
    }

    #[inline]
    pub fn instance_buffer_bytes(&self) -> u64 {
        (self.instance_policy.capacity() * size_of::<RectInstance>()) as u64
    }

    /// Creates a rect pipeline through the selected backend.
    pub fn new(backend: &B, format: B::TextureFormat, antialiasing: crate::AntiAlias) -> Self {
        use crate::backend::*;

        let shader = backend.create_shader_module(backend.rect_shader_source(), "rect shader");

        let viewport_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("rect viewport uniform".to_string()),
            size: 16,
            usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
        });

        let bind_group_layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
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
            &bind_group_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&viewport_buffer),
            }],
        );

        let pipeline_layout = backend.create_pipeline_layout(&[&bind_group_layout]);

        let vertex_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<RectInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &RectInstance::GENERIC_ATTRIBUTES,
        })];

        let pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("rect pipeline".to_string()),
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

        let clear_pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("rect clear pipeline".to_string()),
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
                    blend: None,
                    write_mask: ColorWriteMask::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: crate::pipeline::multisample_state(antialiasing),
        });

        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("rect instance buffer".to_string()),
            size: (Self::INITIAL_CAPACITY * size_of::<RectInstance>()) as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        Self {
            pipeline,
            clear_pipeline,
            viewport_buffer,
            viewport_bind_group,
            instance_buffer,
            instance_policy: InstanceBufferPolicy::new(Self::INITIAL_CAPACITY),
            instances: Vec::new(),
            frame_instance_offset: 0,
            upload: FrameUpload::new(),
            immediate_uploads: false,
            last_viewport: None,
        }
    }

    /// Starts a frame using the selected backend.
    pub fn begin_frame(
        &mut self,
        backend: &B,
        total_rects: usize,
        width: u32,
        height: u32,
        is_srgb: bool,
    ) {
        let previous_capacity = self.instance_policy.capacity();
        self.instance_policy
            .record_usage(self.frame_instance_offset);
        self.instance_policy.grow_to_fit(total_rects);
        if self.instance_policy.capacity() != previous_capacity {
            self.instance_buffer = backend.create_buffer(&crate::backend::BufferDescriptor {
                label: Some("rect instance buffer (resized)".to_string()),
                size: (self.instance_policy.capacity() * size_of::<RectInstance>()) as u64,
                usage: vec![
                    crate::backend::BufferUsage::Vertex,
                    crate::backend::BufferUsage::CopyDst,
                ],
            });
            self.upload.invalidate();
        }

        // Match concrete begin_frame: only size the buffer + write viewport.
        // Queued backends upload instances in end_frame; immediate
        // backends upload each pending range in flush before drawing it.
        #[cfg(target_os = "android")]
        let is_srgb_f32 = 2.0_f32;
        #[cfg(not(target_os = "android"))]
        let is_srgb_f32 = if is_srgb { 1.0_f32 } else { 0.0 };
        if self.last_viewport != Some((width, height, is_srgb)) {
            self.last_viewport = Some((width, height, is_srgb));
            backend.write_buffer(
                &self.viewport_buffer,
                0,
                bytemuck::cast_slice(&[width as f32, height as f32, is_srgb_f32, 0.0]),
            );
        }

        self.clear();
    }

    /// Uploads pending instance data through the selected backend.
    pub fn end_frame(&mut self, backend: &B) {
        if self.immediate_uploads {
            self.upload.mark_uploaded(&self.instances);
        } else {
            self.upload
                .upload(backend, &self.instance_buffer, &self.instances);
        }
    }

    /// Records the pending rectangle batch through the backend render pass.
    pub fn flush<'a>(&mut self, pass: &mut B::RenderPass<'a>)
    where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        let pending = self.instances.len() - self.frame_instance_offset;
        if pending == 0 {
            return;
        }
        debug_assert!(self.instances.len() <= self.instance_policy.capacity());
        let byte_offset = (self.frame_instance_offset * size_of::<RectInstance>()) as u64;
        self.immediate_uploads |= pass.write_buffer_before_draw(
            &self.instance_buffer,
            byte_offset,
            bytemuck::cast_slice(&self.instances[self.frame_instance_offset..]),
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.viewport_bind_group, &[]);
        pass.set_vertex_buffer(0, &self.instance_buffer, byte_offset);
        pass.draw(0..6, 0..pending as u32);
        self.frame_instance_offset = self.instances.len();
    }

    /// Clears the pending damaged rectangle using replacement blending.
    pub fn flush_clear<'a>(&mut self, pass: &mut B::RenderPass<'a>)
    where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        let pending = self.instances.len() - self.frame_instance_offset;
        if pending == 0 {
            return;
        }
        debug_assert!(self.instances.len() <= self.instance_policy.capacity());
        let byte_offset = (self.frame_instance_offset * size_of::<RectInstance>()) as u64;
        self.immediate_uploads |= pass.write_buffer_before_draw(
            &self.instance_buffer,
            byte_offset,
            bytemuck::cast_slice(&self.instances[self.frame_instance_offset..]),
        );
        pass.set_pipeline(&self.clear_pipeline);
        pass.set_bind_group(0, &self.viewport_bind_group, &[]);
        pass.set_vertex_buffer(0, &self.instance_buffer, byte_offset);
        pass.draw(0..6, 0..pending as u32);
        self.frame_instance_offset = self.instances.len();
    }
}

/// Helper: convert a wgpu vertex format to the backend-agnostic equivalent.
#[cfg(all(test, feature = "wgpu"))]
mod tests {
    use super::RectInstance;

    #[test]
    fn rect_instance_uses_unorm_bytes_for_each_paint_color() {
        assert_eq!(std::mem::size_of::<RectInstance>(), 144);
        assert_eq!(
            RectInstance::ATTRIBS[2].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            RectInstance::ATTRIBS[5].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            RectInstance::ATTRIBS[7].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            RectInstance::ATTRIBS[11].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            RectInstance::ATTRIBS[2].offset,
            std::mem::offset_of!(RectInstance, color) as u64
        );
        assert_eq!(
            RectInstance::ATTRIBS[5].offset,
            std::mem::offset_of!(RectInstance, border_color) as u64
        );
        assert_eq!(
            RectInstance::ATTRIBS[7].offset,
            std::mem::offset_of!(RectInstance, outline_color) as u64
        );
        assert_eq!(
            RectInstance::ATTRIBS[11].offset,
            std::mem::offset_of!(RectInstance, shadow_color) as u64
        );
    }
}
