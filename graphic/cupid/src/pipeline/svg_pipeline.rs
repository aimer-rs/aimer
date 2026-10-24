use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use super::frame_upload::FrameUpload;
use super::image_pipeline::InstanceBufferPolicy;
use crate::renderer::SvgRenderItem;
use crate::svg::{
    SvgColor, SvgColorMatrix, SvgFilterInput, SvgFilterPrimitive, SvgGeometryCache, SvgGradient,
    SvgMesh, SvgMeshStyle, SvgNode, SvgNodeStyleOverride, SvgPaint, SvgPaintOrder,
};
use crate::utilities::{Mat3, Rgba8};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SvgVertex {
    position: [f32; 2],
    coverage: f32,
}

impl SvgVertex {
    #[cfg(feature = "wgpu")]
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 7 => Float32];

    #[cfg(feature = "wgpu")]
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 2] = [
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, position) as u64,
            shader_location: 0,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32,
            offset: std::mem::offset_of!(Self, coverage) as u64,
            shader_location: 7,
        },
    ];
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SvgInstance {
    transform_x: [f32; 4],
    transform_y: [f32; 4],
    color: Rgba8,
    clip_rect: [f32; 4],
    clip_border_radius: [f32; 4],
    viewport: [f32; 4],
}

impl SvgInstance {
    #[cfg(feature = "wgpu")]
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        1 => Float32x4,
        2 => Float32x4,
        3 => Unorm8x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x4,
    ];

    #[cfg(feature = "wgpu")]
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 6] = [
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, transform_x) as u64,
            shader_location: 1,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, transform_y) as u64,
            shader_location: 2,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, color) as u64,
            shader_location: 3,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_rect) as u64,
            shader_location: 4,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_border_radius) as u64,
            shader_location: 5,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, viewport) as u64,
            shader_location: 6,
        },
    ];
}

struct GpuMesh<B: crate::backend::GpuBackend> {
    _mesh: Arc<SvgMesh>,
    vertex_buffer: B::Buffer,
    index_buffer: B::Buffer,
    index_count: u32,
    bytes: u64,
    last_used: u64,
}

struct PreparedDraw {
    mesh_key: usize,
    instance_index: u32,
}

pub struct SvgPipeline<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    pipeline: B::RenderPipeline,
    geometry_cache: SvgGeometryCache,
    gradient_meshes: HashMap<(usize, u64), Vec<(Arc<SvgMesh>, SvgColor)>>,
    gradient_mesh_bytes: usize,
    gpu_meshes: HashMap<usize, GpuMesh<B>>,
    prepared_draws: Vec<PreparedDraw>,
    item_ranges: Vec<Range<usize>>,
    instances: Vec<SvgInstance>,
    instance_buffer: B::Buffer,
    instance_policy: InstanceBufferPolicy,
    upload: FrameUpload<SvgInstance>,
    gpu_mesh_bytes: u64,
    usage_clock: u64,
    max_gpu_mesh_bytes: u64,
    max_gpu_meshes: usize,
    analytic_aa: bool,
}

#[cfg(feature = "wgpu")]
#[inline]
fn svg_shader_source() -> &'static str {
    crate::backend::BuiltinShader::Svg.wgsl_str()
}

// ── Backend-generic pipeline implementation ─────────────────────────────────

impl<B: crate::backend::GpuBackend> SvgPipeline<B> {
    const INITIAL_INSTANCE_CAPACITY: usize = 64;
    const MAX_CPU_MESH_BYTES: usize = 32 * 1024 * 1024;
    const MAX_CPU_MESHES: usize = 4096;
    const MAX_GPU_MESH_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_GPU_MESHES: usize = 4096;
    /// Creates the pipeline and its GPU resources through the selected
    /// [`GpuBackend`].
    pub fn new(backend: &B, format: B::TextureFormat, antialiasing: crate::AntiAlias) -> Self {
        use crate::backend::*;

        let shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::Svg),
            "svg shader",
        );

        let layout = backend.create_pipeline_layout(&[]);

        let vertex_buf = VertexBufferLayout {
            array_stride: size_of::<SvgVertex>() as u64,
            step_mode: VertexStepMode::Vertex,
            attributes: &SvgVertex::GENERIC_ATTRIBUTES,
        };

        let instance_buf = VertexBufferLayout {
            array_stride: size_of::<SvgInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &SvgInstance::GENERIC_ATTRIBUTES,
        };

        let buffers = [Some(vertex_buf), Some(instance_buf)];

        let pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("svg pipeline".to_string()),
            layout: Some(&layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &buffers,
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

        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("svg instance buffer".to_string()),
            size: (Self::INITIAL_INSTANCE_CAPACITY * size_of::<SvgInstance>()) as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        Self {
            pipeline,
            geometry_cache: SvgGeometryCache::new(Self::MAX_CPU_MESH_BYTES, Self::MAX_CPU_MESHES),
            gradient_meshes: HashMap::new(),
            gradient_mesh_bytes: 0,
            gpu_meshes: HashMap::new(),
            prepared_draws: Vec::new(),
            item_ranges: Vec::new(),
            instances: Vec::new(),
            instance_buffer,
            instance_policy: InstanceBufferPolicy::new(Self::INITIAL_INSTANCE_CAPACITY),
            upload: FrameUpload::new(),
            gpu_mesh_bytes: 0,
            usage_clock: 0,
            max_gpu_mesh_bytes: Self::MAX_GPU_MESH_BYTES,
            max_gpu_meshes: Self::MAX_GPU_MESHES,
            analytic_aa: antialiasing == crate::AntiAlias::Analytic,
        }
    }

    /// Prepares SVG meshes and uploads their GPU buffers through the selected
    /// backend.
    pub fn prepare(
        &mut self,
        backend: &B,
        items: &[SvgRenderItem],
        width: u32,
        height: u32,
        is_srgb: bool,
    ) {
        self.usage_clock = self.usage_clock.wrapping_add(1);
        self.prepared_draws.clear();
        self.item_ranges.clear();
        self.instances.clear();
        let mut frame_meshes = HashSet::new();
        for item in items {
            let range_start = self.prepared_draws.len();
            if item.opacity > 0.0 && item.destination.width > 0.0 && item.destination.height > 0.0 {
                self.prepare_item_generic(backend, item, width, height, is_srgb, &mut frame_meshes);
            }
            self.item_ranges
                .push(range_start..self.prepared_draws.len());
        }

        let old_capacity = self.instance_policy.capacity();
        self.instance_policy.record_usage(self.instances.len());
        if old_capacity != self.instance_policy.capacity() {
            self.instance_buffer = backend.create_buffer(&crate::backend::BufferDescriptor {
                label: Some("svg instance buffer (resized)".to_string()),
                size: (self.instance_policy.capacity() * size_of::<SvgInstance>()) as u64,
                usage: vec![
                    crate::backend::BufferUsage::Vertex,
                    crate::backend::BufferUsage::CopyDst,
                ],
            });
            self.upload.invalidate();
        }
        self.upload
            .upload(backend, &self.instance_buffer, &self.instances);
        self.evict_gpu_meshes_generic(&frame_meshes);
    }

    fn prepare_item_generic(
        &mut self,
        backend: &B,
        item: &SvgRenderItem,
        width: u32,
        height: u32,
        is_srgb: bool,
        frame_meshes: &mut HashSet<usize>,
    ) {
        for node in item
            .scene
            .nodes
            .iter()
            .filter(|node| node.visible && !node.is_definition && node.geometry.is_some())
        {
            let Some(geometry) = item.scene.geometry(node) else {
                continue;
            };
            let node_override = item
                .overrides
                .iter()
                .find(|value| value.node_id == node.node_id);
            let mut transform = combined_transform(item, node, node_override);
            let effects = resolve_node_effects(item, node, node_override, geometry, transform);
            transform = transform.mul(&Mat3::translate(effects.offset[0], effects.offset[1]));
            if outside_viewport(transform, geometry, width, height) {
                continue;
            }
            let physical_scale = transform_scale(transform);
            let mut opacity = item.opacity
                * node_override
                    .and_then(|value| value.opacity)
                    .unwrap_or(node.opacity);
            opacity *= effects.mask_opacity;
            if opacity <= 0.0 {
                continue;
            }
            let fill = resolved_fill(node, node_override);
            let fill_pattern = resolved_pattern_fill(node, node_override);
            let stroke = resolved_stroke(node, node_override);
            match node.paint_order {
                SvgPaintOrder::FillAndStroke => {
                    if let Some(paint) = fill {
                        self.prepare_paint_generic(
                            backend,
                            geometry,
                            paint,
                            transform,
                            opacity,
                            effects.clip_rect,
                            effects.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            &effects.color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    } else if let Some(id) = fill_pattern {
                        self.prepare_pattern_generic(
                            backend,
                            item,
                            geometry,
                            id,
                            transform,
                            opacity,
                            effects.clip_rect,
                            effects.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            &effects.color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    }
                    if let Some(paint) = stroke {
                        self.prepare_paint_generic(
                            backend,
                            geometry,
                            paint,
                            transform,
                            opacity,
                            effects.clip_rect,
                            effects.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            &effects.color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    }
                }
                SvgPaintOrder::StrokeAndFill => {
                    if let Some(paint) = stroke {
                        self.prepare_paint_generic(
                            backend,
                            geometry,
                            paint,
                            transform,
                            opacity,
                            effects.clip_rect,
                            effects.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            &effects.color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    }
                    if let Some(paint) = fill {
                        self.prepare_paint_generic(
                            backend,
                            geometry,
                            paint,
                            transform,
                            opacity,
                            effects.clip_rect,
                            effects.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            &effects.color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    } else if let Some(id) = fill_pattern {
                        self.prepare_pattern_generic(
                            backend,
                            item,
                            geometry,
                            id,
                            transform,
                            opacity,
                            effects.clip_rect,
                            effects.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            &effects.color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_pattern_generic(
        &mut self,
        backend: &B,
        item: &SvgRenderItem,
        target_geometry: &crate::svg::SvgGeometry,
        pattern_id: &str,
        target_transform: Mat3,
        opacity: f32,
        clip_rect: [f32; 4],
        clip_border_radius: [f32; 4],
        width: u32,
        height: u32,
        is_srgb: bool,
        color_matrices: &[SvgColorMatrix],
        physical_scale: f32,
        frame_meshes: &mut HashSet<usize>,
    ) {
        let Some(pattern) = item.scene.resources.pattern(pattern_id) else {
            return;
        };
        let Some(target_rect) = rect_geometry_bounds(target_geometry) else {
            return;
        };
        if !is_identity_transform(pattern.transform) {
            return;
        }
        let target_bounds = geometry_bounds(target_geometry);
        let (origin_x, origin_y, tile_width, tile_height) = match pattern.units {
            crate::svg::SvgResourceUnits::ObjectBoundingBox => (
                target_bounds[0] + pattern.tile[0] * target_bounds[2],
                target_bounds[1] + pattern.tile[1] * target_bounds[3],
                pattern.tile[2] * target_bounds[2],
                pattern.tile[3] * target_bounds[3],
            ),
            crate::svg::SvgResourceUnits::UserSpaceOnUse => (
                pattern.tile[0],
                pattern.tile[1],
                pattern.tile[2],
                pattern.tile[3],
            ),
        };
        if ![origin_x, origin_y, tile_width, tile_height]
            .into_iter()
            .all(f32::is_finite)
            || tile_width <= 0.0
            || tile_height <= 0.0
        {
            return;
        }
        let first_column = ((target_rect[0] - origin_x) / tile_width).floor() as i32;
        let last_column = ((target_rect[0] + target_rect[2] - origin_x) / tile_width).ceil() as i32;
        let first_row = ((target_rect[1] - origin_y) / tile_height).floor() as i32;
        let last_row = ((target_rect[1] + target_rect[3] - origin_y) / tile_height).ceil() as i32;
        let columns = last_column.saturating_sub(first_column).max(0) as usize;
        let rows = last_row.saturating_sub(first_row).max(0) as usize;
        if columns.saturating_mul(rows) > 4096 {
            return;
        }

        let Some(root_inverse) = item.scene.resources.root_transform.inverse() else {
            return;
        };
        let root_inverse = svg_transform_matrix(root_inverse);
        let content_transform = match pattern.content_units {
            crate::svg::SvgResourceUnits::ObjectBoundingBox => {
                Mat3::translate(target_bounds[0], target_bounds[1])
                    .mul(&Mat3::scale(target_bounds[2], target_bounds[3]))
            }
            crate::svg::SvgResourceUnits::UserSpaceOnUse => Mat3::identity(),
        };
        let mut pattern_clip = NodeEffectState {
            clip_rect,
            clip_border_radius,
            mask_opacity: 1.0,
            offset: [0.0; 2],
            color_matrices: Vec::new(),
        };
        let Some(target_screen_rect) = axis_aligned_rect(target_transform, target_rect) else {
            return;
        };
        if !apply_clip_rect(&mut pattern_clip, target_screen_rect) {
            return;
        }

        for row in first_row..last_row {
            for column in first_column..last_column {
                let x = origin_x + column as f32 * tile_width;
                let y = origin_y + row as f32 * tile_height;
                let tile_transform = Mat3::translate(x, y);
                let Some(tile_screen_rect) =
                    axis_aligned_rect(target_transform, [x, y, tile_width, tile_height])
                else {
                    return;
                };
                let mut tile_clip = NodeEffectState {
                    clip_rect: pattern_clip.clip_rect,
                    clip_border_radius: pattern_clip.clip_border_radius,
                    mask_opacity: 1.0,
                    offset: [0.0; 2],
                    color_matrices: Vec::new(),
                };
                if !apply_clip_rect(&mut tile_clip, tile_screen_rect) {
                    continue;
                }
                for node_id in pattern.nodes.iter().copied() {
                    let Some(pattern_node) = item.scene.node(crate::svg::SvgNodeId(node_id)) else {
                        continue;
                    };
                    if !pattern_node.visible {
                        continue;
                    }
                    let Some(geometry) = item.scene.geometry(pattern_node) else {
                        continue;
                    };
                    let child_transform = tile_transform
                        .mul(&svg_transform_matrix(pattern.transform))
                        .mul(&content_transform)
                        .mul(&root_inverse)
                        .mul(&svg_transform_matrix(pattern_node.transform));
                    let child_transform = target_transform.mul(&child_transform);
                    if outside_viewport(child_transform, geometry, width, height) {
                        continue;
                    }
                    let child_opacity = opacity * pattern_node.opacity;
                    if child_opacity <= 0.0 {
                        continue;
                    }
                    if let Some(paint) = resolved_fill(pattern_node, None) {
                        self.prepare_paint_generic(
                            backend,
                            geometry,
                            paint,
                            child_transform,
                            child_opacity,
                            tile_clip.clip_rect,
                            tile_clip.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    }
                    if let Some(paint) = resolved_stroke(pattern_node, None) {
                        self.prepare_paint_generic(
                            backend,
                            geometry,
                            paint,
                            child_transform,
                            child_opacity,
                            tile_clip.clip_rect,
                            tile_clip.clip_border_radius,
                            width,
                            height,
                            is_srgb,
                            color_matrices,
                            physical_scale,
                            frame_meshes,
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_paint_generic(
        &mut self,
        backend: &B,
        geometry: &crate::svg::SvgGeometry,
        paint: ResolvedPaint<'_>,
        transform: Mat3,
        opacity: f32,
        clip_rect: [f32; 4],
        clip_border_radius: [f32; 4],
        width: u32,
        height: u32,
        is_srgb: bool,
        color_matrices: &[SvgColorMatrix],
        physical_scale: f32,
        frame_meshes: &mut HashSet<usize>,
    ) {
        match paint {
            ResolvedPaint::Solid { color, style } => self.prepare_mesh_generic(
                backend,
                geometry,
                style,
                transform,
                apply_color_matrices(color, color_matrices),
                opacity,
                clip_rect,
                clip_border_radius,
                width,
                height,
                is_srgb,
                physical_scale,
                frame_meshes,
            ),
            ResolvedPaint::Gradient { gradient, style } => self.prepare_gradient_generic(
                backend,
                geometry,
                style,
                gradient,
                transform,
                opacity,
                clip_rect,
                clip_border_radius,
                width,
                height,
                is_srgb,
                color_matrices,
                physical_scale,
                frame_meshes,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_gradient_generic(
        &mut self,
        backend: &B,
        geometry: &crate::svg::SvgGeometry,
        style: SvgMeshStyle,
        gradient: &SvgGradient,
        transform: Mat3,
        opacity: f32,
        clip_rect: [f32; 4],
        clip_border_radius: [f32; 4],
        width: u32,
        height: u32,
        is_srgb: bool,
        color_matrices: &[SvgColorMatrix],
        physical_scale: f32,
        frame_meshes: &mut HashSet<usize>,
    ) {
        const MAX_GRADIENT_MESH_BYTES: usize = 16 * 1024 * 1024;
        let Ok(base) = self.geometry_cache.mesh_for_with_analytic_aa(
            geometry,
            style,
            physical_scale,
            self.analytic_aa,
        ) else {
            return;
        };
        let geometry_key = Arc::as_ptr(&base) as usize;
        let gradient_key = gradient_fingerprint(gradient);
        let key = (geometry_key, gradient_key);
        if !self.gradient_meshes.contains_key(&key) {
            let expanded = build_gradient_meshes(&base, geometry, gradient);
            let bytes = expanded
                .iter()
                .map(|(mesh, _)| mesh.memory_bytes())
                .sum::<usize>();
            if bytes <= MAX_GRADIENT_MESH_BYTES {
                if self.gradient_mesh_bytes.saturating_add(bytes) > MAX_GRADIENT_MESH_BYTES {
                    self.gradient_meshes.clear();
                    self.gradient_mesh_bytes = 0;
                }
                self.gradient_mesh_bytes += bytes;
                self.gradient_meshes.insert(key, expanded);
            } else {
                return;
            }
        }
        let Some(meshes) = self.gradient_meshes.get(&key).cloned() else {
            return;
        };
        for (mesh, color) in meshes {
            self.prepare_mesh_arc_generic(
                backend,
                mesh,
                transform,
                apply_color_matrices(color, color_matrices),
                opacity,
                clip_rect,
                clip_border_radius,
                width,
                height,
                is_srgb,
                frame_meshes,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_mesh_generic(
        &mut self,
        backend: &B,
        geometry: &crate::svg::SvgGeometry,
        style: SvgMeshStyle,
        transform: Mat3,
        color: SvgColor,
        opacity: f32,
        clip_rect: [f32; 4],
        clip_border_radius: [f32; 4],
        width: u32,
        height: u32,
        is_srgb: bool,
        physical_scale: f32,
        frame_meshes: &mut HashSet<usize>,
    ) {
        let Ok(mesh) = self.geometry_cache.mesh_for_with_analytic_aa(
            geometry,
            style,
            physical_scale,
            self.analytic_aa,
        ) else {
            return;
        };
        self.prepare_mesh_arc_generic(
            backend,
            mesh,
            transform,
            color,
            opacity,
            clip_rect,
            clip_border_radius,
            width,
            height,
            is_srgb,
            frame_meshes,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_mesh_arc_generic(
        &mut self,
        backend: &B,
        mesh: Arc<SvgMesh>,
        transform: Mat3,
        color: SvgColor,
        opacity: f32,
        clip_rect: [f32; 4],
        clip_border_radius: [f32; 4],
        width: u32,
        height: u32,
        is_srgb: bool,
        frame_meshes: &mut HashSet<usize>,
    ) {
        if mesh.indices.is_empty() {
            return;
        }
        let mesh_key = Arc::as_ptr(&mesh) as usize;
        frame_meshes.insert(mesh_key);
        self.ensure_gpu_mesh_generic(backend, mesh_key, &mesh);
        let instance_index = self.instances.len() as u32;
        self.instances.push(SvgInstance {
            transform_x: [
                transform.cols[0][0],
                transform.cols[1][0],
                transform.cols[2][0],
                0.0,
            ],
            transform_y: [
                transform.cols[0][1],
                transform.cols[1][1],
                transform.cols[2][1],
                0.0,
            ],
            color: Rgba8::from_unorm([color.r, color.g, color.b, color.a * opacity]),
            clip_rect,
            clip_border_radius,
            viewport: [
                width as f32,
                height as f32,
                surface_srgb_value(is_srgb),
                0.0,
            ],
        });
        self.prepared_draws.push(PreparedDraw {
            mesh_key,
            instance_index,
        });
    }

    fn ensure_gpu_mesh_generic(&mut self, backend: &B, key: usize, mesh: &Arc<SvgMesh>) {
        use crate::backend::{BufferDescriptor, BufferUsage};
        if let Some(entry) = self.gpu_meshes.get_mut(&key) {
            entry.last_used = self.usage_clock;
            return;
        }
        let vertices = mesh
            .vertices
            .iter()
            .copied()
            .zip(mesh.coverages.iter().copied())
            .map(|(position, coverage)| SvgVertex { position, coverage })
            .collect::<Vec<_>>();
        let vertex_bytes = bytemuck::cast_slice(&vertices);
        let vertex_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("svg vertex buffer".to_string()),
            size: vertex_bytes.len() as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });
        backend.write_buffer(&vertex_buffer, 0, vertex_bytes);
        let index_bytes = bytemuck::cast_slice(&mesh.indices);
        let index_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("svg index buffer".to_string()),
            size: index_bytes.len() as u64,
            usage: vec![BufferUsage::Index, BufferUsage::CopyDst],
        });
        backend.write_buffer(&index_buffer, 0, index_bytes);
        let bytes = mesh.memory_bytes() as u64;
        self.gpu_mesh_bytes += bytes;
        self.gpu_meshes.insert(
            key,
            GpuMesh {
                _mesh: mesh.clone(),
                vertex_buffer,
                index_buffer,
                index_count: mesh.indices.len() as u32,
                bytes,
                last_used: self.usage_clock,
            },
        );
    }

    fn evict_gpu_meshes_generic(&mut self, frame_meshes: &HashSet<usize>) {
        while self.gpu_meshes.len() > self.max_gpu_meshes
            || self.gpu_mesh_bytes > self.max_gpu_mesh_bytes
        {
            let Some(key) = self
                .gpu_meshes
                .iter()
                .filter(|(key, _)| !frame_meshes.contains(key))
                .min_by_key(|(_, mesh)| mesh.last_used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(mesh) = self.gpu_meshes.remove(&key) {
                self.gpu_mesh_bytes = self.gpu_mesh_bytes.saturating_sub(mesh.bytes);
            }
        }
    }

    pub fn draw_item<'a>(&'a self, pass: &mut B::RenderPass<'a>, item_index: usize)
    where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::{GpuRenderPass, IndexFormat};
        let Some(range) = self.item_ranges.get(item_index) else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        for draw in &self.prepared_draws[range.clone()] {
            let Some(mesh) = self.gpu_meshes.get(&draw.mesh_key) else {
                continue;
            };
            pass.set_vertex_buffer(0, &mesh.vertex_buffer, 0);
            pass.set_vertex_buffer(1, &self.instance_buffer, 0);
            pass.set_index_buffer(&mesh.index_buffer, IndexFormat::Uint32, 0);
            pass.draw_indexed(
                0..mesh.index_count,
                0,
                draw.instance_index..draw.instance_index + 1,
            );
        }
    }

    pub fn cpu_geometry_bytes(&self) -> u64 {
        self.geometry_cache.memory_bytes() as u64 + self.gradient_mesh_bytes as u64
    }

    pub fn gpu_geometry_bytes(&self) -> u64 {
        self.gpu_mesh_bytes
    }

    pub fn instance_buffer_bytes(&self) -> u64 {
        (self.instance_policy.capacity() * size_of::<SvgInstance>()) as u64
    }

    pub fn clear_resources(&mut self) {
        self.geometry_cache.clear();
        self.gradient_meshes.clear();
        self.gradient_mesh_bytes = 0;
        self.gpu_meshes.clear();
        self.gpu_mesh_bytes = 0;
    }
}

/// Convert a wgpu vertex format to the backend-agnostic equivalent for the
/// formats used by [`SvgVertex`] and [`SvgInstance`].
#[cfg(feature = "wgpu")]
fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("svg instance buffer"),
        size: (capacity.max(1) * size_of::<SvgInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn combined_transform(
    item: &SvgRenderItem,
    node: &SvgNode,
    node_override: Option<&SvgNodeStyleOverride>,
) -> Mat3 {
    let viewport = item.scene.viewport;
    let destination = Mat3::translate(item.destination.x, item.destination.y).mul(&Mat3::scale(
        item.destination.width / viewport.width.max(f32::EPSILON),
        item.destination.height / viewport.height.max(f32::EPSILON),
    ));
    let transform = node_override
        .and_then(|value| value.transform)
        .unwrap_or(node.transform);
    let node_transform = Mat3 {
        cols: [
            [transform.sx, transform.ky, 0.0],
            [transform.kx, transform.sy, 0.0],
            [transform.tx, transform.ty, 1.0],
        ],
    };
    item.world_transform.mul(&destination).mul(&node_transform)
}

struct NodeEffectState {
    clip_rect: [f32; 4],
    clip_border_radius: [f32; 4],
    mask_opacity: f32,
    offset: [f32; 2],
    color_matrices: Vec<SvgColorMatrix>,
}

fn resolve_node_effects(
    item: &SvgRenderItem,
    node: &SvgNode,
    node_override: Option<&SvgNodeStyleOverride>,
    geometry: &crate::svg::SvgGeometry,
    transform: Mat3,
) -> NodeEffectState {
    let mut state = NodeEffectState {
        clip_rect: item.clip_rect,
        clip_border_radius: item.clip_border_radius,
        mask_opacity: 1.0,
        offset: [0.0; 2],
        color_matrices: Vec::new(),
    };
    let target_bounds = geometry_bounds(geometry);
    let resources = &item.scene.resources;

    if let Some(id) = node.clip_path.as_deref() {
        if let Some(clip) = resources.clip_path(id) {
            if clip.nodes.len() == 1 {
                let clip_node_id = crate::svg::SvgNodeId(clip.nodes[0]);
                if let Some(clip_node) = item.scene.node(clip_node_id) {
                    if let Some(rect) = item
                        .scene
                        .geometry(clip_node)
                        .and_then(rect_geometry_bounds)
                        .and_then(|bounds| {
                            resource_rect_to_screen(
                                item,
                                transform,
                                target_bounds,
                                clip.units,
                                Some(clip_node),
                                bounds,
                            )
                        })
                    {
                        apply_clip_rect(&mut state, rect);
                    }
                }
            }
        }
    }

    if let Some(id) = node.mask.as_deref() {
        if let Some(mask) = resources.mask(id) {
            if mask.nodes.len() == 1 {
                let mask_node_id = crate::svg::SvgNodeId(mask.nodes[0]);
                if let Some(mask_node) = item.scene.node(mask_node_id) {
                    if let Some(rect) = item
                        .scene
                        .geometry(mask_node)
                        .and_then(rect_geometry_bounds)
                        .and_then(|bounds| {
                            resource_rect_to_screen(
                                item,
                                transform,
                                target_bounds,
                                mask.content_units,
                                Some(mask_node),
                                bounds,
                            )
                        })
                    {
                        apply_clip_rect(&mut state, rect);
                        state.mask_opacity *= simple_mask_opacity(mask_node, mask.mask_type);
                    } else {
                        state.mask_opacity = 0.0;
                    }
                } else {
                    state.mask_opacity = 0.0;
                }
            } else {
                state.mask_opacity = 0.0;
            }
            if let Some(region) = resource_rect_to_screen(
                item,
                transform,
                target_bounds,
                mask.units,
                None,
                mask.region,
            ) {
                apply_clip_rect(&mut state, region);
            }
        }
    }

    if let Some(id) = node.filter.as_deref() {
        if let Some(filter) = resources.filter(id) {
            if let Some(region) = resource_rect_to_screen(
                item,
                transform,
                target_bounds,
                filter.units,
                None,
                filter.region,
            ) {
                apply_clip_rect(&mut state, region);
            }
            for primitive in filter.primitives.iter() {
                match primitive {
                    SvgFilterPrimitive::Offset { input, dx, dy } if input_is_graphic(input) => {
                        let scale = match filter.primitive_units {
                            crate::svg::SvgResourceUnits::ObjectBoundingBox => {
                                [target_bounds[2], target_bounds[3]]
                            }
                            crate::svg::SvgResourceUnits::UserSpaceOnUse => [1.0, 1.0],
                        };
                        state.offset[0] += dx * scale[0];
                        state.offset[1] += dy * scale[1];
                    }
                    SvgFilterPrimitive::ColorMatrix { input, matrix }
                        if input_is_graphic(input) =>
                    {
                        state.color_matrices.push(matrix.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = node_override;
    state
}

fn simple_mask_opacity(node: &SvgNode, mask_type: crate::svg::SvgMaskType) -> f32 {
    if !node.visible {
        return 0.0;
    }
    let color = match node.fill_paint.as_ref() {
        Some(SvgPaint::Solid(color)) => Some(*color),
        Some(_) => None,
        None => node.fill.as_ref().map(|fill| fill.color),
    };
    let Some(color) = color else {
        return 0.0;
    };
    let luminance = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
    let value = match mask_type {
        crate::svg::SvgMaskType::Alpha => color.a,
        crate::svg::SvgMaskType::Luminance => color.a * luminance,
    } * node.opacity;
    value.clamp(0.0, 1.0)
}

fn input_is_graphic(input: &SvgFilterInput) -> bool {
    matches!(
        input,
        SvgFilterInput::SourceGraphic | SvgFilterInput::Previous
    )
}

fn apply_color_matrices(mut color: SvgColor, matrices: &[SvgColorMatrix]) -> SvgColor {
    for matrix in matrices {
        let [red, green, blue, alpha] = [color.r, color.g, color.b, color.a];
        match matrix {
            SvgColorMatrix::Matrix(values) if values.len() == 20 => {
                let source = [red, green, blue, alpha, 1.0];
                color.r = values[0..5].iter().zip(source).map(|(a, b)| a * b).sum();
                color.g = values[5..10].iter().zip(source).map(|(a, b)| a * b).sum();
                color.b = values[10..15].iter().zip(source).map(|(a, b)| a * b).sum();
                color.a = values[15..20].iter().zip(source).map(|(a, b)| a * b).sum();
            }
            SvgColorMatrix::Saturate(amount) => {
                let amount = *amount;
                color.r = (0.213 + 0.787 * amount) * red
                    + (0.715 - 0.715 * amount) * green
                    + (0.072 - 0.072 * amount) * blue;
                color.g = (0.213 - 0.213 * amount) * red
                    + (0.715 + 0.285 * amount) * green
                    + (0.072 - 0.072 * amount) * blue;
                color.b = (0.213 - 0.213 * amount) * red
                    + (0.715 - 0.715 * amount) * green
                    + (0.072 + 0.928 * amount) * blue;
            }
            SvgColorMatrix::HueRotate(degrees) => {
                let angle = degrees.to_radians();
                let (sin, cos) = angle.sin_cos();
                color.r = (0.213 + 0.787 * cos - 0.213 * sin) * red
                    + (0.715 - 0.715 * cos - 0.715 * sin) * green
                    + (0.072 - 0.072 * cos + 0.928 * sin) * blue;
                color.g = (0.213 - 0.213 * cos + 0.143 * sin) * red
                    + (0.715 + 0.285 * cos + 0.140 * sin) * green
                    + (0.072 - 0.072 * cos - 0.283 * sin) * blue;
                color.b = (0.213 - 0.213 * cos - 0.787 * sin) * red
                    + (0.715 - 0.715 * cos + 0.715 * sin) * green
                    + (0.072 + 0.928 * cos + 0.072 * sin) * blue;
            }
            SvgColorMatrix::LuminanceToAlpha => {
                color = SvgColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.2126 * red + 0.7152 * green + 0.0722 * blue,
                };
            }
            SvgColorMatrix::Matrix(_) => {}
        }
        color.r = color.r.clamp(0.0, 1.0);
        color.g = color.g.clamp(0.0, 1.0);
        color.b = color.b.clamp(0.0, 1.0);
        color.a = color.a.clamp(0.0, 1.0);
    }
    color
}

fn apply_clip_rect(state: &mut NodeEffectState, next: [f32; 4]) -> bool {
    if next[2] <= 0.0 || next[3] <= 0.0 {
        state.mask_opacity = 0.0;
        return true;
    }
    if state.clip_rect[2] <= 0.0 {
        state.clip_rect = next;
        state.clip_border_radius = [0.0; 4];
        return true;
    }
    if state.clip_border_radius.iter().any(|radius| *radius > 0.0) {
        // The shader carries one rounded rectangle clip. Keep its exact shape
        // when a separate SVG resource cannot be represented at the same time.
        return false;
    }
    let left = state.clip_rect[0].max(next[0]);
    let top = state.clip_rect[1].max(next[1]);
    let right = (state.clip_rect[0] + state.clip_rect[2]).min(next[0] + next[2]);
    let bottom = (state.clip_rect[1] + state.clip_rect[3]).min(next[1] + next[3]);
    state.clip_rect = [left, top, (right - left).max(0.0), (bottom - top).max(0.0)];
    if state.clip_rect[2] <= 0.0 || state.clip_rect[3] <= 0.0 {
        state.mask_opacity = 0.0;
    }
    true
}

fn resource_rect_to_screen(
    item: &SvgRenderItem,
    target_transform: Mat3,
    target_bounds: [f32; 4],
    units: crate::svg::SvgResourceUnits,
    definition_node: Option<&SvgNode>,
    rect: [f32; 4],
) -> Option<[f32; 4]> {
    let root_inverse = svg_transform_matrix(item.scene.resources.root_transform.inverse()?);
    let bbox_transform = match units {
        crate::svg::SvgResourceUnits::ObjectBoundingBox => {
            Mat3::translate(target_bounds[0], target_bounds[1])
                .mul(&Mat3::scale(target_bounds[2], target_bounds[3]))
        }
        crate::svg::SvgResourceUnits::UserSpaceOnUse => Mat3::identity(),
    };
    let mut transform = target_transform.mul(&bbox_transform);
    if let Some(definition_node) = definition_node {
        transform = transform
            .mul(&root_inverse)
            .mul(&svg_transform_matrix(definition_node.transform));
    } else if units == crate::svg::SvgResourceUnits::UserSpaceOnUse {
        transform = transform.mul(&root_inverse);
    }
    axis_aligned_rect(transform, rect)
}

fn svg_transform_matrix(transform: crate::svg::SvgTransform) -> Mat3 {
    Mat3 {
        cols: [
            [transform.sx, transform.ky, 0.0],
            [transform.kx, transform.sy, 0.0],
            [transform.tx, transform.ty, 1.0],
        ],
    }
}

fn rect_geometry_bounds(geometry: &crate::svg::SvgGeometry) -> Option<[f32; 4]> {
    let [
        crate::svg::SvgPathCommand::MoveTo { x: x0, y: y0 },
        crate::svg::SvgPathCommand::LineTo { x: x1, y: y1 },
        crate::svg::SvgPathCommand::LineTo { x: x2, y: y2 },
        crate::svg::SvgPathCommand::LineTo { x: x3, y: y3 },
        crate::svg::SvgPathCommand::Close,
    ] = geometry.commands.as_ref()
    else {
        return None;
    };
    if y0 != y1 || x1 != x2 || y2 != y3 || x3 != x0 || x0 == x1 || y0 == y2 {
        return None;
    }
    Some([x0.min(*x2), y0.min(*y2), (x1 - x0).abs(), (y2 - y1).abs()])
}

fn axis_aligned_rect(transform: Mat3, rect: [f32; 4]) -> Option<[f32; 4]> {
    if !transform
        .cols
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        || transform.cols[0][1].abs() > f32::EPSILON * 8.0
        || transform.cols[1][0].abs() > f32::EPSILON * 8.0
    {
        return None;
    }
    let (x0, y0) = transform.transform_point(rect[0], rect[1]);
    let (x1, y1) = transform.transform_point(rect[0] + rect[2], rect[1] + rect[3]);
    Some([x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()])
}

enum ResolvedPaint<'a> {
    Solid {
        color: SvgColor,
        style: SvgMeshStyle,
    },
    Gradient {
        gradient: &'a SvgGradient,
        style: SvgMeshStyle,
    },
}

fn resolved_fill<'a>(
    node: &'a SvgNode,
    node_override: Option<&SvgNodeStyleOverride>,
) -> Option<ResolvedPaint<'a>> {
    let rule = node
        .fill
        .as_ref()
        .map(|fill| fill.rule)
        .unwrap_or(node.fill_rule);
    let style = SvgMeshStyle::Fill(rule);
    match node_override.map(|value| value.fill) {
        Some(Some(Some(color))) => return Some(ResolvedPaint::Solid { color, style }),
        Some(Some(None)) => return None,
        Some(None) | None => {}
    }
    match node.fill_paint.as_ref() {
        Some(SvgPaint::Solid(color)) => Some(ResolvedPaint::Solid {
            color: *color,
            style,
        }),
        Some(SvgPaint::Linear(gradient) | SvgPaint::Radial(gradient)) => {
            Some(ResolvedPaint::Gradient { gradient, style })
        }
        Some(SvgPaint::Pattern { .. }) => None,
        None => node.fill.as_ref().map(|fill| ResolvedPaint::Solid {
            color: fill.color,
            style,
        }),
    }
}

fn resolved_pattern_fill<'a>(
    node: &'a SvgNode,
    node_override: Option<&SvgNodeStyleOverride>,
) -> Option<&'a str> {
    match node_override.map(|value| value.fill) {
        Some(Some(Some(_))) | Some(Some(None)) => return None,
        Some(None) | None => {}
    }
    match node.fill_paint.as_ref()? {
        SvgPaint::Pattern { id } => Some(id),
        _ => None,
    }
}

fn is_identity_transform(transform: crate::svg::SvgTransform) -> bool {
    let epsilon = f32::EPSILON * 8.0;
    (transform.sx - 1.0).abs() <= epsilon
        && (transform.sy - 1.0).abs() <= epsilon
        && transform.ky.abs() <= epsilon
        && transform.kx.abs() <= epsilon
        && transform.tx.abs() <= epsilon
        && transform.ty.abs() <= epsilon
}

fn resolved_stroke<'a>(
    node: &'a SvgNode,
    node_override: Option<&SvgNodeStyleOverride>,
) -> Option<ResolvedPaint<'a>> {
    let stroke = node.stroke.as_ref()?;
    if !stroke.dash_array.is_empty() {
        return None;
    }
    let style = SvgMeshStyle::Stroke {
        width: stroke.width,
        line_cap: stroke.line_cap,
        line_join: stroke.line_join,
        miter_limit: stroke.miter_limit,
    };
    match node_override.map(|value| value.stroke) {
        Some(Some(Some(color))) => return Some(ResolvedPaint::Solid { color, style }),
        Some(Some(None)) => return None,
        Some(None) | None => {}
    }
    match node.stroke_paint.as_ref() {
        Some(SvgPaint::Solid(color)) => Some(ResolvedPaint::Solid {
            color: *color,
            style,
        }),
        Some(SvgPaint::Linear(gradient) | SvgPaint::Radial(gradient)) => {
            Some(ResolvedPaint::Gradient { gradient, style })
        }
        Some(SvgPaint::Pattern { .. }) => None,
        None => Some(ResolvedPaint::Solid {
            color: stroke.color,
            style,
        }),
    }
}

fn gradient_fingerprint(gradient: &SvgGradient) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    gradient.id().hash(&mut hasher);
    match gradient {
        SvgGradient::Linear {
            x1,
            y1,
            x2,
            y2,
            units,
            transform,
            spread,
            stops,
            ..
        } => {
            0_u8.hash(&mut hasher);
            for value in [*x1, *y1, *x2, *y2] {
                value.to_bits().hash(&mut hasher);
            }
            (*units as u8).hash(&mut hasher);
            (*spread as u8).hash(&mut hasher);
            hash_transform(*transform, &mut hasher);
            hash_stops(stops, &mut hasher);
        }
        SvgGradient::Radial {
            cx,
            cy,
            radius,
            fx,
            fy,
            focal_radius,
            units,
            transform,
            spread,
            stops,
            ..
        } => {
            1_u8.hash(&mut hasher);
            for value in [*cx, *cy, *radius, *fx, *fy, *focal_radius] {
                value.to_bits().hash(&mut hasher);
            }
            (*units as u8).hash(&mut hasher);
            (*spread as u8).hash(&mut hasher);
            hash_transform(*transform, &mut hasher);
            hash_stops(stops, &mut hasher);
        }
    }
    hasher.finish()
}

fn hash_transform(transform: crate::svg::SvgTransform, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    for value in [
        transform.sx,
        transform.ky,
        transform.kx,
        transform.sy,
        transform.tx,
        transform.ty,
    ] {
        value.to_bits().hash(hasher);
    }
}

fn hash_stops(stops: &[crate::svg::SvgGradientStop], hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    for stop in stops {
        stop.offset.to_bits().hash(hasher);
        for value in [stop.color.r, stop.color.g, stop.color.b, stop.color.a] {
            value.to_bits().hash(hasher);
        }
    }
}

fn build_gradient_meshes(
    base: &SvgMesh,
    geometry: &crate::svg::SvgGeometry,
    gradient: &SvgGradient,
) -> Vec<(Arc<SvgMesh>, SvgColor)> {
    const SEGMENTS: usize = 32;
    const MAX_TRIANGLES: usize = 300_000;
    let bounds = geometry_bounds(geometry);
    if bounds[2] <= 0.0 || bounds[3] <= 0.0 || base.indices.len() < 3 {
        return Vec::new();
    }
    let source_triangles = base.indices.len() / 3;
    if source_triangles > MAX_TRIANGLES {
        return Vec::new();
    }
    let subdivisions = ((MAX_TRIANGLES / source_triangles.max(1)) as f32)
        .sqrt()
        .floor()
        .clamp(1.0, 8.0) as usize;
    let mut buckets: Vec<(Vec<[f32; 2]>, Vec<f32>, Vec<u32>, Option<SvgColor>)> = (0..SEGMENTS)
        .map(|_| (Vec::new(), Vec::new(), Vec::new(), None))
        .collect();

    let mut push_triangle = |triangle: [([f32; 2], f32); 3]| {
        let center = [
            (triangle[0].0[0] + triangle[1].0[0] + triangle[2].0[0]) / 3.0,
            (triangle[0].0[1] + triangle[1].0[1] + triangle[2].0[1]) / 3.0,
        ];
        let Some(position) = gradient.position_at(center[0], center[1], bounds) else {
            return;
        };
        let bucket_index = ((position * SEGMENTS as f32).floor() as isize)
            .clamp(0, SEGMENTS as isize - 1) as usize;
        let Some(color) = gradient.color_at_position((bucket_index as f32 + 0.5) / SEGMENTS as f32)
        else {
            return;
        };
        let bucket = &mut buckets[bucket_index];
        let start = bucket.0.len() as u32;
        for (position, coverage) in triangle {
            bucket.0.push(position);
            bucket.1.push(coverage);
        }
        bucket.2.extend([start, start + 1, start + 2]);
        bucket.3 = Some(color);
    };

    for indices in base.indices.chunks_exact(3) {
        let Some(a) = base
            .vertices
            .get(indices[0] as usize)
            .zip(base.coverages.get(indices[0] as usize))
        else {
            continue;
        };
        let Some(b) = base
            .vertices
            .get(indices[1] as usize)
            .zip(base.coverages.get(indices[1] as usize))
        else {
            continue;
        };
        let Some(c) = base
            .vertices
            .get(indices[2] as usize)
            .zip(base.coverages.get(indices[2] as usize))
        else {
            continue;
        };
        let point = |i: usize, j: usize| {
            let u = i as f32 / subdivisions as f32;
            let v = j as f32 / subdivisions as f32;
            let w = 1.0 - u - v;
            (
                [
                    a.0[0] * w + b.0[0] * u + c.0[0] * v,
                    a.0[1] * w + b.0[1] * u + c.0[1] * v,
                ],
                *a.1 * w + *b.1 * u + *c.1 * v,
            )
        };
        for i in 0..subdivisions {
            for j in 0..subdivisions - i {
                let first = [point(i, j), point(i + 1, j), point(i, j + 1)];
                push_triangle(first);
                if i + j + 2 <= subdivisions {
                    let second = [point(i + 1, j), point(i + 1, j + 1), point(i, j + 1)];
                    push_triangle(second);
                }
            }
        }
    }

    buckets
        .into_iter()
        .filter_map(|(vertices, coverages, indices, color)| {
            let color = color?;
            Some((
                Arc::new(SvgMesh {
                    vertices: vertices.into(),
                    coverages: coverages.into(),
                    indices: indices.into(),
                }),
                color,
            ))
        })
        .collect()
}

fn geometry_bounds(geometry: &crate::svg::SvgGeometry) -> [f32; 4] {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for command in geometry.commands.iter() {
        let mut include = |x: f32, y: f32| {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        };
        match command {
            crate::svg::SvgPathCommand::MoveTo { x, y }
            | crate::svg::SvgPathCommand::LineTo { x, y } => include(*x, *y),
            crate::svg::SvgPathCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                include(*control_x, *control_y);
                include(*x, *y);
            }
            crate::svg::SvgPathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                include(*control1_x, *control1_y);
                include(*control2_x, *control2_y);
                include(*x, *y);
            }
            crate::svg::SvgPathCommand::Close => {}
        }
    }
    [
        bounds[0],
        bounds[1],
        bounds[2] - bounds[0],
        bounds[3] - bounds[1],
    ]
}

fn transform_scale(transform: Mat3) -> f32 {
    // SVG item transforms land in physical target pixels, including widget
    // sizing through the window's device scale.
    let x = transform.cols[0][0].hypot(transform.cols[0][1]);
    let y = transform.cols[1][0].hypot(transform.cols[1][1]);
    let scale = x.max(y);
    if scale.is_finite() && scale > 0.0 {
        scale.max(f32::EPSILON)
    } else if scale.is_infinite() {
        f32::MAX
    } else {
        1.0
    }
}

#[cfg(test)]
mod tessellation_scale_tests {
    use super::transform_scale;
    use crate::utilities::Mat3;

    #[test]
    fn svg_tessellation_scale_uses_the_svg_to_framebuffer_transform() {
        assert_eq!(transform_scale(Mat3::scale(2.0, 3.0)), 3.0);
    }
}

fn outside_viewport(
    transform: Mat3,
    geometry: &crate::svg::SvgGeometry,
    width: u32,
    height: u32,
) -> bool {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for command in geometry.commands.iter() {
        let point = match *command {
            crate::svg::SvgPathCommand::MoveTo { x, y }
            | crate::svg::SvgPathCommand::LineTo { x, y } => Some((x, y)),
            crate::svg::SvgPathCommand::QuadraticTo { x, y, .. }
            | crate::svg::SvgPathCommand::CubicTo { x, y, .. } => Some((x, y)),
            crate::svg::SvgPathCommand::Close => None,
        };
        if let Some((x, y)) = point {
            let (x, y) = transform.transform_point(x, y);
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    max_x < 0.0 || max_y < 0.0 || min_x > width as f32 || min_y > height as f32
}

#[cfg(target_os = "android")]
fn surface_srgb_value(_: bool) -> f32 {
    2.0
}

#[cfg(not(target_os = "android"))]
fn surface_srgb_value(is_srgb: bool) -> f32 {
    if is_srgb { 1.0 } else { 0.0 }
}

#[cfg(all(test, feature = "wgpu"))]
mod tests {

    #[test]
    fn svg_pipeline_uses_configured_antialiasing() {
        assert_eq!(
            crate::pipeline::multisample_state(crate::AntiAlias::Analytic).count,
            1
        );
        assert_eq!(
            crate::pipeline::multisample_state(crate::AntiAlias::Msaa4x).count,
            4
        );
    }

    #[test]
    fn svg_instance_uses_unorm_bytes_for_paint_color() {
        assert_eq!(std::mem::size_of::<super::SvgInstance>(), 84);
        assert_eq!(
            super::SvgInstance::ATTRIBUTES[2].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            super::SvgInstance::ATTRIBUTES[2].offset,
            std::mem::offset_of!(super::SvgInstance, color) as u64
        );
    }
}

#[cfg(test)]
mod resource_tests {
    use std::sync::Arc;

    use crate::svg::{
        SvgColor, SvgGradient, SvgGradientStop, SvgGradientUnits, SvgSpreadMethod, SvgTransform,
        parse_svg_document,
    };
    use crate::utilities::{Mat3, Rect};

    #[test]
    fn rectangular_clip_path_and_alpha_mask_restrict_the_node() {
        let parsed = parse_svg_document(
            br##"<svg width="10" height="10"><defs>
                <clipPath id="clip" clipPathUnits="objectBoundingBox"><rect width="0.5" height="1"/></clipPath>
                <mask id="mask" mask-type="alpha"><rect width="10" height="10" fill="#ffffff"/></mask>
            </defs><path id="target" d="M0 0h10v10z" clip-path="url(#clip)" mask="url(#mask)"/></svg>"##,
        ).unwrap();
        let scene = Arc::new(parsed.scene);
        let target = scene
            .nodes
            .iter()
            .find(|node| node.svg_id.as_deref() == Some("target"))
            .unwrap();
        let item = crate::renderer::SvgRenderItem {
            scene: scene.clone(),
            destination: Rect::new(0.0, 0.0, 10.0, 10.0),
            overrides: Arc::from([]),
            world_transform: Mat3::identity(),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            opacity: 1.0,
        };
        let geometry = scene.geometry(target).unwrap();
        let effects = super::resolve_node_effects(
            &item,
            target,
            None,
            geometry,
            super::combined_transform(&item, target, None),
        );

        assert_eq!(effects.clip_rect, [0.0, 0.0, 5.0, 10.0]);
        assert_eq!(effects.mask_opacity, 1.0);
    }

    #[test]
    fn clip_path_transform_is_applied_to_its_geometry() {
        let parsed = parse_svg_document(
            br#"<svg width="10" height="10"><defs><clipPath id="clip" clipPathUnits="userSpaceOnUse" transform="translate(2 1)"><rect width="5" height="4"/></clipPath></defs><rect id="target" width="10" height="10" clip-path="url(#clip)"/></svg>"#,
        ).unwrap();
        let scene = Arc::new(parsed.scene);
        let target = scene
            .nodes
            .iter()
            .find(|node| node.svg_id.as_deref() == Some("target"))
            .unwrap();
        let item = crate::renderer::SvgRenderItem {
            scene: scene.clone(),
            destination: Rect::new(0.0, 0.0, 10.0, 10.0),
            overrides: Arc::from([]),
            world_transform: Mat3::identity(),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            opacity: 1.0,
        };
        let effects = super::resolve_node_effects(
            &item,
            target,
            None,
            scene.geometry(target).unwrap(),
            super::combined_transform(&item, target, None),
        );

        assert_eq!(effects.clip_rect, [2.0, 1.0, 5.0, 4.0]);
    }

    #[test]
    fn clip_path_applies_geometry_referenced_through_use() {
        let parsed = parse_svg_document(
            br##"<svg width="10" height="10">
                <defs><path id="clip-shape" d="M0 0h4v8h-4z"/></defs>
                <clipPath id="clip" clipPathUnits="userSpaceOnUse">
                    <use href="#clip-shape" x="2" y="1"/>
                </clipPath>
                <path id="target" d="M0 0h10v10z" clip-path="url(#clip)"/>
            </svg>"##,
        )
        .unwrap();
        let scene = Arc::new(parsed.scene);
        let target = scene
            .nodes
            .iter()
            .find(|node| node.svg_id.as_deref() == Some("target"))
            .unwrap();
        let item = crate::renderer::SvgRenderItem {
            scene: scene.clone(),
            destination: Rect::new(0.0, 0.0, 10.0, 10.0),
            overrides: Arc::from([]),
            world_transform: Mat3::identity(),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            opacity: 1.0,
        };
        let effects = super::resolve_node_effects(
            &item,
            target,
            None,
            scene.geometry(target).unwrap(),
            super::combined_transform(&item, target, None),
        );

        assert_eq!(effects.clip_rect, [2.0, 1.0, 4.0, 8.0]);
    }

    #[test]
    fn gradient_meshes_preserve_geometry_and_vary_paint_bands() {
        let parsed = parse_svg_document(
            br##"<svg width="10" height="10"><defs><linearGradient id="g"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><path d="M0 0h10v10z" fill="url(#g)"/></svg>"##,
        ).unwrap();
        let node = parsed
            .scene
            .nodes
            .iter()
            .find(|node| !node.is_definition)
            .unwrap();
        let geometry = parsed.scene.geometry(node).unwrap();
        let gradient = match node.fill_paint.as_ref().unwrap() {
            crate::svg::SvgPaint::Linear(gradient) => gradient,
            paint => panic!("unexpected paint {paint:?}"),
        };
        let mut cache = crate::svg::SvgGeometryCache::new(1024 * 1024, 4);
        let base = cache
            .mesh_for(
                geometry,
                crate::svg::SvgMeshStyle::Fill(node.fill_rule),
                1.0,
            )
            .unwrap();
        let bands = super::build_gradient_meshes(&base, geometry, gradient);

        assert!(bands.len() > 1);
        assert!(bands.iter().all(|(mesh, _)| !mesh.indices.is_empty()));
        assert!(bands.iter().any(|(_, color)| color.r > color.b));
        assert!(bands.iter().any(|(_, color)| color.b > color.r));
    }

    #[test]
    fn gradients_sample_pad_and_reflect_stops() {
        let gradient = SvgGradient::Linear {
            id: Arc::from("g"),
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 0.0,
            units: SvgGradientUnits::ObjectBoundingBox,
            transform: SvgTransform::default(),
            spread: SvgSpreadMethod::Reflect,
            stops: Arc::from([
                SvgGradientStop {
                    offset: 0.0,
                    color: SvgColor::rgba8(255, 0, 0, 255),
                },
                SvgGradientStop {
                    offset: 1.0,
                    color: SvgColor::rgba8(0, 0, 255, 255),
                },
            ]),
        };
        let start = gradient.sample_at(0.0, 0.5, [0.0, 0.0, 1.0, 1.0]).unwrap();
        let middle = gradient.sample_at(0.5, 0.5, [0.0, 0.0, 1.0, 1.0]).unwrap();
        let reflected = gradient.sample_at(1.5, 0.5, [0.0, 0.0, 1.0, 1.0]).unwrap();

        assert_eq!(start, SvgColor::rgba8(255, 0, 0, 255));
        assert!((middle.r - middle.b).abs() < 0.01);
        assert_eq!(reflected, middle);
    }

    #[test]
    fn radial_gradient_sampling_respects_focal_center_and_radius() {
        let gradient = SvgGradient::Radial {
            id: Arc::from("radial"),
            cx: 0.5,
            cy: 0.5,
            radius: 0.5,
            fx: 0.25,
            fy: 0.5,
            focal_radius: 0.1,
            units: SvgGradientUnits::ObjectBoundingBox,
            transform: SvgTransform::default(),
            spread: SvgSpreadMethod::Pad,
            stops: Arc::from([
                SvgGradientStop {
                    offset: 0.0,
                    color: SvgColor::rgba8(255, 0, 0, 255),
                },
                SvgGradientStop {
                    offset: 1.0,
                    color: SvgColor::rgba8(0, 0, 255, 255),
                },
            ]),
        };

        let position = gradient
            .position_at(0.75, 0.5, [0.0, 0.0, 1.0, 1.0])
            .unwrap();
        assert!((position - (8.0 / 13.0)).abs() < 0.0001);
    }

    #[test]
    fn offset_and_color_matrix_filter_primitives_reach_node_render_state() {
        let parsed = parse_svg_document(
            br#"<svg width="10" height="10"><defs><filter id="fx"><feOffset dx="2" dy="-1"/><feColorMatrix type="matrix" values="0 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0 1 0"/></filter></defs><rect id="target" width="10" height="10" fill="red" filter="url(#fx)"/></svg>"#,
        ).unwrap();
        let scene = Arc::new(parsed.scene);
        let target = scene
            .nodes
            .iter()
            .find(|node| node.svg_id.as_deref() == Some("target"))
            .unwrap();
        let item = crate::renderer::SvgRenderItem {
            scene: scene.clone(),
            destination: Rect::new(0.0, 0.0, 10.0, 10.0),
            overrides: Arc::from([]),
            world_transform: Mat3::identity(),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            opacity: 1.0,
        };
        let effects = super::resolve_node_effects(
            &item,
            target,
            None,
            scene.geometry(target).unwrap(),
            super::combined_transform(&item, target, None),
        );
        let blue =
            super::apply_color_matrices(SvgColor::rgba8(255, 0, 0, 255), &effects.color_matrices);

        assert_eq!(effects.offset, [2.0, -1.0]);
        assert_eq!(effects.clip_rect, [-1.0, -1.0, 12.0, 12.0]);
        assert_eq!(effects.color_matrices.len(), 1);
        assert_eq!(blue, SvgColor::rgba8(0, 0, 255, 255));
    }
}
