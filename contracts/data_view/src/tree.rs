//! Stable tree identity, expansion, lazy children, and keyboard traversal.

use std::collections::HashMap;
use std::hash::Hash;

/// The state of a node's child source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildLoadState {
    /// Children are known and can be traversed.
    Loaded,
    /// The node may have children, but loading has not started.
    Unloaded,
    /// A child request is in flight.
    Loading,
    /// The most recent child request failed.
    Error,
}

/// Keyboard commands understood by [`TreeView::handle_key`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeKey {
    /// Move focus to the previous visible node.
    ArrowUp,
    /// Move focus to the next visible node.
    ArrowDown,
    /// Collapse a node or move focus to its parent.
    ArrowLeft,
    /// Expand a node, request lazy children, or move into its first child.
    ArrowRight,
    /// Focus the first visible node.
    Home,
    /// Focus the last visible node.
    End,
    /// Activate the focused node.
    Enter,
    /// Activate the focused node through the alternate keyboard action.
    Space,
}

/// The observable result of one tree keyboard command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeKeyResult<K> {
    /// The command had no effect.
    Noop,
    /// Focus moved to the contained stable node key.
    Focused(K),
    /// The contained node became expanded.
    Expanded(K),
    /// The contained node became collapsed.
    Collapsed(K),
    /// Lazy children were requested for the contained node.
    RequestChildren(K),
    /// The contained node was activated.
    Activated(K),
}

/// A node description supplied when lazy children finish loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNodeSpec<K, T> {
    id: K,
    label: T,
}

impl<K, T> TreeNodeSpec<K, T> {
    /// Creates a lazy-child description with a stable node ID.
    pub fn new(id: K, label: T) -> Self {
        Self { id, label }
    }

    /// Returns the stable node ID.
    pub fn id(&self) -> &K {
        &self.id
    }

    /// Returns the node label/value.
    pub fn label(&self) -> &T {
        &self.label
    }

    /// Splits the description into its ID and label.
    pub fn into_parts(self) -> (K, T) {
        (self.id, self.label)
    }
}

/// A tree operation referred to an invalid identity or transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError<K> {
    /// A node ID already exists in the tree.
    DuplicateId(K),
    /// A referenced node does not exist.
    MissingNode(K),
    /// The proposed parent/child relationship would create a cycle.
    Cycle {
        /// The proposed parent ID.
        parent: K,
        /// The proposed child ID, which is already an ancestor.
        child: K,
    },
    /// A node already has a parent and cannot be attached a second time.
    AlreadyAttached(K),
    /// A node already owns children and cannot be changed into a lazy source.
    AlreadyHasChildren(K),
    /// A node's child source is not in the required state for this operation.
    InvalidChildState(K),
    /// A lazy completion was attempted without an active request.
    NotLoading(K),
    /// A lazy batch contains a duplicate node ID.
    DuplicateChildId(K),
}

struct Node<K, T> {
    label: T,
    parent: Option<K>,
    children: Vec<K>,
    child_state: ChildLoadState,
    child_error: Option<String>,
    expanded: bool,
}

/// A keyed tree model with stable expansion and keyboard state.
pub struct TreeView<K, T> {
    nodes: HashMap<K, Node<K, T>>,
    roots: Vec<K>,
    focused: Option<K>,
}

impl<K, T> Default for TreeView<K, T>
where
    K: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, T> TreeView<K, T>
where
    K: Eq + Hash + Clone,
{
    /// Creates an empty tree.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            roots: Vec::new(),
            focused: None,
        }
    }

    /// Inserts a root node with loaded, initially empty children.
    pub fn insert_root(&mut self, id: K, label: T) -> Result<(), TreeError<K>> {
        if self.nodes.contains_key(&id) {
            return Err(TreeError::DuplicateId(id));
        }
        self.nodes.insert(
            id.clone(),
            Node {
                label,
                parent: None,
                children: Vec::new(),
                child_state: ChildLoadState::Loaded,
                child_error: None,
                expanded: false,
            },
        );
        self.roots.push(id);
        Ok(())
    }

    /// Inserts a new child below a loaded parent.
    pub fn insert_child(&mut self, parent: K, id: K, label: T) -> Result<(), TreeError<K>> {
        let parent_node = self
            .nodes
            .get(&parent)
            .ok_or_else(|| TreeError::MissingNode(parent.clone()))?;
        if parent_node.child_state != ChildLoadState::Loaded {
            return Err(TreeError::InvalidChildState(parent));
        }
        if self.nodes.contains_key(&id) {
            return Err(TreeError::DuplicateId(id));
        }

        self.nodes.insert(
            id.clone(),
            Node {
                label,
                parent: Some(parent.clone()),
                children: Vec::new(),
                child_state: ChildLoadState::Loaded,
                child_error: None,
                expanded: false,
            },
        );
        self.nodes
            .get_mut(&parent)
            .expect("parent was checked before insertion")
            .children
            .push(id);
        Ok(())
    }

    /// Attaches an existing root node below another loaded node.
    ///
    /// Reparenting an already attached node is rejected. This keeps ownership
    /// explicit and makes accidental cycles impossible to hide in a move.
    pub fn attach_child(&mut self, parent: K, child: K) -> Result<(), TreeError<K>> {
        let parent_node = self
            .nodes
            .get(&parent)
            .ok_or_else(|| TreeError::MissingNode(parent.clone()))?;
        if !self.nodes.contains_key(&child) {
            return Err(TreeError::MissingNode(child));
        }
        if parent == child || self.is_ancestor_of(&child, &parent) {
            return Err(TreeError::Cycle { parent, child });
        }
        if parent_node.child_state != ChildLoadState::Loaded {
            return Err(TreeError::InvalidChildState(parent));
        }
        if self
            .nodes
            .get(&child)
            .and_then(|node| node.parent.as_ref())
            .is_some()
        {
            return Err(TreeError::AlreadyAttached(child));
        }
        if self
            .nodes
            .get(&parent)
            .is_some_and(|node| node.children.contains(&child))
        {
            return Err(TreeError::AlreadyAttached(child));
        }

        self.roots.retain(|root| root != &child);
        self.nodes
            .get_mut(&child)
            .expect("child was checked before attachment")
            .parent = Some(parent.clone());
        self.nodes
            .get_mut(&parent)
            .expect("parent was checked before attachment")
            .children
            .push(child);
        Ok(())
    }

    /// Marks an empty node as a lazy child source.
    pub fn mark_lazy(&mut self, id: K) -> Result<(), TreeError<K>> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| TreeError::MissingNode(id.clone()))?;
        if !node.children.is_empty() {
            return Err(TreeError::AlreadyHasChildren(id));
        }
        node.child_state = ChildLoadState::Unloaded;
        node.child_error = None;
        Ok(())
    }

    /// Starts loading a lazy node's children.
    pub fn begin_loading(&mut self, id: K) -> Result<(), TreeError<K>> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| TreeError::MissingNode(id.clone()))?;
        if node.child_state != ChildLoadState::Unloaded {
            return Err(TreeError::InvalidChildState(id));
        }
        node.child_state = ChildLoadState::Loading;
        node.child_error = None;
        Ok(())
    }

    /// Completes a pending lazy load atomically.
    pub fn complete_children(
        &mut self,
        parent: K,
        children: impl IntoIterator<Item = TreeNodeSpec<K, T>>,
    ) -> Result<(), TreeError<K>> {
        let parent_state = self
            .nodes
            .get(&parent)
            .ok_or_else(|| TreeError::MissingNode(parent.clone()))?
            .child_state;
        if parent_state != ChildLoadState::Loading {
            return Err(TreeError::NotLoading(parent));
        }

        let children: Vec<_> = children.into_iter().collect();
        let mut batch_ids: Vec<&K> = Vec::with_capacity(children.len());
        for child in &children {
            if self.nodes.contains_key(&child.id) {
                return Err(TreeError::DuplicateChildId(child.id.clone()));
            }
            if batch_ids.iter().any(|id| *id == &child.id) {
                return Err(TreeError::DuplicateChildId(child.id.clone()));
            }
            batch_ids.push(&child.id);
        }

        for child in children {
            let (id, label) = child.into_parts();
            self.nodes.insert(
                id.clone(),
                Node {
                    label,
                    parent: Some(parent.clone()),
                    children: Vec::new(),
                    child_state: ChildLoadState::Loaded,
                    child_error: None,
                    expanded: false,
                },
            );
            self.nodes
                .get_mut(&parent)
                .expect("parent was checked before lazy completion")
                .children
                .push(id);
        }
        self.nodes
            .get_mut(&parent)
            .expect("parent was checked before lazy completion")
            .child_state = ChildLoadState::Loaded;
        Ok(())
    }

    /// Records a failed lazy-child request.
    pub fn fail_loading(&mut self, id: K, message: impl Into<String>) -> Result<(), TreeError<K>> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| TreeError::MissingNode(id.clone()))?;
        if node.child_state != ChildLoadState::Loading {
            return Err(TreeError::NotLoading(id));
        }
        node.child_state = ChildLoadState::Error;
        node.child_error = Some(message.into());
        Ok(())
    }

    /// Returns the child-load state of a node.
    pub fn children_state(&self, id: &K) -> Option<ChildLoadState> {
        self.nodes.get(id).map(|node| node.child_state)
    }

    /// Returns the last child-load error message, if any.
    pub fn children_error(&self, id: &K) -> Option<&str> {
        self.nodes
            .get(id)
            .and_then(|node| node.child_error.as_deref())
    }

    /// Returns the label/value of a node.
    pub fn label(&self, id: &K) -> Option<&T> {
        self.nodes.get(id).map(|node| &node.label)
    }

    /// Sets whether a node is expanded.
    pub fn set_expanded(&mut self, id: K, expanded: bool) -> Result<(), TreeError<K>> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| TreeError::MissingNode(id.clone()))?;
        node.expanded = expanded;
        Ok(())
    }

    /// Toggles and returns a node's expanded state.
    pub fn toggle_expanded(&mut self, id: K) -> Result<bool, TreeError<K>> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| TreeError::MissingNode(id.clone()))?;
        node.expanded = !node.expanded;
        Ok(node.expanded)
    }

    /// Returns whether a node is expanded.
    pub fn is_expanded(&self, id: &K) -> Option<bool> {
        self.nodes.get(id).map(|node| node.expanded)
    }

    /// Returns the stable IDs in visible preorder.
    pub fn visible_ids(&self) -> Vec<K> {
        let mut visible = Vec::new();
        for root in &self.roots {
            self.push_visible(root, &mut visible);
        }
        visible
    }

    /// Sets keyboard focus to an existing node.
    pub fn focus(&mut self, id: K) -> Result<(), TreeError<K>> {
        if !self.nodes.contains_key(&id) {
            return Err(TreeError::MissingNode(id));
        }
        self.focused = Some(id);
        Ok(())
    }

    /// Returns the currently focused stable ID.
    pub fn focused(&self) -> Option<&K> {
        self.focused.as_ref()
    }

    /// Applies one keyboard command to the focused visible tree.
    pub fn handle_key(&mut self, key: TreeKey) -> Result<TreeKeyResult<K>, TreeError<K>> {
        let visible = self.visible_ids();
        match key {
            TreeKey::Home => return self.focus_edge(&visible, false),
            TreeKey::End => return self.focus_edge(&visible, true),
            TreeKey::ArrowUp | TreeKey::ArrowDown => {
                return self.focus_adjacent(&visible, matches!(key, TreeKey::ArrowDown));
            }
            TreeKey::Enter | TreeKey::Space => {
                return Ok(self
                    .focused
                    .clone()
                    .map_or(TreeKeyResult::Noop, TreeKeyResult::Activated));
            }
            TreeKey::ArrowLeft | TreeKey::ArrowRight => {}
        }

        let Some(current) = self.focused.clone() else {
            return Ok(TreeKeyResult::Noop);
        };
        let Some(node) = self.nodes.get(&current) else {
            self.focused = None;
            return Ok(TreeKeyResult::Noop);
        };

        if key == TreeKey::ArrowLeft {
            if node.expanded {
                self.nodes
                    .get_mut(&current)
                    .expect("focused node was checked above")
                    .expanded = false;
                return Ok(TreeKeyResult::Collapsed(current));
            }
            if let Some(parent) = node.parent.clone() {
                self.focused = Some(parent.clone());
                return Ok(TreeKeyResult::Focused(parent));
            }
            return Ok(TreeKeyResult::Noop);
        }

        if !node.expanded {
            if node.child_state == ChildLoadState::Unloaded {
                self.nodes
                    .get_mut(&current)
                    .expect("focused node was checked above")
                    .expanded = true;
                self.begin_loading(current.clone())?;
                return Ok(TreeKeyResult::RequestChildren(current));
            }
            self.nodes
                .get_mut(&current)
                .expect("focused node was checked above")
                .expanded = true;
            return Ok(TreeKeyResult::Expanded(current));
        }

        if let Some(child) = node.children.first().cloned() {
            self.focused = Some(child.clone());
            return Ok(TreeKeyResult::Focused(child));
        }
        Ok(TreeKeyResult::Noop)
    }

    fn is_ancestor_of(&self, possible_ancestor: &K, node: &K) -> bool {
        let mut current = Some(node.clone());
        while let Some(id) = current {
            if &id == possible_ancestor {
                return true;
            }
            current = self.nodes.get(&id).and_then(|item| item.parent.clone());
        }
        false
    }

    fn push_visible(&self, id: &K, visible: &mut Vec<K>) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        visible.push(id.clone());
        if node.expanded {
            for child in &node.children {
                self.push_visible(child, visible);
            }
        }
    }

    fn focus_edge(&mut self, visible: &[K], last: bool) -> Result<TreeKeyResult<K>, TreeError<K>> {
        let target = if last {
            visible.last()
        } else {
            visible.first()
        };
        let Some(target) = target else {
            return Ok(TreeKeyResult::Noop);
        };
        if self.focused.as_ref() == Some(target) {
            return Ok(TreeKeyResult::Noop);
        }
        self.focused = Some(target.clone());
        Ok(TreeKeyResult::Focused(target.clone()))
    }

    fn focus_adjacent(
        &mut self,
        visible: &[K],
        forward: bool,
    ) -> Result<TreeKeyResult<K>, TreeError<K>> {
        if visible.is_empty() {
            return Ok(TreeKeyResult::Noop);
        }
        let index = self
            .focused
            .as_ref()
            .and_then(|focused| visible.iter().position(|id| id == focused));
        let Some(index) = index else {
            let target = if forward {
                &visible[0]
            } else {
                visible.last().expect("non-empty")
            };
            self.focused = Some(target.clone());
            return Ok(TreeKeyResult::Focused(target.clone()));
        };
        let target_index = if forward {
            (index + 1).min(visible.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        if target_index == index {
            return Ok(TreeKeyResult::Noop);
        }
        let target = visible[target_index].clone();
        self.focused = Some(target.clone());
        Ok(TreeKeyResult::Focused(target))
    }
}
