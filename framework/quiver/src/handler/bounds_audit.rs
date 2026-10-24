//! Differential audit of cached interaction bounds against the render tree.
//!
//! Many elements publish their hit-test rectangle while they are updated, by
//! reading the legacy canvas transform stack (`pos_start_end`). The render tree
//! already holds every node's world rectangle, derived from layout. Replacing
//! the first source with the second is only safe where they agree, so this audit
//! lists, for a synchronized tree, every element whose cached rectangle differs
//! from its node's world rectangle by more than a pixel. Both are in logical
//! pixels: `CacheBounds` divides by the scale when it saves.
#![cfg_attr(not(any(test, feature = "frame-stats")), allow(dead_code))]

use super::WindowRenderTree;
use aimer_widget::{Element, ElementId};

/// Largest disagreement, in logical pixels, that still counts as the same box.
const TOLERANCE: f32 = 1.0;

/// An element whose cached rectangle and render-tree rectangle disagree.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundsMismatch {
    /// The element's identity.
    pub element: ElementId,
    /// The element's `debug_name`.
    pub debug_name: &'static str,
    /// The element's cached `(left, top, right, bottom)`, in logical pixels.
    pub cached: [f32; 4],
    /// The node's world `(left, top, right, bottom)`, in logical pixels.
    pub render: [f32; 4],
}

/// The result of comparing one synchronized tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundsAudit {
    /// Elements that published a rectangle and had a render node to compare.
    pub compared: usize,
    /// Elements that published a rectangle but have no render node.
    pub unmapped: usize,
    /// Compared elements that disagree.
    pub mismatches: Vec<BoundsMismatch>,
    /// How many elements took their hit area from the render tree at the last
    /// sync. A gate over `interaction_disagreements` is only meaningful when
    /// this is non-zero.
    pub interaction_adopted: usize,
    /// [`Self::interaction_adopted`] by element `debug_name`.
    pub interaction_adopted_by_type: std::collections::BTreeMap<&'static str, usize>,
    /// Elements that already take their hit area from the render tree and whose
    /// last canvas measurement differs from it now. This is the gate
    /// for retiring the canvas transform stack: it must stay empty.
    pub interaction_disagreements: Vec<aimer_widget::InteractionDisagreement>,
}

impl WindowRenderTree {
    /// Compares every cached element rectangle under `root` with its node's
    /// world rectangle.
    pub(crate) fn bounds_audit(&self, root: &dyn Element) -> BoundsAudit {
        let mut audit = BoundsAudit {
            interaction_adopted: self.interaction_adopted,
            interaction_adopted_by_type: self.interaction_adopted_by_type.clone(),
            ..BoundsAudit::default()
        };
        self.audit_node(root, &mut audit);
        audit
    }

    fn audit_node(&self, element: &dyn Element, audit: &mut BoundsAudit) {
        if let Some(disagreement) = element.retained_v2_interaction_disagreement() {
            audit.interaction_disagreements.push(disagreement);
        }
        if let Some((start, end)) = element.pos_start_end() {
            match self
                .node_for_element(element.id())
                .and_then(|node| self.tree.element_bounds(node).ok())
            {
                Some(world) => {
                    audit.compared += 1;
                    let render = [
                        world.x,
                        world.y,
                        world.x + world.width,
                        world.y + world.height,
                    ];
                    let cached = [start.x, start.y, end.x, end.y];
                    if cached
                        .iter()
                        .zip(render)
                        .any(|(cached, render)| !((cached - render).abs() <= TOLERANCE))
                    {
                        audit.mismatches.push(BoundsMismatch {
                            element: element.id(),
                            debug_name: element.debug_name(),
                            cached,
                            render,
                        });
                    }
                }
                None => audit.unmapped += 1,
            }
        }
        element.visit_retained_v2_children(&mut |_, child| self.audit_node(child, audit));
    }
}
