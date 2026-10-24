use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use ash::vk;

use super::*;

mod copy;
use super::pipeline::VulkanRenderPassObject;
pub(super) use copy::{copy_buffer_to_texture, copy_texture_to_buffer, copy_texture_to_texture};

pub(super) struct PendingBufferUpload {
    pub(super) staging: VulkanBuffer,
    pub(super) destination: VulkanBuffer,
    pub(super) offset: u64,
}

pub(super) struct PendingTextureUpload {
    pub(super) staging: VulkanBuffer,
    pub(super) destination: VulkanTexture,
    pub(super) mip_level: u32,
    pub(super) origin: Origin3d,
    pub(super) aspect: TextureAspect,
    pub(super) extent: Extent3d,
    pub(super) row_length: u32,
    pub(super) image_height: u32,
}

/// Opaque Vulkan command encoder used by [`super::VulkanBackend`].
#[doc(hidden)]
pub struct VulkanCommandEncoder {
    shared: Arc<VulkanShared>,
    pool: vk::CommandPool,
    pub(super) raw: vk::CommandBuffer,
    retained: Vec<Box<dyn Any>>,
    layouts: Vec<(VulkanTexture, vk::ImageLayout)>,
    submitted: bool,
}

/// Render-pass state borrowing its active Vulkan command encoder.
#[doc(hidden)]
pub struct VulkanRenderPass<'a> {
    encoder: &'a mut VulkanCommandEncoder,
    extent: vk::Extent2D,
    pipeline: Option<VulkanRenderPipeline>,
}

pub(super) struct InFlight {
    core: Arc<VulkanCore>,
    pools: Vec<vk::CommandPool>,
    fence: vk::Fence,
    semaphores: Vec<vk::Semaphore>,
    _retained: Vec<Box<dyn Any>>,
}

impl VulkanCommandEncoder {
    pub(super) fn new(
        shared: Arc<VulkanShared>,
        label: &str,
        buffer_uploads: Vec<PendingBufferUpload>,
        texture_uploads: Vec<PendingTextureUpload>,
    ) -> Self {
        collect_finished(&shared);
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(shared.graphics_family)
            .flags(
                vk::CommandPoolCreateFlags::TRANSIENT
                    | vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            );
        let pool = unsafe {
            shared
                .core
                .device
                .create_command_pool(&pool_info, None)
                .expect("create Vulkan command pool")
        };
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let raw = unsafe {
            shared
                .core
                .device
                .allocate_command_buffers(&allocate_info)
                .expect("allocate Vulkan command buffer")[0]
        };
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            shared
                .core
                .device
                .begin_command_buffer(raw, &begin)
                .expect("begin Vulkan command buffer");
        }
        let _ = label;
        let mut encoder = Self {
            shared,
            pool,
            raw,
            retained: Vec::new(),
            layouts: Vec::new(),
            submitted: false,
        };
        for upload in buffer_uploads {
            encoder.record_buffer_upload(upload);
        }
        for upload in texture_uploads {
            encoder.record_texture_upload(upload);
        }
        encoder
    }

    fn record_buffer_upload(&mut self, upload: PendingBufferUpload) {
        let buffer_barrier = [vk::BufferMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(upload.staging.0.raw)
            .offset(0)
            .size(vk::WHOLE_SIZE)];
        unsafe {
            self.shared.core.device.cmd_pipeline_barrier(
                self.raw,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &buffer_barrier,
                &[],
            );
            self.shared.core.device.cmd_copy_buffer(
                self.raw,
                upload.staging.0.raw,
                upload.destination.0.raw,
                &[vk::BufferCopy {
                    src_offset: 0,
                    dst_offset: upload.offset,
                    size: upload
                        .staging
                        .0
                        .size
                        .min(upload.destination.0.size.saturating_sub(upload.offset)),
                }],
            );
        }
        self.buffer_barrier(
            &upload.destination,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::ALL_COMMANDS,
        );
        self.retain(upload.staging);
        self.retain(upload.destination);
    }

    fn record_texture_upload(&mut self, upload: PendingTextureUpload) {
        self.image_barrier(&upload.destination, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        let region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(upload.row_length)
            .buffer_image_height(upload.image_height)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: texture_aspect(upload.destination.0.format, upload.aspect),
                mip_level: upload.mip_level,
                base_array_layer: upload.origin.z,
                layer_count: upload.extent.depth_or_array_layers,
            })
            .image_offset(vk::Offset3D {
                x: upload.origin.x as i32,
                y: upload.origin.y as i32,
                z: if matches!(upload.destination.0.size.2, 1) {
                    0
                } else {
                    upload.origin.z as i32
                },
            })
            .image_extent(vk::Extent3D {
                width: upload.extent.width,
                height: upload.extent.height,
                depth: if upload.destination.0.size.2 > 1 && upload.extent.depth_or_array_layers > 1
                {
                    upload.extent.depth_or_array_layers
                } else {
                    1
                },
            });
        unsafe {
            self.shared.core.device.cmd_pipeline_barrier(
                self.raw,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::HOST_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(upload.staging.0.raw)
                    .offset(0)
                    .size(vk::WHOLE_SIZE)],
                &[],
            );
            self.shared.core.device.cmd_copy_buffer_to_image(
                self.raw,
                upload.staging.0.raw,
                upload.destination.0.raw,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
        let final_layout = if upload
            .destination
            .0
            .usage
            .contains(&TextureUsage::TextureBinding)
        {
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
        } else {
            vk::ImageLayout::GENERAL
        };
        self.image_barrier(&upload.destination, final_layout);
        self.retain(upload.staging);
        self.retain(upload.destination);
    }

    pub(super) fn retain<T: Any + 'static>(&mut self, resource: T) {
        self.retained.push(Box::new(resource));
    }

    fn buffer_barrier(
        &self,
        buffer: &VulkanBuffer,
        source_access: vk::AccessFlags,
        destination_access: vk::AccessFlags,
        source_stage: vk::PipelineStageFlags,
        destination_stage: vk::PipelineStageFlags,
    ) {
        let barrier = [vk::BufferMemoryBarrier::default()
            .src_access_mask(source_access)
            .dst_access_mask(destination_access)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(buffer.0.raw)
            .offset(0)
            .size(vk::WHOLE_SIZE)];
        unsafe {
            self.shared.core.device.cmd_pipeline_barrier(
                self.raw,
                source_stage,
                destination_stage,
                vk::DependencyFlags::empty(),
                &[],
                &barrier,
                &[],
            );
        }
    }

    fn current_layout(&self, texture: &VulkanTexture) -> vk::ImageLayout {
        self.layouts
            .iter()
            .rev()
            .find_map(|(candidate, layout)| {
                Arc::ptr_eq(&candidate.0, &texture.0).then_some(*layout)
            })
            .unwrap_or_else(|| {
                *texture
                    .0
                    .layout
                    .lock()
                    .expect("Vulkan image layout mutex is not poisoned")
            })
    }

    fn set_layout(&mut self, texture: &VulkanTexture, layout: vk::ImageLayout) {
        if let Some((_, current)) = self
            .layouts
            .iter_mut()
            .rev()
            .find(|(candidate, _)| Arc::ptr_eq(&candidate.0, &texture.0))
        {
            *current = layout;
        } else {
            self.layouts.push((texture.clone(), layout));
        }
    }

    fn image_barrier(&mut self, texture: &VulkanTexture, new_layout: vk::ImageLayout) {
        let old_layout = self.current_layout(texture);
        if old_layout == new_layout {
            return;
        }
        let (src_stage, src_access) = layout_source(old_layout);
        let (dst_stage, dst_access) = layout_destination(new_layout);
        let barrier = [vk::ImageMemoryBarrier::default()
            .src_access_mask(src_access)
            .dst_access_mask(dst_access)
            .old_layout(old_layout)
            .new_layout(new_layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(texture.0.raw)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: super::image_aspect(texture.0.format),
                base_mip_level: 0,
                level_count: vk::REMAINING_MIP_LEVELS,
                base_array_layer: 0,
                layer_count: vk::REMAINING_ARRAY_LAYERS,
            })];
        unsafe {
            self.shared.core.device.cmd_pipeline_barrier(
                self.raw,
                src_stage,
                dst_stage,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barrier,
            );
        }
        self.set_layout(texture, new_layout);
    }

    pub(super) fn submit(self) {
        self.submit_with(None, None, Vec::new());
    }

    pub(super) fn submit_with_uploads(mut self, mut uploads: VulkanCommandEncoder) {
        unsafe {
            uploads
                .shared
                .core
                .device
                .end_command_buffer(uploads.raw)
                .expect("end Vulkan upload command buffer");
            self.shared
                .core
                .device
                .end_command_buffer(self.raw)
                .expect("end Vulkan render command buffer");
        }
        let semaphore = unsafe {
            self.shared
                .core
                .device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .expect("create Vulkan upload dependency semaphore")
        };
        let fence = unsafe {
            self.shared
                .core
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .expect("create Vulkan render submission fence")
        };
        let upload_buffers = [uploads.raw];
        let upload_signals = [semaphore];
        let upload_submit = vk::SubmitInfo::default()
            .command_buffers(&upload_buffers)
            .signal_semaphores(&upload_signals);
        let render_buffers = [self.raw];
        let render_waits = [semaphore];
        let render_wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS];
        let render_submit = vk::SubmitInfo::default()
            .wait_semaphores(&render_waits)
            .wait_dst_stage_mask(&render_wait_stages)
            .command_buffers(&render_buffers);
        let submits = [upload_submit, render_submit];
        let queue_guard = self
            .shared
            .queue_lock
            .lock()
            .expect("Vulkan graphics queue mutex is not poisoned");
        let result = unsafe {
            self.shared
                .core
                .device
                .queue_submit(self.shared.graphics_queue, &submits, fence)
        };
        drop(queue_guard);
        if let Err(error) = result {
            unsafe {
                self.shared.core.device.destroy_semaphore(semaphore, None);
                self.shared.core.device.destroy_fence(fence, None);
                self.shared
                    .core
                    .device
                    .destroy_command_pool(uploads.pool, None);
                self.shared
                    .core
                    .device
                    .destroy_command_pool(self.pool, None);
            }
            uploads.submitted = true;
            self.submitted = true;
            panic!("submit Vulkan upload and render command buffers: {error:?}");
        }
        for (texture, layout) in &uploads.layouts {
            *texture
                .0
                .layout
                .lock()
                .expect("Vulkan image layout mutex is not poisoned") = *layout;
        }
        for (texture, layout) in &self.layouts {
            *texture
                .0
                .layout
                .lock()
                .expect("Vulkan image layout mutex is not poisoned") = *layout;
        }
        let upload_pool = std::mem::replace(&mut uploads.pool, vk::CommandPool::null());
        let render_pool = std::mem::replace(&mut self.pool, vk::CommandPool::null());
        let mut retained = std::mem::take(&mut uploads.retained);
        retained.append(&mut self.retained);
        uploads.submitted = true;
        self.submitted = true;
        self.shared
            .in_flight
            .lock()
            .expect("Vulkan in-flight mutex is not poisoned")
            .push(InFlight {
                core: self.shared.core.clone(),
                pools: vec![upload_pool, render_pool],
                fence,
                semaphores: vec![semaphore],
                _retained: retained,
            });
        collect_finished(&self.shared);
    }

    fn submit_with(
        mut self,
        wait_semaphore: Option<vk::Semaphore>,
        signal_semaphore: Option<vk::Semaphore>,
        retained_semaphores: Vec<vk::Semaphore>,
    ) {
        unsafe {
            self.shared
                .core
                .device
                .end_command_buffer(self.raw)
                .expect("end Vulkan command buffer");
        }
        let fence = unsafe {
            self.shared
                .core
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .expect("create Vulkan submission fence")
        };
        let command_buffers = [self.raw];
        let wait_semaphores = wait_semaphore.into_iter().collect::<Vec<_>>();
        let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS];
        let signal_semaphores = signal_semaphore.into_iter().collect::<Vec<_>>();
        let mut submit = vk::SubmitInfo::default().command_buffers(&command_buffers);
        if !wait_semaphores.is_empty() {
            submit = submit
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&wait_stages);
        }
        if !signal_semaphores.is_empty() {
            submit = submit.signal_semaphores(&signal_semaphores);
        }
        let queue_lock = self
            .shared
            .queue_lock
            .lock()
            .expect("Vulkan graphics queue mutex is not poisoned");
        let result = unsafe {
            self.shared
                .core
                .device
                .queue_submit(self.shared.graphics_queue, &[submit], fence)
        };
        drop(queue_lock);
        if let Err(error) = result {
            unsafe {
                if let Some(semaphore) = signal_semaphore {
                    self.shared.core.device.destroy_semaphore(semaphore, None);
                }
                self.shared.core.device.destroy_fence(fence, None);
                self.shared
                    .core
                    .device
                    .destroy_command_pool(self.pool, None);
            }
            self.submitted = true;
            panic!("submit Vulkan command buffer: {error:?}");
        }
        for (texture, layout) in &self.layouts {
            *texture
                .0
                .layout
                .lock()
                .expect("Vulkan image layout mutex is not poisoned") = *layout;
        }
        let pool = std::mem::replace(&mut self.pool, vk::CommandPool::null());
        let retained = std::mem::take(&mut self.retained);
        self.submitted = true;
        self.shared
            .in_flight
            .lock()
            .expect("Vulkan in-flight mutex is not poisoned")
            .push(InFlight {
                core: self.shared.core.clone(),
                pools: vec![pool],
                fence,
                semaphores: retained_semaphores,
                _retained: retained,
            });
        collect_finished(&self.shared);
    }
}

impl Drop for VulkanCommandEncoder {
    fn drop(&mut self) {
        if !self.submitted && self.pool != vk::CommandPool::null() {
            unsafe {
                self.shared
                    .core
                    .device
                    .destroy_command_pool(self.pool, None)
            };
        }
    }
}

impl Drop for VulkanShared {
    fn drop(&mut self) {
        unsafe {
            let _ = self.core.device.device_wait_idle();
        }
        let mut in_flight = self
            .in_flight
            .lock()
            .expect("Vulkan in-flight mutex is not poisoned");
        for finished in in_flight.drain(..) {
            unsafe {
                finished.core.device.destroy_fence(finished.fence, None);
                for pool in finished.pools {
                    finished.core.device.destroy_command_pool(pool, None);
                }
                for semaphore in finished.semaphores {
                    finished.core.device.destroy_semaphore(semaphore, None);
                }
            }
        }
    }
}

impl<'a> GpuRenderPass<VulkanBackend> for VulkanRenderPass<'a> {
    fn set_pipeline(&mut self, pipeline: &VulkanRenderPipeline) {
        unsafe {
            self.encoder.shared.core.device.cmd_bind_pipeline(
                self.encoder.raw,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.0.raw,
            );
        }
        self.pipeline = Some((*pipeline).clone());
        self.encoder.retain((*pipeline).clone());
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &VulkanBindGroup,
        dynamic_offsets: &[u32],
    ) {
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("set a Vulkan pipeline before binding groups");
        let sets = [bind_group.0.raw];
        unsafe {
            self.encoder.shared.core.device.cmd_bind_descriptor_sets(
                self.encoder.raw,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.0.layout.0.raw,
                index,
                &sets,
                dynamic_offsets,
            );
        }
        self.encoder.retain((*bind_group).clone());
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &VulkanBuffer, offset: u64) {
        let buffers = [buffer.0.raw];
        let offsets = [offset];
        unsafe {
            self.encoder.shared.core.device.cmd_bind_vertex_buffers(
                self.encoder.raw,
                slot,
                &buffers,
                &offsets,
            )
        };
        self.encoder.retain((*buffer).clone());
    }

    fn set_index_buffer(&mut self, buffer: &VulkanBuffer, index_format: IndexFormat, offset: u64) {
        let format = match index_format {
            IndexFormat::Uint16 => vk::IndexType::UINT16,
            IndexFormat::Uint32 => vk::IndexType::UINT32,
        };
        unsafe {
            self.encoder.shared.core.device.cmd_bind_index_buffer(
                self.encoder.raw,
                buffer.0.raw,
                offset,
                format,
            )
        };
        self.encoder.retain((*buffer).clone());
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let scissor = [vk::Rect2D {
            offset: vk::Offset2D {
                x: x.min(i32::MAX as u32) as i32,
                y: y.min(i32::MAX as u32) as i32,
            },
            extent: vk::Extent2D {
                width: width.min(self.extent.width.saturating_sub(x)),
                height: height.min(self.extent.height.saturating_sub(y)),
            },
        }];
        unsafe {
            self.encoder
                .shared
                .core
                .device
                .cmd_set_scissor(self.encoder.raw, 0, &scissor)
        };
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        unsafe {
            self.encoder.shared.core.device.cmd_draw(
                self.encoder.raw,
                vertices.end.saturating_sub(vertices.start),
                instances.end.saturating_sub(instances.start),
                vertices.start,
                instances.start,
            )
        };
    }

    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        unsafe {
            self.encoder.shared.core.device.cmd_draw_indexed(
                self.encoder.raw,
                indices.end.saturating_sub(indices.start),
                instances.end.saturating_sub(instances.start),
                indices.start,
                base_vertex,
                instances.start,
            )
        };
    }
}

impl Drop for VulkanRenderPass<'_> {
    fn drop(&mut self) {
        unsafe {
            self.encoder
                .shared
                .core
                .device
                .cmd_end_render_pass(self.encoder.raw)
        };
    }
}

pub(super) fn begin_render_pass<'a>(
    encoder: &'a mut VulkanCommandEncoder,
    desc: &RenderPassDescriptor<VulkanBackend>,
) -> VulkanRenderPass<'a> {
    let mut attachments = Vec::new();
    let mut color_refs = Vec::new();
    let mut resolve_refs = Vec::new();
    let mut views = Vec::new();
    let mut framebuffer_views = Vec::new();
    let mut clear_values = Vec::new();
    for color in desc.color_attachments {
        let texture = &color.view.0.texture;
        let final_layout = if texture.0.presentable {
            vk::ImageLayout::PRESENT_SRC_KHR
        } else if texture.0.usage.contains(&TextureUsage::TextureBinding) {
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
        } else {
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        };
        let initial = encoder.current_layout(texture);
        attachments.push(
            vk::AttachmentDescription::default()
                .format(texture.0.format)
                .samples(texture_sample_count(texture))
                .load_op(load_op(color.ops.load))
                .store_op(store_op(color.ops.store))
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(initial)
                .final_layout(final_layout),
        );
        color_refs.push(
            vk::AttachmentReference::default()
                .attachment((attachments.len() - 1) as u32)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
        );
        views.push(color.view.0.raw);
        framebuffer_views.push((*color.view).clone());
        clear_values.push(match color.ops.load {
            LoadOp::Clear(color) => vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: color.map(|channel| channel as f32),
                },
            },
            LoadOp::Load => vk::ClearValue::default(),
        });
    }
    for color in desc.color_attachments {
        if let Some(resolve_target) = color.resolve_target {
            let texture = &resolve_target.0.texture;
            let final_layout = if texture.0.presentable {
                vk::ImageLayout::PRESENT_SRC_KHR
            } else if texture.0.usage.contains(&TextureUsage::TextureBinding) {
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
            } else {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            };
            let initial = encoder.current_layout(texture);
            attachments.push(
                vk::AttachmentDescription::default()
                    .format(texture.0.format)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .initial_layout(initial)
                    .final_layout(final_layout),
            );
            resolve_refs.push(
                vk::AttachmentReference::default()
                    .attachment((attachments.len() - 1) as u32)
                    .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            );
            views.push(resolve_target.0.raw);
            framebuffer_views.push((*resolve_target).clone());
            clear_values.push(vk::ClearValue::default());
        } else {
            resolve_refs.push(
                vk::AttachmentReference::default()
                    .attachment(vk::ATTACHMENT_UNUSED)
                    .layout(vk::ImageLayout::UNDEFINED),
            );
        }
    }
    let depth_ref = desc.depth_stencil_attachment.as_ref().map(|depth| {
        let texture = &depth.view.0.texture;
        let initial = encoder.current_layout(texture);
        let mut attachment = vk::AttachmentDescription::default()
            .format(texture.0.format)
            .samples(texture_sample_count(texture))
            .load_op(
                depth
                    .depth_ops
                    .map(|ops| load_op(ops.load))
                    .unwrap_or(vk::AttachmentLoadOp::DONT_CARE),
            )
            .store_op(
                depth
                    .depth_ops
                    .map(|ops| store_op(ops.store))
                    .unwrap_or(vk::AttachmentStoreOp::DONT_CARE),
            )
            .stencil_load_op(
                depth
                    .stencil_ops
                    .map(|ops| load_op(ops.load))
                    .unwrap_or(vk::AttachmentLoadOp::DONT_CARE),
            )
            .stencil_store_op(
                depth
                    .stencil_ops
                    .map(|ops| store_op(ops.store))
                    .unwrap_or(vk::AttachmentStoreOp::DONT_CARE),
            )
            .initial_layout(initial)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        if !depth.depth_ops.is_some() && !depth.stencil_ops.is_some() {
            attachment = attachment.load_op(vk::AttachmentLoadOp::DONT_CARE);
        }
        attachments.push(attachment);
        views.push(depth.view.0.raw);
        framebuffer_views.push((*depth.view).clone());
        clear_values.push(vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: depth
                    .depth_ops
                    .map(|ops| match ops.load {
                        LoadOp::Clear(value) => value,
                        LoadOp::Load => 1.0,
                    })
                    .unwrap_or(1.0),
                stencil: depth
                    .stencil_ops
                    .map(|ops| match ops.load {
                        LoadOp::Clear(value) => value,
                        LoadOp::Load => 0,
                    })
                    .unwrap_or(0),
            },
        });
        vk::AttachmentReference::default()
            .attachment((attachments.len() - 1) as u32)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
    });
    let mut subpass_desc = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_refs);
    if desc
        .color_attachments
        .iter()
        .any(|attachment| attachment.resolve_target.is_some())
    {
        subpass_desc = subpass_desc.resolve_attachments(&resolve_refs);
    }
    if let Some(depth_ref) = depth_ref.as_ref() {
        subpass_desc = subpass_desc.depth_stencil_attachment(depth_ref);
    }
    let subpass = [subpass_desc];
    let dependencies = [
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::ALL_COMMANDS)
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_READ
                    | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            ),
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
            )
            .dst_stage_mask(vk::PipelineStageFlags::ALL_COMMANDS)
            .src_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE),
    ];
    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpass)
        .dependencies(&dependencies);
    let render_pass = unsafe {
        encoder
            .shared
            .core
            .device
            .create_render_pass(&render_pass_info, None)
            .expect("create Vulkan render pass")
    };
    let extent = desc
        .color_attachments
        .first()
        .map(|attachment| attachment.view.0.texture.0.size)
        .or_else(|| {
            desc.depth_stencil_attachment
                .as_ref()
                .map(|attachment| attachment.view.0.texture.0.size)
        })
        .unwrap_or((1, 1, 1));
    let framebuffer_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(&views)
        .width(extent.0.max(1))
        .height(extent.1.max(1))
        .layers(1);
    let framebuffer = unsafe {
        encoder
            .shared
            .core
            .device
            .create_framebuffer(&framebuffer_info, None)
            .expect("create Vulkan framebuffer")
    };
    let pass_object = Arc::new(super::pipeline::VulkanRenderPassObject {
        core: encoder.shared.core.clone(),
        raw: render_pass,
    });
    encoder.retain(VulkanFramebuffer {
        core: encoder.shared.core.clone(),
        raw: framebuffer,
        _views: framebuffer_views,
        _pass: pass_object,
    });
    for attachment in desc.color_attachments {
        let texture = &attachment.view.0.texture;
        encoder.set_layout(
            texture,
            if texture.0.presentable {
                vk::ImageLayout::PRESENT_SRC_KHR
            } else if texture.0.usage.contains(&TextureUsage::TextureBinding) {
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
            } else {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            },
        );
        encoder.retain((*attachment.view).clone());
        if let Some(resolve) = attachment.resolve_target {
            let texture = &resolve.0.texture;
            encoder.set_layout(
                texture,
                if texture.0.presentable {
                    vk::ImageLayout::PRESENT_SRC_KHR
                } else if texture.0.usage.contains(&TextureUsage::TextureBinding) {
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                } else {
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
                },
            );
            encoder.retain((*resolve).clone());
        }
    }
    if let Some(depth) = desc.depth_stencil_attachment.as_ref() {
        encoder.set_layout(
            &depth.view.0.texture,
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        );
        encoder.retain((*depth.view).clone());
    }
    let begin_info = vk::RenderPassBeginInfo::default()
        .render_pass(render_pass)
        .framebuffer(framebuffer)
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: extent.0.max(1),
                height: extent.1.max(1),
            },
        })
        .clear_values(&clear_values);
    unsafe {
        encoder.shared.core.device.cmd_begin_render_pass(
            encoder.raw,
            &begin_info,
            vk::SubpassContents::INLINE,
        );
        encoder.shared.core.device.cmd_set_viewport(
            encoder.raw,
            0,
            &[vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: extent.0.max(1) as f32,
                height: extent.1.max(1) as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }],
        );
        encoder.shared.core.device.cmd_set_scissor(
            encoder.raw,
            0,
            &[vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: extent.0.max(1),
                    height: extent.1.max(1),
                },
            }],
        );
    }
    VulkanRenderPass {
        encoder,
        extent: vk::Extent2D {
            width: extent.0.max(1),
            height: extent.1.max(1),
        },
        pipeline: None,
    }
}

struct VulkanFramebuffer {
    core: Arc<VulkanCore>,
    raw: vk::Framebuffer,
    _views: Vec<VulkanTextureView>,
    _pass: Arc<VulkanRenderPassObject>,
}

impl Drop for VulkanFramebuffer {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_framebuffer(self.raw, None) };
    }
}

pub(super) fn collect_finished(shared: &Arc<VulkanShared>) {
    let mut in_flight = shared
        .in_flight
        .lock()
        .expect("Vulkan in-flight mutex is not poisoned");
    let mut index = 0;
    while index < in_flight.len() {
        let status = unsafe { shared.core.device.get_fence_status(in_flight[index].fence) };
        if status == Ok(true) {
            let finished = in_flight.swap_remove(index);
            unsafe {
                finished.core.device.destroy_fence(finished.fence, None);
                for pool in finished.pools {
                    finished.core.device.destroy_command_pool(pool, None);
                }
                for semaphore in finished.semaphores {
                    finished.core.device.destroy_semaphore(semaphore, None);
                }
            }
        } else {
            index += 1;
        }
    }
}

pub(super) fn collect_all(shared: &Arc<VulkanShared>) {
    let mut in_flight = shared
        .in_flight
        .lock()
        .expect("Vulkan in-flight mutex is not poisoned");
    for finished in in_flight.drain(..) {
        unsafe {
            finished.core.device.destroy_fence(finished.fence, None);
            for pool in finished.pools {
                finished.core.device.destroy_command_pool(pool, None);
            }
            for semaphore in finished.semaphores {
                finished.core.device.destroy_semaphore(semaphore, None);
            }
        }
    }
}

fn texture_sample_count(texture: &VulkanTexture) -> vk::SampleCountFlags {
    super::sample_count(texture.0.sample_count)
}

fn load_op<T: Copy>(operation: LoadOp<T>) -> vk::AttachmentLoadOp {
    match operation {
        LoadOp::Load => vk::AttachmentLoadOp::LOAD,
        LoadOp::Clear(_) => vk::AttachmentLoadOp::CLEAR,
    }
}

fn store_op(operation: StoreOp) -> vk::AttachmentStoreOp {
    match operation {
        StoreOp::Store => vk::AttachmentStoreOp::STORE,
        StoreOp::Discard => vk::AttachmentStoreOp::DONT_CARE,
    }
}

fn texture_aspect(format: vk::Format, requested: TextureAspect) -> vk::ImageAspectFlags {
    match requested {
        TextureAspect::DepthOnly => vk::ImageAspectFlags::DEPTH,
        TextureAspect::StencilOnly => vk::ImageAspectFlags::STENCIL,
        TextureAspect::All => super::image_aspect(format),
    }
}

fn layout_source(layout: vk::ImageLayout) -> (vk::PipelineStageFlags, vk::AccessFlags) {
    match layout {
        vk::ImageLayout::UNDEFINED => (
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::AccessFlags::empty(),
        ),
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => (
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::TRANSFER_WRITE,
        ),
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => (
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::TRANSFER_READ,
        ),
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => (
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::AccessFlags::SHADER_READ,
        ),
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => (
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        ),
        vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL => (
            vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
            vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
        ),
        vk::ImageLayout::PRESENT_SRC_KHR => (
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::AccessFlags::MEMORY_READ,
        ),
        _ => (
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
        ),
    }
}

fn layout_destination(layout: vk::ImageLayout) -> (vk::PipelineStageFlags, vk::AccessFlags) {
    match layout {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => (
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::TRANSFER_WRITE,
        ),
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => (
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::TRANSFER_READ,
        ),
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => (
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::AccessFlags::SHADER_READ,
        ),
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => (
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        ),
        vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL => (
            vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
            vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
        ),
        vk::ImageLayout::PRESENT_SRC_KHR => (
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::AccessFlags::MEMORY_READ,
        ),
        _ => (
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
        ),
    }
}
