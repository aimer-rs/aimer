//! Render-only scaling escapes ancestor clips. Keep this policy beside node
//! geometry so world bounds, damage, interaction and rendering agree.

use super::*;

impl RenderTree {
    /// Whether any retained node escapes ancestor clips.
    /// This constant-time query lets ordinary frames preserve offscreen culling.
    #[doc(hidden)]
    pub fn has_clip_escape(&self) -> bool {
        self.draw_cmd.borrow().clip_escape_count != 0
    }

    /// Allows this node and its descendants to escape all ancestor widget clips.
    ///
    /// Descendant-owned clips still apply. An unchanged policy preserves the
    /// geometry revision and pending damage.
    #[doc(hidden)]
    pub fn set_clip_escape(&self, element: RenderNodeId, enabled: bool) -> Result<(), RenderTreeError> {
        let mut tree = self.draw_cmd.borrow_mut();
        let node = tree.node(element).ok_or(RenderTreeError::UnknownNode(element))?;
        if node.escapes_clip == enabled {
            return Ok(());
        }
        let old = tree.subtree_damage_bounds(element);
        tree.node_mut(element).unwrap().escapes_clip = enabled;
        if enabled {
            tree.clip_escape_count += 1;
        } else {
            tree.clip_escape_count -= 1;
        }
        let new = tree.subtree_damage_bounds(element);
        if let Some(damage) = old.into_iter().chain(new).reduce(Rect::union) {
            tree.push_damage(damage);
        }
        Ok(())
    }
}

impl CurrentBuildContext {
    /// Changes the current node's ancestor-clip escape policy during state sync.
    #[doc(hidden)]
    pub fn set_clip_escape(&self, enabled: bool) -> bool {
        RenderTree { draw_cmd: self.draw_cmd.clone() }
            .set_clip_escape(self.element, enabled).is_ok()
    }
}

#[cfg(test)]
mod tests;
