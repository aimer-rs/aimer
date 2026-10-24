use std::ffi::CString;
use std::sync::Arc;

use ash::vk;

use super::*;
use crate::vulkan_descriptor_kind::{DescriptorKind, descriptor_kind};

pub(super) struct VulkanRenderPassObject {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::RenderPass,
}

impl Drop for VulkanRenderPassObject {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_render_pass(self.raw, None) };
    }
}

pub(super) fn create_sampler(core: &Arc<VulkanCore>, desc: &SamplerDescriptor) -> VulkanSampler {
    let anisotropy =
        desc.max_anisotropy > 1 && core.enabled_features.sampler_anisotropy == vk::TRUE;
    let create_info = vk::SamplerCreateInfo::default()
        .mag_filter(filter_mode(desc.mag_filter))
        .min_filter(filter_mode(desc.min_filter))
        .mipmap_mode(mipmap_filter_mode(desc.mipmap_filter))
        .address_mode_u(address_mode(desc.address_mode_u))
        .address_mode_v(address_mode(desc.address_mode_v))
        .address_mode_w(address_mode(desc.address_mode_w))
        .mip_lod_bias(0.0)
        .anisotropy_enable(anisotropy)
        .max_anisotropy(if anisotropy {
            desc.max_anisotropy as f32
        } else {
            1.0
        })
        .compare_enable(desc.compare.is_some())
        .compare_op(
            desc.compare
                .map(compare_function)
                .unwrap_or(vk::CompareOp::ALWAYS),
        )
        .min_lod(desc.lod_min_clamp)
        .max_lod(desc.lod_max_clamp)
        .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK)
        .unnormalized_coordinates(false);
    let raw = unsafe {
        core.device
            .create_sampler(&create_info, None)
            .expect("create Vulkan sampler")
    };
    VulkanSampler(Arc::new(VulkanSamplerInner {
        core: core.clone(),
        raw,
    }))
}

pub(super) fn create_bind_group_layout(
    core: &Arc<VulkanCore>,
    entries: &[BindGroupLayoutEntry],
) -> VulkanBindGroupLayout {
    let bindings: Vec<_> = entries
        .iter()
        .map(|entry| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(entry.binding)
                .descriptor_type(binding_descriptor_type(&entry.ty))
                .descriptor_count(entry.count.map_or(1, |count| count.get()))
                .stage_flags(shader_stages(&entry.visibility))
        })
        .collect();
    let create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    let raw = unsafe {
        core.device
            .create_descriptor_set_layout(&create_info, None)
            .expect("create Vulkan descriptor set layout")
    };
    VulkanBindGroupLayout(Arc::new(VulkanBindGroupLayoutInner {
        core: core.clone(),
        raw,
        entries: entries.to_vec(),
    }))
}

pub(super) fn create_bind_group(
    core: &Arc<VulkanCore>,
    layout: &VulkanBindGroupLayout,
    entries: &[BindGroupEntry<VulkanBackend>],
) -> VulkanBindGroup {
    let mut pool_sizes: Vec<vk::DescriptorPoolSize> = Vec::new();
    for entry in &layout.0.entries {
        let descriptor_type = binding_descriptor_type(&entry.ty);
        let descriptor_count = entry.count.map_or(1, |count| count.get());
        if let Some(existing) = pool_sizes
            .iter_mut()
            .find(|size| size.ty == descriptor_type)
        {
            existing.descriptor_count += descriptor_count;
        } else {
            pool_sizes.push(
                vk::DescriptorPoolSize::default()
                    .ty(descriptor_type)
                    .descriptor_count(descriptor_count),
            );
        }
    }
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(&pool_sizes);
    let pool = unsafe {
        core.device
            .create_descriptor_pool(&pool_info, None)
            .expect("create Vulkan descriptor pool")
    };
    let set_layouts = [layout.0.raw];
    let allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&set_layouts);
    let raw = unsafe {
        core.device
            .allocate_descriptor_sets(&allocate_info)
            .expect("allocate Vulkan descriptor set")[0]
    };

    let mut buffer_infos = Vec::with_capacity(entries.len());
    let mut image_infos = Vec::with_capacity(entries.len());
    let mut writes = Vec::with_capacity(entries.len());
    let mut write_refs = Vec::with_capacity(entries.len());
    let mut buffers = Vec::new();
    let mut textures = Vec::new();
    let mut samplers = Vec::new();
    for entry in entries {
        let binding = layout
            .0
            .entries
            .iter()
            .find(|item| item.binding == entry.binding)
            .expect("Vulkan bind group entry must exist in its layout");
        assert_eq!(
            binding.count.map_or(1, |count| count.get()),
            1,
            "Cupid Vulkan bind groups require one resource per binding"
        );
        let descriptor_type = binding_descriptor_type(&binding.ty);
        match &entry.resource {
            BindingResource::Buffer(buffer) => {
                buffer_infos.push(
                    vk::DescriptorBufferInfo::default()
                        .buffer(buffer.0.raw)
                        .offset(0)
                        .range(buffer.0.size),
                );
                write_refs.push((entry.binding, descriptor_type, true, buffer_infos.len() - 1));
                buffers.push((**buffer).clone());
            }
            BindingResource::BufferRange(buffer, offset, size) => {
                buffer_infos.push(
                    vk::DescriptorBufferInfo::default()
                        .buffer(buffer.0.raw)
                        .offset(*offset)
                        .range(*size),
                );
                write_refs.push((entry.binding, descriptor_type, true, buffer_infos.len() - 1));
                buffers.push((**buffer).clone());
            }
            BindingResource::TextureView(view) => {
                image_infos.push(
                    vk::DescriptorImageInfo::default()
                        .image_view(view.0.raw)
                        .image_layout(
                            if matches!(&binding.ty, BindingType::StorageTexture { .. }) {
                                vk::ImageLayout::GENERAL
                            } else {
                                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                            },
                        ),
                );
                write_refs.push((entry.binding, descriptor_type, false, image_infos.len() - 1));
                textures.push((**view).clone());
            }
            BindingResource::Sampler(sampler) => {
                image_infos.push(vk::DescriptorImageInfo::default().sampler(sampler.0.raw));
                write_refs.push((entry.binding, descriptor_type, false, image_infos.len() - 1));
                samplers.push((**sampler).clone());
            }
        }
    }
    for (binding, descriptor_type, is_buffer, info_index) in write_refs {
        let write = vk::WriteDescriptorSet::default()
            .dst_set(raw)
            .dst_binding(binding)
            .descriptor_type(descriptor_type);
        writes.push(if is_buffer {
            write.buffer_info(std::slice::from_ref(&buffer_infos[info_index]))
        } else {
            write.image_info(std::slice::from_ref(&image_infos[info_index]))
        });
    }
    unsafe { core.device.update_descriptor_sets(&writes, &[]) };
    VulkanBindGroup(Arc::new(VulkanBindGroupInner {
        core: core.clone(),
        pool,
        raw,
        _layout: (*layout).clone(),
        _buffers: buffers,
        _textures: textures,
        _samplers: samplers,
    }))
}

pub(super) fn create_pipeline_layout(
    core: &Arc<VulkanCore>,
    layouts: &[&VulkanBindGroupLayout],
) -> VulkanPipelineLayout {
    let raw_layouts: Vec<_> = layouts.iter().map(|layout| layout.0.raw).collect();
    let create_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&raw_layouts);
    let raw = unsafe {
        core.device
            .create_pipeline_layout(&create_info, None)
            .expect("create Vulkan pipeline layout")
    };
    VulkanPipelineLayout(Arc::new(VulkanPipelineLayoutInner {
        core: core.clone(),
        raw,
        _layouts: layouts.iter().map(|layout| (**layout).clone()).collect(),
    }))
}

pub(super) fn create_render_pipeline(
    core: &Arc<VulkanCore>,
    desc: &RenderPipelineDescriptor<VulkanBackend>,
) -> Result<VulkanRenderPipeline, BackendError> {
    let layout = if let Some(layout) = desc.layout {
        (*layout).clone()
    } else {
        create_pipeline_layout(core, &[])
    };
    let compatible_render_pass = create_compatible_render_pass(core, desc)?;
    let vertex_entry = CString::new(desc.vertex.entry_point).map_err(|error| BackendError {
        operation: "create_render_pipeline",
        message: format!("vertex entry point contains NUL: {error}"),
    })?;
    let vertex_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(desc.vertex.module.0.raw)
        .name(&vertex_entry);
    let mut shader_stages = vec![vertex_stage];
    let fragment_entry = desc
        .fragment
        .as_ref()
        .map(|fragment| {
            CString::new(fragment.entry_point).map_err(|error| BackendError {
                operation: "create_render_pipeline",
                message: format!("fragment entry point contains NUL: {error}"),
            })
        })
        .transpose()?;
    if let (Some(fragment), Some(entry)) = (&desc.fragment, fragment_entry.as_ref()) {
        shader_stages.push(
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment.module.0.raw)
                .name(entry),
        );
    }

    let attribute_storage: Vec<Vec<vk::VertexInputAttributeDescription>> = desc
        .vertex
        .buffers
        .iter()
        .map(|layout| {
            layout
                .as_ref()
                .map(|layout| {
                    layout
                        .attributes
                        .iter()
                        .map(|attribute| {
                            vk::VertexInputAttributeDescription::default()
                                .location(attribute.shader_location)
                                .binding(0)
                                .format(vertex_format(attribute.format))
                                .offset(attribute.offset as u32)
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    let mut bindings = Vec::new();
    let mut attributes = Vec::new();
    for (index, layout) in desc.vertex.buffers.iter().enumerate() {
        if let Some(layout) = layout {
            bindings.push(
                vk::VertexInputBindingDescription::default()
                    .binding(index as u32)
                    .stride(layout.array_stride as u32)
                    .input_rate(match layout.step_mode {
                        VertexStepMode::Vertex => vk::VertexInputRate::VERTEX,
                        VertexStepMode::Instance => vk::VertexInputRate::INSTANCE,
                    }),
            );
            for attribute in &attribute_storage[index] {
                attributes.push(vk::VertexInputAttributeDescription {
                    binding: index as u32,
                    ..*attribute
                });
            }
        }
    }
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(primitive_topology(desc.primitive.topology))
        .primitive_restart_enable(desc.primitive.strip_index_format.is_some());
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let mut rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(desc.primitive.unclipped_depth)
        .rasterizer_discard_enable(false)
        .polygon_mode(polygon_mode(desc.primitive.polygon_mode))
        .cull_mode(
            desc.primitive
                .cull_mode
                .map(face)
                .unwrap_or(vk::CullModeFlags::NONE),
        )
        .front_face(match desc.primitive.front_face {
            FrontFace::Ccw => vk::FrontFace::COUNTER_CLOCKWISE,
            FrontFace::Cw => vk::FrontFace::CLOCKWISE,
        })
        .depth_bias_enable(desc.depth_stencil.is_some())
        .line_width(1.0);
    if let Some(depth) = &desc.depth_stencil {
        rasterization = rasterization
            .depth_bias_constant_factor(depth.bias.constant as f32)
            .depth_bias_slope_factor(depth.bias.slope_scale)
            .depth_bias_clamp(depth.bias.clamp);
    }
    if desc.primitive.unclipped_depth && core.enabled_features.depth_clamp != vk::TRUE {
        return Err(BackendError {
            operation: "create_render_pipeline",
            message: "unclipped depth is not supported by this Vulkan device".to_string(),
        });
    }
    if desc.primitive.conservative {
        return Err(BackendError {
            operation: "create_render_pipeline",
            message: "conservative rasterization is not supported by Cupid's Vulkan backend"
                .to_string(),
        });
    }
    if desc.primitive.polygon_mode != PolygonMode::Fill
        && core.enabled_features.fill_mode_non_solid != vk::TRUE
    {
        return Err(BackendError {
            operation: "create_render_pipeline",
            message: "line and point polygon modes are not supported by this Vulkan device"
                .to_string(),
        });
    }
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(sample_count(desc.multisample.count))
        .sample_shading_enable(false)
        .alpha_to_coverage_enable(desc.multisample.alpha_to_coverage_enabled)
        .alpha_to_one_enable(false);
    let depth_stencil = desc
        .depth_stencil
        .as_ref()
        .map(depth_stencil_state)
        .unwrap_or_default();
    let color_blends: Vec<_> = desc
        .fragment
        .as_ref()
        .map(|fragment| {
            fragment
                .targets
                .iter()
                .map(|target| {
                    target
                        .as_ref()
                        .map(|target| {
                            vk::PipelineColorBlendAttachmentState::default()
                                .blend_enable(target.blend.is_some())
                                .src_color_blend_factor(
                                    target
                                        .blend
                                        .as_ref()
                                        .map(|blend| blend_factor(blend.color.src_factor))
                                        .unwrap_or(vk::BlendFactor::ONE),
                                )
                                .dst_color_blend_factor(
                                    target
                                        .blend
                                        .as_ref()
                                        .map(|blend| blend_factor(blend.color.dst_factor))
                                        .unwrap_or(vk::BlendFactor::ZERO),
                                )
                                .color_blend_op(
                                    target
                                        .blend
                                        .as_ref()
                                        .map(|blend| blend_operation(blend.color.operation))
                                        .unwrap_or(vk::BlendOp::ADD),
                                )
                                .src_alpha_blend_factor(
                                    target
                                        .blend
                                        .as_ref()
                                        .map(|blend| blend_factor(blend.alpha.src_factor))
                                        .unwrap_or(vk::BlendFactor::ONE),
                                )
                                .dst_alpha_blend_factor(
                                    target
                                        .blend
                                        .as_ref()
                                        .map(|blend| blend_factor(blend.alpha.dst_factor))
                                        .unwrap_or(vk::BlendFactor::ZERO),
                                )
                                .alpha_blend_op(
                                    target
                                        .blend
                                        .as_ref()
                                        .map(|blend| blend_operation(blend.alpha.operation))
                                        .unwrap_or(vk::BlendOp::ADD),
                                )
                                .color_write_mask(color_write_mask(target.write_mask))
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .attachments(&color_blends);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let create_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterization)
        .multisample_state(&multisample)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic)
        .layout(layout.0.raw)
        .render_pass(compatible_render_pass.raw)
        .subpass(0);
    let raw = unsafe {
        core.device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[create_info], None)
    }
    .map_err(|(_, error)| BackendError {
        operation: "create_render_pipeline",
        message: format!("Vulkan returned {error:?}"),
    })?[0];
    Ok(VulkanRenderPipeline(Arc::new(VulkanRenderPipelineInner {
        core: core.clone(),
        raw,
        layout,
        _compatible_render_pass: compatible_render_pass,
    })))
}

pub(super) fn create_compatible_render_pass(
    core: &Arc<VulkanCore>,
    desc: &RenderPipelineDescriptor<VulkanBackend>,
) -> Result<Arc<VulkanRenderPassObject>, BackendError> {
    let targets = desc
        .fragment
        .as_ref()
        .map(|fragment| fragment.targets)
        .unwrap_or(&[]);
    let samples = sample_count(desc.multisample.count);
    let attachments: Vec<_> = targets
        .iter()
        .map(|target| {
            vk::AttachmentDescription::default()
                .format(
                    target
                        .as_ref()
                        .map(|target| target.format)
                        .unwrap_or(vk::Format::R8G8B8A8_UNORM),
                )
                .samples(samples)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        })
        .collect();
    let color_refs: Vec<_> = (0..targets.len())
        .map(|index| {
            if targets[index].is_some() {
                vk::AttachmentReference::default()
                    .attachment(index as u32)
                    .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            } else {
                vk::AttachmentReference::default()
                    .attachment(vk::ATTACHMENT_UNUSED)
                    .layout(vk::ImageLayout::UNDEFINED)
            }
        })
        .collect();
    let depth_index = attachments.len();
    let mut attachments = attachments;
    let depth_ref = desc.depth_stencil.as_ref().map(|depth| {
        attachments.push(
            vk::AttachmentDescription::default()
                .format(depth.format)
                .samples(samples)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
        );
        vk::AttachmentReference::default()
            .attachment(depth_index as u32)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
    });
    let mut subpass_desc = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_refs);
    if let Some(depth_ref) = depth_ref.as_ref() {
        subpass_desc = subpass_desc.depth_stencil_attachment(depth_ref);
    }
    let subpass = [subpass_desc];
    let create_info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpass);
    let raw = unsafe { core.device.create_render_pass(&create_info, None) }.map_err(|error| {
        BackendError {
            operation: "create_render_pipeline",
            message: format!("create compatible Vulkan render pass: {error:?}"),
        }
    })?;
    Ok(Arc::new(VulkanRenderPassObject {
        core: core.clone(),
        raw,
    }))
}

fn binding_descriptor_type(ty: &BindingType) -> vk::DescriptorType {
    match descriptor_kind(ty) {
        DescriptorKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
        DescriptorKind::UniformBufferDynamic => vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
        DescriptorKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
        DescriptorKind::StorageBufferDynamic => vk::DescriptorType::STORAGE_BUFFER_DYNAMIC,
        DescriptorKind::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
        DescriptorKind::Sampler => vk::DescriptorType::SAMPLER,
        DescriptorKind::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
    }
}

fn shader_stages(stages: &[ShaderStage]) -> vk::ShaderStageFlags {
    stages
        .iter()
        .fold(vk::ShaderStageFlags::empty(), |flags, stage| {
            flags
                | match stage {
                    ShaderStage::Vertex => vk::ShaderStageFlags::VERTEX,
                    ShaderStage::Fragment => vk::ShaderStageFlags::FRAGMENT,
                    ShaderStage::Compute => vk::ShaderStageFlags::COMPUTE,
                }
        })
}

fn address_mode(mode: AddressMode) -> vk::SamplerAddressMode {
    match mode {
        AddressMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        AddressMode::Repeat => vk::SamplerAddressMode::REPEAT,
        AddressMode::MirrorRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
    }
}

fn filter_mode(mode: FilterMode) -> vk::Filter {
    match mode {
        FilterMode::Nearest => vk::Filter::NEAREST,
        FilterMode::Linear => vk::Filter::LINEAR,
    }
}

fn mipmap_filter_mode(mode: FilterMode) -> vk::SamplerMipmapMode {
    match mode {
        FilterMode::Nearest => vk::SamplerMipmapMode::NEAREST,
        FilterMode::Linear => vk::SamplerMipmapMode::LINEAR,
    }
}

fn compare_function(compare: CompareFunction) -> vk::CompareOp {
    match compare {
        CompareFunction::Never => vk::CompareOp::NEVER,
        CompareFunction::Less => vk::CompareOp::LESS,
        CompareFunction::Equal => vk::CompareOp::EQUAL,
        CompareFunction::LessEqual => vk::CompareOp::LESS_OR_EQUAL,
        CompareFunction::Greater => vk::CompareOp::GREATER,
        CompareFunction::NotEqual => vk::CompareOp::NOT_EQUAL,
        CompareFunction::GreaterEqual => vk::CompareOp::GREATER_OR_EQUAL,
        CompareFunction::Always => vk::CompareOp::ALWAYS,
    }
}

fn vertex_format(format: VertexFormat) -> vk::Format {
    match format {
        VertexFormat::Uint8x2 => vk::Format::R8G8_UINT,
        VertexFormat::Uint8x4 => vk::Format::R8G8B8A8_UINT,
        VertexFormat::Sint8x2 => vk::Format::R8G8_SINT,
        VertexFormat::Sint8x4 => vk::Format::R8G8B8A8_SINT,
        VertexFormat::Unorm8x2 => vk::Format::R8G8_UNORM,
        VertexFormat::Unorm8x4 => vk::Format::R8G8B8A8_UNORM,
        VertexFormat::Snorm8x2 => vk::Format::R8G8_SNORM,
        VertexFormat::Snorm8x4 => vk::Format::R8G8B8A8_SNORM,
        VertexFormat::Uint16x2 => vk::Format::R16G16_UINT,
        VertexFormat::Uint16x4 => vk::Format::R16G16B16A16_UINT,
        VertexFormat::Sint16x2 => vk::Format::R16G16_SINT,
        VertexFormat::Sint16x4 => vk::Format::R16G16B16A16_SINT,
        VertexFormat::Unorm16x2 => vk::Format::R16G16_UNORM,
        VertexFormat::Unorm16x4 => vk::Format::R16G16B16A16_UNORM,
        VertexFormat::Snorm16x2 => vk::Format::R16G16_SNORM,
        VertexFormat::Snorm16x4 => vk::Format::R16G16B16A16_SNORM,
        VertexFormat::Float16x2 => vk::Format::R16G16_SFLOAT,
        VertexFormat::Float16x4 => vk::Format::R16G16B16A16_SFLOAT,
        VertexFormat::Float32 => vk::Format::R32_SFLOAT,
        VertexFormat::Float32x2 => vk::Format::R32G32_SFLOAT,
        VertexFormat::Float32x3 => vk::Format::R32G32B32_SFLOAT,
        VertexFormat::Float32x4 => vk::Format::R32G32B32A32_SFLOAT,
        VertexFormat::Uint32 => vk::Format::R32_UINT,
        VertexFormat::Uint32x2 => vk::Format::R32G32_UINT,
        VertexFormat::Uint32x3 => vk::Format::R32G32B32_UINT,
        VertexFormat::Uint32x4 => vk::Format::R32G32B32A32_UINT,
        VertexFormat::Sint32 => vk::Format::R32_SINT,
        VertexFormat::Sint32x2 => vk::Format::R32G32_SINT,
        VertexFormat::Sint32x3 => vk::Format::R32G32B32_SINT,
        VertexFormat::Sint32x4 => vk::Format::R32G32B32A32_SINT,
    }
}

fn primitive_topology(topology: PrimitiveTopology) -> vk::PrimitiveTopology {
    match topology {
        PrimitiveTopology::PointList => vk::PrimitiveTopology::POINT_LIST,
        PrimitiveTopology::LineList => vk::PrimitiveTopology::LINE_LIST,
        PrimitiveTopology::LineStrip => vk::PrimitiveTopology::LINE_STRIP,
        PrimitiveTopology::TriangleList => vk::PrimitiveTopology::TRIANGLE_LIST,
        PrimitiveTopology::TriangleStrip => vk::PrimitiveTopology::TRIANGLE_STRIP,
    }
}

fn polygon_mode(mode: PolygonMode) -> vk::PolygonMode {
    match mode {
        PolygonMode::Fill => vk::PolygonMode::FILL,
        PolygonMode::Line => vk::PolygonMode::LINE,
        PolygonMode::Point => vk::PolygonMode::POINT,
    }
}

fn face(face: Face) -> vk::CullModeFlags {
    match face {
        Face::Front => vk::CullModeFlags::FRONT,
        Face::Back => vk::CullModeFlags::BACK,
    }
}

fn blend_factor(factor: BlendFactor) -> vk::BlendFactor {
    match factor {
        BlendFactor::Zero => vk::BlendFactor::ZERO,
        BlendFactor::One => vk::BlendFactor::ONE,
        BlendFactor::Src => vk::BlendFactor::SRC_COLOR,
        BlendFactor::OneMinusSrc => vk::BlendFactor::ONE_MINUS_SRC_COLOR,
        BlendFactor::SrcAlpha => vk::BlendFactor::SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        BlendFactor::Dst => vk::BlendFactor::DST_COLOR,
        BlendFactor::OneMinusDst => vk::BlendFactor::ONE_MINUS_DST_COLOR,
        BlendFactor::DstAlpha => vk::BlendFactor::DST_ALPHA,
        BlendFactor::OneMinusDstAlpha => vk::BlendFactor::ONE_MINUS_DST_ALPHA,
        BlendFactor::SrcAlphaSaturated => vk::BlendFactor::SRC_ALPHA_SATURATE,
        BlendFactor::Constant => vk::BlendFactor::CONSTANT_COLOR,
        BlendFactor::OneMinusConstant => vk::BlendFactor::ONE_MINUS_CONSTANT_COLOR,
    }
}

fn blend_operation(operation: BlendOperation) -> vk::BlendOp {
    match operation {
        BlendOperation::Add => vk::BlendOp::ADD,
        BlendOperation::Subtract => vk::BlendOp::SUBTRACT,
        BlendOperation::ReverseSubtract => vk::BlendOp::REVERSE_SUBTRACT,
        BlendOperation::Min => vk::BlendOp::MIN,
        BlendOperation::Max => vk::BlendOp::MAX,
    }
}

fn color_write_mask(mask: ColorWriteMask) -> vk::ColorComponentFlags {
    let mut flags = vk::ColorComponentFlags::empty();
    if mask.red {
        flags |= vk::ColorComponentFlags::R;
    }
    if mask.green {
        flags |= vk::ColorComponentFlags::G;
    }
    if mask.blue {
        flags |= vk::ColorComponentFlags::B;
    }
    if mask.alpha {
        flags |= vk::ColorComponentFlags::A;
    }
    flags
}

fn depth_stencil_state(
    state: &DepthStencilState<vk::Format>,
) -> vk::PipelineDepthStencilStateCreateInfo<'_> {
    vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(state.depth_write_enabled)
        .depth_compare_op(compare_function(state.depth_compare))
        .depth_bounds_test_enable(false)
        .stencil_test_enable(true)
        .front(
            stencil_face_state(state.stencil.front)
                .compare_mask(state.stencil.read_mask)
                .write_mask(state.stencil.write_mask),
        )
        .back(
            stencil_face_state(state.stencil.back)
                .compare_mask(state.stencil.read_mask)
                .write_mask(state.stencil.write_mask),
        )
        .min_depth_bounds(0.0)
        .max_depth_bounds(1.0)
}

fn stencil_face_state(state: StencilFaceState) -> vk::StencilOpState {
    vk::StencilOpState::default()
        .fail_op(stencil_operation(state.fail_op))
        .pass_op(stencil_operation(state.pass_op))
        .depth_fail_op(stencil_operation(state.depth_fail_op))
        .compare_op(compare_function(state.compare))
        .compare_mask(u32::MAX)
        .write_mask(u32::MAX)
        .reference(0)
}

fn stencil_operation(operation: StencilOperation) -> vk::StencilOp {
    match operation {
        StencilOperation::Keep => vk::StencilOp::KEEP,
        StencilOperation::Zero => vk::StencilOp::ZERO,
        StencilOperation::Replace => vk::StencilOp::REPLACE,
        StencilOperation::IncrementClamp => vk::StencilOp::INCREMENT_AND_CLAMP,
        StencilOperation::DecrementClamp => vk::StencilOp::DECREMENT_AND_CLAMP,
        StencilOperation::Invert => vk::StencilOp::INVERT,
        StencilOperation::IncrementWrap => vk::StencilOp::INCREMENT_AND_WRAP,
        StencilOperation::DecrementWrap => vk::StencilOp::DECREMENT_AND_WRAP,
    }
}
