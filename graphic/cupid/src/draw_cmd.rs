use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use hashbrown::DefaultHashBuilder;

use crate::draw_cmd_v2::ImageResource;
use crate::font::{FontFamily, FontStyle, TextLanguage};
use crate::svg::{SvgNodeStyleOverride, SvgScene};
use crate::text_pipeline::text_layout::TextHorizontalAlign;
use crate::text_pipeline::{TextOverflowMode, TextShadowRequest};
#[cfg(test)]
use crate::utilities::Rgba8;
use crate::utilities::{Color, Mat3, Rect, TextureId, Vec2d};

// Texture IDs are content identities across frames; keep one random seed so
// identical images do not receive a new ID on every call.
static IMAGE_TEXTURE_HASHER: OnceLock<DefaultHashBuilder> = OnceLock::new();

/// Maximum texture dimensions used by a single retained scroll layer. Larger
/// static subtrees stay on the direct path until tiled retention is available.
pub const RETAINED_LAYER_MAX_DIMENSION: u32 = 8_192;

/// Maximum allocation for one retained scroll layer.
pub const RETAINED_LAYER_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Edge length used for each retained scroll tile when one full layer is too
/// large. Tiles are deliberately modest so the scroll cache can keep a
/// viewport-sized working set without allocating the whole document.
pub const RETAINED_LAYER_TILE_SIZE: u32 = 1_024;

/// Maximum number of retained tiles selected for one scroll frame. A malformed
/// or unusually large viewport falls back to direct drawing rather than
/// turning one frame into an unbounded tile-recording job.
pub const RETAINED_LAYER_MAX_TILES_PER_FRAME: usize = 64;

/// Physical-pixel padding around a retained layer's local content.
///
/// The order is left, top, right, bottom. Padding is used for paint that is
/// allowed to bleed outside the layer's logical bounds, such as a text shadow.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RetainedLayerPadding {
    /// Pixels kept to the left of the content origin.
    pub left: f32,
    /// Pixels kept above the content origin.
    pub top: f32,
    /// Pixels kept to the right of the content bounds.
    pub right: f32,
    /// Pixels kept below the content bounds.
    pub bottom: f32,
}

impl RetainedLayerPadding {
    /// No extra pixels around the layer.
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// Creates padding in physical pixels.
    #[inline]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[inline]
    fn is_valid(self) -> bool {
        self.left.is_finite()
            && self.top.is_finite()
            && self.right.is_finite()
            && self.bottom.is_finite()
            && self.left >= 0.0
            && self.top >= 0.0
            && self.right >= 0.0
            && self.bottom >= 0.0
    }
}

/// Allocates an ID for a renderer-owned retained layer.
///
/// IDs are process-wide because a renderer receives scroll and widget layers
/// through the same command stream. Callers should allocate once and retain
/// the result for the lifetime of the cached layer.
#[inline]
pub fn next_retained_layer_id() -> u64 {
    static NEXT_RETAINED_LAYER_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_RETAINED_LAYER_ID.fetch_add(1, Ordering::Relaxed)
}

/// Shared state between the canvas-side draw list and the renderer.
///
/// A finished draw list can be moved to the raster thread while the canvas
/// starts recording the next frame. The registry therefore carries only the
/// small amount of cross-thread state needed to tell retained image widgets
/// that the renderer evicted one of their GPU textures.
#[derive(Default)]
pub(crate) struct TextureRegistry {
    cache_epoch: AtomicU64,
    next_serial: AtomicU64,
    textures: Mutex<HashMap<TextureId, RegistryTexture>>,
    references: Mutex<HashMap<TextureId, u32>>,
}

struct RegistryTexture {
    width: u32,
    height: u32,
    serial: u64,
    available: bool,
}

impl TextureRegistry {
    #[inline]
    fn cache_epoch(&self) -> u64 {
        self.cache_epoch.load(Ordering::Acquire)
    }

    #[inline]
    fn changed(&self) {
        self.cache_epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn record_size(
        &self,
        texture_id: TextureId,
        width: u32,
        height: u32,
        force_new_serial: bool,
    ) -> u64 {
        let (became_available, serial) = {
            let mut textures = self.textures.lock().unwrap();
            match textures.get_mut(&texture_id) {
                Some(texture)
                    if !force_new_serial
                        && texture.available
                        && texture.width == width
                        && texture.height == height =>
                {
                    return texture.serial;
                }
                Some(texture) => {
                    let serial = self
                        .next_serial
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1);
                    let became_available = !texture.available;
                    texture.width = width;
                    texture.height = height;
                    texture.serial = serial;
                    texture.available = true;
                    (became_available, serial)
                }
                None => {
                    let serial = self
                        .next_serial
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1);
                    textures.insert(
                        texture_id,
                        RegistryTexture {
                            width,
                            height,
                            serial,
                            available: true,
                        },
                    );
                    (false, serial)
                }
            }
        };
        if became_available {
            self.changed();
        }
        serial
    }

    fn texture_state(&self, texture_id: TextureId) -> Option<(u32, u32, u64, bool)> {
        self.textures
            .lock()
            .unwrap()
            .get(&texture_id)
            .map(|texture| {
                (
                    texture.width,
                    texture.height,
                    texture.serial,
                    texture.available,
                )
            })
    }

    fn is_texture_available_or_unknown(&self, texture_id: TextureId) -> bool {
        self.textures
            .lock()
            .unwrap()
            .get(&texture_id)
            .map_or(true, |texture| texture.available)
    }

    fn retain_reference(&self, texture_id: TextureId) {
        *self
            .references
            .lock()
            .unwrap()
            .entry(texture_id)
            .or_default() += 1;
    }

    fn release_reference(&self, texture_id: TextureId) {
        let mut references = self.references.lock().unwrap();
        if let Some(count) = references.get_mut(&texture_id) {
            if *count <= 1 {
                references.remove(&texture_id);
            } else {
                *count -= 1;
            }
        }
    }

    fn is_referenced(&self, texture_id: TextureId) -> bool {
        self.references
            .lock()
            .unwrap()
            .get(&texture_id)
            .is_some_and(|count| *count > 0)
    }

    fn remove_if_current(&self, texture_id: TextureId, serial: Option<u64>) {
        let mut textures = self.textures.lock().unwrap();
        let removed = textures
            .get_mut(&texture_id)
            .filter(|texture| serial.is_none_or(|serial| texture.serial == serial))
            .map(|texture| {
                let was_available = texture.available;
                texture.available = false;
                was_available
            })
            .unwrap_or(false);
        drop(textures);
        if removed {
            self.changed();
        }
    }

    fn mark_evicted_if_current(&self, texture_id: TextureId, serial: u64) {
        let mut textures = self.textures.lock().unwrap();
        let changed = textures
            .get_mut(&texture_id)
            .filter(|texture| texture.serial == serial && texture.available)
            .map(|texture| {
                texture.available = false;
            })
            .is_some();
        drop(textures);
        if changed {
            self.changed();
        }
    }
}

#[derive(Clone, Debug)]
pub struct RichTextSegment {
    // Shared reference-counted text so cloning a segment (the draw list is
    // rebuilt every frame) does not reallocate the string.
    pub text: Arc<str>,
    pub font_size: Option<f32>,
    pub color: Option<Color>,
    pub font_weight: Option<u16>,
    pub italic: Option<bool>,
}

impl RichTextSegment {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            font_size: None,
            color: None,
            font_weight: None,
            italic: None,
        }
    }

    pub fn with_style(mut self, font_size: Option<f32>, color: Option<Color>) -> Self {
        self.font_size = font_size;
        self.color = color;
        self
    }
}

pub enum DrawCommand {
    FillRect {
        rect: Rect,
        color: Color,
        /// Per-corner border radius: [top-left, top-right, bottom-right,
        /// bottom-left]
        border_radius: [f32; 4],
        /// Per-side border width: [top, right, bottom, left]
        border_width: [f32; 4],
        border_color: Color,
        /// Per-side outline width: [top, right, bottom, left]
        outline_width: [f32; 4],
        outline_color: Color,
    },
    ClearRect {
        rect: Rect,
    },
    DrawText {
        position: Vec2d,
        text: Arc<str>,
        font_size: f32,
        color: Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: TextOverflowMode,
        horizontal_align: TextHorizontalAlign,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        shadow: Option<TextShadowRequest>,
        draw_glyphs: bool,
    },
    DrawRichText {
        position: Vec2d,
        spans: Vec<RichTextSegment>,
        font_size: f32,
        color: Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: TextOverflowMode,
    },
    /// A single styled text-decoration line (underline/overline/line-through).
    /// `rect` is the decoration band in local coordinates (`rect.height` is the
    /// band height); the text engine draws the styled stroke inside it.
    DrawTextDecoration {
        rect: Rect,
        color: Color,
        /// Style id, see `aimer_style::TextDecorationStyle::id`.
        style: u32,
        /// Stroke thickness in logical pixels.
        thickness: f32,
        /// Repeat period for dotted/dashed/wavy styles (logical pixels).
        period: f32,
    },
    PushClip {
        rect: Rect,
        border_radius: [f32; 4],
    },
    PopClip,
    PushTransform {
        matrix: Mat3,
    },
    PopTransform,
    SetAlpha {
        alpha: f32,
    },
    RestoreAlpha,
    /// Sets the italic state applied to subsequent plain `DrawText` commands.
    /// Rich text carries italic per span instead, so it is unaffected.
    SetItalic {
        italic: bool,
    },
    /// Sets the written language of subsequent text commands.
    ///
    /// Han is unified, so a run of ideographs does not say whether it wants a
    /// Chinese or a Japanese face: `你好` is covered by both and stays on
    /// whichever the platform's cascade prefers, until a character only one
    /// language writes is added and the whole word changes typeface. A
    /// producer that knows better — a text field knows the keyboard it is
    /// edited with — announces it once here and every text it draws
    /// afterwards is resolved, shaped and measured in that language, exactly
    /// as [`SetItalic`] applies to the text after it.
    ///
    /// `None` restores the default: the run is judged on its own characters.
    ///
    /// [`SetItalic`]: DrawCommand::SetItalic
    SetTextLanguage {
        language: Option<TextLanguage>,
    },
    LoadImage {
        bytes: Vec<u8>,
        texture_id: TextureId,
        width: u32,
        height: u32,
    },
    LoadImageWithId {
        texture_id: TextureId,
        bytes: Vec<u8>,
        width: u32,
        height: u32,
    },
    RemoveTexture {
        texture_id: TextureId,
    },
    DrawImage {
        rect: Rect,
        texture_id: TextureId,
    },
    /// Draws an image while retaining the pixels needed to upload its texture.
    DrawImageWithResource {
        rect: Rect,
        resource: Arc<ImageResource>,
    },
    /// Composite a renderer-owned texture containing a retained static
    /// subtree. The content is kept alongside the command so the renderer can
    /// rasterize it on the first use or after invalidation, while later frames
    /// only submit the layer quad.
    RetainedLayer {
        layer_id: u64,
        rect: Rect,
        content: Arc<RetainedLayerContent>,
    },
    Svg {
        scene: Arc<SvgScene>,
        destination: Rect,
        overrides: Arc<[SvgNodeStyleOverride]>,
    },
    SetTransform {
        matrix: Mat3,
    },
    DrawShadowRect {
        rect: Rect,
        shadow_color: Color,
        /// [offset_x, offset_y, blur, spread]
        shadow_params: [f32; 4],
        border_radius: [f32; 4],
        inset: bool,
        /// [side_type, angle_start, angle_end]
        side_params: [f32; 3],
    },
    /// Draw using a user-registered custom pipeline.
    /// `pipeline_name` must match the name returned by
    /// `CustomPipelineGeneric::name()`. `data` is an arbitrary payload forwarded
    /// to `CustomPipelineGeneric::prepare_command()`.
    Custom {
        pipeline_name: String,
        data: Box<dyn Any + Send>,
    },
}

/// A reusable, local-coordinate draw-command stream for a static subtree.
///
/// The stream is deliberately a separate type instead of `Clone` on
/// [`DrawCommand`]. Image uploads, rich-text span vectors, and custom pipeline
/// payloads can carry large or non-cloneable state, so a snapshot containing
/// any of them is rejected. The commands that are retained share their text,
/// SVG, and other reference-counted payloads; replaying them therefore only
/// copies the small command values and does not rebuild the widget subtree.
///
/// Commands are recorded with an identity transform. Replay composes every
/// local transform with the caller's current canvas transform, which is what
/// lets a scroll frame change its translation and clip without invalidating
/// the retained content.
pub struct RetainedDrawList {
    commands: Box<[DrawCommand]>,
    texture_registry: Arc<TextureRegistry>,
    texture_ids: Box<[TextureId]>,
}

impl Clone for RetainedDrawList {
    fn clone(&self) -> Self {
        Self {
            commands: self
                .commands
                .iter()
                .map(|command| {
                    command
                        .clone_for_retention()
                        .expect("retained draw lists contain only cloneable commands")
                })
                .collect(),
            texture_registry: self.texture_registry.clone(),
            texture_ids: self.texture_ids.clone(),
        }
    }
}

impl RetainedDrawList {
    fn from_draw_list(draw_list: &DrawList) -> Option<Self> {
        let mut transform_depth = 0_usize;
        let mut clip_depth = 0_usize;
        let mut commands = Vec::with_capacity(draw_list.commands.len());

        for command in &draw_list.commands {
            match command {
                DrawCommand::PushTransform { .. } => transform_depth += 1,
                DrawCommand::PopTransform if transform_depth > 0 => transform_depth -= 1,
                DrawCommand::PopTransform => return None,
                DrawCommand::PushClip { .. } => clip_depth += 1,
                DrawCommand::PopClip if clip_depth > 0 => clip_depth -= 1,
                DrawCommand::PopClip => return None,
                _ => {}
            }

            commands.push(command.clone_for_retention()?);
        }

        (transform_depth == 0 && clip_depth == 0).then_some(Self {
            commands: commands.into_boxed_slice(),
            texture_registry: draw_list.texture_registry.clone(),
            texture_ids: draw_list
                .referenced_textures
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }

    /// Returns the number of commands kept by this retained stream.
    #[inline]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns whether this retained stream contains no commands.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    #[inline]
    fn texture_ids(&self) -> &[TextureId] {
        &self.texture_ids
    }
}

/// Shared payload for a compositor-style retained scroll layer.
///
/// The snapshot is immutable after construction. It is behind a mutex because
/// [`DrawCommand`] also supports custom payloads that are only `Send`; keeping
/// the retained stream in this small synchronization wrapper lets a finished
/// frame cross to the raster thread without making the general draw-command
/// enum `Sync`. The mutex is only taken when a layer is first rasterized or
/// invalidated, never on a cache-hit scroll frame.
pub struct RetainedLayerContent {
    snapshot: Mutex<RetainedDrawList>,
    texture_ids: Box<[TextureId]>,
    compositor_safe: bool,
    padding: RetainedLayerPadding,
}

impl RetainedLayerContent {
    /// Wraps a validated retained snapshot for renderer-side layer caching.
    #[inline]
    pub fn from_snapshot(snapshot: RetainedDrawList) -> Self {
        Self::from_snapshot_with_padding(snapshot, RetainedLayerPadding::ZERO)
    }

    /// Wraps a validated snapshot and records physical pixels reserved around
    /// its logical bounds for paint that bleeds outside them.
    ///
    /// A shadowed text command is compositor-safe only when this padding is
    /// large enough for its finite, sanitized offset and blur. Unsupported or
    /// insufficiently padded content remains valid for direct replay.
    #[inline]
    pub fn from_snapshot_with_padding(
        snapshot: RetainedDrawList,
        padding: RetainedLayerPadding,
    ) -> Self {
        let texture_ids = snapshot.texture_ids().into();
        let compositor_safe = padding.is_valid()
            && snapshot.commands.iter().all(|command| match command {
                DrawCommand::FillRect { outline_width, .. } => {
                    outline_width.iter().all(|width| *width <= 0.0)
                }
                DrawCommand::DrawText { shadow, .. } => {
                    shadow.is_none_or(|shadow| shadow_is_covered(shadow, padding))
                }
                DrawCommand::DrawShadowRect { .. } => false,
                _ => true,
            });
        Self {
            snapshot: Mutex::new(snapshot),
            texture_ids,
            compositor_safe,
            padding,
        }
    }

    #[inline]
    pub(crate) fn texture_ids(&self) -> &[TextureId] {
        &self.texture_ids
    }

    /// Returns whether the snapshot can be rasterized into a layer without
    /// losing pixels that intentionally bleed outside its local bounds.
    ///
    /// Effects with unbounded or neighbouring paint semantics stay on the
    /// ordinary retained-command path. That path is slightly more expensive,
    /// but preserves the draw order and edge behaviour of the original tree.
    pub fn is_compositor_safe(&self) -> bool {
        self.compositor_safe
    }

    /// Returns the physical pixels reserved on the left, top, right, and
    /// bottom edges of this layer.
    #[inline]
    pub fn padding(&self) -> RetainedLayerPadding {
        self.padding
    }

    /// Materializes a temporary draw list for a layer refresh.
    ///
    /// This clones only the already-validated, retention-safe commands. It is
    /// deliberately off the offset-only path and therefore paid only when the
    /// layer has no GPU texture yet or its content was invalidated.
    pub(crate) fn to_draw_list(&self) -> DrawList {
        let snapshot = self.snapshot.lock().unwrap();
        let mut draw_list = DrawList::with_texture_registry(snapshot.texture_registry.clone());
        if self.padding != RetainedLayerPadding::ZERO {
            draw_list.translate(self.padding.left, self.padding.top);
        }
        let base = *draw_list.current_transform();
        draw_list.append_retained(&snapshot, base);
        draw_list
    }

    fn append_to_draw_list(&self, draw_list: &mut DrawList, base: Mat3) {
        let snapshot = self.snapshot.lock().unwrap();
        draw_list.append_retained(&snapshot, base);
    }
}

#[inline]
fn shadow_is_covered(shadow: TextShadowRequest, padding: RetainedLayerPadding) -> bool {
    if shadow.color.0[3] == 0 {
        return true;
    }
    let offset_x = shadow
        .offset_x
        .is_finite()
        .then_some(shadow.offset_x)
        .unwrap_or(0.0);
    let offset_y = shadow
        .offset_y
        .is_finite()
        .then_some(shadow.offset_y)
        .unwrap_or(0.0);
    let blur = shadow
        .blur
        .is_finite()
        .then_some(shadow.blur.max(0.0))
        .unwrap_or(0.0);
    padding.left >= (blur - offset_x).max(0.0)
        && padding.right >= (blur + offset_x).max(0.0)
        && padding.top >= (blur - offset_y).max(0.0)
        && padding.bottom >= (blur + offset_y).max(0.0)
}

#[inline]
fn retained_layer_size_is_supported(width: f32, height: f32) -> bool {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return false;
    }
    let width = width.ceil().max(1.0) as u64;
    let height = height.ceil().max(1.0) as u64;
    width <= u64::from(RETAINED_LAYER_MAX_DIMENSION)
        && height <= u64::from(RETAINED_LAYER_MAX_DIMENSION)
        && width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .is_some_and(|bytes| bytes <= RETAINED_LAYER_MAX_BYTES)
}

impl DrawCommand {
    #[inline]
    fn is_paint_command(&self) -> bool {
        matches!(
            self,
            Self::FillRect { .. }
                | Self::ClearRect { .. }
                | Self::DrawText { .. }
                | Self::DrawRichText { .. }
                | Self::DrawTextDecoration { .. }
                | Self::DrawImage { .. }
                | Self::DrawImageWithResource { .. }
                | Self::RetainedLayer { .. }
                | Self::Svg { .. }
                | Self::DrawShadowRect { .. }
                | Self::Custom { .. }
        )
    }

    /// Clones only command payloads that are safe and cheap to replay.
    fn clone_for_retention(&self) -> Option<Self> {
        Some(match self {
            Self::FillRect {
                rect,
                color,
                border_radius,
                border_width,
                border_color,
                outline_width,
                outline_color,
            } => Self::FillRect {
                rect: *rect,
                color: *color,
                border_radius: *border_radius,
                border_width: *border_width,
                border_color: *border_color,
                outline_width: *outline_width,
                outline_color: *outline_color,
            },
            Self::ClearRect { rect } => Self::ClearRect { rect: *rect },
            Self::DrawText {
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
            } => Self::DrawText {
                position: *position,
                text: text.clone(),
                font_size: *font_size,
                color: *color,
                bounds_width: *bounds_width,
                bounds_height: *bounds_height,
                overflow: *overflow,
                horizontal_align: *horizontal_align,
                font_family: *font_family,
                font_style: *font_style,
                font_weight: *font_weight,
                shadow: *shadow,
                draw_glyphs: *draw_glyphs,
            },
            // Rich text owns a span vector. Keeping it out of the retained
            // path avoids an allocation on every scroll replay; the normal
            // draw path still handles it correctly.
            Self::DrawRichText { .. } => return None,
            Self::DrawTextDecoration {
                rect,
                color,
                style,
                thickness,
                period,
            } => Self::DrawTextDecoration {
                rect: *rect,
                color: *color,
                style: *style,
                thickness: *thickness,
                period: *period,
            },
            Self::PushClip {
                rect,
                border_radius,
            } => Self::PushClip {
                rect: *rect,
                border_radius: *border_radius,
            },
            Self::PopClip => Self::PopClip,
            Self::PushTransform { matrix } => Self::PushTransform { matrix: *matrix },
            Self::PopTransform => Self::PopTransform,
            Self::SetAlpha { alpha } => Self::SetAlpha { alpha: *alpha },
            Self::RestoreAlpha => Self::RestoreAlpha,
            Self::SetItalic { italic } => Self::SetItalic { italic: *italic },
            Self::SetTextLanguage { language } => Self::SetTextLanguage {
                language: *language,
            },
            // Upload commands own byte buffers. A static image may still be
            // retained after it has become a stable texture draw, but a
            // loading/upload transition must go through the provider path.
            Self::LoadImage { .. } | Self::LoadImageWithId { .. } => return None,
            Self::RemoveTexture { .. } => return None,
            Self::DrawImage { rect, texture_id } => Self::DrawImage {
                rect: *rect,
                texture_id: *texture_id,
            },
            Self::DrawImageWithResource { rect, resource } => Self::DrawImageWithResource {
                rect: *rect,
                resource: resource.clone(),
            },
            Self::RetainedLayer { .. } => return None,
            Self::Svg {
                scene,
                destination,
                overrides,
            } => Self::Svg {
                scene: scene.clone(),
                destination: *destination,
                overrides: overrides.clone(),
            },
            Self::SetTransform { matrix } => Self::SetTransform { matrix: *matrix },
            Self::DrawShadowRect {
                rect,
                shadow_color,
                shadow_params,
                border_radius,
                inset,
                side_params,
            } => Self::DrawShadowRect {
                rect: *rect,
                shadow_color: *shadow_color,
                shadow_params: *shadow_params,
                border_radius: *border_radius,
                inset: *inset,
                side_params: *side_params,
            },
            Self::Custom { .. } => return None,
        })
    }

    fn rebase_for_replay(&self, base: Mat3) -> Self {
        match self {
            Self::PushTransform { matrix } => Self::PushTransform {
                matrix: base.mul(matrix),
            },
            Self::SetTransform { matrix } => Self::SetTransform {
                matrix: base.mul(matrix),
            },
            _ => self
                .clone_for_retention()
                .expect("retained draw lists contain only commands supported by replay"),
        }
    }
}

/// Counts the broad kinds of work recorded in one frame's draw list.
///
/// The counts are intentionally derived from commands rather than renderer
/// internals: they are available at the UI/raster boundary and make it
/// possible to compare scroll frames without changing rendering semantics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrawListStats {
    /// Total number of commands recorded, including state and clip commands.
    pub commands: usize,
    /// Number of retained compositor layers recorded without expanding their
    /// content into this frame.
    pub retained_layers: usize,
    /// Number of plain, rich-text, and text-decoration commands.
    pub text_commands: usize,
    /// Number of commands that draw an already-loaded texture.
    pub image_draws: usize,
    /// Number of commands that upload image bytes for a texture.
    pub image_uploads: usize,
}

pub struct DrawList {
    commands: Vec<DrawCommand>,
    transform_stack: Vec<Mat3>,
    current_transform: Mat3,
    paint_command_suppression_depth: usize,
    /// Paint commands dropped while suppression was active.
    dropped_paint_commands: std::cell::Cell<u64>,
    texture_sizes: HashMap<TextureId, TextureMetadata>,
    texture_registry: Arc<TextureRegistry>,
    referenced_textures: HashSet<TextureId>,
}

struct TextureMetadata {
    width: u32,
    height: u32,
    registry_serial: u64,
    available: AtomicBool,
}

impl TextureMetadata {
    #[inline]
    fn new(width: u32, height: u32, registry_serial: u64) -> Self {
        Self {
            width,
            height,
            registry_serial,
            available: AtomicBool::new(true),
        }
    }
}

impl DrawList {
    pub fn new() -> Self {
        // debug("Creating draw list");
        Self::with_texture_registry(Arc::new(TextureRegistry::default()))
    }

    pub(crate) fn with_texture_registry(texture_registry: Arc<TextureRegistry>) -> Self {
        Self {
            commands: Vec::with_capacity(16),
            transform_stack: Vec::with_capacity(16),
            current_transform: Mat3::identity(),
            paint_command_suppression_depth: 0,
            dropped_paint_commands: std::cell::Cell::new(0),
            texture_sizes: HashMap::new(),
            texture_registry,
            referenced_textures: HashSet::new(),
        }
    }

    pub fn clear(&mut self) {
        self.release_texture_references();
        self.commands.clear();
        self.transform_stack.clear();
        self.current_transform = Mat3::identity();
    }

    #[inline]
    pub fn push(&mut self, cmd: DrawCommand) {
        if self.paint_command_suppression_depth == 0 || !cmd.is_paint_command() {
            self.commands.push(cmd);
        } else {
            self.note_dropped_paint_command();
        }
    }

    #[inline]
    fn push_paint_command(&mut self, cmd: DrawCommand) {
        if !self.is_suppressing_paint_commands() {
            self.commands.push(cmd);
        }
    }

    pub(crate) fn suppress_paint_commands(&mut self) -> usize {
        let previous_depth = self.paint_command_suppression_depth;
        self.paint_command_suppression_depth = previous_depth.saturating_add(1);
        previous_depth
    }

    pub(crate) fn suspend_paint_command_suppression(&mut self) -> usize {
        let previous_depth = self.paint_command_suppression_depth;
        self.paint_command_suppression_depth = 0;
        previous_depth
    }

    pub(crate) fn restore_paint_command_suppression(&mut self, depth: usize) {
        self.paint_command_suppression_depth = depth;
    }

    #[inline]
    /// Whether a paint command being recorded right now is dropped, counting it
    /// when it is.
    pub(crate) fn is_suppressing_paint_commands(&self) -> bool {
        let suppressing = self.paint_command_suppression_depth != 0;
        if suppressing {
            self.note_dropped_paint_command();
        }
        suppressing
    }

    #[inline]
    fn note_dropped_paint_command(&self) {
        self.dropped_paint_commands
            .set(self.dropped_paint_commands.get().wrapping_add(1));
    }

    /// How many paint commands suppression has dropped so far.
    pub(crate) fn dropped_paint_commands(&self) -> u64 {
        self.dropped_paint_commands.get()
    }

    pub fn fill_rect(
        &mut self,
        rect: Rect,
        color: Color,
        border_radius: [f32; 4],
        border_width: [f32; 4],
        border_color: Color,
    ) {
        self.push_paint_command(DrawCommand::FillRect {
            rect,
            color,
            border_radius,
            border_width,
            border_color,
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        });
    }

    /// Enqueue a draw command for a user-registered custom pipeline.
    /// `pipeline_name` must match `CustomPipelineGeneric::name()` of a registered
    /// pipeline. `data` is forwarded to its `prepare_command()` hook.
    pub fn draw_custom(&mut self, pipeline_name: impl Into<String>, data: impl Any + Send) {
        if self.is_suppressing_paint_commands() {
            return;
        }
        self.commands.push(DrawCommand::Custom {
            pipeline_name: pipeline_name.into(),
            data: Box::new(data),
        });
    }

    pub fn draw_shadow_rect(
        &mut self,
        rect: Rect,
        shadow_color: Color,
        shadow_params: [f32; 4],
        border_radius: [f32; 4],
        inset: bool,
        side_params: [f32; 3],
    ) {
        self.push_paint_command(DrawCommand::DrawShadowRect {
            rect,
            shadow_color,
            shadow_params,
            border_radius,
            inset,
            side_params,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect_with_outline(
        &mut self,
        rect: Rect,
        color: Color,
        border_radius: [f32; 4],
        border_width: [f32; 4],
        border_color: Color,
        outline_width: [f32; 4],
        outline_color: Color,
    ) {
        self.push_paint_command(DrawCommand::FillRect {
            rect,
            color,
            border_radius,
            border_width,
            border_color,
            outline_width,
            outline_color,
        });
    }

    pub fn clear_rect(&mut self, rect: Rect) {
        self.push_paint_command(DrawCommand::ClearRect { rect });
    }

    pub fn draw_image(&mut self, rect: Rect, texture_id: TextureId) {
        if self.is_suppressing_paint_commands() {
            return;
        }
        self.retain_texture_reference(texture_id);
        self.commands
            .push(DrawCommand::DrawImage { rect, texture_id });
    }

    /// Records a draw while retaining the pixels needed to restore its texture.
    pub fn draw_image_with_resource(&mut self, rect: Rect, resource: Arc<ImageResource>) {
        if self.is_suppressing_paint_commands() {
            return;
        }
        self.retain_texture_reference(resource.texture_id());
        self.commands
            .push(DrawCommand::DrawImageWithResource { rect, resource });
    }

    /// Records one compositor layer draw without expanding its retained
    /// content into this frame's command buffer.
    pub fn draw_retained_layer(
        &mut self,
        layer_id: u64,
        rect: Rect,
        content: Arc<RetainedLayerContent>,
    ) {
        if self.is_suppressing_paint_commands() {
            return;
        }
        if !retained_layer_size_is_supported(rect.width, rect.height)
            || !content.is_compositor_safe()
        {
            let base = *self.current_transform();
            content.append_to_draw_list(self, base);
            return;
        }
        for texture_id in content.texture_ids() {
            self.retain_texture_reference(*texture_id);
        }
        self.commands.push(DrawCommand::RetainedLayer {
            layer_id,
            rect,
            content,
        });
    }

    pub fn draw_svg(
        &mut self,
        scene: Arc<SvgScene>,
        destination: Rect,
        overrides: Arc<[SvgNodeStyleOverride]>,
    ) {
        self.push_paint_command(DrawCommand::Svg {
            scene,
            destination,
            overrides,
        });
    }

    pub fn draw_text(
        &mut self,
        position: Vec2d,
        text: Arc<str>,
        font_size: f32,
        color: Color,
        font_weight: u16,
    ) {
        self.draw_text_styled(
            position,
            text,
            font_size,
            color,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            font_weight,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_styled(
        &mut self,
        position: Vec2d,
        text: Arc<str>,
        font_size: f32,
        color: Color,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) {
        self.draw_text_with_overflow(
            position,
            text,
            font_size,
            color,
            None,
            None,
            TextOverflowMode::Clip,
            font_family,
            font_style,
            font_weight,
        );
    }

    /// Records a shadow-only styled text request. The text pipeline expands
    /// the request into clipped glyph samples during preparation.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_shadow_styled(
        &mut self,
        position: Vec2d,
        text: Arc<str>,
        font_size: f32,
        color: Color,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        shadow: TextShadowRequest,
    ) {
        self.push_paint_command(DrawCommand::DrawText {
            position,
            text,
            font_size,
            color,
            bounds_width: None,
            bounds_height: None,
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            font_family,
            font_style,
            font_weight,
            shadow: Some(shadow),
            draw_glyphs: false,
        });
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_with_overflow(
        &mut self,
        position: Vec2d,
        text: Arc<str>,
        font_size: f32,
        color: Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: TextOverflowMode,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) {
        self.draw_text_aligned_with_overflow(
            position,
            text,
            font_size,
            color,
            bounds_width,
            bounds_height,
            overflow,
            TextHorizontalAlign::Left,
            font_family,
            font_style,
            font_weight,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_aligned_with_overflow(
        &mut self,
        position: Vec2d,
        text: Arc<str>,
        font_size: f32,
        color: Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: TextOverflowMode,
        horizontal_align: TextHorizontalAlign,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) {
        self.push_paint_command(DrawCommand::DrawText {
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
            shadow: None,
            draw_glyphs: true,
        });
    }

    pub fn draw_rich_text(
        &mut self,
        position: Vec2d,
        spans: Vec<RichTextSegment>,
        font_size: f32,
        color: Color,
    ) {
        self.draw_rich_text_with_overflow(
            position,
            spans,
            font_size,
            color,
            None,
            None,
            TextOverflowMode::Clip,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_rich_text_with_overflow(
        &mut self,
        position: Vec2d,
        spans: Vec<RichTextSegment>,
        font_size: f32,
        color: Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: TextOverflowMode,
    ) {
        self.push_paint_command(DrawCommand::DrawRichText {
            position,
            spans,
            font_size,
            color,
            bounds_width,
            bounds_height,
            overflow,
        });
    }

    pub fn draw_text_decoration(
        &mut self,
        rect: Rect,
        color: Color,
        style: u32,
        thickness: f32,
        period: f32,
    ) {
        self.push_paint_command(DrawCommand::DrawTextDecoration {
            rect,
            color,
            style,
            thickness,
            period,
        });
    }

    pub fn push_clip(&mut self, rect: Rect) {
        self.commands.push(DrawCommand::PushClip {
            rect,
            border_radius: [0.0; 4],
        });
    }

    pub fn push_clip_rounded(&mut self, rect: Rect, border_radius: [f32; 4]) {
        self.commands.push(DrawCommand::PushClip {
            rect,
            border_radius,
        });
    }

    pub fn pop_clip(&mut self) {
        self.commands.push(DrawCommand::PopClip);
    }

    pub fn set_alpha(&mut self, alpha: f32) {
        self.commands.push(DrawCommand::SetAlpha { alpha });
    }

    pub fn set_italic(&mut self, italic: bool) {
        self.commands.push(DrawCommand::SetItalic { italic });
    }

    /// Records the language subsequent text commands are written in — see
    /// [`DrawCommand::SetTextLanguage`].
    pub fn set_text_language(&mut self, language: Option<TextLanguage>) {
        self.commands
            .push(DrawCommand::SetTextLanguage { language });
    }

    pub fn restore_alpha(&mut self) {
        self.commands.push(DrawCommand::RestoreAlpha);
    }

    /// Returns the content-based texture ID used by [`Self::load_image`].
    #[doc(hidden)]
    pub fn image_texture_id(bytes: &[u8], width: u32, height: u32) -> TextureId {
        // Hash only a small sample of the buffer to avoid O(n) cost on large images.
        let mut hasher = IMAGE_TEXTURE_HASHER
            .get_or_init(DefaultHashBuilder::default)
            .build_hasher();
        width.hash(&mut hasher);
        height.hash(&mut hasher);
        let sample_len = 256.min(bytes.len());
        if sample_len > 0 {
            bytes[..sample_len].hash(&mut hasher);
            bytes[bytes.len() - sample_len..].hash(&mut hasher);
        }
        hasher.finish() as u32
    }

    /// Returns a content revision for retained resources, hashing every pixel.
    #[doc(hidden)]
    pub fn image_content_revision(bytes: &[u8], width: u32, height: u32) -> u64 {
        let mut hasher = IMAGE_TEXTURE_HASHER
            .get_or_init(DefaultHashBuilder::default)
            .build_hasher();
        width.hash(&mut hasher);
        height.hash(&mut hasher);
        bytes.hash(&mut hasher);
        hasher.finish()
    }

    pub fn load_image(&mut self, bytes: &[u8], width: u32, height: u32) -> TextureId {
        let texture_id = Self::image_texture_id(bytes, width, height);
        if self.has_queued_image(texture_id) {
            return texture_id;
        }
        // A content-stable ID can be uploaded again after the renderer evicts
        // its GPU texture. Publish a new serial even when the registry still
        // reports the old dimensions as available, so an older submitted
        // frame cannot invalidate this upload after the fact.
        self.set_texture_size_internal(texture_id, width, height, true);
        // The ID is content-stable, but the upload payload is still a mutable
        // cache value. Retained paint must be retired when an image with the
        // same dimensions is uploaded again.
        self.texture_registry.changed();
        self.commands.push(DrawCommand::LoadImage {
            bytes: bytes.to_vec(),
            texture_id,
            width,
            height,
        });
        texture_id
    }

    pub fn load_image_with_id(
        &mut self,
        texture_id: TextureId,
        bytes: &[u8],
        width: u32,
        height: u32,
    ) {
        // Explicit IDs can carry a new payload without changing dimensions;
        // publish a new serial so an eviction from an older submitted frame
        // cannot invalidate this upload.
        self.set_texture_size_internal(texture_id, width, height, true);
        // Explicit IDs are commonly used for dynamic image sources; changing
        // their bytes must invalidate any retained draw stream that references
        // the ID even when the intrinsic dimensions stay the same.
        self.texture_registry.changed();
        self.commands.push(DrawCommand::LoadImageWithId {
            texture_id,
            bytes: bytes.to_vec(),
            width,
            height,
        });
    }

    pub fn set_texture_size(&mut self, texture_id: TextureId, width: u32, height: u32) {
        self.set_texture_size_internal(texture_id, width, height, false);
    }

    fn set_texture_size_internal(
        &mut self,
        texture_id: TextureId,
        width: u32,
        height: u32,
        force_new_serial: bool,
    ) {
        let registry_needs_update = match self.texture_registry.texture_state(texture_id) {
            Some((registry_width, registry_height, _, available)) => {
                !available || registry_width != width || registry_height != height
            }
            None => true,
        };
        let update_registry = if let Some(metadata) = self.texture_sizes.get_mut(&texture_id) {
            let update_registry = force_new_serial
                || metadata.width != width
                || metadata.height != height
                || !metadata.available.swap(true, Ordering::AcqRel)
                || registry_needs_update;
            metadata.width = width;
            metadata.height = height;
            update_registry
        } else {
            self.texture_sizes
                .insert(texture_id, TextureMetadata::new(width, height, 0));
            true
        };
        if update_registry {
            let registry_serial =
                self.texture_registry
                    .record_size(texture_id, width, height, force_new_serial);
            if let Some(metadata) = self.texture_sizes.get_mut(&texture_id) {
                metadata.registry_serial = registry_serial;
            }
        }
    }

    pub fn remove_texture(&mut self, texture_id: TextureId) {
        if let Some(metadata) = self.texture_sizes.remove(&texture_id) {
            self.texture_registry
                .remove_if_current(texture_id, Some(metadata.registry_serial));
        } else {
            self.texture_registry.remove_if_current(texture_id, None);
        }
        self.commands
            .push(DrawCommand::RemoveTexture { texture_id });
    }

    pub fn save(&mut self) {
        self.transform_stack.push(self.current_transform);
        self.commands.push(DrawCommand::PushTransform {
            matrix: self.current_transform,
        });
    }

    pub fn restore(&mut self) {
        if let Some(prev) = self.transform_stack.pop() {
            self.current_transform = prev;
            self.commands.push(DrawCommand::PopTransform);
        }
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        let t = Mat3::translate(x, y);
        self.current_transform = self.current_transform.mul(&t);
        self.commands.push(DrawCommand::SetTransform {
            matrix: self.current_transform,
        });
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        let s = Mat3::scale(sx, sy);
        self.current_transform = self.current_transform.mul(&s);
        self.commands.push(DrawCommand::SetTransform {
            matrix: self.current_transform,
        });
    }

    pub fn rotate(&mut self, radians: f32) {
        let r = Mat3::rotate(radians);
        self.current_transform = self.current_transform.mul(&r);
        self.commands.push(DrawCommand::SetTransform {
            matrix: self.current_transform,
        });
    }

    pub fn current_transform(&self) -> &Mat3 {
        &self.current_transform
    }

    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Moves the current command stream out while preserving its resource
    /// registry and texture references for mixed v2/legacy frame composition.
    pub(crate) fn take_commands_for_composition(&mut self) -> Vec<DrawCommand> {
        self.transform_stack.clear();
        self.current_transform = Mat3::identity();
        std::mem::take(&mut self.commands)
    }

    /// Reconciles texture retention with the command stream after composition.
    pub(crate) fn sync_composed_texture_references(&mut self) {
        self.sync_composed_texture_references_with_ids(std::iter::empty());
    }

    /// Reconciles texture retention after composition and keeps IDs referenced
    /// by element-local v2 lists alive even though they are not in this legacy
    /// command stream.
    /// The textures this list currently holds a reference to.
    #[cfg(test)]
    pub(crate) fn referenced_texture_ids(&self) -> Vec<TextureId> {
        let mut ids = self.referenced_textures.iter().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub(crate) fn sync_composed_texture_references_with_ids(
        &mut self,
        additional_texture_ids: impl IntoIterator<Item = TextureId>,
    ) {
        let mut needed = HashSet::new();
        for command in &self.commands {
            match command {
                DrawCommand::DrawImage { texture_id, .. } => {
                    needed.insert(*texture_id);
                }
                DrawCommand::DrawImageWithResource { resource, .. } => {
                    needed.insert(resource.texture_id());
                }
                DrawCommand::RetainedLayer { content, .. } => {
                    needed.extend(content.texture_ids().iter().copied());
                }
                _ => {}
            }
        }
        needed.extend(additional_texture_ids);

        let previous = std::mem::take(&mut self.referenced_textures);
        for texture_id in previous {
            if needed.contains(&texture_id) {
                self.referenced_textures.insert(texture_id);
            } else {
                self.texture_registry.release_reference(texture_id);
            }
        }
        for texture_id in needed {
            self.retain_texture_reference(texture_id);
        }
    }

    /// Takes a snapshot when every command in this list can be replayed
    /// without cloning owned byte buffers or custom pipeline state.
    ///
    /// The snapshot is local to the list's coordinate system. Callers should
    /// replay it while the canvas is translated/clipped for the subtree that
    /// produced it.
    pub fn retained_snapshot(&self) -> Option<RetainedDrawList> {
        RetainedDrawList::from_draw_list(self)
    }

    /// Appends a retained local-coordinate stream under the current transform.
    ///
    /// The draw-list transform is intentionally unchanged: a balanced retained
    /// stream is a child of the caller's current state, just like a normal
    /// `save`/`restore` child draw. Texture references are registered on the
    /// destination list so renderer eviction cannot remove a texture used by
    /// this frame's replay.
    pub(crate) fn append_retained(&mut self, retained: &RetainedDrawList, base: Mat3) {
        if self.is_suppressing_paint_commands() {
            return;
        }
        for texture_id in retained.texture_ids() {
            self.retain_texture_reference(*texture_id);
        }
        for command in &retained.commands {
            self.commands.push(command.rebase_for_replay(base));
        }
    }

    /// Summarizes the recorded commands without exposing their payloads.
    ///
    /// This is primarily useful for debug profiling at the end of a frame.
    /// It performs one linear pass over the command stream and does not alter
    /// the list or retain any command data.
    pub fn stats(&self) -> DrawListStats {
        let mut stats = DrawListStats {
            commands: self.commands.len(),
            ..DrawListStats::default()
        };

        for command in &self.commands {
            match command {
                DrawCommand::DrawText { .. }
                | DrawCommand::DrawRichText { .. }
                | DrawCommand::DrawTextDecoration { .. } => stats.text_commands += 1,
                DrawCommand::DrawImage { .. } | DrawCommand::DrawImageWithResource { .. } => {
                    stats.image_draws += 1;
                }
                DrawCommand::RetainedLayer { .. } => stats.retained_layers += 1,
                DrawCommand::LoadImage { .. } | DrawCommand::LoadImageWithId { .. } => {
                    stats.image_uploads += 1;
                }
                _ => {}
            }
        }

        stats
    }

    // pub fn drain_commands(&mut self) -> Vec<DrawCommand> {
    //     std::mem::take(&mut self.commands)
    // }

    pub fn has_texture_id(&self, texture_id: TextureId) -> bool {
        self.commands.iter().any(|cmd| match cmd {
            DrawCommand::DrawImage { texture_id: id, .. } => *id == texture_id,
            DrawCommand::DrawImageWithResource { resource, .. } => {
                resource.texture_id() == texture_id
            }
            DrawCommand::RetainedLayer { content, .. } => {
                content.texture_ids().contains(&texture_id)
            }
            _ => false,
        })
    }

    fn has_queued_image(&self, texture_id: TextureId) -> bool {
        self.commands.iter().any(|cmd| match cmd {
            DrawCommand::LoadImage { texture_id: id, .. } => *id == texture_id,
            _ => false,
        })
    }

    pub fn get_texture_size(&self, texture_id: TextureId) -> Option<(u32, u32)> {
        match self.texture_sizes.get(&texture_id) {
            Some(metadata) => match self.texture_registry.texture_state(texture_id) {
                // The shared registry is authoritative once it has observed
                // this list's metadata (or a newer replacement). A recycled
                // draw list can retain `available = true` locally after the
                // renderer evicts the texture from the submitted list.
                Some((width, height, serial, available)) if serial >= metadata.registry_serial => {
                    available.then_some((width, height))
                }
                Some(_) | None => metadata
                    .available
                    .load(Ordering::Acquire)
                    .then_some((metadata.width, metadata.height)),
            },
            None => self
                .texture_registry
                .texture_state(texture_id)
                .filter(|(_, _, _, available)| *available)
                .map(|(width, height, _, _)| (width, height)),
        }
    }

    /// Returns the generation of renderer-side image-cache changes.
    #[inline]
    pub(crate) fn texture_cache_epoch(&self) -> u64 {
        self.texture_registry.cache_epoch()
    }

    /// Returns whether the texture still has valid retained metadata and has
    /// not been evicted by the renderer.
    #[inline]
    pub(crate) fn is_texture_available(&self, texture_id: TextureId) -> bool {
        match self.texture_sizes.get(&texture_id) {
            Some(metadata) => match self.texture_registry.texture_state(texture_id) {
                Some((_, _, serial, available)) if serial >= metadata.registry_serial => available,
                Some(_) | None => metadata.available.load(Ordering::Acquire),
            },
            None => self
                .texture_registry
                .is_texture_available_or_unknown(texture_id),
        }
    }

    /// Returns the shared registry serial for a texture, if the ID has been
    /// published by a draw list.
    #[inline]
    pub(crate) fn texture_registry_serial(&self, texture_id: TextureId) -> Option<u64> {
        self.texture_registry
            .texture_state(texture_id)
            .map(|(_, _, serial, _)| serial)
    }

    /// Marks a renderer-evicted texture without taking ownership of the draw
    /// list's command buffer. The serial is captured when the renderer selects
    /// the eviction, so a newer upload published by another in-flight frame
    /// cannot be invalidated by an older removal. This also works when the
    /// current draw list has no local metadata for the evicted ID.
    #[inline]
    pub(crate) fn mark_texture_evicted(&self, texture_id: TextureId, serial: u64) {
        if let Some(metadata) = self.texture_sizes.get(&texture_id)
            && metadata.registry_serial == serial
        {
            metadata.available.store(false, Ordering::Release);
        }
        self.texture_registry
            .mark_evicted_if_current(texture_id, serial);
    }

    #[inline]
    pub(crate) fn has_live_texture_reference(&self, texture_id: TextureId) -> bool {
        self.texture_registry.is_referenced(texture_id)
    }

    fn release_texture_references(&mut self) {
        for texture_id in self.referenced_textures.drain() {
            self.texture_registry.release_reference(texture_id);
        }
    }

    /// Carries the shared registry's latest metadata into a destination draw
    /// list as a retained stream is attached to it. Without this hand-off an
    /// image could be evicted while its layer is not visible, yet the next
    /// source-backed widget would have no local metadata to observe that the
    /// texture became unavailable and reload it.
    fn retain_texture_reference(&mut self, texture_id: TextureId) {
        if self.referenced_textures.insert(texture_id) {
            self.texture_registry.retain_reference(texture_id);
        }

        let Some((width, height, registry_serial, available)) =
            self.texture_registry.texture_state(texture_id)
        else {
            return;
        };
        match self.texture_sizes.get_mut(&texture_id) {
            Some(metadata) if registry_serial > metadata.registry_serial => {
                metadata.width = width;
                metadata.height = height;
                metadata.registry_serial = registry_serial;
                metadata.available.store(available, Ordering::Release);
            }
            Some(_) => {}
            None => {
                self.texture_sizes.insert(
                    texture_id,
                    TextureMetadata {
                        width,
                        height,
                        registry_serial,
                        available: AtomicBool::new(available),
                    },
                );
            }
        }
    }
}

impl Drop for DrawList {
    fn drop(&mut self) {
        self.release_texture_references();
    }
}

impl Default for DrawList {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod memory_tests {
    use std::hint::black_box;
    use aimer_time::Instant;

    use super::*;
    use crate::font::{FontFamily, FontStyle};
    use crate::svg::{SvgScene, SvgViewport};

    /// A finished frame is handed to the raster thread by value, so the whole
    /// draw list — every command payload included — has to be `Send`.
    #[test]
    fn draw_list_can_cross_a_thread_boundary() {
        fn assert_send<T: Send>() {}
        assert_send::<DrawCommand>();
        assert_send::<DrawList>();

        let mut list = DrawList::new();
        list.fill_rect(
            Rect::new(0.0, 0.0, 2.0, 2.0),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );

        let moved = std::thread::spawn(move || list.commands().len())
            .join()
            .expect("raster worker panicked");

        assert_eq!(moved, 1);
    }

    #[test]
    fn draw_list_stats_classify_recorded_work() {
        let mut list = DrawList::new();

        list.draw_text(
            Vec2d::new(0.0, 0.0),
            Arc::from("stats"),
            16.0,
            Color::black(),
            400,
        );
        list.load_image(&[1, 2, 3, 4], 1, 1);
        list.draw_image(Rect::new(0.0, 0.0, 1.0, 1.0), 7);

        assert_eq!(
            list.stats(),
            DrawListStats {
                commands: 3,
                retained_layers: 0,
                text_commands: 1,
                image_draws: 1,
                image_uploads: 1,
            }
        );
    }

    #[test]
    fn removing_a_texture_clears_metadata_and_records_gpu_release() {
        let mut list = DrawList::new();
        list.set_texture_size(42, 640, 480);

        list.remove_texture(42);

        assert_eq!(list.get_texture_size(42), None);
        assert!(matches!(
            list.commands().last(),
            Some(DrawCommand::RemoveTexture { texture_id: 42 })
        ));
    }

    #[test]
    fn svg_commands_remain_interleaved_with_other_primitives() {
        let scene = Arc::new(SvgScene {
            viewport: SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            nodes: Arc::from([]),
            geometries: Arc::from([]),
            resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
        });
        let mut list = DrawList::new();
        list.fill_rect(
            Rect::new(0.0, 0.0, 2.0, 2.0),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        list.draw_svg(scene, Rect::new(2.0, 2.0, 10.0, 10.0), Arc::from([]));
        list.draw_image(Rect::new(12.0, 12.0, 2.0, 2.0), 7);

        assert!(matches!(list.commands()[0], DrawCommand::FillRect { .. }));
        assert!(matches!(list.commands()[1], DrawCommand::Svg { .. }));
        assert!(matches!(list.commands()[2], DrawCommand::DrawImage { .. }));
    }

    #[test]
    fn styled_text_command_retains_face_selection() {
        let mut list = DrawList::new();
        list.draw_text_styled(
            Vec2d::new(1.0, 2.0),
            Arc::from("code"),
            16.0,
            Color::black(),
            FontFamily::MONOSPACE,
            FontStyle::Italic,
            700,
        );

        assert!(matches!(
            list.commands().last(),
            Some(DrawCommand::DrawText {
                font_family: FontFamily::MONOSPACE,
                font_style: FontStyle::Italic,
                font_weight: 700,
                ..
            })
        ));
    }

    #[test]
    fn shadow_text_is_recorded_before_the_foreground_request() {
        let mut list = DrawList::new();
        list.draw_text_shadow_styled(
            Vec2d::new(1.0, 2.0),
            Arc::from("shadow"),
            16.0,
            Color::black(),
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            400,
            TextShadowRequest {
                offset_x: 2.0,
                offset_y: 1.0,
                blur: 3.0,
                color: Rgba8::new(0, 0, 0, 128),
            },
        );
        list.draw_text_styled(
            Vec2d::new(1.0, 2.0),
            Arc::from("shadow"),
            16.0,
            Color::white(),
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            400,
        );

        assert!(matches!(
            list.commands().first(),
            Some(DrawCommand::DrawText {
                shadow: Some(_),
                draw_glyphs: false,
                ..
            })
        ));
        assert!(matches!(
            list.commands().get(1),
            Some(DrawCommand::DrawText {
                shadow: None,
                draw_glyphs: true,
                ..
            })
        ));
    }

    #[test]
    fn duplicate_hashed_image_loads_share_one_queued_command() {
        let data = [1, 2, 3, 4];
        let mut list = DrawList::new();

        let first = list.load_image(&data, 1, 1);
        let first_epoch = list.texture_cache_epoch();
        let second = list.load_image(&data, 1, 1);

        assert_eq!(first, second);
        assert_eq!(list.texture_cache_epoch(), first_epoch);
        assert_eq!(
            list.commands()
                .iter()
                .filter(|command| matches!(command, DrawCommand::LoadImage { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn explicit_id_image_loads_are_not_deduplicated() {
        let data = [1, 2, 3, 4];
        let mut list = DrawList::new();

        list.load_image_with_id(7, &data, 1, 1);
        list.load_image_with_id(7, &data, 1, 1);

        assert_eq!(
            list.commands()
                .iter()
                .filter(|command| matches!(command, DrawCommand::LoadImageWithId { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn replacing_same_sized_image_payload_advances_the_texture_epoch() {
        let mut list = DrawList::new();
        list.load_image_with_id(7, &[1, 2, 3, 4], 1, 1);
        let first_upload = list.texture_cache_epoch();

        list.load_image_with_id(7, &[4, 3, 2, 1], 1, 1);

        assert!(list.texture_cache_epoch() > first_upload);
    }

    #[test]
    fn evicted_texture_metadata_can_be_reactivated_by_a_reload() {
        let mut list = DrawList::new();
        list.set_texture_size(42, 640, 480);
        let initial_epoch = list.texture_cache_epoch();
        let serial = list
            .texture_registry_serial(42)
            .expect("the texture must be registered");

        list.mark_texture_evicted(42, serial);

        assert!(!list.is_texture_available(42));
        assert_eq!(list.get_texture_size(42), None);
        assert!(list.texture_cache_epoch() > initial_epoch);

        list.set_texture_size(42, 640, 480);

        assert!(list.is_texture_available(42));
        assert_eq!(list.get_texture_size(42), Some((640, 480)));
        assert!(list.texture_cache_epoch() > initial_epoch);
    }

    #[test]
    fn a_replacement_draw_list_sees_shared_texture_metadata() {
        let registry = Arc::new(TextureRegistry::default());
        let mut submitted = DrawList::with_texture_registry(registry.clone());
        let replacement = DrawList::with_texture_registry(registry);

        submitted.set_texture_size(7, 320, 200);
        let serial = submitted
            .texture_registry_serial(7)
            .expect("the texture must be registered");

        assert!(replacement.is_texture_available(7));
        assert_eq!(replacement.get_texture_size(7), Some((320, 200)));

        submitted.mark_texture_evicted(7, serial);

        assert!(!replacement.is_texture_available(7));
        assert_eq!(replacement.get_texture_size(7), None);
    }

    #[test]
    fn an_eviction_from_a_draw_list_without_local_metadata_updates_the_registry() {
        let registry = Arc::new(TextureRegistry::default());
        let mut uploaded = DrawList::with_texture_registry(registry.clone());
        let observer = DrawList::with_texture_registry(registry);

        uploaded.set_texture_size(7, 320, 200);
        let serial = uploaded
            .texture_registry_serial(7)
            .expect("the texture must be registered");

        observer.mark_texture_evicted(7, serial);

        assert!(!uploaded.is_texture_available(7));
        assert_eq!(uploaded.get_texture_size(7), None);
        assert!(!observer.is_texture_available(7));
        assert_eq!(observer.get_texture_size(7), None);
    }

    #[test]
    fn a_reused_draw_list_does_not_treat_evicted_texture_metadata_as_available() {
        let registry = Arc::new(TextureRegistry::default());
        let mut submitted = DrawList::with_texture_registry(registry.clone());
        let mut replacement = DrawList::with_texture_registry(registry.clone());

        submitted.set_texture_size(7, 320, 200);
        let (_, _, serial, _) = registry
            .texture_state(7)
            .expect("the uploaded texture must be registered");
        replacement
            .texture_sizes
            .insert(7, TextureMetadata::new(320, 200, serial));

        submitted.mark_texture_evicted(7, serial);

        assert!(!replacement.is_texture_available(7));
        assert_eq!(replacement.get_texture_size(7), None);
    }

    #[test]
    fn re_registering_an_available_texture_does_not_hide_an_in_flight_eviction() {
        let registry = Arc::new(TextureRegistry::default());
        let mut submitted = DrawList::with_texture_registry(registry.clone());
        let mut replacement = DrawList::with_texture_registry(registry);

        submitted.set_texture_size(7, 320, 200);
        replacement.set_texture_size(7, 320, 200);
        let serial = submitted
            .texture_registry_serial(7)
            .expect("the texture must be registered");

        submitted.mark_texture_evicted(7, serial);

        assert!(!replacement.is_texture_available(7));
        assert_eq!(replacement.get_texture_size(7), None);

        replacement.set_texture_size(7, 320, 200);

        assert!(replacement.is_texture_available(7));
        assert_eq!(replacement.get_texture_size(7), Some((320, 200)));
    }

    #[test]
    fn an_older_eviction_cannot_hide_a_newer_reload() {
        let registry = Arc::new(TextureRegistry::default());
        let mut submitted = DrawList::with_texture_registry(registry.clone());
        let mut replacement = DrawList::with_texture_registry(registry);

        submitted.set_texture_size(7, 320, 200);
        let serial = submitted
            .texture_registry_serial(7)
            .expect("the texture must be registered");
        replacement.load_image_with_id(7, &[1, 2, 3, 4], 320, 200);
        submitted.mark_texture_evicted(7, serial);

        assert!(replacement.is_texture_available(7));
        assert_eq!(replacement.get_texture_size(7), Some((320, 200)));
        // The submitted list may be recycled after the newer frame has
        // already published its replacement metadata.
        assert!(submitted.is_texture_available(7));
        assert_eq!(submitted.get_texture_size(7), Some((320, 200)));
    }

    #[test]
    fn an_older_eviction_cannot_hide_a_newer_hashed_reload() {
        let registry = Arc::new(TextureRegistry::default());
        let data = [1, 2, 3, 4];
        let mut submitted = DrawList::with_texture_registry(registry.clone());
        let mut replacement = DrawList::with_texture_registry(registry);

        let texture_id = submitted.load_image(&data, 320, 200);
        let serial = submitted
            .texture_registry_serial(texture_id)
            .expect("the texture must be registered");
        assert_eq!(replacement.load_image(&data, 320, 200), texture_id);

        submitted.mark_texture_evicted(texture_id, serial);

        assert!(replacement.is_texture_available(texture_id));
        assert_eq!(replacement.get_texture_size(texture_id), Some((320, 200)));
        // The submitted list may be recycled after the newer frame has
        // already published its replacement metadata.
        assert!(submitted.is_texture_available(texture_id));
        assert_eq!(submitted.get_texture_size(texture_id), Some((320, 200)));
    }

    #[test]
    fn in_flight_draw_lists_protect_shared_image_references() {
        let registry = Arc::new(TextureRegistry::default());
        let mut submitted = DrawList::with_texture_registry(registry.clone());
        let mut replacement = DrawList::with_texture_registry(registry);

        submitted.draw_image(Rect::new(0.0, 0.0, 1.0, 1.0), 7);
        assert!(replacement.has_live_texture_reference(7));

        replacement.draw_image(Rect::new(0.0, 0.0, 1.0, 1.0), 7);
        submitted.clear();
        assert!(replacement.has_live_texture_reference(7));

        replacement.clear();
        assert!(!replacement.has_live_texture_reference(7));
    }

    #[test]
    fn retained_commands_rebase_local_transforms_without_copying_bulk_payloads() {
        let mut recorded = DrawList::new();
        recorded.save();
        recorded.translate(3.0, 4.0);
        recorded.fill_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::white(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        recorded.restore();
        let retained = recorded
            .retained_snapshot()
            .expect("basic paint commands should be retainable");

        let mut replayed = DrawList::new();
        replayed.translate(10.0, 20.0);
        let base = *replayed.current_transform();
        replayed.append_retained(&retained, base);

        assert_eq!(retained.len(), 4);
        assert!(matches!(
            replayed.commands().get(1),
            Some(DrawCommand::PushTransform { matrix })
                if matrix.cols[2][0] == 10.0 && matrix.cols[2][1] == 20.0
        ));
        assert!(matches!(
            replayed.commands().get(2),
            Some(DrawCommand::SetTransform { matrix })
                if matrix.cols[2][0] == 13.0 && matrix.cols[2][1] == 24.0
        ));
    }

    #[test]
    fn retained_snapshot_rejects_commands_that_would_copy_or_rebuild_bulk_state() {
        let mut list = DrawList::new();
        list.draw_rich_text(
            Vec2d::default(),
            vec![RichTextSegment::new("rich")],
            16.0,
            Color::black(),
        );
        assert!(list.retained_snapshot().is_none());

        let mut upload = DrawList::new();
        upload.load_image(&[1, 2, 3, 4], 1, 1);
        assert!(upload.retained_snapshot().is_none());
    }

    #[test]
    fn retained_layer_is_recorded_as_one_command() {
        let mut recorded = DrawList::new();
        recorded.fill_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::white(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let content = Arc::new(RetainedLayerContent::from_snapshot(
            recorded
                .retained_snapshot()
                .expect("a plain rectangle can be retained"),
        ));

        let mut frame = DrawList::new();
        frame.draw_retained_layer(7, Rect::new(0.0, 0.0, 10.0, 10.0), content);

        assert_eq!(frame.commands().len(), 1);
        assert!(matches!(
            frame.commands().first(),
            Some(DrawCommand::RetainedLayer { layer_id: 7, .. })
        ));
    }

    #[test]
    fn padded_text_shadow_is_safe_and_translated_only_when_rasterized() {
        let mut recorded = DrawList::new();
        recorded.push(DrawCommand::DrawText {
            position: Vec2d::new(0.0, 16.0),
            text: Arc::from("shadow"),
            font_size: 16.0,
            color: Color::white(),
            bounds_width: Some(100.0),
            bounds_height: Some(24.0),
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: 400,
            shadow: Some(TextShadowRequest {
                offset_x: 2.0,
                offset_y: -3.0,
                blur: 4.0,
                color: Rgba8::new(0, 0, 0, 255),
            }),
            draw_glyphs: true,
        });
        let content = RetainedLayerContent::from_snapshot_with_padding(
            recorded
                .retained_snapshot()
                .expect("text commands should be retainable"),
            RetainedLayerPadding::new(2.0, 7.0, 6.0, 1.0),
        );

        assert!(content.is_compositor_safe());
        assert_eq!(
            content.padding(),
            RetainedLayerPadding::new(2.0, 7.0, 6.0, 1.0)
        );

        let layer_draw_list = content.to_draw_list();
        assert!(matches!(
            layer_draw_list.commands().first(),
            Some(DrawCommand::SetTransform { matrix })
                if matrix.cols[2][0] == 2.0 && matrix.cols[2][1] == 7.0
        ));
        assert!(matches!(
            layer_draw_list.commands().get(1),
            Some(DrawCommand::DrawText { position, .. })
                if position.x == 0.0 && position.y == 16.0
        ));
    }

    #[test]
    fn insufficient_text_shadow_padding_uses_direct_fallback() {
        let mut recorded = DrawList::new();
        recorded.push(DrawCommand::DrawText {
            position: Vec2d::new(0.0, 16.0),
            text: Arc::from("shadow"),
            font_size: 16.0,
            color: Color::white(),
            bounds_width: Some(100.0),
            bounds_height: Some(24.0),
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: 400,
            shadow: Some(TextShadowRequest {
                offset_x: 0.0,
                offset_y: 0.0,
                blur: 4.0,
                color: Rgba8::new(0, 0, 0, 255),
            }),
            draw_glyphs: true,
        });
        let content = Arc::new(RetainedLayerContent::from_snapshot_with_padding(
            recorded
                .retained_snapshot()
                .expect("text commands should be retainable"),
            RetainedLayerPadding::new(3.0, 4.0, 4.0, 4.0),
        ));
        assert!(!content.is_compositor_safe());

        let mut frame = DrawList::new();
        frame.draw_retained_layer(10, Rect::new(0.0, 0.0, 100.0, 24.0), content);

        assert_eq!(frame.stats().retained_layers, 0);
        assert!(matches!(
            frame.commands().first(),
            Some(DrawCommand::DrawText { position, .. })
                if position.x == 0.0 && position.y == 16.0
        ));
    }

    #[test]
    fn retained_layer_falls_back_for_unsafe_effects_and_oversized_bounds() {
        let mut recorded = DrawList::new();
        recorded.draw_shadow_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::black(),
            [1.0, 1.0, 2.0, 0.0],
            [0.0; 4],
            false,
            [0.0; 3],
        );
        let unsafe_content = Arc::new(RetainedLayerContent::from_snapshot(
            recorded
                .retained_snapshot()
                .expect("shadow commands remain valid direct-path snapshots"),
        ));
        assert!(!unsafe_content.is_compositor_safe());

        let mut unsafe_frame = DrawList::new();
        unsafe_frame.draw_retained_layer(8, Rect::new(0.0, 0.0, 10.0, 10.0), unsafe_content);
        assert_eq!(unsafe_frame.stats().retained_layers, 0);
        assert!(matches!(
            unsafe_frame.commands().first(),
            Some(DrawCommand::DrawShadowRect { .. })
        ));

        let mut plain = DrawList::new();
        plain.fill_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::white(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let plain_content = Arc::new(RetainedLayerContent::from_snapshot(
            plain
                .retained_snapshot()
                .expect("plain commands should be retainable"),
        ));
        let mut oversized_frame = DrawList::new();
        oversized_frame.draw_retained_layer(
            9,
            Rect::new(0.0, 0.0, RETAINED_LAYER_MAX_DIMENSION as f32 + 1.0, 10.0),
            plain_content,
        );
        assert_eq!(oversized_frame.stats().retained_layers, 0);
        assert!(matches!(
            oversized_frame.commands().first(),
            Some(DrawCommand::FillRect { .. })
        ));
    }

    fn patterned_bytes(len: usize) -> Vec<u8> {
        (0..len)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(7))
            .collect()
    }

    fn retained_image_bytes(list: &DrawList) -> usize {
        match list.commands().last() {
            Some(DrawCommand::LoadImage { bytes, .. })
            | Some(DrawCommand::LoadImageWithId { bytes, .. }) => {
                let bytes = black_box(bytes.as_slice());
                bytes.len()
                    + bytes.first().copied().unwrap_or_default() as usize
                    + bytes.last().copied().unwrap_or_default() as usize
            }
            _ => panic!("image load did not append an image command"),
        }
    }

    #[test]
    #[ignore = "manual bulk-data profile"]
    fn profile_image_command_copy() {
        const ROUNDS: usize = 7;

        let cases = [
            ("4kb", 4_096, 32, 32, 256),
            ("256kb", 262_144, 256, 256, 16),
            ("4mb", 4_194_304, 1_024, 1_024, 2),
        ];
        let mut checksum = 0u64;

        for (name, byte_count, width, height, measured) in cases {
            let data = patterned_bytes(byte_count);
            for with_id in [false, true] {
                let mut samples = Vec::with_capacity(ROUNDS);
                for _ in 0..ROUNDS {
                    for _ in 0..2 {
                        let mut list = DrawList::new();
                        if with_id {
                            list.load_image_with_id(17, &data, width, height);
                        } else {
                            list.load_image(&data, width, height);
                        }
                        checksum = checksum.wrapping_add(retained_image_bytes(&list) as u64);
                    }

                    let start = Instant::now();
                    for _ in 0..measured {
                        let mut list = DrawList::new();
                        if with_id {
                            list.load_image_with_id(17, &data, width, height);
                        } else {
                            list.load_image(&data, width, height);
                        }
                        checksum = checksum.wrapping_add(retained_image_bytes(&list) as u64);
                    }
                    samples.push(start.elapsed().as_secs_f64() * 1e6 / measured as f64);
                }

                samples.sort_by(f64::total_cmp);
                let p50 = samples[ROUNDS / 2];
                let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
                let method = if with_id { "with-id" } else { "hashed" };
                println!("{name} {method}: p50 {p50:.3} us, p95 {p95:.3} us");
            }
        }

        let data = patterned_bytes(4_194_304);
        let mut samples = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            for _ in 0..2 {
                let mut list = DrawList::new();
                list.load_image(&data, 1_024, 1_024);
                list.load_image(&data, 1_024, 1_024);
                checksum = checksum
                    .wrapping_add(retained_image_bytes(&list) as u64)
                    .wrapping_add(black_box(list.commands().len()) as u64);
            }

            let start = Instant::now();
            for _ in 0..2 {
                let mut list = DrawList::new();
                list.load_image(&data, 1_024, 1_024);
                list.load_image(&data, 1_024, 1_024);
                checksum = checksum
                    .wrapping_add(retained_image_bytes(&list) as u64)
                    .wrapping_add(black_box(list.commands().len()) as u64);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / 2.0);
        }

        samples.sort_by(f64::total_cmp);
        let p50 = samples[ROUNDS / 2];
        let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
        println!("4mb duplicate hashed: p50 {p50:.3} us, p95 {p95:.3} us");

        assert_ne!(checksum, 0);
    }
}
