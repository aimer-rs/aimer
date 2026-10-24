use std::sync::Arc;

mod generic_renderer;
pub use generic_renderer::{Renderer, RendererImpl, RendererMemoryStats};

#[cfg(all(
    test,
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod unpresented_tests;

#[cfg(all(
    test,
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod prewarm_tests;

#[cfg(test)]
mod gradient_scale_tests;

#[cfg(all(
    test,
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod mask_group_tests;

#[cfg(all(
    test,
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod fade_group_tests;

#[cfg(all(
    test,
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod opacity_group_backdrop_tests;

#[cfg(all(
    test,
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod morph_group_tests;

use crate::pipeline::material::{MaterialClip, MaterialRequest};
use crate::rect_pipeline::RectInstance;
use crate::svg::{SvgNodeStyleOverride, SvgScene};
use crate::text_pipeline::TextShadowRequest;
use crate::utilities::Rgba8;
use crate::utilities::{Color, Mat3, Rect};

struct ClipState {
    rect: Rect,
    border_radius: [f32; 4],
}

fn clip_to_array(clip: Option<&ClipState>) -> [f32; 4] {
    clip.map(|c| [c.rect.x, c.rect.y, c.rect.width, c.rect.height])
        .unwrap_or([0.0, 0.0, -1.0, 0.0])
}

fn clip_border_radius(clip: Option<&ClipState>) -> [f32; 4] {
    clip.map(|c| c.border_radius).unwrap_or([0.0; 4])
}

struct AlphaState {
    current: f32,
    stack: Vec<f32>,
}

impl AlphaState {
    fn new() -> Self {
        Self {
            current: 1.0,
            stack: Vec::new(),
        }
    }

    fn current(&self) -> f32 {
        self.current
    }

    fn set(&mut self, alpha: f32) {
        self.current = alpha.clamp(0.0, 1.0);
    }

    fn save(&mut self) {
        self.stack.push(self.current);
    }

    fn restore(&mut self) {
        self.current = self.stack.pop().unwrap_or(1.0);
    }
}

impl Default for AlphaState {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn apply_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

#[inline]
fn packed_color(color: Color, alpha: f32) -> Rgba8 {
    Rgba8::from(color).with_opacity(alpha)
}

fn resolve_material_request(
    mut request: MaterialRequest,
    transform: &Mat3,
    scales: (f32, f32),
    clip: Option<&ClipState>,
    alpha: f32,
) -> MaterialRequest {
    let [x, y, width, height] = request.bounds;
    let (x1, y1) = transform.transform_point(x, y);
    let (x2, y2) = transform.transform_point(x + width, y + height);
    let scale = (scales.0 + scales.1) * 0.5;

    request.bounds = [x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs()];
    // LightEdge keeps dimensions logical until uniform preparation, so bridge
    // normalization cannot cap Retina-scaled widths or radii back to logical limits.
    if request.kind != crate::pipeline::material::MaterialKind::LightEdge {
        for radius in &mut request.corner_radii {
            *radius *= scale;
        }
        request.border_width *= scale;
    }
    if request.kind == crate::pipeline::material::MaterialKind::Gradient {
        // Ramp geometry is authored in logical pixels from the bounds origin.
        // A conic ramp keeps its start angle; every other slot is a length.
        let lengths = if request.gradient.shape == crate::pipeline::material::GradientShape::Conic
        {
            2
        } else {
            4
        };
        for value in &mut request.gradient.geometry[..lengths] {
            *value *= scale;
        }
    }
    request.shadow_blur *= scale;
    request.elevation *= scale;
    request.opacity = (request.opacity * alpha).clamp(0.0, 1.0);
    request.transform = [
        transform.cols[0][0],
        transform.cols[0][1],
        transform.cols[0][2],
        transform.cols[1][0],
        transform.cols[1][1],
        transform.cols[1][2],
        transform.cols[2][0],
        transform.cols[2][1],
        transform.cols[2][2],
    ];
    request.clip = clip.map(|clip| MaterialClip {
        rect: [clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height],
        corner_radii: clip.border_radius,
    });
    request.normalized()
}

#[inline]
fn transform_scales(transform: &Mat3) -> (f32, f32) {
    (
        (transform.cols[0][0].powi(2) + transform.cols[0][1].powi(2)).sqrt(),
        (transform.cols[1][0].powi(2) + transform.cols[1][1].powi(2)).sqrt(),
    )
}

fn transform_text_shadow(
    shadow: TextShadowRequest,
    transform: &Mat3,
    alpha: f32,
) -> TextShadowRequest {
    let offset_x = transform.cols[0][0] * shadow.offset_x + transform.cols[1][0] * shadow.offset_y;
    let offset_y = transform.cols[0][1] * shadow.offset_x + transform.cols[1][1] * shadow.offset_y;
    TextShadowRequest {
        offset_x,
        offset_y,
        color: shadow.color.with_opacity(alpha),
        ..shadow
    }
}

/// Builds the GPU rect instance(s) for a single `FillRect` command.
///
/// Returns the box's own instance (background + border, its outline fields
/// always zeroed) and, when the command carries a visible outline, a second
/// instance covering only the outline ring at its expanded bounds (its fill
/// and border fields always zeroed, so it paints nothing but the ring).
///
/// An outline is drawn just outside its box's own edge, so it can overlap
/// whatever is painted right after it — an adjacent sibling in a `Row`, say.
/// Splitting it into its own instance lets the caller defer it: see
/// [`Renderer::deferred_outlines`].
#[allow(clippy::too_many_arguments)]
fn build_fill_rect_instances(
    rect: Rect,
    color: Color,
    border_radius: [f32; 4],
    border_width: [f32; 4],
    border_color: Color,
    outline_width: [f32; 4],
    outline_color: Color,
    current_transform: &Mat3,
    scale_x: f32,
    scale_y: f32,
    clip: Option<&ClipState>,
    alpha: f32,
) -> (RectInstance, Option<RectInstance>) {
    let ol = outline_width[3]; // left
    let or = outline_width[1]; // right
    let ot = outline_width[0]; // top
    let ob = outline_width[2]; // bottom
    let has_outline = ol > 0.0 || or > 0.0 || ot > 0.0 || ob > 0.0;

    // Transform the top-left and bottom-right corners of the box itself. This
    // correctly handles translation and scaling.
    let (p1x, p1y) = current_transform.transform_point(rect.x, rect.y);
    let (p2x, p2y) = current_transform.transform_point(rect.x + rect.width, rect.y + rect.height);

    let mut scaled_br = border_radius;
    for r in &mut scaled_br {
        *r *= scale_x;
    } // Assuming uniform scale for simplicity, or use scale_x

    let mut scaled_bw = border_width;
    scaled_bw[0] *= scale_y; // top
    scaled_bw[1] *= scale_x; // right
    scaled_bw[2] *= scale_y; // bottom
    scaled_bw[3] *= scale_x; // left

    let clip_rect = clip_to_array(clip);
    let clip_radii = clip_border_radius(clip);

    let outline_instance = has_outline.then(|| {
        // Transform the top-left and bottom-right corners of the quad expanded
        // by the outline width, so the ring is visible.
        let (ep1x, ep1y) = current_transform.transform_point(rect.x - ol, rect.y - ot);
        let (ep2x, ep2y) =
            current_transform.transform_point(rect.x + rect.width + or, rect.y + rect.height + ob);

        let mut scaled_ow = outline_width;
        scaled_ow[0] *= scale_y; // top
        scaled_ow[1] *= scale_x; // right
        scaled_ow[2] *= scale_y; // bottom
        scaled_ow[3] *= scale_x; // left

        RectInstance {
            position: [ep1x.min(ep2x), ep1y.min(ep2y)],
            size: [(ep2x - ep1x).abs(), (ep2y - ep1y).abs()],
            color: Rgba8::TRANSPARENT,
            border_radius: scaled_br,
            border_width: [0.0; 4],
            border_color: Rgba8::TRANSPARENT,
            outline_width: scaled_ow,
            outline_color: packed_color(outline_color, alpha),
            clip_rect,
            clip_border_radius: clip_radii,
            shadow_params: [0.0; 4],
            shadow_color: Rgba8::TRANSPARENT,
            shadow_flags: [0.0; 4],
        }
    });

    let main_instance = RectInstance {
        position: [p1x.min(p2x), p1y.min(p2y)],
        size: [(p2x - p1x).abs(), (p2y - p1y).abs()],
        color: packed_color(color, alpha),
        border_radius: scaled_br,
        border_width: scaled_bw,
        border_color: packed_color(border_color, alpha),
        outline_width: [0.0; 4],
        outline_color: Rgba8::TRANSPARENT,
        clip_rect,
        clip_border_radius: clip_radii,
        shadow_params: [0.0; 4],
        shadow_color: Rgba8::TRANSPARENT,
        shadow_flags: [0.0; 4],
    };

    (main_instance, outline_instance)
}

fn retained_layer_dimensions(rect: Rect, max_dimension: u32) -> Option<(u32, u32)> {
    if !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
        || max_dimension == 0
    {
        return None;
    }

    let width = rect.width.ceil().max(1.0) as u64;
    let height = rect.height.ceil().max(1.0) as u64;
    let bytes = width.checked_mul(height)?.checked_mul(4)?;
    let max_dimension = max_dimension.min(crate::draw_cmd::RETAINED_LAYER_MAX_DIMENSION);
    (width <= u64::from(max_dimension)
        && height <= u64::from(max_dimension)
        && bytes <= crate::draw_cmd::RETAINED_LAYER_MAX_BYTES)
        .then_some((width as u32, height as u32))
}

pub struct SvgRenderItem {
    pub scene: Arc<SvgScene>,
    pub destination: Rect,
    pub overrides: Arc<[SvgNodeStyleOverride]>,
    pub world_transform: Mat3,
    pub clip_rect: [f32; 4],
    pub clip_border_radius: [f32; 4],
    pub opacity: f32,
}

fn resolve_svg_item(
    scene: Arc<SvgScene>,
    destination: Rect,
    overrides: Arc<[SvgNodeStyleOverride]>,
    world_transform: Mat3,
    clip: Option<&ClipState>,
    opacity: f32,
) -> SvgRenderItem {
    SvgRenderItem {
        scene,
        destination,
        overrides,
        world_transform,
        clip_rect: clip_to_array(clip),
        clip_border_radius: clip_border_radius(clip),
        opacity,
    }
}
