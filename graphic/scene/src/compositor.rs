//! The in-process retained compositor scene.
//!
//! The scene is the immutable frame transaction lowered from Aimer's retained
//! element tree. It carries logical nodes, paint order, transforms, clips,
//! damage footprints, and the command/surface payload ranges that the renderer
//! can consume without walking widgets again. Ordinary commands remain in
//! [`crate::draw_cmd::DrawList`] as the safe payload fallback; promoted content
//! is represented by renderer-owned surfaces and composed directly when its
//! pixels are unchanged.
//! 
use std::cell::RefCell;
use std::cmp::Reverse;
use std::rc::Rc;

use crate::damage_region::{DamageRect, DamageSet};
use crate::draw_cmd::{DrawCommand, DrawList};
use crate::utilities::{Mat3, Rect};

mod retained;
pub use retained::{RetainedSceneTree, SceneCommit};

/// Work counters for the most recently submitted compositor frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompositorStats {
    /// Number of retained surfaces in the frame.
    pub promoted_surfaces: usize,
    /// Number of promoted surfaces whose content was rasterized this frame.
    pub rasterized_surfaces: usize,
    /// Number of promoted surfaces composited from an existing GPU texture.
    pub reused_surfaces: usize,
    /// Number of retained surfaces submitted to the composition pass.
    pub composed_surfaces: usize,
    /// Number of normalized damage regions considered by the renderer.
    pub damage_regions: usize,
    /// Sum of the normalized damage-region areas.
    pub damaged_pixels: u64,
    /// Whether the renderer had to repaint the full persistent target.
    pub full_repaint: bool,
    /// Number of target-to-window composition passes encoded for the frame.
    pub composition_passes: usize,
    /// Number of logical scene nodes visited while lowering the frame.
    pub logical_nodes: usize,
    /// Number of scene nodes whose content stayed on the live command path.
    pub live_nodes: usize,
    /// Number of scene nodes reused from the previous accepted scene.
    pub reused_nodes: usize,
    /// Number of scene properties changed since the previous accepted scene.
    pub scene_changes: usize,
}

/// Stable identity of one logical element in the compositor scene.
///
/// This identity is intentionally separate from [`SurfaceId`]. An element can
/// stay the same while its renderer-owned surface is recreated after a resize,
/// format change, or cache eviction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SceneNodeId(u64);

impl SceneNodeId {
    /// Creates a scene identity from the element-tree identity.
    #[inline]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw element identity used by diagnostics and adapters.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Revisions used to decide whether a node's recorded content can be reused.
///
/// The values come from the retained element tree and resource registry. They
/// deliberately do not hash draw commands, which keeps lowering predictable
/// for large text, image, and custom-pipeline payloads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SceneRevision {
    /// Revision of the installed subtree/configuration.
    pub subtree: u64,
    /// Revision of direct paint invalidation for this node.
    pub paint: u64,
    /// Revision of layout inputs affecting the node's geometry.
    pub layout: u64,
    /// Revision of renderer resources referenced by the node.
    pub resources: u64,
}

impl SceneRevision {
    /// Creates a revision tuple for one scene node.
    #[inline]
    pub const fn new(subtree: u64, paint: u64, layout: u64, resources: u64) -> Self {
        Self {
            subtree,
            paint,
            layout,
            resources,
        }
    }
}

/// Presence of a node in the visible scene.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenePresence {
    /// The element emitted live or cached content this frame.
    Visible,
    /// The element's previous retained content was reused without repainting.
    Reused,
    /// The element exists in the retained widget tree but is outside the
    /// visible/windowed paint pass.
    Culled,
}

/// How a logical scene node supplies its visual content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneContent {
    /// The node's commands are replayed from the frame payload. This includes
    /// command-retained content that avoids a GPU surface because a separate
    /// texture would cost more than replaying its small command stream.
    Live,
    /// A renderer-owned retained surface supplies the node's pixels.
    CachedSurface(SurfaceId),
}

/// A clip captured at the point where a node was painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneClip {
    rect: Rect,
    border_radius: [f32; 4],
    transform: Mat3,
    frame_bounds: Option<Rect>,
    effective_bounds: Option<Rect>,
}

impl SceneClip {
    /// Returns the clip rectangle in the local coordinate space in which it
    /// was pushed.
    #[inline]
    pub const fn rect(self) -> Rect {
        self.rect
    }

    /// Returns the clip's per-corner radii in local coordinates.
    #[inline]
    pub const fn border_radius(self) -> [f32; 4] {
        self.border_radius
    }

    /// Returns the transform active when this clip was pushed.
    #[inline]
    pub const fn transform(self) -> Mat3 {
        self.transform
    }

    /// Returns the conservative frame-space bounds before parent clips.
    #[inline]
    pub const fn frame_bounds(self) -> Option<Rect> {
        self.frame_bounds
    }

    /// Returns the conservative frame-space bounds after inherited clips.
    #[inline]
    pub const fn effective_bounds(self) -> Option<Rect> {
        self.effective_bounds
    }
}

/// An inherited ordered chain of rectangular or rounded clips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipChain {
    clips: Vec<SceneClip>,
}

impl ClipChain {
    /// Creates an empty clip chain.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns clips from the outermost inherited clip to the innermost one.
    #[inline]
    pub fn clips(&self) -> &[SceneClip] {
        &self.clips
    }

    /// Returns the conservative effective bounds of the complete chain.
    pub fn effective_bounds(&self) -> Option<Rect> {
        let mut effective = None;
        for clip in &self.clips {
            let bounds = clip.effective_bounds?;
            effective = Some(effective.map_or(bounds, |parent| intersect_rect(parent, bounds)));
        }
        effective
    }
}

/// The descriptor supplied by the Element paint seam when a node opens.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneNodeDescriptor {
    /// Stable identity of the originating element.
    pub id: SceneNodeId,
    /// Local content bounds before the current canvas transform.
    pub bounds: Rect,
    /// Explicit widget paint layer used as a stable ordering tie-breaker.
    pub order: u32,
    /// Revision tuple used by renderer-side scene diffing.
    pub revision: SceneRevision,
    /// Whether the node satisfies the paint-only cache contract.
    pub cache_eligible: bool,
    /// Whether visual output is known to remain inside `bounds`.
    pub bounded: bool,
    /// Whether an explicit widget hint asks the compositor to prefer isolation.
    pub priority: bool,
}

impl SceneNodeDescriptor {
    /// Creates a conservative live-node descriptor.
    #[inline]
    pub const fn new(id: SceneNodeId, bounds: Rect, order: u32) -> Self {
        Self {
            id,
            bounds,
            order,
            revision: SceneRevision::new(0, 0, 0, 0),
            cache_eligible: false,
            bounded: false,
            priority: false,
        }
    }
}

/// One ordered paint operation in a logical node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScenePaintOp {
    /// Direct commands owned by the node.
    Commands { start: usize, end: usize },
    /// A child node at its exact paint position.
    Child(SceneNodeId),
    /// A retained surface replacing a direct retained-layer command.
    Surface(SurfaceNode),
}

/// One logical drawable element in the public read-only scene tree.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    id: SceneNodeId,
    parent: Option<SceneNodeId>,
    bounds: Rect,
    frame_bounds: Option<Rect>,
    effective_bounds: Option<Rect>,
    transform: Mat3,
    opacity: f32,
    order: u32,
    revision: SceneRevision,
    presence: ScenePresence,
    content: SceneContent,
    cache_eligible: bool,
    bounded: bool,
    priority: bool,
    clip_chain: ClipChain,
    children: Vec<SceneNodeId>,
    paint_ops: Vec<ScenePaintOp>,
}

impl SceneNode {
    /// Returns the stable logical identity.
    #[inline]
    pub const fn id(&self) -> SceneNodeId {
        self.id
    }

    /// Returns the direct parent, or `None` for a scene root.
    #[inline]
    pub const fn parent(&self) -> Option<SceneNodeId> {
        self.parent
    }

    /// Returns local logical content bounds.
    #[inline]
    pub const fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Returns transformed frame-space bounds when the transform is finite.
    #[inline]
    pub const fn frame_bounds(&self) -> Option<Rect> {
        self.frame_bounds
    }

    /// Returns transformed bounds after applying the inherited clip chain.
    ///
    /// `None` means the geometry or clip state was not finite and therefore
    /// requires conservative full-target damage. A zero-sized rectangle is a
    /// valid, fully clipped/empty footprint and does not require damage.
    #[inline]
    pub const fn effective_bounds(&self) -> Option<Rect> {
        self.effective_bounds
    }

    /// Returns the local-to-frame transform captured during paint.
    #[inline]
    pub const fn transform(&self) -> Mat3 {
        self.transform
    }

    /// Returns effective node opacity.
    #[inline]
    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Returns the widget's paint-layer ordering value.
    #[inline]
    pub const fn order(&self) -> u32 {
        self.order
    }

    /// Returns the content revision tuple.
    #[inline]
    pub const fn revision(&self) -> SceneRevision {
        self.revision
    }

    /// Returns whether the node is visible, reused, or culled.
    #[inline]
    pub const fn presence(&self) -> ScenePresence {
        self.presence
    }

    /// Returns the current content supply mode.
    #[inline]
    pub const fn content(&self) -> SceneContent {
        self.content
    }

    /// Returns whether this node satisfies automatic retention eligibility.
    #[inline]
    pub const fn cache_eligible(&self) -> bool {
        self.cache_eligible
    }

    /// Returns whether the node is known not to paint outside its bounds.
    #[inline]
    pub const fn bounded(&self) -> bool {
        self.bounded
    }

    /// Returns whether an explicit widget hint prioritizes this node.
    #[inline]
    pub const fn priority(&self) -> bool {
        self.priority
    }

    /// Returns the inherited clip chain.
    #[inline]
    pub fn clip_chain(&self) -> &ClipChain {
        &self.clip_chain
    }

    /// Returns direct children in their retained paint order.
    #[inline]
    pub fn children(&self) -> &[SceneNodeId] {
        &self.children
    }

    /// Returns direct commands and children in exact paint order.
    #[inline]
    pub fn paint_ops(&self) -> &[ScenePaintOp] {
        &self.paint_ops
    }
}

/// Stable identity of a promoted compositor surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceId(u64);

impl SurfaceId {
    /// Creates an id from the retained-layer identity used by the draw list.
    #[inline]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw id for diagnostics and renderer-side caches.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Compositor properties that can change without rerasterizing surface content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceProperties {
    /// The surface's local logical bounds.
    pub bounds: Rect,
    /// Transform from local surface coordinates into frame coordinates.
    pub transform: Mat3,
    /// Premultiplied opacity applied while composing the surface.
    pub opacity: f32,
    /// Stable paint order. Larger values are painted later.
    pub order: u32,
}

impl SurfaceProperties {
    /// Creates ordinary, fully opaque properties at the identity transform.
    #[inline]
    pub const fn new(bounds: Rect, order: u32) -> Self {
        Self {
            bounds,
            transform: Mat3::identity(),
            opacity: 1.0,
            order,
        }
    }

    /// Returns the conservative frame-space bounds for this surface.
    #[inline]
    pub fn frame_bounds(self) -> Option<Rect> {
        let corners = [
            self.transform.transform_point(self.bounds.x, self.bounds.y),
            self.transform
                .transform_point(self.bounds.x + self.bounds.width, self.bounds.y),
            self.transform
                .transform_point(self.bounds.x, self.bounds.y + self.bounds.height),
            self.transform.transform_point(
                self.bounds.x + self.bounds.width,
                self.bounds.y + self.bounds.height,
            ),
        ];
        let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
        for (x, y) in corners {
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }
}

/// The kind of content promoted into a compositor surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceKind {
    /// Content is rasterized from a renderer-owned retained layer.
    RetainedLayer,
}

/// One ordered surface in a compositor scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceNode {
    id: SurfaceId,
    kind: SurfaceKind,
    properties: SurfaceProperties,
}

impl SurfaceNode {
    /// Creates a retained-layer scene node.
    #[inline]
    pub const fn retained_layer(id: SurfaceId, properties: SurfaceProperties) -> Self {
        Self {
            id,
            kind: SurfaceKind::RetainedLayer,
            properties,
        }
    }

    /// Returns the stable surface identity.
    #[inline]
    pub const fn id(self) -> SurfaceId {
        self.id
    }

    /// Returns how the surface's content is supplied.
    #[inline]
    pub const fn kind(self) -> SurfaceKind {
        self.kind
    }

    /// Returns the property state captured for this frame.
    #[inline]
    pub const fn properties(self) -> SurfaceProperties {
        self.properties
    }
}

/// One ordered item in a compositor scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneItem {
    /// A contiguous range of ordinary draw-list commands.
    Commands {
        /// Inclusive command index at which the segment starts.
        start: usize,
        /// Exclusive command index at which the segment ends.
        end: usize,
    },
    /// A promoted retained surface.
    Surface(SurfaceNode),
}

/// One item yielded by the scene-led renderer.
#[doc(hidden)]
#[allow(unused)]
pub(crate) enum SceneRenderItem<'a> {
    /// A live DrawList command owned by the scene's command payload.
    Command(&'a DrawCommand),
    /// A retained surface replacing its original DrawList command.
    Surface(SurfaceNode),
}

/// A renderer-side change detected between two accepted scenes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneChangeKind {
    /// A node appeared in the visible scene.
    Added,
    /// A node disappeared from the visible scene.
    Removed,
    /// The node's recorded content revision changed.
    Content,
    /// The node's bounds or transform changed.
    Geometry,
    /// The inherited clip chain changed.
    Clip,
    /// The node's opacity changed.
    Opacity,
    /// The node's paint order or parent changed.
    Order,
}

/// One scene change and its conservative old/new damage footprints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneChange {
    /// Logical node affected by the change.
    pub id: SceneNodeId,
    /// Property class that changed.
    pub kind: SceneChangeKind,
    /// Old frame-space footprint, when available.
    pub old_bounds: Option<Rect>,
    /// New frame-space footprint, when available.
    pub new_bounds: Option<Rect>,
}

/// The immutable result of comparing two scene snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneDiff {
    changes: Vec<SceneChange>,
    damage: DamageSet,
    full: bool,
}

impl SceneDiff {
    /// Returns all changes in stable node-id order.
    #[inline]
    pub fn changes(&self) -> &[SceneChange] {
        &self.changes
    }

    /// Returns conservative damage caused by the changes.
    #[inline]
    pub fn damage(&self) -> &DamageSet {
        &self.damage
    }

    /// Returns whether the diff requires a full target repaint.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.full
    }

    /// Returns whether no scene properties changed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Immutable compositor frame transaction attached to one frame packet.
///
/// Nodes are kept in lowering order and retain the exact command ranges or
/// surfaces that produced them. The draw list remains the payload store: an
/// unsupported promotion is represented as ordinary commands and therefore
/// remains on the safe live path without weakening scene validation.
#[derive(Clone, Debug)]
pub struct CompositorScene {
    target_width: u32,
    target_height: u32,
    damage: DamageSet,
    surfaces: Vec<SurfaceNode>,
    items: Vec<SceneItem>,
    render_items: Vec<SceneItem>,
    roots: Vec<SceneNodeId>,
    nodes: Vec<SceneNode>,
    recorded: bool,
}

impl CompositorScene {
    /// Creates an empty scene for a target.
    #[inline]
    pub fn new(target_width: u32, target_height: u32) -> Self {
        Self {
            target_width,
            target_height,
            damage: DamageSet::new(target_width, target_height),
            surfaces: Vec::new(),
            items: Vec::new(),
            render_items: Vec::new(),
            roots: Vec::new(),
            nodes: Vec::new(),
            recorded: false,
        }
    }

    /// Creates scene metadata with the frame's normalized damage.
    #[inline]
    pub fn with_damage(target_width: u32, target_height: u32, damage: DamageSet) -> Self {
        let damage = if damage.target_size() == (target_width, target_height) {
            damage
        } else {
            DamageSet::full(target_width, target_height)
        };
        Self {
            target_width,
            target_height,
            damage,
            surfaces: Vec::new(),
            items: Vec::new(),
            render_items: Vec::new(),
            roots: Vec::new(),
            nodes: Vec::new(),
            recorded: false,
        }
    }

    /// Builds the scene's promoted-surface list from a recorded draw list.
    ///
    /// This is intentionally conservative: only explicit retained-layer
    /// commands become surfaces. Every other command stays in the ordered
    /// command stream and can never be accidentally omitted by the compositor.
    pub fn from_draw_list(
        draw_list: &DrawList,
        target_width: u32,
        target_height: u32,
        damage: DamageSet,
    ) -> Self {
        let mut scene = Self::with_damage(target_width, target_height, damage);
        let mut transforms = Vec::new();
        let mut transform = Mat3::identity();
        let mut opacities = Vec::new();
        let mut opacity = 1.0;

        let mut command_segment_start = 0;
        for (command_index, command) in draw_list.commands().iter().enumerate() {
            match command {
                DrawCommand::PushTransform { matrix } => {
                    transforms.push(transform);
                    transform = matrix.pixel_aligned();
                    opacities.push(opacity);
                }
                DrawCommand::PopTransform => {
                    transform = transforms.pop().unwrap_or_default();
                    opacity = opacities.pop().unwrap_or(1.0);
                }
                DrawCommand::SetTransform { matrix } => {
                    transform = matrix.pixel_aligned();
                }
                DrawCommand::SetAlpha { alpha } => {
                    opacity = alpha.clamp(0.0, 1.0);
                }
                DrawCommand::RestoreAlpha => {
                    opacity = 1.0;
                }
                DrawCommand::RetainedLayer { layer_id, rect, .. } => {
                    if command_segment_start < command_index {
                        scene.items.push(SceneItem::Commands {
                            start: command_segment_start,
                            end: command_index,
                        });
                    }
                    let mut properties = SurfaceProperties::new(*rect, scene.surfaces.len() as u32);
                    properties.transform = transform;
                    properties.opacity = opacity;
                    let surface =
                        SurfaceNode::retained_layer(SurfaceId::from_raw(*layer_id), properties);
                    scene.surfaces.push(surface);
                    scene.items.push(SceneItem::Surface(surface));
                    command_segment_start = command_index + 1;
                }
                _ => {}
            }
        }
        // The draw list remains the authoritative ordinary-content stream. A
        // scene with no promotion does not need to allocate a mirror segment;
        // this is the common path for ordinary frames.
        if !scene.surfaces.is_empty() && command_segment_start < draw_list.commands().len() {
            scene.items.push(SceneItem::Commands {
                start: command_segment_start,
                end: draw_list.commands().len(),
            });
        }
        scene.render_items = scene.items.clone();
        scene
    }

    /// Appends an explicitly promoted surface in paint order.
    #[inline]
    pub fn push_surface(&mut self, surface: SurfaceNode) {
        self.surfaces.push(surface);
        self.items.push(SceneItem::Surface(surface));
        self.render_items.push(SceneItem::Surface(surface));
    }

    /// Returns the target dimensions captured by this scene.
    #[inline]
    pub const fn target_size(&self) -> (u32, u32) {
        (self.target_width, self.target_height)
    }

    /// Returns the damage contract captured by this scene.
    #[inline]
    pub fn damage(&self) -> &DamageSet {
        &self.damage
    }

    /// Returns surfaces in their original paint order.
    #[inline]
    pub fn surfaces(&self) -> &[SurfaceNode] {
        &self.surfaces
    }

    /// Returns all scene items in paint order, including ordinary command
    /// segments and promoted surfaces.
    #[inline]
    pub fn items(&self) -> &[SceneItem] {
        &self.items
    }

    /// Returns the command/surface stream used by a recorded scene.
    ///
    /// Unlike [`Self::items`], this stream contains ordinary commands even
    /// when no retained surface exists. It is intentionally read-only: the
    /// DrawList remains the payload store and this sequence only decides which
    /// payload ranges the renderer visits.
    #[inline]
    pub fn render_items(&self) -> &[SceneItem] {
        &self.render_items
    }

    /// Returns whether this scene came from the Element paint recorder.
    #[inline]
    pub const fn is_recorded(&self) -> bool {
        self.recorded
    }

    /// Returns the logical scene roots in paint order.
    #[inline]
    pub fn roots(&self) -> &[SceneNodeId] {
        &self.roots
    }

    /// Returns all logical nodes in stable recorder order.
    #[inline]
    pub fn nodes(&self) -> &[SceneNode] {
        &self.nodes
    }

    /// Looks up one logical node by its stable identity.
    #[inline]
    pub fn node(&self, id: SceneNodeId) -> Option<&SceneNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Returns whether this frame promoted at least one surface.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// Returns the total number of device pixels covered by known damage.
    pub fn damaged_area(&self) -> u64 {
        if self.damage.is_full() {
            return u64::from(self.target_width) * u64::from(self.target_height);
        }
        self.damage
            .regions()
            .iter()
            .map(|region: &DamageRect| u64::from(region.width) * u64::from(region.height))
            .sum()
    }

    /// Compares this scene with the last accepted scene for the same target.
    ///
    /// A missing or incompatible previous scene is treated as a full repaint;
    /// this is the safe first-frame and context-reset policy used by the
    /// renderer.
    pub fn diff(&self, previous: Option<&Self>) -> SceneDiff {
        let mut damage = DamageSet::new(self.target_width, self.target_height);
        let Some(previous) = previous.filter(|previous| {
            previous.target_size() == self.target_size() && previous.recorded == self.recorded
        }) else {
            damage.mark_full();
            return SceneDiff {
                changes: self
                    .nodes
                    .iter()
                    .map(|node| SceneChange {
                        id: node.id,
                        kind: SceneChangeKind::Added,
                        old_bounds: None,
                        new_bounds: node.effective_bounds,
                    })
                    .collect(),
                damage,
                full: true,
            };
        };

        let mut changes = Vec::new();
        let mut previous_by_id = std::collections::HashMap::with_capacity(previous.nodes.len());
        for node in &previous.nodes {
            previous_by_id.insert(node.id, node);
        }
        let mut current_ids = std::collections::HashSet::with_capacity(self.nodes.len());

        for node in &self.nodes {
            current_ids.insert(node.id);
            let Some(old) = previous_by_id.get(&node.id).copied() else {
                push_scene_change(
                    &mut changes,
                    &mut damage,
                    SceneChange {
                        id: node.id,
                        kind: SceneChangeKind::Added,
                        old_bounds: None,
                        new_bounds: node.effective_bounds,
                    },
                );
                continue;
            };

            let kind = scene_change_kind(old, node, &previous.roots, &self.roots);

            if let Some(kind) = kind {
                push_scene_change(
                    &mut changes,
                    &mut damage,
                    SceneChange {
                        id: node.id,
                        kind,
                        old_bounds: old.effective_bounds,
                        new_bounds: node.effective_bounds,
                    },
                );
            }
        }

        for old in &previous.nodes {
            if !current_ids.contains(&old.id) && old.presence != ScenePresence::Culled {
                push_scene_change(
                    &mut changes,
                    &mut damage,
                    SceneChange {
                        id: old.id,
                        kind: SceneChangeKind::Removed,
                        old_bounds: old.effective_bounds,
                        new_bounds: None,
                    },
                );
            }
        }

        changes.sort_unstable_by_key(|change| change.id.get());
        SceneDiff {
            full: damage.is_full(),
            changes,
            damage,
        }
    }
}

fn scene_change_kind(
    old: &SceneNode,
    current: &SceneNode,
    previous_roots: &[SceneNodeId],
    current_roots: &[SceneNodeId],
) -> Option<SceneChangeKind> {
    if old.revision != current.revision || old.content != current.content {
        Some(SceneChangeKind::Content)
    } else if old.bounds != current.bounds
        || old.frame_bounds != current.frame_bounds
        || old.transform != current.transform
    {
        Some(SceneChangeKind::Geometry)
    } else if old.clip_chain != current.clip_chain {
        Some(SceneChangeKind::Clip)
    } else if old.opacity.to_bits() != current.opacity.to_bits() {
        Some(SceneChangeKind::Opacity)
    } else if old.parent != current.parent
        || old.order != current.order
        || old.children != current.children
        || root_position(previous_roots, old.id) != root_position(current_roots, current.id)
    {
        Some(SceneChangeKind::Order)
    } else if old.presence != current.presence {
        Some(SceneChangeKind::Geometry)
    } else {
        None
    }
}

fn push_scene_change(changes: &mut Vec<SceneChange>, damage: &mut DamageSet, change: SceneChange) {
    match change.old_bounds {
        Some(bounds) => add_frame_damage(damage, bounds),
        None if change.kind != SceneChangeKind::Added => damage.mark_full(),
        None => {}
    }
    match change.new_bounds {
        Some(bounds) => add_frame_damage(damage, bounds),
        None if change.kind != SceneChangeKind::Removed => damage.mark_full(),
        None => {}
    }
    changes.push(change);
}

fn add_frame_damage(damage: &mut DamageSet, bounds: Rect) {
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width < 0.0
        || bounds.height < 0.0
    {
        damage.mark_full();
        return;
    }
    if bounds.width == 0.0 || bounds.height == 0.0 {
        return;
    }
    let x = bounds.x.floor().max(0.0);
    let y = bounds.y.floor().max(0.0);
    let right = (bounds.x + bounds.width).ceil().max(x);
    let bottom = (bounds.y + bounds.height).ceil().max(y);
    if !right.is_finite() || !bottom.is_finite() {
        damage.mark_full();
        return;
    }
    damage.add(DamageRect::new(
        x as u32,
        y as u32,
        (right - x) as u32,
        (bottom - y) as u32,
    ));
}

fn root_position(roots: &[SceneNodeId], id: SceneNodeId) -> Option<usize> {
    roots.iter().position(|root| *root == id)
}

fn intersect_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    Rect::new(x, y, (right_edge - x).max(0.0), (bottom_edge - y).max(0.0))
}

fn effective_node_bounds(frame_bounds: Option<Rect>, clips: &ClipChain) -> Option<Rect> {
    let frame_bounds = frame_bounds?;
    if clips.clips().is_empty() {
        return Some(frame_bounds);
    }
    clips
        .effective_bounds()
        .map(|clip| intersect_rect(frame_bounds, clip))
}

fn transformed_rect(transform: Mat3, rect: Rect) -> Option<Rect> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width < 0.0
        || rect.height < 0.0
    {
        return None;
    }
    let corners = [
        transform.transform_point(rect.x, rect.y),
        transform.transform_point(rect.x + rect.width, rect.y),
        transform.transform_point(rect.x, rect.y + rect.height),
        transform.transform_point(rect.x + rect.width, rect.y + rect.height),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
}

#[derive(Clone, Debug)]
struct SceneState {
    transform: Mat3,
    opacity: f32,
    clips: Vec<SceneClip>,
}

impl Default for SceneState {
    fn default() -> Self {
        Self {
            transform: Mat3::identity(),
            opacity: 1.0,
            clips: Vec::new(),
        }
    }
}

impl SceneState {
    fn clip(&mut self, rect: Rect, border_radius: [f32; 4]) {
        let frame_bounds = transformed_rect(self.transform, rect);
        let effective_bounds = match frame_bounds {
            None => None,
            Some(frame_bounds) if self.clips.is_empty() => Some(frame_bounds),
            Some(frame_bounds) => self
                .clips
                .last()
                .and_then(|clip| clip.effective_bounds)
                .map(|parent| intersect_rect(frame_bounds, parent)),
        };
        self.clips.push(SceneClip {
            rect,
            border_radius,
            transform: self.transform,
            frame_bounds,
            effective_bounds,
        });
    }

    fn snapshot(&self) -> SceneStateSnapshot {
        SceneStateSnapshot {
            transform: self.transform,
            opacity: self.opacity,
            clip_chain: ClipChain {
                clips: self.clips.clone(),
            },
        }
    }
}

#[derive(Clone, Debug)]
struct SceneStateSnapshot {
    transform: Mat3,
    opacity: f32,
    clip_chain: ClipChain,
}

struct NodeDraft {
    descriptor: SceneNodeDescriptor,
    parent: Option<usize>,
    children: Vec<usize>,
    start: usize,
    end: Option<usize>,
    state: Option<SceneStateSnapshot>,
}

/// Records logical Element scopes while the existing DrawList is painted.
///
/// The recorder owns only small frame-local metadata. It does not retain
/// widget references and can therefore be finished before the DrawList moves
/// to the raster thread.
#[doc(hidden)]
pub struct SceneRecorder {
    nodes: Vec<NodeDraft>,
    stack: Vec<usize>,
    invalid: bool,
}

impl SceneRecorder {
    /// Creates an empty frame recorder.
    #[inline]
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            stack: Vec::new(),
            invalid: false,
        }
    }

    /// Returns whether no logical node was opened in this frame.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Opens one logical node at the current DrawList command cursor.
    #[doc(hidden)]
    pub fn begin_node(&mut self, descriptor: SceneNodeDescriptor, command_start: usize) -> usize {
        let index = self.nodes.len();
        let parent = self.stack.last().copied();
        self.nodes.push(NodeDraft {
            descriptor,
            parent,
            children: Vec::new(),
            start: command_start,
            end: None,
            state: None,
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        }
        self.stack.push(index);
        index
    }

    /// Closes a node at the current DrawList command cursor.
    #[doc(hidden)]
    pub fn end_node(&mut self, index: usize, command_end: usize) {
        if self.stack.pop() != Some(index) {
            self.invalid = true;
            self.stack.retain(|open| *open != index);
        }
        if let Some(node) = self.nodes.get_mut(index) {
            node.end = Some(command_end);
        } else {
            self.invalid = true;
        }
    }

    /// Finishes the recorder and lowers its scopes into an immutable scene.
    pub fn finish(
        mut self,
        draw_list: &DrawList,
        target_width: u32,
        target_height: u32,
        damage: DamageSet,
    ) -> CompositorScene {
        let command_count = draw_list.commands().len();
        if !self.stack.is_empty() {
            self.invalid = true;
        }
        for node in &mut self.nodes {
            if node.end.is_none() {
                node.end = Some(command_count);
                self.invalid = true;
            }
        }

        let snapshots = state_snapshots(&self.nodes, draw_list);
        for (node, snapshot) in self.nodes.iter_mut().zip(snapshots) {
            node.state = Some(snapshot);
        }

        let mut scene = CompositorScene::with_damage(target_width, target_height, damage);
        let (surfaces, items) = render_items_from_draw_list(draw_list);
        scene.surfaces = surfaces;
        scene.items = items.clone();
        scene.render_items = render_items_for_recorded_scene(draw_list, &items);
        scene.recorded = !self.invalid;

        let mut owners = vec![None; command_count];
        let mut ordered_nodes = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.start,
                    Reverse(node.end.unwrap_or(command_count)),
                    index,
                )
            })
            .collect::<Vec<_>>();
        ordered_nodes.sort_unstable();
        let mut open = Vec::new();
        let mut cursor = 0;
        for command_index in 0..command_count {
            while cursor < ordered_nodes.len() && ordered_nodes[cursor].0 == command_index {
                open.push(ordered_nodes[cursor].2);
                cursor += 1;
            }
            while open.last().is_some_and(|index| {
                self.nodes[*index].end.unwrap_or(command_count) <= command_index
            }) {
                open.pop();
            }
            owners[command_index] = open.last().copied();
        }

        let mut surface_by_command = std::collections::HashMap::new();
        for item in &items {
            if let SceneItem::Surface(surface) = item {
                // The retained-layer id is unique within a normal frame. The
                // command index is filled by the helper below when available.
                surface_by_command.insert(surface.id(), *surface);
            }
        }

        let mut node_ids = Vec::with_capacity(self.nodes.len());
        for draft in &self.nodes {
            node_ids.push(draft.descriptor.id);
        }
        for (index, draft) in self.nodes.iter().enumerate() {
            let snapshot = draft
                .state
                .clone()
                .unwrap_or_else(|| SceneState::default().snapshot());
            let frame_bounds = transformed_rect(snapshot.transform, draft.descriptor.bounds);
            let effective_bounds = effective_node_bounds(frame_bounds, &snapshot.clip_chain);
            let parent = draft.parent.map(|parent| self.nodes[parent].descriptor.id);
            let mut children = Vec::with_capacity(draft.children.len());
            for child in &draft.children {
                children.push(self.nodes[*child].descriptor.id);
            }

            let mut direct_indices = owners
                .iter()
                .enumerate()
                .filter_map(|(command_index, owner)| {
                    (*owner == Some(index)).then_some(command_index)
                })
                .collect::<Vec<_>>();
            let mut paint_events = Vec::new();
            for range in contiguous_ranges(&mut direct_indices) {
                for (start, op) in direct_paint_ops(draw_list, range, &surface_by_command) {
                    paint_events.push((start, op));
                }
            }
            for child in &draft.children {
                let child_start = self.nodes[*child].start;
                paint_events.push((child_start, ScenePaintOp::Child(node_ids[*child])));
            }
            paint_events.sort_unstable_by_key(|(start, _)| *start);
            let paint_ops = paint_events
                .into_iter()
                .map(|(_, op)| op)
                .collect::<Vec<_>>();

            let cached_surface = direct_indices.iter().find_map(|command_index| {
                match draw_list.commands().get(*command_index) {
                    Some(DrawCommand::RetainedLayer { layer_id, .. }) => surface_by_command
                        .get(&SurfaceId::from_raw(*layer_id))
                        .copied(),
                    _ => None,
                }
            });
            let content = cached_surface
                .map(|surface| SceneContent::CachedSurface(surface.id()))
                .unwrap_or(SceneContent::Live);
            scene.nodes.push(SceneNode {
                id: draft.descriptor.id,
                parent,
                bounds: draft.descriptor.bounds,
                frame_bounds,
                effective_bounds,
                transform: snapshot.transform,
                opacity: snapshot.opacity,
                order: draft.descriptor.order,
                revision: draft.descriptor.revision,
                presence: ScenePresence::Visible,
                content,
                cache_eligible: draft.descriptor.cache_eligible,
                bounded: draft.descriptor.bounded,
                priority: draft.descriptor.priority,
                clip_chain: snapshot.clip_chain,
                children,
                paint_ops,
            });
            if parent.is_none() {
                scene.roots.push(draft.descriptor.id);
            }
        }

        scene
    }
}

impl Default for SceneRecorder {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// RAII scope returned by the canvas scene seam.
#[doc(hidden)]
pub struct SceneNodeGuard {
    recorder: Option<Rc<RefCell<Option<SceneRecorder>>>>,
    draw_list: Option<Rc<RefCell<DrawList>>>,
    index: usize,
}

impl SceneNodeGuard {
    pub(crate) fn active(
        recorder: Rc<RefCell<Option<SceneRecorder>>>,
        draw_list: Rc<RefCell<DrawList>>,
        index: usize,
    ) -> Self {
        Self {
            recorder: Some(recorder),
            draw_list: Some(draw_list),
            index,
        }
    }

    pub(crate) fn inactive() -> Self {
        Self {
            recorder: None,
            draw_list: None,
            index: 0,
        }
    }
}

impl Drop for SceneNodeGuard {
    fn drop(&mut self) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        let mut recorder = recorder.borrow_mut();
        let Some(recorder) = recorder.as_mut() else {
            return;
        };
        let end = self
            .draw_list
            .as_ref()
            .map(|draw_list| draw_list.borrow().commands().len())
            .unwrap_or(0);
        recorder.end_node(self.index, end);
    }
}

fn state_snapshots(nodes: &[NodeDraft], draw_list: &DrawList) -> Vec<SceneStateSnapshot> {
    let mut starts = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            (
                node.start,
                Reverse(node.end.unwrap_or(draw_list.commands().len())),
                index,
            )
        })
        .collect::<Vec<_>>();
    starts.sort_unstable();
    let mut snapshots = vec![None; nodes.len()];
    let mut transform_stack = Vec::new();
    let mut opacity_stack = Vec::new();
    let mut state = SceneState::default();
    let mut cursor = 0;
    for command_index in 0..=draw_list.commands().len() {
        while cursor < starts.len() && starts[cursor].0 == command_index {
            snapshots[starts[cursor].2] = Some(state.snapshot());
            cursor += 1;
        }
        let Some(command) = draw_list.commands().get(command_index) else {
            continue;
        };
        match command {
            DrawCommand::PushTransform { matrix } => {
                transform_stack.push(state.transform);
                opacity_stack.push(state.opacity);
                state.transform = matrix.pixel_aligned();
            }
            DrawCommand::PopTransform => {
                state.transform = transform_stack.pop().unwrap_or_default();
                state.opacity = opacity_stack.pop().unwrap_or(1.0);
            }
            DrawCommand::SetTransform { matrix } => state.transform = matrix.pixel_aligned(),
            DrawCommand::PushClip {
                rect,
                border_radius,
            } => state.clip(*rect, *border_radius),
            DrawCommand::PopClip => {
                state.clips.pop();
            }
            DrawCommand::SetAlpha { alpha } => state.opacity = alpha.clamp(0.0, 1.0),
            DrawCommand::RestoreAlpha => state.opacity = 1.0,
            _ => {}
        }
    }
    snapshots
        .into_iter()
        .map(|snapshot| snapshot.unwrap_or_else(|| SceneState::default().snapshot()))
        .collect()
}

fn contiguous_ranges(indices: &mut [usize]) -> Vec<(usize, usize)> {
    if indices.is_empty() {
        return Vec::new();
    }
    indices.sort_unstable();
    let mut ranges = Vec::new();
    let mut start = indices[0];
    let mut end = start + 1;
    for index in &indices[1..] {
        if *index == end {
            end += 1;
        } else {
            ranges.push((start, end));
            start = *index;
            end = start + 1;
        }
    }
    ranges.push((start, end));
    ranges
}

fn direct_paint_ops(
    draw_list: &DrawList,
    range: (usize, usize),
    surfaces: &std::collections::HashMap<SurfaceId, SurfaceNode>,
) -> Vec<(usize, ScenePaintOp)> {
    let mut operations = Vec::new();
    let mut command_start = range.0;
    for index in range.0..range.1 {
        let Some(DrawCommand::RetainedLayer { layer_id, .. }) = draw_list.commands().get(index)
        else {
            continue;
        };
        if command_start < index {
            operations.push((
                command_start,
                ScenePaintOp::Commands {
                    start: command_start,
                    end: index,
                },
            ));
        }
        if let Some(surface) = surfaces.get(&SurfaceId::from_raw(*layer_id)).copied() {
            operations.push((index, ScenePaintOp::Surface(surface)));
        }
        command_start = index + 1;
    }
    if command_start < range.1 {
        operations.push((
            command_start,
            ScenePaintOp::Commands {
                start: command_start,
                end: range.1,
            },
        ));
    }
    operations
}

fn render_items_from_draw_list(draw_list: &DrawList) -> (Vec<SurfaceNode>, Vec<SceneItem>) {
    let mut surfaces = Vec::new();
    let mut items = Vec::new();
    let mut transforms = Vec::new();
    let mut transform = Mat3::identity();
    let mut opacities = Vec::new();
    let mut opacity = 1.0;
    let mut command_start = 0;
    for (command_index, command) in draw_list.commands().iter().enumerate() {
        match command {
            DrawCommand::PushTransform { matrix } => {
                transforms.push(transform);
                opacities.push(opacity);
                transform = matrix.pixel_aligned();
            }
            DrawCommand::PopTransform => {
                transform = transforms.pop().unwrap_or_default();
                opacity = opacities.pop().unwrap_or(1.0);
            }
            DrawCommand::SetTransform { matrix } => transform = matrix.pixel_aligned(),
            DrawCommand::SetAlpha { alpha } => opacity = alpha.clamp(0.0, 1.0),
            DrawCommand::RestoreAlpha => opacity = 1.0,
            DrawCommand::RetainedLayer { layer_id, rect, .. } => {
                if command_start < command_index {
                    items.push(SceneItem::Commands {
                        start: command_start,
                        end: command_index,
                    });
                }
                let mut properties = SurfaceProperties::new(*rect, surfaces.len() as u32);
                properties.transform = transform;
                properties.opacity = opacity;
                let surface =
                    SurfaceNode::retained_layer(SurfaceId::from_raw(*layer_id), properties);
                surfaces.push(surface);
                items.push(SceneItem::Surface(surface));
                command_start = command_index + 1;
            }
            _ => {}
        }
    }
    if !surfaces.is_empty() && command_start < draw_list.commands().len() {
        items.push(SceneItem::Commands {
            start: command_start,
            end: draw_list.commands().len(),
        });
    }
    (surfaces, items)
}

fn render_items_for_recorded_scene(draw_list: &DrawList, items: &[SceneItem]) -> Vec<SceneItem> {
    if items.is_empty() {
        return if draw_list.commands().is_empty() {
            Vec::new()
        } else {
            vec![SceneItem::Commands {
                start: 0,
                end: draw_list.commands().len(),
            }]
        };
    }
    items.to_vec()
}

/// Iterates the command payload according to the scene's authoritative paint
/// stream. Retained-layer commands are replaced by their scene surface item.
#[doc(hidden)]
pub(crate) struct SceneRenderIter<'a> {
    draw_list: &'a DrawList,
    items: Option<&'a [SceneItem]>,
    damage: Option<DamageRect>,
    item_index: usize,
    command_index: usize,
    command_end: usize,
}

impl<'a> SceneRenderIter<'a> {
    pub(crate) fn new(draw_list: &'a DrawList, scene: Option<&'a CompositorScene>) -> Self {
        Self::with_damage(draw_list, scene, None)
    }

    pub(crate) fn new_for_damage(
        draw_list: &'a DrawList,
        scene: Option<&'a CompositorScene>,
        damage: DamageRect,
    ) -> Self {
        Self::with_damage(draw_list, scene, Some(damage))
    }

    fn with_damage(
        draw_list: &'a DrawList,
        scene: Option<&'a CompositorScene>,
        damage: Option<DamageRect>,
    ) -> Self {
        let items = scene
            .filter(|scene| scene.render_items_cover(draw_list))
            .map(|scene| scene.render_items.as_slice());
        Self {
            draw_list,
            items,
            damage,
            item_index: 0,
            command_index: 0,
            command_end: 0,
        }
    }
}

impl<'a> Iterator for SceneRenderIter<'a> {
    type Item = SceneRenderItem<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.items.is_none() {
            let command = self.draw_list.commands().get(self.command_index)?;
            self.command_index += 1;
            return Some(SceneRenderItem::Command(command));
        }
        loop {
            if self.command_index < self.command_end {
                let command = self.draw_list.commands().get(self.command_index)?;
                self.command_index += 1;
                return Some(SceneRenderItem::Command(command));
            }
            let item = self.items?.get(self.item_index)?;
            self.item_index += 1;
            match item {
                SceneItem::Commands { start, end } => {
                    self.command_index = *start;
                    self.command_end = (*end).min(self.draw_list.commands().len());
                }
                SceneItem::Surface(surface) => {
                    if self
                        .damage
                        .is_none_or(|damage| surface_intersects_damage(*surface, damage))
                    {
                        return Some(SceneRenderItem::Surface(*surface));
                    }
                }
            }
        }
    }
}

fn surface_intersects_damage(surface: SurfaceNode, damage: DamageRect) -> bool {
    let Some(bounds) = surface.properties().frame_bounds() else {
        return true;
    };
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width < 0.0
        || bounds.height < 0.0
    {
        return true;
    }
    let damage_x = damage.x as f32;
    let damage_y = damage.y as f32;
    let damage_right = damage_x + damage.width as f32;
    let damage_bottom = damage_y + damage.height as f32;
    bounds.x < damage_right
        && damage_x < bounds.x + bounds.width
        && bounds.y < damage_bottom
        && damage_y < bounds.y + bounds.height
}

impl CompositorScene {
    /// Whether the scene-led stream accounts for every draw-list command.
    ///
    /// Public packet construction is intentionally allowed to attach a scene
    /// built by another producer. If that producer supplies only a surface
    /// item for a draw list that also contains live commands, the renderer
    /// must fall back to the complete DrawList rather than silently dropping
    /// those commands.
    fn render_items_cover(&self, draw_list: &DrawList) -> bool {
        let mut cursor = 0;
        for item in &self.render_items {
            match item {
                SceneItem::Commands { start, end } => {
                    if *start != cursor || *start > *end || *end > draw_list.commands().len() {
                        return false;
                    }
                    cursor = *end;
                }
                SceneItem::Surface(surface) => {
                    let Some(DrawCommand::RetainedLayer { layer_id, .. }) =
                        draw_list.commands().get(cursor)
                    else {
                        return false;
                    };
                    if SurfaceId::from_raw(*layer_id) != surface.id() {
                        return false;
                    }
                    cursor += 1;
                }
            }
        }
        cursor == draw_list.commands().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw_cmd::DrawList;

    #[test]
    fn scene_preserves_retained_layer_order_and_properties() {
        let mut draw = DrawList::new();
        draw.push(DrawCommand::RetainedLayer {
            layer_id: 7,
            rect: Rect::new(1.0, 2.0, 10.0, 11.0),
            content: std::sync::Arc::new(RetainedLayerContentForTest::content()),
        });
        draw.push(DrawCommand::SetAlpha { alpha: 0.5 });
        draw.push(DrawCommand::RetainedLayer {
            layer_id: 8,
            rect: Rect::new(3.0, 4.0, 5.0, 6.0),
            content: std::sync::Arc::new(RetainedLayerContentForTest::content()),
        });

        let scene = CompositorScene::from_draw_list(&draw, 32, 24, DamageSet::full(32, 24));

        assert_eq!(scene.surfaces().len(), 2);
        assert_eq!(
            scene.items(),
            &[
                SceneItem::Surface(scene.surfaces()[0]),
                SceneItem::Commands { start: 1, end: 2 },
                SceneItem::Surface(scene.surfaces()[1]),
            ]
        );
        assert_eq!(scene.surfaces()[0].id().get(), 7);
        assert_eq!(scene.surfaces()[1].id().get(), 8);
        assert_eq!(scene.surfaces()[1].properties().opacity, 0.5);
    }

    #[test]
    fn ordinary_scene_does_not_mirror_the_draw_list() {
        let mut draw = DrawList::new();
        draw.clear_rect(Rect::new(0.0, 0.0, 4.0, 4.0));

        let scene = CompositorScene::from_draw_list(&draw, 8, 8, DamageSet::full(8, 8));

        assert!(scene.is_empty());
        assert!(scene.items().is_empty());
    }

    #[test]
    fn incomplete_scene_stream_falls_back_to_the_complete_draw_list() {
        let mut draw = DrawList::new();
        draw.clear_rect(Rect::new(0.0, 0.0, 4.0, 4.0));
        let mut scene = CompositorScene::new(8, 8);
        scene.push_surface(SurfaceNode::retained_layer(
            SurfaceId::from_raw(9),
            SurfaceProperties::new(Rect::new(0.0, 0.0, 4.0, 4.0), 0),
        ));

        let mut iter = SceneRenderIter::new(&draw, Some(&scene));
        assert!(matches!(iter.next(), Some(SceneRenderItem::Command(_))));
        assert!(iter.next().is_none());
    }

    #[test]
    fn damage_render_stream_composes_only_intersecting_surfaces() {
        let mut draw = DrawList::new();
        draw.push(DrawCommand::RetainedLayer {
            layer_id: 1,
            rect: Rect::new(0.0, 0.0, 16.0, 16.0),
            content: std::sync::Arc::new(RetainedLayerContentForTest::content()),
        });
        draw.push(DrawCommand::RetainedLayer {
            layer_id: 2,
            rect: Rect::new(100.0, 0.0, 16.0, 16.0),
            content: std::sync::Arc::new(RetainedLayerContentForTest::content()),
        });
        let scene = CompositorScene::from_draw_list(&draw, 128, 32, DamageSet::new(128, 32));

        let mut iter =
            SceneRenderIter::new_for_damage(&draw, Some(&scene), DamageRect::new(0, 0, 32, 32));
        let surfaces = std::iter::from_fn(|| iter.next())
            .filter_map(|item| match item {
                SceneRenderItem::Surface(surface) => Some(surface.id().get()),
                SceneRenderItem::Command(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(surfaces, vec![1]);
    }

    #[test]
    fn recorded_scene_keeps_nested_order_transform_and_clip_chain() {
        let mut draw = DrawList::new();
        draw.push(DrawCommand::PushTransform {
            matrix: Mat3::translate(10.0, 20.0),
        });
        draw.push(DrawCommand::PushClip {
            rect: Rect::new(0.0, 0.0, 30.0, 30.0),
            border_radius: [2.0; 4],
        });

        let mut recorder = SceneRecorder::new();
        let root = recorder.begin_node(
            SceneNodeDescriptor {
                id: SceneNodeId::from_raw(1),
                bounds: Rect::new(0.0, 0.0, 30.0, 30.0),
                order: 0,
                revision: SceneRevision::new(1, 0, 0, 0),
                cache_eligible: false,
                bounded: true,
                priority: false,
            },
            draw.commands().len(),
        );
        draw.fill_rect(
            Rect::new(0.0, 0.0, 30.0, 30.0),
            crate::utilities::Color::black(),
            [0.0; 4],
            [0.0; 4],
            crate::utilities::Color::transparent(),
        );
        let child = recorder.begin_node(
            SceneNodeDescriptor {
                id: SceneNodeId::from_raw(2),
                bounds: Rect::new(1.0, 2.0, 4.0, 5.0),
                order: 3,
                revision: SceneRevision::new(2, 0, 0, 0),
                cache_eligible: true,
                bounded: true,
                priority: false,
            },
            draw.commands().len(),
        );
        draw.clear_rect(Rect::new(1.0, 2.0, 4.0, 5.0));
        recorder.end_node(child, draw.commands().len());
        draw.push(DrawCommand::PopClip);
        draw.push(DrawCommand::PopTransform);
        recorder.end_node(root, draw.commands().len());

        let scene = recorder.finish(&draw, 64, 64, DamageSet::full(64, 64));

        assert!(scene.is_recorded());
        assert_eq!(scene.roots(), &[SceneNodeId::from_raw(1)]);
        let root = scene.node(SceneNodeId::from_raw(1)).unwrap();
        assert_eq!(root.children(), &[SceneNodeId::from_raw(2)]);
        assert_eq!(root.clip_chain().clips().len(), 1);
        assert_eq!(root.transform(), Mat3::translate(10.0, 20.0));
        assert_eq!(root.paint_ops().len(), 3);
        let child = scene.node(SceneNodeId::from_raw(2)).unwrap();
        assert_eq!(child.parent(), Some(SceneNodeId::from_raw(1)));
        assert_eq!(child.clip_chain().clips().len(), 1);
        assert_eq!(child.frame_bounds(), Some(Rect::new(11.0, 22.0, 4.0, 5.0)));
        assert_eq!(
            child.effective_bounds(),
            Some(Rect::new(11.0, 22.0, 4.0, 5.0))
        );
    }

    #[test]
    fn scene_diff_damages_old_and_new_geometry_without_command_hashing() {
        fn scene_at(x: f32, revision: u64) -> CompositorScene {
            let mut draw = DrawList::new();
            draw.push(DrawCommand::SetTransform {
                matrix: Mat3::translate(x, 0.0),
            });
            let mut recorder = SceneRecorder::new();
            let node = recorder.begin_node(
                SceneNodeDescriptor {
                    id: SceneNodeId::from_raw(7),
                    bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
                    order: 0,
                    revision: SceneRevision::new(revision, 0, 0, 0),
                    cache_eligible: true,
                    bounded: true,
                    priority: false,
                },
                draw.commands().len(),
            );
            recorder.end_node(node, draw.commands().len());
            recorder.finish(&draw, 64, 64, DamageSet::new(64, 64))
        }

        let old = scene_at(0.0, 1);
        let current = scene_at(5.0, 2);
        let diff = current.diff(Some(&old));

        assert_eq!(diff.changes().len(), 1);
        assert_eq!(diff.changes()[0].kind, SceneChangeKind::Content);
        assert_eq!(diff.damage().regions().len(), 1);
        assert_eq!(diff.damage().regions()[0], DamageRect::new(0, 0, 15, 10));
    }

    #[test]
    fn scene_diff_tracks_clip_opacity_and_sibling_order_changes() {
        fn scene_at(order: [u64; 2], clip_x: f32, opacity: f32) -> CompositorScene {
            let mut draw = DrawList::new();
            draw.push(DrawCommand::PushClip {
                rect: Rect::new(clip_x, 0.0, 12.0, 10.0),
                border_radius: [0.0; 4],
            });
            draw.push(DrawCommand::SetAlpha { alpha: opacity });
            let mut recorder = SceneRecorder::new();
            let root = recorder.begin_node(
                SceneNodeDescriptor::new(
                    SceneNodeId::from_raw(99),
                    Rect::new(0.0, 0.0, 20.0, 10.0),
                    0,
                ),
                draw.commands().len(),
            );
            for id in order {
                let child = recorder.begin_node(
                    SceneNodeDescriptor::new(
                        SceneNodeId::from_raw(id),
                        Rect::new(0.0, 0.0, 8.0, 10.0),
                        0,
                    ),
                    draw.commands().len(),
                );
                draw.clear_rect(Rect::new(0.0, 0.0, 8.0, 10.0));
                recorder.end_node(child, draw.commands().len());
            }
            recorder.end_node(root, draw.commands().len());
            draw.push(DrawCommand::PopClip);
            recorder.finish(&draw, 32, 24, DamageSet::new(32, 24))
        }

        let old = scene_at([1, 2], 0.0, 1.0);
        let order_diff = scene_at([2, 1], 0.0, 1.0).diff(Some(&old));

        assert!(
            order_diff
                .changes()
                .iter()
                .any(|change| change.id == SceneNodeId::from_raw(99)
                    && change.kind == SceneChangeKind::Order)
        );

        let clip_diff = scene_at([1, 2], 4.0, 1.0).diff(Some(&old));
        assert!(
            clip_diff
                .changes()
                .iter()
                .any(|change| change.id == SceneNodeId::from_raw(99)
                    && change.kind == SceneChangeKind::Clip)
        );

        let opacity_diff = scene_at([1, 2], 0.0, 0.5).diff(Some(&old));
        assert!(
            opacity_diff
                .changes()
                .iter()
                .any(|change| change.id == SceneNodeId::from_raw(99)
                    && change.kind == SceneChangeKind::Opacity)
        );
        assert!(!order_diff.damage().is_empty());
        assert!(!clip_diff.damage().is_empty());
        assert!(!opacity_diff.damage().is_empty());
    }

    #[test]
    fn retained_scene_tree_reuses_unchanged_nodes_and_updates_changed_nodes() {
        fn scene_at(revisions: [u64; 2]) -> CompositorScene {
            let mut draw = DrawList::new();
            let mut recorder = SceneRecorder::new();
            let root = recorder.begin_node(
                SceneNodeDescriptor {
                    id: SceneNodeId::from_raw(1),
                    bounds: Rect::new(0.0, 0.0, 20.0, 20.0),
                    order: 0,
                    revision: SceneRevision::new(revisions[0], 0, 0, 0),
                    cache_eligible: true,
                    bounded: true,
                    priority: false,
                },
                draw.commands().len(),
            );
            let child = recorder.begin_node(
                SceneNodeDescriptor {
                    id: SceneNodeId::from_raw(2),
                    bounds: Rect::new(2.0, 2.0, 8.0, 8.0),
                    order: 0,
                    revision: SceneRevision::new(revisions[1], 0, 0, 0),
                    cache_eligible: true,
                    bounded: true,
                    priority: false,
                },
                draw.commands().len(),
            );
            draw.clear_rect(Rect::new(2.0, 2.0, 8.0, 8.0));
            recorder.end_node(child, draw.commands().len());
            recorder.end_node(root, draw.commands().len());
            recorder.finish(&draw, 32, 32, DamageSet::new(32, 32))
        }

        let first = scene_at([1, 1]);
        let changed = scene_at([1, 2]);
        let mut tree = RetainedSceneTree::new();

        let initial = tree.commit(&first);
        assert_eq!(initial.updated_nodes(), 2);
        assert_eq!(initial.reused_nodes(), 0);

        let warm = tree.commit(&first);
        assert!(warm.diff().is_empty());
        assert_eq!(warm.updated_nodes(), 0);
        assert_eq!(warm.reused_nodes(), 2);

        let partial = tree.commit(&changed);
        assert_eq!(partial.diff().changes().len(), 1);
        assert_eq!(partial.updated_nodes(), 1);
        assert_eq!(partial.reused_nodes(), 1);
        assert_eq!(
            tree.node(SceneNodeId::from_raw(2))
                .unwrap()
                .revision()
                .subtree,
            2
        );
    }

    // The production command needs a retained payload. Keeping this helper
    // local avoids exposing a test-only constructor on the draw-command API.
    struct RetainedLayerContentForTest;

    impl RetainedLayerContentForTest {
        fn content() -> crate::draw_cmd::RetainedLayerContent {
            crate::draw_cmd::RetainedLayerContent::from_snapshot(
                DrawList::new().retained_snapshot().unwrap(),
            )
        }
    }
}
