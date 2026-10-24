//! The hand-off payload between the thread that builds a frame and the thread
//! that rasterizes it.

use std::sync::Arc;

use crate::damage_region::{DamageRect, DamageSet};
use crate::draw_cmd::DrawList;
use crate::draw_cmd_v2::{
    DrawCommand as V2DrawCommand, Mat3, MorphGroup, Rect as V2Rect, RenderNodeId,
};

/// Ordered local lists for damage-limited replay.
#[doc(hidden)]
pub struct RetainedRenderPlan {
    operations: Vec<RetainedRenderOperation>,
    complete: bool,
}

/// One retained item's bounds and paint payload in the direct replay plan.
#[doc(hidden)]
pub struct RetainedRenderOperation {
    pub(crate) kind: RetainedRenderOperationKind,
    pub(crate) bounds: DamageRect,
}

pub(crate) enum RetainedRenderOperationKind {
    /// `masked` groups show their first item only where the rest have coverage;
    /// `fade` groups show the rest scaled by the alpha of their first item.
    /// `morph` groups blend their two source groups with a shader.
    OpacityGroupBegin {
        element: u64,
        opacity: f32,
        masked: bool,
        fade: bool,
        morph: Option<MorphGroup>,
    },
    LocalV2(RetainedV2Item),
    OpacityGroupEnd {
        element: u64,
    },
}

pub(crate) struct RetainedV2Item {
    pub(crate) element: RenderNodeId,
    pub(crate) revision: u64,
    pub(crate) bounds: V2Rect,
    pub(crate) origin: (f32, f32),
    pub(crate) transform: Mat3,
    pub(crate) clip: Option<V2Rect>,
    /// Corner radii of `clip` in world logical pixels.
    pub(crate) clip_radius: [f32; 4],
    pub(crate) commands: Arc<[V2DrawCommand]>,
}

impl RetainedRenderPlan {
    pub(crate) fn new(operations: Vec<RetainedRenderOperation>, complete: bool) -> Self {
        Self {
            operations,
            complete,
        }
    }

    pub(crate) fn local_v2_operation(
        item: RetainedV2Item,
        bounds: DamageRect,
    ) -> RetainedRenderOperation {
        RetainedRenderOperation {
            kind: RetainedRenderOperationKind::LocalV2(item),
            bounds,
        }
    }

    pub(crate) fn opacity_group_begin_operation(
        element: u64,
        opacity: f32,
        masked: bool,
        fade: bool,
        morph: Option<MorphGroup>,
        bounds: DamageRect,
    ) -> RetainedRenderOperation {
        RetainedRenderOperation {
            kind: RetainedRenderOperationKind::OpacityGroupBegin {
                element,
                opacity,
                masked,
                fade,
                morph,
            },
            bounds,
        }
    }

    pub(crate) fn opacity_group_end_operation(
        element: u64,
        bounds: DamageRect,
    ) -> RetainedRenderOperation {
        RetainedRenderOperation {
            kind: RetainedRenderOperationKind::OpacityGroupEnd { element },
            bounds,
        }
    }

    /// Returns whether the ordered operations include the whole visible tree.
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns the retained list revision for one local-v2 node, if submitted.
    #[doc(hidden)]
    pub fn local_v2_revision(&self, element: RenderNodeId) -> Option<u64> {
        self.operations
            .iter()
            .find_map(|operation| match &operation.kind {
                RetainedRenderOperationKind::LocalV2(item) if item.element == element => {
                    Some(item.revision)
                }
                RetainedRenderOperationKind::OpacityGroupBegin { .. }
                | RetainedRenderOperationKind::OpacityGroupEnd { .. }
                | RetainedRenderOperationKind::LocalV2(_) => None,
            })
    }

    pub(crate) fn operations_for_region(
        &self,
        region: DamageRect,
    ) -> impl Iterator<Item = &RetainedRenderOperation> {
        self.operations
            .iter()
            .filter(move |operation| rects_intersect(operation.bounds, region))
    }
}

fn rects_intersect(left: DamageRect, right: DamageRect) -> bool {
    let left_right = left.x.saturating_add(left.width);
    let left_bottom = left.y.saturating_add(left.height);
    let right_right = right.x.saturating_add(right.width);
    let right_bottom = right.y.saturating_add(right.height);
    left.x < right_right && right.x < left_right && left.y < right_bottom && right.y < left_bottom
}

/// A finished, immutable frame: everything the renderer needs to encode and
/// present, and nothing else.
///
/// Building a frame walks the widget tree, which is full of `Rc` and `RefCell`
/// and therefore bound to the thread that owns it. A `Frame` deliberately
/// carries none of that — only the recorded draw commands and the surface
/// dimensions they were laid out for — which is what makes it `Send` and lets a
/// raster thread consume it while the UI thread moves on to the next frame.
///
/// The dimensions travel with the frame rather than being read back from the GPU
/// context at present time. A resize between build and present would otherwise
/// encode a frame laid out for the old size against the new one.
///
/// # Examples
///
/// ```
/// use aimer_cupid::draw_cmd::DrawList;
/// use aimer_cupid::frame::Frame;
///
/// let frame = Frame::new(DrawList::new(), 800, 600);
///
/// assert_eq!(frame.width, 800);
/// assert_eq!(frame.height, 600);
/// assert!(frame.is_empty());
///
/// // The whole payload can be moved to another thread.
/// std::thread::spawn(move || frame.draw_list.commands().len())
///     .join()
///     .unwrap();
/// ```
pub struct Frame {
    /// The commands recorded for this frame, owned outright.
    pub draw_list: DrawList,
    /// Surface width, in physical pixels, the frame was built for.
    pub width: u32,
    /// Surface height, in physical pixels, the frame was built for.
    pub height: u32,
}

/// Renderer metadata that travels with a finished frame.
#[doc(hidden)]
pub struct FrameRenderMetadata {
    device_scale_bits: u32,
    surface_identity: u64,
    renderer_generation: u64,
    context_generation: u64,
    resource_generation: u64,
    damage: DamageSet,
}

/// An owned frame plus the target and damage information needed by a raster
/// thread to choose between a full and a persistent-target repaint.
#[doc(hidden)]
pub struct FramePacket {
    frame: Frame,
    metadata: FrameRenderMetadata,
    scene: Option<crate::compositor::CompositorScene>,
    render_plan: Option<RetainedRenderPlan>,
}

impl Frame {
    /// Wraps a recorded draw list together with the surface size it targets.
    #[inline]
    pub fn new(draw_list: DrawList, width: u32, height: u32) -> Self {
        Self {
            draw_list,
            width,
            height,
        }
    }

    /// Returns `true` when the frame records no commands.
    ///
    /// An empty frame still has to be presented: skipping it would leave the
    /// previous frame's contents on screen.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.draw_list.commands().is_empty()
    }

    /// Consumes the frame and returns the draw list, so the caller can hand the
    /// buffer back to the canvas for reuse.
    #[inline]
    pub fn into_draw_list(self) -> DrawList {
        self.draw_list
    }
}

impl FrameRenderMetadata {
    /// Creates metadata for a conservative full repaint.
    #[inline]
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            device_scale_bits: 1.0_f32.to_bits(),
            surface_identity: 0,
            renderer_generation: 0,
            context_generation: 0,
            resource_generation: 0,
            damage: DamageSet::full(width, height),
        }
    }

    /// Creates metadata for a frame whose damage was collected by the widget
    /// renderer.
    #[inline]
    pub fn new(
        device_scale: f32,
        surface_identity: u64,
        renderer_generation: u64,
        context_generation: u64,
        resource_generation: u64,
        damage: DamageSet,
    ) -> Self {
        Self {
            device_scale_bits: device_scale.to_bits(),
            surface_identity,
            renderer_generation,
            context_generation,
            resource_generation,
            damage,
        }
    }

    /// Returns the device scale captured when the frame was built.
    #[inline]
    pub fn device_scale(&self) -> f32 {
        f32::from_bits(self.device_scale_bits)
    }

    /// Returns the identity of the presentation surface.
    #[inline]
    pub fn surface_identity(&self) -> u64 {
        self.surface_identity
    }

    /// Returns the renderer generation that produced the frame.
    #[inline]
    pub fn renderer_generation(&self) -> u64 {
        self.renderer_generation
    }

    /// Returns the GPU context generation that produced the frame.
    #[inline]
    pub fn context_generation(&self) -> u64 {
        self.context_generation
    }

    /// Returns the resource generation associated with the frame target.
    #[inline]
    pub fn resource_generation(&self) -> u64 {
        self.resource_generation
    }

    /// Returns the normalized target damage.
    #[inline]
    pub fn damage(&self) -> &DamageSet {
        &self.damage
    }

    /// Returns metadata with a renderer-computed damage contract.
    ///
    /// Scene diffing uses this at the raster seam to union old and new node
    /// footprints without mutating the immutable packet supplied by the UI
    /// thread.
    #[doc(hidden)]
    #[inline]
    pub fn with_damage(&self, damage: DamageSet) -> Self {
        Self {
            device_scale_bits: self.device_scale_bits,
            surface_identity: self.surface_identity,
            renderer_generation: self.renderer_generation,
            context_generation: self.context_generation,
            resource_generation: self.resource_generation,
            damage,
        }
    }
}

impl FramePacket {
    /// Wraps a frame with explicit renderer metadata.
    #[inline]
    pub fn new(frame: Frame, metadata: FrameRenderMetadata) -> Self {
        Self {
            frame,
            metadata,
            scene: None,
            render_plan: None,
        }
    }

    /// Wraps a frame, metadata, and immutable compositor scene.
    #[doc(hidden)]
    #[inline]
    pub fn with_scene(
        frame: Frame,
        metadata: FrameRenderMetadata,
        scene: crate::compositor::CompositorScene,
    ) -> Self {
        Self {
            frame,
            metadata,
            scene: Some(scene),
            render_plan: None,
        }
    }

    /// Wraps a frame, renderer metadata, compositor scene, and retained replay plan.
    #[doc(hidden)]
    #[inline]
    pub fn with_render_plan(
        frame: Frame,
        metadata: FrameRenderMetadata,
        scene: Option<crate::compositor::CompositorScene>,
        render_plan: Option<RetainedRenderPlan>,
    ) -> Self {
        Self {
            frame,
            metadata,
            scene,
            render_plan,
        }
    }

    /// Wraps a frame in the compatibility full-repaint policy.
    #[inline]
    pub fn full(frame: Frame) -> Self {
        let metadata = FrameRenderMetadata::full(frame.width, frame.height);
        Self {
            frame,
            metadata,
            scene: None,
            render_plan: None,
        }
    }

    /// Borrows the immutable frame payload.
    #[inline]
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Borrows the renderer metadata.
    #[inline]
    pub fn metadata(&self) -> &FrameRenderMetadata {
        &self.metadata
    }

    /// Borrows optional compositor metadata attached to this packet.
    #[inline]
    pub fn scene(&self) -> Option<&crate::compositor::CompositorScene> {
        self.scene.as_ref()
    }

    /// Borrows the retained ordered local lists and legacy ranges when this frame was built from v2.
    #[doc(hidden)]
    #[inline]
    pub fn render_plan(&self) -> Option<&RetainedRenderPlan> {
        self.render_plan.as_ref()
    }

    /// Removes and returns the retained render plan for transfer to a canvas frame builder.
    #[doc(hidden)]
    #[inline]
    pub fn take_render_plan(&mut self) -> Option<RetainedRenderPlan> {
        self.render_plan.take()
    }

    /// Consumes the packet and returns its frame payload.
    #[inline]
    pub fn into_frame(self) -> Frame {
        self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utilities::{Color, Rect};

    #[test]
    fn frame_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Frame>();
        assert_send::<FramePacket>();
    }

    #[test]
    fn frame_keeps_the_size_it_was_built_for() {
        let frame = Frame::new(DrawList::new(), 1280, 720);

        assert_eq!((frame.width, frame.height), (1280, 720));
    }

    #[test]
    fn an_empty_frame_is_reported_as_empty() {
        let mut list = DrawList::new();

        assert!(Frame::new(DrawList::new(), 1, 1).is_empty());

        list.fill_rect(
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );

        assert!(!Frame::new(list, 1, 1).is_empty());
    }

    #[test]
    fn a_compatibility_packet_requests_a_full_repaint() {
        let packet = FramePacket::full(Frame::new(DrawList::new(), 12, 8));

        assert!(packet.metadata().damage().is_full());
        assert_eq!(packet.metadata().damage().target_size(), (12, 8));
    }

    #[test]
    fn a_packet_can_carry_an_ordered_compositor_scene() {
        let scene = crate::compositor::CompositorScene::new(12, 8);
        let packet = FramePacket::with_scene(
            Frame::new(DrawList::new(), 12, 8),
            FrameRenderMetadata::full(12, 8),
            scene,
        );

        assert_eq!(
            packet
                .scene()
                .map(crate::compositor::CompositorScene::target_size),
            Some((12, 8))
        );
    }

    #[test]
    fn retained_plan_returns_only_nodes_intersecting_each_damage_region() {
        let tree = crate::draw_cmd_v2::RenderTree::new();
        let left_node = tree.add_root(V2Rect::new(0.0, 0.0, 8.0, 8.0)).unwrap();
        let right_node = tree.add_root(V2Rect::new(16.0, 0.0, 8.0, 8.0)).unwrap();
        let item = |element| RetainedV2Item {
            element,
            revision: 1,
            bounds: V2Rect::new(0.0, 0.0, 8.0, 8.0),
            origin: (0.0, 0.0),
            transform: Mat3::identity(),
            clip: None,
            clip_radius: [0.0; 4],
            commands: Arc::from(Vec::new()),
        };
        let plan = RetainedRenderPlan::new(
            vec![
                RetainedRenderPlan::local_v2_operation(
                    item(left_node),
                    DamageRect::new(0, 0, 8, 8),
                ),
                RetainedRenderPlan::local_v2_operation(
                    item(right_node),
                    DamageRect::new(16, 0, 8, 8),
                ),
            ],
            true,
        );

        let left = plan
            .operations_for_region(DamageRect::new(2, 2, 2, 2))
            .collect::<Vec<_>>();
        let right = plan
            .operations_for_region(DamageRect::new(18, 2, 2, 2))
            .collect::<Vec<_>>();

        assert_eq!(left.len(), 1);
        assert_eq!(right.len(), 1);
        assert!(matches!(
            &left[0].kind,
            RetainedRenderOperationKind::LocalV2(item) if item.element == left_node
        ));
        assert!(matches!(
            &right[0].kind,
            RetainedRenderOperationKind::LocalV2(item) if item.element == right_node
        ));
        assert!(plan.is_complete());
    }
}
