/// Copies a shader-readable frame texture into the presentation target.
///
/// Material frames use this final pass when the presentation surface cannot be
/// copied. The pass is deliberately sample-free and uniform-free: one texture
/// load per output pixel keeps the compatibility path predictable and cheap.
pub(crate) struct FrameCompositePipeline<
    B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend,
> {
    pipeline: B::RenderPipeline,
    bind_group_layout: B::BindGroupLayout,
}

// ── Backend-agnostic generic path ──────────────────────────────────────────
//
// Pipeline resources and draw operations use the selected backend's associated types.
impl<B: crate::backend::GpuBackend> FrameCompositePipeline<B> {
    /// Creates the frame composite pipeline using the selected backend.
    pub(crate) fn new(backend: &B, format: B::TextureFormat) -> Self {
        use crate::backend::*;

        let shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::FrameComposite),
            "frame composite shader",
        );

        let bind_group_layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
            binding: 0,
            visibility: vec![ShaderStage::Fragment],
            ty: BindingType::Texture {
                multisampled: false,
                view_dimension: TextureViewDimension::D2,
                sample_type: TextureSampleType::Float { filterable: false },
            },
            count: None,
        }]);

        let pipeline_layout = backend.create_pipeline_layout(&[&bind_group_layout]);

        let pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("frame composite pipeline".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
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
            multisample: MultisampleState::default(),
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }

    pub(crate) fn create_bind_group(&self, backend: &B, source: &B::TextureView) -> B::BindGroup {
        backend.create_bind_group(
            &self.bind_group_layout,
            &[crate::backend::BindGroupEntry {
                binding: 0,
                resource: crate::backend::BindingResource::TextureView(source),
            }],
        )
    }

    pub(crate) fn render<'a>(&'a self, pass: &mut B::RenderPass<'a>, bind_group: &'a B::BindGroup)
    where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_uses_one_direct_texture_load() {
        let shader = include_str!("shaders/wgsl/frame_composite.wgsl");
        assert_eq!(shader.matches("textureLoad").count(), 1);
        assert!(!shader.contains("textureSample"));
    }
}
