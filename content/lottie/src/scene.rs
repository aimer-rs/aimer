//! A Lottie composition as an SVG scene that Cupid already knows how to draw.
//!
//! The renderer draws a flat list of path nodes, each with its own complete
//! transform and opacity, and takes per-node overrides on top. A Lottie group
//! becomes one node per fill or stroke it contains, the shapes above that
//! painter merged into a single path. The scene is built once; each frame only
//! computes overrides for the nodes whose transform, opacity or color differ
//! from the base node. Geometry that animates, and animated stroke widths,
//! rebuild just their own slice of the scene for that frame.

use std::sync::Arc;

use aimer_cupid::svg::{
    SvgColor, SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgLineCap, SvgLineJoin, SvgNode,
    SvgNodeId, SvgNodeStyleOverride, SvgPaintOrder, SvgPathCommand, SvgResourceGraph, SvgScene,
    SvgStroke, SvgTransform, SvgViewport,
};

use crate::affine::Affine;
use crate::geometry;
use crate::model::{Composition, FillRule, LayerKind, LineCap, LineJoin, MatteMode, Property, Rgba, Shape};
use crate::trim::trim;

/// What to draw for one frame: a scene and the per-node overrides on top of it.
#[derive(Debug, Clone)]
pub struct LottieFrame {
    /// Shared between frames unless a shape or stroke width animates.
    pub scene: Arc<SvgScene>,
    /// In node order, and only for nodes that differ from the base scene.
    pub overrides: Arc<[SvgNodeStyleOverride]>,
}

/// A composition prepared for drawing.
///
/// A composition becomes one or more planes: scenes drawn on their own, each
/// with its own nodes. Layers that are drawn as they are share a plane; a layer
/// cut by a track matte gets a plane of its own, and so does its matte, because
/// the renderer applies a matte between two separate draws. [`blocks`](Self::blocks)
/// says how the planes are used, bottom first.
pub struct LottieScene {
    composition: Composition,
    planes: Vec<Plane>,
    blocks: Vec<Block>,
}

/// One thing a [`Lottie`](crate::Lottie) draws, bottom first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// A plane drawn as it is.
    Plain(usize),
    /// A plane shown only where another plane has coverage.
    Matted {
        /// The plane that is cut.
        content: usize,
        /// The plane that cuts it; it is never drawn itself.
        matte: usize,
    },
}

/// A scene and the leaves that map its nodes back to the composition.
struct Plane {
    scene: Arc<SvgScene>,
    /// One per scene node, in node order.
    leaves: Vec<Leaf>,
}

/// How one scene node maps back to the composition.
struct Leaf {
    layer: usize,
    /// Index of the enclosing group in its list at each level, outermost first.
    path: Vec<usize>,
    /// Index of the fill or stroke in the innermost list.
    painter: usize,
    /// The shapes the painter covers, grouped by the trims that apply to them.
    groups: Vec<ShapeGroup>,
    animated_geometry: bool,
    animated_width: bool,
}

/// Shapes that share the same trims, with the trims in the order they apply.
struct ShapeGroup {
    /// Indices in the innermost list.
    shapes: Vec<usize>,
    /// Indices in the innermost list of the trims listed below these shapes and
    /// above the painter, nearest to the shapes first.
    trims: Vec<usize>,
}

/// One group on the way down to a leaf, with everything above it applied.
struct Level<'a> {
    item: usize,
    matrix: Affine,
    opacity: f32,
    items: &'a [Shape],
}

impl LottieScene {
    /// Builds the scene for `composition`, with every property read at its
    /// first frame.
    ///
    /// Only an alpha track matte can be applied. A layer cut by any other kind
    /// is left out, and [`Composition::diagnostics`] says so.
    pub fn new(composition: Composition) -> Self {
        let mut planes = Vec::new();
        let mut blocks = Vec::new();
        // Plain layers waiting for the next matted layer or the end; they share
        // one plane so that a flat file stays one draw.
        let mut run: Vec<usize> = Vec::new();
        // Later layers are drawn first, so they get the lowest node indices.
        for index in (0..composition.layers.len()).rev() {
            let layer = &composition.layers[index];
            if layer.is_matte {
                continue;
            }
            match (layer.matte, layer.matte_slot) {
                (None, _) => run.push(index),
                (Some(MatteMode::Alpha), Some(matte)) => {
                    flush_plain(&composition, &mut run, &mut planes, &mut blocks);
                    let content = Plane::build(&composition, &[index]);
                    let matte = Plane::build(&composition, &[matte]);
                    // Cutting something by nothing, or nothing by something,
                    // shows nothing.
                    if content.leaves.is_empty() || matte.leaves.is_empty() {
                        continue;
                    }
                    blocks.push(Block::Matted { content: planes.len(), matte: planes.len() + 1 });
                    planes.push(content);
                    planes.push(matte);
                }
                _ => {}
            }
        }
        flush_plain(&composition, &mut run, &mut planes, &mut blocks);
        if blocks.is_empty() {
            blocks.push(Block::Plain(0));
            planes.push(Plane::build(&composition, &[]));
        }
        Self { composition, planes, blocks }
    }

    /// The composition this scene draws.
    pub fn composition(&self) -> &Composition {
        &self.composition
    }

    /// What to draw, bottom first. Never empty.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The scene and overrides that show the first plane at `frame`.
    ///
    /// A composition without track mattes has exactly one plane, which holds all
    /// of it; see [`plane_frame`](Self::plane_frame) for the others.
    pub fn frame(&self, frame: f32) -> LottieFrame {
        self.plane_frame(0, frame)
    }

    /// The scene and overrides that show plane `plane` at `frame`.
    ///
    /// # Panics
    ///
    /// Panics when `plane` is not one of the planes named by
    /// [`blocks`](Self::blocks).
    pub fn plane_frame(&self, plane: usize, frame: f32) -> LottieFrame {
        self.planes[plane].frame(&self.composition, frame)
    }
}

/// Moves the waiting plain layers into a plane of their own, if they draw
/// anything.
fn flush_plain(
    composition: &Composition,
    run: &mut Vec<usize>,
    planes: &mut Vec<Plane>,
    blocks: &mut Vec<Block>,
) {
    if run.is_empty() {
        return;
    }
    let plane = Plane::build(composition, run);
    run.clear();
    if !plane.leaves.is_empty() {
        blocks.push(Block::Plain(planes.len()));
        planes.push(plane);
    }
}

impl Plane {
    /// The plane that draws `layers`, bottom first.
    fn build(composition: &Composition, layers: &[usize]) -> Self {
        let mut leaves = Vec::new();
        for &index in layers {
            if let LayerKind::Shape(items) = &composition.layers[index].kind {
                let mut path = Vec::new();
                collect(items, index, &mut path, &mut leaves);
            }
        }
        let base = composition.in_frame;
        let mut nodes = Vec::with_capacity(leaves.len());
        let mut geometries = Vec::with_capacity(leaves.len());
        for (i, leaf) in leaves.iter().enumerate() {
            let items = leaf.items(composition);
            let mut commands = Vec::new();
            leaf.commands(items, base, &mut commands);
            geometries.push(SvgGeometry { commands: Arc::from(commands) });
            nodes.push(base_node(i, &items[leaf.painter], base));
        }
        let scene = Arc::new(SvgScene {
            viewport: SvgViewport { width: composition.width, height: composition.height },
            nodes: Arc::from(nodes),
            geometries: Arc::from(geometries),
            resources: Arc::new(SvgResourceGraph::empty()),
        });
        Self { scene, leaves }
    }

    fn frame(&self, composition: &Composition, frame: f32) -> LottieFrame {
        let mut states = Vec::new();
        composition.layer_states(frame, &mut states);

        let mut overrides = Vec::new();
        let mut geometry_patch: Vec<(usize, SvgGeometry)> = Vec::new();
        let mut width_patch: Vec<(usize, f32)> = Vec::new();
        // Leaves of one layer are adjacent and share group prefixes, so the
        // stack of evaluated groups is reused instead of rebuilt per leaf.
        let mut stack: Vec<Level> = Vec::new();
        let mut stack_layer = usize::MAX;

        for (i, leaf) in self.leaves.iter().enumerate() {
            let state = &states[leaf.layer];
            let LayerKind::Shape(layer_items) = &composition.layers[leaf.layer].kind else {
                continue;
            };
            let layer_opacity = if state.visible { state.opacity } else { 0.0 };
            if leaf.layer != stack_layer {
                stack.clear();
                stack_layer = leaf.layer;
            }
            let mut keep = 0;
            while keep < stack.len() && keep < leaf.path.len() && stack[keep].item == leaf.path[keep] {
                keep += 1;
            }
            stack.truncate(keep);
            for depth in keep..leaf.path.len() {
                let (matrix, opacity, items) = match stack.last() {
                    Some(level) => (level.matrix, level.opacity, level.items),
                    None => (state.world, layer_opacity, layer_items.as_slice()),
                };
                let Shape::Group(group) = &items[leaf.path[depth]] else {
                    unreachable!("leaf paths only name groups");
                };
                let local = group.transform.local_at(frame);
                stack.push(Level {
                    item: leaf.path[depth],
                    matrix: matrix * local.matrix,
                    opacity: opacity * local.opacity,
                    items: &group.items,
                });
            }
            let (matrix, opacity, items) = match stack.last() {
                Some(level) => (level.matrix, level.opacity, level.items),
                None => (state.world, layer_opacity, layer_items.as_slice()),
            };

            let node = &self.scene.nodes[i];
            let mut change = SvgNodeStyleOverride {
                node_id: node.node_id,
                fill: None,
                stroke: None,
                opacity: None,
                transform: None,
            };
            let transform = svg_transform(matrix);
            if transform != node.transform {
                change.transform = Some(transform);
            }
            if opacity != node.opacity {
                change.opacity = Some(opacity);
            }
            match &items[leaf.painter] {
                Shape::Fill(fill) => {
                    let color = paint_color(fill.color.value_at(frame), fill.opacity.value_at(frame));
                    if node.fill.as_ref().is_none_or(|base| base.color != color) {
                        change.fill = Some(Some(color));
                    }
                }
                Shape::Stroke(stroke) => {
                    let color =
                        paint_color(stroke.color.value_at(frame), stroke.opacity.value_at(frame));
                    if node.stroke.as_ref().is_none_or(|base| base.color != color) {
                        change.stroke = Some(Some(color));
                    }
                    if leaf.animated_width {
                        width_patch.push((i, stroke.width.value_at(frame).max(0.0)));
                    }
                }
                _ => {}
            }
            if leaf.animated_geometry {
                let mut commands = Vec::new();
                leaf.commands(items, frame, &mut commands);
                geometry_patch.push((i, SvgGeometry { commands: Arc::from(commands) }));
            }
            if change.transform.is_some()
                || change.opacity.is_some()
                || change.fill.is_some()
                || change.stroke.is_some()
            {
                overrides.push(change);
            }
        }

        let scene = if geometry_patch.is_empty() && width_patch.is_empty() {
            self.scene.clone()
        } else {
            let nodes = if width_patch.is_empty() {
                self.scene.nodes.clone()
            } else {
                let mut nodes = self.scene.nodes.to_vec();
                for (i, width) in width_patch {
                    if let Some(stroke) = nodes[i].stroke.as_mut() {
                        stroke.width = width;
                    }
                }
                Arc::from(nodes)
            };
            let geometries = if geometry_patch.is_empty() {
                self.scene.geometries.clone()
            } else {
                let mut geometries = self.scene.geometries.to_vec();
                for (i, geometry) in geometry_patch {
                    geometries[i] = geometry;
                }
                Arc::from(geometries)
            };
            Arc::new(SvgScene {
                viewport: self.scene.viewport,
                nodes,
                geometries,
                resources: self.scene.resources.clone(),
            })
        };
        LottieFrame { scene, overrides: Arc::from(overrides) }
    }
}

impl Leaf {
    /// The innermost list of items this leaf's painter sits in.
    fn items<'a>(&self, composition: &'a Composition) -> &'a [Shape] {
        let LayerKind::Shape(layer_items) = &composition.layers[self.layer].kind else {
            unreachable!("leaves only come from shape layers");
        };
        let mut items: &[Shape] = layer_items.as_slice();
        for &index in &self.path {
            let Shape::Group(group) = &items[index] else {
                unreachable!("leaf paths only name groups");
            };
            items = &group.items;
        }
        items
    }

    /// Appends the covered shapes, evaluated at `frame` and trimmed, as one path.
    fn commands(&self, items: &[Shape], frame: f32, out: &mut Vec<SvgPathCommand>) {
        for group in &self.groups {
            if group.trims.is_empty() {
                shape_commands(items, &group.shapes, frame, out);
                continue;
            }
            let mut current = Vec::new();
            shape_commands(items, &group.shapes, frame, &mut current);
            for &index in &group.trims {
                let Shape::Trim(spec) = &items[index] else {
                    unreachable!("trim lists only name trims");
                };
                let mut next = Vec::with_capacity(current.len());
                trim(&current, spec.segments_at(frame), spec.simultaneous, &mut next);
                current = next;
            }
            out.extend(current);
        }
    }
}

/// Appends the listed shapes of `items`, evaluated at `frame`.
fn shape_commands(items: &[Shape], shapes: &[usize], frame: f32, out: &mut Vec<SvgPathCommand>) {
    for &index in shapes {
        match &items[index] {
            Shape::Rect(rect) => geometry::rect(
                rect.position.value_at(frame),
                rect.size.value_at(frame),
                rect.roundness.value_at(frame),
                out,
            ),
            Shape::Ellipse(ellipse) => {
                geometry::ellipse(ellipse.position.value_at(frame), ellipse.size.value_at(frame), out)
            }
            Shape::Path(path) => geometry::bezier(&path.path.value_at(frame), out),
            _ => {}
        }
    }
}

/// Walks one list of items, bottom to top, adding a leaf per painter that has
/// shapes above it. Lottie lists items top-first, and the renderer draws nodes
/// in index order, so the list is walked in reverse.
///
/// A trim applies to the shapes listed above it, and a painter draws whatever
/// the trims between its shapes and itself leave of them.
fn collect(items: &[Shape], layer: usize, path: &mut Vec<usize>, leaves: &mut Vec<Leaf>) {
    for (k, item) in items.iter().enumerate().rev() {
        match item {
            Shape::Group(group) => {
                path.push(k);
                collect(&group.items, layer, path, leaves);
                path.pop();
            }
            Shape::Fill(_) | Shape::Stroke(_) => {
                let mut groups: Vec<ShapeGroup> = Vec::new();
                for s in (0..k).filter(|&j| matches!(items[j], Shape::Rect(_) | Shape::Ellipse(_) | Shape::Path(_))) {
                    let trims: Vec<usize> =
                        (s + 1..k).filter(|&j| matches!(items[j], Shape::Trim(_))).collect();
                    match groups.last_mut() {
                        Some(group) if group.trims == trims => group.shapes.push(s),
                        _ => groups.push(ShapeGroup { shapes: vec![s], trims }),
                    }
                }
                if groups.is_empty() {
                    continue;
                }
                let animated_geometry = groups.iter().any(|group| {
                    group.shapes.iter().any(|&s| geometry_is_animated(&items[s]))
                        || group.trims.iter().any(|&t| trim_is_animated(&items[t]))
                });
                leaves.push(Leaf {
                    layer,
                    path: path.clone(),
                    painter: k,
                    groups,
                    animated_geometry,
                    animated_width: matches!(item, Shape::Stroke(s) if matches!(s.width, Property::Animated(_))),
                });
            }
            _ => {}
        }
    }
}

fn trim_is_animated(shape: &Shape) -> bool {
    match shape {
        Shape::Trim(t) => {
            matches!(t.start, Property::Animated(_))
                || matches!(t.end, Property::Animated(_))
                || matches!(t.offset, Property::Animated(_))
        }
        _ => false,
    }
}

fn geometry_is_animated(shape: &Shape) -> bool {
    match shape {
        Shape::Rect(r) => {
            matches!(r.position, Property::Animated(_))
                || matches!(r.size, Property::Animated(_))
                || matches!(r.roundness, Property::Animated(_))
        }
        Shape::Ellipse(e) => {
            matches!(e.position, Property::Animated(_)) || matches!(e.size, Property::Animated(_))
        }
        Shape::Path(p) => matches!(p.path, Property::Animated(_)),
        _ => false,
    }
}

/// The base node for a painter, with its paint read at `frame`.
fn base_node(index: usize, painter: &Shape, frame: f32) -> SvgNode {
    let mut fill = None;
    let mut stroke = None;
    let mut fill_rule = SvgFillRule::NonZero;
    match painter {
        Shape::Fill(f) => {
            fill_rule = match f.rule {
                FillRule::NonZero => SvgFillRule::NonZero,
                FillRule::EvenOdd => SvgFillRule::EvenOdd,
            };
            fill = Some(SvgFill {
                color: paint_color(f.color.value_at(frame), f.opacity.value_at(frame)),
                rule: fill_rule,
            });
        }
        Shape::Stroke(s) => {
            stroke = Some(SvgStroke {
                color: paint_color(s.color.value_at(frame), s.opacity.value_at(frame)),
                width: s.width.value_at(frame).max(0.0),
                line_cap: match s.cap {
                    LineCap::Butt => SvgLineCap::Butt,
                    LineCap::Round => SvgLineCap::Round,
                    LineCap::Square => SvgLineCap::Square,
                },
                line_join: match s.join {
                    LineJoin::Miter => SvgLineJoin::Miter,
                    LineJoin::Round => SvgLineJoin::Round,
                    LineJoin::Bevel => SvgLineJoin::Bevel,
                },
                miter_limit: s.miter_limit,
                dash_array: Arc::from([]),
                dash_offset: 0.0,
            });
        }
        _ => {}
    }
    SvgNode {
        node_id: SvgNodeId(index as u32),
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
        transform: SvgTransform::default(),
        opacity: 1.0,
        geometry: Some(index),
        fill_rule,
        fill,
        stroke,
        fill_paint: None,
        stroke_paint: None,
        paint_order: SvgPaintOrder::FillAndStroke,
        visible: true,
    }
}

/// A paint color with the paint's own opacity (percent) folded into alpha.
fn paint_color(color: Rgba, opacity_percent: f32) -> SvgColor {
    SvgColor {
        r: color[0],
        g: color[1],
        b: color[2],
        a: color[3] * (opacity_percent / 100.0).clamp(0.0, 1.0),
    }
}

fn svg_transform(m: Affine) -> SvgTransform {
    SvgTransform { sx: m.a, ky: m.b, kx: m.c, sy: m.d, tx: m.e, ty: m.f }
}
