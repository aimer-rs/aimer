//! Cupid's adapter for the renderer-neutral [`aimer_shape`] model.
//!
//! Shapes use the existing SVG geometry and mesh-cache machinery. This keeps
//! one tessellation implementation for paths, curves, fills, and strokes;
//! the public geometry crate never carries a GPU handle or a `wgpu` type.

use std::fmt;
use std::sync::Arc;

use aimer_shape::{
    FillRule, FillStyle, LineCap, LineJoin, PaintError, ShapeClip, ShapeHitTest, ShapePath,
    ShapeSize, ShapeTransform, StrokeStyle,
};

use crate::svg::{
    SvgColor, SvgElementKind, SvgFill, SvgGeometry, SvgGeometryCache, SvgLineCap, SvgLineJoin,
    SvgMesh, SvgMeshStyle, SvgNode, SvgNodeId, SvgPaintOrder, SvgScene, SvgStroke,
    SvgTessellationError, SvgTransform, SvgViewport,
};

/// Renderer-side bounds used by this adapter before GPU submission.
pub const DEFAULT_MAX_SHAPE_SCENE_COORDINATE: f32 = 1_000_000.0;

/// A typed, renderer-facing shape input. It contains only borrowed geometry and
/// plain paint values; a renderer may cache the resulting scene or mesh.
#[derive(Clone, Debug)]
pub struct ShapeRenderRequest<'a> {
    /// Validated local path.
    pub path: &'a ShapePath,
    /// Local-to-viewport affine transform.
    pub transform: ShapeTransform,
    /// Optional fill.
    pub fill: Option<FillStyle>,
    /// Optional stroke.
    pub stroke: Option<&'a StrokeStyle>,
    /// Shape-owned clipping policy.
    pub clip: &'a ShapeClip,
    /// Alpha multiplier.
    pub opacity: f32,
    /// Pointer metadata retained for event adapters.
    pub hit_test: ShapeHitTest,
}

/// Errors returned by the bounded Cupid shape adapter.
#[derive(Debug)]
pub enum ShapeRenderError {
    /// The request's transform, opacity, or paint was invalid.
    InvalidRequest,
    /// The target viewport was invalid.
    InvalidViewport,
    /// The existing SVG pipeline does not yet submit dashed strokes.
    UnsupportedStroke,
    /// A path clip requires a clip path command not present in the existing
    /// shared SVG draw command; bounds clipping remains supported by the canvas.
    UnsupportedClip,
    /// The SVG tessellator rejected the converted path.
    Tessellation(String),
    /// The paint value was invalid.
    Paint(PaintError),
}

impl fmt::Display for ShapeRenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest => f.write_str("shape render request is invalid"),
            Self::InvalidViewport => f.write_str("shape viewport is invalid"),
            Self::UnsupportedStroke => f.write_str("Cupid shape strokes do not support dashes yet"),
            Self::UnsupportedClip => f.write_str("Cupid shape path clips are not supported yet"),
            Self::Tessellation(error) => write!(f, "shape tessellation failed: {error}"),
            Self::Paint(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ShapeRenderError {}

impl From<SvgTessellationError> for ShapeRenderError {
    fn from(error: SvgTessellationError) -> Self {
        Self::Tessellation(error.to_string())
    }
}

/// Builds one SVG scene from a typed shape request.
///
/// The scene has a single path node and therefore preserves command ordering
/// while allowing the existing Cupid renderer to keep its normal z-order,
/// clipping, alpha, and mesh-cache behavior.
pub fn build_scene(
    request: &ShapeRenderRequest<'_>,
    viewport: ShapeSize,
) -> Result<Arc<SvgScene>, ShapeRenderError> {
    validate_request(request, viewport)?;
    let geometry = to_svg_geometry(request.path)?;
    let fill = request.fill.map(|fill| SvgFill {
        color: svg_color(fill.color),
        rule: svg_fill_rule(fill.rule),
    });
    let stroke = request.stroke.map(|stroke| SvgStroke {
        color: svg_color(stroke.color),
        width: stroke.width,
        line_cap: svg_line_cap(stroke.line_cap),
        line_join: svg_line_join(stroke.line_join),
        miter_limit: stroke.miter_limit,
        dash_array: Arc::from([]),
        dash_offset: 0.0,
    });
    let node = SvgNode {
        node_id: SvgNodeId(0),
        svg_id: None,
        classes: Arc::from([]),
        element: SvgElementKind::Path,
        is_definition: false,
        definition_owner: None,
        clip_path: None,
        mask: None,
        filter: None,
        parent: None,
        children: Arc::from([]),
        transform: svg_transform(request.transform),
        opacity: request.opacity,
        geometry: Some(0),
        fill_rule: fill
            .as_ref()
            .map(|fill| fill.rule)
            .unwrap_or(crate::svg::SvgFillRule::NonZero),
        fill,
        stroke,
        fill_paint: None,
        stroke_paint: None,
        paint_order: SvgPaintOrder::FillAndStroke,
        visible: true,
    };
    Ok(Arc::new(SvgScene {
        viewport: SvgViewport {
            width: viewport.width.max(f32::EPSILON),
            height: viewport.height.max(f32::EPSILON),
        },
        nodes: Arc::from([node]),
        geometries: Arc::from([geometry]),
        resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
    }))
}

/// Converts a validated shape path to the existing SVG command model.
pub fn to_svg_geometry(path: &ShapePath) -> Result<SvgGeometry, ShapeRenderError> {
    let mut commands = Vec::with_capacity(path.command_count());
    for command in path.commands().iter().copied() {
        match command {
            aimer_shape::ShapeCommand::MoveTo { x, y } => {
                commands.push(crate::svg::SvgPathCommand::MoveTo { x, y });
            }
            aimer_shape::ShapeCommand::LineTo { x, y } => {
                commands.push(crate::svg::SvgPathCommand::LineTo { x, y });
            }
            aimer_shape::ShapeCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                commands.push(crate::svg::SvgPathCommand::QuadraticTo {
                    control_x,
                    control_y,
                    x,
                    y,
                });
            }
            aimer_shape::ShapeCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                commands.push(crate::svg::SvgPathCommand::CubicTo {
                    control1_x,
                    control1_y,
                    control2_x,
                    control2_y,
                    x,
                    y,
                });
            }
            aimer_shape::ShapeCommand::ArcTo {
                center_x,
                center_y,
                radius_x,
                radius_y,
                start_angle,
                sweep_angle,
                rotation,
            } => append_arc_as_cubics(
                &mut commands,
                center_x,
                center_y,
                radius_x,
                radius_y,
                start_angle,
                sweep_angle,
                rotation,
            ),
            aimer_shape::ShapeCommand::Close => {
                commands.push(crate::svg::SvgPathCommand::Close);
            }
        }
    }
    Ok(SvgGeometry {
        commands: commands.into(),
    })
}

/// A small cache-owning adapter that reuses the SVG tessellator rather than
/// introducing a second custom-path mesh implementation.
pub struct ShapeTessellator {
    cache: SvgGeometryCache,
}

impl ShapeTessellator {
    /// Creates a bounded shape tessellator cache.
    pub fn new(max_memory_bytes: usize, max_entries: usize) -> Self {
        Self {
            cache: SvgGeometryCache::new(max_memory_bytes, max_entries),
        }
    }

    /// Tessellates a fill and reuses it when geometry, rule, and scale match.
    pub fn mesh_for_fill(
        &mut self,
        path: &ShapePath,
        fill: FillStyle,
        physical_scale: f32,
    ) -> Result<Arc<SvgMesh>, ShapeRenderError> {
        fill.validate().map_err(ShapeRenderError::Paint)?;
        let geometry = to_svg_geometry(path)?;
        Ok(self.cache.mesh_for(
            &geometry,
            SvgMeshStyle::Fill(svg_fill_rule(fill.rule)),
            physical_scale,
        )?)
    }

    /// Tessellates a solid stroke and reuses it when its paint geometry matches.
    /// Dashed strokes are rejected until the shared SVG pipeline gains dash
    /// segment tessellation; callers receive a safe fallback error.
    pub fn mesh_for_stroke(
        &mut self,
        path: &ShapePath,
        stroke: &StrokeStyle,
        physical_scale: f32,
    ) -> Result<Arc<SvgMesh>, ShapeRenderError> {
        stroke.validate().map_err(ShapeRenderError::Paint)?;
        if !stroke.dash().is_solid() {
            return Err(ShapeRenderError::UnsupportedStroke);
        }
        let geometry = to_svg_geometry(path)?;
        Ok(self.cache.mesh_for(
            &geometry,
            SvgMeshStyle::Stroke {
                width: stroke.width,
                line_cap: svg_line_cap(stroke.line_cap),
                line_join: svg_line_join(stroke.line_join),
                miter_limit: stroke.miter_limit,
            },
            physical_scale,
        )?)
    }

    /// Returns current cache entry count.
    #[inline]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns whether the cache has no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Returns current CPU mesh memory usage.
    #[inline]
    pub fn memory_bytes(&self) -> usize {
        self.cache.memory_bytes()
    }
}

fn validate_request(
    request: &ShapeRenderRequest<'_>,
    viewport: ShapeSize,
) -> Result<(), ShapeRenderError> {
    if !viewport.is_valid()
        || viewport.width > DEFAULT_MAX_SHAPE_SCENE_COORDINATE
        || viewport.height > DEFAULT_MAX_SHAPE_SCENE_COORDINATE
    {
        return Err(ShapeRenderError::InvalidViewport);
    }
    if !request.transform.is_valid()
        || !request.opacity.is_finite()
        || !(0.0..=1.0).contains(&request.opacity)
    {
        return Err(ShapeRenderError::InvalidRequest);
    }
    let bounds = request.path.bounds();
    if !bounds.min.is_finite()
        || !bounds.max.is_finite()
        || [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
            .into_iter()
            .any(|value| value.abs() > DEFAULT_MAX_SHAPE_SCENE_COORDINATE)
    {
        return Err(ShapeRenderError::InvalidRequest);
    }
    for point in [bounds.min, bounds.max] {
        let transformed = request.transform.transform_point(point);
        if !transformed.is_finite()
            || transformed.x.abs() > DEFAULT_MAX_SHAPE_SCENE_COORDINATE
            || transformed.y.abs() > DEFAULT_MAX_SHAPE_SCENE_COORDINATE
        {
            return Err(ShapeRenderError::InvalidRequest);
        }
    }
    if request.fill.is_none() && request.stroke.is_none() {
        return Err(ShapeRenderError::InvalidRequest);
    }
    if let Some(fill) = request.fill {
        fill.validate().map_err(ShapeRenderError::Paint)?;
    }
    if let Some(stroke) = request.stroke {
        stroke.validate().map_err(ShapeRenderError::Paint)?;
        if !stroke.dash().is_solid() {
            return Err(ShapeRenderError::UnsupportedStroke);
        }
    }
    if matches!(request.clip, ShapeClip::Path(_)) {
        return Err(ShapeRenderError::UnsupportedClip);
    }
    Ok(())
}

fn svg_color(color: aimer_shape::ShapeColor) -> SvgColor {
    SvgColor {
        r: color.r,
        g: color.g,
        b: color.b,
        a: color.a,
    }
}

fn svg_fill_rule(rule: FillRule) -> crate::svg::SvgFillRule {
    match rule {
        FillRule::NonZero => crate::svg::SvgFillRule::NonZero,
        FillRule::EvenOdd => crate::svg::SvgFillRule::EvenOdd,
    }
}

fn svg_line_cap(cap: LineCap) -> SvgLineCap {
    match cap {
        LineCap::Butt => SvgLineCap::Butt,
        LineCap::Round => SvgLineCap::Round,
        LineCap::Square => SvgLineCap::Square,
    }
}

fn svg_line_join(join: LineJoin) -> SvgLineJoin {
    match join {
        LineJoin::Miter => SvgLineJoin::Miter,
        LineJoin::Round => SvgLineJoin::Round,
        LineJoin::Bevel => SvgLineJoin::Bevel,
        LineJoin::MiterClip => SvgLineJoin::MiterClip,
    }
}

fn svg_transform(transform: ShapeTransform) -> SvgTransform {
    let (sin, cos) = transform.rotation.sin_cos();
    SvgTransform {
        sx: cos * transform.sx,
        ky: sin * transform.sx,
        kx: -sin * transform.sy,
        sy: cos * transform.sy,
        tx: transform.tx,
        ty: transform.ty,
    }
}

fn append_arc_as_cubics(
    commands: &mut Vec<crate::svg::SvgPathCommand>,
    center_x: f32,
    center_y: f32,
    radius_x: f32,
    radius_y: f32,
    start_angle: f32,
    sweep_angle: f32,
    rotation: f32,
) {
    let pieces = (sweep_angle.abs() / (core::f32::consts::FRAC_PI_2))
        .ceil()
        .clamp(1.0, 4.0) as usize;
    let delta = sweep_angle / pieces as f32;
    for piece in 0..pieces {
        let angle0 = start_angle + delta * piece as f32;
        let angle1 = angle0 + delta;
        let p0 = ellipse_point(center_x, center_y, radius_x, radius_y, angle0, rotation);
        let p1 = ellipse_point(center_x, center_y, radius_x, radius_y, angle1, rotation);
        let d0 = ellipse_derivative(radius_x, radius_y, angle0, rotation);
        let d1 = ellipse_derivative(radius_x, radius_y, angle1, rotation);
        let coefficient = 4.0 / 3.0 * (delta * 0.25).tan();
        commands.push(crate::svg::SvgPathCommand::CubicTo {
            control1_x: p0.0 + d0.0 * coefficient,
            control1_y: p0.1 + d0.1 * coefficient,
            control2_x: p1.0 - d1.0 * coefficient,
            control2_y: p1.1 - d1.1 * coefficient,
            x: p1.0,
            y: p1.1,
        });
    }
}

fn ellipse_point(
    center_x: f32,
    center_y: f32,
    radius_x: f32,
    radius_y: f32,
    angle: f32,
    rotation: f32,
) -> (f32, f32) {
    let (sin_angle, cos_angle) = angle.sin_cos();
    let (sin_rotation, cos_rotation) = rotation.sin_cos();
    (
        center_x + cos_rotation * radius_x * cos_angle - sin_rotation * radius_y * sin_angle,
        center_y + sin_rotation * radius_x * cos_angle + cos_rotation * radius_y * sin_angle,
    )
}

fn ellipse_derivative(radius_x: f32, radius_y: f32, angle: f32, rotation: f32) -> (f32, f32) {
    let (sin_angle, cos_angle) = angle.sin_cos();
    let (sin_rotation, cos_rotation) = rotation.sin_cos();
    (
        -cos_rotation * radius_x * sin_angle - sin_rotation * radius_y * cos_angle,
        -sin_rotation * radius_x * sin_angle + cos_rotation * radius_y * cos_angle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> ShapePath {
        ShapePath::builder()
            .move_to(0.0, 0.0)
            .line_to(10.0, 0.0)
            .line_to(10.0, 10.0)
            .line_to(0.0, 10.0)
            .close()
            .build()
            .unwrap()
    }

    #[test]
    fn converts_arc_to_finite_cubics_and_builds_one_scene_node() {
        let path = ShapePath::builder()
            .ellipse(10.0, 10.0, 5.0, 3.0, 0.2)
            .build()
            .unwrap();
        let fill = FillStyle::solid(aimer_shape::ShapeColor::rgba8(255, 0, 0, 255));
        let clip = ShapeClip::None;
        let request = ShapeRenderRequest {
            path: &path,
            transform: ShapeTransform::identity(),
            fill: Some(fill),
            stroke: None,
            clip: &clip,
            opacity: 1.0,
            hit_test: ShapeHitTest::Fill,
        };
        let scene = build_scene(&request, ShapeSize::new(20.0, 20.0)).unwrap();
        assert_eq!(scene.nodes.len(), 1);
        assert!(scene.geometries[0].commands.iter().all(|command| {
            match command {
                crate::svg::SvgPathCommand::MoveTo { x, y }
                | crate::svg::SvgPathCommand::LineTo { x, y } => x.is_finite() && y.is_finite(),
                crate::svg::SvgPathCommand::QuadraticTo {
                    x,
                    y,
                    control_x,
                    control_y,
                } => [*x, *y, *control_x, *control_y]
                    .into_iter()
                    .all(f32::is_finite),
                crate::svg::SvgPathCommand::CubicTo {
                    control1_x,
                    control1_y,
                    control2_x,
                    control2_y,
                    x,
                    y,
                } => [*control1_x, *control1_y, *control2_x, *control2_y, *x, *y]
                    .into_iter()
                    .all(f32::is_finite),
                crate::svg::SvgPathCommand::Close => true,
            }
        }));
    }

    #[test]
    fn mesh_cache_reuses_same_geometry_and_invalidates_stroke_parameters() {
        let path = square();
        let fill = FillStyle::solid(aimer_shape::ShapeColor::BLACK);
        let mut tessellator = ShapeTessellator::new(1024 * 1024, 8);
        let first = tessellator.mesh_for_fill(&path, fill, 1.0).unwrap();
        let second = tessellator.mesh_for_fill(&path, fill, 1.0).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let thin = StrokeStyle::new(1.0, aimer_shape::ShapeColor::BLACK).unwrap();
        let thick = StrokeStyle::new(4.0, aimer_shape::ShapeColor::BLACK).unwrap();
        let thin_mesh = tessellator.mesh_for_stroke(&path, &thin, 1.0).unwrap();
        let thick_mesh = tessellator.mesh_for_stroke(&path, &thick, 1.0).unwrap();
        assert!(!Arc::ptr_eq(&thin_mesh, &thick_mesh));
        assert_eq!(tessellator.len(), 3);
    }

    #[test]
    fn unsupported_clip_and_dash_have_typed_errors() {
        let path = square();
        let clip = ShapeClip::Path(Arc::new(path.clone()));
        let fill = FillStyle::solid(aimer_shape::ShapeColor::BLACK);
        let request = ShapeRenderRequest {
            path: &path,
            transform: ShapeTransform::identity(),
            fill: Some(fill),
            stroke: None,
            clip: &clip,
            opacity: 1.0,
            hit_test: ShapeHitTest::None,
        };
        assert!(matches!(
            build_scene(&request, ShapeSize::new(10.0, 10.0)),
            Err(ShapeRenderError::UnsupportedClip)
        ));
        let stroke = StrokeStyle::new(2.0, aimer_shape::ShapeColor::BLACK)
            .unwrap()
            .with_dash([2.0, 1.0], 0.0)
            .unwrap();
        let clip = ShapeClip::None;
        let request = ShapeRenderRequest {
            path: &path,
            transform: ShapeTransform::identity(),
            fill: None,
            stroke: Some(&stroke),
            clip: &clip,
            opacity: 1.0,
            hit_test: ShapeHitTest::Stroke,
        };
        assert!(matches!(
            build_scene(&request, ShapeSize::new(10.0, 10.0)),
            Err(ShapeRenderError::UnsupportedStroke)
        ));
    }
}
