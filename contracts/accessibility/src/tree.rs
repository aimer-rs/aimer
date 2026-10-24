use std::collections::HashSet;
use std::fmt::Write;

use super::{ActionRequest, SemanticAction, SemanticBehavior, SemanticNode};
use super::{NodeId, SemanticState};

/// A source of focus order supplied by the host focus system.
///
/// The implementation is expected to wrap the application's existing focus
/// manager or focus candidates. The accessibility crate only filters that
/// order to nodes that are present and marked focusable; it never owns focus.
pub trait FocusTraversalSource {
    /// Returns node identities in the host's keyboard traversal order.
    fn ordered_nodes(&self) -> &[NodeId];
}

/// Receives an already validated semantic action request.
pub trait ActionHandler {
    /// Performs the request using the host widget/event system.
    fn handle(&mut self, request: ActionRequest);
}

/// A failure while dispatching an action through a semantic snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionDispatchError {
    /// The requested node is not in the published tree.
    NodeNotFound(NodeId),
    /// The node exists, but it did not publish the requested action.
    ActionUnavailable {
        /// The node that rejected the action.
        node_id: NodeId,
        /// The action that was not published.
        action: SemanticAction,
    },
}

/// A tree before its merge, exclude, and leaf policies are applied.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticTree {
    root: SemanticNode,
}

impl SemanticTree {
    /// Creates a tree from an owned root node.
    ///
    /// Duplicate identities are reported when [`Self::snapshot`] is built so
    /// callers may assemble a tree with ordinary builder code first.
    pub fn new(root: SemanticNode) -> Self {
        Self { root }
    }

    /// Returns the unprojected root node.
    pub fn root(&self) -> &SemanticNode {
        &self.root
    }

    /// Applies child policies and returns an immutable platform-facing view.
    pub fn snapshot(&self) -> Result<SemanticSnapshot, TreeError> {
        let mut ids = HashSet::new();
        validate_ids(&self.root, &mut ids)?;
        let root = normalize_node(&self.root).ok_or(TreeError::ExcludedRoot)?;
        Ok(SemanticSnapshot { root })
    }
}

/// A tree ready for a native, browser, or test adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticSnapshot {
    root: SemanticNode,
}

impl SemanticSnapshot {
    /// Returns the projected root node.
    pub fn root(&self) -> &SemanticNode {
        &self.root
    }

    /// Returns a deterministic pre-order traversal of the published nodes.
    pub fn traverse(&self) -> SemanticTraversal<'_> {
        SemanticTraversal {
            pending: vec![&self.root],
        }
    }

    /// Finds one published node by stable identity.
    pub fn node(&self, id: NodeId) -> Option<&SemanticNode> {
        self.traverse().find(|node| node.id() == id)
    }

    /// Returns the number of published nodes.
    pub fn len(&self) -> usize {
        self.traverse().count()
    }

    /// Returns whether the published tree is empty.
    ///
    /// A snapshot always has a root, so this is currently always false. It is
    /// provided to keep collection-style adapter code straightforward.
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Projects the existing host focus order onto published focusable nodes.
    pub fn focus_order<S: FocusTraversalSource>(&self, source: &S) -> Vec<NodeId> {
        source
            .ordered_nodes()
            .iter()
            .copied()
            .filter(|id| self.node(*id).is_some_and(SemanticNode::is_focusable))
            .collect()
    }

    /// Dispatches an action only when the target node published it.
    pub fn dispatch_action<H: ActionHandler>(
        &self,
        node_id: NodeId,
        action: &SemanticAction,
        handler: &mut H,
    ) -> Result<(), ActionDispatchError> {
        let node = self
            .node(node_id)
            .ok_or(ActionDispatchError::NodeNotFound(node_id))?;
        if !node.actions().iter().any(|published| published == action) {
            return Err(ActionDispatchError::ActionUnavailable {
                node_id,
                action: action.clone(),
            });
        }
        handler.handle(ActionRequest::new(node_id, action.clone()));
        Ok(())
    }

    /// Encodes the tree as stable, human-readable lines for tests and
    /// diagnostics.
    ///
    /// This is deliberately not a wire format. Adapters should map the typed
    /// node API directly, while tests can compare this output without relying
    /// on a platform accessibility implementation.
    pub fn canonical_string(&self) -> String {
        let mut output = String::new();
        write_canonical_node(&self.root, &mut output);
        output
    }
}

/// A pre-order iterator over a [`SemanticSnapshot`].
pub struct SemanticTraversal<'a> {
    pending: Vec<&'a SemanticNode>,
}

impl<'a> Iterator for SemanticTraversal<'a> {
    type Item = &'a SemanticNode;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.pending.pop()?;
        self.pending.extend(node.children().iter().rev());
        Some(node)
    }
}

/// A structural error found while publishing a semantic tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreeError {
    /// Two raw nodes used the same stable identity.
    DuplicateNodeId(NodeId),
    /// The root was marked excluded, so there is no publishable tree.
    ExcludedRoot,
}

fn validate_ids(node: &SemanticNode, ids: &mut HashSet<NodeId>) -> Result<(), TreeError> {
    if !ids.insert(node.id()) {
        return Err(TreeError::DuplicateNodeId(node.id()));
    }
    for child in node.children() {
        validate_ids(child, ids)?;
    }
    Ok(())
}

fn normalize_node(node: &SemanticNode) -> Option<SemanticNode> {
    match node.behavior() {
        SemanticBehavior::Exclude => None,
        SemanticBehavior::Leaf => {
            let mut normalized = node.clone();
            normalized.behavior = SemanticBehavior::Normal;
            normalized.children.clear();
            Some(normalized)
        }
        SemanticBehavior::Normal => {
            let mut normalized = node.clone();
            normalized.behavior = SemanticBehavior::Normal;
            normalized.children = node.children().iter().filter_map(normalize_node).collect();
            Some(normalized)
        }
        SemanticBehavior::Merge => Some(merge_node(node)),
    }
}

fn merge_node(node: &SemanticNode) -> SemanticNode {
    let mut merged = node.clone();
    merged.behavior = SemanticBehavior::Normal;
    merged.children.clear();

    let mut names = Vec::new();
    let mut descriptions = Vec::new();
    let mut actions = Vec::new();
    for child in node.children() {
        if let Some(normalized) = normalize_node(child) {
            collect_merge_contributions(&normalized, &mut names, &mut descriptions, &mut actions);
        }
    }

    if merged.name.is_none() {
        merged.name = join_labels(names);
    }
    if merged.description.is_none() {
        merged.description = join_labels(descriptions);
    }
    for action in actions {
        if !merged.actions.contains(&action) {
            merged.actions.push(action);
        }
    }
    merged
}

fn collect_merge_contributions(
    node: &SemanticNode,
    names: &mut Vec<String>,
    descriptions: &mut Vec<String>,
    actions: &mut Vec<SemanticAction>,
) {
    if let Some(name) = node.name() {
        names.push(name.to_owned());
    }
    if let Some(description) = node.description() {
        descriptions.push(description.to_owned());
    }
    for action in node.actions() {
        if !actions.contains(action) {
            actions.push(action.clone());
        }
    }
    for child in node.children() {
        collect_merge_contributions(child, names, descriptions, actions);
    }
}

fn join_labels(labels: Vec<String>) -> Option<String> {
    if labels.is_empty() {
        None
    } else {
        Some(labels.join(" "))
    }
}

fn write_canonical_node(node: &SemanticNode, output: &mut String) {
    if !output.is_empty() {
        output.push('\n');
    }
    write!(
        output,
        "id={};role={};",
        node.id().get(),
        node.role().canonical_name()
    )
    .unwrap();
    write_optional(output, "name", node.name());
    write_optional(output, "description", node.description());
    write_optional(output, "value", node.value());
    match node.value_range() {
        Some(range) => write!(
            output,
            "range={},{},{},{};",
            range.min(),
            range.max(),
            range.current(),
            range
                .step()
                .map_or_else(|| "-".to_owned(), |step| step.to_string())
        )
        .unwrap(),
        None => output.push_str("range=-;"),
    }
    write_state(output, node.state());
    match node.bounds() {
        Some(bounds) => write!(
            output,
            "bounds={},{},{},{};",
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height()
        )
        .unwrap(),
        None => output.push_str("bounds=-;"),
    }
    write!(output, "focusable={};actions=[", node.is_focusable()).unwrap();
    for (index, action) in node.actions().iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write_quoted(output, &action.canonical_name());
    }
    output.push_str("];children=[");
    for (index, child) in node.children().iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(output, "{}", child.id().get()).unwrap();
    }
    output.push(']');
    for child in node.children() {
        write_canonical_node(child, output);
    }
}

fn write_state(output: &mut String, state: SemanticState) {
    write!(output, "enabled={};", state.enabled()).unwrap();
    match state.selected() {
        Some(selected) => write!(output, "selected={selected};").unwrap(),
        None => output.push_str("selected=-;"),
    }
    match state.checked() {
        Some(checked) => write!(output, "checked={};", checked.canonical_name()).unwrap(),
        None => output.push_str("checked=-;"),
    }
    match state.expanded() {
        Some(expanded) => write!(output, "expanded={expanded};").unwrap(),
        None => output.push_str("expanded=-;"),
    }
    write!(output, "busy={};", state.busy()).unwrap();
}

fn write_optional(output: &mut String, key: &str, value: Option<&str>) {
    output.push_str(key);
    output.push('=');
    match value {
        Some(value) => write_quoted(output, value),
        None => output.push('-'),
    }
    output.push(';');
}

fn write_quoted(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                write!(output, "\\u{{{:x}}}", character as u32).unwrap();
            }
            character => output.push(character),
        }
    }
    output.push('"');
}
