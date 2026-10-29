//! Keeps the platform's accessibility tree in step with the widget tree.
//!
//! Nothing here runs for an application no assistive technology has looked at:
//! the platform reports whether it was queried, and until it was the whole of
//! this module is one lookup per frame.

use aimer_accessibility::Bounds;
use aimer_platform::accessibility;
use aimer_widget::Widget;
use aimer_widget::semantics::{collect_semantic_tree, perform_semantic_action};

use crate::handler::AimerApplicationHandler;

impl<W: Widget + 'static> AimerApplicationHandler<W> {
    /// Performs the actions assistive technology requested, then republishes the
    /// semantic tree if the widgets or their geometry moved since it was last
    /// published.
    ///
    /// Called when a frame ends and when the platform reports a query or an
    /// action, so a screen reader that attaches to a static screen still gets a
    /// tree without waiting for the next frame.
    pub(crate) fn sync_accessibility(&mut self) {
        let Some(window) = self.native_window().cloned() else {
            return;
        };
        if !accessibility::is_requested(&window) {
            return;
        }
        let actions = accessibility::take_actions(&window);
        if let Some(root) = &self.widget_root {
            let mut handled = false;
            for request in actions {
                handled |= perform_semantic_action(&**root, request.node_id(), request.action());
            }
            if handled {
                self.request_full_redraw();
            }
        }

        let Some(root) = &self.widget_root else {
            return;
        };
        let tree = self.render_tree.tree();
        let stamp = (aimer_widget::element_tree_generation(), tree.geometry_revision());
        if self.render_tree.accessibility_stamp == Some(stamp) {
            return;
        }
        let render_tree = &self.render_tree;
        let semantic = collect_semantic_tree(&**root, &mut |id| {
            let node = render_tree.node_for_element(id)?;
            let rect = render_tree.tree().element_bounds(node).ok()?;
            Bounds::new(rect.x, rect.y, rect.width, rect.height).ok()
        });
        match semantic.snapshot() {
            Ok(snapshot) => {
                accessibility::publish(&window, &snapshot);
                self.render_tree.accessibility_stamp = Some(stamp);
            }
            Err(error) => {
                aimer_utils::debug!("the semantic tree was not published: {error:?}");
            }
        }
    }
}
