use std::collections::{HashMap, HashSet};

use crate::damage_region::DamageSet;

use super::{
    CompositorScene, SceneChange, SceneChangeKind, SceneDiff, SceneNode, SceneNodeId,
    ScenePresence, push_scene_change, scene_change_kind,
};

/// The result of committing one immutable frame scene into a retained scene
/// tree.
///
/// The tree keeps the last accepted properties by [`SceneNodeId`]. Unchanged
/// nodes remain in place; only added or changed nodes are copied from the
/// frame packet. The [`SceneDiff`] remains the damage contract used by the
/// renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneCommit {
    diff: SceneDiff,
    updated_nodes: usize,
    reused_nodes: usize,
    removed_nodes: usize,
}

impl SceneCommit {
    /// Returns property changes and conservative damage for this commit.
    #[inline]
    pub fn diff(&self) -> &SceneDiff {
        &self.diff
    }

    /// Returns the number of nodes inserted or replaced in the retained tree.
    #[inline]
    pub const fn updated_nodes(&self) -> usize {
        self.updated_nodes
    }

    /// Returns the number of current nodes whose retained properties were
    /// reused without replacement.
    #[inline]
    pub const fn reused_nodes(&self) -> usize {
        self.reused_nodes
    }

    /// Returns the number of old nodes removed by this commit.
    #[inline]
    pub const fn removed_nodes(&self) -> usize {
        self.removed_nodes
    }

    /// Consumes the commit and returns its diff.
    #[inline]
    pub fn into_diff(self) -> SceneDiff {
        self.diff
    }
}

/// Renderer-side retained property tree for one target identity.
///
/// This module owns scene property retention, not widget state. A frame packet
/// supplies an immutable scene transaction; `commit` applies only changed
/// node records and leaves unchanged records in the tree. Surface pixels are
/// retained separately by the renderer's surface cache and can therefore be
/// composed when only a node transform, opacity, or clip changes.
#[derive(Clone, Debug, Default)]
pub struct RetainedSceneTree {
    target_size: Option<(u32, u32)>,
    recorded: bool,
    roots: Vec<SceneNodeId>,
    nodes: Vec<SceneNode>,
    index: HashMap<SceneNodeId, usize>,
}

impl RetainedSceneTree {
    /// Creates an empty retained scene tree.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops all retained properties, forcing the next scene to be a first
    /// frame for damage purposes.
    #[inline]
    pub fn clear(&mut self) {
        self.target_size = None;
        self.recorded = false;
        self.roots.clear();
        self.nodes.clear();
        self.index.clear();
    }

    /// Commits a frame scene and updates only changed node properties.
    ///
    /// A target-size or recording-mode change starts a new scene transaction
    /// and conservatively reports a full repaint. Invalid scenes are retained
    /// only as the latest conservative state when a caller explicitly commits
    /// one; normal renderer paths commit recorded scenes only.
    pub fn commit(&mut self, scene: &CompositorScene) -> SceneCommit {
        let compatible = self.target_size == Some(scene.target_size())
            && self.recorded == scene.is_recorded()
            && self.target_size.is_some();
        let mut damage = DamageSet::new(scene.target_size().0, scene.target_size().1);
        let mut changes = Vec::new();
        let mut changed_ids = HashSet::new();
        let mut current_ids = HashSet::with_capacity(scene.nodes().len());
        let mut updated_nodes = 0;
        let mut reused_nodes = 0;
        let removed_nodes;

        if !compatible {
            damage.mark_full();
            for node in scene.nodes() {
                current_ids.insert(node.id());
                changes.push(SceneChange {
                    id: node.id(),
                    kind: SceneChangeKind::Added,
                    old_bounds: None,
                    new_bounds: node.effective_bounds(),
                });
                updated_nodes += 1;
            }
            removed_nodes = self
                .nodes
                .iter()
                .filter(|node| !current_ids.contains(&node.id()))
                .count();
        } else {
            for node in scene.nodes() {
                current_ids.insert(node.id());
                let Some(&old_index) = self.index.get(&node.id()) else {
                    push_scene_change(
                        &mut changes,
                        &mut damage,
                        SceneChange {
                            id: node.id(),
                            kind: SceneChangeKind::Added,
                            old_bounds: None,
                            new_bounds: node.effective_bounds(),
                        },
                    );
                    changed_ids.insert(node.id());
                    updated_nodes += 1;
                    continue;
                };
                let old = &self.nodes[old_index];
                if let Some(kind) = scene_change_kind(old, node, &self.roots, scene.roots()) {
                    push_scene_change(
                        &mut changes,
                        &mut damage,
                        SceneChange {
                            id: node.id(),
                            kind,
                            old_bounds: old.effective_bounds(),
                            new_bounds: node.effective_bounds(),
                        },
                    );
                    changed_ids.insert(node.id());
                    updated_nodes += 1;
                } else {
                    reused_nodes += 1;
                }
            }

            for old in &self.nodes {
                if !current_ids.contains(&old.id()) && old.presence() != ScenePresence::Culled {
                    push_scene_change(
                        &mut changes,
                        &mut damage,
                        SceneChange {
                            id: old.id(),
                            kind: SceneChangeKind::Removed,
                            old_bounds: old.effective_bounds(),
                            new_bounds: None,
                        },
                    );
                }
            }
            removed_nodes = self
                .nodes
                .iter()
                .filter(|node| {
                    !current_ids.contains(&node.id()) && node.presence() != ScenePresence::Culled
                })
                .count();
        }

        if !compatible {
            self.nodes.clear();
            self.index.clear();
            self.nodes.reserve(scene.nodes().len());
            for node in scene.nodes() {
                self.index.insert(node.id(), self.nodes.len());
                self.nodes.push(node.clone());
            }
        } else {
            for node in scene.nodes() {
                match self.index.get(&node.id()).copied() {
                    Some(index) if changed_ids.contains(&node.id()) => {
                        self.nodes[index] = node.clone();
                    }
                    Some(_) => {}
                    None => {
                        self.index.insert(node.id(), self.nodes.len());
                        self.nodes.push(node.clone());
                    }
                }
            }
            self.nodes.retain(|node| current_ids.contains(&node.id()));
            self.index.clear();
            for (index, node) in self.nodes.iter().enumerate() {
                self.index.insert(node.id(), index);
            }
        }

        self.target_size = Some(scene.target_size());
        self.recorded = scene.is_recorded();
        self.roots.clear();
        self.roots.extend_from_slice(scene.roots());

        changes.sort_unstable_by_key(|change| change.id.get());
        let diff = SceneDiff {
            full: damage.is_full(),
            changes,
            damage,
        };
        SceneCommit {
            diff,
            updated_nodes,
            reused_nodes,
            removed_nodes,
        }
    }

    /// Returns the retained node for an identity, if present.
    #[inline]
    pub fn node(&self, id: SceneNodeId) -> Option<&SceneNode> {
        self.index.get(&id).and_then(|index| self.nodes.get(*index))
    }

    /// Returns retained nodes in their last committed order.
    #[inline]
    pub fn nodes(&self) -> &[SceneNode] {
        &self.nodes
    }

    /// Returns the last committed root order.
    #[inline]
    pub fn roots(&self) -> &[SceneNodeId] {
        &self.roots
    }

    /// Returns the last committed target size, if any.
    #[inline]
    pub const fn target_size(&self) -> Option<(u32, u32)> {
        self.target_size
    }
}
