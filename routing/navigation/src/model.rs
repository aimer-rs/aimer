use core::fmt;

/// Errors raised while constructing a navigation model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationError {
    /// No items were supplied.
    EmptyItems,
    /// Every supplied item is disabled, so no initial focus can be assigned.
    AllItemsDisabled,
    /// Two items use the same key.
    DuplicateKey,
    /// A layout item has zero extent and cannot be used to calculate overflow.
    InvalidItemExtent,
}

impl fmt::Display for NavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyItems => "navigation requires at least one item",
            Self::AllItemsDisabled => "navigation requires at least one enabled item",
            Self::DuplicateKey => "navigation item keys must be unique",
            Self::InvalidItemExtent => "navigation item extent must be greater than zero",
        };
        f.write_str(message)
    }
}

impl std::error::Error for NavigationError {}

/// The directional keyboard commands understood by navigation models.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationKey {
    /// Move to the previous item in a horizontal navigation surface.
    Left,
    /// Move to the next item in a horizontal navigation surface.
    Right,
    /// Move to the previous item in a vertical navigation surface.
    Up,
    /// Move to the next item in a vertical navigation surface.
    Down,
    /// Focus the first enabled item.
    Home,
    /// Focus the last enabled item.
    End,
    /// Activate the focused item.
    Enter,
    /// Activate the focused item.
    Space,
    /// Leave the navigation surface in the forward tab order.
    Tab,
    /// Leave the navigation surface in the reverse tab order.
    ShiftTab,
}

/// The axis used by arrow-key navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orientation {
    /// Items are arranged left to right.
    Horizontal,
    /// Items are arranged top to bottom.
    Vertical,
}

/// The common result of moving focus or activating a navigation item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationAction {
    /// The key or index did not produce a change.
    Ignored,
    /// Focus moved to the given item index.
    FocusChanged(usize),
    /// The given item was activated.
    Activated(usize),
    /// The requested item exists but is disabled.
    Disabled(usize),
    /// Focus should continue outside this navigation surface.
    TabbedAway,
}

/// A selectable item shared by tabs and navigation surfaces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tab<K> {
    key: K,
    label: String,
    disabled: bool,
}

impl<K> Tab<K> {
    /// Creates an enabled item with a stable application key and visible label.
    pub fn new(key: K, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            disabled: false,
        }
    }

    /// Marks this item as enabled or disabled.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Returns the stable item key.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns the user-facing label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns whether this item can receive navigation focus or activation.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

/// A semantic role exposed by the navigation model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticRole {
    /// A selectable tab in a tab list.
    Tab,
    /// A destination in a drawer, rail, or bottom navigation surface.
    NavigationItem,
    /// A location in a breadcrumb trail.
    Breadcrumb,
    /// A step in a stepper.
    Step,
}

/// The platform-neutral semantic information for one navigation item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationSemantics {
    /// The role a platform adapter should expose.
    pub role: SemanticRole,
    /// The accessible name.
    pub label: String,
    /// Whether this item is the active selection.
    pub selected: bool,
    /// Whether this item is disabled.
    pub disabled: bool,
    /// Whether this item may receive keyboard focus.
    pub focusable: bool,
    /// One-based position in the complete item set.
    pub position: usize,
    /// Number of items in the complete set.
    pub set_size: usize,
}

/// A single selectable tab or navigation destination.
#[derive(Debug, Eq, PartialEq)]
pub struct TabBar<K> {
    tabs: Vec<Tab<K>>,
    selected: usize,
    focused: usize,
    orientation: Orientation,
    wrap_navigation: bool,
}

impl<K: PartialEq> TabBar<K> {
    /// Creates a tab bar with the first enabled item selected and focused.
    pub fn new(tabs: Vec<Tab<K>>) -> Result<Self, NavigationError> {
        let first_enabled =
            tabs.iter()
                .position(|tab| !tab.is_disabled())
                .ok_or(if tabs.is_empty() {
                    NavigationError::EmptyItems
                } else {
                    NavigationError::AllItemsDisabled
                })?;

        for (index, tab) in tabs.iter().enumerate() {
            if tabs[index + 1..]
                .iter()
                .any(|other| other.key() == tab.key())
            {
                return Err(NavigationError::DuplicateKey);
            }
        }

        Ok(Self {
            tabs,
            selected: first_enabled,
            focused: first_enabled,
            orientation: Orientation::Horizontal,
            wrap_navigation: true,
        })
    }

    /// Returns all items in their stable focus order.
    #[inline]
    pub fn tabs(&self) -> &[Tab<K>] {
        &self.tabs
    }

    /// Returns the selected item index.
    #[inline]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Returns the focused item index.
    #[inline]
    pub fn focused_index(&self) -> usize {
        self.focused
    }

    /// Returns the selected key, if the bar contains an item.
    #[inline]
    pub fn selected_key(&self) -> Option<&K> {
        self.tabs.get(self.selected).map(Tab::key)
    }

    /// Returns the focused key, if the bar contains an item.
    #[inline]
    pub fn focused_key(&self) -> Option<&K> {
        self.tabs.get(self.focused).map(Tab::key)
    }

    /// Configures whether an arrow past an end wraps to the other end.
    #[inline]
    pub fn wrap_navigation(mut self, wrap: bool) -> Self {
        self.wrap_navigation = wrap;
        self
    }

    /// Sets the arrow-key axis.
    #[inline]
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Returns the configured arrow-key axis.
    #[inline]
    pub fn navigation_orientation(&self) -> Orientation {
        self.orientation
    }

    /// Selects an enabled item by index and moves focus to it.
    pub fn select(&mut self, index: usize) -> NavigationAction {
        let Some(tab) = self.tabs.get(index) else {
            return NavigationAction::Ignored;
        };
        if tab.is_disabled() {
            return NavigationAction::Disabled(index);
        }
        self.selected = index;
        self.focused = index;
        NavigationAction::Activated(index)
    }

    /// Selects an enabled item by key.
    pub fn select_key(&mut self, key: &K) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.key() == key) else {
            return false;
        };
        matches!(self.select(index), NavigationAction::Activated(_))
    }

    /// Moves keyboard focus to an enabled item without changing selection.
    ///
    /// Widget adapters use this when the framework reports a focus gain after
    /// pointer hit testing. Keeping focus separate from selection is what lets
    /// a tab bar show a pending keyboard target before the user activates it.
    pub fn focus(&mut self, index: usize) -> NavigationAction {
        let Some(tab) = self.tabs.get(index) else {
            return NavigationAction::Ignored;
        };
        if tab.is_disabled() {
            return NavigationAction::Disabled(index);
        }
        if self.focused == index {
            NavigationAction::Ignored
        } else {
            self.focused = index;
            NavigationAction::FocusChanged(index)
        }
    }

    /// Moves keyboard focus to an enabled item identified by key.
    #[inline]
    pub fn focus_key(&mut self, key: &K) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.key() == key) else {
            return false;
        };
        matches!(
            self.focus(index),
            NavigationAction::FocusChanged(_) | NavigationAction::Ignored
        )
    }

    /// Handles one keyboard command, including arrow, Home/End, Enter/Space,
    /// and leaving through Tab or Shift+Tab.
    pub fn handle_key(&mut self, key: NavigationKey) -> NavigationAction {
        match key {
            NavigationKey::Home => self.focus_edge(false),
            NavigationKey::End => self.focus_edge(true),
            NavigationKey::Enter | NavigationKey::Space => self.select(self.focused),
            NavigationKey::Tab | NavigationKey::ShiftTab => NavigationAction::TabbedAway,
            NavigationKey::Left if self.orientation == Orientation::Horizontal => {
                self.move_focus(-1)
            }
            NavigationKey::Right if self.orientation == Orientation::Horizontal => {
                self.move_focus(1)
            }
            NavigationKey::Up if self.orientation == Orientation::Vertical => self.move_focus(-1),
            NavigationKey::Down if self.orientation == Orientation::Vertical => self.move_focus(1),
            _ => NavigationAction::Ignored,
        }
    }

    /// Returns the indices that can receive focus, excluding disabled items.
    pub fn focus_order(&self) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| (!tab.is_disabled()).then_some(index))
            .collect()
    }

    /// Produces semantic items in DOM/tree order.
    pub fn semantics(&self) -> Vec<NavigationSemantics> {
        self.semantics_with_role(SemanticRole::Tab)
    }

    pub(crate) fn semantics_with_role(&self, role: SemanticRole) -> Vec<NavigationSemantics> {
        let set_size = self.tabs.len();
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| NavigationSemantics {
                role,
                label: tab.label().to_owned(),
                selected: index == self.selected,
                disabled: tab.is_disabled(),
                focusable: !tab.is_disabled(),
                position: index + 1,
                set_size,
            })
            .collect()
    }

    fn focus_edge(&mut self, last: bool) -> NavigationAction {
        let index = if last {
            self.tabs.iter().rposition(|tab| !tab.is_disabled())
        } else {
            self.tabs.iter().position(|tab| !tab.is_disabled())
        };
        let Some(index) = index else {
            return NavigationAction::Ignored;
        };
        if self.focused == index {
            NavigationAction::Ignored
        } else {
            self.focused = index;
            NavigationAction::FocusChanged(index)
        }
    }

    fn move_focus(&mut self, direction: isize) -> NavigationAction {
        let count = self.tabs.len();
        let mut candidate = self.focused as isize;
        for _ in 0..count {
            candidate += direction;
            if candidate < 0 || candidate >= count as isize {
                if !self.wrap_navigation {
                    return NavigationAction::Ignored;
                }
                candidate = if direction < 0 { count as isize - 1 } else { 0 };
            }
            let index = candidate as usize;
            if !self.tabs[index].is_disabled() {
                if self.focused == index {
                    return NavigationAction::Ignored;
                }
                self.focused = index;
                return NavigationAction::FocusChanged(index);
            }
        }
        NavigationAction::Ignored
    }
}

/// A tab page retained alongside its tab, so switching tabs does not discard
/// page-local state.
#[derive(Debug, Eq, PartialEq)]
pub struct TabPage<K, V> {
    tab: Tab<K>,
    content: V,
}

/// A tab view whose page values stay mounted while the selected tab changes.
pub struct TabView<K, V> {
    bar: TabBar<K>,
    pages: Vec<TabPage<K, V>>,
}

impl<K: Clone + PartialEq, V> TabView<K, V> {
    /// Creates a retained tab view from `(tab, page_state)` pairs.
    pub fn new(pages: Vec<(Tab<K>, V)>) -> Result<Self, NavigationError> {
        let tabs = pages.iter().map(|(tab, _)| tab.clone()).collect();
        let bar = TabBar::new(tabs)?;
        let pages = pages
            .into_iter()
            .map(|(tab, content)| TabPage { tab, content })
            .collect();
        Ok(Self { bar, pages })
    }

    /// Returns the retained tab bar state.
    #[inline]
    pub fn bar(&self) -> &TabBar<K> {
        &self.bar
    }

    /// Returns mutable access to selection/focus state.
    #[inline]
    pub fn bar_mut(&mut self) -> &mut TabBar<K> {
        &mut self.bar
    }

    /// Returns the selected key.
    #[inline]
    pub fn selected_key(&self) -> Option<&K> {
        self.bar.selected_key()
    }

    /// Returns the selected retained page.
    #[inline]
    pub fn selected_page(&self) -> Option<&V> {
        self.pages
            .get(self.bar.selected_index())
            .map(|page| &page.content)
    }

    /// Returns mutable state for a page by key.
    pub fn page_mut(&mut self, key: &K) -> Option<&mut V> {
        self.pages
            .iter_mut()
            .find(|page| page.tab.key() == key)
            .map(|page| &mut page.content)
    }

    /// Returns a page by key without changing selection.
    pub fn page(&self, key: &K) -> Option<&V> {
        self.pages
            .iter()
            .find(|page| page.tab.key() == key)
            .map(|page| &page.content)
    }

    /// Selects a page by its stable key.
    #[inline]
    pub fn select_key(&mut self, key: &K) -> bool {
        self.bar.select_key(key)
    }

    /// Handles keyboard navigation for the retained tab view.
    #[inline]
    pub fn handle_key(&mut self, key: NavigationKey) -> NavigationAction {
        self.bar.handle_key(key)
    }
}

/// The three common navigation-surface placements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationSurface {
    /// A side drawer that can be collapsed or expanded by a host.
    Drawer,
    /// A compact vertical side rail.
    Rail,
    /// A bottom navigation bar.
    Bottom,
}

/// The policy used when a navigation surface is narrower than its items.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverflowPolicy {
    /// Permit a scrollable or clipped host to reveal the remaining items.
    Scroll,
    /// Expose a deterministic overflow-menu affordance.
    Menu,
}

/// The result of applying a viewport constraint to a navigation surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NavigationLayout {
    /// Number of items that fit in the viewport.
    pub visible_count: usize,
    /// Number of items remaining outside the viewport.
    pub hidden_count: usize,
    /// Whether the host should expose an overflow affordance.
    pub overflow_control: bool,
}

/// Inputs required for deterministic navigation overflow calculation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutConstraints {
    /// Available width or height along the navigation axis.
    pub viewport_extent: u32,
    /// Minimum extent reserved for one item.
    pub item_extent: u32,
    /// Behavior for items outside the viewport.
    pub overflow: OverflowPolicy,
}

impl LayoutConstraints {
    /// Creates layout constraints for a navigation surface.
    pub const fn new(viewport_extent: u32, item_extent: u32, overflow: OverflowPolicy) -> Self {
        Self {
            viewport_extent,
            item_extent,
            overflow,
        }
    }
}

/// A drawer, rail, or bottom-navigation model sharing tab selection policy.
pub struct NavigationMenu<K> {
    surface: NavigationSurface,
    selection: TabBar<K>,
    overflow: OverflowPolicy,
}

impl<K: PartialEq> NavigationMenu<K> {
    /// Creates a navigation menu with the first enabled destination selected.
    pub fn new(surface: NavigationSurface, items: Vec<Tab<K>>) -> Result<Self, NavigationError> {
        Ok(Self {
            surface,
            selection: TabBar::new(items)?.orientation(match surface {
                NavigationSurface::Drawer | NavigationSurface::Rail => Orientation::Vertical,
                NavigationSurface::Bottom => Orientation::Horizontal,
            }),
            overflow: OverflowPolicy::Scroll,
        })
    }

    /// Returns where this menu is intended to be rendered.
    #[inline]
    pub fn surface(&self) -> NavigationSurface {
        self.surface
    }

    /// Returns the shared selection model.
    #[inline]
    pub fn selection(&self) -> &TabBar<K> {
        &self.selection
    }

    /// Returns mutable selection/focus state.
    #[inline]
    pub fn selection_mut(&mut self) -> &mut TabBar<K> {
        &mut self.selection
    }

    /// Sets the default overflow behavior for this menu.
    #[inline]
    pub fn overflow_policy(mut self, overflow: OverflowPolicy) -> Self {
        self.overflow = overflow;
        self
    }

    /// Selects an enabled destination by key.
    #[inline]
    pub fn select_key(&mut self, key: &K) -> bool {
        self.selection.select_key(key)
    }

    /// Handles keyboard navigation for the menu.
    #[inline]
    pub fn handle_key(&mut self, key: NavigationKey) -> NavigationAction {
        self.selection.handle_key(key)
    }

    /// Returns the non-disabled focus order.
    #[inline]
    pub fn focus_order(&self) -> Vec<usize> {
        self.selection.focus_order()
    }

    /// Returns semantic destination items with a navigation-item role.
    #[inline]
    pub fn semantics(&self) -> Vec<NavigationSemantics> {
        self.selection
            .semantics_with_role(SemanticRole::NavigationItem)
    }

    /// Calculates visible and hidden items without making a layout decision for
    /// the eventual renderer.
    pub fn layout(
        &self,
        constraints: LayoutConstraints,
    ) -> Result<NavigationLayout, NavigationError> {
        if constraints.item_extent == 0 {
            return Err(NavigationError::InvalidItemExtent);
        }
        let total = self.selection.tabs().len();
        let visible_count = (constraints.viewport_extent / constraints.item_extent) as usize;
        let visible_count = visible_count.min(total);
        let hidden_count = total - visible_count;
        let overflow_control = hidden_count > 0 && constraints.overflow == OverflowPolicy::Menu;
        Ok(NavigationLayout {
            visible_count,
            hidden_count,
            overflow_control,
        })
    }
}

/// A vertical drawer navigation model.
pub type NavigationDrawer<K> = NavigationMenu<K>;

/// A compact vertical rail navigation model.
pub type NavigationRail<K> = NavigationMenu<K>;

/// A bottom navigation model.
pub type BottomNavigation<K> = NavigationMenu<K>;

/// One location in a breadcrumb trail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Breadcrumb<K> {
    key: K,
    label: String,
    current: bool,
    disabled: bool,
}

impl<K> Breadcrumb<K> {
    /// Creates an enabled, non-current breadcrumb.
    pub fn new(key: K, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            current: false,
            disabled: false,
        }
    }

    /// Marks this breadcrumb as the current location.
    #[inline]
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    /// Returns whether this breadcrumb is the current location.
    #[inline]
    pub fn is_current(&self) -> bool {
        self.current
    }

    /// Marks this breadcrumb as disabled.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Returns whether this breadcrumb can be activated.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns its stable key.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns its label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A breadcrumb trail whose current location can be changed by activation.
pub struct Breadcrumbs<K> {
    items: Vec<Breadcrumb<K>>,
    current: usize,
}

impl<K: PartialEq> Breadcrumbs<K> {
    /// Creates a breadcrumb trail. If no item is explicitly current, the last
    /// enabled item becomes current.
    pub fn new(items: Vec<Breadcrumb<K>>) -> Result<Self, NavigationError> {
        if items.is_empty() {
            return Err(NavigationError::EmptyItems);
        }
        for (index, item) in items.iter().enumerate() {
            if items[index + 1..]
                .iter()
                .any(|other| other.key() == item.key())
            {
                return Err(NavigationError::DuplicateKey);
            }
        }
        let current = items
            .iter()
            .position(|item| item.current && !item.disabled)
            .or_else(|| items.iter().rposition(|item| !item.disabled))
            .ok_or(NavigationError::AllItemsDisabled)?;
        Ok(Self { items, current })
    }

    /// Returns all breadcrumb items.
    #[inline]
    pub fn items(&self) -> &[Breadcrumb<K>] {
        &self.items
    }

    /// Returns the current breadcrumb index.
    #[inline]
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Activates a non-disabled breadcrumb and returns its key.
    pub fn activate(&mut self, index: usize) -> Option<K>
    where
        K: Clone,
    {
        let item = self.items.get(index)?;
        if item.disabled {
            return None;
        }
        self.current = index;
        Some(item.key.clone())
    }

    /// Returns semantic breadcrumb items in focus order.
    pub fn semantics(&self) -> Vec<NavigationSemantics> {
        let set_size = self.items.len();
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| NavigationSemantics {
                role: SemanticRole::Breadcrumb,
                label: item.label.clone(),
                selected: index == self.current,
                disabled: item.disabled,
                focusable: !item.disabled,
                position: index + 1,
                set_size,
            })
            .collect()
    }

    /// Returns non-disabled breadcrumb indices.
    pub fn focus_order(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| (!item.disabled).then_some(index))
            .collect()
    }
}

/// The state of one step in a stepper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepStatus {
    /// The step has not been reached.
    Pending,
    /// The step is active.
    Current,
    /// The step was completed.
    Complete,
    /// The step has an error requiring attention.
    Error,
}

/// One step in a stepper/progress navigation model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step<K> {
    key: K,
    label: String,
    status: StepStatus,
    disabled: bool,
}

impl<K> Step<K> {
    /// Creates an enabled pending step.
    pub fn new(key: K, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            status: StepStatus::Pending,
            disabled: false,
        }
    }

    /// Sets the step status.
    #[inline]
    pub fn status(mut self, status: StepStatus) -> Self {
        self.status = status;
        self
    }

    /// Returns the visible step label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Marks the step as disabled.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Returns whether this step can be activated.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns the key.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns the current status.
    #[inline]
    pub fn status_value(&self) -> StepStatus {
        self.status
    }
}

/// A stepper with explicit current-step and disabled-step policy.
pub struct Stepper<K> {
    steps: Vec<Step<K>>,
    current: usize,
}

impl<K: PartialEq> Stepper<K> {
    /// Creates a stepper from ordered steps. An explicitly current step wins;
    /// otherwise the first enabled step is current.
    pub fn new(steps: Vec<Step<K>>) -> Result<Self, NavigationError> {
        if steps.is_empty() {
            return Err(NavigationError::EmptyItems);
        }
        for (index, step) in steps.iter().enumerate() {
            if steps[index + 1..]
                .iter()
                .any(|other| other.key() == step.key())
            {
                return Err(NavigationError::DuplicateKey);
            }
        }
        let current = steps
            .iter()
            .position(|step| step.status == StepStatus::Current && !step.disabled)
            .or_else(|| steps.iter().position(|step| !step.disabled))
            .ok_or(NavigationError::AllItemsDisabled)?;
        Ok(Self { steps, current })
    }

    /// Returns ordered steps.
    #[inline]
    pub fn steps(&self) -> &[Step<K>] {
        &self.steps
    }

    /// Returns the active step index.
    #[inline]
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Sets the active step, rejecting invalid or disabled indices.
    pub fn set_current(&mut self, index: usize) -> bool {
        let Some(step) = self.steps.get(index) else {
            return false;
        };
        if step.disabled {
            return false;
        }
        self.current = index;
        true
    }

    /// Advances to the next enabled step.
    pub fn next(&mut self) -> bool {
        self.steps
            .get(self.current.saturating_add(1)..)
            .unwrap_or(&[])
            .iter()
            .position(|step| !step.disabled)
            .map(|offset| {
                self.current += offset + 1;
                true
            })
            .unwrap_or(false)
    }

    /// Moves to the previous enabled step.
    pub fn previous(&mut self) -> bool {
        self.steps
            .get(..self.current)
            .and_then(|steps| steps.iter().rposition(|step| !step.disabled))
            .map(|index| {
                self.current = index;
                true
            })
            .unwrap_or(false)
    }

    /// Returns semantic step items in their ordered focus sequence.
    pub fn semantics(&self) -> Vec<NavigationSemantics> {
        let set_size = self.steps.len();
        self.steps
            .iter()
            .enumerate()
            .map(|(index, step)| NavigationSemantics {
                role: SemanticRole::Step,
                label: step.label.clone(),
                selected: index == self.current,
                disabled: step.disabled,
                focusable: !step.disabled,
                position: index + 1,
                set_size,
            })
            .collect()
    }
}
