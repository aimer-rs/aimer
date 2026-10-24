use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use web_sys::{
    WebGl2RenderingContext as Gl, WebGlBuffer, WebGlFramebuffer, WebGlProgram, WebGlSampler,
    WebGlShader, WebGlTexture, WebGlVertexArrayObject,
};

use crate::backend::{
    BackendError, BindGroupLayoutEntry, BindingType, BlendFactor, BlendOperation, ColorWriteMask,
    CompareFunction, DepthBiasState, Face, FrontFace, GpuRenderPass, IndexFormat, LoadOp,
    PolygonMode, PrimitiveState, PrimitiveTopology, RenderPassDescriptor, RenderPipelineDescriptor,
    StencilOperation, StencilState, VertexAttribute, VertexFormat, VertexStepMode,
};

use super::{WebGl2Backend, WebGl2Shared};

const BINDINGS_PER_GROUP: u32 = 16;

/// Texture formats implemented by Cupid's direct browser backend.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WebGl2TextureFormat {
    R8Unorm,
    Rg8Unorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba16Float,
    Depth16Unorm,
    Depth24Plus,
    Depth24PlusStencil8,
    Depth32Float,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct FramebufferKey {
    pub(super) attachments: Vec<(u32, u32, u32, i32)>,
}

#[derive(Clone)]
pub struct WebGl2Buffer(pub(super) Rc<WebGl2BufferInner>);

pub(super) struct WebGl2BufferInner {
    pub(super) shared: Rc<WebGl2Shared>,
    pub(super) objects: Vec<(u32, WebGlBuffer)>,
    pub(super) size: u64,
}

impl WebGl2BufferInner {
    pub(super) fn object_for_target(&self, target: u32) -> &WebGlBuffer {
        self.objects
            .iter()
            .find_map(|(object_target, object)| (*object_target == target).then_some(object))
            .unwrap_or_else(|| panic!("WebGL2 buffer has no object for target {target:#x}"))
    }
}

impl Drop for WebGl2BufferInner {
    fn drop(&mut self) {
        for (_, object) in &self.objects {
            self.shared.gl.delete_buffer(Some(object));
        }
    }
}

#[derive(Clone)]
pub struct WebGl2Texture(pub(super) Rc<WebGl2TextureInner>);

pub(super) struct WebGl2TextureInner {
    pub(super) shared: Rc<WebGl2Shared>,
    pub(super) id: u32,
    pub(super) object: WebGlTexture,
    pub(super) target: u32,
    pub(super) size: (u32, u32, u32),
    pub(super) mip_level_count: u32,
    pub(super) format: WebGl2TextureFormat,
}

impl Drop for WebGl2TextureInner {
    fn drop(&mut self) {
        self.shared.invalidate_framebuffers_for(self.id);
        self.shared.gl.delete_texture(Some(&self.object));
    }
}

#[derive(Clone)]
pub struct WebGl2TextureView(pub(super) Rc<WebGl2TextureViewInner>);

pub(super) struct WebGl2TextureViewInner {
    pub(super) target: WebGl2ViewTarget,
}

pub(super) enum WebGl2ViewTarget {
    Surface(Rc<WebGl2Shared>),
    Texture {
        texture: WebGl2Texture,
        level: u32,
        layer: Option<u32>,
    },
}

impl WebGl2TextureView {
    pub(super) fn surface(shared: Rc<WebGl2Shared>) -> Self {
        Self(Rc::new(WebGl2TextureViewInner {
            target: WebGl2ViewTarget::Surface(shared),
        }))
    }

    pub(super) fn size(&self) -> (u32, u32) {
        match &self.0.target {
            WebGl2ViewTarget::Surface(shared) => (
                shared.gl.drawing_buffer_width().max(0) as u32,
                shared.gl.drawing_buffer_height().max(0) as u32,
            ),
            WebGl2ViewTarget::Texture { texture, level, .. } => (
                (texture.0.size.0 >> *level).max(1),
                (texture.0.size.1 >> *level).max(1),
            ),
        }
    }
}

#[derive(Clone)]
pub struct WebGl2Sampler(pub(super) Rc<WebGl2SamplerInner>);

pub(super) struct WebGl2SamplerInner {
    pub(super) shared: Rc<WebGl2Shared>,
    pub(super) object: WebGlSampler,
}

impl Drop for WebGl2SamplerInner {
    fn drop(&mut self) {
        self.shared.gl.delete_sampler(Some(&self.object));
    }
}

#[derive(Clone)]
pub struct WebGl2BindGroupLayout(pub(super) Rc<WebGl2BindGroupLayoutInner>);

pub(super) struct WebGl2BindGroupLayoutInner {
    pub(super) entries: Vec<BindGroupLayoutEntry>,
}

#[derive(Clone)]
pub struct WebGl2BindGroup(pub(super) Rc<WebGl2BindGroupInner>);

pub(super) struct WebGl2BindGroupInner {
    pub(super) layout: WebGl2BindGroupLayout,
    pub(super) resources: BTreeMap<u32, WebGl2BoundResource>,
}

#[derive(Clone)]
pub(super) enum WebGl2BoundResource {
    Buffer(WebGl2Buffer),
    BufferRange(WebGl2Buffer, u64, u64),
    TextureView(WebGl2TextureView),
    Sampler(WebGl2Sampler),
}

#[derive(Clone)]
pub struct WebGl2PipelineLayout(pub(super) Rc<WebGl2PipelineLayoutInner>);

pub(super) struct WebGl2PipelineLayoutInner {
    pub(super) groups: Vec<WebGl2BindGroupLayout>,
}

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

pub struct WebGl2ShaderModule(pub(super) Rc<WebGl2ShaderModuleInner>);

pub(super) struct WebGl2ShaderModuleInner {
    shared: Rc<WebGl2Shared>,
    pub(super) vertex: Option<WebGlShader>,
    pub(super) fragment: Option<WebGlShader>,
    pub(super) resources: Vec<ShaderResource>,
}

impl Drop for WebGl2ShaderModuleInner {
    fn drop(&mut self) {
        if let Some(shader) = &self.vertex {
            self.shared.gl.delete_shader(Some(shader));
        }
        if let Some(shader) = &self.fragment {
            self.shared.gl.delete_shader(Some(shader));
        }
    }
}

pub struct WebGl2CommandEncoder {
    pub(super) shared: Rc<WebGl2Shared>,
}

pub struct WebGl2RenderPass<'a> {
    pub(super) encoder: &'a mut WebGl2CommandEncoder,
    extent: (u32, u32),
    attachments: [Option<WebGl2TextureView>; 2],
    discard_attachments: Vec<u32>,
    pipeline: Option<WebGl2RenderPipeline>,
    index: Option<WebGl2IndexBuffer>,
    vertex_buffers: Vec<Option<(WebGl2Buffer, u64)>>,
    applied_vertex_offsets: Option<(i32, u32)>,
}

pub(super) struct WebGl2IndexBuffer {
    _buffer: WebGl2Buffer,
    format: IndexFormat,
    offset: u64,
}

#[derive(Clone)]
pub struct WebGl2RenderPipeline(pub(super) Rc<WebGl2RenderPipelineInner>);

pub(super) struct WebGl2RenderPipelineInner {
    pub(super) shared: Rc<WebGl2Shared>,
    pub(super) program: WebGlProgram,
    pub(super) vertex_array: WebGlVertexArrayObject,
    pub(super) vertex_slots: Vec<Option<VertexBufferSlot>>,
    pub(super) samplers: Vec<SamplerBinding>,
    pub(super) textures: Vec<TextureBinding>,
    pub(super) uniform_blocks: Vec<UniformBlockBinding>,
    pub(super) primitive: PrimitiveState,
    pub(super) depth: Option<WebGlDepthState>,
    pub(super) blend: Option<crate::backend::BlendState>,
    pub(super) write_mask: ColorWriteMask,
    pub(super) alpha_to_coverage: bool,
}

impl Drop for WebGl2RenderPipelineInner {
    fn drop(&mut self) {
        self.shared.gl.delete_program(Some(&self.program));
        self.shared.gl.delete_vertex_array(Some(&self.vertex_array));
    }
}

pub(super) struct VertexBufferSlot {
    stride: u64,
    step_mode: VertexStepMode,
    attributes: Vec<VertexAttribute>,
}

#[derive(Clone)]
pub(super) struct SamplerBinding {
    sampler_group: u32,
    sampler_binding: u32,
    texture_unit: u32,
}

#[derive(Clone)]
pub(super) struct TextureBinding {
    group: u32,
    binding: u32,
    texture_unit: u32,
}

#[derive(Clone, Copy)]
pub(super) struct UniformBlockBinding {
    group: u32,
    binding: u32,
    binding_point: u32,
}

#[derive(Clone, Copy)]
pub(super) struct WebGlDepthState {
    depth_write_enabled: bool,
    depth_compare: CompareFunction,
    stencil: StencilState,
    bias: DepthBiasState,
}

impl WebGl2ShaderModule {
    pub(super) fn create(
        shared: &Rc<WebGl2Shared>,
        source: &[u8],
        label: &str,
    ) -> Result<Self, BackendError> {
        let source = std::str::from_utf8(source).map_err(|error| {
            backend_error(
                "create_shader_module",
                format!("{label} is not UTF-8 GLSL package data: {error}"),
            )
        })?;
        let (vertex_source, fragment_source, resources) = parse_shader_package(source)
            .map_err(|error| backend_error("create_shader_module", format!("{label}: {error}")))?;
        let vertex = if vertex_source.is_empty() {
            None
        } else {
            Some(compile_shader(
                shared,
                Gl::VERTEX_SHADER,
                vertex_source,
                label,
            )?)
        };
        let fragment = if fragment_source.is_empty() {
            None
        } else {
            match compile_shader(shared, Gl::FRAGMENT_SHADER, fragment_source, label) {
                Ok(shader) => Some(shader),
                Err(error) => {
                    if let Some(vertex) = &vertex {
                        shared.gl.delete_shader(Some(vertex));
                    }
                    return Err(error);
                }
            }
        };
        if vertex.is_none() && fragment.is_none() {
            return Err(backend_error(
                "create_shader_module",
                format!("{label} contains no shader stages"),
            ));
        }
        Ok(Self(Rc::new(WebGl2ShaderModuleInner {
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
    shared: &WebGl2Shared,
    stage: u32,
    source: &str,
    label: &str,
) -> Result<WebGlShader, BackendError> {
    let shader = shared.gl.create_shader(stage).ok_or_else(|| {
        backend_error(
            "create_shader_module",
            format!("{label}: createShader returned null"),
        )
    })?;
    let source = webgl_shader_source(source);
    shared.gl.shader_source(&shader, &source);
    shared.gl.compile_shader(&shader);
    let compiled = shared
        .gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false);
    if !compiled {
        let log = shared.gl.get_shader_info_log(&shader).unwrap_or_default();
        shared.gl.delete_shader(Some(&shader));
        return Err(backend_error("compile_shader", format!("{label}: {log}")));
    }
    Ok(shader)
}

fn webgl_shader_source(source: &str) -> String {
    source.replacen(
        "#version 330 core",
        "#version 300 es\nprecision highp float;\nprecision highp int;",
        1,
    )
}

pub(super) fn backend_error(operation: &'static str, message: impl Into<String>) -> BackendError {
    BackendError {
        operation,
        message: message.into(),
    }
}

pub(super) fn create_render_pipeline(
    shared: &Rc<WebGl2Shared>,
    desc: &RenderPipelineDescriptor<WebGl2Backend>,
) -> Result<WebGl2RenderPipeline, BackendError> {
    if desc.primitive.conservative || desc.primitive.unclipped_depth {
        return Err(backend_error(
            "create_render_pipeline",
            "conservative rasterization and unclipped depth are unsupported by WebGL2",
        ));
    }
    if desc.primitive.polygon_mode != PolygonMode::Fill {
        return Err(backend_error(
            "create_render_pipeline",
            "WebGL2 supports filled polygons only",
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
            "vertex attribute location exceeds the WebGL2 limit",
        ));
    }
    if desc.multisample.count != 1 {
        return Err(backend_error(
            "create_render_pipeline",
            "multisampled render pipelines are unsupported by this WebGL2 backend",
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
                    &entry.ty,
                    BindingType::Buffer {
                        ty: crate::backend::BufferBindingType::Storage
                            | crate::backend::BufferBindingType::ReadOnlyStorage,
                        ..
                    } | BindingType::StorageTexture { .. }
                )
            })
    }) {
        return Err(backend_error(
            "create_render_pipeline",
            "storage buffers and storage textures are not supported by the WebGL2 backend",
        ));
    }

    let vertex_shader = desc.vertex.module.0.vertex.as_ref().ok_or_else(|| {
        backend_error(
            "create_render_pipeline",
            "vertex module has no vertex stage",
        )
    })?;
    let fragment_shader = desc
        .fragment
        .as_ref()
        .map(|fragment| {
            fragment.module.0.fragment.as_ref().ok_or_else(|| {
                backend_error(
                    "create_render_pipeline",
                    "fragment module has no fragment stage",
                )
            })
        })
        .transpose()?;
    let gl = &shared.gl;
    let program = gl
        .create_program()
        .ok_or_else(|| backend_error("create_render_pipeline", "createProgram returned null"))?;
    gl.attach_shader(&program, vertex_shader);
    if let Some(fragment_shader) = fragment_shader {
        gl.attach_shader(&program, fragment_shader);
    }
    gl.link_program(&program);
    if !gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        let log = gl.get_program_info_log(&program).unwrap_or_default();
        gl.delete_program(Some(&program));
        return Err(backend_error("link_render_pipeline", log));
    }
    let vertex_array = gl.create_vertex_array().ok_or_else(|| {
        gl.delete_program(Some(&program));
        backend_error("create_render_pipeline", "createVertexArray returned null")
    })?;

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
                    let block_index = gl.get_uniform_block_index(&program, name);
                    if block_index != u32::MAX {
                        let binding_point = group
                            .saturating_mul(BINDINGS_PER_GROUP)
                            .saturating_add(*binding);
                        if binding_point >= shared.max_uniform_buffer_bindings {
                            gl.delete_program(Some(&program));
                            gl.delete_vertex_array(Some(&vertex_array));
                            return Err(backend_error(
                                "create_render_pipeline",
                                "uniform-buffer binding exceeds the WebGL2 limit",
                            ));
                        }
                        gl.uniform_block_binding(&program, block_index, binding_point);
                        if !uniform_blocks.iter().any(|known: &UniformBlockBinding| {
                            known.group == *group && known.binding == *binding
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
            gl.delete_program(Some(&program));
            gl.delete_vertex_array(Some(&vertex_array));
            return Err(backend_error(
                "create_render_pipeline",
                "sampler count exceeds the WebGL2 texture-unit limit",
            ));
        }
        if let Some(location) = gl.get_uniform_location(&program, &name) {
            gl.use_program(Some(&program));
            gl.uniform1i(Some(&location), next_unit as i32);
            samplers.push(SamplerBinding {
                sampler_group,
                sampler_binding,
                texture_unit: next_unit,
            });
            textures.push(TextureBinding {
                group: texture_group,
                binding: texture_binding,
                texture_unit: next_unit,
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
            gl.delete_program(Some(&program));
            gl.delete_vertex_array(Some(&vertex_array));
            return Err(backend_error(
                "create_render_pipeline",
                "texture count exceeds the WebGL2 texture-unit limit",
            ));
        }
        if let Some(location) = gl.get_uniform_location(&program, &name) {
            gl.use_program(Some(&program));
            gl.uniform1i(Some(&location), next_unit as i32);
            textures.push(TextureBinding {
                group,
                binding,
                texture_unit: next_unit,
            });
            next_unit += 1;
        }
    }
    gl.use_program(None);

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
        .unwrap_or((None, ColorWriteMask::ALL));
    let depth = desc.depth_stencil.as_ref().map(|depth| WebGlDepthState {
        depth_write_enabled: depth.depth_write_enabled,
        depth_compare: depth.depth_compare,
        stencil: depth.stencil,
        bias: depth.bias,
    });

    Ok(WebGl2RenderPipeline(Rc::new(WebGl2RenderPipelineInner {
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
        alpha_to_coverage: desc.multisample.alpha_to_coverage_enabled,
    })))
}

pub(super) fn begin_render_pass<'a>(
    encoder: &'a mut WebGl2CommandEncoder,
    desc: &RenderPassDescriptor<'_, WebGl2Backend>,
) -> Result<WebGl2RenderPass<'a>, BackendError> {
    let shared = &encoder.shared;
    if desc.color_attachments.len() > 1 {
        return Err(backend_error(
            "begin_render_pass",
            "multiple color attachments are not supported by the WebGL2 adapter yet",
        ));
    }
    if desc
        .color_attachments
        .iter()
        .any(|attachment| attachment.resolve_target.is_some())
    {
        return Err(backend_error(
            "begin_render_pass",
            "resolve targets require multisampled render targets, which WebGL2 does not expose here",
        ));
    }
    let (framebuffer, extent) = framebuffer_for_pass(shared, desc)?;
    shared
        .gl
        .bind_framebuffer(Gl::FRAMEBUFFER, framebuffer.as_ref());
    shared.gl.viewport(0, 0, extent.0 as i32, extent.1 as i32);
    shared.gl.disable(Gl::SCISSOR_TEST);
    shared.gl.color_mask(true, true, true, true);
    shared.gl.depth_mask(true);
    shared.gl.stencil_mask(u32::MAX);

    let mut clear_mask = 0;
    let mut discard_attachments = Vec::new();
    if let Some(color) = desc.color_attachments.first() {
        match color.ops.load {
            LoadOp::Load => {}
            LoadOp::Clear(clear) => {
                shared.gl.clear_color(
                    clear[0] as f32,
                    clear[1] as f32,
                    clear[2] as f32,
                    clear[3] as f32,
                );
                clear_mask |= Gl::COLOR_BUFFER_BIT;
            }
        }
        if color.ops.store == crate::backend::StoreOp::Discard {
            discard_attachments.push(Gl::COLOR_ATTACHMENT0);
        }
    }
    if let Some(depth) = &desc.depth_stencil_attachment {
        if let Some(ops) = depth.depth_ops {
            match ops.load {
                LoadOp::Load => {}
                LoadOp::Clear(value) => {
                    shared.gl.clear_depth(value);
                    clear_mask |= Gl::DEPTH_BUFFER_BIT;
                }
            }
            if ops.store == crate::backend::StoreOp::Discard {
                discard_attachments.push(Gl::DEPTH_ATTACHMENT);
            }
        }
        if let Some(ops) = depth.stencil_ops {
            match ops.load {
                LoadOp::Load => {}
                LoadOp::Clear(value) => {
                    shared.gl.clear_stencil(value as i32);
                    clear_mask |= Gl::STENCIL_BUFFER_BIT;
                }
            }
            if ops.store == crate::backend::StoreOp::Discard {
                discard_attachments.push(Gl::STENCIL_ATTACHMENT);
            }
        }
    }
    if clear_mask != 0 {
        shared.gl.clear(clear_mask);
    }

    let attachments = [
        desc.color_attachments
            .first()
            .map(|attachment| attachment.view.clone()),
        desc.depth_stencil_attachment
            .as_ref()
            .map(|attachment| attachment.view.clone()),
    ];
    Ok(WebGl2RenderPass {
        encoder,
        extent,
        attachments,
        discard_attachments,
        pipeline: None,
        index: None,
        vertex_buffers: Vec::new(),
        applied_vertex_offsets: None,
    })
}

fn framebuffer_for_pass(
    shared: &Rc<WebGl2Shared>,
    desc: &RenderPassDescriptor<'_, WebGl2Backend>,
) -> Result<(Option<WebGlFramebuffer>, (u32, u32)), BackendError> {
    let mut extent = None;
    let mut has_surface = false;
    let mut key = FramebufferKey {
        attachments: Vec::new(),
    };
    let mut color_attachments = Vec::new();
    let mut attachment_views = Vec::new();

    for (index, attachment) in desc.color_attachments.iter().enumerate() {
        let attachment_point = Gl::COLOR_ATTACHMENT0 + index as u32;
        append_attachment(
            &mut key,
            &mut attachment_views,
            &mut extent,
            &mut has_surface,
            attachment.view,
            attachment_point,
        )?;
        color_attachments.push(attachment_point);
    }
    if let Some(depth) = &desc.depth_stencil_attachment {
        let attachment_point = match &depth.view.0.target {
            WebGl2ViewTarget::Texture { texture, .. }
                if texture.0.format == WebGl2TextureFormat::Depth24PlusStencil8 =>
            {
                Gl::DEPTH_STENCIL_ATTACHMENT
            }
            _ => Gl::DEPTH_ATTACHMENT,
        };
        append_attachment(
            &mut key,
            &mut attachment_views,
            &mut extent,
            &mut has_surface,
            depth.view,
            attachment_point,
        )?;
    }

    let extent = extent.unwrap_or((
        shared.gl.drawing_buffer_width().max(0) as u32,
        shared.gl.drawing_buffer_height().max(0) as u32,
    ));
    if has_surface {
        if !key.attachments.is_empty() || desc.color_attachments.len() > 1 {
            return Err(backend_error(
                "begin_render_pass",
                "cannot mix the default framebuffer with texture attachments",
            ));
        }
        return Ok((None, extent));
    }

    let framebuffer = if let Some(framebuffer) = shared.framebuffer_cache.borrow().get(&key) {
        framebuffer.clone()
    } else {
        let framebuffer = shared.gl.create_framebuffer().ok_or_else(|| {
            backend_error("create_framebuffer", "createFramebuffer returned null")
        })?;
        shared
            .gl
            .bind_framebuffer(Gl::FRAMEBUFFER, Some(&framebuffer));
        for (view, attachment) in attachment_views {
            attach_texture_view(shared, &framebuffer, &view, attachment);
        }
        if !color_attachments.is_empty() {
            set_draw_buffers(&shared.gl, &color_attachments);
        } else {
            set_draw_buffers(&shared.gl, &[Gl::NONE]);
            shared.gl.read_buffer(Gl::NONE);
        }
        let status = shared.gl.check_framebuffer_status(Gl::FRAMEBUFFER);
        if status != Gl::FRAMEBUFFER_COMPLETE {
            shared.gl.delete_framebuffer(Some(&framebuffer));
            return Err(backend_error(
                "create_framebuffer",
                format!("framebuffer status {status:#x}"),
            ));
        }
        shared
            .framebuffer_cache
            .borrow_mut()
            .insert(key, framebuffer.clone());
        framebuffer
    };
    Ok((Some(framebuffer), extent))
}

fn append_attachment(
    key: &mut FramebufferKey,
    views: &mut Vec<(WebGl2TextureView, u32)>,
    extent: &mut Option<(u32, u32)>,
    has_surface: &mut bool,
    view: &WebGl2TextureView,
    attachment: u32,
) -> Result<(), BackendError> {
    let size = view.size();
    if let Some(current) = extent {
        if *current != size {
            return Err(backend_error(
                "begin_render_pass",
                "framebuffer attachment extents differ",
            ));
        }
    } else {
        *extent = Some(size);
    }
    match &view.0.target {
        WebGl2ViewTarget::Surface(_) => *has_surface = true,
        WebGl2ViewTarget::Texture {
            texture,
            level,
            layer,
        } => {
            key.attachments.push((
                attachment,
                texture.0.id,
                *level,
                layer.map_or(-1, |layer| layer as i32),
            ));
            views.push((view.clone(), attachment));
        }
    }
    Ok(())
}

pub(super) fn framebuffer_for_view(
    shared: &Rc<WebGl2Shared>,
    view: &WebGl2TextureView,
    attachment: u32,
) -> Result<Option<WebGlFramebuffer>, BackendError> {
    let WebGl2ViewTarget::Texture {
        texture,
        level,
        layer,
    } = &view.0.target
    else {
        return Ok(None);
    };
    let key = FramebufferKey {
        attachments: vec![(
            attachment,
            texture.0.id,
            *level,
            layer.map_or(-1, |layer| layer as i32),
        )],
    };
    if let Some(framebuffer) = shared.framebuffer_cache.borrow().get(&key) {
        return Ok(Some(framebuffer.clone()));
    }
    let framebuffer = shared
        .gl
        .create_framebuffer()
        .ok_or_else(|| backend_error("create_framebuffer", "createFramebuffer returned null"))?;
    shared
        .gl
        .bind_framebuffer(Gl::FRAMEBUFFER, Some(&framebuffer));
    attach_texture_view(shared, &framebuffer, view, attachment);
    if attachment == Gl::COLOR_ATTACHMENT0 {
        set_draw_buffers(&shared.gl, &[Gl::COLOR_ATTACHMENT0]);
    } else {
        set_draw_buffers(&shared.gl, &[Gl::NONE]);
        shared.gl.read_buffer(Gl::NONE);
    }
    let status = shared.gl.check_framebuffer_status(Gl::FRAMEBUFFER);
    if status != Gl::FRAMEBUFFER_COMPLETE {
        shared.gl.delete_framebuffer(Some(&framebuffer));
        return Err(backend_error(
            "create_framebuffer",
            format!("framebuffer status {status:#x}"),
        ));
    }
    shared
        .framebuffer_cache
        .borrow_mut()
        .insert(key, framebuffer.clone());
    Ok(Some(framebuffer))
}

pub(super) fn attach_texture_view(
    shared: &WebGl2Shared,
    _framebuffer: &WebGlFramebuffer,
    view: &WebGl2TextureView,
    attachment: u32,
) {
    let WebGl2ViewTarget::Texture {
        texture,
        level,
        layer,
    } = &view.0.target
    else {
        return;
    };
    if let Some(layer) = layer {
        shared.gl.framebuffer_texture_layer(
            Gl::FRAMEBUFFER,
            attachment,
            Some(&texture.0.object),
            *level as i32,
            *layer as i32,
        );
    } else {
        shared.gl.framebuffer_texture_2d(
            Gl::FRAMEBUFFER,
            attachment,
            texture.0.target,
            Some(&texture.0.object),
            *level as i32,
        );
    }
}

pub(super) fn set_draw_buffers(gl: &Gl, buffers: &[u32]) {
    let values = js_sys::Array::new();
    for buffer in buffers {
        values.push(&wasm_bindgen::JsValue::from_f64(*buffer as f64));
    }
    gl.draw_buffers(values.as_ref());
}

impl Drop for WebGl2RenderPass<'_> {
    fn drop(&mut self) {
        if !self.discard_attachments.is_empty() {
            let attachments = js_sys::Array::new();
            for attachment in &self.discard_attachments {
                attachments.push(&wasm_bindgen::JsValue::from_f64(*attachment as f64));
            }
            let _ = self
                .encoder
                .shared
                .gl
                .invalidate_framebuffer(Gl::FRAMEBUFFER, attachments.as_ref());
        }
        let _ = &self.attachments;
    }
}

impl GpuRenderPass<WebGl2Backend> for WebGl2RenderPass<'_> {
    fn set_pipeline(&mut self, pipeline: &WebGl2RenderPipeline) {
        let gl = &self.encoder.shared.gl;
        let state = &pipeline.0;
        gl.use_program(Some(&state.program));
        gl.bind_vertex_array(Some(&state.vertex_array));
        apply_pipeline_state(gl, state);
        self.vertex_buffers.clear();
        self.vertex_buffers.resize(state.vertex_slots.len(), None);
        self.index = None;
        self.applied_vertex_offsets = None;
        self.pipeline = Some(pipeline.clone());
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &WebGl2BindGroup,
        dynamic_offsets: &[u32],
    ) {
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("WebGL2 bind group set before pipeline");
        let shared = &self.encoder.shared;
        let mut dynamic_index = 0;
        for entry in &bind_group.0.layout.0.entries {
            let dynamic_offset = match &entry.ty {
                BindingType::Buffer {
                    has_dynamic_offset: true,
                    ..
                } => {
                    let offset = *dynamic_offsets
                        .get(dynamic_index)
                        .expect("missing WebGL2 dynamic offset");
                    dynamic_index += 1;
                    offset as u64
                }
                _ => 0,
            };
            let resource = bind_group
                .0
                .resources
                .get(&entry.binding)
                .unwrap_or_else(|| panic!("missing WebGL2 bind-group binding {}", entry.binding));
            match resource {
                WebGl2BoundResource::Buffer(buffer) => {
                    if let Some(block) = pipeline
                        .0
                        .uniform_blocks
                        .iter()
                        .find(|block| block.group == index && block.binding == entry.binding)
                    {
                        let size = buffer.0.size.saturating_sub(dynamic_offset);
                        shared.gl.bind_buffer_range_with_f64_and_f64(
                            Gl::UNIFORM_BUFFER,
                            block.binding_point,
                            Some(buffer.0.object_for_target(Gl::UNIFORM_BUFFER)),
                            dynamic_offset as f64,
                            size as f64,
                        );
                    }
                }
                WebGl2BoundResource::BufferRange(buffer, offset, size) => {
                    if let Some(block) = pipeline
                        .0
                        .uniform_blocks
                        .iter()
                        .find(|block| block.group == index && block.binding == entry.binding)
                    {
                        shared.gl.bind_buffer_range_with_f64_and_f64(
                            Gl::UNIFORM_BUFFER,
                            block.binding_point,
                            Some(buffer.0.object_for_target(Gl::UNIFORM_BUFFER)),
                            offset.saturating_add(dynamic_offset) as f64,
                            *size as f64,
                        );
                    }
                }
                WebGl2BoundResource::TextureView(view) => {
                    if let Some(binding) =
                        pipeline.0.textures.iter().find(|binding| {
                            binding.group == index && binding.binding == entry.binding
                        })
                    {
                        if let WebGl2ViewTarget::Texture { texture, .. } = &view.0.target {
                            shared
                                .gl
                                .active_texture(Gl::TEXTURE0 + binding.texture_unit);
                            shared
                                .gl
                                .bind_texture(texture.0.target, Some(&texture.0.object));
                            if !pipeline
                                .0
                                .samplers
                                .iter()
                                .any(|sampler| sampler.texture_unit == binding.texture_unit)
                            {
                                shared.gl.bind_sampler(binding.texture_unit, None);
                            }
                        }
                    }
                }
                WebGl2BoundResource::Sampler(sampler) => {
                    if let Some(binding) = pipeline.0.samplers.iter().find(|binding| {
                        binding.sampler_group == index && binding.sampler_binding == entry.binding
                    }) {
                        shared
                            .gl
                            .bind_sampler(binding.texture_unit, Some(&sampler.0.object));
                    }
                }
            }
        }
        assert_eq!(
            dynamic_index,
            dynamic_offsets.len(),
            "too many WebGL2 dynamic offsets"
        );
        shared.gl.active_texture(Gl::TEXTURE0);
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &WebGl2Buffer, offset: u64) {
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("WebGL2 vertex buffer set before pipeline");
        let Some(Some(_)) = pipeline.0.vertex_slots.get(slot as usize) else {
            return;
        };
        self.vertex_buffers[slot as usize] = Some((buffer.clone(), offset));
        self.applied_vertex_offsets = None;
    }

    fn write_buffer_before_draw(
        &mut self,
        buffer: &WebGl2Buffer,
        offset: u64,
        data: &[u8],
    ) -> bool {
        super::trait_impl::write_buffer_immediate(&self.encoder.shared, buffer, offset, data);
        true
    }

    fn set_index_buffer(&mut self, buffer: &WebGl2Buffer, format: IndexFormat, offset: u64) {
        self.encoder.shared.gl.bind_buffer(
            Gl::ELEMENT_ARRAY_BUFFER,
            Some(buffer.0.object_for_target(Gl::ELEMENT_ARRAY_BUFFER)),
        );
        self.index = Some(WebGl2IndexBuffer {
            _buffer: buffer.clone(),
            format,
            offset,
        });
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let gl_y = self.extent.1.saturating_sub(y.saturating_add(height));
        self.encoder.shared.gl.enable(Gl::SCISSOR_TEST);
        self.encoder
            .shared
            .gl
            .scissor(x as i32, gl_y as i32, width as i32, height as i32);
    }

    fn draw(&mut self, vertices: std::ops::Range<u32>, instances: std::ops::Range<u32>) {
        let topology = primitive_topology(
            self.pipeline
                .as_ref()
                .expect("WebGL2 draw before pipeline")
                .0
                .primitive
                .topology,
        );
        self.apply_vertex_offsets(0, instances.start);
        self.encoder.shared.gl.draw_arrays_instanced(
            topology,
            vertices.start as i32,
            vertices.end.saturating_sub(vertices.start) as i32,
            instances.end.saturating_sub(instances.start) as i32,
        );
    }

    fn draw_indexed(
        &mut self,
        indices: std::ops::Range<u32>,
        base_vertex: i32,
        instances: std::ops::Range<u32>,
    ) {
        let topology = primitive_topology(
            self.pipeline
                .as_ref()
                .expect("WebGL2 indexed draw before pipeline")
                .0
                .primitive
                .topology,
        );
        let (index_element_format, index_offset) = self
            .index
            .as_ref()
            .map(|index| (index.format, index.offset))
            .expect("WebGL2 indexed draw without index buffer");
        self.apply_vertex_offsets(base_vertex, instances.start);
        let (format, bytes) = index_format(index_element_format);
        let byte_offset = index_offset + indices.start as u64 * bytes;
        self.encoder.shared.gl.draw_elements_instanced_with_i32(
            topology,
            indices.end.saturating_sub(indices.start) as i32,
            format,
            byte_offset as i32,
            instances.end.saturating_sub(instances.start) as i32,
        );
    }
}

impl WebGl2RenderPass<'_> {
    fn apply_vertex_offsets(&mut self, base_vertex: i32, first_instance: u32) {
        if self.applied_vertex_offsets == Some((base_vertex, first_instance)) {
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
            let mut adjusted = *offset;
            match layout.step_mode {
                VertexStepMode::Vertex => {
                    let delta = base_vertex as i64 * layout.stride as i64;
                    adjusted = (adjusted as i64 + delta)
                        .try_into()
                        .expect("WebGL2 vertex base offset is out of bounds");
                }
                VertexStepMode::Instance => {
                    adjusted = adjusted.saturating_add(first_instance as u64 * layout.stride);
                }
            }
            bind_vertex_buffer(&self.encoder.shared.gl, layout, buffer, adjusted);
        }
        self.applied_vertex_offsets = Some((base_vertex, first_instance));
    }
}

fn bind_vertex_buffer(gl: &Gl, layout: &VertexBufferSlot, buffer: &WebGl2Buffer, base_offset: u64) {
    gl.bind_buffer(
        Gl::ARRAY_BUFFER,
        Some(buffer.0.object_for_target(Gl::ARRAY_BUFFER)),
    );
    for attribute in &layout.attributes {
        let (components, format, normalized, integer) = vertex_format(attribute.format);
        let offset = base_offset.saturating_add(attribute.offset);
        gl.enable_vertex_attrib_array(attribute.shader_location);
        if integer {
            gl.vertex_attrib_i_pointer_with_i32(
                attribute.shader_location,
                components,
                format,
                layout.stride as i32,
                offset as i32,
            );
        } else {
            gl.vertex_attrib_pointer_with_i32(
                attribute.shader_location,
                components,
                format,
                normalized,
                layout.stride as i32,
                offset as i32,
            );
        }
        gl.vertex_attrib_divisor(
            attribute.shader_location,
            u32::from(layout.step_mode == VertexStepMode::Instance),
        );
    }
}

fn apply_pipeline_state(gl: &Gl, state: &WebGl2RenderPipelineInner) {
    gl.front_face(match state.primitive.front_face {
        FrontFace::Ccw => Gl::CCW,
        FrontFace::Cw => Gl::CW,
    });
    if let Some(face) = state.primitive.cull_mode {
        gl.enable(Gl::CULL_FACE);
        gl.cull_face(match face {
            Face::Front => Gl::FRONT,
            Face::Back => Gl::BACK,
        });
    } else {
        gl.disable(Gl::CULL_FACE);
    }
    if let Some(blend) = &state.blend {
        gl.enable(Gl::BLEND);
        if has_constant_blend_factor(blend.color.src_factor)
            || has_constant_blend_factor(blend.color.dst_factor)
            || has_constant_blend_factor(blend.alpha.src_factor)
            || has_constant_blend_factor(blend.alpha.dst_factor)
        {
            gl.blend_color(0.0, 0.0, 0.0, 0.0);
        }
        gl.blend_func_separate(
            blend_factor(blend.color.src_factor),
            blend_factor(blend.color.dst_factor),
            blend_factor(blend.alpha.src_factor),
            blend_factor(blend.alpha.dst_factor),
        );
        gl.blend_equation_separate(
            blend_operation(blend.color.operation),
            blend_operation(blend.alpha.operation),
        );
    } else {
        gl.disable(Gl::BLEND);
    }
    gl.color_mask(
        state.write_mask.red,
        state.write_mask.green,
        state.write_mask.blue,
        state.write_mask.alpha,
    );
    if let Some(depth) = state.depth {
        gl.enable(Gl::DEPTH_TEST);
        gl.depth_mask(depth.depth_write_enabled);
        gl.depth_func(compare_function(depth.depth_compare));
        gl.enable(Gl::STENCIL_TEST);
        for face in [Gl::FRONT, Gl::BACK] {
            let stencil = if face == Gl::FRONT {
                depth.stencil.front
            } else {
                depth.stencil.back
            };
            gl.stencil_func_separate(
                face,
                compare_function(stencil.compare),
                0,
                depth.stencil.read_mask,
            );
            gl.stencil_op_separate(
                face,
                stencil_operation(stencil.fail_op),
                stencil_operation(stencil.depth_fail_op),
                stencil_operation(stencil.pass_op),
            );
            gl.stencil_mask_separate(face, depth.stencil.write_mask);
        }
        if depth.bias.constant != 0 || depth.bias.slope_scale != 0.0 {
            gl.enable(Gl::POLYGON_OFFSET_FILL);
            gl.polygon_offset(depth.bias.slope_scale, depth.bias.constant as f32);
        } else {
            gl.disable(Gl::POLYGON_OFFSET_FILL);
        }
    } else {
        gl.disable(Gl::DEPTH_TEST);
        gl.disable(Gl::STENCIL_TEST);
        gl.depth_mask(false);
        gl.disable(Gl::POLYGON_OFFSET_FILL);
    }
    if state.alpha_to_coverage {
        gl.enable(Gl::SAMPLE_ALPHA_TO_COVERAGE);
    } else {
        gl.disable(Gl::SAMPLE_ALPHA_TO_COVERAGE);
    }
}

fn has_constant_blend_factor(factor: BlendFactor) -> bool {
    matches!(
        factor,
        BlendFactor::Constant | BlendFactor::OneMinusConstant
    )
}

fn primitive_topology(topology: PrimitiveTopology) -> u32 {
    match topology {
        PrimitiveTopology::PointList => Gl::POINTS,
        PrimitiveTopology::LineList => Gl::LINES,
        PrimitiveTopology::LineStrip => Gl::LINE_STRIP,
        PrimitiveTopology::TriangleList => Gl::TRIANGLES,
        PrimitiveTopology::TriangleStrip => Gl::TRIANGLE_STRIP,
    }
}

fn index_format(format: IndexFormat) -> (u32, u64) {
    match format {
        IndexFormat::Uint16 => (Gl::UNSIGNED_SHORT, 2),
        IndexFormat::Uint32 => (Gl::UNSIGNED_INT, 4),
    }
}

fn compare_function(compare: CompareFunction) -> u32 {
    match compare {
        CompareFunction::Never => Gl::NEVER,
        CompareFunction::Less => Gl::LESS,
        CompareFunction::Equal => Gl::EQUAL,
        CompareFunction::LessEqual => Gl::LEQUAL,
        CompareFunction::Greater => Gl::GREATER,
        CompareFunction::NotEqual => Gl::NOTEQUAL,
        CompareFunction::GreaterEqual => Gl::GEQUAL,
        CompareFunction::Always => Gl::ALWAYS,
    }
}

fn blend_factor(factor: BlendFactor) -> u32 {
    match factor {
        BlendFactor::Zero => Gl::ZERO,
        BlendFactor::One => Gl::ONE,
        BlendFactor::Src => Gl::SRC_COLOR,
        BlendFactor::OneMinusSrc => Gl::ONE_MINUS_SRC_COLOR,
        BlendFactor::SrcAlpha => Gl::SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => Gl::ONE_MINUS_SRC_ALPHA,
        BlendFactor::Dst => Gl::DST_COLOR,
        BlendFactor::OneMinusDst => Gl::ONE_MINUS_DST_COLOR,
        BlendFactor::DstAlpha => Gl::DST_ALPHA,
        BlendFactor::OneMinusDstAlpha => Gl::ONE_MINUS_DST_ALPHA,
        BlendFactor::SrcAlphaSaturated => Gl::SRC_ALPHA_SATURATE,
        BlendFactor::Constant => Gl::CONSTANT_COLOR,
        BlendFactor::OneMinusConstant => Gl::ONE_MINUS_CONSTANT_COLOR,
    }
}

fn blend_operation(operation: BlendOperation) -> u32 {
    match operation {
        BlendOperation::Add => Gl::FUNC_ADD,
        BlendOperation::Subtract => Gl::FUNC_SUBTRACT,
        BlendOperation::ReverseSubtract => Gl::FUNC_REVERSE_SUBTRACT,
        BlendOperation::Min => Gl::MIN,
        BlendOperation::Max => Gl::MAX,
    }
}

fn stencil_operation(operation: StencilOperation) -> u32 {
    match operation {
        StencilOperation::Keep => Gl::KEEP,
        StencilOperation::Zero => Gl::ZERO,
        StencilOperation::Replace => Gl::REPLACE,
        StencilOperation::IncrementClamp => Gl::INCR,
        StencilOperation::DecrementClamp => Gl::DECR,
        StencilOperation::Invert => Gl::INVERT,
        StencilOperation::IncrementWrap => Gl::INCR_WRAP,
        StencilOperation::DecrementWrap => Gl::DECR_WRAP,
    }
}

fn vertex_format(format: VertexFormat) -> (i32, u32, bool, bool) {
    match format {
        VertexFormat::Uint8x2 => (2, Gl::UNSIGNED_BYTE, false, true),
        VertexFormat::Uint8x4 => (4, Gl::UNSIGNED_BYTE, false, true),
        VertexFormat::Sint8x2 => (2, Gl::BYTE, false, true),
        VertexFormat::Sint8x4 => (4, Gl::BYTE, false, true),
        VertexFormat::Unorm8x2 => (2, Gl::UNSIGNED_BYTE, true, false),
        VertexFormat::Unorm8x4 => (4, Gl::UNSIGNED_BYTE, true, false),
        VertexFormat::Snorm8x2 => (2, Gl::BYTE, true, false),
        VertexFormat::Snorm8x4 => (4, Gl::BYTE, true, false),
        VertexFormat::Uint16x2 => (2, Gl::UNSIGNED_SHORT, false, true),
        VertexFormat::Uint16x4 => (4, Gl::UNSIGNED_SHORT, false, true),
        VertexFormat::Sint16x2 => (2, Gl::SHORT, false, true),
        VertexFormat::Sint16x4 => (4, Gl::SHORT, false, true),
        VertexFormat::Unorm16x2 => (2, Gl::UNSIGNED_SHORT, true, false),
        VertexFormat::Unorm16x4 => (4, Gl::UNSIGNED_SHORT, true, false),
        VertexFormat::Snorm16x2 => (2, Gl::SHORT, true, false),
        VertexFormat::Snorm16x4 => (4, Gl::SHORT, true, false),
        VertexFormat::Float16x2 => (2, Gl::HALF_FLOAT, false, false),
        VertexFormat::Float16x4 => (4, Gl::HALF_FLOAT, false, false),
        VertexFormat::Float32 => (1, Gl::FLOAT, false, false),
        VertexFormat::Float32x2 => (2, Gl::FLOAT, false, false),
        VertexFormat::Float32x3 => (3, Gl::FLOAT, false, false),
        VertexFormat::Float32x4 => (4, Gl::FLOAT, false, false),
        VertexFormat::Uint32 => (1, Gl::UNSIGNED_INT, false, true),
        VertexFormat::Uint32x2 => (2, Gl::UNSIGNED_INT, false, true),
        VertexFormat::Uint32x3 => (3, Gl::UNSIGNED_INT, false, true),
        VertexFormat::Uint32x4 => (4, Gl::UNSIGNED_INT, false, true),
        VertexFormat::Sint32 => (1, Gl::INT, false, true),
        VertexFormat::Sint32x2 => (2, Gl::INT, false, true),
        VertexFormat::Sint32x3 => (3, Gl::INT, false, true),
        VertexFormat::Sint32x4 => (4, Gl::INT, false, true),
    }
}
