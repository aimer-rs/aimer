use super::Bounds;

/// Stable identity for one node in a semantic tree.
///
/// A host should derive this from the same stable identity it gives its
/// retained element or focus candidate. The accessibility crate does not
/// allocate or recycle identities.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(u64);

impl NodeId {
    /// Creates an identifier from a host-owned value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the host-owned numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for NodeId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

/// The platform-neutral role exposed for a semantic node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// A node with no more specific role.
    Generic,
    /// A command that can be activated.
    Button,
    /// A binary or tri-state check control.
    Checkbox,
    /// A control that opens or filters a list of choices.
    Combobox,
    /// A modal or non-modal dialog surface.
    Dialog,
    /// A grouping container.
    Group,
    /// A heading in the document structure.
    Heading,
    /// An image or other non-text visual.
    Image,
    /// A navigable hyperlink.
    Link,
    /// A collection whose children are list items.
    List,
    /// One item in a list.
    ListItem,
    /// A menu containing menu items.
    Menu,
    /// An actionable item in a menu.
    MenuItem,
    /// A progress indicator with a numeric range.
    ProgressBar,
    /// One option in a mutually exclusive group.
    Radio,
    /// A scrollable region.
    ScrollView,
    /// A single-value range control.
    Slider,
    /// A numeric text-and-step control.
    SpinButton,
    /// A binary on/off control.
    Switch,
    /// A tab in a tab list.
    Tab,
    /// The container for tabs.
    TabList,
    /// The content associated with a tab.
    TabPanel,
    /// An editable text control.
    TextField,
    /// A static text node.
    Text,
    /// A hierarchical collection.
    Tree,
    /// One item in a hierarchical collection.
    TreeItem,
    /// A non-interactive presentation-only node.
    Presentation,
}

impl Role {
    pub(crate) const fn canonical_name(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Button => "button",
            Self::Checkbox => "checkbox",
            Self::Combobox => "combobox",
            Self::Dialog => "dialog",
            Self::Group => "group",
            Self::Heading => "heading",
            Self::Image => "image",
            Self::Link => "link",
            Self::List => "list",
            Self::ListItem => "list-item",
            Self::Menu => "menu",
            Self::MenuItem => "menu-item",
            Self::ProgressBar => "progress-bar",
            Self::Radio => "radio",
            Self::ScrollView => "scroll-view",
            Self::Slider => "slider",
            Self::SpinButton => "spin-button",
            Self::Switch => "switch",
            Self::Tab => "tab",
            Self::TabList => "tab-list",
            Self::TabPanel => "tab-panel",
            Self::TextField => "text-field",
            Self::Text => "text",
            Self::Tree => "tree",
            Self::TreeItem => "tree-item",
            Self::Presentation => "presentation",
        }
    }
}

/// A checked state when a role supports checked semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckedState {
    /// The control is not checked.
    Unchecked,
    /// The control is checked.
    Checked,
    /// The control contains a mixture of checked and unchecked descendants.
    Mixed,
}

impl CheckedState {
    pub(crate) const fn canonical_name(self) -> &'static str {
        match self {
            Self::Unchecked => "unchecked",
            Self::Checked => "checked",
            Self::Mixed => "mixed",
        }
    }
}

/// A finite numeric value and its optional range metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueRange {
    min: f32,
    max: f32,
    current: f32,
    step: Option<f32>,
}

impl ValueRange {
    /// Creates a range whose current value is inclusive of its bounds.
    pub fn new(min: f32, max: f32, current: f32) -> Result<Self, RangeError> {
        if !min.is_finite() || !max.is_finite() || !current.is_finite() {
            return Err(RangeError::NonFinite);
        }
        if min > max {
            return Err(RangeError::Reversed { min, max });
        }
        if current < min || current > max {
            return Err(RangeError::CurrentOutOfBounds { current, min, max });
        }
        Ok(Self {
            min,
            max,
            current,
            step: None,
        })
    }

    /// Adds a strictly positive finite step to the range.
    pub fn with_step(mut self, step: f32) -> Result<Self, RangeError> {
        if !step.is_finite() || step <= 0.0 {
            return Err(RangeError::InvalidStep(step));
        }
        self.step = Some(step);
        Ok(self)
    }

    /// Returns the inclusive minimum.
    pub const fn min(self) -> f32 {
        self.min
    }

    /// Returns the inclusive maximum.
    pub const fn max(self) -> f32 {
        self.max
    }

    /// Returns the current value.
    pub const fn current(self) -> f32 {
        self.current
    }

    /// Returns the optional increment.
    pub const fn step(self) -> Option<f32> {
        self.step
    }
}

/// An invalid value-range input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeError {
    /// At least one range value was NaN or infinite.
    NonFinite,
    /// The minimum was greater than the maximum.
    Reversed {
        /// The rejected minimum.
        min: f32,
        /// The rejected maximum.
        max: f32,
    },
    /// The current value was outside the inclusive range.
    CurrentOutOfBounds {
        /// The rejected current value.
        current: f32,
        /// The range minimum.
        min: f32,
        /// The range maximum.
        max: f32,
    },
    /// The step was not finite and strictly positive.
    InvalidStep(f32),
}

/// The state flags applicable to a semantic node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticState {
    enabled: bool,
    selected: Option<bool>,
    checked: Option<CheckedState>,
    expanded: Option<bool>,
    busy: bool,
}

impl SemanticState {
    /// Creates the default state: enabled, not busy, and without
    /// role-specific optional flags.
    pub const fn new() -> Self {
        Self {
            enabled: true,
            selected: None,
            checked: None,
            expanded: None,
            busy: false,
        }
    }

    /// Returns whether the node can currently be interacted with.
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Returns the selected state when the role supports it.
    pub const fn selected(self) -> Option<bool> {
        self.selected
    }

    /// Returns the checked state when the role supports it.
    pub const fn checked(self) -> Option<CheckedState> {
        self.checked
    }

    /// Returns the expanded state when the node owns expandable content.
    pub const fn expanded(self) -> Option<bool> {
        self.expanded
    }

    /// Returns whether the node is busy.
    pub const fn busy(self) -> bool {
        self.busy
    }
}

impl Default for SemanticState {
    fn default() -> Self {
        Self::new()
    }
}

/// An action a platform adapter may expose for a semantic node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticAction {
    /// Activates the node's primary command.
    Activate,
    /// Dismisses the node or its containing surface.
    Dismiss,
    /// Expands the node.
    Expand,
    /// Collapses the node.
    Collapse,
    /// Increments a numeric value.
    Increment,
    /// Decrements a numeric value.
    Decrement,
    /// Sets a value using the host's value editor.
    SetValue,
    /// A product-specific action with a stable name.
    Custom(String),
}

impl SemanticAction {
    pub(crate) fn canonical_name(&self) -> String {
        match self {
            Self::Activate => "activate".to_owned(),
            Self::Dismiss => "dismiss".to_owned(),
            Self::Expand => "expand".to_owned(),
            Self::Collapse => "collapse".to_owned(),
            Self::Increment => "increment".to_owned(),
            Self::Decrement => "decrement".to_owned(),
            Self::SetValue => "set-value".to_owned(),
            Self::Custom(name) => format!("custom:{name}"),
        }
    }
}

/// Determines how a node and its children are projected to the platform tree.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SemanticBehavior {
    /// Keep this node and recursively expose its children.
    #[default]
    Normal,
    /// Keep one node and fold descendant labels, descriptions, and actions
    /// into it. Explicit values, ranges, states, bounds, and focusability on
    /// the merge node are not inferred from descendants.
    Merge,
    /// Omit this node and its entire subtree.
    Exclude,
    /// Keep this node but omit all of its children.
    Leaf,
}

/// One mutable-by-rebuild semantic node.
///
/// Nodes are ordinary owned values so a widget can create a fresh description
/// each rebuild without introducing a platform handle. Use a stable [`NodeId`]
/// when publishing the node so a host can preserve focus and native element
/// identity.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticNode {
    pub(crate) id: NodeId,
    pub(crate) role: Role,
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) value_range: Option<ValueRange>,
    pub(crate) state: SemanticState,
    pub(crate) bounds: Option<Bounds>,
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) focusable: bool,
    pub(crate) behavior: SemanticBehavior,
    pub(crate) children: Vec<SemanticNode>,
}

impl SemanticNode {
    /// Creates an enabled, unnamed node without children or actions.
    pub fn new(id: NodeId, role: Role) -> Self {
        Self {
            id,
            role,
            name: None,
            description: None,
            value: None,
            value_range: None,
            state: SemanticState::new(),
            bounds: None,
            actions: Vec::new(),
            focusable: false,
            behavior: SemanticBehavior::Normal,
            children: Vec::new(),
        }
    }

    /// Returns this node's stable identity.
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// Returns this node's role.
    pub const fn role(&self) -> Role {
        self.role
    }

    /// Returns the accessible name, if one was supplied.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the longer accessible description, if one was supplied.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the human-readable value, if one was supplied.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Returns numeric range metadata, if one was supplied.
    pub const fn value_range(&self) -> Option<ValueRange> {
        self.value_range
    }

    /// Returns the node state flags.
    pub const fn state(&self) -> SemanticState {
        self.state
    }

    /// Returns layout bounds when the host supplied them.
    pub const fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }

    /// Returns the actions published by this node.
    pub fn actions(&self) -> &[SemanticAction] {
        &self.actions
    }

    /// Returns whether this node is eligible for keyboard focus.
    pub const fn is_focusable(&self) -> bool {
        self.focusable
    }

    /// Returns the child projection policy.
    pub const fn behavior(&self) -> SemanticBehavior {
        self.behavior
    }

    /// Returns the raw children before tree projection.
    pub fn children(&self) -> &[SemanticNode] {
        &self.children
    }

    /// Sets the accessible name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the accessible description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets a human-readable value.
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Sets a finite numeric value range.
    pub fn with_value_range(mut self, value_range: ValueRange) -> Self {
        self.value_range = Some(value_range);
        self
    }

    /// Sets layout bounds for hit testing and platform mapping.
    pub fn with_bounds(mut self, bounds: Bounds) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Sets whether the node is enabled.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.state.enabled = enabled;
        self
    }

    /// Publishes a selected state.
    pub fn selected(mut self, selected: bool) -> Self {
        self.state.selected = Some(selected);
        self
    }

    /// Publishes or clears the selected state.
    pub fn selected_state(mut self, selected: Option<bool>) -> Self {
        self.state.selected = selected;
        self
    }

    /// Publishes a checked state.
    pub fn checked(mut self, checked: CheckedState) -> Self {
        self.state.checked = Some(checked);
        self
    }

    /// Publishes or clears the checked state.
    pub fn checked_state(mut self, checked: Option<CheckedState>) -> Self {
        self.state.checked = checked;
        self
    }

    /// Publishes an expanded state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.state.expanded = Some(expanded);
        self
    }

    /// Publishes or clears the expanded state.
    pub fn expanded_state(mut self, expanded: Option<bool>) -> Self {
        self.state.expanded = expanded;
        self
    }

    /// Sets whether the node is currently doing work.
    pub fn busy(mut self, busy: bool) -> Self {
        self.state.busy = busy;
        self
    }

    /// Adds an action if it is not already present.
    pub fn with_action(mut self, action: SemanticAction) -> Self {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
        self
    }

    /// Sets whether the host's focus infrastructure may focus this node.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// Sets the child projection policy.
    pub fn with_behavior(mut self, behavior: SemanticBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    /// Marks this node as a single merged platform node.
    pub fn merge(self) -> Self {
        self.with_behavior(SemanticBehavior::Merge)
    }

    /// Omits this node and its descendants from the published tree.
    pub fn exclude(self) -> Self {
        self.with_behavior(SemanticBehavior::Exclude)
    }

    /// Publishes this node as a leaf, ignoring its raw children.
    pub fn leaf(self) -> Self {
        self.with_behavior(SemanticBehavior::Leaf)
    }

    /// Appends one child in deterministic insertion order.
    pub fn with_child(mut self, child: SemanticNode) -> Self {
        self.children.push(child);
        self
    }
}

/// A request sent to the host's action system after an action has been
/// validated against a published semantic node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    node_id: NodeId,
    action: SemanticAction,
}

impl ActionRequest {
    /// Creates an action request for a node and action.
    pub const fn new(node_id: NodeId, action: SemanticAction) -> Self {
        Self { node_id, action }
    }

    /// Returns the target node identity.
    pub const fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Returns the requested action.
    pub fn action(&self) -> &SemanticAction {
        &self.action
    }
}
