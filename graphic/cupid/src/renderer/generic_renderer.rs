#[path = "generic_renderer/material_detection.rs"]
mod material_detection;
use material_detection::{
    draw_list_uses_material, groups_sampling_backdrop, render_plan_uses_material,
};

use std::collections::HashMap;
use std::sync::Arc;

#[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
use aimer_time::Instant;

#[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
use aimer_utils::info;

use crate::backend::{
    GpuBackend, LoadOp, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
};
use crate::compositor::{CompositorScene, CompositorStats, RetainedSceneTree};
use crate::custom_pipeline::{
    CustomPipelineGeneric, CustomPipelineSlotGeneric, RenderContextGeneric,
};
use crate::damage_region::DamageRect;
use crate::draw_cmd::{DrawCommand, DrawList, RetainedLayerContent};
use crate::frame::{
    FramePacket, FrameRenderMetadata, RetainedRenderOperationKind, RetainedRenderPlan,
    RetainedV2Item,
};
use crate::persistent_target::{
    PersistentTargetGeneric, PersistentTargetKey, TargetEnsureResult, TargetValidity,
};
use crate::pipeline::frame_composite::FrameCompositePipeline;
use crate::pipeline::image_pipeline::{ImageInstance, ImagePipeline};
use crate::pipeline::material::{MATERIAL_PIPELINE_NAME, MaterialPipeline};
use crate::pipeline::morph_pipeline::MorphPipeline;
use crate::pipeline::rect_pipeline::{RectInstance, RectPipeline};
use crate::pipeline::svg_pipeline::SvgPipeline;
use crate::pipeline::text_pipeline::{
    RichTextSpan, TextDecorationDraw, TextDrawRequest, TextOverflowMode, TextPipelineV2,
    text_layout::{TextHorizontalAlign, TextWritingMode},
};
use crate::utilities::{Mat3, Rect, Rgba8, TextureId};

use super::{
    AlphaState, ClipState, SvgRenderItem, apply_alpha, build_fill_rect_instances,
    clip_border_radius, clip_to_array, packed_color, resolve_material_request, resolve_svg_item,
    retained_layer_dimensions, transform_scales, transform_text_shadow,
};

mod damage_clear;
mod frame_recovery;
mod morph_group;
use damage_clear::DamageClearTextureGeneric;
use morph_group::{MorphFrame, MorphSource, ends_host, next_group_depth};

const RETAINED_LAYER_CACHE_BUDGET_BYTES: u64 = 64 * 1024 * 1024;
const RETAINED_LAYER_IDLE_FRAMES: u64 = 120;

#[cfg(all(test, target_os = "macos", feature = "native"))]
#[path = "generic_renderer/damage_clear_tests.rs"]
mod damage_clear_tests;

/// GPU memory and cache measurements exposed by the generic renderer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RendererMemoryStats {
    pub image_texture_count: usize,
    pub image_texture_bytes: u64,
    pub retained_layer_count: usize,
    pub retained_layer_bytes: u64,
    pub glyph_atlas_bytes: u64,
    pub glyph_bitmap_cache_entries: usize,
    pub glyph_bitmap_cache_bytes: usize,
    pub instance_buffer_bytes: u64,
    pub svg_geometry_cpu_bytes: u64,
    pub svg_geometry_gpu_bytes: u64,
    pub svg_instance_buffer_bytes: u64,
    pub multisample_target_bytes: u64,
}

#[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
macro_rules! dx12_stage_start {
    ($enabled:expr) => {
        $enabled.then(Instant::now)
    };
}

#[cfg(not(all(target_os = "windows", feature = "native", not(feature = "wgpu"))))]
macro_rules! dx12_stage_start {
    ($enabled:expr) => {{
        let _ = $enabled;
        ()
    }};
}

#[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
macro_rules! dx12_stage_finish {
    ($started:expr, $label:literal) => {{
        if let Some(started) = $started {
            info!(
                "DX12 renderer first-frame stage: {} {:.2} ms (CPU elapsed)",
                $label,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }};
}

#[cfg(not(all(target_os = "windows", feature = "native", not(feature = "wgpu"))))]
macro_rules! dx12_stage_finish {
    ($started:expr, $label:literal) => {{
        let _ = $started;
    }};
}

/// Backend-generic renderer using the selected platform backend by default.
pub struct Renderer<B: GpuBackend = crate::backend::DefaultGpuBackend> {
    rect_pipeline: RectPipeline<B>,
    image_pipeline: Option<ImagePipeline<B>>,
    text_pipeline: Option<TextPipelineV2<B>>,
    svg_pipeline: Option<SvgPipeline<B>>,
    frame_composite_pipeline: Option<FrameCompositePipeline<B>>,
    /// Made on the first morph group, which most frames never have.
    morph_pipeline: Option<MorphPipeline<B>>,
    material_target: PersistentTargetGeneric<B>,
    material_composite_bind_group: Option<B::BindGroup>,
    multisample_target: Option<MultisampleTargetGeneric<B>>,
    scene_target: PersistentTargetGeneric<B>,
    scene_composite_bind_group: Option<B::BindGroup>,
    damage_clear_texture: Option<DamageClearTextureGeneric<B>>,
    opacity_group_targets: Vec<OpacityGroupTargetGeneric<B>>,
    /// Set while a chunk is drawn into a seeded opacity group; see
    /// [`ActiveOpacityGroup::seeded`].
    group_backdrop: Option<GroupBackdrop<B>>,
    /// A copy of the frame behind a seeded group, for the multisampled case, where
    /// the group's own target cannot be filled by a copy.
    group_seed_target: PersistentTargetGeneric<B>,
    group_seed_bind_group: Option<B::BindGroup>,
    custom_pipelines: Vec<CustomPipelineSlotGeneric<B>>,
    antialiasing: crate::AntiAlias,
    material_pipeline_enabled: bool,
    material_pipeline_initialized: bool,
    /// Frames this renderer has started; see [`Self::prewarm_material_pipeline`].
    frames_started: u32,
    format: B::TextureFormat,
    resolved: Vec<ResolvedCommand<B>>,
    text_requests: Vec<TextDrawRequest>,
    decoration_requests: Vec<TextDecorationDraw>,
    svg_items: Vec<SvgRenderItem>,
    deferred_outlines: Vec<RectInstance>,
    textures_to_remove: Vec<TextureId>,
    image_batch: Vec<ImageInstance>,
    transform_stack: Vec<Mat3>,
    clip_stack: Vec<ClipState>,
    retained_layers: HashMap<u64, RetainedLayerGeneric<B>>,
    frame_index: u64,
    compositor_stats: CompositorStats,
    scene_tree: RetainedSceneTree,
    scene_history_key: Option<(u64, u64, u64, u64, u32, u32)>,
    v2_command_cache: HashMap<u64, CachedV2CommandList>,
    #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
    first_render_timing_enabled: bool,
}

enum ResolvedCommand<B: GpuBackend> {
    Rect {
        instance: RectInstance,
        clear: bool,
    },
    Image {
        texture_id: TextureId,
        instance: ImageInstance,
    },
    RetainedLayer {
        bind_group: B::BindGroup,
        instance: ImageInstance,
    },
    Text(usize),
    TextDecoration {
        start: usize,
        end: usize,
    },
    Svg(usize),
    Custom {
        pipeline_index: usize,
        command_index: Option<usize>,
    },
}

struct RetainedLayerGeneric<B: GpuBackend> {
    content: Arc<RetainedLayerContent>,
    target: PersistentTargetGeneric<B>,
    bind_group: Option<B::BindGroup>,
    width: u32,
    height: u32,
    bytes: u64,
    last_used_frame: u64,
    is_srgb: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedLayerPreparation {
    Rasterized,
    Reused,
    Skipped,
}

struct MultisampleTargetGeneric<B: GpuBackend> {
    _texture: B::Texture,
    view: B::TextureView,
    width: u32,
    height: u32,
    sample_count: u32,
    format: B::TextureFormat,
    bytes: u64,
}

struct OpacityGroupTargetGeneric<B: GpuBackend> {
    target: PersistentTargetGeneric<B>,
    bind_group: Option<B::BindGroup>,
    multisample_target: Option<MultisampleTargetGeneric<B>>,
}

impl<B: GpuBackend> Default for OpacityGroupTargetGeneric<B> {
    fn default() -> Self {
        Self {
            target: PersistentTargetGeneric::default(),
            bind_group: None,
            multisample_target: None,
        }
    }
}

/// What an active group is for in a mask composite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaskRole {
    /// An ordinary opacity group.
    None,
    /// A masked group still waiting for the node's own item, its content.
    AwaitingContent,
    /// A masked group whose content is drawn and whose coverage group is above it.
    HasCoverage,
    /// The coverage target of a masked group: a mask, never shown itself.
    Coverage,
    /// A fade group still waiting for the node's own item, its alpha.
    AwaitingAlpha,
    /// A fade group whose alpha is drawn and whose content group is above it.
    HasContent,
    /// The content target of a fade group: it is scaled by the alpha below it,
    /// then composited like any opacity group.
    Content,
}

struct ActiveOpacityGroup<B: GpuBackend> {
    element: u64,
    mask: MaskRole,
    /// Set for the source groups of a morph, which are kept for the host
    /// instead of being composited into their parent.
    morph_source: Option<MorphSource>,
    /// The single-sample view of the group's target, readable by a shader.
    sample_view: B::TextureView,
    opacity: f32,
    bounds: DamageRect,
    depth: usize,
    bind_group: B::BindGroup,
    render_view: B::TextureView,
    resolve_target: Option<B::TextureView>,
    /// Whether the group's target was started from a copy of the frame behind it
    /// and not from nothing, so that a surface inside it that samples the backdrop
    /// sees what the group drew before it, as it does outside a group.
    ///
    /// Without it the backdrop is the frame behind the group alone, and a filter
    /// drawn after a tint covers the tint with the filtered frame: a glass surface
    /// loses its tint for as long as it is faded, and has it only once the fade has
    /// ended and the group with it. The group is composited by lerping from the
    /// frame to its target, which a target started from the frame leaves the same
    /// for everything that does not sample the backdrop.
    seeded: bool,
}

/// Where backdrop surfaces read the backdrop from while they are drawn into a
/// seeded group: the group's own target, whose origin is its own corner.
struct GroupBackdrop<B: GpuBackend> {
    texture: B::Texture,
}

struct CachedV2CommandList {
    revision: u64,
    scale_bits: u32,
    origin_bits: (u32, u32),
    transform_bits: [u32; 9],
    /// Clip rectangle bits and corner-radius bits; a radius-only change must
    /// not replay commands lowered with the previous corners.
    clip_bits: Option<([u32; 4], [u32; 4])>,
    commands: Vec<DrawCommand>,
    last_used_frame: u64,
}

impl<B: GpuBackend> Renderer<B> {
    /// Creates the built-in pipelines using the selected backend.
    ///
    /// The material pipeline is created on the first material draw.
    pub fn new(backend: &B, format: B::TextureFormat) -> Self {
        Self::with_antialiasing(backend, format, crate::AntiAlias::default())
    }

    /// Creates the built-in pipelines using the selected backend and antialiasing mode.
    ///
    /// The material pipeline is created on the first material draw.
    pub fn with_antialiasing(
        backend: &B,
        format: B::TextureFormat,
        antialiasing: crate::AntiAlias,
    ) -> Self {
        Self::build(backend, format, antialiasing, true)
    }

    /// Creates a renderer with only the rectangle pipeline initialized.
    ///
    /// This supports staged backend bring-up or callers that only need
    /// rectangles. Draw commands requiring another pipeline will fail with a
    /// clear initialization message.
    pub fn new_rect_only(backend: &B, format: B::TextureFormat) -> Self {
        Self::build(backend, format, crate::AntiAlias::default(), false)
    }

    fn build(
        backend: &B,
        format: B::TextureFormat,
        antialiasing: crate::AntiAlias,
        initialize_all: bool,
    ) -> Self {
        let rect_pipeline = RectPipeline::<B>::new(backend, format, antialiasing);
        let image_pipeline =
            initialize_all.then(|| ImagePipeline::<B>::new(backend, format, antialiasing));
        let text_pipeline =
            initialize_all.then(|| TextPipelineV2::<B>::new(backend, format, antialiasing));
        let svg_pipeline =
            initialize_all.then(|| SvgPipeline::<B>::new(backend, format, antialiasing));
        let frame_composite_pipeline =
            initialize_all.then(|| FrameCompositePipeline::<B>::new(backend, format));
        Self {
            rect_pipeline,
            image_pipeline,
            text_pipeline,
            svg_pipeline,
            frame_composite_pipeline,
            morph_pipeline: None,
            material_target: PersistentTargetGeneric::default(),
            material_composite_bind_group: None,
            multisample_target: None,
            scene_target: PersistentTargetGeneric::default(),
            scene_composite_bind_group: None,
            damage_clear_texture: None,
            opacity_group_targets: Vec::new(),
            group_backdrop: None,
            group_seed_target: PersistentTargetGeneric::default(),
            group_seed_bind_group: None,
            custom_pipelines: Vec::new(),
            antialiasing,
            material_pipeline_enabled: initialize_all,
            material_pipeline_initialized: false,
            frames_started: 0,
            format,
            resolved: Vec::new(),
            text_requests: Vec::new(),
            decoration_requests: Vec::new(),
            svg_items: Vec::new(),
            deferred_outlines: Vec::new(),
            textures_to_remove: Vec::new(),
            image_batch: Vec::new(),
            transform_stack: Vec::new(),
            clip_stack: Vec::new(),
            retained_layers: HashMap::new(),
            frame_index: 0,
            compositor_stats: CompositorStats::default(),
            scene_tree: RetainedSceneTree::new(),
            scene_history_key: None,
            v2_command_cache: HashMap::new(),
            #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
            first_render_timing_enabled: false,
        }
    }

    /// Enables a one-shot breakdown of the next DX12 renderer frame.
    #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
    #[doc(hidden)]
    pub fn enable_first_render_timing(&mut self) {
        self.first_render_timing_enabled = true;
    }

    /// Returns the render-target format selected at construction.
    #[inline]
    pub fn surface_format(&self) -> B::TextureFormat {
        self.format
    }

    /// Whether the previous render left off-screen text unprepared for lack of budget.
    #[inline]
    pub fn has_postponed_text_preparation(&self) -> bool {
        self.text_pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.has_postponed_preparation())
    }

    /// Returns the renderer's current GPU memory and cache measurements.
    pub fn memory_stats(&self) -> RendererMemoryStats {
        let image = self.image_pipeline.as_ref();
        let text = self.text_pipeline.as_ref();
        let svg = self.svg_pipeline.as_ref();
        RendererMemoryStats {
            image_texture_count: image.map_or(0, ImagePipeline::texture_count),
            image_texture_bytes: image.map_or(0, ImagePipeline::texture_bytes),
            retained_layer_count: self.retained_layers.len(),
            retained_layer_bytes: self
                .retained_layers
                .values()
                .fold(0u64, |sum, layer| sum.saturating_add(layer.bytes)),
            glyph_atlas_bytes: text.map_or(0, TextPipelineV2::glyph_atlas_bytes),
            glyph_bitmap_cache_entries: text.map_or(0, TextPipelineV2::cached_glyph_count),
            glyph_bitmap_cache_bytes: text.map_or(0, TextPipelineV2::glyph_bitmap_cache_bytes),
            instance_buffer_bytes: self.rect_pipeline.instance_buffer_bytes()
                + image.map_or(0, ImagePipeline::instance_buffer_bytes)
                + text.map_or(0, TextPipelineV2::instance_buffer_bytes)
                + svg.map_or(0, SvgPipeline::instance_buffer_bytes),
            svg_geometry_cpu_bytes: svg.map_or(0, SvgPipeline::cpu_geometry_bytes),
            svg_geometry_gpu_bytes: svg.map_or(0, SvgPipeline::gpu_geometry_bytes),
            svg_instance_buffer_bytes: svg.map_or(0, SvgPipeline::instance_buffer_bytes),
            multisample_target_bytes: self
                .multisample_target
                .as_ref()
                .map_or(0, |target| target.bytes),
        }
    }

    /// Returns the work counters from the most recently rendered frame.
    #[inline]
    pub fn compositor_stats(&self) -> CompositorStats {
        self.compositor_stats
    }

    /// Clears CPU and GPU SVG geometry retained by the renderer.
    pub fn clear_svg_resources(&mut self) {
        if let Some(svg_pipeline) = self.svg_pipeline.as_mut() {
            svg_pipeline.clear_resources();
        }
    }

    /// Saves any backend-managed pipeline cache to persistent storage.
    pub fn save_pipeline_cache(&self, backend: &B) {
        backend.save_pipeline_cache();
    }

    /// Prepares text and its glyph atlas before the first visible frame.
    pub fn preload_text(&mut self, backend: &B, text: &str, font_size: f32) {
        self.warm_text(backend, text, font_size, 0.0);
    }

    /// Prepares a common ASCII set at each requested font size.
    pub fn warm_glyph_set(&mut self, backend: &B, font_sizes: &[f32]) {
        const COMMON_GLYPH_SET: &str = " 0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
        for &font_size in font_sizes {
            self.warm_text(backend, COMMON_GLYPH_SET, font_size, 0.0);
        }
    }

    /// Warms shaping, layout, and glyph data for a known string.
    pub fn warm_text(&mut self, backend: &B, text: &str, font_size: f32, layout_width: f32) {
        let width = layout_width.ceil().max(font_size).max(1.0) as u32;
        let height = (font_size * 4.0).ceil().max(1.0) as u32;
        let request = TextDrawRequest {
            x: 0.0,
            y: 0.0,
            text: Arc::from(text),
            font_size,
            color: Rgba8::new(0, 0, 0, 255),
            bounds_width: layout_width,
            bounds_height: 0.0,
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: TextWritingMode::HorizontalTb,
            line_height: None,
            shadow: None,
            draw_glyphs: true,
            font_family: crate::font::FontFamily::SANS_SERIF,
            font_style: crate::font::FontStyle::Normal,
            font_weight: None,
            language: None,
            italic: false,
            clip_rect: [0.0, 0.0, width as f32, height as f32],
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
        };
        self.text_pipeline
            .as_mut()
            .expect("text warming requires the text pipeline")
            .prepare(backend, width, height, false, &[request], &[]);
    }

    /// Registers a backend-generic custom pipeline.
    pub fn register_custom_pipeline(&mut self, pipeline: impl CustomPipelineGeneric<B>) {
        self.custom_pipelines
            .push(CustomPipelineSlotGeneric::new(pipeline));
    }

    /// Uploads RGBA8 image data and returns the texture id used by draw commands.
    pub fn upload_image(&mut self, backend: &B, width: u32, height: u32, data: &[u8]) -> TextureId {
        self.image_pipeline
            .as_mut()
            .expect("image uploads require the image pipeline")
            .upload_image(backend, width, height, data)
    }

    /// Renders a draw list to a texture view, clearing it before drawing.
    pub fn render(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
    ) {
        self.render_with_source_texture(backend, view, None, width, height, is_srgb, draw_list);
    }

    /// Renders with an optional copyable source texture for backdrop effects.
    /// When a material command has no source texture, the renderer uses a
    /// retained offscreen target and composites its result to `view`.
    pub fn render_with_source_texture(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
    ) {
        self.render_with_source_texture_and_plan(
            backend,
            view,
            source_texture,
            width,
            height,
            is_srgb,
            draw_list,
            None,
            1.0,
            None,
        );
    }

    fn render_with_source_texture_and_plan(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        render_plan: Option<&RetainedRenderPlan>,
        device_scale: f32,
        opacity_target_key: Option<PersistentTargetKey>,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        self.compositor_stats = CompositorStats {
            promoted_surfaces: draw_list
                .commands()
                .iter()
                .filter(|command| matches!(command, DrawCommand::RetainedLayer { .. }))
                .count(),
            full_repaint: true,
            ..CompositorStats::default()
        };
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        let startup_timing = std::mem::replace(&mut self.first_render_timing_enabled, false);
        #[cfg(not(all(target_os = "windows", feature = "native", not(feature = "wgpu"))))]
        let startup_timing = false;
        self.frame_index = self.frame_index.wrapping_add(1);

        let retained_started = dx12_stage_start!(startup_timing);
        self.prepare_retained_layers(backend, draw_list, is_srgb);
        dx12_stage_finish!(retained_started, "retained layer preparation");

        let retained_plan =
            render_plan.is_some_and(|plan| render_plan_uses_material(plan, width, height));
        if source_texture.is_none() && (draw_list_uses_material(draw_list) || retained_plan) {
            let key = PersistentTargetKey::new(width, height, 1.0, 0, 0, 0, TargetValidity::Valid);
            let material_target_started = dx12_stage_start!(startup_timing);
            let result = self.material_target.ensure(backend, self.format, key);
            if matches!(
                result,
                TargetEnsureResult::Created | TargetEnsureResult::Recreated
            ) {
                let target_view = self
                    .material_target
                    .view()
                    .expect("ensured material target has a texture view");
                self.material_composite_bind_group = Some(
                    self.frame_composite_pipeline
                        .as_ref()
                        .expect("material rendering requires the frame composite pipeline")
                        .create_bind_group(backend, target_view),
                );
            }
            dx12_stage_finish!(material_target_started, "material target setup");
            if let (Some(target_view), Some(target_texture)) = (
                self.material_target.view().cloned(),
                self.material_target.texture().cloned(),
            ) {
                let (render_view, resolve_target) = if self.antialiasing.uses_multisampling() {
                    (
                        self.ensure_multisample_target(backend, width, height),
                        Some(target_view.clone()),
                    )
                } else {
                    (target_view.clone(), None)
                };
                let contents_started = dx12_stage_start!(startup_timing);
                if let Some(plan) = render_plan {
                    self.render_retained_plan_contents(
                        backend,
                        &render_view,
                        Some(&target_texture),
                        resolve_target.as_ref(),
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        plan,
                        device_scale,
                        opacity_target_key.unwrap_or(key),
                        startup_timing,
                        LoadOp::Clear([0.0; 4]),
                        None,
                    );
                } else {
                    self.render_contents(
                        backend,
                        &render_view,
                        Some(&target_texture),
                        resolve_target.as_ref(),
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        startup_timing,
                        LoadOp::Clear([0.0; 4]),
                        None,
                    );
                }
                dx12_stage_finish!(contents_started, "render contents total");
                self.material_target.mark_valid();
                let composition_started = dx12_stage_start!(startup_timing);
                self.composite_material_target(backend, view);
                dx12_stage_finish!(composition_started, "material target composition");
                let reclaim_started = dx12_stage_start!(startup_timing);
                self.reclaim_retained_layers();
                dx12_stage_finish!(reclaim_started, "retained layer reclamation");
                return;
            }
        }

        let (render_view, resolve_target) = if self.antialiasing.uses_multisampling() {
            (
                self.ensure_multisample_target(backend, width, height),
                Some(view),
            )
        } else {
            (view.clone(), None)
        };
        let contents_started = dx12_stage_start!(startup_timing);
        if let Some(plan) = render_plan {
            let opacity_target_key = opacity_target_key.unwrap_or_else(|| {
                PersistentTargetKey::new(
                    width,
                    height,
                    device_scale,
                    0,
                    0,
                    0,
                    TargetValidity::Valid,
                )
            });
            self.render_retained_plan_contents(
                backend,
                &render_view,
                source_texture,
                resolve_target,
                width,
                height,
                is_srgb,
                draw_list,
                plan,
                device_scale,
                opacity_target_key,
                startup_timing,
                LoadOp::Clear([0.0; 4]),
                None,
            );
        } else {
            self.render_contents(
                backend,
                &render_view,
                source_texture,
                resolve_target,
                width,
                height,
                is_srgb,
                draw_list,
                startup_timing,
                LoadOp::Clear([0.0; 4]),
                None,
            );
        }
        dx12_stage_finish!(contents_started, "render contents total");
        let reclaim_started = dx12_stage_start!(startup_timing);
        self.reclaim_retained_layers();
        dx12_stage_finish!(reclaim_started, "retained layer reclamation");
    }

    fn ensure_multisample_target(
        &mut self,
        backend: &B,
        width: u32,
        height: u32,
    ) -> B::TextureView {
        let sample_count = self.antialiasing.sample_count();
        let recreate = self.multisample_target.as_ref().is_none_or(|target| {
            target.width != width
                || target.height != height
                || target.sample_count != sample_count
                || target.format != self.format
        });
        if recreate {
            let texture = backend.create_texture(&crate::backend::TextureDescriptor {
                label: Some("cupid multisample color target".to_string()),
                size: (width, height, 1),
                mip_level_count: 1,
                sample_count,
                dimension: crate::backend::TextureDimension::D2,
                format: self.format,
                usage: vec![crate::backend::TextureUsage::RenderAttachment],
            });
            let view = backend.create_texture_view(&texture, "cupid multisample color target view");
            self.multisample_target = Some(MultisampleTargetGeneric {
                _texture: texture,
                view,
                width,
                height,
                sample_count,
                format: self.format,
                bytes: width as u64 * height as u64 * 4 * u64::from(sample_count),
            });
        }
        self.multisample_target
            .as_ref()
            .expect("multisample target initialized")
            .view
            .clone()
    }

    /// Renders an owned frame packet with its damage and scene metadata.
    pub fn render_packet(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        packet: &FramePacket,
        is_srgb: bool,
    ) {
        let frame = packet.frame();
        self.render_with_metadata(
            backend,
            view,
            None,
            frame.width,
            frame.height,
            is_srgb,
            &frame.draw_list,
            packet.metadata(),
            packet.scene(),
            packet.render_plan(),
        );
    }

    /// Renders a packet with an optional swap-chain texture for backdrop effects.
    pub fn render_packet_with_source_texture(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: &B::Texture,
        packet: &FramePacket,
        is_srgb: bool,
    ) {
        let frame = packet.frame();
        self.render_with_metadata(
            backend,
            view,
            Some(source_texture),
            frame.width,
            frame.height,
            is_srgb,
            &frame.draw_list,
            packet.metadata(),
            packet.scene(),
            packet.render_plan(),
        );
    }

    /// Renders a packet against a surface whose dimensions changed after the
    /// frame was recorded, promoting the packet to full-target damage.
    #[doc(hidden)]
    pub fn render_packet_at_size(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        packet: &FramePacket,
        width: u32,
        height: u32,
        is_srgb: bool,
    ) {
        let frame = packet.frame();
        let metadata = packet
            .metadata()
            .with_damage(crate::damage_region::DamageSet::full(width, height));
        self.render_with_metadata(
            backend,
            view,
            None,
            width,
            height,
            is_srgb,
            &frame.draw_list,
            &metadata,
            packet.scene(),
            packet.render_plan(),
        );
    }

    /// Renders a draw list with explicit frame metadata.
    pub fn render_frame_with_metadata(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        metadata: &FrameRenderMetadata,
    ) {
        self.render_with_metadata(
            backend, view, None, width, height, is_srgb, draw_list, metadata, None, None,
        );
    }

    /// Builds the material pipeline and the filter pipeline beside it.
    fn install_material_pipeline(&mut self, backend: &B) {
        let mut pipeline = MaterialPipeline::<B>::new(backend, self.format, self.antialiasing);
        pipeline.prewarm(backend, self.format);
        pipeline.begin_frame();
        self.custom_pipelines
            .insert(0, CustomPipelineSlotGeneric::new(pipeline));
        self.material_pipeline_initialized = true;
    }

    /// Builds the material pipelines on the second frame, if no draw has yet.
    ///
    /// They are built lazily because most frames draw no material, but the
    /// first backdrop blur then compiled the material shader mid-animation: 70 ms
    /// on a laptop whose shader cache was cold. Waiting for the second frame
    /// keeps the shader off the path to the first one.
    fn prewarm_material_pipeline(&mut self, backend: &B) {
        if self.material_pipeline_enabled && !self.material_pipeline_initialized {
            self.install_material_pipeline(backend);
        }
    }

    /// Whether the material pipelines exist, whether a material has drawn or not.
    #[cfg(test)]
    pub(crate) fn material_pipeline_is_warm(&self) -> bool {
        self.material_pipeline_initialized
    }

    fn render_with_metadata(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        metadata: &FrameRenderMetadata,
        scene: Option<&CompositorScene>,
        render_plan: Option<&RetainedRenderPlan>,
    ) {
        if self.frames_started == 1 {
            self.prewarm_material_pipeline(backend);
        }
        self.frames_started = self.frames_started.saturating_add(1);
        let scene_matches_target = scene.is_some_and(|scene| {
            scene.target_size() == (width, height)
                && scene.damage().target_size() == metadata.damage().target_size()
        });
        let scene = scene.filter(|_| scene_matches_target);
        let history_key = (
            metadata.surface_identity(),
            metadata.renderer_generation(),
            metadata.context_generation(),
            metadata.resource_generation(),
            width,
            height,
        );
        let opacity_target_key = PersistentTargetKey::new_with_resource(
            width,
            height,
            metadata.device_scale(),
            metadata.surface_identity(),
            metadata.renderer_generation(),
            metadata.context_generation(),
            metadata.resource_generation(),
            TargetValidity::Valid,
        );
        let (scene_diff, reused_nodes) =
            if let Some(scene) = scene.filter(|scene| scene.is_recorded()) {
                if self.scene_history_key != Some(history_key) {
                    self.scene_tree.clear();
                }
                let commit = self.scene_tree.commit(scene);
                let reused_nodes = commit.reused_nodes();
                (Some(commit.into_diff()), reused_nodes)
            } else {
                self.scene_tree.clear();
                self.scene_history_key = None;
                (None, 0)
            };

        let mut damage = metadata.damage().clone();
        let mut scene_changes = 0;
        if let Some(diff) = &scene_diff {
            scene_changes = diff.changes().len();
            if diff.damage().is_full() {
                damage.mark_full();
            } else {
                for region in diff.damage().regions() {
                    damage.add(*region);
                }
            }
        }
        let logical_nodes = scene.map_or(0, |scene| scene.nodes().len());
        let live_nodes = scene.map_or(0, |scene| {
            scene
                .nodes()
                .iter()
                .filter(|node| matches!(node.content(), crate::compositor::SceneContent::Live))
                .count()
        });
        let promoted_surfaces = scene.map_or_else(
            || {
                draw_list
                    .commands()
                    .iter()
                    .filter(|command| matches!(command, DrawCommand::RetainedLayer { .. }))
                    .count()
            },
            |scene| scene.surfaces().len(),
        );
        self.compositor_stats = CompositorStats {
            promoted_surfaces,
            damage_regions: damage.regions().len(),
            damaged_pixels: damage
                .regions()
                .iter()
                .map(|region| u64::from(region.width) * u64::from(region.height))
                .fold(0u64, u64::saturating_add),
            full_repaint: damage.is_full(),
            logical_nodes,
            live_nodes,
            reused_nodes,
            scene_changes,
            ..CompositorStats::default()
        };

        let retained_plan_has_custom_pipeline =
            render_plan.is_some_and(|plan| render_plan_uses_custom_pipeline(plan, width, height));
        let persistent_contract = source_texture.is_none()
            && !self.antialiasing.uses_multisampling()
            && !draw_list_uses_custom_pipeline(draw_list)
            && !retained_plan_has_custom_pipeline
            && self.frame_composite_pipeline.is_some()
            && metadata.damage().target_size() == (width, height)
            && metadata.device_scale().is_finite()
            && metadata.device_scale() > 0.0;
        if !persistent_contract || width == 0 || height == 0 {
            self.compositor_stats.full_repaint = true;
            self.render_with_source_texture_and_plan(
                backend,
                view,
                source_texture,
                width,
                height,
                is_srgb,
                draw_list,
                render_plan,
                metadata.device_scale(),
                Some(opacity_target_key),
            );
            return;
        }

        self.frame_index = self.frame_index.wrapping_add(1);
        self.prepare_retained_layers(backend, draw_list, is_srgb);
        let key = opacity_target_key;
        let result = self.scene_target.ensure(backend, self.format, key);
        if result == TargetEnsureResult::Unavailable {
            self.compositor_stats.full_repaint = true;
            self.render_with_source_texture_and_plan(
                backend,
                view,
                None,
                width,
                height,
                is_srgb,
                draw_list,
                render_plan,
                metadata.device_scale(),
                Some(key),
            );
            return;
        }
        if matches!(
            result,
            TargetEnsureResult::Created | TargetEnsureResult::Recreated
        ) {
            let target_view = self
                .scene_target
                .view()
                .expect("ensured scene target has a texture view");
            self.scene_composite_bind_group = Some(
                self.frame_composite_pipeline
                    .as_ref()
                    .expect("persistent scene rendering requires the composite pipeline")
                    .create_bind_group(backend, target_view),
            );
        }
        if matches!(
            result,
            TargetEnsureResult::Created
                | TargetEnsureResult::Recreated
                | TargetEnsureResult::ReusedInvalid
        ) {
            damage.mark_full();
            self.compositor_stats.full_repaint = true;
            self.compositor_stats.damage_regions = damage.regions().len();
            self.compositor_stats.damaged_pixels = u64::from(width) * u64::from(height);
        }

        if let (Some(target_view), Some(target_texture)) = (
            self.scene_target.view().cloned(),
            self.scene_target.texture().cloned(),
        ) {
            if damage.is_full() {
                // Resizing invalidates the whole target. Clear it on the GPU
                // instead of allocating and uploading a window-sized zero image.
                self.clear_render_target(backend, &target_view, None, [0.0; 4]);
            } else if !damage.is_empty() {
                let clear_texture = self.ensure_damage_clear_texture(backend, width, height);
                self.clear_damage_regions(
                    backend,
                    &clear_texture,
                    &target_texture,
                    damage.regions(),
                );
            }
            for region in damage.regions() {
                if region.width == 0 || region.height == 0 {
                    continue;
                }
                if let Some(plan) = render_plan {
                    self.render_retained_plan_contents(
                        backend,
                        &target_view,
                        None,
                        None,
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        plan,
                        metadata.device_scale(),
                        key,
                        false,
                        LoadOp::Load,
                        Some((region.x, region.y, region.width, region.height)),
                    );
                } else {
                    self.render_contents(
                        backend,
                        &target_view,
                        None,
                        None,
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        false,
                        LoadOp::Load,
                        Some((region.x, region.y, region.width, region.height)),
                    );
                }
            }
            self.scene_target.mark_valid();
            self.composite_scene_target(backend, view);
            self.compositor_stats.composition_passes = 1;
            if let Some(scene) = scene {
                self.scene_history_key = Some(history_key);
                debug_assert_eq!(scene.target_size(), (width, height));
            }
        }
        self.reclaim_retained_layers();
    }

    fn render_retained_plan_contents(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        resolve_target: Option<&B::TextureView>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        plan: &RetainedRenderPlan,
        device_scale: f32,
        opacity_target_key: PersistentTargetKey,
        startup_timing: bool,
        initial_load: LoadOp<[f64; 4]>,
        scissor: Option<(u32, u32, u32, u32)>,
    ) {
        let region = scissor.map_or_else(
            || crate::damage_region::DamageRect::new(0, 0, width, height),
            |(x, y, width, height)| crate::damage_region::DamageRect::new(x, y, width, height),
        );
        let operations = plan.operations_for_region(region).collect::<Vec<_>>();
        let has_opacity_groups = operations.iter().any(|operation| {
            matches!(
                operation.kind,
                RetainedRenderOperationKind::OpacityGroupBegin { .. }
                    | RetainedRenderOperationKind::OpacityGroupEnd { .. }
            )
        });
        if has_opacity_groups {
            self.render_retained_plan_with_opacity_groups(
                backend,
                view,
                source_texture,
                resolve_target,
                width,
                height,
                is_srgb,
                draw_list,
                &operations,
                device_scale,
                opacity_target_key,
                startup_timing,
                initial_load,
                region,
            );
        } else {
            self.render_retained_plan_chunk(
                backend,
                view,
                source_texture,
                resolve_target,
                width,
                height,
                (width, height),
                is_srgb,
                draw_list,
                &operations,
                device_scale,
                (0, 0),
                startup_timing,
                initial_load,
                scissor,
            );
        }
    }

    fn render_retained_plan_chunk(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        resolve_target: Option<&B::TextureView>,
        width: u32,
        height: u32,
        frame_size: (u32, u32),
        is_srgb: bool,
        draw_list: &DrawList,
        operations: &[&crate::frame::RetainedRenderOperation],
        device_scale: f32,
        target_offset: (u32, u32),
        startup_timing: bool,
        initial_load: LoadOp<[f64; 4]>,
        scissor: Option<(u32, u32, u32, u32)>,
    ) {
        let mut retained_lists = Vec::new();
        let mut retained_indices = HashMap::new();
        for operation in operations.iter().copied() {
            let RetainedRenderOperationKind::LocalV2(item) = &operation.kind else {
                continue;
            };
            let element = item.element.get();
            let cached = self.take_v2_command_list(item, device_scale);
            if cached.commands.is_empty() {
                self.v2_command_cache.insert(element, cached);
                continue;
            }
            let index = retained_lists.len();
            retained_lists.push((element, cached));
            retained_indices.insert(element, index);
        }

        let mut slices = Vec::with_capacity(operations.len() * 3);
        for operation in operations.iter().copied() {
            match &operation.kind {
                RetainedRenderOperationKind::LocalV2(item) => {
                    if let Some(index) = retained_indices.get(&item.element.get()) {
                        slices.push(retained_lists[*index].1.commands.as_slice());
                    }
                }
                RetainedRenderOperationKind::OpacityGroupBegin { .. }
                | RetainedRenderOperationKind::OpacityGroupEnd { .. } => {}
            }
        }
        self.render_contents_with_slices(
            backend,
            view,
            source_texture,
            resolve_target,
            width,
            height,
            frame_size,
            is_srgb,
            draw_list,
            Some(&slices),
            startup_timing,
            initial_load,
            scissor,
            target_offset,
        );
        drop(slices);
        for (element, cached) in retained_lists {
            self.v2_command_cache.insert(element, cached);
        }
    }

    fn render_retained_plan_with_opacity_groups(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        resolve_target: Option<&B::TextureView>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        operations: &[&crate::frame::RetainedRenderOperation],
        device_scale: f32,
        opacity_target_key: PersistentTargetKey,
        startup_timing: bool,
        initial_load: LoadOp<[f64; 4]>,
        region: DamageRect,
    ) {
        if let LoadOp::Clear(color) = initial_load {
            self.clear_render_target(backend, view, resolve_target, color);
        }

        let mut groups = Vec::<ActiveOpacityGroup<B>>::new();
        let mut morph_frames = Vec::<MorphFrame<B>>::new();
        let mut chunk = Vec::new();
        let seeded_groups = groups_sampling_backdrop(operations);
        for operation in operations.iter().copied() {
            match operation.kind {
                RetainedRenderOperationKind::OpacityGroupBegin {
                    element,
                    opacity,
                    masked,
                    fade,
                    morph,
                } => {
                    self.flush_opacity_group_chunk(
                        backend,
                        view,
                        source_texture,
                        resolve_target,
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        &mut chunk,
                        &groups,
                        device_scale,
                        startup_timing,
                        region,
                    );

                    if let Some(crate::draw_cmd_v2::MorphGroup::Host(params)) = morph {
                        // The host owns no pixels: it only waits for its sources.
                        morph_frames.push(MorphFrame {
                            element,
                            params,
                            opacity,
                            bounds: operation.bounds,
                            groups_open: groups.len(),
                            from: None,
                            to: None,
                        });
                        continue;
                    }
                    let mut group = self.ensure_opacity_group_target(
                        backend,
                        next_group_depth(groups.len(), &morph_frames),
                        element,
                        opacity,
                        operation.bounds,
                        opacity_target_key,
                    );
                    group.morph_source = MorphSource::of(morph);
                    if masked {
                        group.mask = MaskRole::AwaitingContent;
                    } else if fade {
                        group.mask = MaskRole::AwaitingAlpha;
                    }
                    self.clear_render_target(
                        backend,
                        &group.render_view,
                        group.resolve_target.as_ref(),
                        [0.0; 4],
                    );
                    // An ordinary group that holds a surface sampling the backdrop
                    // starts from what is behind it: the frame, or, inside another
                    // group, that group as it is so far, which a group of its own
                    // that is seeded holds the frame in. Every other group starts
                    // from nothing.
                    if !masked
                        && !fade
                        && seeded_groups.contains(&element)
                        && groups.last().is_none_or(|parent| parent.seeded)
                    {
                        group.seeded = self.seed_group_from_behind(
                            backend,
                            source_texture,
                            groups.last(),
                            &group,
                            width,
                            height,
                            is_srgb,
                        );
                    }
                    groups.push(group);
                }
                RetainedRenderOperationKind::OpacityGroupEnd { element } => {
                    self.flush_opacity_group_chunk(
                        backend,
                        view,
                        source_texture,
                        resolve_target,
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        &mut chunk,
                        &groups,
                        device_scale,
                        startup_timing,
                        region,
                    );
                    if ends_host(groups.len(), &morph_frames) {
                        let frame = morph_frames
                            .pop()
                            .expect("a morph host is open when it ends");
                        assert_eq!(
                            frame.element, element,
                            "morph group end matches its begin"
                        );
                        if let Some(composite_region) =
                            opacity_group_region(region, &groups, frame.bounds)
                        {
                            let (parent_view, parent_resolve, parent_origin, parent_size) =
                                groups.last().map_or_else(
                                    || (view.clone(), resolve_target.cloned(), (0, 0), (width, height)),
                                    |parent| {
                                        (
                                            parent.render_view.clone(),
                                            parent.resolve_target.clone(),
                                            (parent.bounds.x, parent.bounds.y),
                                            (parent.bounds.width, parent.bounds.height),
                                        )
                                    },
                                );
                            let local_region = DamageRect::new(
                                composite_region.x - parent_origin.0,
                                composite_region.y - parent_origin.1,
                                composite_region.width,
                                composite_region.height,
                            );
                            self.composite_morph_group(
                                backend,
                                &frame,
                                &parent_view,
                                parent_resolve.as_ref(),
                                parent_origin,
                                parent_size,
                                is_srgb,
                                device_scale,
                                local_region,
                            );
                        }
                        self.release_morph_sources(&frame);
                        continue;
                    }
                    if groups
                        .last()
                        .is_some_and(|top| top.mask == MaskRole::Coverage)
                    {
                        // The coverage is complete: cut the content out with it.
                        let coverage = groups.pop().expect("a coverage group was just seen");
                        if let Some(coverage_region) =
                            opacity_group_region(region, &groups, coverage.bounds)
                        {
                            let content = groups
                                .last()
                                .expect("a coverage group sits above its content");
                            let origin = (content.bounds.x, content.bounds.y);
                            let local_region = DamageRect::new(
                                coverage_region.x - origin.0,
                                coverage_region.y - origin.1,
                                coverage_region.width,
                                coverage_region.height,
                            );
                            let (content_view, content_resolve) =
                                (content.render_view.clone(), content.resolve_target.clone());
                            let (content_width, content_height) =
                                (content.bounds.width, content.bounds.height);
                            self.composite_opacity_group(
                                backend,
                                &coverage,
                                &content_view,
                                content_resolve.as_ref(),
                                origin,
                                content_width,
                                content_height,
                                is_srgb,
                                local_region,
                                true,
                            );
                        }
                        self.opacity_group_targets[coverage.depth].target.mark_valid();
                    }
                    if groups.last().is_some_and(|top| top.mask == MaskRole::Content) {
                        // The content is complete: scale it by the alpha below.
                        let mut content = groups.pop().expect("a content group was just seen");
                        if let Some(content_region) =
                            opacity_group_region(region, &groups, content.bounds)
                        {
                            let alpha = groups
                                .last()
                                .expect("a content group sits above its alpha");
                            let origin = (content.bounds.x, content.bounds.y);
                            let local_region = DamageRect::new(
                                content_region.x - origin.0,
                                content_region.y - origin.1,
                                content_region.width,
                                content_region.height,
                            );
                            self.composite_opacity_group(
                                backend,
                                alpha,
                                &content.render_view,
                                content.resolve_target.as_ref(),
                                origin,
                                content.bounds.width,
                                content.bounds.height,
                                is_srgb,
                                local_region,
                                true,
                            );
                        }
                        let alpha = groups.pop().expect("a content group sits above its alpha");
                        self.opacity_group_targets[alpha.depth].target.mark_valid();
                        // Scaled, it is an ordinary group from here on.
                        content.mask = MaskRole::None;
                        groups.push(content);
                    }
                    let group = groups
                        .pop()
                        .expect("validated opacity groups have matching begin operations");
                    assert_eq!(
                        group.element, element,
                        "opacity group end matches its begin"
                    );
                    if matches!(
                        group.mask,
                        MaskRole::AwaitingContent | MaskRole::AwaitingAlpha
                    ) {
                        // Nothing was drawn inside it, so there is no coverage to
                        // show the content through.
                        continue;
                    }
                    if let Some(role) = group.morph_source {
                        // Its host blends it with the other source once both
                        // are drawn.
                        Self::keep_morph_source(&mut morph_frames, role, &group);
                        continue;
                    }
                    let Some(composite_region) =
                        opacity_group_region(region, &groups, group.bounds)
                    else {
                        continue;
                    };
                    let (parent_view, parent_resolve, parent_origin, parent_width, parent_height) =
                        groups.last().map_or_else(
                            || (view.clone(), resolve_target.cloned(), (0, 0), width, height),
                            |parent| {
                                (
                                    parent.render_view.clone(),
                                    parent.resolve_target.clone(),
                                    (parent.bounds.x, parent.bounds.y),
                                    parent.bounds.width,
                                    parent.bounds.height,
                                )
                            },
                        );
                    let local_composite_region = DamageRect::new(
                        composite_region.x - parent_origin.0,
                        composite_region.y - parent_origin.1,
                        composite_region.width,
                        composite_region.height,
                    );
                    self.composite_opacity_group(
                        backend,
                        &group,
                        &parent_view,
                        parent_resolve.as_ref(),
                        parent_origin,
                        parent_width,
                        parent_height,
                        is_srgb,
                        local_composite_region,
                        false,
                    );
                    self.opacity_group_targets[group.depth].target.mark_valid();
                }
                RetainedRenderOperationKind::LocalV2(ref item) => {
                    if groups
                        .last()
                        .is_some_and(|top| top.mask == MaskRole::AwaitingAlpha)
                    {
                        // The group's first item is the node's own: its alpha.
                        // Draw it into the alpha target, then everything after
                        // it is content and goes into a target of its own.
                        let (group_element, group_bounds, group_opacity) = {
                            let group = groups.last_mut().expect("a group awaits its alpha");
                            // The content carries the group's opacity once.
                            (group.element, group.bounds, std::mem::replace(&mut group.opacity, 1.0))
                        };
                        let is_alpha = item.element.get() == group_element;
                        if is_alpha {
                            chunk.push(operation);
                        }
                        self.flush_opacity_group_chunk(
                            backend,
                            view,
                            source_texture,
                            resolve_target,
                            width,
                            height,
                            is_srgb,
                            draw_list,
                            &mut chunk,
                            &groups,
                            device_scale,
                            startup_timing,
                            region,
                        );
                        groups
                            .last_mut()
                            .expect("a group awaited its alpha")
                            .mask = MaskRole::HasContent;
                        let mut content = self.ensure_opacity_group_target(
                            backend,
                            next_group_depth(groups.len(), &morph_frames),
                            group_element,
                            group_opacity,
                            group_bounds,
                            opacity_target_key,
                        );
                        content.mask = MaskRole::Content;
                        self.clear_render_target(
                            backend,
                            &content.render_view,
                            content.resolve_target.as_ref(),
                            [0.0; 4],
                        );
                        groups.push(content);
                        if !is_alpha {
                            chunk.push(operation);
                        }
                        continue;
                    }
                    let awaiting = groups
                        .last()
                        .filter(|top| top.mask == MaskRole::AwaitingContent)
                        .map(|top| (top.element, top.bounds));
                    let Some((group_element, group_bounds)) = awaiting else {
                        chunk.push(operation);
                        continue;
                    };
                    // The group's first item is the node's own: its content. Draw
                    // it into the content target, then everything after it is
                    // coverage and goes into a target of its own.
                    let is_content = item.element.get() == group_element;
                    if is_content {
                        chunk.push(operation);
                    }
                    self.flush_opacity_group_chunk(
                        backend,
                        view,
                        source_texture,
                        resolve_target,
                        width,
                        height,
                        is_srgb,
                        draw_list,
                        &mut chunk,
                        &groups,
                        device_scale,
                        startup_timing,
                        region,
                    );
                    groups
                        .last_mut()
                        .expect("a group was awaiting its content")
                        .mask = MaskRole::HasCoverage;
                    let mut coverage = self.ensure_opacity_group_target(
                        backend,
                        next_group_depth(groups.len(), &morph_frames),
                        group_element,
                        1.0,
                        group_bounds,
                        opacity_target_key,
                    );
                    coverage.mask = MaskRole::Coverage;
                    self.clear_render_target(
                        backend,
                        &coverage.render_view,
                        coverage.resolve_target.as_ref(),
                        [0.0; 4],
                    );
                    groups.push(coverage);
                    if !is_content {
                        chunk.push(operation);
                    }
                }
            }
        }
        self.flush_opacity_group_chunk(
            backend,
            view,
            source_texture,
            resolve_target,
            width,
            height,
            is_srgb,
            draw_list,
            &mut chunk,
            &groups,
            device_scale,
            startup_timing,
            region,
        );
        debug_assert!(groups.is_empty());
        debug_assert!(morph_frames.is_empty());
    }

    fn flush_opacity_group_chunk(
        &mut self,
        backend: &B,
        root_view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        root_resolve_target: Option<&B::TextureView>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        chunk: &mut Vec<&crate::frame::RetainedRenderOperation>,
        groups: &[ActiveOpacityGroup<B>],
        device_scale: f32,
        startup_timing: bool,
        region: DamageRect,
    ) {
        if chunk.is_empty() {
            return;
        }
        let Some(scissor) = opacity_group_scissor(region, groups) else {
            chunk.clear();
            return;
        };
        let (view, resolve_target) = groups.last().map_or_else(
            || (root_view.clone(), root_resolve_target.cloned()),
            |group| (group.render_view.clone(), group.resolve_target.clone()),
        );
        let (target_width, target_height, target_offset) =
            groups.last().map_or((width, height, (0, 0)), |group| {
                (
                    group.bounds.width,
                    group.bounds.height,
                    (group.bounds.x, group.bounds.y),
                )
            });
        let local_scissor = DamageRect::new(
            scissor.x - target_offset.0,
            scissor.y - target_offset.1,
            scissor.width,
            scissor.height,
        );
        let operations = std::mem::take(chunk);
        // A seeded group is its own backdrop: what it drew so far is on top of the
        // frame it started from.
        self.group_backdrop = groups
            .last()
            .filter(|group| group.seeded)
            .and_then(|group| self.opacity_group_targets[group.depth].target.texture())
            .cloned()
            .map(|texture| GroupBackdrop { texture });
        self.render_retained_plan_chunk(
            backend,
            &view,
            source_texture,
            resolve_target.as_ref(),
            target_width,
            target_height,
            (width, height),
            is_srgb,
            draw_list,
            &operations,
            device_scale,
            target_offset,
            startup_timing,
            LoadOp::Load,
            Some((
                local_scissor.x,
                local_scissor.y,
                local_scissor.width,
                local_scissor.height,
            )),
        );
        self.group_backdrop = None;
    }

    fn ensure_opacity_group_target(
        &mut self,
        backend: &B,
        depth: usize,
        element: u64,
        opacity: f32,
        bounds: DamageRect,
        key: PersistentTargetKey,
    ) -> ActiveOpacityGroup<B> {
        let width = bounds.width;
        let height = bounds.height;
        assert!(
            width != 0 && height != 0,
            "opacity groups have nonempty bounds"
        );
        while self.opacity_group_targets.len() <= depth {
            self.opacity_group_targets
                .push(OpacityGroupTargetGeneric::default());
        }
        let group_target = &mut self.opacity_group_targets[depth];
        let result = group_target
            .target
            .ensure(backend, self.format, key.with_size(width, height));
        if matches!(
            result,
            TargetEnsureResult::Created | TargetEnsureResult::Recreated
        ) {
            group_target.multisample_target = None;
            let target_view = group_target
                .target
                .view()
                .expect("ensured opacity group target has a view");
            group_target.bind_group = Some(
                self.image_pipeline
                    .as_ref()
                    .expect("opacity groups require the image pipeline")
                    .create_external_bind_group(backend, target_view),
            );
        }
        let target_view = group_target
            .target
            .view()
            .cloned()
            .expect("ensured opacity group target has a view");

        let (render_view, resolve_target) = if self.antialiasing.uses_multisampling() {
            let sample_count = self.antialiasing.sample_count();
            let recreate = group_target
                .multisample_target
                .as_ref()
                .is_none_or(|target| {
                    target.width != width
                        || target.height != height
                        || target.sample_count != sample_count
                        || target.format != self.format
                });
            if recreate {
                let texture = backend.create_texture(&crate::backend::TextureDescriptor {
                    label: Some("cupid opacity group multisample target".to_string()),
                    size: (width, height, 1),
                    mip_level_count: 1,
                    sample_count,
                    dimension: crate::backend::TextureDimension::D2,
                    format: self.format,
                    usage: vec![crate::backend::TextureUsage::RenderAttachment],
                });
                let view = backend
                    .create_texture_view(&texture, "cupid opacity group multisample target view");
                group_target.multisample_target = Some(MultisampleTargetGeneric {
                    _texture: texture,
                    view,
                    width,
                    height,
                    sample_count,
                    format: self.format,
                    bytes: width as u64 * height as u64 * 4 * u64::from(sample_count),
                });
            }
            let multisample_view = group_target
                .multisample_target
                .as_ref()
                .expect("opacity group multisample target initialized")
                .view
                .clone();
            (multisample_view, Some(target_view.clone()))
        } else {
            (target_view.clone(), None)
        };

        let bind_group = group_target
            .bind_group
            .clone()
            .expect("opacity group target bind group initialized");
        ActiveOpacityGroup {
            element,
            mask: MaskRole::None,
            morph_source: None,
            sample_view: target_view,
            opacity,
            bounds,
            depth,
            bind_group,
            render_view,
            resolve_target,
            seeded: false,
        }
    }

    /// Starts `group`'s target from what is behind it, so that what it draws is
    /// drawn over that; see [`ActiveOpacityGroup::seeded`].
    ///
    /// What is behind it is the frame, or, for a group inside `parent`, `parent`'s
    /// target as it is so far. Returns whether it did, which it does not without
    /// something to copy from, or for a group that starts before its parent does.
    /// A group that reaches past what it is copied from is started from the part
    /// of it that is inside.
    ///
    /// A single-sample target is filled by a copy. A multisampled one cannot be,
    /// so the copy is made to a single-sample texture of its own, which is drawn
    /// over the target as an image.
    fn seed_group_from_behind(
        &mut self,
        backend: &B,
        frame_texture: Option<&B::Texture>,
        parent: Option<&ActiveOpacityGroup<B>>,
        group: &ActiveOpacityGroup<B>,
        frame_width: u32,
        frame_height: u32,
        is_srgb: bool,
    ) -> bool {
        use crate::backend::{Extent3d, Origin3d, TexelCopyTextureInfo, TextureAspect};

        // What is copied from, where the group is in it, and how much of it there is.
        let (source, x, y, available_width, available_height) = match parent {
            None => {
                let Some(frame) = frame_texture else {
                    return false;
                };
                (
                    frame.clone(),
                    group.bounds.x,
                    group.bounds.y,
                    frame_width,
                    frame_height,
                )
            }
            Some(parent) => {
                let Some(target) = self.opacity_group_targets[parent.depth].target.texture() else {
                    return false;
                };
                if group.bounds.x < parent.bounds.x || group.bounds.y < parent.bounds.y {
                    return false;
                }
                (
                    target.clone(),
                    group.bounds.x - parent.bounds.x,
                    group.bounds.y - parent.bounds.y,
                    parent.bounds.width,
                    parent.bounds.height,
                )
            }
        };
        if x >= available_width || y >= available_height {
            return false;
        }
        let extent = Extent3d {
            width: group.bounds.width.min(available_width - x),
            height: group.bounds.height.min(available_height - y),
            depth_or_array_layers: 1,
        };
        let copy = |backend: &B, destination: &B::Texture| {
            let mut encoder = backend.create_command_encoder("cupid seed opacity group");
            backend.copy_texture_to_texture(
                &mut encoder,
                &TexelCopyTextureInfo {
                    texture: &source,
                    mip_level: 0,
                    origin: Origin3d { x, y, z: 0 },
                    aspect: TextureAspect::All,
                },
                &TexelCopyTextureInfo {
                    texture: destination,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                extent,
            );
            backend.submit(encoder);
        };

        if !self.antialiasing.uses_multisampling() {
            let Some(target) = self.opacity_group_targets[group.depth].target.texture() else {
                return false;
            };
            copy(backend, target);
            return true;
        }

        let key = PersistentTargetKey::new(
            group.bounds.width,
            group.bounds.height,
            1.0,
            0,
            0,
            0,
            TargetValidity::Valid,
        );
        let result = self.group_seed_target.ensure(backend, self.format, key);
        if matches!(
            result,
            TargetEnsureResult::Created | TargetEnsureResult::Recreated
        ) {
            let view = self
                .group_seed_target
                .view()
                .expect("ensured seed target has a view");
            self.group_seed_bind_group = Some(
                self.image_pipeline
                    .as_ref()
                    .expect("opacity groups require the image pipeline")
                    .create_external_bind_group(backend, view),
            );
        }
        let (Some(seed_view), Some(seed_texture), Some(seed_bind_group)) = (
            self.group_seed_target.view().cloned(),
            self.group_seed_target.texture().cloned(),
            self.group_seed_bind_group.clone(),
        ) else {
            return false;
        };
        // What the copy does not reach stays clear, not what an earlier group left.
        self.clear_render_target(backend, &seed_view, None, [0.0; 4]);
        copy(backend, &seed_texture);
        let seed = ActiveOpacityGroup {
            element: group.element,
            mask: MaskRole::None,
            morph_source: None,
            sample_view: seed_view,
            opacity: 1.0,
            bounds: group.bounds,
            depth: group.depth,
            bind_group: seed_bind_group,
            render_view: group.render_view.clone(),
            resolve_target: group.resolve_target.clone(),
            seeded: false,
        };
        self.composite_opacity_group(
            backend,
            &seed,
            &group.render_view,
            group.resolve_target.as_ref(),
            (group.bounds.x, group.bounds.y),
            group.bounds.width,
            group.bounds.height,
            is_srgb,
            DamageRect::new(0, 0, group.bounds.width, group.bounds.height),
            false,
        );
        true
    }

    fn composite_opacity_group(
        &mut self,
        backend: &B,
        group: &ActiveOpacityGroup<B>,
        view: &B::TextureView,
        resolve_target: Option<&B::TextureView>,
        parent_origin: (u32, u32),
        width: u32,
        height: u32,
        is_srgb: bool,
        scissor: DamageRect,
        mask: bool,
    ) {
        let image = ImageInstance {
            position: [
                (group.bounds.x - parent_origin.0) as f32,
                (group.bounds.y - parent_origin.1) as f32,
            ],
            size: [group.bounds.width as f32, group.bounds.height as f32],
            uv_offset: [0.0, 0.0],
            uv_scale: [1.0, 1.0],
            // Descendant clips were applied while building the isolated
            // surface. The group bounds only limit its damage scissor; treating
            // that union as another clip would add an unwanted antialiased edge.
            clip_rect: [-1.0; 4],
            clip_border_radius: [0.0; 4],
            alpha: group.opacity,
            source_premultiplied: 1.0,
        };
        let image_pipeline = self
            .image_pipeline
            .as_mut()
            .expect("opacity groups require the image pipeline");
        image_pipeline.begin_frame(backend, 1, width, height, is_srgb);
        let mut encoder = backend.create_command_encoder("cupid composite opacity group");
        let attachments = [RenderPassColorAttachment {
            view,
            resolve_target,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        }];
        {
            use crate::backend::GpuRenderPass;
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("cupid opacity group composite pass".to_string()),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                },
            );
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
            if mask {
                image_pipeline.draw_external_mask_batch(
                    backend,
                    &mut pass,
                    &group.bind_group,
                    std::slice::from_ref(&image),
                );
            } else {
                image_pipeline.draw_external_batch(
                    backend,
                    &mut pass,
                    &group.bind_group,
                    std::slice::from_ref(&image),
                );
            }
        }
        image_pipeline.end_frame(backend);
        backend.submit(encoder);
    }

    fn take_v2_command_list(&mut self, item: &RetainedV2Item, scale: f32) -> CachedV2CommandList {
        let element = item.element.get();
        let scale_bits = scale.to_bits();
        let origin_bits = (item.origin.0.to_bits(), item.origin.1.to_bits());
        let transform_bits = [
            item.transform.cols[0][0].to_bits(),
            item.transform.cols[0][1].to_bits(),
            item.transform.cols[0][2].to_bits(),
            item.transform.cols[1][0].to_bits(),
            item.transform.cols[1][1].to_bits(),
            item.transform.cols[1][2].to_bits(),
            item.transform.cols[2][0].to_bits(),
            item.transform.cols[2][1].to_bits(),
            item.transform.cols[2][2].to_bits(),
        ];
        let clip_bits = item.clip.map(|clip| {
            (
                [
                    clip.x.to_bits(),
                    clip.y.to_bits(),
                    clip.width.to_bits(),
                    clip.height.to_bits(),
                ],
                item.clip_radius.map(f32::to_bits),
            )
        });
        if let Some(mut cached) = self.v2_command_cache.remove(&element)
            && cached.revision == item.revision
            && cached.scale_bits == scale_bits
            && cached.origin_bits == origin_bits
            && cached.transform_bits == transform_bits
            && cached.clip_bits == clip_bits
        {
            cached.last_used_frame = self.frame_index;
            return cached;
        }

        let commands = crate::v2_frame_adapter::lower_retained_v2_commands(item, scale)
            .expect("validated v2 items must lower during frame replay");
        CachedV2CommandList {
            revision: item.revision,
            scale_bits,
            origin_bits,
            transform_bits,
            clip_bits,
            commands,
            last_used_frame: self.frame_index,
        }
    }

    fn composite_scene_target(&self, backend: &B, view: &B::TextureView) {
        let (Some(bind_group), Some(pipeline)) = (
            self.scene_composite_bind_group.as_ref(),
            self.frame_composite_pipeline.as_ref(),
        ) else {
            return;
        };
        let mut encoder = backend.create_command_encoder("cupid retained scene composition");
        {
            let attachments = [RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear([0.0; 4]),
                    store: StoreOp::Store,
                },
            }];
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("cupid retained scene composite pass".to_string()),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                },
            );
            pipeline.render(&mut pass, bind_group);
        }
        backend.submit(encoder);
    }

    fn composite_material_target(&self, backend: &B, view: &B::TextureView) {
        let Some(bind_group) = self.material_composite_bind_group.as_ref() else {
            return;
        };
        let mut encoder = backend.create_command_encoder("cupid generic frame composite");
        {
            let attachments = [RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear([0.0; 4]),
                    store: StoreOp::Store,
                },
            }];
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("cupid generic frame composite pass".to_string()),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                },
            );
            self.frame_composite_pipeline
                .as_ref()
                .expect("material rendering requires the frame composite pipeline")
                .render(&mut pass, bind_group);
        }
        backend.submit(encoder);
    }

    fn render_contents(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        resolve_target: Option<&B::TextureView>,
        width: u32,
        height: u32,
        is_srgb: bool,
        draw_list: &DrawList,
        startup_timing: bool,
        initial_load: LoadOp<[f64; 4]>,
        scissor: Option<(u32, u32, u32, u32)>,
    ) {
        self.render_contents_with_slices(
            backend,
            view,
            source_texture,
            resolve_target,
            width,
            height,
            (width, height),
            is_srgb,
            draw_list,
            None,
            startup_timing,
            initial_load,
            scissor,
            (0, 0),
        );
    }

    fn render_contents_with_slices(
        &mut self,
        backend: &B,
        view: &B::TextureView,
        source_texture: Option<&B::Texture>,
        resolve_target: Option<&B::TextureView>,
        width: u32,
        height: u32,
        frame_size: (u32, u32),
        is_srgb: bool,
        draw_list: &DrawList,
        command_slices: Option<&[&[DrawCommand]]>,
        startup_timing: bool,
        initial_load: LoadOp<[f64; 4]>,
        scissor: Option<(u32, u32, u32, u32)>,
        target_offset: (u32, u32),
    ) {
        let collect_started = dx12_stage_start!(startup_timing);
        self.collect_commands(
            backend,
            draw_list,
            command_slices,
            frame_size,
            startup_timing,
            target_offset,
        );
        dx12_stage_finish!(collect_started, "draw-list command collection");

        let custom_started = dx12_stage_start!(startup_timing);
        for slot in &mut self.custom_pipelines {
            if slot.pipeline.has_work() {
                slot.pipeline.prepare(&RenderContextGeneric {
                    backend,
                    width,
                    height,
                    is_srgb,
                    format: self.format,
                    sample_count: 1,
                    source_texture,
                });
            }
        }
        dx12_stage_finish!(custom_started, "custom pipeline preparation");

        let text_started = dx12_stage_start!(startup_timing);
        if !self.text_requests.is_empty() || !self.decoration_requests.is_empty() {
            self.text_pipeline
                .as_mut()
                .expect("text commands require the text pipeline")
                .prepare(
                    backend,
                    width,
                    height,
                    is_srgb,
                    &self.text_requests,
                    &self.decoration_requests,
                );
        }
        dx12_stage_finish!(
            text_started,
            "text shaping, glyph rasterization, and upload"
        );

        let svg_started = dx12_stage_start!(startup_timing);
        if let Some(svg_pipeline) = self.svg_pipeline.as_mut() {
            svg_pipeline.prepare(backend, &self.svg_items, width, height, is_srgb);
        } else if !self.svg_items.is_empty() {
            panic!("SVG commands require the SVG pipeline");
        }
        dx12_stage_finish!(svg_started, "SVG preparation");

        let batch_started = dx12_stage_start!(startup_timing);
        let mut total_rects = 0;
        let mut total_images = 0;
        for command in &self.resolved {
            match command {
                ResolvedCommand::Rect { .. } => total_rects += 1,
                ResolvedCommand::Image { .. } | ResolvedCommand::RetainedLayer { .. } => {
                    total_images += 1
                }
                _ => {}
            }
        }
        self.rect_pipeline
            .begin_frame(backend, total_rects, width, height, is_srgb);
        if let Some(image_pipeline) = self.image_pipeline.as_mut() {
            image_pipeline.begin_frame(backend, total_images, width, height, is_srgb);
        } else if total_images != 0 {
            panic!("image commands require the image pipeline");
        }
        dx12_stage_finish!(batch_started, "rect and image batch setup");

        let encode_started = dx12_stage_start!(startup_timing);
        let mut encoder = backend.create_command_encoder("cupid generic render encoder");
        let mut start = 0;
        let mut first_pass = true;
        for index in 0..self.resolved.len() {
            let ResolvedCommand::Custom {
                pipeline_index,
                command_index,
            } = &self.resolved[index]
            else {
                continue;
            };
            let pipeline_index = *pipeline_index;
            let command_index = *command_index;
            let needs_backdrop = self
                .custom_pipelines
                .get(pipeline_index)
                .is_some_and(|slot| slot.pipeline.needs_backdrop(command_index));
            if !needs_backdrop {
                continue;
            }
            self.render_resolved_range(
                backend,
                &mut encoder,
                view,
                resolve_target,
                start..index,
                if first_pass {
                    initial_load
                } else {
                    LoadOp::Load
                },
                scissor,
            );
            first_pass = false;
            // Inside a seeded group the backdrop is the group's own target, read
            // from its own corner; everywhere else it is the frame.
            let backdrop = match &self.group_backdrop {
                Some(group) => Some((&group.texture, (0, 0))),
                None => source_texture.map(|texture| (texture, target_offset)),
            };
            if let (Some((backdrop_texture, backdrop_offset)), Some(slot)) =
                (backdrop, self.custom_pipelines.get(pipeline_index))
            {
                slot.pipeline.capture_backdrop_command(
                    command_index,
                    backend,
                    &mut encoder,
                    backdrop_texture,
                    backdrop_offset,
                    width,
                    height,
                );
            }
            self.render_resolved_range(
                backend,
                &mut encoder,
                view,
                resolve_target,
                index..index + 1,
                LoadOp::Load,
                scissor,
            );
            start = index + 1;
        }
        if first_pass || start < self.resolved.len() {
            self.render_resolved_range(
                backend,
                &mut encoder,
                view,
                resolve_target,
                start..self.resolved.len(),
                if first_pass {
                    initial_load
                } else {
                    LoadOp::Load
                },
                scissor,
            );
        }

        self.rect_pipeline.end_frame(backend);
        if let Some(image_pipeline) = self.image_pipeline.as_mut() {
            image_pipeline.end_frame(backend);
        }
        backend.submit(encoder);
        dx12_stage_finish!(encode_started, "render command encoding and submission");

        let cleanup_started = dx12_stage_start!(startup_timing);
        for texture_id in self.textures_to_remove.drain(..) {
            if let Some(image_pipeline) = self.image_pipeline.as_mut() {
                image_pipeline.remove_texture(texture_id);
            }
        }
        if let Some(image_pipeline) = self.image_pipeline.as_mut() {
            for texture_id in image_pipeline.eviction_candidates() {
                if !draw_list.has_live_texture_reference(texture_id) {
                    let serial = draw_list.texture_registry_serial(texture_id);
                    image_pipeline.remove_texture(texture_id);
                    if let Some(serial) = serial {
                        draw_list.mark_texture_evicted(texture_id, serial);
                    }
                }
            }
        }
        dx12_stage_finish!(cleanup_started, "post-submit texture cleanup");
    }

    fn render_resolved_range(
        &mut self,
        backend: &B,
        encoder: &mut B::CommandEncoder,
        view: &B::TextureView,
        resolve_target: Option<&B::TextureView>,
        range: std::ops::Range<usize>,
        load: LoadOp<[f64; 4]>,
        scissor: Option<(u32, u32, u32, u32)>,
    ) {
        let attachments = [RenderPassColorAttachment {
            view,
            resolve_target,
            ops: Operations {
                load,
                store: StoreOp::Store,
            },
        }];
        let mut pass = backend.begin_render_pass(
            encoder,
            &RenderPassDescriptor {
                label: Some("cupid generic render pass".to_string()),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
            },
        );
        if let Some((x, y, width, height)) = scissor {
            use crate::backend::GpuRenderPass;
            pass.set_scissor_rect(x, y, width, height);
        }
        self.draw_resolved_range(backend, &mut pass, range);
    }

    fn draw_resolved_range<'pass>(
        &'pass mut self,
        backend: &B,
        pass: &mut B::RenderPass<'pass>,
        range: std::ops::Range<usize>,
    ) {
        let Self {
            rect_pipeline,
            image_pipeline,
            text_pipeline,
            svg_pipeline,
            custom_pipelines,
            resolved,
            image_batch,
            ..
        } = self;
        let mut current_texture_id = None;
        image_batch.clear();
        for index in range {
            match &resolved[index] {
                ResolvedCommand::Rect { instance, clear } => {
                    flush_image_batch(
                        backend,
                        pass,
                        image_pipeline.as_mut(),
                        image_batch,
                        &mut current_texture_id,
                    );
                    if *clear {
                        rect_pipeline.flush(pass);
                        rect_pipeline.push(*instance);
                        rect_pipeline.flush_clear(pass);
                    } else {
                        rect_pipeline.push(*instance);
                    }
                }
                ResolvedCommand::Image {
                    texture_id,
                    instance,
                } => {
                    rect_pipeline.flush(pass);
                    if current_texture_id.is_some_and(|current| current != *texture_id) {
                        flush_image_batch(
                            backend,
                            pass,
                            image_pipeline.as_mut(),
                            image_batch,
                            &mut current_texture_id,
                        );
                    }
                    current_texture_id = Some(*texture_id);
                    image_batch.push(*instance);
                }
                ResolvedCommand::RetainedLayer {
                    bind_group,
                    instance,
                } => {
                    rect_pipeline.flush(pass);
                    flush_image_batch(
                        backend,
                        pass,
                        image_pipeline.as_mut(),
                        image_batch,
                        &mut current_texture_id,
                    );
                    image_pipeline
                        .as_mut()
                        .expect("retained layers require the image pipeline")
                        .draw_external_batch(
                            backend,
                            pass,
                            bind_group,
                            std::slice::from_ref(instance),
                        );
                }
                ResolvedCommand::Text(text_index) => {
                    rect_pipeline.flush(pass);
                    flush_image_batch(
                        backend,
                        pass,
                        image_pipeline.as_mut(),
                        image_batch,
                        &mut current_texture_id,
                    );
                    text_pipeline
                        .as_ref()
                        .expect("text commands require the text pipeline")
                        .render_request(pass, *text_index);
                }
                ResolvedCommand::TextDecoration { start, end } => {
                    rect_pipeline.flush(pass);
                    flush_image_batch(
                        backend,
                        pass,
                        image_pipeline.as_mut(),
                        image_batch,
                        &mut current_texture_id,
                    );
                    text_pipeline
                        .as_ref()
                        .expect("text decorations require the text pipeline")
                        .render_decoration_range(pass, *start, *end);
                }
                ResolvedCommand::Svg(svg_index) => {
                    rect_pipeline.flush(pass);
                    flush_image_batch(
                        backend,
                        pass,
                        image_pipeline.as_mut(),
                        image_batch,
                        &mut current_texture_id,
                    );
                    svg_pipeline
                        .as_ref()
                        .expect("SVG commands require the SVG pipeline")
                        .draw_item(pass, *svg_index);
                }
                ResolvedCommand::Custom {
                    pipeline_index,
                    command_index,
                } => {
                    rect_pipeline.flush(pass);
                    flush_image_batch(
                        backend,
                        pass,
                        image_pipeline.as_mut(),
                        image_batch,
                        &mut current_texture_id,
                    );
                    if let Some(slot) = custom_pipelines.get(*pipeline_index) {
                        slot.pipeline.render_command(*command_index, pass);
                    }
                }
            }
        }
        flush_image_batch(
            backend,
            pass,
            image_pipeline.as_mut(),
            image_batch,
            &mut current_texture_id,
        );
        rect_pipeline.flush(pass);
    }

    fn collect_commands(
        &mut self,
        backend: &B,
        draw_list: &DrawList,
        command_slices: Option<&[&[DrawCommand]]>,
        frame_size: (u32, u32),
        startup_timing: bool,
        target_offset: (u32, u32),
    ) {
        self.resolved.clear();
        self.text_requests.clear();
        self.decoration_requests.clear();
        self.svg_items.clear();
        self.deferred_outlines.clear();
        self.textures_to_remove.clear();
        self.transform_stack.clear();
        self.clip_stack.clear();
        for slot in &mut self.custom_pipelines {
            slot.pipeline.begin_frame();
        }

        let mut current_transform = Mat3::identity();
        let mut current_scales = (1.0, 1.0);
        let mut current_italic = false;
        let mut current_language = None;
        let mut alpha_state = AlphaState::new();
        let target_translation = target_relative_transform(Mat3::identity(), target_offset);
        let fallback_slices = [draw_list.commands()];
        let command_slices = command_slices.unwrap_or(&fallback_slices);
        for command in command_slices.iter().flat_map(|commands| commands.iter()) {
            match command {
                DrawCommand::PushTransform { matrix } => {
                    self.transform_stack.push(current_transform);
                    alpha_state.save();
                    current_transform = target_translation.mul(matrix).pixel_aligned();
                    current_scales = transform_scales(&current_transform);
                }
                DrawCommand::PopTransform => {
                    if let Some(previous) = self.transform_stack.pop() {
                        current_transform = previous;
                        current_scales = transform_scales(&current_transform);
                    }
                    alpha_state.restore();
                }
                DrawCommand::PushClip {
                    rect,
                    border_radius,
                } => {
                    let (x1, y1) = current_transform.transform_point(rect.x, rect.y);
                    let (x2, y2) = current_transform
                        .transform_point(rect.x + rect.width, rect.y + rect.height);
                    let (sx, _) = current_scales;
                    let next = Rect::new(x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs());
                    let effective = self.clip_stack.last().map_or(next, |parent| {
                        let left = next.x.max(parent.rect.x);
                        let top = next.y.max(parent.rect.y);
                        let right = (next.x + next.width).min(parent.rect.x + parent.rect.width);
                        let bottom = (next.y + next.height).min(parent.rect.y + parent.rect.height);
                        Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
                    });
                    let mut radii = *border_radius;
                    for radius in &mut radii {
                        *radius *= sx;
                    }
                    self.clip_stack.push(ClipState {
                        rect: effective,
                        border_radius: radii,
                    });
                }
                DrawCommand::PopClip => {
                    self.clip_stack.pop();
                }
                DrawCommand::FillRect {
                    rect,
                    color,
                    border_radius,
                    border_width,
                    border_color,
                    outline_width,
                    outline_color,
                } => {
                    let (main, outline) = build_fill_rect_instances(
                        *rect,
                        *color,
                        *border_radius,
                        *border_width,
                        *border_color,
                        *outline_width,
                        *outline_color,
                        &current_transform,
                        current_scales.0,
                        current_scales.1,
                        self.clip_stack.last(),
                        alpha_state.current(),
                    );
                    if let Some(outline) = outline {
                        self.deferred_outlines.push(outline);
                    }
                    self.resolved.push(ResolvedCommand::Rect {
                        instance: main,
                        clear: false,
                    });
                }
                DrawCommand::ClearRect { rect } => {
                    let (x1, y1) = current_transform.transform_point(rect.x, rect.y);
                    let (x2, y2) = current_transform
                        .transform_point(rect.x + rect.width, rect.y + rect.height);
                    self.resolved.push(ResolvedCommand::Rect {
                        instance: RectInstance {
                            position: [x1.min(x2), y1.min(y2)],
                            size: [(x2 - x1).abs(), (y2 - y1).abs()],
                            color: Rgba8::TRANSPARENT,
                            border_radius: [0.0; 4],
                            border_width: [0.0; 4],
                            border_color: Rgba8::TRANSPARENT,
                            outline_width: [0.0; 4],
                            outline_color: Rgba8::TRANSPARENT,
                            clip_rect: clip_to_array(self.clip_stack.last()),
                            clip_border_radius: clip_border_radius(self.clip_stack.last()),
                            shadow_params: [0.0; 4],
                            shadow_color: Rgba8::TRANSPARENT,
                            shadow_flags: [0.0; 4],
                        },
                        clear: true,
                    });
                }
                DrawCommand::DrawText {
                    position,
                    text,
                    font_size,
                    color,
                    bounds_width,
                    bounds_height,
                    overflow,
                    horizontal_align,
                    font_family,
                    font_style,
                    font_weight,
                    shadow,
                    draw_glyphs,
                } => {
                    let (x, y) = current_transform.transform_point(position.x, position.y);
                    let request_index = self.text_requests.len();
                    self.text_requests.push(TextDrawRequest {
                        x,
                        y,
                        text: text.clone(),
                        font_size: *font_size,
                        color: Rgba8::from_unorm(apply_alpha(
                            color.to_array(),
                            alpha_state.current(),
                        )),
                        bounds_width: bounds_width
                            .unwrap_or(frame_size.0 as f32 - target_offset.0 as f32 - x),
                        bounds_height: bounds_height
                            .unwrap_or(frame_size.1 as f32 - target_offset.1 as f32 - y),
                        overflow: *overflow,
                        horizontal_align: *horizontal_align,
                        writing_mode: TextWritingMode::HorizontalTb,
                        line_height: None,
                        shadow: shadow.map(|value| {
                            transform_text_shadow(value, &current_transform, alpha_state.current())
                        }),
                        draw_glyphs: *draw_glyphs,
                        font_family: *font_family,
                        font_style: *font_style,
                        font_weight: Some(*font_weight),
                        language: current_language,
                        italic: current_italic,
                        clip_rect: clip_to_array(self.clip_stack.last()),
                        clip_border_radius: clip_border_radius(self.clip_stack.last()),
                        spans: Vec::new(),
                    });
                    self.resolved.push(ResolvedCommand::Text(request_index));
                }
                DrawCommand::DrawRichText {
                    position,
                    spans,
                    font_size,
                    color,
                    bounds_width,
                    bounds_height,
                    overflow,
                } => {
                    let (x, y) = current_transform.transform_point(position.x, position.y);
                    let request_index = self.text_requests.len();
                    self.text_requests.push(TextDrawRequest {
                        x,
                        y,
                        text: spans
                            .iter()
                            .map(|span| &*span.text)
                            .collect::<String>()
                            .into(),
                        font_size: *font_size,
                        color: Rgba8::from_unorm(apply_alpha(
                            color.to_array(),
                            alpha_state.current(),
                        )),
                        bounds_width: bounds_width
                            .unwrap_or(frame_size.0 as f32 - target_offset.0 as f32 - x),
                        bounds_height: bounds_height
                            .unwrap_or(frame_size.1 as f32 - target_offset.1 as f32 - y),
                        overflow: *overflow,
                        horizontal_align: TextHorizontalAlign::Left,
                        writing_mode: TextWritingMode::HorizontalTb,
                        line_height: None,
                        shadow: None,
                        draw_glyphs: true,
                        font_family: crate::font::FontFamily::SANS_SERIF,
                        font_style: crate::font::FontStyle::Normal,
                        font_weight: None,
                        language: current_language,
                        italic: false,
                        clip_rect: clip_to_array(self.clip_stack.last()),
                        clip_border_radius: clip_border_radius(self.clip_stack.last()),
                        spans: spans
                            .iter()
                            .map(|span| RichTextSpan {
                                text: span.text.clone(),
                                font_size: span.font_size,
                                color: span.color.map(|color| {
                                    Rgba8::from_unorm(apply_alpha(
                                        color.to_array(),
                                        alpha_state.current(),
                                    ))
                                }),
                                font_weight: span.font_weight,
                                italic: span.italic,
                            })
                            .collect(),
                    });
                    self.resolved.push(ResolvedCommand::Text(request_index));
                }
                DrawCommand::DrawTextDecoration {
                    rect,
                    color,
                    style,
                    thickness,
                    period,
                } => {
                    let (sx, sy) = current_scales;
                    let (x1, y1) = current_transform.transform_point(rect.x, rect.y);
                    let (x2, y2) = current_transform
                        .transform_point(rect.x + rect.width, rect.y + rect.height);
                    let index = self.decoration_requests.len();
                    self.decoration_requests.push(TextDecorationDraw {
                        x: x1.min(x2),
                        y: y1.min(y2),
                        width: (x2 - x1).abs(),
                        band_height: (y2 - y1).abs(),
                        thickness: (*thickness * sy).max(1.0),
                        period: (*period * sx).max(1.0),
                        style: *style,
                        color: Rgba8::from_unorm(apply_alpha(
                            color.to_array(),
                            alpha_state.current(),
                        )),
                        clip_rect: clip_to_array(self.clip_stack.last()),
                        clip_border_radius: clip_border_radius(self.clip_stack.last()),
                    });
                    self.resolved.push(ResolvedCommand::TextDecoration {
                        start: index,
                        end: index + 1,
                    });
                }
                DrawCommand::Svg {
                    scene,
                    destination,
                    overrides,
                } => {
                    let index = self.svg_items.len();
                    self.svg_items.push(resolve_svg_item(
                        scene.clone(),
                        *destination,
                        overrides.clone(),
                        current_transform,
                        self.clip_stack.last(),
                        alpha_state.current(),
                    ));
                    self.resolved.push(ResolvedCommand::Svg(index));
                }
                DrawCommand::SetTransform { matrix } => {
                    current_transform = target_translation.mul(matrix).pixel_aligned();
                    current_scales = transform_scales(&current_transform);
                }
                DrawCommand::SetAlpha { alpha } => alpha_state.set(*alpha),
                DrawCommand::RestoreAlpha => alpha_state.set(1.0),
                DrawCommand::SetItalic { italic } => current_italic = *italic,
                DrawCommand::SetTextLanguage { language } => current_language = *language,
                DrawCommand::DrawImage { rect, texture_id } => {
                    let (x1, y1) = current_transform.transform_point(rect.x, rect.y);
                    let (x2, y2) = current_transform
                        .transform_point(rect.x + rect.width, rect.y + rect.height);
                    self.resolved.push(ResolvedCommand::Image {
                        texture_id: *texture_id,
                        instance: ImageInstance {
                            position: [x1.min(x2), y1.min(y2)],
                            size: [(x2 - x1).abs(), (y2 - y1).abs()],
                            uv_offset: [0.0, 0.0],
                            uv_scale: [1.0, 1.0],
                            clip_rect: clip_to_array(self.clip_stack.last()),
                            clip_border_radius: clip_border_radius(self.clip_stack.last()),
                            alpha: alpha_state.current(),
                            source_premultiplied: 0.0,
                        },
                    });
                }
                DrawCommand::DrawImageWithResource { rect, resource } => {
                    self.image_pipeline
                        .as_mut()
                        .expect("retained image resources require the image pipeline")
                        .upload_retained_image_with_id(
                            backend,
                            resource.texture_id(),
                            resource.revision(),
                            resource.width(),
                            resource.height(),
                            resource.rgba(),
                        );
                    let (x1, y1) = current_transform.transform_point(rect.x, rect.y);
                    let (x2, y2) = current_transform
                        .transform_point(rect.x + rect.width, rect.y + rect.height);
                    self.resolved.push(ResolvedCommand::Image {
                        texture_id: resource.texture_id(),
                        instance: ImageInstance {
                            position: [x1.min(x2), y1.min(y2)],
                            size: [(x2 - x1).abs(), (y2 - y1).abs()],
                            uv_offset: [0.0, 0.0],
                            uv_scale: [1.0, 1.0],
                            clip_rect: clip_to_array(self.clip_stack.last()),
                            clip_border_radius: clip_border_radius(self.clip_stack.last()),
                            alpha: alpha_state.current(),
                            source_premultiplied: 0.0,
                        },
                    });
                }
                DrawCommand::RetainedLayer { layer_id, rect, .. } => {
                    if let Some(layer) = self.retained_layers.get(layer_id) {
                        if let Some(bind_group) = layer.bind_group.clone() {
                            let (x1, y1) = current_transform.transform_point(rect.x, rect.y);
                            let (x2, y2) = current_transform
                                .transform_point(rect.x + rect.width, rect.y + rect.height);
                            self.resolved.push(ResolvedCommand::RetainedLayer {
                                bind_group,
                                instance: ImageInstance {
                                    position: [x1.min(x2), y1.min(y2)],
                                    size: [(x2 - x1).abs(), (y2 - y1).abs()],
                                    uv_offset: [0.0, 0.0],
                                    uv_scale: [1.0, 1.0],
                                    clip_rect: clip_to_array(self.clip_stack.last()),
                                    clip_border_radius: clip_border_radius(self.clip_stack.last()),
                                    alpha: alpha_state.current(),
                                    source_premultiplied: 1.0,
                                },
                            });
                        }
                    }
                }
                DrawCommand::LoadImage {
                    bytes,
                    texture_id,
                    width,
                    height,
                } => {
                    self.image_pipeline
                        .as_mut()
                        .expect("image uploads require the image pipeline")
                        .upload_if_absent(backend, *texture_id, *width, *height, bytes);
                }
                DrawCommand::LoadImageWithId {
                    texture_id,
                    bytes,
                    width,
                    height,
                } => self
                    .image_pipeline
                    .as_mut()
                    .expect("image uploads require the image pipeline")
                    .upload_image_with_id(backend, *texture_id, *width, *height, bytes),
                DrawCommand::RemoveTexture { texture_id } => {
                    self.textures_to_remove.push(*texture_id);
                }
                DrawCommand::DrawShadowRect {
                    rect,
                    shadow_color,
                    shadow_params,
                    border_radius,
                    inset,
                    side_params,
                } => {
                    let (sx, sy) = current_scales;
                    let expand_x =
                        shadow_params[2] + shadow_params[3].abs() + shadow_params[0].abs();
                    let expand_y =
                        shadow_params[2] + shadow_params[3].abs() + shadow_params[1].abs();
                    let (x1, y1) =
                        current_transform.transform_point(rect.x - expand_x, rect.y - expand_y);
                    let (x2, y2) = current_transform.transform_point(
                        rect.x + rect.width + expand_x,
                        rect.y + rect.height + expand_y,
                    );
                    let mut radii = *border_radius;
                    for radius in &mut radii {
                        *radius *= sx;
                    }
                    self.resolved.push(ResolvedCommand::Rect {
                        instance: RectInstance {
                            position: [x1.min(x2), y1.min(y2)],
                            size: [(x2 - x1).abs(), (y2 - y1).abs()],
                            color: Rgba8::TRANSPARENT,
                            border_radius: radii,
                            border_width: [0.0; 4],
                            border_color: Rgba8::TRANSPARENT,
                            outline_width: [0.0; 4],
                            outline_color: Rgba8::TRANSPARENT,
                            clip_rect: clip_to_array(self.clip_stack.last()),
                            clip_border_radius: clip_border_radius(self.clip_stack.last()),
                            shadow_params: [
                                shadow_params[0] * sx,
                                shadow_params[1] * sy,
                                shadow_params[2] * sx,
                                shadow_params[3] * sx,
                            ],
                            shadow_color: packed_color(*shadow_color, alpha_state.current()),
                            shadow_flags: [
                                if *inset { 1.0 } else { 0.0 },
                                side_params[0],
                                side_params[1],
                                side_params[2],
                            ],
                        },
                        clear: false,
                    });
                }
                DrawCommand::Custom {
                    pipeline_name,
                    data,
                } => {
                    if pipeline_name == MATERIAL_PIPELINE_NAME
                        && self.material_pipeline_enabled
                        && !self.material_pipeline_initialized
                    {
                        let material_pipeline_started = dx12_stage_start!(startup_timing);
                        self.install_material_pipeline(backend);
                        dx12_stage_finish!(
                            material_pipeline_started,
                            "lazy material pipeline initialization"
                        );
                    }
                    let Some(pipeline_index) = self
                        .custom_pipelines
                        .iter()
                        .position(|slot| slot.pipeline.name() == pipeline_name.as_str())
                    else {
                        continue;
                    };
                    let command_index = if pipeline_name == MATERIAL_PIPELINE_NAME {
                        let request = data
                            .downcast_ref::<Vec<u8>>()
                            .and_then(|bytes| {
                                crate::pipeline::material::MaterialRequest::decode(bytes).ok()
                            })
                            .map(|request| {
                                resolve_material_request(
                                    request,
                                    &current_transform,
                                    current_scales,
                                    self.clip_stack.last(),
                                    alpha_state.current(),
                                )
                            });
                        if let Some(request) = request {
                            self.custom_pipelines[pipeline_index]
                                .pipeline
                                .prepare_command(&request)
                        } else {
                            self.custom_pipelines[pipeline_index]
                                .pipeline
                                .prepare_command(data.as_ref())
                        }
                    } else {
                        self.custom_pipelines[pipeline_index]
                            .pipeline
                            .prepare_command(data.as_ref())
                    };
                    self.resolved.push(ResolvedCommand::Custom {
                        pipeline_index,
                        command_index,
                    });
                }
            }
        }
        for instance in self.deferred_outlines.iter().copied() {
            self.resolved.push(ResolvedCommand::Rect {
                instance,
                clear: false,
            });
        }
    }

    fn prepare_retained_layers(&mut self, backend: &B, draw_list: &DrawList, is_srgb: bool) {
        let frame_stats = self.compositor_stats;
        let mut rasterized_surfaces = 0usize;
        let mut reused_surfaces = 0usize;
        let mut composed_surfaces = 0usize;
        for command in draw_list.commands() {
            let DrawCommand::RetainedLayer {
                layer_id,
                rect,
                content,
            } = command
            else {
                continue;
            };
            match self.prepare_retained_layer(backend, *layer_id, *rect, content, is_srgb) {
                RetainedLayerPreparation::Rasterized => {
                    rasterized_surfaces = rasterized_surfaces.saturating_add(1);
                    composed_surfaces = composed_surfaces.saturating_add(1);
                }
                RetainedLayerPreparation::Reused => {
                    reused_surfaces = reused_surfaces.saturating_add(1);
                    composed_surfaces = composed_surfaces.saturating_add(1);
                }
                RetainedLayerPreparation::Skipped => {}
            }
        }

        // Rasterizing a layer uses the same renderer recursively. Keep the
        // enclosing window-frame counters and add the work done for its layers.
        self.compositor_stats = frame_stats;
        self.compositor_stats.rasterized_surfaces = self
            .compositor_stats
            .rasterized_surfaces
            .saturating_add(rasterized_surfaces);
        self.compositor_stats.reused_surfaces = self
            .compositor_stats
            .reused_surfaces
            .saturating_add(reused_surfaces);
        self.compositor_stats.composed_surfaces = self
            .compositor_stats
            .composed_surfaces
            .saturating_add(composed_surfaces);
    }

    fn prepare_retained_layer(
        &mut self,
        backend: &B,
        layer_id: u64,
        rect: Rect,
        content: &Arc<RetainedLayerContent>,
        is_srgb: bool,
    ) -> RetainedLayerPreparation {
        let Some((width, height)) =
            retained_layer_dimensions(rect, backend.limits().max_texture_dimension_2d)
        else {
            return RetainedLayerPreparation::Skipped;
        };
        let entry = self
            .retained_layers
            .entry(layer_id)
            .or_insert_with(|| RetainedLayerGeneric {
                content: content.clone(),
                target: PersistentTargetGeneric::default(),
                bind_group: None,
                width,
                height,
                bytes: width as u64 * height as u64 * 4,
                last_used_frame: self.frame_index,
                is_srgb,
            });
        if entry.width != width || entry.height != height || entry.is_srgb != is_srgb {
            entry.width = width;
            entry.height = height;
            entry.bytes = width as u64 * height as u64 * 4;
            entry.is_srgb = is_srgb;
            entry.target.invalidate();
        }
        if !Arc::ptr_eq(&entry.content, content) {
            entry.content = content.clone();
            entry.target.invalidate();
        }
        entry.last_used_frame = self.frame_index;
        let key =
            PersistentTargetKey::new(width, height, 1.0, layer_id, 0, 0, TargetValidity::Valid);
        let result = entry.target.ensure(backend, self.format, key);
        if matches!(
            result,
            TargetEnsureResult::Created | TargetEnsureResult::Recreated
        ) {
            if let Some(target_view) = entry.target.view() {
                entry.bind_group = Some(
                    self.image_pipeline
                        .as_ref()
                        .expect("retained layers require the image pipeline")
                        .create_external_bind_group(backend, target_view),
                );
            }
        }
        if matches!(result, TargetEnsureResult::ReusedValid) {
            return RetainedLayerPreparation::Reused;
        }
        let Some(target_view) = entry.target.view().cloned() else {
            return RetainedLayerPreparation::Skipped;
        };
        let layer_draw_list = content.to_draw_list();
        self.render_with_source_texture(
            backend,
            &target_view,
            None,
            width,
            height,
            is_srgb,
            &layer_draw_list,
        );
        if let Some(entry) = self.retained_layers.get_mut(&layer_id) {
            entry.target.mark_valid();
            if entry.bind_group.is_none() {
                if let Some(target_view) = entry.target.view() {
                    entry.bind_group = Some(
                        self.image_pipeline
                            .as_ref()
                            .expect("retained layers require the image pipeline")
                            .create_external_bind_group(backend, target_view),
                    );
                }
            }
        }
        RetainedLayerPreparation::Rasterized
    }

    fn reclaim_retained_layers(&mut self) {
        let frame_index = self.frame_index;
        self.v2_command_cache.retain(|_, cached| {
            frame_index.saturating_sub(cached.last_used_frame) < RETAINED_LAYER_IDLE_FRAMES
        });
        let mut total_bytes = self
            .retained_layers
            .values()
            .map(|layer| layer.bytes)
            .sum::<u64>();
        let mut candidates = self
            .retained_layers
            .iter()
            .map(|(&id, layer)| (id, layer.last_used_frame, layer.bytes))
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(_, last_used, _)| *last_used);
        for (id, last_used, bytes) in candidates {
            let idle = self.frame_index.saturating_sub(last_used);
            if idle < RETAINED_LAYER_IDLE_FRAMES && total_bytes <= RETAINED_LAYER_CACHE_BUDGET_BYTES
            {
                break;
            }
            if self.retained_layers.remove(&id).is_some() {
                total_bytes = total_bytes.saturating_sub(bytes);
            }
        }
    }
}

fn opacity_group_region<B: GpuBackend>(
    region: DamageRect,
    groups: &[ActiveOpacityGroup<B>],
    bounds: DamageRect,
) -> Option<DamageRect> {
    groups
        .iter()
        .try_fold(intersect_damage_rects(region, bounds)?, |region, group| {
            intersect_damage_rects(region, group.bounds)
        })
}

fn opacity_group_scissor<B: GpuBackend>(
    region: DamageRect,
    groups: &[ActiveOpacityGroup<B>],
) -> Option<DamageRect> {
    groups.iter().try_fold(region, |region, group| {
        intersect_damage_rects(region, group.bounds)
    })
}

fn intersect_damage_rects(left: DamageRect, right: DamageRect) -> Option<DamageRect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width)
        .min(right.x.saturating_add(right.width));
    let bottom_edge = left
        .y
        .saturating_add(left.height)
        .min(right.y.saturating_add(right.height));
    if right_edge <= x || bottom_edge <= y {
        return None;
    }
    Some(DamageRect::new(x, y, right_edge - x, bottom_edge - y))
}

fn target_relative_transform(matrix: Mat3, target_offset: (u32, u32)) -> Mat3 {
    Mat3::translate(-(target_offset.0 as f32), -(target_offset.1 as f32))
        .mul(&matrix)
        .pixel_aligned()
}

fn flush_image_batch<'pass, B: GpuBackend>(
    backend: &B,
    pass: &mut B::RenderPass<'pass>,
    image_pipeline: Option<&mut ImagePipeline<B>>,
    image_batch: &mut Vec<ImageInstance>,
    texture_id: &mut Option<TextureId>,
) {
    if let Some(id) = texture_id.take() {
        if !image_batch.is_empty() {
            image_pipeline
                .expect("image draws require the image pipeline")
                .draw_batch(backend, pass, id, image_batch);
            image_batch.clear();
        }
    }
}

fn draw_list_uses_custom_pipeline(draw_list: &DrawList) -> bool {
    draw_list
        .commands()
        .iter()
        .any(|command| matches!(command, DrawCommand::Custom { .. }))
}

fn render_plan_uses_custom_pipeline(plan: &RetainedRenderPlan, width: u32, height: u32) -> bool {
    plan.operations_for_region(crate::damage_region::DamageRect::new(0, 0, width, height))
        .any(|operation| match &operation.kind {
            crate::frame::RetainedRenderOperationKind::LocalV2(item) => {
                item.commands.iter().any(|command| {
                    matches!(command, crate::draw_cmd_v2::DrawCommand::DrawCustom { .. })
                })
            }
            crate::frame::RetainedRenderOperationKind::OpacityGroupBegin { .. }
            | crate::frame::RetainedRenderOperationKind::OpacityGroupEnd { .. } => false,
        })
}

/// Compatibility name for the backend-generic [`Renderer`].
pub type RendererImpl<B = crate::backend::DefaultGpuBackend> = Renderer<B>;

#[cfg(test)]
#[path = "generic_renderer/material_detection_tests.rs"]
mod material_detection_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::damage_region::DamageSet;
    use crate::draw_cmd_v2::{DrawCommand, Rect, RenderFrame, RenderPaintSource, RenderTree};
    use crate::frame::{FramePacket, FrameRenderMetadata};
    use crate::utilities::Mat3;

    use super::{
        MATERIAL_PIPELINE_NAME, render_plan_uses_custom_pipeline, render_plan_uses_material,
        target_relative_transform,
    };

    #[test]
    fn cropped_group_transform_maps_world_pixels_into_local_surface_pixels() {
        let world = Mat3::translate(52.0, 27.0).mul(&Mat3::scale(2.0, 3.0));
        let local = target_relative_transform(world, (40, 20));

        assert_eq!(local.transform_point(4.0, 5.0), (20.0, 22.0));
        assert_eq!(local.transform_point(0.0, 0.0), (12.0, 7.0));
    }

    #[test]
    fn retained_material_commands_are_detected_before_target_selection() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        tree.set_paint_source(root, RenderPaintSource::LocalV2)
            .unwrap();
        tree.context(root)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(vec![DrawCommand::DrawCustom {
                pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                data: Arc::from(vec![1_u8, 2, 3]),
            }])
            .unwrap();
        let packet = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.0, 1, 1, 1, 1, DamageSet::new(64, 64)),
        )
        .unwrap();
        let plan = packet.render_plan().unwrap();

        assert!(render_plan_uses_custom_pipeline(plan, 64, 64));
        assert!(render_plan_uses_material(plan, 64, 64));
    }
}
