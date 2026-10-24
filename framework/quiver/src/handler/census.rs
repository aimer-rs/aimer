//! Diagnostic census of how a window's render nodes obtain their paint.
//!
//! A node records its own commands (`LocalV2`, possibly an empty list for an
//! element that paints nothing) or has not been painted yet (`Unresolved`). The
//! census reports the second group by its topmost nodes, so a wrapper that is
//! never drawn cannot hide behind a healthy looking `LocalV2` count.
//!
//! The frame loop reports it under the `frame-stats` feature; otherwise only
//! tests call it.
#![cfg_attr(not(any(test, feature = "frame-stats")), allow(dead_code))]

use super::WindowRenderTree;
use aimer_cupid::draw_cmd_v2::RenderPaintSource;
use aimer_widget::{Element, ElementId};

/// One element a census entry points at, with its type for diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CensusElement {
    /// The element's identity.
    pub element: ElementId,
    /// The element's `debug_name`.
    pub debug_name: &'static str,
}

/// Counts of render nodes by paint source for one synchronized tree.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PaintSourceCensus {
    /// Nodes that record their own v2 commands.
    pub local_v2: usize,
    /// Nodes that have not recorded a paint source yet.
    pub unresolved: usize,
    /// Of the unresolved nodes, those the render tree does not know yet: they
    /// were created after the last structure sync, so no node exists to
    /// receive a paint source.
    pub unmapped: usize,
    /// The tops of unresolved regions: unresolved nodes whose parent is not
    /// itself unresolved. A node that is never drawn stays unresolved and
    /// paints nothing, so these name the elements whose draw never ran.
    pub unresolved_roots: Vec<CensusElement>,
    /// Elements that ran `draw` under retained presentation and still have no
    /// render node after the frame's final structure sync. Whatever they paint
    /// is dropped; a parent that draws a child without exposing it through
    /// `visit_children` causes this.
    pub drawn_unmapped: Vec<CensusElement>,
    /// Elements that had no retained paint to show in some frame: they
    /// declined (`can_paint_local_v2` was false) or their state was not valid
    /// enough to paint. They paint nothing. Debug builds only.
    pub declined_paint: Vec<CensusElement>,
    /// Why the last render-tree synchronization failed (the error's name), if
    /// it did. A failed sync sends the whole frame down the full legacy repaint
    /// path, so every element is "drawn without a render node".
    pub sync_error: Option<String>,
    /// Elements whose bounds or clip were not finite at the last sync. They
    /// were collapsed to an empty rectangle so the sync could proceed; they
    /// paint nothing, which is usually a layout bug (for example a percentage of
    /// an unbounded axis).
    pub invalid_bounds: Vec<CensusElement>,
}

impl WindowRenderTree {
    /// Logs the census, and the element types behind every island, whenever it
    /// differs from the last reported one.
    #[cfg(feature = "frame-stats")]
    pub(crate) fn report_census_change(&mut self, root: &dyn Element) {
        let census = self.paint_source_census(root);
        if self.last_census.as_ref() == Some(&census) {
            return;
        }
        let unresolved: Vec<&'static str> = census
            .unresolved_roots
            .iter()
            .map(|root| root.debug_name)
            .collect();
        let drawn_unmapped: Vec<&'static str> = census
            .drawn_unmapped
            .iter()
            .map(|root| root.debug_name)
            .collect();
        aimer_utils::debug!(
            "render census: local_v2={} unresolved={} (unmapped={}) unresolved_roots={:?} drawn_unmapped={:?} sync_error={:?}",
            census.local_v2,
            census.unresolved,
            census.unmapped,
            unresolved,
            drawn_unmapped,
            census.sync_error
        );
        self.last_census = Some(census);
    }

    /// Counts this tree's nodes by paint source, walking `root` in retained
    /// paint order.
    pub(crate) fn paint_source_census(&self, root: &dyn Element) -> PaintSourceCensus {
        let mut census = PaintSourceCensus::default();
        self.census_node(root, false, &mut census);
        census.drawn_unmapped = self.drawn_unmapped.clone();
        census.declined_paint = self.declined_paint.clone();
        census.sync_error = self.sync_error.map(|error| format!("{error:?}"));
        census.invalid_bounds = self.invalid_bounds.clone();
        census
    }

    /// Keeps the elements that were drawn this frame but are *still* without a
    /// render node after the final structure sync. Call it once per frame,
    /// after that sync: an element created during the draw itself is mapped by
    /// then and is not a problem.
    pub(crate) fn settle_unmapped_draws(&mut self) {
        let still_unmapped: Vec<CensusElement> = aimer_widget::take_unmapped_draws()
            .into_iter()
            .filter(|(id, _)| self.node_for_element(*id).is_none())
            .map(|(element, debug_name)| CensusElement {
                element,
                debug_name,
            })
            .collect();
        self.drawn_unmapped = still_unmapped;
        for (element, debug_name) in aimer_widget::take_declined_paint_elements() {
            if !self
                .declined_paint
                .iter()
                .any(|seen| seen.element == element)
            {
                self.declined_paint.push(CensusElement {
                    element,
                    debug_name,
                });
            }
        }
    }

    // Uses the same retained paint-order traversal as `collect_render_tree_nodes`
    // so the census sees exactly the nodes the render tree was built from.
    fn census_node(
        &self,
        element: &dyn Element,
        parent_unresolved: bool,
        census: &mut PaintSourceCensus,
    ) {
        let mut unresolved = false;
        let node = self.node_for_element(element.id());
        if node.is_none() {
            census.unmapped += 1;
        }
        match node.and_then(|node| self.tree.paint_source(node).ok()) {
            Some(RenderPaintSource::LocalV2) => census.local_v2 += 1,
            Some(RenderPaintSource::Unresolved) | None => {
                census.unresolved += 1;
                unresolved = true;
                if !parent_unresolved {
                    census.unresolved_roots.push(CensusElement {
                        element: element.id(),
                        debug_name: element.debug_name(),
                    });
                }
            }
        }
        element.visit_retained_v2_children(&mut |_, child| {
            self.census_node(child, unresolved, census);
        });
    }
}
