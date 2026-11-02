use std::mem::ManuallyDrop;
use std::ops::Range;
use std::sync::Arc;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16_UINT;
use windows::core::{IUnknown, Interface};

use crate::backend::{
    GpuRenderPass, IndexFormat, LoadOp, RenderPassDescriptor, TexelCopyBufferInfo,
    TexelCopyTextureInfo, TextureAspect,
};

use super::pipeline::{BoundResource, Dx12BindGroup, Dx12RenderPipeline, RootBindingKind};
use super::resource::{DescriptorLease, Dx12Buffer, Dx12Texture};
use super::{
    BackendShared, COPY_ROW_ALIGNMENT, Dx12Backend, PendingBufferUpload, PendingTextureUpload,
    PendingUpload, RetiredBatch,
};

#[doc(hidden)]
/// Command recording state used by the Direct3D 12 backend interface.
pub struct Dx12CommandEncoder {
    pub(super) list: ID3D12GraphicsCommandList,
    allocator: ID3D12CommandAllocator,
    shared: Arc<BackendShared>,
    resources: Vec<ID3D12Resource>,
    descriptors: Vec<Arc<DescriptorLease>>,
    objects: Vec<IUnknown>,
}

#[doc(hidden)]
/// Render-pass state borrowing its active Direct3D 12 command encoder.
pub struct Dx12RenderPass<'a> {
    encoder: &'a mut Dx12CommandEncoder,
    targets: Vec<(Dx12Texture, D3D12_RESOURCE_STATES)>,
    pipeline: Option<Dx12RenderPipeline>,
    index_buffer: Option<(Dx12Buffer, IndexFormat, u64)>,
}

impl Dx12Backend {
    pub(super) fn make_command_encoder(&self, label: &str) -> Dx12CommandEncoder {
        let uploads = std::mem::take(
            &mut *self
                .shared
                .pending_uploads
                .lock()
                .expect("D3D12 upload queue mutex is not poisoned"),
        );
        self.make_command_encoder_with_uploads(label, uploads)
    }

    fn make_command_encoder_with_uploads(
        &self,
        label: &str,
        uploads: Vec<PendingUpload>,
    ) -> Dx12CommandEncoder {
        let allocator = unsafe {
            self.shared
                .device
                .CreateCommandAllocator::<ID3D12CommandAllocator>(D3D12_COMMAND_LIST_TYPE_DIRECT)
        }
        .expect("create D3D12 command allocator");
        let list = unsafe {
            self.shared
                .device
                .CreateCommandList::<_, _, ID3D12GraphicsCommandList>(
                    0,
                    D3D12_COMMAND_LIST_TYPE_DIRECT,
                    &allocator,
                    None::<&ID3D12PipelineState>,
                )
        }
        .expect("create D3D12 graphics command list");
        let _ = label;
        let mut encoder = Dx12CommandEncoder {
            list,
            allocator,
            shared: self.shared.clone(),
            resources: Vec::new(),
            descriptors: Vec::new(),
            objects: Vec::new(),
        };
        for upload in uploads {
            encode_upload(&mut encoder, upload);
        }
        encoder
    }

    pub(super) fn make_render_pass<'a>(
        &self,
        encoder: &'a mut Dx12CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Dx12RenderPass<'a> {
        assert!(
            desc.depth_stencil_attachment.is_none(),
            "D3D12 depth-stencil attachments are not implemented yet"
        );
        let mut handles = Vec::with_capacity(desc.color_attachments.len());
        let mut targets = Vec::with_capacity(desc.color_attachments.len());
        for attachment in desc.color_attachments {
            assert!(
                attachment.resolve_target.is_none(),
                "D3D12 resolve attachments are not implemented yet"
            );
            let view = attachment.view;
            let rtv = view.rtv.as_ref().expect("render target has no D3D12 RTV");
            let previous = transition_texture(
                &encoder.list,
                &view.texture,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
            handles.push(rtv.pool.cpu_handle(rtv.index));
            encoder.resources.push(view.texture.resource.clone());
            encoder.descriptors.push(rtv.clone());
            targets.push((view.texture.clone(), previous));
        }
        if !handles.is_empty() {
            unsafe {
                encoder.list.OMSetRenderTargets(
                    handles.len() as u32,
                    Some(handles.as_ptr()),
                    false,
                    None,
                );
            }
            let target = desc.color_attachments[0].view;
            let viewport = D3D12_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: target.texture.size.0 as f32,
                Height: target.texture.size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            unsafe { encoder.list.RSSetViewports(&[viewport]) };
            let scissor = RECT {
                left: 0,
                top: 0,
                right: target.texture.size.0.min(i32::MAX as u32) as i32,
                bottom: target.texture.size.1.min(i32::MAX as u32) as i32,
            };
            unsafe { encoder.list.RSSetScissorRects(&[scissor]) };
        }
        for (attachment, handle) in desc.color_attachments.iter().zip(handles.iter()) {
            if let LoadOp::Clear(color) = attachment.ops.load {
                let color = color.map(|channel| channel as f32);
                unsafe { encoder.list.ClearRenderTargetView(*handle, &color, None) };
            }
        }
        Dx12RenderPass {
            encoder,
            targets,
            pipeline: None,
            index_buffer: None,
        }
    }

    pub(super) fn encode_copy_buffer_to_texture(
        &self,
        encoder: &mut Dx12CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: crate::backend::Extent3d,
    ) {
        assert_eq!(dst.aspect, TextureAspect::All);
        let row_pitch = src
            .layout
            .bytes_per_row
            .expect("D3D12 buffer-to-texture copies need bytes_per_row");
        assert_eq!(row_pitch % COPY_ROW_ALIGNMENT, 0);
        assert!(row_pitch >= extent.width * dst.texture.format.bytes_per_pixel() as u32);
        let image_pitch = row_pitch
            .checked_mul(src.layout.rows_per_image.unwrap_or(extent.height))
            .expect("D3D12 texture copy image pitch overflowed");
        let buffer_state =
            transition_buffer(&encoder.list, src.buffer, D3D12_RESOURCE_STATE_COPY_SOURCE);
        let texture_state =
            transition_texture(&encoder.list, dst.texture, D3D12_RESOURCE_STATE_COPY_DEST);
        for layer in 0..extent.depth_or_array_layers {
            let source = placed_buffer_location(
                &src.buffer.resource,
                src.layout.offset + image_pitch as u64 * layer as u64,
                dst.texture.format.dxgi(),
                extent.width,
                extent.height,
                row_pitch,
            );
            let destination = subresource_location(
                &dst.texture.resource,
                texture_subresource(dst.texture, dst.mip_level, dst.origin.z + layer),
            );
            unsafe {
                encoder.list.CopyTextureRegion(
                    &destination,
                    dst.origin.x,
                    dst.origin.y,
                    0,
                    &source,
                    None,
                );
            }
            drop_copy_location(source);
            drop_copy_location(destination);
        }
        transition_buffer(&encoder.list, src.buffer, buffer_state);
        transition_texture(&encoder.list, dst.texture, texture_state);
        encoder.resources.push(src.buffer.resource.clone());
        encoder.resources.push(dst.texture.resource.clone());
    }

    pub(super) fn encode_copy_texture_to_texture(
        &self,
        encoder: &mut Dx12CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: crate::backend::Extent3d,
    ) {
        assert_eq!(src.aspect, TextureAspect::All);
        assert_eq!(dst.aspect, TextureAspect::All);
        let before_src =
            transition_texture(&encoder.list, src.texture, D3D12_RESOURCE_STATE_COPY_SOURCE);
        let before_dst =
            transition_texture(&encoder.list, dst.texture, D3D12_RESOURCE_STATE_COPY_DEST);
        for layer in 0..extent.depth_or_array_layers {
            let source = subresource_location(
                &src.texture.resource,
                texture_subresource(src.texture, src.mip_level, src.origin.z + layer),
            );
            let destination = subresource_location(
                &dst.texture.resource,
                texture_subresource(dst.texture, dst.mip_level, dst.origin.z + layer),
            );
            unsafe {
                encoder.list.CopyTextureRegion(
                    &destination,
                    dst.origin.x,
                    dst.origin.y,
                    0,
                    &source,
                    Some(&D3D12_BOX {
                        left: src.origin.x,
                        top: src.origin.y,
                        front: 0,
                        right: src.origin.x + extent.width,
                        bottom: src.origin.y + extent.height,
                        back: 1,
                    }),
                );
            }
            drop_copy_location(source);
            drop_copy_location(destination);
        }
        restore_texture_state(&encoder.list, src.texture, before_src);
        restore_texture_state(&encoder.list, dst.texture, before_dst);
        encoder.resources.push(src.texture.resource.clone());
        encoder.resources.push(dst.texture.resource.clone());
    }

    pub(super) fn encode_copy_texture_to_buffer(
        &self,
        encoder: &mut Dx12CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: crate::backend::Extent3d,
    ) {
        assert_eq!(src.aspect, TextureAspect::All);
        let row_pitch = dst
            .layout
            .bytes_per_row
            .expect("D3D12 texture-to-buffer copies need bytes_per_row");
        assert_eq!(row_pitch % COPY_ROW_ALIGNMENT, 0);
        let image_pitch = row_pitch
            .checked_mul(dst.layout.rows_per_image.unwrap_or(extent.height))
            .expect("D3D12 texture readback image pitch overflowed");
        let texture_state =
            transition_texture(&encoder.list, src.texture, D3D12_RESOURCE_STATE_COPY_SOURCE);
        let buffer_state =
            transition_buffer(&encoder.list, dst.buffer, D3D12_RESOURCE_STATE_COPY_DEST);
        for layer in 0..extent.depth_or_array_layers {
            let source = subresource_location(
                &src.texture.resource,
                texture_subresource(src.texture, src.mip_level, src.origin.z + layer),
            );
            let destination = placed_buffer_location(
                &dst.buffer.resource,
                dst.layout.offset + image_pitch as u64 * layer as u64,
                src.texture.format.dxgi(),
                extent.width,
                extent.height,
                row_pitch,
            );
            unsafe {
                encoder.list.CopyTextureRegion(
                    &destination,
                    0,
                    0,
                    0,
                    &source,
                    Some(&D3D12_BOX {
                        left: src.origin.x,
                        top: src.origin.y,
                        front: 0,
                        right: src.origin.x + extent.width,
                        bottom: src.origin.y + extent.height,
                        back: 1,
                    }),
                );
            }
            drop_copy_location(source);
            drop_copy_location(destination);
        }
        transition_texture(&encoder.list, src.texture, texture_state);
        transition_buffer(&encoder.list, dst.buffer, buffer_state);
        encoder.resources.push(src.texture.resource.clone());
        encoder.resources.push(dst.buffer.resource.clone());
    }
}

impl Dx12CommandEncoder {
    pub(super) fn submit(self) {
        let shared = self.shared.clone();
        // Buffer uploads issued while a render pass is being recorded (the
        // rectangle and image instance buffers) must execute before its draw
        // list. Record them into a leading list and execute both lists in one
        // queue submission so the CPU still pays for only one fence signal.
        let uploads = std::mem::take(
            &mut *self
                .shared
                .pending_uploads
                .lock()
                .expect("D3D12 upload queue mutex is not poisoned"),
        );
        let upload_encoder = if uploads.is_empty() {
            None
        } else {
            let backend = Dx12Backend {
                shared: self.shared.clone(),
            };
            Some(backend.make_command_encoder_with_uploads("D3D12 queued buffer uploads", uploads))
        };

        let mut command_lists = Vec::with_capacity(if upload_encoder.is_some() { 2 } else { 1 });
        let mut resources = Vec::new();
        let mut descriptors = Vec::new();
        let mut objects = Vec::new();
        if let Some(encoder) = upload_encoder {
            let (list, mut encoder_resources, mut encoder_descriptors, mut encoder_objects) =
                encoder.close();
            command_lists.push(Some(list));
            resources.append(&mut encoder_resources);
            descriptors.append(&mut encoder_descriptors);
            objects.append(&mut encoder_objects);
        }
        let (list, mut encoder_resources, mut encoder_descriptors, mut encoder_objects) =
            self.close();
        command_lists.push(Some(list));
        resources.append(&mut encoder_resources);
        descriptors.append(&mut encoder_descriptors);
        objects.append(&mut encoder_objects);

        let submission = shared
            .submit_lock
            .lock()
            .expect("D3D12 submission mutex is not poisoned");
        unsafe { shared.queue.ExecuteCommandLists(&command_lists) };
        let fence_value = shared
            .next_fence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        unsafe {
            shared
                .queue
                .Signal(&shared.fence, fence_value)
                .expect("signal D3D12 submission fence");
        }
        shared
            .last_fence
            .store(fence_value, std::sync::atomic::Ordering::Release);
        shared
            .retired
            .lock()
            .expect("D3D12 retired resource mutex is not poisoned")
            .push(RetiredBatch {
                fence_value,
                _resources: resources,
                _descriptors: descriptors,
                _objects: objects,
            });
        drop(submission);
    }

    fn close(
        self,
    ) -> (
        ID3D12CommandList,
        Vec<ID3D12Resource>,
        Vec<Arc<DescriptorLease>>,
        Vec<IUnknown>,
    ) {
        unsafe { self.list.Close().expect("close D3D12 command list") };
        let base = self.list.cast().expect("cast D3D12 command list");
        let mut objects = self.objects;
        objects.push(self.allocator.cast().expect("retain D3D12 allocator"));
        objects.push(self.list.cast().expect("retain D3D12 command list"));
        (base, self.resources, self.descriptors, objects)
    }
}

impl GpuRenderPass<Dx12Backend> for Dx12RenderPass<'_> {
    fn set_pipeline(&mut self, pipeline: &Dx12RenderPipeline) {
        self.pipeline = Some(pipeline.clone());
        unsafe {
            self.encoder
                .list
                .SetGraphicsRootSignature(&pipeline.layout.root_signature);
            self.encoder.list.SetPipelineState(&pipeline.state);
            self.encoder.list.IASetPrimitiveTopology(pipeline.topology);
            let heaps = [
                Some(pipeline_heap(&self.encoder.shared.resource_heap.heap)),
                Some(pipeline_heap(&self.encoder.shared.sampler_heap.heap)),
            ];
            self.encoder.list.SetDescriptorHeaps(&heaps);
        }
        self.encoder
            .objects
            .push(pipeline.state.cast().expect("retain D3D12 pipeline state"));
        self.encoder.objects.push(
            pipeline
                .layout
                .root_signature
                .cast()
                .expect("retain D3D12 root signature"),
        );
    }

    fn set_bind_group(&mut self, index: u32, bind_group: &Dx12BindGroup, dynamic_offsets: &[u32]) {
        let pipeline = self.pipeline.as_ref().expect("set a D3D12 pipeline first");
        let group = pipeline
            .layout
            .groups
            .get(index as usize)
            .unwrap_or_else(|| panic!("D3D12 pipeline has no bind group {index}"));
        for plan in &group.bindings {
            let resource = bind_group
                .entries
                .iter()
                .find(|(binding, _)| *binding == plan.binding)
                .map(|(_, resource)| resource)
                .unwrap_or_else(|| panic!("D3D12 bind group is missing binding {}", plan.binding));
            match (plan.kind, resource) {
                (
                    RootBindingKind::Buffer(ty),
                    BoundResource::Buffer {
                        buffer,
                        offset,
                        size,
                    },
                ) => {
                    let dynamic_offset = plan
                        .dynamic_offset_index
                        .and_then(|slot| dynamic_offsets.get(slot))
                        .copied()
                        .unwrap_or(0) as u64;
                    let address = unsafe { buffer.resource.GetGPUVirtualAddress() }
                        + *offset
                        + dynamic_offset;
                    unsafe {
                        match ty {
                            crate::backend::BufferBindingType::Uniform => self
                                .encoder
                                .list
                                .SetGraphicsRootConstantBufferView(plan.root_index, address),
                            crate::backend::BufferBindingType::ReadOnlyStorage => self
                                .encoder
                                .list
                                .SetGraphicsRootShaderResourceView(plan.root_index, address),
                            crate::backend::BufferBindingType::Storage => self
                                .encoder
                                .list
                                .SetGraphicsRootUnorderedAccessView(plan.root_index, address),
                        }
                    }
                    assert!(*size <= buffer.size.saturating_sub(*offset + dynamic_offset));
                    let required_state = match ty {
                        crate::backend::BufferBindingType::Uniform => {
                            D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER
                        }
                        crate::backend::BufferBindingType::ReadOnlyStorage => {
                            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
                        }
                        crate::backend::BufferBindingType::Storage => {
                            D3D12_RESOURCE_STATE_UNORDERED_ACCESS
                        }
                    };
                    transition_buffer(&self.encoder.list, buffer, required_state);
                    self.encoder.resources.push(buffer.resource.clone());
                }
                (RootBindingKind::Texture, BoundResource::Texture { view })
                | (RootBindingKind::StorageTexture(true), BoundResource::Texture { view }) => {
                    transition_texture(
                        &self.encoder.list,
                        &view.texture,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    );
                    let descriptor = view.srv.as_ref().expect("texture has no D3D12 SRV");
                    unsafe {
                        self.encoder.list.SetGraphicsRootDescriptorTable(
                            plan.root_index,
                            descriptor.pool.gpu_handle(descriptor.index),
                        );
                    }
                    self.encoder.resources.push(view.texture.resource.clone());
                    self.encoder.descriptors.push(descriptor.clone());
                }
                (RootBindingKind::StorageTexture(false), BoundResource::Texture { view }) => {
                    transition_texture(
                        &self.encoder.list,
                        &view.texture,
                        D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    );
                    let descriptor = view.uav.as_ref().expect("texture has no D3D12 UAV");
                    unsafe {
                        self.encoder.list.SetGraphicsRootDescriptorTable(
                            plan.root_index,
                            descriptor.pool.gpu_handle(descriptor.index),
                        );
                    }
                    self.encoder.resources.push(view.texture.resource.clone());
                    self.encoder.descriptors.push(descriptor.clone());
                }
                (RootBindingKind::Sampler, BoundResource::Sampler { sampler }) => {
                    unsafe {
                        self.encoder.list.SetGraphicsRootDescriptorTable(
                            plan.root_index,
                            sampler.descriptor.pool.gpu_handle(sampler.descriptor.index),
                        );
                    }
                    self.encoder.descriptors.push(sampler.descriptor.clone());
                }
                _ => panic!(
                    "D3D12 bind group binding {} does not match the pipeline",
                    plan.binding
                ),
            }
        }
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &Dx12Buffer, offset: u64) {
        let pipeline = self.pipeline.as_ref().expect("set a D3D12 pipeline first");
        let stride = *pipeline
            .vertex_strides
            .get(slot as usize)
            .unwrap_or_else(|| panic!("D3D12 vertex buffer slot {slot} is not in the pipeline"));
        assert!(
            offset < buffer.size,
            "D3D12 vertex buffer offset exceeds its allocation"
        );
        let view = D3D12_VERTEX_BUFFER_VIEW {
            BufferLocation: unsafe { buffer.resource.GetGPUVirtualAddress() } + offset,
            SizeInBytes: u32::try_from(buffer.size - offset)
                .expect("D3D12 vertex buffers must fit within 4 GiB"),
            StrideInBytes: stride,
        };
        transition_buffer(
            &self.encoder.list,
            buffer,
            D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER,
        );
        unsafe { self.encoder.list.IASetVertexBuffers(slot, Some(&[view])) };
        self.encoder.resources.push(buffer.resource.clone());
    }

    fn set_index_buffer(&mut self, buffer: &Dx12Buffer, format: IndexFormat, offset: u64) {
        assert!(
            offset < buffer.size,
            "D3D12 index buffer offset exceeds its allocation"
        );
        let view = D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: unsafe { buffer.resource.GetGPUVirtualAddress() } + offset,
            SizeInBytes: u32::try_from(buffer.size - offset)
                .expect("D3D12 index buffers must fit within 4 GiB"),
            Format: match format {
                IndexFormat::Uint16 => DXGI_FORMAT_R16_UINT,
                IndexFormat::Uint32 => windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R32_UINT,
            },
        };
        transition_buffer(
            &self.encoder.list,
            buffer,
            D3D12_RESOURCE_STATE_INDEX_BUFFER,
        );
        unsafe { self.encoder.list.IASetIndexBuffer(Some(&view)) };
        self.index_buffer = Some((buffer.clone(), format, offset));
        self.encoder.resources.push(buffer.resource.clone());
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let left = x.min(i32::MAX as u32) as i32;
        let top = y.min(i32::MAX as u32) as i32;
        let rect = RECT {
            left,
            top,
            right: left.saturating_add(width.min(i32::MAX as u32) as i32),
            bottom: top.saturating_add(height.min(i32::MAX as u32) as i32),
        };
        unsafe { self.encoder.list.RSSetScissorRects(&[rect]) };
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        unsafe {
            self.encoder.list.DrawInstanced(
                vertices.end - vertices.start,
                instances.end - instances.start,
                vertices.start,
                instances.start,
            );
        }
    }

    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        assert!(
            self.index_buffer.is_some(),
            "set_index_buffer before draw_indexed"
        );
        unsafe {
            self.encoder.list.DrawIndexedInstanced(
                indices.end - indices.start,
                instances.end - instances.start,
                indices.start,
                base_vertex,
                instances.start,
            );
        }
    }
}

impl Drop for Dx12RenderPass<'_> {
    fn drop(&mut self) {
        for (texture, previous) in &self.targets {
            transition_texture(&self.encoder.list, texture, *previous);
        }
    }
}

fn pipeline_heap(heap: &ID3D12DescriptorHeap) -> ID3D12DescriptorHeap {
    heap.clone()
}

fn encode_upload(encoder: &mut Dx12CommandEncoder, upload: PendingUpload) {
    match upload {
        PendingUpload::Buffer(PendingBufferUpload {
            source,
            destination,
            destination_offset,
            size,
        }) => {
            transition_buffer(&encoder.list, &destination, D3D12_RESOURCE_STATE_COPY_DEST);
            unsafe {
                encoder.list.CopyBufferRegion(
                    &destination.resource,
                    destination_offset,
                    &source,
                    0,
                    size,
                );
            }
            transition_buffer(
                &encoder.list,
                &destination,
                preferred_buffer_state(&destination.usage),
            );
            encoder.resources.push(source);
            encoder.resources.push(destination.resource.clone());
        }
        PendingUpload::Texture(PendingTextureUpload {
            source,
            destination,
            mip_level,
            origin,
            extent,
            row_pitch,
            image_pitch,
        }) => {
            transition_texture(&encoder.list, &destination, D3D12_RESOURCE_STATE_COPY_DEST);
            for layer in 0..extent.depth_or_array_layers {
                let src = placed_buffer_location(
                    &source,
                    image_pitch as u64 * layer as u64,
                    destination.format.dxgi(),
                    extent.width,
                    extent.height,
                    row_pitch,
                );
                let dst = subresource_location(
                    &destination.resource,
                    texture_subresource(&destination, mip_level, origin.z + layer),
                );
                unsafe {
                    encoder
                        .list
                        .CopyTextureRegion(&dst, origin.x, origin.y, 0, &src, None);
                }
                drop_copy_location(src);
                drop_copy_location(dst);
            }
            transition_texture(&encoder.list, &destination, D3D12_RESOURCE_STATE_COMMON);
            encoder.resources.push(source);
            encoder.resources.push(destination.resource.clone());
        }
    }
}

fn preferred_buffer_state(usages: &[crate::backend::BufferUsage]) -> D3D12_RESOURCE_STATES {
    use crate::backend::BufferUsage;
    let mut state = D3D12_RESOURCE_STATE_COMMON;
    if usages.contains(&BufferUsage::Vertex) || usages.contains(&BufferUsage::Uniform) {
        state |= D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
    }
    if usages.contains(&BufferUsage::Index) {
        state |= D3D12_RESOURCE_STATE_INDEX_BUFFER;
    }
    if usages.contains(&BufferUsage::ReadOnlyStorage) {
        state |= D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
    }
    if usages.contains(&BufferUsage::Storage) {
        state |= D3D12_RESOURCE_STATE_UNORDERED_ACCESS;
    }
    if usages.contains(&BufferUsage::Indirect) {
        state |= D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT;
    }
    if usages.contains(&BufferUsage::CopySrc) {
        state |= D3D12_RESOURCE_STATE_COPY_SOURCE;
    }
    state
}

fn transition_buffer(
    list: &ID3D12GraphicsCommandList,
    buffer: &Dx12Buffer,
    target: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_STATES {
    if buffer.upload_heap {
        return D3D12_RESOURCE_STATE_GENERIC_READ;
    }
    transition_resource(list, &buffer.resource, &buffer.state, target)
}

fn transition_texture(
    list: &ID3D12GraphicsCommandList,
    texture: &Dx12Texture,
    target: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_STATES {
    transition_resource(list, &texture.resource, &texture.state, target)
}

fn restore_texture_state(
    list: &ID3D12GraphicsCommandList,
    texture: &Dx12Texture,
    previous: D3D12_RESOURCE_STATES,
) {
    transition_texture(list, texture, previous);
}

fn transition_resource(
    list: &ID3D12GraphicsCommandList,
    resource: &ID3D12Resource,
    state: &std::sync::Mutex<D3D12_RESOURCE_STATES>,
    target: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_STATES {
    let mut state = state
        .lock()
        .expect("D3D12 resource state mutex is not poisoned");
    let before = *state;
    if before != target {
        let transition = D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: ManuallyDrop::new(Some(resource.clone())),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: before,
            StateAfter: target,
        };
        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: ManuallyDrop::new(transition),
            },
        };
        let mut barriers = [barrier];
        unsafe {
            list.ResourceBarrier(&barriers);
            // SAFETY: D3D12_RESOURCE_BARRIER stores this COM pointer in a
            // ManuallyDrop union. The call consumes the pointer only for the
            // duration of ResourceBarrier, so the local clone is released here.
            ManuallyDrop::drop(&mut (*barriers[0].Anonymous.Transition).pResource);
        }
        *state = target;
    }
    before
}

fn texture_subresource(texture: &Dx12Texture, mip_level: u32, array_slice: u32) -> u32 {
    mip_level + array_slice * unsafe { texture.resource.GetDesc().MipLevels as u32 }
}

fn subresource_location(
    resource: &ID3D12Resource,
    subresource: u32,
) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(resource.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            SubresourceIndex: subresource,
        },
    }
}

fn placed_buffer_location(
    resource: &ID3D12Resource,
    offset: u64,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    width: u32,
    height: u32,
    row_pitch: u32,
) -> D3D12_TEXTURE_COPY_LOCATION {
    D3D12_TEXTURE_COPY_LOCATION {
        pResource: ManuallyDrop::new(Some(resource.clone())),
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                Offset: offset,
                Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                    Format: format,
                    Width: width,
                    Height: height,
                    Depth: 1,
                    RowPitch: row_pitch,
                },
            },
        },
    }
}

fn drop_copy_location(mut location: D3D12_TEXTURE_COPY_LOCATION) {
    // SAFETY: the function constructed each location from one cloned COM
    // resource and owns the ManuallyDrop field until this point.
    unsafe { ManuallyDrop::drop(&mut location.pResource) };
}
