use aimer_attribute::{Dimension, ResolvedSize, Size, Vec2d};

use crate::base::BuildContext;
use crate::components::element::VisitorElement;

/// The sizing and placement half of an element.
///
/// Layout in Aimer is constraint-driven: a parent gives each child a
/// [`BuildContext`] whose `box_constraint` and `parent_size` describe the room
/// available, and the child answers with a [`ResolvedSize`]. This trait is how
/// an element answers.
///
/// Every method has a default, so a purely decorative element can implement it
/// with an empty `impl`. The defaults resolve the element's declared
/// [`size`](LayoutElement::size) against the constraint and fall back to the
/// parent's size when none is declared. An element with real layout work (a
/// flex container, a scroll view) overrides
/// [`computed_size`](LayoutElement::computed_size) or
/// [`layout`](LayoutElement::layout).
///
/// Measuring can be expensive, so containers cache what they measure. An
/// element whose size can change must say so by being invalidated
/// ([`invalidate_layout`](LayoutElement::invalidate_layout)); one whose size
/// provably cannot opts in to retention with
/// [`is_layout_stable`](LayoutElement::is_layout_stable).
pub trait LayoutElement: VisitorElement {
    /// Returning the position of the element inside their parent
    fn pos(&self) -> Option<Vec2d> {
        None
    }

    /// size of the element
    fn size(&self) -> Option<Size> {
        None
    }

    /// Lays the element out under `ctx` and returns the size it occupies.
    ///
    /// This is the entry point a parent calls on each child. The default
    /// returns [`computed_size`](LayoutElement::computed_size); override this
    /// instead when laying out has effects beyond measuring, such as placing
    /// the element's own children.
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    /// calculate the size after apply layout
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.size()
            .map(|s| {
                s.resolve(
                    &ResolvedSize {
                        width: ctx.box_constraint.max_width,
                        height: ctx.box_constraint.max_height,
                    },
                    ctx.scale,
                )
            })
            .unwrap_or(ctx.parent_size)
    }

    /// i don't know why this appear here :) just for the shorter ?
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    /// get the layer of the element (used for z-index ordering)
    fn layer(&self) -> u32 {
        0
    }

    /// The flex factor of this element when it lives inside a flex container
    /// (`Row`, `Column`, `Flex`).
    ///
    /// Returning `Some(factor)` marks the element as *flexible*: the flex
    /// parent gives it a share of the remaining main-axis space
    /// proportional to `factor` (see `Expanded`). Regular elements return
    /// `None` and are laid out according to their own size.
    fn flex(&self) -> Option<f32> {
        None
    }

    /// Returns whether this element's resolved size is independent of its
    /// descendants and runtime state for the same constraint and scale.
    ///
    /// Layout containers use this opt-in to retain a measured child size when
    /// an unrelated generated subtree advances the global element-tree
    /// generation. Implementations must return `true` only when the answer to
    /// [`LayoutElement::computed_size`] cannot change without replacing the
    /// element itself.
    #[inline]
    fn is_layout_stable(&self) -> bool {
        false
    }

    /// get the size from the child when parent has no size explicit
    fn get_size_from_child(&self) -> Option<Size> {
        if let Some(s) = self.size() {
            return Some(s);
        }

        let mut result_w = Dimension::Auto;
        let mut result_h = Dimension::Auto;
        let mut found = false;

        self.visit_children(&mut |item| {
            if let Some(child_size) = item.size().or_else(|| item.get_size_from_child()) {
                // For Px values, take the max; otherwise keep what we have
                result_w = match (result_w, child_size.width) {
                    (Dimension::Px(a), Dimension::Px(b)) => Dimension::Px(a.max(b)),
                    (Dimension::Auto, w) => w,
                    (w, _) => w,
                };
                result_h = match (result_h, child_size.height) {
                    (Dimension::Px(a), Dimension::Px(b)) => Dimension::Px(a.max(b)),
                    (Dimension::Auto, h) => h,
                    (h, _) => h,
                };
                found = true;
            }
        });

        if found {
            Some(Size {
                width: result_w,
                height: result_h,
            })
        } else {
            None
        }
    }

    /// Invalidate cached layout data for this element and all children.
    /// Called at the start of each frame to ensure fresh layout.
    fn invalidate_layout(&self) {
        self.visit_children(&mut |child| {
            child.invalidate_layout();
        });
    }

    /// Returns the element's top-left and bottom-right corners, from its
    /// declared [`pos`](LayoutElement::pos) and [`size`](LayoutElement::size).
    ///
    /// `None` when either is missing. Only a fixed (`Px`) dimension contributes
    /// to the extent: an `Auto` or relative dimension counts as zero here, since
    /// resolving it needs a constraint this method does not have.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        if self.size().is_none() || self.pos().is_none() {
            return None;
        }
        let start = self.pos().unwrap();
        let size = self.size().unwrap();
        let resolved = ResolvedSize {
            width: match size.width {
                Dimension::Px(v) => v,
                _ => 0.0,
            },
            height: match size.height {
                Dimension::Px(v) => v,
                _ => 0.0,
            },
        };
        let end = start.get_end(resolved);
        Some((start, end))
    }

    /// Returns stable bounds that the sparse event tree can cache for this
    /// element. The default uses [`Self::pos_start_end`]. Return `None` when
    /// screen-space bounds can move during painting without invalidating
    /// layout; the event tree will retain the target as a candidate and live
    /// hit testing can still reject events during dispatch.
    #[inline]
    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.pos_start_end()
    }
    // fn layer(&self) -> u32;
}
