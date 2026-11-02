use std::marker::PhantomData;
use std::rc::Rc;

use web_sys::{
    GpuBindGroup, GpuBindGroupLayout, GpuBuffer, GpuCommandEncoder, GpuIndexFormat,
    GpuPipelineLayout, GpuRenderPassEncoder, GpuRenderPipeline, GpuSampler, GpuShaderModule,
    GpuTexture, GpuTextureView,
};

use crate::backend::{GpuRenderPass, IndexFormat};

use super::WebGpuShared;

pub struct WebGpuBuffer {
    pub(super) raw: GpuBuffer,
    pub(super) size: u64,
}

#[derive(Clone)]
pub struct WebGpuTexture(pub(super) Rc<WebGpuTextureInner>);

pub(super) struct WebGpuTextureInner {
    pub(super) raw: GpuTexture,
}

#[derive(Clone)]
pub struct WebGpuTextureView(pub(super) Rc<WebGpuTextureViewInner>);

pub(super) struct WebGpuTextureViewInner {
    pub(super) target: WebGpuViewTarget,
}

pub(super) enum WebGpuViewTarget {
    Surface(Rc<WebGpuShared>),
    Texture {
        raw: GpuTextureView,
        _texture: WebGpuTexture,
    },
}

impl WebGpuTextureView {
    pub(super) fn surface(shared: Rc<WebGpuShared>) -> Self {
        Self(Rc::new(WebGpuTextureViewInner {
            target: WebGpuViewTarget::Surface(shared),
        }))
    }

    pub(super) fn texture(texture: WebGpuTexture, raw: GpuTextureView) -> Self {
        Self(Rc::new(WebGpuTextureViewInner {
            target: WebGpuViewTarget::Texture {
                _texture: texture,
                raw,
            },
        }))
    }
}

#[derive(Clone)]
pub struct WebGpuBindGroup(pub(super) GpuBindGroup);
pub struct WebGpuBindGroupLayout(pub(super) GpuBindGroupLayout);
pub struct WebGpuPipelineLayout(pub(super) GpuPipelineLayout);
pub struct WebGpuRenderPipeline(pub(super) GpuRenderPipeline);
pub struct WebGpuSampler(pub(super) GpuSampler);
pub struct WebGpuShaderModule(pub(super) GpuShaderModule);

pub struct WebGpuCommandEncoder {
    pub(super) raw: GpuCommandEncoder,
    pub(super) surface: Option<(Rc<WebGpuShared>, GpuTexture, GpuTextureView)>,
}

impl WebGpuCommandEncoder {
    pub(super) fn new(raw: GpuCommandEncoder) -> Self {
        Self { raw, surface: None }
    }

    pub(super) fn texture_view(&mut self, view: &WebGpuTextureView) -> GpuTextureView {
        match &view.0.target {
            WebGpuViewTarget::Texture { raw, .. } => raw.clone(),
            WebGpuViewTarget::Surface(shared) => {
                if let Some((active_shared, _, active_view)) = &self.surface {
                    assert!(
                        Rc::ptr_eq(active_shared, shared),
                        "one command encoder cannot render to multiple WebGPU canvas contexts"
                    );
                    return active_view.clone();
                }

                let texture = shared
                    .get_surface_texture()
                    .unwrap_or_else(|error| panic!("get current WebGPU canvas texture: {error:?}"));
                let raw_view = texture
                    .create_view()
                    .unwrap_or_else(|error| panic!("create WebGPU canvas texture view: {error:?}"));
                self.surface = Some((shared.clone(), texture, raw_view.clone()));
                raw_view
            }
        }
    }
}

pub struct WebGpuRenderPass<'a> {
    pub(super) raw: GpuRenderPassEncoder,
    _encoder: PhantomData<&'a mut WebGpuCommandEncoder>,
}

impl<'a> WebGpuRenderPass<'a> {
    pub(super) fn new(raw: GpuRenderPassEncoder) -> Self {
        Self {
            raw,
            _encoder: PhantomData,
        }
    }
}

impl Drop for WebGpuRenderPass<'_> {
    fn drop(&mut self) {
        self.raw.end();
    }
}

impl GpuRenderPass<super::WebGpuBackend> for WebGpuRenderPass<'_> {
    fn set_pipeline(&mut self, pipeline: &WebGpuRenderPipeline) {
        self.raw.set_pipeline(&pipeline.0);
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &WebGpuBindGroup,
        dynamic_offsets: &[u32],
    ) {
        if dynamic_offsets.is_empty() {
            self.raw.set_bind_group(index, Some(&bind_group.0));
        } else {
            self.raw
                .set_bind_group_with_u32_slice_and_u32_and_dynamic_offsets_data_length(
                    index,
                    Some(&bind_group.0),
                    dynamic_offsets,
                    0,
                    dynamic_offsets.len() as u32,
                )
                .unwrap_or_else(|error| panic!("set WebGPU bind group: {error:?}"));
        }
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &WebGpuBuffer, offset: u64) {
        self.raw
            .set_vertex_buffer_with_f64(slot, Some(&buffer.raw), offset as f64);
    }

    fn set_index_buffer(&mut self, buffer: &WebGpuBuffer, index_format: IndexFormat, offset: u64) {
        let format = match index_format {
            IndexFormat::Uint16 => GpuIndexFormat::Uint16,
            IndexFormat::Uint32 => GpuIndexFormat::Uint32,
        };
        self.raw
            .set_index_buffer_with_f64(&buffer.raw, format, offset as f64);
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.raw.set_scissor_rect(x, y, width, height);
    }

    fn draw(&mut self, vertices: std::ops::Range<u32>, instances: std::ops::Range<u32>) {
        self.raw
            .draw_with_instance_count_and_first_vertex_and_first_instance(
                vertices.end.saturating_sub(vertices.start),
                instances.end.saturating_sub(instances.start),
                vertices.start,
                instances.start,
            );
    }

    fn draw_indexed(
        &mut self,
        indices: std::ops::Range<u32>,
        base_vertex: i32,
        instances: std::ops::Range<u32>,
    ) {
        self.raw
            .draw_indexed_with_instance_count_and_first_index_and_base_vertex_and_first_instance(
                indices.end.saturating_sub(indices.start),
                instances.end.saturating_sub(instances.start),
                indices.start,
                base_vertex,
                instances.start,
            );
    }
}

impl WebGpuShared {
    pub(super) fn create_texture_view(&self, texture: &WebGpuTexture) -> GpuTextureView {
        texture
            .0
            .raw
            .create_view()
            .unwrap_or_else(|error| panic!("create WebGPU texture view: {error:?}"))
    }
}

impl WebGpuTexture {
    pub(super) fn new(raw: GpuTexture) -> Self {
        Self(Rc::new(WebGpuTextureInner { raw }))
    }
}
