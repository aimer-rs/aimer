//! Retained per-element drawing primitives for the v2 renderer experiment.
//!
//! The tree and its shared handles are single-threaded recording data. A caller
//! can snapshot each local command list into a transferable frame representation
//! before handing work to a raster thread.

mod morph;
#[cfg(test)]
mod morph_tests;
mod render_order;
mod clip_policy;
mod command_equality;
mod current_build_context;
#[cfg(test)]
mod clip_radius_tests;
#[cfg(test)]
mod context_bounds_tests;
#[cfg(test)]
mod render_order_tests;
#[cfg(test)]
mod sync_damage_tests;
mod subtree_extent;
#[cfg(test)]
mod subtree_extent_tests;
mod world;
#[cfg(test)]
mod world_tests;

/// The custom-pipeline name of the coverage-mask marker.
///
/// A node whose local list begins with a `DrawCustom` of this name declares that
/// the rest of its list is visible only where its descendants have coverage, and
/// that the descendants themselves are not drawn. The marker carries no payload.
///
/// The render plan wraps such a node and its descendants in a masked
/// [`RenderOp::BeginOpacityGroup`]. The renderer draws the node's own list into
/// one offscreen target and the descendants into another, then keeps the first
/// only where the second has alpha. Content that samples the backdrop, such as
/// a backdrop filter, still sees what is behind the group. A marked node with no
/// descendants shows nothing. The marker command itself draws nothing: the
/// renderer skips custom commands for pipelines it has not registered.
pub const COVERAGE_MASK_PIPELINE_NAME: &str = "aimer.coverage_mask";

/// The custom-pipeline name of the fade-mask marker.
///
/// The mirror of [`COVERAGE_MASK_PIPELINE_NAME`]: a node whose local list begins
/// with a `DrawCustom` of this name declares that its own commands are not
/// shown but only provide alpha, and that its descendants are drawn through it.
/// The marker carries no payload.
///
/// The render plan wraps such a node and its descendants in a `fade`
/// [`RenderOp::BeginOpacityGroup`]. The renderer draws the node's own list into
/// one offscreen target and the descendants into another, then scales the
/// descendants' pixels by the first target's alpha, so a ramp from opaque to
/// transparent fades them out. A marked node with no descendants shows nothing.
/// The marker command itself draws nothing: the renderer skips custom commands
/// for pipelines it has not registered.
pub const FADE_MASK_PIPELINE_NAME: &str = "aimer.fade_mask";

pub use morph::{MORPH_PIPELINE_NAME, MorphGroup, MorphParams, MorphStyle};

use std::cell::RefCell;
// `hashbrown`'s default hasher: the keys are process-local node identifiers, and
// SipHash dominated tree synchronization in profiles.
use hashbrown::{HashMap, HashSet};
use std::mem;
use std::sync::Arc;

use aimer_rubick::Shared;

pub use crate::draw_cmd::RichTextSegment;
use crate::font::{FontFamily, FontStyle, TextLanguage};
use crate::svg::{SvgNodeStyleOverride, SvgScene};
use crate::text_pipeline::text_layout::TextHorizontalAlign;
use crate::text_pipeline::{TextOverflowMode, TextShadowRequest};
pub use crate::utilities::{Color, Mat3, Rect, TextureId, Vec2d};

/// Stable identity for one retained render node in this tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RenderNodeId(u64);

impl RenderNodeId {
    /// Returns the numeric identity used by the v2 render tree.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Rect {
    #[inline]
    fn translated(self, x: f32, y: f32) -> Self {
        Self::new(self.x + x, self.y + y, self.width, self.height)
    }

    #[inline]
    fn right(self) -> f32 {
        self.x + self.width
    }

    #[inline]
    fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[inline]
    fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
    }

    #[inline]
    fn intersection(self, other: Self) -> Option<Self> {
        if !self.is_valid() || !other.is_valid() {
            return None;
        }
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Self::new(x, y, right - x, bottom - y))
    }

    /// Tests whether a point lies within the rectangle's half-open bounds.
    #[inline]
    pub fn contains_point(self, x: f32, y: f32) -> bool {
        self.is_valid() && x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    #[inline]
    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(x, y, right - x, bottom - y)
    }
}

/// Shared RGBA8 pixels retained by an element-local image command.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ImageResource {
    texture_id: TextureId,
    revision: u64,
    width: u32,
    height: u32,
    intrinsic_width: u32,
    intrinsic_height: u32,
    rgba: Arc<[u8]>,
}

impl ImageResource {
    /// Creates a retained RGBA8 resource when `rgba` exactly matches its size.
    #[inline]
    pub fn rgba8(
        texture_id: TextureId,
        revision: u64,
        width: u32,
        height: u32,
        rgba: Arc<[u8]>,
    ) -> Option<Self> {
        Self::rgba8_with_intrinsic_size(texture_id, revision, width, height, width, height, rgba)
    }

    /// Creates a retained RGBA8 resource with separate upload and layout sizes.
    #[inline]
    pub fn rgba8_with_intrinsic_size(
        texture_id: TextureId,
        revision: u64,
        width: u32,
        height: u32,
        intrinsic_width: u32,
        intrinsic_height: u32,
        rgba: Arc<[u8]>,
    ) -> Option<Self> {
        let expected_len = u64::from(width)
            .checked_mul(u64::from(height))?
            .checked_mul(4)?;
        (width > 0
            && height > 0
            && intrinsic_width > 0
            && intrinsic_height > 0
            && usize::try_from(expected_len).ok()? == rgba.len())
        .then_some(Self {
            texture_id,
            revision,
            width,
            height,
            intrinsic_width,
            intrinsic_height,
            rgba,
        })
    }

    /// Returns the renderer texture identifier for this resource.
    #[inline]
    pub const fn texture_id(&self) -> TextureId {
        self.texture_id
    }

    /// Returns the generation that changes when the pixels are replaced.
    #[inline]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the pixel width uploaded to the renderer.
    #[inline]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the pixel height uploaded to the renderer.
    #[inline]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the source width used for image fitting and layout.
    #[inline]
    pub const fn intrinsic_width(&self) -> u32 {
        self.intrinsic_width
    }

    /// Returns the source height used for image fitting and layout.
    #[inline]
    pub const fn intrinsic_height(&self) -> u32 {
        self.intrinsic_height
    }

    /// Returns the retained unpremultiplied RGBA8 pixels.
    #[inline]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// One element-local paint or state instruction emitted by `aimer_canvas::Canvas`.
#[derive(Clone, Debug)]
pub enum DrawCommand {
    /// Fills a local rectangle with border and outline paint.
    FillRect {
        /// Element-local fill rectangle.
        rect: Rect,
        /// Interior fill color.
        color: Color,
        /// Per-corner radii: top-left, top-right, bottom-right, bottom-left.
        border_radius: [f32; 4],
        /// Per-side border widths: top, right, bottom, left.
        border_width: [f32; 4],
        /// Border stroke color.
        border_color: Color,
        /// Per-side outline widths: top, right, bottom, left.
        outline_width: [f32; 4],
        /// Outline stroke color.
        outline_color: Color,
    },
    /// Draws one local text run with fully resolved paint and layout settings.
    DrawText {
        /// Local baseline origin.
        position: Vec2d,
        /// Shared text payload.
        text: Arc<str>,
        /// Font size in logical pixels.
        font_size: f32,
        /// Glyph color.
        color: Color,
        /// Optional text layout width.
        bounds_width: Option<f32>,
        /// Optional text layout height.
        bounds_height: Option<f32>,
        /// Overflow policy for the supplied bounds.
        overflow: TextOverflowMode,
        /// Horizontal alignment inside the text bounds.
        horizontal_align: TextHorizontalAlign,
        /// Requested font family.
        font_family: FontFamily,
        /// Requested font style.
        font_style: FontStyle,
        /// Numeric font weight.
        font_weight: u16,
        /// Optional glyph shadow.
        shadow: Option<TextShadowRequest>,
        /// Whether to draw foreground glyphs.
        draw_glyphs: bool,
    },
    /// Draws locally positioned spans with per-span style overrides.
    DrawRichText {
        /// Local baseline origin.
        position: Vec2d,
        /// Ordered rich text spans.
        spans: Vec<RichTextSegment>,
        /// Base font size.
        font_size: f32,
        /// Base glyph color.
        color: Color,
        /// Optional text layout width.
        bounds_width: Option<f32>,
        /// Optional text layout height.
        bounds_height: Option<f32>,
        /// Overflow policy for the supplied bounds.
        overflow: TextOverflowMode,
    },
    /// Draws one local text-decoration band.
    DrawTextDecoration {
        /// Element-local decoration bounds.
        rect: Rect,
        /// Decoration stroke color.
        color: Color,
        /// Style identifier from the text-decoration style registry.
        style: u32,
        /// Stroke thickness in logical pixels.
        thickness: f32,
        /// Repeat period for patterned styles.
        period: f32,
    },
    /// Draws a previously uploaded texture by ID.
    DrawImage {
        /// Element-local destination rectangle.
        rect: Rect,
        /// Texture identifier from the existing image resource path.
        texture_id: TextureId,
    },
    /// Draws a texture and retains the pixels needed to upload it if it is evicted.
    DrawImageWithResource {
        /// Element-local destination rectangle.
        rect: Rect,
        /// Shared image pixels and their stable renderer identity.
        resource: Arc<ImageResource>,
    },
    /// Draws an SVG scene with node-style overrides.
    Svg {
        /// Immutable retained SVG payload.
        scene: Arc<SvgScene>,
        /// Element-local destination rectangle.
        destination: Rect,
        /// Per-node style overrides applied during SVG preparation.
        overrides: Arc<[SvgNodeStyleOverride]>,
    },
    /// Dispatches a retained byte payload through an existing custom pipeline.
    DrawCustom {
        /// Registered Cupid custom-pipeline name.
        pipeline_name: Arc<str>,
        /// Immutable payload consumed by the selected pipeline.
        data: Arc<[u8]>,
    },
    /// Draws a shadowed rectangle.
    DrawShadowRect {
        /// Element-local shadow rectangle.
        rect: Rect,
        /// Shadow color.
        shadow_color: Color,
        /// Offset x, offset y, blur, and spread.
        shadow_params: [f32; 4],
        /// Per-corner radii: top-left, top-right, bottom-right, bottom-left.
        border_radius: [f32; 4],
        /// Whether the shadow is inset.
        inset: bool,
        /// Side type, start angle, and end angle.
        side_params: [f32; 3],
    },
    /// Begins a clip scope local to this element's command list.
    PushClip {
        /// Element-local clip bounds.
        rect: Rect,
        /// Per-corner radii: top-left, top-right, bottom-right, bottom-left.
        border_radius: [f32; 4],
    },
    /// Ends the most recent local clip scope.
    PopClip,
    /// Begins a transform scope local to this element's command list.
    PushTransform {
        /// Transform composed with subsequent local draws.
        matrix: Mat3,
    },
    /// Ends the most recent local transform scope.
    PopTransform,
    /// Changes local command alpha until [`Self::RestoreAlpha`].
    SetAlpha {
        /// Alpha applied to subsequent local draws.
        alpha: f32,
    },
    /// Restores the default alpha for subsequent local commands.
    RestoreAlpha,
    /// Sets italic style for subsequent plain text commands in this list.
    SetItalic {
        /// Whether subsequent plain text is italic.
        italic: bool,
    },
    /// Sets written language for subsequent text commands; `None` resets it.
    SetTextLanguage {
        /// Written-language hint for subsequent text.
        language: Option<TextLanguage>,
    },
    /// Sets the current element-local transform.
    SetTransform {
        /// Current element-local transform.
        matrix: Mat3,
    },
}

/// The retained local command buffer owned by one render node.
///
/// Commands use element-local coordinates. A backend replays the list from the
/// node's inherited transform, clip, and opacity, then resets its state before
/// drawing the next element list.
#[derive(Default)]
pub struct DrawList {
    commands: Arc<[DrawCommand]>,
    /// The textures `commands` draw directly, in command order, repeats kept.
    ///
    /// Found once, when the list is committed, because every frame that draws
    /// the node needs them and the commands do not change in between.
    image_textures: Arc<[TextureId]>,
    revision: u64,
    dirty: bool,
    /// Set whenever the list becomes out of date, and cleared once a recording
    /// attempt has finished since: by a commit, or by the owner reporting that
    /// it painted nothing. Unlike `dirty`, an element that never commits (a
    /// layout container) can clear it, so it says whether the list is *known
    /// current*, not whether a commit is still outstanding.
    stale: bool,
    /// The list is stale only because its owner was rebuilt (see
    /// [`RenderTree::refresh_paint`]), so a recording equal to the current
    /// commands may keep the revision and damage nothing. Every other reason a
    /// list goes stale (a resize, an explicit invalidation, an abandoned
    /// recording) clears it: those say the pixels under the node cannot be
    /// trusted, whatever gets recorded next.
    refreshed: bool,
    recording: bool,
    /// The list opens with [`COVERAGE_MASK_PIPELINE_NAME`]: it shows only where
    /// its node's descendants have coverage. Found once, at commit, because the
    /// render plan reads it for every node of every frame.
    masks_descendants: bool,
    /// The list opens with [`FADE_MASK_PIPELINE_NAME`]: its node's descendants
    /// are shown through the alpha of the list. Found once, at commit, for the
    /// same reason as `masks_descendants`.
    fades_descendants: bool,
    /// The list opens with a decodable [`MORPH_PIPELINE_NAME`] marker: the
    /// node's two children are the sources of a shader transition. Found once,
    /// at commit, for the same reason as `masks_descendants`.
    morph: Option<MorphParams>,
}

impl DrawList {
    /// Returns the revision of the last successfully completed recording.
    #[inline]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns whether this node needs its local commands to be recorded.
    #[inline]
    pub fn needs_recording(&self) -> bool {
        self.dirty
    }

    /// Returns a shared immutable snapshot of the node's current local commands.
    #[inline]
    pub fn snapshot(&self) -> Arc<[DrawCommand]> {
        self.commands.clone()
    }
}

impl RenderNode {
    /// Replaces the node's bounds.
    ///
    /// A local list is authored against the node's size: a background fills it
    /// and a border follows it. A node that grew or shrank therefore needs its
    /// list recorded again, or the new area stays unpainted. A move keeps the
    /// list, because it is expressed relative to the node's own origin.
    fn assign_bounds(&mut self, bounds: Rect) {
        if self.bounds.width != bounds.width || self.bounds.height != bounds.height {
            let mut list = self.draw_list.borrow_mut();
            list.dirty = true;
            list.stale = true;
            list.refreshed = false;
        }
        self.bounds = bounds;
    }
}

struct RenderNode {
    id: RenderNodeId,
    parent: Option<RenderNodeId>,
    children: Vec<RenderNodeId>,
    bounds: Rect,
    clip: Option<Rect>,
    /// Corner radii of `clip`, in the clip's local logical pixels.
    clip_radius: [f32; 4],
    escapes_clip: bool,
    animation_clip: Option<Rect>,
    presentation_transform: Mat3,
    presentation_opacity: f32,
    transform: Mat3,
    opacity: f32,
    paint_source: RenderPaintSource,
    draw_list: Shared<RefCell<DrawList>>,
}

fn transform_rect(transform: Mat3, rect: Rect) -> Option<Rect> {
    // Almost every node of an interface is placed without a transform, and
    // mapping a rectangle through the identity hands back its own corners
    // exactly. Skipping the four point transforms and their checks is the
    // difference between one comparison and a few dozen calls per node.
    if transform.is_identity() {
        let (left, top) = (rect.x, rect.y);
        let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
        if !(left.is_finite() && top.is_finite() && right.is_finite() && bottom.is_finite()) {
            return None;
        }
        let (min_x, max_x) = (left.min(right), left.max(right));
        let (min_y, max_y) = (top.min(bottom), top.max(bottom));
        let bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
        return bounds.is_valid().then_some(bounds);
    }
    #[cfg(test)]
    GENERAL_RECT_TRANSFORMS.with(|count| count.set(count.get() + 1));
    let corners = [
        transform.transform_point(rect.x, rect.y),
        transform.transform_point(rect.x + rect.width, rect.y),
        transform.transform_point(rect.x, rect.y + rect.height),
        transform.transform_point(rect.x + rect.width, rect.y + rect.height),
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
    let bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
    bounds.is_valid().then_some(bounds)
}

#[derive(Clone, Copy, PartialEq)]
enum ClipState {
    Unclipped,
    Clipped(Rect),
    Empty,
}

impl ClipState {
    #[inline]
    fn intersect(self, clip: Option<Rect>) -> Self {
        match (self, clip) {
            (Self::Empty, _) => Self::Empty,
            (state, None) => state,
            (Self::Unclipped, Some(clip)) => Self::Clipped(clip),
            (Self::Clipped(parent), Some(clip)) => {
                parent.intersection(clip).map_or(Self::Empty, Self::Clipped)
            }
        }
    }

    #[inline]
    fn intersect_state(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, _) | (_, Self::Empty) => Self::Empty,
            (Self::Unclipped, state) | (state, Self::Unclipped) => state,
            (Self::Clipped(left), Self::Clipped(right)) => {
                left.intersection(right).map_or(Self::Empty, Self::Clipped)
            }
        }
    }

    #[inline]
    fn intersect_bounds(self, bounds: Rect) -> Option<Rect> {
        match self {
            Self::Unclipped => Some(bounds),
            Self::Clipped(clip) => bounds.intersection(clip),
            Self::Empty => None,
        }
    }

    #[inline]
    fn as_option(self) -> Option<Rect> {
        match self {
            Self::Unclipped => None,
            Self::Clipped(clip) => Some(clip),
            Self::Empty => {
                unreachable!("empty clip states are culled before emitting operations")
            }
        }
    }
}

/// Where the content of a render node can reach pixels, in the node's own
/// local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocalVisibleRegion {
    /// No clip limits the node, or its placement is not a plain translation,
    /// so nothing may be assumed to be out of sight.
    Unbounded,
    /// The node and everything below it are clipped away entirely.
    Nothing,
    /// Only this rectangle can be seen.
    Within(Rect),
}

/// Errors reported while constructing or recording the retained tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderTreeError {
    /// The requested render node is not part of this tree.
    UnknownNode(RenderNodeId),
    /// A child was inserted before its parent existed.
    MissingParent(RenderNodeId),
    /// A render node appeared more than once in one synchronized structure.
    DuplicateNode(RenderNodeId),
    /// A synchronized child must refer to an earlier parent entry.
    InvalidParentIndex { node: usize, parent: usize },
    /// A second recorder was opened for the same element before the first ended.
    RecorderAlreadyOpen(RenderNodeId),
    /// Recording was requested with a non-finite or negative rectangle extent.
    InvalidBounds,
    /// Opacity must be finite and in the closed interval from zero to one.
    InvalidOpacity,
    /// A clip, transform, or alpha scope was not closed before commit.
    UnbalancedState,
    /// A node transform is non-finite or overflows its subtree bounds.
    InvalidTransform,
}

/// How one synchronized render node supplies its paint content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenderPaintSource {
    /// The element has not recorded its paint yet.
    #[default]
    Unresolved,
    /// The node records only its own commands in its v2 local list.
    LocalV2,
}

/// New geometry for one existing node, for [`RenderTree::sync_geometry`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeGeometry {
    /// The existing node to update.
    pub node: RenderNodeId,
    /// Element-local position and size in logical pixels.
    pub bounds: Rect,
    /// The node's own clip, if any.
    pub clip: Option<Rect>,
    /// Top-left, top-right, bottom-right and bottom-left radii of the clip.
    pub clip_radius: [f32; 4],
}

/// One element's place in a synchronized retained render structure.
///
/// `parent_index` refers to an earlier entry in the same slice. An existing
/// `RenderNodeId` preserves that node's local paint list and effects; `None`
/// creates a new node. The returned IDs from [`RenderTree::sync_structure`]
/// align with the input slice.
#[derive(Clone, Copy, Debug)]
pub struct RenderNodeSpec {
    /// Previously assigned render identity, if this element already exists.
    pub existing_id: Option<RenderNodeId>,
    /// Index of the parent descriptor, or `None` for a tree root.
    pub parent_index: Option<usize>,
    /// Element-local position and size in logical pixels.
    pub bounds: Rect,
}

impl RenderNodeSpec {
    /// Creates a structure descriptor for an existing or new render node.
    #[inline]
    pub const fn new(
        existing_id: Option<RenderNodeId>,
        parent_index: Option<usize>,
        bounds: Rect,
    ) -> Self {
        Self {
            existing_id,
            parent_index,
            bounds,
        }
    }
}

// Counts `DrawCommandList::node` lookups so tests can bound the cost of
// world-space queries. Compiled out of every non-test build.
#[cfg(test)]
thread_local! {
    static NODE_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Counts how many node frames are derived from their parents, so tests can
// bound the work world-space queries repeat. Compiled out of every non-test
// build.
#[cfg(test)]
thread_local! {
    static FRAME_DERIVATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Counts rectangles mapped through a transform that is not the identity, so
// tests can show the common untransformed case never takes that path.
#[cfg(test)]
thread_local! {
    static GENERAL_RECT_TRANSFORMS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Counts how often a plan derives the geometry of every node, so tests can
// show it is reused while nothing that affects geometry has changed.
#[cfg(test)]
thread_local! {
    static PLAN_GEOMETRY_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn plan_geometry_runs() -> usize {
    PLAN_GEOMETRY_RUNS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn general_rect_transforms() -> usize {
    GENERAL_RECT_TRANSFORMS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn frame_derivations() -> usize {
    FRAME_DERIVATIONS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_node_lookups() {
    NODE_LOOKUPS.with(|count| count.set(0));
    FRAME_DERIVATIONS.with(|count| count.set(0));
    GENERAL_RECT_TRANSFORMS.with(|count| count.set(0));
    PLAN_GEOMETRY_RUNS.with(|count| count.set(0));
}

#[cfg(test)]
fn node_lookups() -> usize {
    NODE_LOOKUPS.with(std::cell::Cell::get)
}

/// A retained tree of local paint lists and ordered child links.
#[derive(Default)]
pub struct DrawCommandList {
    nodes: Vec<RenderNode>,
    indices: HashMap<RenderNodeId, usize, crate::utilities::IdBuildHasher>,
    roots: Vec<RenderNodeId>,
    next_id: u64,
    clip_escape_count: usize,
    pending_damage: Vec<Rect>,
    /// Advances whenever a node is added, handed out mutably, or the structure
    /// is replaced. Those are the only ways a node's world rectangle can
    /// change, so a cached rectangle is current while this is unchanged. It
    /// may advance without any rectangle changing (a paint source is set
    /// through the same accessor); that only costs a recomputation.
    geometry_revision: u64,
    /// Advances whenever the parent/child structure can change: a node is
    /// added or the node array is replaced by a structure sync. The paint-order
    /// plan caches the resolved positions of parents and children against it,
    /// so every edit to `nodes`, `indices`, `roots`, `parent` or `children`
    /// must advance it. Geometry, clip and paint edits must not.
    topology_revision: u64,
    /// Positions of every parent and child, resolved lazily against
    /// `topology_revision`; read through [`DrawCommandList::topology`].
    topology: RefCell<render_order::Topology>,
    /// The frame each node hands its children, as of the `geometry_revision`
    /// stored beside it. A slot from an earlier revision is ignored, so no edit
    /// has to clear anything: every way a node's geometry can change advances
    /// the revision. See [`DrawCommandList::frame_at`].
    world_frames: RefCell<Vec<Option<(u64, world::WorldFrame)>>>,
    render_workspace: RefCell<render_order::RenderWorkspace>,
    /// What each subtree covers, kept for as long as nothing inside it is
    /// edited. See [`subtree_extent`].
    extents: RefCell<subtree_extent::ExtentCache>,
}

impl DrawCommandList {
    fn insert(
        &mut self,
        parent: Option<RenderNodeId>,
        bounds: Rect,
    ) -> Result<RenderNodeId, RenderTreeError> {
        if !bounds.is_valid() {
            return Err(RenderTreeError::InvalidBounds);
        }
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        self.topology_revision = self.topology_revision.wrapping_add(1);
        let parent_index = match parent {
            Some(parent) => Some(
                *self
                    .indices
                    .get(&parent)
                    .ok_or(RenderTreeError::MissingParent(parent))?,
            ),
            None => None,
        };

        let id = RenderNodeId(
            self.next_id
                .checked_add(1)
                .expect("render node IDs exhausted"),
        );
        self.next_id = id.get();
        let index = self.nodes.len();
        self.indices.insert(id, index);
        self.nodes.push(RenderNode {
            id,
            parent,
            children: Vec::new(),
            bounds,
            clip: None,
            clip_radius: [0.0; 4],
            escapes_clip: false,
            animation_clip: None,
            presentation_transform: Mat3::identity(),
            presentation_opacity: 1.0,
            transform: Mat3::identity(),
            opacity: 1.0,
            paint_source: RenderPaintSource::Unresolved,
            draw_list: Shared::new(RefCell::new(DrawList {
                dirty: true,
                stale: true,
                ..DrawList::default()
            })),
        });
        if let Some(parent_index) = parent_index {
            self.nodes[parent_index].children.push(id);
        } else {
            self.roots.push(id);
        }
        if let Some(bounds) = self.visible_node_bounds(id) {
            self.push_damage(bounds);
        }
        Ok(id)
    }

    fn node(&self, id: RenderNodeId) -> Option<&RenderNode> {
        #[cfg(test)]
        NODE_LOOKUPS.with(|count| count.set(count.get() + 1));
        self.indices
            .get(&id)
            .and_then(|index| self.nodes.get(*index))
    }

    fn node_mut(&mut self, id: RenderNodeId) -> Option<&mut RenderNode> {
        let index = *self.indices.get(&id)?;
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        self.mark_extent_stale(index, true);
        self.nodes.get_mut(index)
    }

    /// Hands out a node for an edit to its own `transform` or presentation only.
    ///
    /// Like [`Self::node_mut`], but the node's measured extent stays valid: an
    /// extent is taken before the node's own placement, so only what is above
    /// the node, which places it, is measured again. An edit to anything else
    /// about the node must use `node_mut`.
    fn node_mut_placement(&mut self, id: RenderNodeId) -> Option<&mut RenderNode> {
        let index = *self.indices.get(&id)?;
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        self.mark_extent_stale(index, false);
        self.nodes.get_mut(index)
    }

    /// Hands out a node for an edit that cannot change any node's geometry:
    /// its paint source, its opacity, or the state of its local command list.
    ///
    /// Unlike [`Self::node_mut`] this leaves `geometry_revision` alone, so the
    /// frames and plans derived from geometry stay valid across the repaints,
    /// hovers and fades that make up most frames. An edit that touches bounds,
    /// clips, transforms or the structure must use `node_mut`.
    fn node_mut_paint(&mut self, id: RenderNodeId) -> Option<&mut RenderNode> {
        let index = *self.indices.get(&id)?;
        self.nodes.get_mut(index)
    }

    fn subtree_uses_only_local_v2(&self, id: RenderNodeId) -> bool {
        let Some(node) = self.node(id) else {
            return false;
        };
        node.paint_source == RenderPaintSource::LocalV2
            && node
                .children
                .iter()
                .copied()
                .all(|child| self.subtree_uses_only_local_v2(child))
    }

    fn push_damage(&mut self, rect: Rect) {
        if !rect.is_valid() || rect.width == 0.0 || rect.height == 0.0 {
            return;
        }
        if let Some(existing) = self
            .pending_damage
            .iter_mut()
            .find(|existing| existing.intersection(rect).is_some())
        {
            *existing = existing.union(rect);
        } else {
            self.pending_damage.push(rect);
        }
    }

    /// Runs `apply`, which edits the geometry of nodes at or inside `tops`, and
    /// damages the footprint of each of those subtrees from before and after.
    ///
    /// A subtree's footprint contains every descendant, so `tops` must be the
    /// changed nodes that have no changed ancestor: walking the subtree of
    /// every changed node instead visits each node once per ancestor, which
    /// made a window resize cost the sum of all subtree sizes twice per node.
    fn damage_subtrees_around<R>(
        &mut self,
        tops: &[RenderNodeId],
        apply: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let before: Vec<Option<Rect>> = tops
            .iter()
            .map(|id| self.subtree_damage_bounds(*id))
            .collect();
        let result = apply(self);
        for (id, old_bounds) in tops.iter().zip(before) {
            let new_bounds = self.subtree_damage_bounds(*id);
            if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
                self.push_damage(damage);
            }
        }
        result
    }
}

/// A shareable handle to the retained v2 render tree.
#[derive(Clone, Default)]
pub struct RenderTree {
    draw_cmd: Shared<RefCell<DrawCommandList>>,
}

impl RenderTree {
    /// Creates an empty retained render tree.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a root element in paint order.
    pub fn add_root(&self, bounds: Rect) -> Result<RenderNodeId, RenderTreeError> {
        self.draw_cmd.borrow_mut().insert(None, bounds)
    }

    /// Adds a child after the parent's existing children.
    pub fn add_child(
        &self,
        parent: RenderNodeId,
        bounds: Rect,
    ) -> Result<RenderNodeId, RenderTreeError> {
        self.draw_cmd.borrow_mut().insert(Some(parent), bounds)
    }

    /// Synchronizes element order, parentage, and layout while retaining lists
    /// for nodes whose IDs remain present.
    ///
    /// Descriptors are supplied in parent-before-child traversal order. A
    /// structural change damages the previous and next visible bounds of
    /// affected subtrees; a bounds-only change damages the changed node's
    /// previous and next subtree footprints. IDs returned at each position
    /// line up with `nodes`.
    pub fn sync_structure(
        &self,
        nodes: &[RenderNodeSpec],
    ) -> Result<Vec<RenderNodeId>, RenderTreeError> {
        self.sync_structure_inner(nodes, None, None)
    }

    /// Synchronizes render nodes and their local clips as one geometry update.
    ///
    /// `clips` must align with `nodes`. Applying bounds and clips together
    /// keeps moving viewports from damaging an intermediate position.
    #[doc(hidden)]
    pub fn sync_structure_with_clips(
        &self,
        nodes: &[RenderNodeSpec],
        clips: &[Option<Rect>],
    ) -> Result<Vec<RenderNodeId>, RenderTreeError> {
        if nodes.len() != clips.len() || clips.iter().flatten().any(|clip| !clip.is_valid()) {
            return Err(RenderTreeError::InvalidBounds);
        }
        self.sync_structure_inner(nodes, Some(clips), None)
    }

    /// Like [`Self::sync_structure_with_clips`], with corner radii for each clip.
    ///
    /// `radii` aligns with `nodes` and gives the top-left, top-right,
    /// bottom-right, and bottom-left radii of the same node's clip in the
    /// clip's local logical pixels. A radius on a node without a clip is
    /// ignored by rendering. Radii must be finite and non-negative.
    #[doc(hidden)]
    pub fn sync_structure_with_clip_radii(
        &self,
        nodes: &[RenderNodeSpec],
        clips: &[Option<Rect>],
        radii: &[[f32; 4]],
    ) -> Result<Vec<RenderNodeId>, RenderTreeError> {
        if nodes.len() != clips.len()
            || nodes.len() != radii.len()
            || clips.iter().flatten().any(|clip| !clip.is_valid())
            || radii
                .iter()
                .flatten()
                .any(|radius| !radius.is_finite() || *radius < 0.0)
        {
            return Err(RenderTreeError::InvalidBounds);
        }
        self.sync_structure_inner(nodes, Some(clips), Some(radii))
    }

    /// Applies new bounds, clips and clip radii to existing nodes, leaving the
    /// tree's structure alone.
    ///
    /// This is the part of [`Self::sync_structure_with_clip_radii`] that runs
    /// when no node was added, removed or re-parented, for a caller that already
    /// knows that and has descriptors for only some of the nodes. A node whose
    /// geometry is unchanged costs one comparison and records no damage. Nothing
    /// is applied if any descriptor is invalid or names an unknown node.
    #[doc(hidden)]
    pub fn sync_geometry(&self, updates: &[NodeGeometry]) -> Result<(), RenderTreeError> {
        if updates.iter().any(|update| {
            !update.bounds.is_valid()
                || update.clip.is_some_and(|clip| !clip.is_valid())
                || update
                    .clip_radius
                    .iter()
                    .any(|radius| !radius.is_finite() || *radius < 0.0)
        }) {
            return Err(RenderTreeError::InvalidBounds);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        if let Some(missing) = updates
            .iter()
            .find(|update| tree.node(update.node).is_none())
        {
            return Err(RenderTreeError::UnknownNode(missing.node));
        }
        let changed: HashSet<RenderNodeId> = updates
            .iter()
            .filter(|update| {
                let node = tree.node(update.node).expect("presence checked above");
                node.bounds != update.bounds
                    || node.clip != update.clip
                    || node.clip_radius != update.clip_radius
            })
            .map(|update| update.node)
            .collect();
        let tops: Vec<RenderNodeId> = changed
            .iter()
            .copied()
            .filter(|id| {
                let mut ancestor = tree.node(*id).and_then(|node| node.parent);
                while let Some(parent) = ancestor {
                    if changed.contains(&parent) {
                        return false;
                    }
                    ancestor = tree.node(parent).and_then(|node| node.parent);
                }
                true
            })
            .collect();
        tree.damage_subtrees_around(&tops, |tree| {
            for update in updates {
                if !changed.contains(&update.node) {
                    continue;
                }
                let node = tree.node_mut(update.node).expect("presence checked above");
                node.assign_bounds(update.bounds);
                node.clip = update.clip;
                node.clip_radius = update.clip_radius;
            }
        });
        Ok(())
    }

    fn sync_structure_inner(
        &self,
        nodes: &[RenderNodeSpec],
        clip_updates: Option<&[Option<Rect>]>,
        radius_updates: Option<&[[f32; 4]]>,
    ) -> Result<Vec<RenderNodeId>, RenderTreeError> {
        // Radii are part of the clip update: without a radius slice the clip
        // is square.
        let radius_at = |index: usize| radius_updates.map_or([0.0; 4], |radii| radii[index]);
        let mut tree = self.draw_cmd.borrow_mut();
        let mut seen = HashSet::with_capacity(nodes.len());
        let mut ids = Vec::with_capacity(nodes.len());
        let mut next_id = tree.next_id;

        for (index, spec) in nodes.iter().enumerate() {
            if !spec.bounds.is_valid() {
                return Err(RenderTreeError::InvalidBounds);
            }
            if let Some(parent) = spec.parent_index
                && parent >= index
            {
                return Err(RenderTreeError::InvalidParentIndex {
                    node: index,
                    parent,
                });
            }

            let id = if let Some(id) = spec.existing_id {
                if !seen.insert(id) {
                    return Err(RenderTreeError::DuplicateNode(id));
                }
                if tree.node(id).is_none() {
                    return Err(RenderTreeError::UnknownNode(id));
                }
                id
            } else {
                next_id = next_id.checked_add(1).expect("render node IDs exhausted");
                RenderNodeId(next_id)
            };
            ids.push(id);
        }

        let mut parents = Vec::with_capacity(nodes.len());
        let mut children = vec![Vec::new(); nodes.len()];
        let mut roots = Vec::new();
        for (index, spec) in nodes.iter().enumerate() {
            let parent = spec.parent_index.map(|parent| ids[parent]);
            if let Some(parent_index) = spec.parent_index {
                children[parent_index].push(ids[index]);
            } else {
                roots.push(ids[index]);
            }
            parents.push(parent);
        }

        let structure_changed = tree.nodes.len() != nodes.len()
            || tree.roots != roots
            || nodes.iter().enumerate().any(|(index, _)| {
                let Some(old) = tree.node(ids[index]) else {
                    return true;
                };
                old.parent != parents[index] || old.children != children[index]
            });

        if !structure_changed {
            // `covered[index]` is set for a changed node and everything below
            // one, so the first changed node on each path is the only one whose
            // subtree footprint has to be damaged.
            let mut covered = vec![false; nodes.len()];
            let mut changed = Vec::new();
            let mut tops = Vec::new();
            for (index, spec) in nodes.iter().enumerate() {
                let id = ids[index];
                let node = tree.node(id).ok_or(RenderTreeError::UnknownNode(id))?;
                let clip_changed = clip_updates.is_some_and(|clips| {
                    node.clip != clips[index] || node.clip_radius != radius_at(index)
                });
                let is_changed = node.bounds != spec.bounds || clip_changed;
                let under_changed = spec.parent_index.is_some_and(|parent| covered[parent]);
                covered[index] = under_changed || is_changed;
                if is_changed {
                    changed.push(index);
                    if !under_changed {
                        tops.push(id);
                    }
                }
            }
            tree.damage_subtrees_around(&tops, |tree| {
                for index in changed {
                    let Some(node) = tree.node_mut(ids[index]) else {
                        continue;
                    };
                    node.assign_bounds(nodes[index].bounds);
                    if let Some(clips) = clip_updates {
                        node.clip = clips[index];
                        node.clip_radius = radius_at(index);
                    }
                }
            });
            return Ok(ids);
        }

        // A structural edit only changes pixels in the changed parent
        // subtrees. Damaging the union of every root here made a virtualized
        // row-window update repaint the complete window whenever the app root
        // filled it, even though the changed rows were clipped by a scroll
        // viewport. Capture the old clipped bounds before replacing the node
        // array, then pair them with the corresponding new bounds below.
        let mut structural_damage_ids = HashSet::new();
        for (index, id) in ids.iter().copied().enumerate() {
            if let Some(old) = tree.node(id) {
                if old.children != children[index]
                    || old.bounds != nodes[index].bounds
                    || clip_updates.is_some_and(|clips| {
                        old.clip != clips[index] || old.clip_radius != radius_at(index)
                    })
                {
                    structural_damage_ids.insert(id);
                }
                if old.parent != parents[index] {
                    structural_damage_ids.insert(old.parent.unwrap_or(id));
                    structural_damage_ids.insert(parents[index].unwrap_or(id));
                    structural_damage_ids.insert(id);
                }
            } else {
                structural_damage_ids.insert(parents[index].unwrap_or(id));
            }
        }
        for old in &tree.nodes {
            if !seen.contains(&old.id) {
                structural_damage_ids.insert(old.parent.unwrap_or(old.id));
            }
        }
        let old_retained_roots = tree
            .roots
            .iter()
            .filter(|id| roots.contains(id))
            .copied()
            .collect::<Vec<_>>();
        let new_retained_roots = roots
            .iter()
            .filter(|id| tree.roots.contains(id))
            .copied()
            .collect::<Vec<_>>();
        if old_retained_roots != new_retained_roots {
            structural_damage_ids.extend(old_retained_roots);
            structural_damage_ids.extend(new_retained_roots);
        }
        let mut structural_damage_ids = structural_damage_ids.into_iter().collect::<Vec<_>>();
        structural_damage_ids.sort_unstable_by_key(|id| id.get());
        let old_damage = tree.visible_subtree_bounds_of(&structural_damage_ids);
        let mut previous = mem::take(&mut tree.nodes)
            .into_iter()
            .map(|node| (node.id, node))
            .collect::<HashMap<_, _>>();
        let mut synchronized = Vec::with_capacity(nodes.len());
        let mut clip_escape_count = 0;
        for (index, spec) in nodes.iter().enumerate() {
            let id = ids[index];
            let mut node = previous.remove(&id).unwrap_or_else(|| RenderNode {
                id,
                parent: None,
                children: Vec::new(),
                bounds: spec.bounds,
                clip: None,
                clip_radius: [0.0; 4],
                escapes_clip: false,
                animation_clip: None,
                presentation_transform: Mat3::identity(),
                presentation_opacity: 1.0,
                transform: Mat3::identity(),
                opacity: 1.0,
                paint_source: RenderPaintSource::Unresolved,
                draw_list: Shared::new(RefCell::new(DrawList {
                    dirty: true,
                    stale: true,
                    ..DrawList::default()
                })),
            });
            node.parent = parents[index];
            node.children = mem::take(&mut children[index]);
            node.assign_bounds(spec.bounds);
            if let Some(clips) = clip_updates {
                node.clip = clips[index];
                node.clip_radius = radius_at(index);
            }
            clip_escape_count += usize::from(node.escapes_clip);
            synchronized.push(node);
        }

        tree.geometry_revision = tree.geometry_revision.wrapping_add(1);
        tree.topology_revision = tree.topology_revision.wrapping_add(1);
        tree.nodes = synchronized;
        tree.clip_escape_count = clip_escape_count;
        tree.indices = tree
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
        tree.roots = roots;
        tree.next_id = next_id;

        let new_damage = tree.visible_subtree_bounds_of(&structural_damage_ids);
        for damage in old_damage.into_iter().chain(new_damage) {
            tree.push_damage(damage);
        }
        Ok(ids)
    }

    /// Creates a build context for one element's local paint list.
    pub fn context(&self, element: RenderNodeId) -> Result<CurrentBuildContext, RenderTreeError> {
        if self.draw_cmd.borrow().node(element).is_none() {
            return Err(RenderTreeError::UnknownNode(element));
        }
        Ok(CurrentBuildContext {
            draw_cmd: self.draw_cmd.clone(),
            element,
        })
    }

    /// Returns the paint source currently selected for one element.
    pub fn paint_source(
        &self,
        element: RenderNodeId,
    ) -> Result<RenderPaintSource, RenderTreeError> {
        self.draw_cmd
            .borrow()
            .node(element)
            .map(|node| node.paint_source)
            .ok_or(RenderTreeError::UnknownNode(element))
    }

    /// Returns whether this element's local v2 draw list needs recording.
    pub fn needs_recording(&self, element: RenderNodeId) -> Result<bool, RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        Ok(node.draw_list.borrow().needs_recording())
    }

    /// Changes the paint source for one retained node.
    pub fn set_paint_source(
        &self,
        element: RenderNodeId,
        source: RenderPaintSource,
    ) -> Result<(), RenderTreeError> {
        let mut tree = self.draw_cmd.borrow_mut();
        let current_source = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?
            .paint_source;
        if current_source == source {
            return Ok(());
        }
        let bounds = tree.visible_subtree_bounds(element);
        let node = tree
            .node_mut_paint(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        if node.draw_list.borrow().recording {
            return Err(RenderTreeError::RecorderAlreadyOpen(element));
        }
        node.paint_source = source;
        if let Some(bounds) = bounds {
            tree.push_damage(bounds);
        }
        Ok(())
    }

    /// Changes an element's local bounds and damages both its old and new area.
    pub fn set_bounds(&self, element: RenderNodeId, bounds: Rect) -> Result<(), RenderTreeError> {
        if !bounds.is_valid() {
            return Err(RenderTreeError::InvalidBounds);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        let current_bounds = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?
            .bounds;
        if current_bounds == bounds {
            return Ok(());
        }
        let old = tree.visible_subtree_bounds(element);
        let node = tree
            .node_mut(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        node.assign_bounds(bounds);
        let new = tree.visible_subtree_bounds(element);
        if let Some(damage) = old.into_iter().chain(new).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    /// Sets the node-local transform inherited by its retained descendants.
    ///
    /// The transform changes presentation geometry and damage only; it leaves
    /// every node's local command list and revision untouched. The supplied
    /// matrix maps coordinates relative to this node's origin. It must be
    /// finite and produce finite subtree bounds.
    pub fn set_transform(
        &self,
        element: RenderNodeId,
        transform: Mat3,
    ) -> Result<(), RenderTreeError> {
        if !transform
            .cols
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(RenderTreeError::InvalidTransform);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        let old_transform = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?
            .transform;
        if old_transform == transform {
            return Ok(());
        }
        let old_bounds = tree.damage_footprint(element);
        tree.node_mut_placement(element)
            .ok_or(RenderTreeError::UnknownNode(element))?
            .transform = transform;
        if !tree.subtree_is_finite(element) {
            tree.node_mut_placement(element)
                .ok_or(RenderTreeError::UnknownNode(element))?
                .transform = old_transform;
            return Err(RenderTreeError::InvalidTransform);
        }
        let new_bounds = tree.damage_footprint(element);
        if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    fn set_presentation(
        &self,
        element: RenderNodeId,
        transform: Mat3,
        opacity: f32,
    ) -> Result<(), RenderTreeError> {
        if !transform
            .cols
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(RenderTreeError::InvalidTransform);
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(RenderTreeError::InvalidOpacity);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        if node.presentation_transform == transform && node.presentation_opacity == opacity {
            return Ok(());
        }
        let old_transform = node.presentation_transform;
        let old_opacity = node.presentation_opacity;
        let old_bounds = tree.damage_footprint(element);
        let node = tree
            .node_mut_placement(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        node.presentation_transform = transform;
        node.presentation_opacity = opacity;
        if !tree.subtree_is_finite(element) {
            let node = tree
                .node_mut_placement(element)
                .ok_or(RenderTreeError::UnknownNode(element))?;
            node.presentation_transform = old_transform;
            node.presentation_opacity = old_opacity;
            return Err(RenderTreeError::InvalidTransform);
        }
        let new_bounds = tree.damage_footprint(element);
        if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    /// Applies a local clip to the element and its descendants.
    pub fn set_clip(
        &self,
        element: RenderNodeId,
        clip: Option<Rect>,
    ) -> Result<(), RenderTreeError> {
        if clip.is_some_and(|clip| !clip.is_valid()) {
            return Err(RenderTreeError::InvalidBounds);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        if tree.node(element).is_none() {
            return Err(RenderTreeError::UnknownNode(element));
        }
        // This setter describes a square clip: it also clears any radius.
        if tree
            .node(element)
            .is_some_and(|node| node.clip == clip && node.clip_radius == [0.0; 4])
        {
            return Ok(());
        }
        let old_bounds = tree.visible_subtree_bounds(element);
        let node = tree
            .node_mut(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        node.clip = clip;
        node.clip_radius = [0.0; 4];
        let new_bounds = tree.visible_subtree_bounds(element);
        if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    /// Applies a local clip with rounded corners to the element and its
    /// descendants.
    ///
    /// `radius` is top-left, top-right, bottom-right, bottom-left in the
    /// element's own logical pixels, so it scales with the node's presentation
    /// transform exactly as the clip rectangle does. It is dropped when `clip`
    /// is `None`. An unchanged clip and radius record no damage, which lets a
    /// caller update the radius every frame of an animation.
    #[doc(hidden)]
    pub fn set_clip_with_radius(
        &self,
        element: RenderNodeId,
        clip: Option<Rect>,
        radius: [f32; 4],
    ) -> Result<(), RenderTreeError> {
        if clip.is_some_and(|clip| !clip.is_valid())
            || radius.iter().any(|corner| !corner.is_finite() || *corner < 0.0)
        {
            return Err(RenderTreeError::InvalidBounds);
        }
        let radius = if clip.is_some() { radius } else { [0.0; 4] };
        let mut tree = self.draw_cmd.borrow_mut();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        if node.clip == clip && node.clip_radius == radius {
            return Ok(());
        }
        let old_bounds = tree.visible_subtree_bounds(element);
        let node = tree
            .node_mut(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        node.clip = clip;
        node.clip_radius = radius;
        let new_bounds = tree.visible_subtree_bounds(element);
        if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    /// Changes an element's local bounds and clip as one geometry update.
    ///
    /// This is useful for moving a clipped viewport child: applying bounds
    /// before its compensating clip would otherwise damage the intermediate
    /// position as well as the old and new footprints.
    pub fn set_geometry(
        &self,
        element: RenderNodeId,
        bounds: Rect,
        clip: Option<Rect>,
    ) -> Result<(), RenderTreeError> {
        if !bounds.is_valid() || clip.is_some_and(|clip| !clip.is_valid()) {
            return Err(RenderTreeError::InvalidBounds);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        if node.bounds == bounds && node.clip == clip && node.clip_radius == [0.0; 4] {
            return Ok(());
        }
        let old_bounds = tree.visible_subtree_bounds(element);
        let node = tree
            .node_mut(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        node.bounds = bounds;
        node.clip = clip;
        node.clip_radius = [0.0; 4];
        let new_bounds = tree.visible_subtree_bounds(element);
        if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    /// Sets node opacity without invalidating its local draw commands.
    ///
    /// A node with children becomes an opacity group so overlapping
    /// descendants are blended together once. A leaf carries opacity directly
    /// on its [`RenderItem`]. Both cases damage the node's subtree bounds.
    pub fn set_opacity(&self, element: RenderNodeId, opacity: f32) -> Result<(), RenderTreeError> {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(RenderTreeError::InvalidOpacity);
        }
        let mut tree = self.draw_cmd.borrow_mut();
        let current_opacity = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?
            .opacity;
        if current_opacity == opacity {
            return Ok(());
        }
        let bounds = tree.visible_subtree_bounds(element);
        let node = tree
            .node_mut_paint(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        node.opacity = opacity;
        if let Some(bounds) = bounds {
            tree.push_damage(bounds);
        }
        Ok(())
    }

    /// Returns the transform and opacity a parent last published for a node
    /// with [`Self::set_presentation`], separate from any compositor animation.
    #[doc(hidden)]
    pub fn presentation(&self, element: RenderNodeId) -> Result<(Mat3, f32), RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        Ok((node.presentation_transform, node.presentation_opacity))
    }

    /// Returns the transform and opacity the compositor currently applies to a
    /// node, as last set by [`Self::set_compositor_animation`].
    ///
    /// A node that was never animated reports the identity transform and full
    /// opacity.
    #[doc(hidden)]
    pub fn compositor_animation(
        &self,
        element: RenderNodeId,
    ) -> Result<(Mat3, f32), RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        Ok((node.transform, node.opacity))
    }

    /// Updates an animation transform, opacity, and pre-transform clip as one
    /// retained presentation change without re-recording local commands.
    #[doc(hidden)]
    pub fn set_compositor_animation(
        &self,
        element: RenderNodeId,
        transform: Mat3,
        opacity: f32,
        clip: Option<Rect>,
    ) -> Result<(), RenderTreeError> {
        if !transform
            .cols
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(RenderTreeError::InvalidTransform);
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(RenderTreeError::InvalidOpacity);
        }
        if clip.is_some_and(|clip| !clip.is_valid()) {
            return Err(RenderTreeError::InvalidBounds);
        }

        let mut tree = self.draw_cmd.borrow_mut();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        if node.transform == transform && node.opacity == opacity && node.animation_clip == clip {
            return Ok(());
        }

        let old_bounds = tree.visible_subtree_bounds(element);
        let (old_transform, old_opacity, old_clip) = {
            let node = tree
                .node_mut(element)
                .ok_or(RenderTreeError::UnknownNode(element))?;
            let previous = (node.transform, node.opacity, node.animation_clip);
            node.transform = transform;
            node.opacity = opacity;
            node.animation_clip = clip;
            previous
        };
        if tree.subtree_bounds(element).is_none() {
            let node = tree
                .node_mut(element)
                .ok_or(RenderTreeError::UnknownNode(element))?;
            node.transform = old_transform;
            node.opacity = old_opacity;
            node.animation_clip = old_clip;
            return Err(RenderTreeError::InvalidTransform);
        }

        let new_bounds = tree.visible_subtree_bounds(element);
        if let Some(damage) = old_bounds.into_iter().chain(new_bounds).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }

    /// Marks this element's local command list stale so it is recorded again,
    /// without damaging anything yet.
    ///
    /// For an element that was rebuilt and may or may not draw something new:
    /// the commit of its next recording damages the node only if the recorded
    /// picture differs from the previous one. Use
    /// [`invalidate_paint`](Self::invalidate_paint) when the pixels under the
    /// node are known to be out of date whatever the element records.
    pub fn refresh_paint(&self, element: RenderNodeId) -> Result<(), RenderTreeError> {
        let mut tree = self.draw_cmd.borrow_mut();
        let node = tree
            .node_mut_paint(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        let mut list = node.draw_list.borrow_mut();
        // Already stale for a stronger reason stays that way.
        if !list.stale {
            list.refreshed = true;
        }
        list.dirty = true;
        list.stale = true;
        Ok(())
    }

    /// Marks only this element's local command list stale.
    pub fn invalidate_paint(&self, element: RenderNodeId) -> Result<(), RenderTreeError> {
        let mut tree = self.draw_cmd.borrow_mut();
        if tree.node(element).is_none() {
            return Err(RenderTreeError::UnknownNode(element));
        }
        let bounds = tree.visible_node_bounds(element);
        let node = tree
            .node_mut_paint(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        {
            let mut list = node.draw_list.borrow_mut();
            list.dirty = true;
            list.stale = true;
            list.refreshed = false;
        }
        if let Some(bounds) = bounds {
            tree.push_damage(bounds);
        }
        Ok(())
    }

    /// Returns the IDs whose local command lists need recording.
    pub fn dirty_elements(&self) -> Vec<RenderNodeId> {
        self.draw_cmd
            .borrow()
            .nodes
            .iter()
            .filter_map(|node| node.draw_list.borrow().dirty.then_some(node.id))
            .collect()
    }

    /// Returns the most recently committed local command revision.
    pub fn draw_list_revision(&self, element: RenderNodeId) -> Result<u64, RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        let revision = node.draw_list.borrow().revision;
        Ok(revision)
    }

    /// Whether `other` is a handle to this same tree.
    #[inline]
    pub fn same_tree(&self, other: &Self) -> bool {
        Shared::ptr_eq(&self.draw_cmd, &other.draw_cmd)
    }

    /// A counter that changes whenever any node's world rectangle may have.
    ///
    /// Callers that cache [`Self::element_bounds`] results are current while
    /// this is unchanged. It can also advance when nothing moved.
    #[inline]
    pub fn geometry_revision(&self) -> u64 {
        self.draw_cmd.borrow().geometry_revision
    }

    /// Returns an element's current world-space layout bounds.
    pub fn element_bounds(&self, element: RenderNodeId) -> Result<Rect, RenderTreeError> {
        self.draw_cmd
            .borrow()
            .world_bounds(element)
            .ok_or(RenderTreeError::UnknownNode(element))
    }

    /// Returns where anything drawn by an element or its descendants can reach
    /// pixels, in the element's own local logical coordinates.
    ///
    /// It is the clip the element hands down, so it does not depend on the
    /// element's own bounds: children that overflow a parent are still
    /// covered. See [`LocalVisibleRegion`] for the three possible answers.
    pub fn local_visible_region(
        &self,
        element: RenderNodeId,
    ) -> Result<LocalVisibleRegion, RenderTreeError> {
        self.draw_cmd
            .borrow()
            .local_visible_region(element)
            .ok_or(RenderTreeError::UnknownNode(element))
    }

    /// Reports whether an element's retained paint is known to be up to date:
    /// it paints through a local v2 list that a recording attempt has finished
    /// on since the list last went stale, and no recorder is open on it.
    ///
    /// That holds for an element whose commands were committed and for one that
    /// painted nothing and reported it with
    /// [`mark_recording_attempted`](Self::mark_recording_attempted). A settled
    /// element has nothing new to record, so skipping its traversal while it
    /// is off screen loses no paint.
    pub fn is_settled(&self, element: RenderNodeId) -> Result<bool, RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        let list = node.draw_list.borrow();
        Ok(node.paint_source == RenderPaintSource::LocalV2 && !list.stale && !list.recording)
    }

    /// Reports that an element's paint callback has run for the current state
    /// of its list, whether or not it committed anything.
    ///
    /// A layout container paints nothing and never commits, so its list keeps
    /// asking to be recorded. Marking the attempt tells the tree the list is
    /// still current; [`needs_recording`](Self::needs_recording) is left alone,
    /// so the element is asked again exactly as often as before. A node with
    /// an open recorder is left unsettled.
    pub fn mark_recording_attempted(&self, element: RenderNodeId) -> Result<(), RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        let mut list = node.draw_list.borrow_mut();
        if !list.recording {
            list.stale = false;
            list.refreshed = false;
        }
        Ok(())
    }

    /// Returns an element's direct parent, or `None` when it is a root.
    pub fn parent_of(
        &self,
        element: RenderNodeId,
    ) -> Result<Option<RenderNodeId>, RenderTreeError> {
        self.draw_cmd
            .borrow()
            .node(element)
            .map(|node| node.parent)
            .ok_or(RenderTreeError::UnknownNode(element))
    }

    /// Shares an immutable snapshot of one element's local retained commands.
    pub fn draw_list_snapshot(
        &self,
        element: RenderNodeId,
    ) -> Result<DrawListSnapshot, RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let node = tree
            .node(element)
            .ok_or(RenderTreeError::UnknownNode(element))?;
        let draw_list = node.draw_list.borrow();
        Ok(DrawListSnapshot {
            revision: draw_list.revision,
            commands: draw_list.commands.clone(),
            image_textures: draw_list.image_textures.clone(),
        })
    }

    /// Returns whether any damage is waiting to be taken, without taking it.
    pub fn has_pending_damage(&self) -> bool {
        !self.draw_cmd.borrow().pending_damage.is_empty()
    }

    /// Takes damage accumulated by new nodes, local repaint, or geometry changes.
    pub fn take_damage(&self) -> Vec<Rect> {
        mem::take(&mut self.draw_cmd.borrow_mut().pending_damage)
    }

    /// Reports whether an element and its descendants all use local-v2 paint.
    ///
    /// Callers can use this to decide whether the retained tree owns every visual
    /// update below an element after rebuilding, recording, and syncing the tree
    /// for the current frame. A legacy or unresolved node returns `false`.
    #[doc(hidden)]
    pub fn subtree_uses_only_local_v2(
        &self,
        element: RenderNodeId,
    ) -> Result<bool, RenderTreeError> {
        let tree = self.draw_cmd.borrow();
        let Some(node) = tree.node(element) else {
            return Err(RenderTreeError::UnknownNode(element));
        };
        Ok(tree.subtree_uses_only_local_v2(node.id))
    }
}

/// The active node identity and command tree passed while an element records paint.
#[derive(Clone)]
pub struct CurrentBuildContext {
    draw_cmd: Shared<RefCell<DrawCommandList>>,
    element: RenderNodeId,
}

/// Internal commit lease for one element's retained draw list.
///
/// The public recorder lives in `aimer_canvas::Canvas`. This lease keeps list
/// reservation, scope validation, and commit behavior beside Cupid's retained
/// storage without exposing that storage to the canvas crate.
#[doc(hidden)]
pub struct DrawListWriter {
    owner: RenderNodeId,
    draw_cmd: Shared<RefCell<DrawCommandList>>,
    draw_list: Shared<RefCell<DrawList>>,
    finished: bool,
}

impl DrawListWriter {
    /// Commits the element-local commands and returns their new revision.
    #[doc(hidden)]
    pub fn commit(mut self, commands: Vec<DrawCommand>) -> Result<u64, RenderTreeError> {
        validate_state_scopes(&commands)?;
        let bounds = {
            let tree = self.draw_cmd.borrow();
            if tree.node(self.owner).is_none() {
                return Err(RenderTreeError::UnknownNode(self.owner));
            }
            tree.visible_node_bounds(self.owner)
        };
        let (revision, changed, blends_children) = {
            let mut draw_list = self.draw_list.borrow_mut();
            let had_morph = draw_list.morph.is_some();
            // A rebuilt element records again whether or not its picture changed.
            // The first recording always counts: revision 0 means nothing has
            // been drawn from this list yet.
            if draw_list.refreshed
                && draw_list.revision > 0
                && command_equality::commands_equal(&draw_list.commands, &commands)
            {
                draw_list.dirty = false;
                draw_list.stale = false;
                draw_list.refreshed = false;
                draw_list.recording = false;
                (draw_list.revision, false, false)
            } else {
            draw_list.image_textures = commands
                .iter()
                .filter_map(|command| match command {
                    DrawCommand::DrawImage { texture_id, .. } => Some(*texture_id),
                    DrawCommand::DrawImageWithResource { resource, .. } => {
                        Some(resource.texture_id())
                    }
                    _ => None,
                })
                .collect();
            draw_list.masks_descendants = matches!(
                commands.first(),
                Some(DrawCommand::DrawCustom { pipeline_name, .. })
                    if pipeline_name.as_ref() == COVERAGE_MASK_PIPELINE_NAME
            );
            draw_list.fades_descendants = matches!(
                commands.first(),
                Some(DrawCommand::DrawCustom { pipeline_name, .. })
                    if pipeline_name.as_ref() == FADE_MASK_PIPELINE_NAME
            );
            draw_list.morph = match commands.first() {
                Some(DrawCommand::DrawCustom { pipeline_name, data })
                    if pipeline_name.as_ref() == MORPH_PIPELINE_NAME =>
                {
                    MorphParams::decode(data)
                }
                _ => None,
            };
            draw_list.commands = Arc::from(commands);
            draw_list.revision = draw_list
                .revision
                .checked_add(1)
                .expect("draw-list revisions exhausted");
            draw_list.dirty = false;
            draw_list.stale = false;
            draw_list.refreshed = false;
            draw_list.recording = false;
            (draw_list.revision, true, had_morph || draw_list.morph.is_some())
            }
        };
        if changed {
            let mut tree = self.draw_cmd.borrow_mut();
            // A morph blends its children's pixels, which reach beyond the
            // node's own bounds (the outgoing picture is often larger than the
            // incoming one the node is sized to), so each frame of it, and the
            // frame that drops the marker, must repaint the whole subtree.
            let bounds = if blends_children {
                tree.visible_subtree_bounds(self.owner).or(bounds)
            } else {
                bounds
            };
            if let Some(bounds) = bounds {
                tree.push_damage(bounds);
            }
        }
        self.finished = true;
        Ok(revision)
    }
}

fn validate_state_scopes(commands: &[DrawCommand]) -> Result<(), RenderTreeError> {
    let mut clip_depth = 0usize;
    let mut transform_depth = 0usize;
    let mut alpha_set = false;
    let mut italic_set = false;
    let mut language_set = false;
    for command in commands {
        match command {
            DrawCommand::PushClip { .. } => clip_depth += 1,
            DrawCommand::PopClip => {
                let Some(depth) = clip_depth.checked_sub(1) else {
                    return Err(RenderTreeError::UnbalancedState);
                };
                clip_depth = depth;
            }
            DrawCommand::PushTransform { .. } => transform_depth += 1,
            DrawCommand::PopTransform => {
                let Some(depth) = transform_depth.checked_sub(1) else {
                    return Err(RenderTreeError::UnbalancedState);
                };
                transform_depth = depth;
            }
            DrawCommand::SetAlpha { .. } => alpha_set = true,
            DrawCommand::RestoreAlpha => alpha_set = false,
            DrawCommand::SetItalic { italic } => italic_set = *italic,
            DrawCommand::SetTextLanguage { language } => language_set = language.is_some(),
            DrawCommand::FillRect { .. }
            | DrawCommand::DrawText { .. }
            | DrawCommand::DrawRichText { .. }
            | DrawCommand::DrawTextDecoration { .. }
            | DrawCommand::DrawImage { .. }
            | DrawCommand::DrawImageWithResource { .. }
            | DrawCommand::Svg { .. }
            | DrawCommand::DrawCustom { .. }
            | DrawCommand::DrawShadowRect { .. }
            | DrawCommand::SetTransform { .. } => {}
        }
    }
    if clip_depth != 0 || transform_depth != 0 || alpha_set || italic_set || language_set {
        return Err(RenderTreeError::UnbalancedState);
    }
    Ok(())
}

impl Drop for DrawListWriter {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut draw_list = self.draw_list.borrow_mut();
        draw_list.recording = false;
        draw_list.dirty = true;
        draw_list.stale = true;
        draw_list.refreshed = false;
    }
}

/// One visible node in the render plan, in parent-first paint order.
pub struct RenderItem {
    /// Identity of the element to draw.
    pub element: RenderNodeId,
    /// World-space element bounds.
    pub bounds: Rect,
    /// Untransformed world-space origin for local commands.
    pub origin: (f32, f32),
    /// Cumulative transform mapping untransformed world coordinates to display coordinates.
    pub transform: Mat3,
    /// Effective world-space clip, if any.
    pub clip: Option<Rect>,
    /// Corner radii of the effective clip in world logical pixels: top-left,
    /// top-right, bottom-right, bottom-left.
    ///
    /// The renderer keeps one clip at a time. It intersects nested clip
    /// rectangles but takes the radii from the innermost clip, so this is the
    /// radius of the closest clip-owning node (the item's own node included).
    /// It is all zeros when `clip` is `None` or square.
    pub clip_radius: [f32; 4],
    /// Opacity applied directly to this node's commands.
    pub opacity: f32,
    /// Whether this node has recorded its local v2 commands yet.
    pub paint_source: RenderPaintSource,
    draw_list: Shared<RefCell<DrawList>>,
}

/// An ordered drawing or group-compositing operation.
pub enum RenderOp {
    /// Begins an offscreen group for a node with children.
    ///
    /// A plain group composites its items together once, at `opacity`. A
    /// `masked` group holds a node that opened its list with the
    /// [coverage-mask marker](COVERAGE_MASK_PIPELINE_NAME): that node's own
    /// item is the content, every other item in the group is coverage, and the
    /// group shows the content only where the coverage is, at `opacity`.
    /// A `fade` group holds a node that opened its list with the
    /// [fade-mask marker](FADE_MASK_PIPELINE_NAME): that node's own item is only
    /// alpha, every other item in the group is content, and the group shows the
    /// content scaled by that alpha, at `opacity`. `masked` and `fade` are never
    /// both set.
    BeginOpacityGroup {
        element: RenderNodeId,
        bounds: Rect,
        opacity: f32,
        clip: Option<Rect>,
        masked: bool,
        fade: bool,
        /// Set when the group belongs to a morph; see [`MORPH_PIPELINE_NAME`].
        /// A host group holds exactly its two source groups and no item of its
        /// own, and `masked` is false for it and for its sources.
        morph: Option<MorphGroup>,
    },
    /// Draws one element's local command list.
    ///
    /// Backends must initialize from the item's node state and restore that
    /// state after the list, so commands cannot affect sibling elements.
    Draw(RenderItem),
    /// Composites the current opacity group into its parent.
    EndOpacityGroup { element: RenderNodeId },
}

impl RenderItem {
    /// Returns the local commands and recording revision for this node.
    pub fn snapshot(&self) -> DrawListSnapshot {
        let draw_list = self.draw_list.borrow();
        DrawListSnapshot {
            revision: draw_list.revision,
            commands: draw_list.commands.clone(),
            image_textures: draw_list.image_textures.clone(),
        }
    }
}

/// Shared immutable snapshot of one node's retained local commands.
#[derive(Clone, Debug, Default)]
pub struct DrawListSnapshot {
    /// Monotonic local recording revision.
    pub revision: u64,
    /// Commands in element-local coordinates.
    pub commands: Arc<[DrawCommand]>,
    /// The textures `commands` draw directly, in command order with repeats.
    pub image_textures: Arc<[TextureId]>,
}

/// Damage and ordered render operations for one v2 frame.
pub struct RenderFrame {
    /// Device-independent regions that changed.
    pub damage: Vec<Rect>,
    /// Nodes to composite in paint order within those regions.
    pub operations: Vec<RenderOp>,
}
