use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CString, c_void};
use std::rc::Rc;

use crate::backend::{
    BackendError, BindGroupLayoutEntry, BindingType, BlendFactor, BlendOperation, BlendState,
    CompareFunction, DepthBiasState, Extent3d, Face, FrontFace, GpuRenderPass, IndexFormat, LoadOp,
    Origin3d, PolygonMode, PrimitiveState, PrimitiveTopology, RenderPassDescriptor,
    RenderPipelineDescriptor, StencilOperation, StencilState, TexelCopyBufferLayout, TextureAspect,
    TextureDimension, VertexAttribute, VertexFormat, VertexStepMode,
};

use super::gl;
use super::{
    FramebufferKey, OpenGlBackend, OpenGlBuffer, OpenGlShared, OpenGlTexture, OpenGlTextureFormat,
    OpenGlTextureView, OpenGlViewTarget,
};

const BINDINGS_PER_GROUP: u32 = 16;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ShaderResource {
    UniformBlock {
        group: u32,
        binding: u32,
        name: String,
    },
    Sampler {
        texture_group: u32,
        texture_binding: u32,
        sampler_group: u32,
        sampler_binding: u32,
        name: String,
    },
    Texture {
        group: u32,
        binding: u32,
        name: String,
    },
}

pub struct OpenGlShaderModule(pub(super) Rc<OpenGlShaderModuleInner>);

pub(super) struct OpenGlShaderModuleInner {
    shared: Rc<OpenGlShared>,
    vertex: Option<u32>,
    fragment: Option<u32>,
    resources: Vec<ShaderResource>,
}

impl Drop for OpenGlShaderModuleInner {
    fn drop(&mut self) {
        if self.shared.context.make_current().is_ok() {
            unsafe {
                if let Some(shader) = self.vertex {
                    (self.shared.gl.delete_shader)(shader);
                }
                if let Some(shader) = self.fragment {
                    (self.shared.gl.delete_shader)(shader);
                }
            }
        }
    }
}

pub(super) struct OpenGlIndexBuffer {
    pub(super) _buffer: OpenGlBuffer,
    pub(super) format: IndexFormat,
    pub(super) offset: u64,
}

pub struct OpenGlCommandEncoder {
    pub(super) shared: Rc<OpenGlShared>,
    pub(super) commands: Vec<OpenGlCommand>,
}

pub(super) enum OpenGlCommand {
    BeginPass(OpenGlPassSetup),
    EndPass(Vec<(u32, u32, u32, (u32, u32))>),
    SetPipeline(OpenGlRenderPipeline),
    SetBindGroup {
        index: u32,
        group: OpenGlBindGroup,
        dynamic_offsets: Vec<u32>,
    },
    SetVertexBuffer {
        slot: u32,
        buffer: OpenGlBuffer,
        offset: u64,
    },
    SetIndexBuffer(OpenGlIndexBuffer),
    SetScissor {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Draw {
        vertices: std::ops::Range<u32>,
        instances: std::ops::Range<u32>,
    },
    DrawIndexed {
        indices: std::ops::Range<u32>,
        base_vertex: i32,
        instances: std::ops::Range<u32>,
    },
    CopyBufferToTexture {
        buffer: OpenGlBuffer,
        texture: super::OpenGlTexture,
        mip_level: u32,
        origin: Origin3d,
        aspect: TextureAspect,
        layout: TexelCopyBufferLayout,
        extent: Extent3d,
    },
    CopyTextureToTexture {
        source: super::OpenGlTexture,
        source_mip: u32,
        source_origin: Origin3d,
        source_aspect: TextureAspect,
        destination: super::OpenGlTexture,
        destination_mip: u32,
        destination_origin: Origin3d,
        destination_aspect: TextureAspect,
        extent: Extent3d,
    },
    CopyTextureToBuffer {
        texture: super::OpenGlTexture,
        mip_level: u32,
        origin: Origin3d,
        aspect: TextureAspect,
        buffer: OpenGlBuffer,
        layout: TexelCopyBufferLayout,
        extent: Extent3d,
    },
}

pub(super) struct OpenGlPassSetup {
    framebuffer: u32,
    size: (u32, u32),
    is_default: bool,
    clear_colors: Vec<Option<[f32; 4]>>,
    clear_depth: Option<f32>,
    clear_stencil: Option<i32>,
    _attachments: Vec<OpenGlTextureView>,
}

pub struct OpenGlRenderPass<'a> {
    pub(super) encoder: &'a mut OpenGlCommandEncoder,
    resolve: Vec<(u32, u32, u32, (u32, u32))>,
    _attachments: Vec<OpenGlTextureView>,
}

pub(super) struct OpenGlExecutorPass {
    shared: Rc<OpenGlShared>,
    pipeline: Option<OpenGlRenderPipeline>,
    index: Option<OpenGlIndexBuffer>,
    vertex_buffers: Vec<Option<(super::OpenGlBuffer, u64)>>,
    extent: (u32, u32),
    _attachments: Vec<OpenGlTextureView>,
}

impl Drop for OpenGlRenderPass<'_> {
    fn drop(&mut self) {
        self.encoder
            .commands
            .push(OpenGlCommand::EndPass(std::mem::take(&mut self.resolve)));
    }
}

impl GpuRenderPass<OpenGlBackend> for OpenGlRenderPass<'_> {
    fn set_pipeline(&mut self, pipeline: &OpenGlRenderPipeline) {
        self.encoder
            .commands
            .push(OpenGlCommand::SetPipeline(pipeline.clone()));
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &OpenGlBindGroup,
        dynamic_offsets: &[u32],
    ) {
        self.encoder.commands.push(OpenGlCommand::SetBindGroup {
            index,
            group: bind_group.clone(),
            dynamic_offsets: dynamic_offsets.to_vec(),
        });
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &OpenGlBuffer, offset: u64) {
        self.encoder.commands.push(OpenGlCommand::SetVertexBuffer {
            slot,
            buffer: buffer.clone(),
            offset,
        });
    }

    fn set_index_buffer(&mut self, buffer: &OpenGlBuffer, format: IndexFormat, offset: u64) {
        self.encoder
            .commands
            .push(OpenGlCommand::SetIndexBuffer(OpenGlIndexBuffer {
                _buffer: buffer.clone(),
                format,
                offset,
            }));
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.encoder.commands.push(OpenGlCommand::SetScissor {
            x,
            y,
            width,
            height,
        });
    }

    fn draw(&mut self, vertices: std::ops::Range<u32>, instances: std::ops::Range<u32>) {
        self.encoder.commands.push(OpenGlCommand::Draw {
            vertices,
            instances,
        });
    }

    fn draw_indexed(
        &mut self,
        indices: std::ops::Range<u32>,
        base_vertex: i32,
        instances: std::ops::Range<u32>,
    ) {
        self.encoder.commands.push(OpenGlCommand::DrawIndexed {
            indices,
            base_vertex,
            instances,
        });
    }
}

pub(super) fn submit_encoder(shared: &Rc<OpenGlShared>, commands: Vec<OpenGlCommand>) {
    let _ = shared.context.make_current();
    let mut pass: Option<OpenGlExecutorPass> = None;
    for command in commands {
        match command {
            OpenGlCommand::BeginPass(setup) => {
                assert!(pass.is_none(), "nested OpenGL render pass");
                pass = Some(execute_begin_pass(shared, setup));
            }
            OpenGlCommand::EndPass(resolve) => {
                let ended_pass = pass.take().expect("OpenGL render pass end without a begin");
                execute_resolves(shared, &resolve);
                drop(ended_pass);
            }
            OpenGlCommand::SetPipeline(pipeline) => pass
                .as_mut()
                .expect("OpenGL pipeline set outside render pass")
                .set_pipeline(&pipeline),
            OpenGlCommand::SetBindGroup {
                index,
                group,
                dynamic_offsets,
            } => pass
                .as_mut()
                .expect("OpenGL bind group set outside render pass")
                .set_bind_group(index, &group, &dynamic_offsets),
            OpenGlCommand::SetVertexBuffer {
                slot,
                buffer,
                offset,
            } => pass
                .as_mut()
                .expect("OpenGL vertex buffer set outside render pass")
                .set_vertex_buffer(slot, &buffer, offset),
            OpenGlCommand::SetIndexBuffer(index) => {
                pass.as_mut()
                    .expect("OpenGL index buffer set outside render pass")
                    .index = Some(index)
            }
            OpenGlCommand::SetScissor {
                x,
                y,
                width,
                height,
            } => pass
                .as_mut()
                .expect("OpenGL scissor set outside render pass")
                .set_scissor_rect(x, y, width, height),
            OpenGlCommand::Draw {
                vertices,
                instances,
            } => pass
                .as_mut()
                .expect("OpenGL draw outside render pass")
                .draw(vertices, instances),
            OpenGlCommand::DrawIndexed {
                indices,
                base_vertex,
                instances,
            } => pass
                .as_mut()
                .expect("OpenGL indexed draw outside render pass")
                .draw_indexed(indices, base_vertex, instances),
            OpenGlCommand::CopyBufferToTexture {
                buffer,
                texture,
                mip_level,
                origin,
                aspect,
                layout,
                extent,
            } => {
                assert!(
                    pass.is_none(),
                    "OpenGL texture copy recorded inside a render pass"
                );
                super::trait_impl::execute_copy_buffer_to_texture(
                    shared, &buffer, &texture, mip_level, origin, aspect, layout, extent,
                );
            }
            OpenGlCommand::CopyTextureToTexture {
                source,
                source_mip,
                source_origin,
                source_aspect,
                destination,
                destination_mip,
                destination_origin,
                destination_aspect,
                extent,
            } => {
                assert!(
                    pass.is_none(),
                    "OpenGL texture copy recorded inside a render pass"
                );
                super::trait_impl::execute_copy_texture_to_texture(
                    shared,
                    &source,
                    source_mip,
                    source_origin,
                    source_aspect,
                    &destination,
                    destination_mip,
                    destination_origin,
                    destination_aspect,
                    extent,
                );
            }
            OpenGlCommand::CopyTextureToBuffer {
                texture,
                mip_level,
                origin,
                aspect,
                buffer,
                layout,
                extent,
            } => {
                assert!(
                    pass.is_none(),
                    "OpenGL readback recorded inside a render pass"
                );
                super::trait_impl::execute_copy_texture_to_buffer(
                    shared, &texture, mip_level, origin, aspect, &buffer, layout, extent,
                );
            }
        }
    }
    assert!(
        pass.is_none(),
        "OpenGL command encoder ended inside a render pass"
    );
    unsafe { (shared.gl.flush)() };
}

fn execute_begin_pass(shared: &Rc<OpenGlShared>, setup: OpenGlPassSetup) -> OpenGlExecutorPass {
    shared.bind_framebuffer(gl::FRAMEBUFFER, setup.framebuffer);
    shared.set_viewport(0, 0, setup.size.0 as i32, setup.size.1 as i32);
    shared.set_capability(gl::SCISSOR_TEST, false);
    unsafe {
        for (index, clear) in setup.clear_colors.iter().enumerate() {
            if let Some(clear) = clear {
                if setup.is_default {
                    (shared.gl.clear_color)(clear[0], clear[1], clear[2], clear[3]);
                    (shared.gl.clear)(gl::COLOR_BUFFER_BIT);
                } else {
                    (shared.gl.clear_buffer_fv)(gl::COLOR, index as i32, clear.as_ptr());
                }
            }
        }
        if let Some(depth) = setup.clear_depth {
            (shared.gl.clear_buffer_fv)(gl::DEPTH, 0, &depth);
        }
        if let Some(stencil) = setup.clear_stencil {
            (shared.gl.clear_buffer_iv)(gl::STENCIL, 0, &stencil);
        }
    }
    OpenGlExecutorPass {
        shared: shared.clone(),
        pipeline: None,
        index: None,
        vertex_buffers: Vec::new(),
        extent: setup.size,
        _attachments: setup._attachments,
    }
}

fn execute_resolves(shared: &Rc<OpenGlShared>, resolve: &[(u32, u32, u32, (u32, u32))]) {
    for (read, draw, color_index, size) in resolve {
        shared.bind_framebuffer(gl::READ_FRAMEBUFFER, *read);
        unsafe { (shared.gl.read_buffer)(gl::COLOR_ATTACHMENT0 + color_index) };
        shared.bind_framebuffer(gl::DRAW_FRAMEBUFFER, *draw);
        unsafe {
            (shared.gl.blit_framebuffer)(
                0,
                0,
                size.0 as i32,
                size.1 as i32,
                0,
                0,
                size.0 as i32,
                size.1 as i32,
                gl::COLOR_BUFFER_BIT,
                gl::NEAREST,
            );
        }
    }
    shared.bind_framebuffer(gl::FRAMEBUFFER, 0);
}

impl GpuRenderPass<OpenGlBackend> for OpenGlExecutorPass {
    fn set_pipeline(&mut self, pipeline: &OpenGlRenderPipeline) {
        let shared = &self.shared;
        let _ = shared.context.make_current();
        let state = &pipeline.0;
        shared.use_program(state.program);
        shared.bind_vertex_array(state.vertex_array);
        unsafe {
            (shared.gl.front_face)(match state.primitive.front_face {
                FrontFace::Ccw => gl::CCW,
                FrontFace::Cw => gl::CW,
            });
            if let Some(face) = state.primitive.cull_mode {
                shared.set_capability(gl::CULL_FACE, true);
                (shared.gl.cull_face)(match face {
                    Face::Front => gl::FRONT,
                    Face::Back => gl::BACK,
                });
            } else {
                shared.set_capability(gl::CULL_FACE, false);
            }
            (shared.gl.polygon_mode)(
                gl::FRONT_AND_BACK,
                match state.primitive.polygon_mode {
                    PolygonMode::Fill => gl::FILL,
                    PolygonMode::Line => gl::LINE,
                    PolygonMode::Point => gl::POINT,
                },
            );
            if let Some(blend) = &state.blend {
                shared.set_capability(gl::BLEND, true);
                if matches!(
                    blend.color.src_factor,
                    BlendFactor::Constant | BlendFactor::OneMinusConstant
                ) || matches!(
                    blend.color.dst_factor,
                    BlendFactor::Constant | BlendFactor::OneMinusConstant
                ) || matches!(
                    blend.alpha.src_factor,
                    BlendFactor::Constant | BlendFactor::OneMinusConstant
                ) || matches!(
                    blend.alpha.dst_factor,
                    BlendFactor::Constant | BlendFactor::OneMinusConstant
                ) {
                    (shared.gl.blend_color)(0.0, 0.0, 0.0, 0.0);
                }
                (shared.gl.blend_func_separate)(
                    blend_factor(blend.color.src_factor),
                    blend_factor(blend.color.dst_factor),
                    blend_factor(blend.alpha.src_factor),
                    blend_factor(blend.alpha.dst_factor),
                );
                (shared.gl.blend_equation_separate)(
                    blend_operation(blend.color.operation),
                    blend_operation(blend.alpha.operation),
                );
            } else {
                shared.set_capability(gl::BLEND, false);
            }
            let mask = state.write_mask;
            (shared.gl.color_mask_i)(
                0,
                mask.red.into(),
                mask.green.into(),
                mask.blue.into(),
                mask.alpha.into(),
            );
            if let Some(depth) = &state.depth {
                shared.set_capability(gl::DEPTH_TEST, true);
                (shared.gl.depth_mask)(depth.depth_write_enabled.into());
                (shared.gl.depth_func)(compare_function(depth.depth_compare));
                shared.set_capability(gl::STENCIL_TEST, true);
                for face in [gl::FRONT, gl::BACK] {
                    let stencil = if face == gl::FRONT {
                        depth.stencil.front
                    } else {
                        depth.stencil.back
                    };
                    (shared.gl.stencil_func_separate)(
                        face,
                        compare_function(stencil.compare),
                        0,
                        depth.stencil.read_mask,
                    );
                    (shared.gl.stencil_op_separate)(
                        face,
                        stencil_operation(stencil.fail_op),
                        stencil_operation(stencil.depth_fail_op),
                        stencil_operation(stencil.pass_op),
                    );
                    (shared.gl.stencil_mask_separate)(face, depth.stencil.write_mask);
                }
                if depth.bias.constant != 0 || depth.bias.slope_scale != 0.0 {
                    shared.set_capability(gl::POLYGON_OFFSET_FILL, true);
                    (shared.gl.polygon_offset)(depth.bias.slope_scale, depth.bias.constant as f32);
                } else {
                    shared.set_capability(gl::POLYGON_OFFSET_FILL, false);
                }
            } else {
                shared.set_capability(gl::DEPTH_TEST, false);
                shared.set_capability(gl::STENCIL_TEST, false);
                (shared.gl.depth_mask)(gl::FALSE);
                shared.set_capability(gl::POLYGON_OFFSET_FILL, false);
            }
            if state.multisample_count > 1 {
                shared.set_capability(gl::MULTISAMPLE, true);
            } else {
                shared.set_capability(gl::MULTISAMPLE, false);
            }
            if state.alpha_to_coverage {
                shared.set_capability(gl::SAMPLE_ALPHA_TO_COVERAGE, true);
            } else {
                shared.set_capability(gl::SAMPLE_ALPHA_TO_COVERAGE, false);
            }
        }
        self.vertex_buffers.resize(state.vertex_slots.len(), None);
        self.pipeline = Some(pipeline.clone());
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &OpenGlBindGroup,
        dynamic_offsets: &[u32],
    ) {
        let Some(pipeline) = self.pipeline.as_ref() else {
            panic!("OpenGL bind group set before a render pipeline");
        };
        let shared = &self.shared;
        let mut dynamic_index = 0;
        let _ = shared.context.make_current();
        for entry in &bind_group.0.layout.0.entries {
            let dynamic_offset = match &entry.ty {
                BindingType::Buffer {
                    has_dynamic_offset: true,
                    ..
                } => {
                    let offset = *dynamic_offsets
                        .get(dynamic_index)
                        .expect("missing OpenGL dynamic offset");
                    dynamic_index += 1;
                    offset as u64
                }
                _ => 0,
            };
            let resource = bind_group
                .0
                .resources
                .get(&entry.binding)
                .unwrap_or_else(|| panic!("missing OpenGL bind-group binding {}", entry.binding));
            match resource {
                OpenGlBoundResource::Buffer(buffer) => {
                    if let Some(block) = pipeline
                        .0
                        .uniform_blocks
                        .iter()
                        .find(|block| block.group == index && block.binding == entry.binding)
                    {
                        let size = buffer.0.size.saturating_sub(dynamic_offset);
                        shared.bind_uniform_buffer(
                            block.binding_point,
                            buffer.0.raw,
                            dynamic_offset as isize,
                            size as isize,
                        );
                    }
                }
                OpenGlBoundResource::BufferRange(buffer, offset, size) => {
                    if let Some(block) = pipeline
                        .0
                        .uniform_blocks
                        .iter()
                        .find(|block| block.group == index && block.binding == entry.binding)
                    {
                        shared.bind_uniform_buffer(
                            block.binding_point,
                            buffer.0.raw,
                            (offset + dynamic_offset) as isize,
                            *size as isize,
                        );
                    }
                }
                OpenGlBoundResource::TextureView(view) => {
                    if let Some(mapping) =
                        pipeline.0.textures.iter().find(|mapping| {
                            mapping.group == index && mapping.binding == entry.binding
                        })
                    {
                        if let OpenGlViewTarget::Texture { texture, .. } = &view.0.target {
                            shared.bind_texture(
                                mapping.texture_unit,
                                texture.0.target,
                                texture.0.raw,
                            );
                        }
                    }
                }
                OpenGlBoundResource::Sampler(sampler) => {
                    if let Some(mapping) = pipeline.0.samplers.iter().find(|mapping| {
                        mapping.sampler_group == index && mapping.sampler_binding == entry.binding
                    }) {
                        shared.bind_sampler(mapping.texture_unit, sampler.0.raw);
                    }
                }
            }
        }
        assert_eq!(
            dynamic_index,
            dynamic_offsets.len(),
            "too many OpenGL dynamic offsets"
        );
        shared.active_texture(0);
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &OpenGlBuffer, offset: u64) {
        let Some(pipeline) = self.pipeline.as_ref() else {
            panic!("OpenGL vertex buffer set before a render pipeline");
        };
        let Some(Some(layout)) = pipeline.0.vertex_slots.get(slot as usize) else {
            return;
        };
        self.vertex_buffers
            .resize(pipeline.0.vertex_slots.len(), None);
        self.vertex_buffers[slot as usize] = Some((buffer.clone(), offset));
        bind_vertex_buffer(&self.shared, layout, buffer, offset);
    }

    fn set_index_buffer(&mut self, buffer: &OpenGlBuffer, index_format: IndexFormat, offset: u64) {
        unsafe { (self.shared.gl.bind_buffer)(gl::ELEMENT_ARRAY_BUFFER, buffer.0.raw) };
        self.index = Some(OpenGlIndexBuffer {
            _buffer: buffer.clone(),
            format: index_format,
            offset,
        });
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let gl_y = self.extent.1.saturating_sub(y.saturating_add(height));
        self.shared.set_capability(gl::SCISSOR_TEST, true);
        self.shared
            .set_scissor(x as i32, gl_y as i32, width as i32, height as i32);
    }

    fn draw(&mut self, vertices: std::ops::Range<u32>, instances: std::ops::Range<u32>) {
        let Some(pipeline) = self.pipeline.as_ref() else {
            panic!("OpenGL draw called before a render pipeline");
        };
        self.apply_instance_base(instances.start);
        unsafe {
            (self.shared.gl.draw_arrays_instanced)(
                topology_for_draw(pipeline),
                vertices.start as i32,
                vertices.end.saturating_sub(vertices.start) as i32,
                instances.end.saturating_sub(instances.start) as i32,
            );
        }
    }

    fn draw_indexed(
        &mut self,
        indices: std::ops::Range<u32>,
        base_vertex: i32,
        instances: std::ops::Range<u32>,
    ) {
        let Some(pipeline) = self.pipeline.as_ref() else {
            panic!("OpenGL indexed draw called before a render pipeline");
        };
        let Some(index) = self.index.as_ref() else {
            panic!("OpenGL indexed draw called without an index buffer");
        };
        self.apply_instance_base(instances.start);
        let (format, bytes) = index_format(index.format);
        let byte_offset = index.offset + indices.start as u64 * bytes;
        unsafe {
            (self.shared.gl.draw_elements_instanced_base_vertex)(
                topology_for_draw(pipeline),
                indices.end.saturating_sub(indices.start) as i32,
                format,
                byte_offset as usize as *const c_void,
                instances.end.saturating_sub(instances.start) as i32,
                base_vertex,
            );
        }
    }
}

impl OpenGlExecutorPass {
    fn apply_instance_base(&self, first_instance: u32) {
        if first_instance == 0 {
            return;
        }
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        for (slot, bound) in self.vertex_buffers.iter().enumerate() {
            let (Some((buffer, offset)), Some(Some(layout))) =
                (bound, pipeline.0.vertex_slots.get(slot))
            else {
                continue;
            };
            if layout.step_mode == VertexStepMode::Instance {
                let advanced = offset.saturating_add(first_instance as u64 * layout.stride);
                bind_vertex_buffer(&self.shared, layout, buffer, advanced);
            }
        }
    }
}

fn bind_vertex_buffer(
    shared: &OpenGlShared,
    layout: &VertexBufferSlot,
    buffer: &OpenGlBuffer,
    base_offset: u64,
) {
    unsafe { (shared.gl.bind_buffer)(gl::ARRAY_BUFFER, buffer.0.raw) };
    for attribute in &layout.attributes {
        let (components, format, normalized, integer) = vertex_format(attribute.format);
        let offset = base_offset + attribute.offset;
        unsafe {
            (shared.gl.enable_vertex_attrib_array)(attribute.shader_location);
            if integer {
                (shared.gl.vertex_attrib_i_pointer)(
                    attribute.shader_location,
                    components,
                    format,
                    layout.stride as i32,
                    offset as usize as *const c_void,
                );
            } else {
                (shared.gl.vertex_attrib_pointer)(
                    attribute.shader_location,
                    components,
                    format,
                    normalized.into(),
                    layout.stride as i32,
                    offset as usize as *const c_void,
                );
            }
            (shared.gl.vertex_attrib_divisor)(
                attribute.shader_location,
                u32::from(layout.step_mode == VertexStepMode::Instance),
            );
        }
    }
}

pub(super) fn begin_render_pass<'a>(
    shared: &Rc<OpenGlShared>,
    encoder: &'a mut OpenGlCommandEncoder,
    desc: &RenderPassDescriptor<'_, OpenGlBackend>,
) -> Result<OpenGlRenderPass<'a>, BackendError> {
    let framebuffer = framebuffer_for_pass(shared, desc)?;
    let mut resolve = Vec::new();
    let mut keepalive: Vec<OpenGlTextureView> = desc
        .color_attachments
        .iter()
        .map(|color| color.view.clone())
        .collect();
    for (index, color) in desc.color_attachments.iter().enumerate() {
        if let Some(target) = color.resolve_target {
            let draw = framebuffer_for_view(shared, target, gl::COLOR_ATTACHMENT0)?;
            resolve.push((
                framebuffer.framebuffer,
                draw.framebuffer,
                index as u32,
                framebuffer.size,
            ));
            keepalive.push(target.clone());
        }
    }
    if let Some(depth) = &desc.depth_stencil_attachment {
        keepalive.push(depth.view.clone());
    }
    let clear_colors = desc
        .color_attachments
        .iter()
        .map(|color| match color.ops.load {
            LoadOp::Load => None,
            LoadOp::Clear(clear) => Some([
                clear[0] as f32,
                clear[1] as f32,
                clear[2] as f32,
                clear[3] as f32,
            ]),
        })
        .collect();
    let clear_depth = desc
        .depth_stencil_attachment
        .as_ref()
        .and_then(|depth| depth.depth_ops)
        .and_then(|ops| match ops.load {
            LoadOp::Load => None,
            LoadOp::Clear(depth) => Some(depth),
        });
    let clear_stencil = desc
        .depth_stencil_attachment
        .as_ref()
        .and_then(|depth| depth.stencil_ops)
        .and_then(|ops| match ops.load {
            LoadOp::Load => None,
            LoadOp::Clear(stencil) => Some(stencil as i32),
        });
    encoder
        .commands
        .push(OpenGlCommand::BeginPass(OpenGlPassSetup {
            framebuffer: framebuffer.framebuffer,
            size: framebuffer.size,
            is_default: framebuffer.is_default,
            clear_colors,
            clear_depth,
            clear_stencil,
            _attachments: keepalive.clone(),
        }));
    Ok(OpenGlRenderPass {
        encoder,
        resolve,
        _attachments: keepalive,
    })
}

pub(super) fn framebuffer_for_pass(
    shared: &Rc<OpenGlShared>,
    desc: &RenderPassDescriptor<'_, OpenGlBackend>,
) -> Result<FramebufferInfo, BackendError> {
    let mut attachments = Vec::new();
    let mut color_attachments = Vec::new();
    let mut draw_buffers = Vec::new();
    let mut extent = None;
    let mut has_surface = false;
    for (index, attachment) in desc.color_attachments.iter().enumerate() {
        match texture_attachment(attachment.view, gl::COLOR_ATTACHMENT0 + index as u32)? {
            Some((key, size, gl_attachment)) => {
                check_attachment_size(&mut extent, size)?;
                attachments.push(gl_attachment);
                color_attachments.push(key);
                draw_buffers.push(gl::COLOR_ATTACHMENT0 + index as u32);
            }
            None => {
                has_surface = true;
                check_attachment_size(&mut extent, attachment.view.gl_view_size())?;
            }
        }
    }
    if let Some(depth) = &desc.depth_stencil_attachment {
        match texture_attachment(depth.view, gl::DEPTH_ATTACHMENT)? {
            Some((key, size, gl_attachment)) => {
                check_attachment_size(&mut extent, size)?;
                attachments.push(gl_attachment);
                color_attachments.push(key);
            }
            None => {
                has_surface = true;
                check_attachment_size(&mut extent, depth.view.gl_view_size())?;
            }
        }
    }
    let extent = extent.unwrap_or((1, 1));
    if has_surface {
        if !color_attachments.is_empty() {
            return Err(backend_error(
                "begin_render_pass",
                "cannot mix default-framebuffer and texture attachments",
            ));
        }
        if desc.color_attachments.len() > 1 {
            return Err(backend_error(
                "begin_render_pass",
                "the default framebuffer has one color attachment",
            ));
        }
        return Ok(FramebufferInfo {
            framebuffer: 0,
            size: extent,
            is_default: true,
        });
    }
    let key = FramebufferKey {
        attachments: color_attachments,
        color_count: draw_buffers.len(),
    };
    let framebuffer = create_or_get_framebuffer(shared, key, &attachments, &draw_buffers)?;
    Ok(FramebufferInfo {
        framebuffer,
        size: extent,
        is_default: false,
    })
}

pub(super) fn framebuffer_for_view(
    shared: &Rc<OpenGlShared>,
    view: &OpenGlTextureView,
    attachment: u32,
) -> Result<FramebufferInfo, BackendError> {
    let Some((key, size, gl_attachment)) = texture_attachment(view, attachment)? else {
        return Ok(FramebufferInfo {
            framebuffer: 0,
            size: view.gl_view_size(),
            is_default: true,
        });
    };
    let color_attachment =
        attachment >= gl::COLOR_ATTACHMENT0 && attachment < gl::COLOR_ATTACHMENT0 + 16;
    let draw_buffers = if color_attachment {
        vec![attachment]
    } else {
        Vec::new()
    };
    let framebuffer = create_or_get_framebuffer(
        shared,
        FramebufferKey {
            attachments: vec![key],
            color_count: draw_buffers.len(),
        },
        &[gl_attachment],
        &draw_buffers,
    )?;
    Ok(FramebufferInfo {
        framebuffer,
        size,
        is_default: false,
    })
}

fn texture_attachment(
    view: &OpenGlTextureView,
    attachment: u32,
) -> Result<Option<((u32, u32, u32, i32), (u32, u32), (u32, u32, u32, i32))>, BackendError> {
    match &view.0.target {
        OpenGlViewTarget::Surface { .. } => Ok(None),
        OpenGlViewTarget::Texture {
            texture,
            level,
            layer,
        } => {
            let layer_index = layer.map_or(-1, |layer| layer as i32);
            let key = (attachment, texture.0.raw, *level, layer_index);
            let extent = view.gl_view_size();
            Ok(Some((
                key,
                extent,
                (attachment, texture.0.raw, *level, layer_index),
            )))
        }
    }
}

fn create_or_get_framebuffer(
    shared: &Rc<OpenGlShared>,
    key: FramebufferKey,
    attachments: &[(u32, u32, u32, i32)],
    draw_buffers: &[u32],
) -> Result<u32, BackendError> {
    if let Some(framebuffer) = shared.framebuffer_cache.borrow().get(&key).copied() {
        return Ok(framebuffer);
    }
    let mut framebuffer = 0;
    unsafe {
        (shared.gl.gen_framebuffers)(1, &mut framebuffer);
        shared.bind_framebuffer(gl::FRAMEBUFFER, framebuffer);
        for (attachment, texture, level, layer) in attachments {
            if *layer >= 0 {
                (shared.gl.framebuffer_texture_layer)(
                    gl::FRAMEBUFFER,
                    *attachment,
                    *texture,
                    *level as i32,
                    *layer,
                );
            } else {
                (shared.gl.framebuffer_texture)(
                    gl::FRAMEBUFFER,
                    *attachment,
                    *texture,
                    *level as i32,
                );
            }
        }
        if !draw_buffers.is_empty() {
            (shared.gl.draw_buffers)(draw_buffers.len() as i32, draw_buffers.as_ptr());
        } else {
            let none = gl::NONE;
            (shared.gl.draw_buffers)(1, &none);
            (shared.gl.read_buffer)(gl::NONE);
        }
    }
    let status = unsafe { (shared.gl.check_framebuffer_status)(gl::FRAMEBUFFER) };
    if status != gl::FRAMEBUFFER_COMPLETE {
        shared.invalidate_framebuffer(framebuffer);
        unsafe { (shared.gl.delete_framebuffers)(1, &framebuffer) };
        return Err(backend_error(
            "create_framebuffer",
            format!("framebuffer status {status:#x}"),
        ));
    }
    shared
        .framebuffer_cache
        .borrow_mut()
        .insert(key, framebuffer);
    Ok(framebuffer)
}

fn check_attachment_size(
    size: &mut Option<(u32, u32)>,
    next: (u32, u32),
) -> Result<(), BackendError> {
    if let Some(current) = size {
        if *current != next {
            return Err(backend_error(
                "begin_render_pass",
                "OpenGL framebuffer attachment extents differ",
            ));
        }
    } else {
        *size = Some(next);
    }
    Ok(())
}

pub(super) fn subresource_view(
    _shared: &Rc<OpenGlShared>,
    texture: &OpenGlTexture,
    mip_level: u32,
    layer: u32,
) -> OpenGlTextureView {
    OpenGlTextureView(Rc::new(super::OpenGlTextureViewInner {
        target: OpenGlViewTarget::Texture {
            texture: texture.clone(),
            level: mip_level,
            layer: (texture.0.size.2 > 1).then_some(layer),
        },
    }))
}

impl OpenGlShaderModule {
    pub(super) fn create(
        shared: &Rc<OpenGlShared>,
        source: &[u8],
        label: &str,
    ) -> Result<Self, BackendError> {
        let source = std::str::from_utf8(source).map_err(|error| BackendError {
            operation: "create_shader_module",
            message: format!("{label} is not UTF-8 GLSL package data: {error}"),
        })?;
        let (vertex_source, fragment_source, resources) =
            parse_shader_package(source).map_err(|error| BackendError {
                operation: "create_shader_module",
                message: format!("{label}: {error}"),
            })?;
        shared
            .context
            .make_current()
            .map_err(|error| BackendError {
                operation: "create_shader_module",
                message: error,
            })?;
        let vertex = if vertex_source.is_empty() {
            None
        } else {
            Some(compile_shader(
                &shared.gl,
                gl::VERTEX_SHADER,
                vertex_source,
                label,
            )?)
        };
        let fragment = if fragment_source.is_empty() {
            None
        } else {
            match compile_shader(&shared.gl, gl::FRAGMENT_SHADER, fragment_source, label) {
                Ok(shader) => Some(shader),
                Err(error) => {
                    if let Some(vertex) = vertex {
                        unsafe { (shared.gl.delete_shader)(vertex) };
                    }
                    return Err(error);
                }
            }
        };
        if vertex.is_none() && fragment.is_none() {
            return Err(BackendError {
                operation: "create_shader_module",
                message: format!("{label} contains no shader stages"),
            });
        }
        Ok(Self(Rc::new(OpenGlShaderModuleInner {
            shared: shared.clone(),
            vertex,
            fragment,
            resources,
        })))
    }
}

fn parse_shader_package(source: &str) -> Result<(&str, &str, Vec<ShaderResource>), String> {
    let source = source
        .strip_prefix("AIMER-OPENGL-SHADER-PACK-1\n")
        .ok_or_else(|| "unknown GLSL shader package header".to_owned())?;
    let (vertex, rest) = source
        .split_once("[FRAGMENT]\n")
        .ok_or_else(|| "GLSL shader package has no fragment section".to_owned())?;
    let vertex = vertex
        .strip_prefix("[VERTEX]\n")
        .ok_or_else(|| "GLSL shader package has no vertex section".to_owned())?;
    let (fragment, resource_text) = rest
        .split_once("[RESOURCES]\n")
        .ok_or_else(|| "GLSL shader package has no resource section".to_owned())?;
    let mut resources = Vec::new();
    for line in resource_text.lines().filter(|line| !line.is_empty()) {
        let parts: Vec<_> = line.split_whitespace().collect();
        let resource = match parts.as_slice() {
            ["UBO", group, binding, name] => ShaderResource::UniformBlock {
                group: group
                    .parse()
                    .map_err(|_| format!("invalid UBO group in `{line}`"))?,
                binding: binding
                    .parse()
                    .map_err(|_| format!("invalid UBO binding in `{line}`"))?,
                name: (*name).to_owned(),
            },
            ["TEXTURE", group, binding, name] => ShaderResource::Texture {
                group: group
                    .parse()
                    .map_err(|_| format!("invalid texture group in `{line}`"))?,
                binding: binding
                    .parse()
                    .map_err(|_| format!("invalid texture binding in `{line}`"))?,
                name: (*name).to_owned(),
            },
            [
                "SAMPLER",
                texture_group,
                texture_binding,
                sampler_group,
                sampler_binding,
                name,
            ] => ShaderResource::Sampler {
                texture_group: texture_group
                    .parse()
                    .map_err(|_| format!("invalid texture group in `{line}`"))?,
                texture_binding: texture_binding
                    .parse()
                    .map_err(|_| format!("invalid texture binding in `{line}`"))?,
                sampler_group: sampler_group
                    .parse()
                    .map_err(|_| format!("invalid sampler group in `{line}`"))?,
                sampler_binding: sampler_binding
                    .parse()
                    .map_err(|_| format!("invalid sampler binding in `{line}`"))?,
                name: (*name).to_owned(),
            },
            _ => return Err(format!("invalid GLSL resource metadata `{line}`")),
        };
        resources.push(resource);
    }
    Ok((vertex.trim(), fragment.trim(), resources))
}

fn compile_shader(
    gl: &gl::GlFns,
    stage: u32,
    source: &str,
    label: &str,
) -> Result<u32, BackendError> {
    #[cfg(target_os = "android")]
    let source = source.replacen(
        "#version 330 core",
        if stage == gl::FRAGMENT_SHADER {
            "#version 300 es\nprecision highp float;\nprecision highp int;"
        } else {
            "#version 300 es"
        },
        1,
    );
    let shader = unsafe { (gl.create_shader)(stage) };
    if shader == 0 {
        return Err(BackendError {
            operation: "create_shader_module",
            message: format!("{label}: glCreateShader returned zero"),
        });
    }
    let Ok(length) = i32::try_from(source.len()) else {
        unsafe { (gl.delete_shader)(shader) };
        return Err(BackendError {
            operation: "create_shader_module",
            message: format!("{label}: shader source exceeds the OpenGL size limit"),
        });
    };
    let source_ptr = source.as_ptr().cast();
    unsafe {
        (gl.shader_source)(shader, 1, &source_ptr, &length);
        (gl.compile_shader)(shader);
    }
    let mut status = 0;
    unsafe { (gl.get_shader_i)(shader, gl::COMPILE_STATUS, &mut status) };
    if status == 0 {
        let message = shader_log(gl, shader);
        unsafe { (gl.delete_shader)(shader) };
        return Err(BackendError {
            operation: "compile_shader",
            message: format!("{label}: {message}"),
        });
    }
    Ok(shader)
}

fn shader_log(gl: &gl::GlFns, shader: u32) -> String {
    let mut length = 0;
    unsafe { (gl.get_shader_i)(shader, gl::INFO_LOG_LENGTH, &mut length) };
    let mut bytes = vec![0_u8; length.max(1) as usize];
    let mut written = 0;
    unsafe {
        (gl.get_shader_info_log)(
            shader,
            bytes.len() as i32,
            &mut written,
            bytes.as_mut_ptr().cast(),
        )
    };
    bytes.truncate(written.max(0) as usize);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn program_log(gl: &gl::GlFns, program: u32) -> String {
    let mut length = 0;
    unsafe { (gl.get_program_i)(program, gl::INFO_LOG_LENGTH, &mut length) };
    let mut bytes = vec![0_u8; length.max(1) as usize];
    let mut written = 0;
    unsafe {
        (gl.get_program_info_log)(
            program,
            bytes.len() as i32,
            &mut written,
            bytes.as_mut_ptr().cast(),
        )
    };
    bytes.truncate(written.max(0) as usize);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[derive(Clone)]
pub(super) struct VertexBufferSlot {
    pub(super) stride: u64,
    pub(super) step_mode: VertexStepMode,
    pub(super) attributes: Vec<VertexAttribute>,
}

#[derive(Clone)]
pub(super) struct SamplerBinding {
    pub(super) sampler_group: u32,
    pub(super) sampler_binding: u32,
    pub(super) texture_unit: u32,
    pub(super) uniform_location: i32,
}

#[derive(Clone)]
pub(super) struct TextureBinding {
    pub(super) group: u32,
    pub(super) binding: u32,
    pub(super) texture_unit: u32,
    pub(super) uniform_location: i32,
}

#[derive(Clone)]
pub(super) struct UniformBlockBinding {
    pub(super) group: u32,
    pub(super) binding: u32,
    pub(super) binding_point: u32,
}

#[derive(Clone)]
pub(super) struct OpenGlDepthState {
    pub(super) depth_write_enabled: bool,
    pub(super) depth_compare: CompareFunction,
    pub(super) stencil: StencilState,
    pub(super) bias: DepthBiasState,
}

#[derive(Clone)]
pub struct OpenGlBindGroupLayout(pub(super) Rc<OpenGlBindGroupLayoutInner>);
pub(super) struct OpenGlBindGroupLayoutInner {
    pub(super) entries: Vec<BindGroupLayoutEntry>,
}

#[derive(Clone)]
pub enum OpenGlBoundResource {
    Buffer(super::OpenGlBuffer),
    BufferRange(super::OpenGlBuffer, u64, u64),
    TextureView(super::OpenGlTextureView),
    Sampler(super::OpenGlSampler),
}

#[derive(Clone)]
pub struct OpenGlBindGroup(pub(super) Rc<OpenGlBindGroupInner>);
pub(super) struct OpenGlBindGroupInner {
    pub(super) layout: OpenGlBindGroupLayout,
    pub(super) resources: BTreeMap<u32, OpenGlBoundResource>,
}

pub struct OpenGlPipelineLayout(pub(super) Rc<OpenGlPipelineLayoutInner>);
pub(super) struct OpenGlPipelineLayoutInner {
    pub(super) groups: Vec<OpenGlBindGroupLayout>,
}

#[derive(Clone)]
pub struct OpenGlRenderPipeline(pub(super) Rc<OpenGlRenderPipelineInner>);
pub(super) struct OpenGlRenderPipelineInner {
    shared: Rc<OpenGlShared>,
    pub(super) program: u32,
    pub(super) vertex_array: u32,
    pub(super) vertex_slots: Vec<Option<VertexBufferSlot>>,
    pub(super) samplers: Vec<SamplerBinding>,
    pub(super) textures: Vec<TextureBinding>,
    pub(super) uniform_blocks: Vec<UniformBlockBinding>,
    pub(super) primitive: PrimitiveState,
    pub(super) depth: Option<OpenGlDepthState>,
    pub(super) blend: Option<BlendState>,
    pub(super) write_mask: crate::backend::ColorWriteMask,
    pub(super) multisample_count: u32,
    pub(super) alpha_to_coverage: bool,
}

impl Drop for OpenGlRenderPipelineInner {
    fn drop(&mut self) {
        if self.shared.context.make_current().is_ok() {
            self.shared.invalidate_program(self.program);
            self.shared.invalidate_vertex_array(self.vertex_array);
            unsafe {
                (self.shared.gl.delete_program)(self.program);
                (self.shared.gl.delete_vertex_arrays)(1, &self.vertex_array);
            }
        }
    }
}

pub(super) fn create_render_pipeline(
    shared: &Rc<OpenGlShared>,
    desc: &RenderPipelineDescriptor<OpenGlBackend>,
) -> Result<OpenGlRenderPipeline, BackendError> {
    if desc.primitive.conservative || desc.primitive.unclipped_depth {
        return Err(backend_error(
            "create_render_pipeline",
            "conservative rasterization and unclipped depth are not supported by OpenGL 3.3",
        ));
    }
    if desc
        .fragment
        .as_ref()
        .is_some_and(|fragment| fragment.targets.iter().flatten().count() > 1)
    {
        return Err(backend_error(
            "create_render_pipeline",
            "multiple color targets are not supported by the OpenGL 3.3 backend",
        ));
    }
    if desc
        .vertex
        .buffers
        .iter()
        .flatten()
        .flat_map(|layout| layout.attributes)
        .any(|attribute| attribute.shader_location >= shared.max_vertex_attributes)
    {
        return Err(backend_error(
            "create_render_pipeline",
            "vertex attribute location exceeds the OpenGL limit",
        ));
    }
    if desc.layout.is_some_and(|layout| {
        layout
            .0
            .groups
            .iter()
            .flat_map(|group| &group.0.entries)
            .any(|entry| {
                matches!(
                    entry.ty,
                    crate::backend::BindingType::Buffer {
                        ty: crate::backend::BufferBindingType::Storage
                            | crate::backend::BufferBindingType::ReadOnlyStorage,
                        ..
                    } | crate::backend::BindingType::StorageTexture { .. }
                )
            })
    }) {
        return Err(backend_error(
            "create_render_pipeline",
            "storage buffers and storage textures require OpenGL 4.3 or an extension",
        ));
    }
    shared
        .context
        .make_current()
        .map_err(|message| backend_error("create_render_pipeline", message))?;
    let program = unsafe { (shared.gl.create_program)() };
    if program == 0 {
        return Err(backend_error(
            "create_render_pipeline",
            "glCreateProgram returned zero",
        ));
    }
    unsafe {
        (shared.gl.attach_shader)(
            program,
            desc.vertex.module.0.vertex.ok_or_else(|| {
                backend_error(
                    "create_render_pipeline",
                    "vertex module has no vertex stage",
                )
            })?,
        );
        if let Some(fragment) = &desc.fragment {
            let shader = fragment.module.0.fragment.ok_or_else(|| {
                backend_error(
                    "create_render_pipeline",
                    "fragment module has no fragment stage",
                )
            })?;
            (shared.gl.attach_shader)(program, shader);
        }
        (shared.gl.link_program)(program);
    }
    let mut link_status = 0;
    unsafe { (shared.gl.get_program_i)(program, gl::LINK_STATUS, &mut link_status) };
    if link_status == 0 {
        let message = program_log(&shared.gl, program);
        unsafe { (shared.gl.delete_program)(program) };
        return Err(backend_error("link_render_pipeline", message));
    }

    let vertex_array = unsafe {
        let mut array = 0;
        (shared.gl.gen_vertex_arrays)(1, &mut array);
        array
    };
    if vertex_array == 0 {
        unsafe { (shared.gl.delete_program)(program) };
        return Err(backend_error(
            "create_render_pipeline",
            "glGenVertexArrays returned zero",
        ));
    }

    let mut uniform_blocks = Vec::new();
    let mut sampler_resources = BTreeSet::new();
    let mut texture_resources = BTreeSet::new();
    for module in std::iter::once(&desc.vertex.module.0)
        .chain(desc.fragment.iter().map(|fragment| &fragment.module.0))
    {
        for resource in &module.resources {
            match resource {
                ShaderResource::UniformBlock {
                    group,
                    binding,
                    name,
                } => {
                    let block_name = CString::new(name.as_str()).map_err(|error| {
                        backend_error("create_render_pipeline", error.to_string())
                    })?;
                    let block_index = unsafe {
                        (shared.gl.get_uniform_block_index)(program, block_name.as_ptr())
                    };
                    if block_index != gl::INVALID_INDEX {
                        let binding_point = group
                            .saturating_mul(BINDINGS_PER_GROUP)
                            .saturating_add(*binding);
                        if binding_point >= shared.max_uniform_buffer_bindings {
                            unsafe {
                                (shared.gl.delete_program)(program);
                                (shared.gl.delete_vertex_arrays)(1, &vertex_array);
                            }
                            return Err(backend_error(
                                "create_render_pipeline",
                                "uniform-buffer binding exceeds the OpenGL limit",
                            ));
                        }
                        unsafe {
                            (shared.gl.uniform_block_binding)(program, block_index, binding_point)
                        };
                        if !uniform_blocks.iter().any(|known: &UniformBlockBinding| {
                            known.group == *group
                                && known.binding == *binding
                                && known.binding_point == binding_point
                        }) {
                            uniform_blocks.push(UniformBlockBinding {
                                group: *group,
                                binding: *binding,
                                binding_point,
                            });
                        }
                    }
                }
                ShaderResource::Sampler {
                    texture_group,
                    texture_binding,
                    sampler_group,
                    sampler_binding,
                    name,
                } => {
                    sampler_resources.insert((
                        *texture_group,
                        *texture_binding,
                        *sampler_group,
                        *sampler_binding,
                        name.clone(),
                    ));
                }
                ShaderResource::Texture {
                    group,
                    binding,
                    name,
                } => {
                    texture_resources.insert((*group, *binding, name.clone()));
                }
            }
        }
    }

    let mut samplers = Vec::new();
    let mut textures = Vec::new();
    let mut next_unit = 0_u32;
    for (texture_group, texture_binding, sampler_group, sampler_binding, name) in sampler_resources
    {
        if next_unit >= shared.max_texture_units {
            unsafe {
                (shared.gl.delete_program)(program);
                (shared.gl.delete_vertex_arrays)(1, &vertex_array);
            }
            return Err(backend_error(
                "create_render_pipeline",
                "sampler count exceeds the OpenGL texture-unit limit",
            ));
        }
        let uniform_name = CString::new(name.as_str())
            .map_err(|error| backend_error("create_render_pipeline", error.to_string()))?;
        let location = unsafe { (shared.gl.get_uniform_location)(program, uniform_name.as_ptr()) };
        if location >= 0 {
            samplers.push(SamplerBinding {
                sampler_group,
                sampler_binding,
                texture_unit: next_unit,
                uniform_location: location,
            });
            textures.push(TextureBinding {
                group: texture_group,
                binding: texture_binding,
                texture_unit: next_unit,
                uniform_location: location,
            });
            next_unit += 1;
        }
    }
    for (group, binding, name) in texture_resources {
        if textures
            .iter()
            .any(|texture| texture.group == group && texture.binding == binding)
        {
            continue;
        }
        if next_unit >= shared.max_texture_units {
            unsafe {
                (shared.gl.delete_program)(program);
                (shared.gl.delete_vertex_arrays)(1, &vertex_array);
            }
            return Err(backend_error(
                "create_render_pipeline",
                "texture count exceeds the OpenGL texture-unit limit",
            ));
        }
        let uniform_name = CString::new(name.as_str())
            .map_err(|error| backend_error("create_render_pipeline", error.to_string()))?;
        let location = unsafe { (shared.gl.get_uniform_location)(program, uniform_name.as_ptr()) };
        if location >= 0 {
            textures.push(TextureBinding {
                group,
                binding,
                texture_unit: next_unit,
                uniform_location: location,
            });
            next_unit += 1;
        }
    }
    shared.use_program(program);
    unsafe {
        for sampler in &samplers {
            (shared.gl.uniform_1i)(sampler.uniform_location, sampler.texture_unit as i32);
        }
        for texture in &textures {
            (shared.gl.uniform_1i)(texture.uniform_location, texture.texture_unit as i32);
        }
    }
    shared.use_program(0);

    let vertex_slots = desc
        .vertex
        .buffers
        .iter()
        .map(|slot| {
            slot.as_ref().map(|layout| VertexBufferSlot {
                stride: layout.array_stride,
                step_mode: layout.step_mode,
                attributes: layout.attributes.to_vec(),
            })
        })
        .collect();
    let (blend, write_mask) = desc
        .fragment
        .as_ref()
        .and_then(|fragment| fragment.targets.iter().flatten().next())
        .map(|target| (target.blend.clone(), target.write_mask))
        .unwrap_or((None, crate::backend::ColorWriteMask::ALL));
    let depth = desc.depth_stencil.as_ref().map(|depth| OpenGlDepthState {
        depth_write_enabled: depth.depth_write_enabled,
        depth_compare: depth.depth_compare,
        stencil: depth.stencil,
        bias: depth.bias,
    });
    Ok(OpenGlRenderPipeline(Rc::new(OpenGlRenderPipelineInner {
        shared: shared.clone(),
        program,
        vertex_array,
        vertex_slots,
        samplers,
        textures,
        uniform_blocks,
        primitive: desc.primitive,
        depth,
        blend,
        write_mask,
        multisample_count: desc.multisample.count,
        alpha_to_coverage: desc.multisample.alpha_to_coverage_enabled,
    })))
}

fn backend_error(operation: &'static str, message: impl Into<String>) -> BackendError {
    BackendError {
        operation,
        message: message.into(),
    }
}

pub(super) struct FramebufferInfo {
    pub(super) framebuffer: u32,
    pub(super) size: (u32, u32),
    pub(super) is_default: bool,
}

pub(super) fn format_info(format: OpenGlTextureFormat) -> (u32, u32, u32, usize) {
    match format {
        OpenGlTextureFormat::R8Unorm => (gl::R8, gl::RED, gl::UNSIGNED_BYTE, 1),
        OpenGlTextureFormat::Rg8Unorm => (gl::RG8, gl::RG, gl::UNSIGNED_BYTE, 2),
        OpenGlTextureFormat::Rgba8Unorm => (gl::RGBA8, gl::RGBA, gl::UNSIGNED_BYTE, 4),
        OpenGlTextureFormat::Rgba8UnormSrgb => (gl::SRGB8_ALPHA8, gl::RGBA, gl::UNSIGNED_BYTE, 4),
        OpenGlTextureFormat::Bgra8Unorm => (gl::RGBA8, gl::BGRA, gl::UNSIGNED_BYTE, 4),
        OpenGlTextureFormat::Bgra8UnormSrgb => (gl::SRGB8_ALPHA8, gl::BGRA, gl::UNSIGNED_BYTE, 4),
        OpenGlTextureFormat::Rgba16Float => (gl::RGBA16F, gl::RGBA, gl::HALF_FLOAT, 8),
        OpenGlTextureFormat::Depth16Unorm => (
            gl::DEPTH_COMPONENT16,
            gl::DEPTH_COMPONENT,
            gl::UNSIGNED_SHORT,
            2,
        ),
        OpenGlTextureFormat::Depth24Plus => (
            gl::DEPTH_COMPONENT24,
            gl::DEPTH_COMPONENT,
            gl::UNSIGNED_INT,
            4,
        ),
        OpenGlTextureFormat::Depth24PlusStencil8 => (
            gl::DEPTH24_STENCIL8,
            gl::DEPTH_STENCIL,
            gl::UNSIGNED_INT_24_8,
            4,
        ),
        OpenGlTextureFormat::Depth32Float => {
            (gl::DEPTH_COMPONENT32F, gl::DEPTH_COMPONENT, gl::FLOAT, 4)
        }
    }
}

pub(super) fn sampler_filter(
    filter: crate::backend::FilterMode,
    mip: crate::backend::FilterMode,
) -> u32 {
    match (filter, mip) {
        (crate::backend::FilterMode::Nearest, crate::backend::FilterMode::Nearest) => {
            gl::NEAREST_MIPMAP_NEAREST
        }
        (crate::backend::FilterMode::Linear, crate::backend::FilterMode::Nearest) => {
            gl::LINEAR_MIPMAP_NEAREST
        }
        (crate::backend::FilterMode::Nearest, crate::backend::FilterMode::Linear) => {
            gl::NEAREST_MIPMAP_LINEAR
        }
        (crate::backend::FilterMode::Linear, crate::backend::FilterMode::Linear) => {
            gl::LINEAR_MIPMAP_LINEAR
        }
    }
}

pub(super) fn address_mode(mode: crate::backend::AddressMode) -> u32 {
    match mode {
        crate::backend::AddressMode::ClampToEdge => gl::CLAMP_TO_EDGE,
        crate::backend::AddressMode::Repeat => gl::REPEAT,
        crate::backend::AddressMode::MirrorRepeat => gl::MIRRORED_REPEAT,
    }
}

pub(super) fn primitive_topology(topology: PrimitiveTopology) -> u32 {
    match topology {
        PrimitiveTopology::PointList => gl::POINTS,
        PrimitiveTopology::LineList => gl::LINES,
        PrimitiveTopology::LineStrip => gl::LINE_STRIP,
        PrimitiveTopology::TriangleList => gl::TRIANGLES,
        PrimitiveTopology::TriangleStrip => gl::TRIANGLE_STRIP,
    }
}

pub(super) fn compare_function(compare: CompareFunction) -> u32 {
    match compare {
        CompareFunction::Never => gl::NEVER,
        CompareFunction::Less => gl::LESS,
        CompareFunction::Equal => gl::EQUAL,
        CompareFunction::LessEqual => gl::LEQUAL,
        CompareFunction::Greater => gl::GREATER,
        CompareFunction::NotEqual => gl::NOTEQUAL,
        CompareFunction::GreaterEqual => gl::GEQUAL,
        CompareFunction::Always => gl::ALWAYS,
    }
}

pub(super) fn blend_factor(factor: BlendFactor) -> u32 {
    match factor {
        BlendFactor::Zero => gl::ZERO,
        BlendFactor::One => gl::ONE,
        BlendFactor::Src => gl::SRC_COLOR,
        BlendFactor::OneMinusSrc => gl::ONE_MINUS_SRC_COLOR,
        BlendFactor::SrcAlpha => gl::SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => gl::ONE_MINUS_SRC_ALPHA,
        BlendFactor::Dst => gl::DST_COLOR,
        BlendFactor::OneMinusDst => gl::ONE_MINUS_DST_COLOR,
        BlendFactor::DstAlpha => gl::DST_ALPHA,
        BlendFactor::OneMinusDstAlpha => gl::ONE_MINUS_DST_ALPHA,
        BlendFactor::SrcAlphaSaturated => gl::SRC_ALPHA_SATURATE,
        BlendFactor::Constant => gl::CONSTANT_COLOR,
        BlendFactor::OneMinusConstant => gl::ONE_MINUS_CONSTANT_COLOR,
    }
}

pub(super) fn blend_operation(operation: BlendOperation) -> u32 {
    match operation {
        BlendOperation::Add => gl::FUNC_ADD,
        BlendOperation::Subtract => gl::FUNC_SUBTRACT,
        BlendOperation::ReverseSubtract => gl::FUNC_REVERSE_SUBTRACT,
        BlendOperation::Min => gl::MIN,
        BlendOperation::Max => gl::MAX,
    }
}

pub(super) fn stencil_operation(operation: StencilOperation) -> u32 {
    match operation {
        StencilOperation::Keep => gl::KEEP,
        StencilOperation::Zero => gl::ZERO,
        StencilOperation::Replace => gl::REPLACE,
        StencilOperation::IncrementClamp => gl::INCR,
        StencilOperation::DecrementClamp => gl::DECR,
        StencilOperation::Invert => gl::INVERT,
        StencilOperation::IncrementWrap => gl::INCR_WRAP,
        StencilOperation::DecrementWrap => gl::DECR_WRAP,
    }
}

pub(super) fn vertex_format(format: VertexFormat) -> (i32, u32, bool, bool) {
    match format {
        VertexFormat::Uint8x2 => (2, gl::UNSIGNED_BYTE, false, true),
        VertexFormat::Uint8x4 => (4, gl::UNSIGNED_BYTE, false, true),
        VertexFormat::Sint8x2 => (2, gl::BYTE, false, true),
        VertexFormat::Sint8x4 => (4, gl::BYTE, false, true),
        VertexFormat::Unorm8x2 => (2, gl::UNSIGNED_BYTE, true, false),
        VertexFormat::Unorm8x4 => (4, gl::UNSIGNED_BYTE, true, false),
        VertexFormat::Snorm8x2 => (2, gl::BYTE, true, false),
        VertexFormat::Snorm8x4 => (4, gl::BYTE, true, false),
        VertexFormat::Uint16x2 => (2, gl::UNSIGNED_SHORT, false, true),
        VertexFormat::Uint16x4 => (4, gl::UNSIGNED_SHORT, false, true),
        VertexFormat::Sint16x2 => (2, gl::SHORT, false, true),
        VertexFormat::Sint16x4 => (4, gl::SHORT, false, true),
        VertexFormat::Unorm16x2 => (2, gl::UNSIGNED_SHORT, true, false),
        VertexFormat::Unorm16x4 => (4, gl::UNSIGNED_SHORT, true, false),
        VertexFormat::Snorm16x2 => (2, gl::SHORT, true, false),
        VertexFormat::Snorm16x4 => (4, gl::SHORT, true, false),
        VertexFormat::Float16x2 => (2, gl::HALF_FLOAT, false, false),
        VertexFormat::Float16x4 => (4, gl::HALF_FLOAT, false, false),
        VertexFormat::Float32 => (1, gl::FLOAT, false, false),
        VertexFormat::Float32x2 => (2, gl::FLOAT, false, false),
        VertexFormat::Float32x3 => (3, gl::FLOAT, false, false),
        VertexFormat::Float32x4 => (4, gl::FLOAT, false, false),
        VertexFormat::Uint32 => (1, gl::UNSIGNED_INT, false, true),
        VertexFormat::Uint32x2 => (2, gl::UNSIGNED_INT, false, true),
        VertexFormat::Uint32x3 => (3, gl::UNSIGNED_INT, false, true),
        VertexFormat::Uint32x4 => (4, gl::UNSIGNED_INT, false, true),
        VertexFormat::Sint32 => (1, gl::INT, false, true),
        VertexFormat::Sint32x2 => (2, gl::INT, false, true),
        VertexFormat::Sint32x3 => (3, gl::INT, false, true),
        VertexFormat::Sint32x4 => (4, gl::INT, false, true),
    }
}

pub(super) fn texture_target(dimension: TextureDimension, size: (u32, u32, u32)) -> u32 {
    match dimension {
        TextureDimension::D1 if size.2 > 1 => gl::TEXTURE_1D_ARRAY,
        TextureDimension::D1 => gl::TEXTURE_1D,
        TextureDimension::D2 if size.2 > 1 => gl::TEXTURE_2D_ARRAY,
        TextureDimension::D2 => gl::TEXTURE_2D,
        TextureDimension::D3 => gl::TEXTURE_3D,
    }
}

pub(super) fn index_format(format: IndexFormat) -> (u32, u64) {
    match format {
        IndexFormat::Uint16 => (gl::UNSIGNED_SHORT, 2),
        IndexFormat::Uint32 => (gl::UNSIGNED_INT, 4),
    }
}

pub(super) fn topology_for_draw(pipeline: &OpenGlRenderPipeline) -> u32 {
    primitive_topology(pipeline.0.primitive.topology)
}
