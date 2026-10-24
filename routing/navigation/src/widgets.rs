//! Aimer widget adapters for the navigation models.
//!
//! The models in [`crate::model`] own selection policy, route identity, and
//! semantic state. This module is the thin UI seam: it retains focus nodes and child
//! builders, translating framework events into [`NavigationKey`] values,
//! and paints the current model state. A portable renderer can replace the
//! presentation without duplicating navigation rules.

use crate::{
    Breadcrumb, Breadcrumbs, NavigationAction, NavigationError, NavigationKey, NavigationSurface,
    Orientation, OverflowPolicy, RouteTab, RouteTabBar, Step, StepStatus, Stepper, Tab, TabBar,
};
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_container::Container;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_flex::{Column, Row};
use aimer_focus::FocusNode;
use aimer_input::button::Button;
use aimer_provider::ProviderContext;
use aimer_router::{NavigatorController, Route};
use aimer_style::{BoxDecoration, LayoutSpacing, TextStyle, ThemeData};
use aimer_text::Text;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Drawable, Element, ErrorWidget, EventElement, EventResult,
    Focusable, LayoutElement, PortableWidget, Rebuildable, RequiredChild, State, StateUpdater,
    StatefulElement, StatefulWidget, VisitorElement, Widget,
};
use std::rc::Rc;

/// A callback invoked after a navigation item is activated.
pub type NavigationCallback<K> = Rc<dyn Fn(K)>;

/// A callback invoked when a menu moves items into its overflow affordance.
pub type OverflowCallback<K> = Rc<dyn Fn(Vec<K>)>;

#[derive(Clone, Copy)]
struct NavigationPalette {
    normal_background: aimer_widget::base::Color,
    selected_background: aimer_widget::base::Color,
    focused_background: aimer_widget::base::Color,
    foreground: aimer_widget::base::Color,
    disabled_foreground: aimer_widget::base::Color,
}

fn navigation_palette(ctx: &BuildContext) -> NavigationPalette {
    let theme = ctx
        .try_copied::<ThemeData>()
        .unwrap_or_else(ThemeData::light);
    NavigationPalette {
        normal_background: theme.surface_color,
        selected_background: theme.primary_color.with_opacity(42),
        focused_background: theme.primary_color.with_opacity(24),
        foreground: theme.on_surface_color,
        disabled_foreground: theme.on_surface_color.with_opacity(96),
    }
}

#[derive(Clone, Copy)]
enum ItemInteraction {
    Activate(usize),
    Key(NavigationKey),
    FocusChanged(usize, bool),
}

/// The retained adapter state shared by tab bars and navigation surfaces.
pub(crate) struct SelectionRuntime<K> {
    tabs: Vec<Tab<K>>,
    model: Result<TabBar<K>, NavigationError>,
    nodes: Vec<FocusNode>,
}

impl<K: Clone + PartialEq> SelectionRuntime<K> {
    pub(crate) fn new(tabs: Vec<Tab<K>>, orientation: Orientation, wrap_navigation: bool) -> Self {
        let model = TabBar::new(tabs.clone()).map(|bar| {
            bar.orientation(orientation)
                .wrap_navigation(wrap_navigation)
        });
        let nodes = tabs.iter().map(|_| FocusNode::new()).collect();
        Self { tabs, model, nodes }
    }

    fn adopt_config_from(&mut self, mut new: Self) {
        let selected_key = self
            .model
            .as_ref()
            .ok()
            .and_then(|model| model.selected_key().cloned());
        let focused_key = self
            .model
            .as_ref()
            .ok()
            .and_then(|model| model.focused_key().cloned());

        let mut nodes = new
            .tabs
            .iter()
            .map(|_| FocusNode::new())
            .collect::<Vec<_>>();
        for (new_index, tab) in new.tabs.iter().enumerate() {
            if let Some(old_index) = self.tabs.iter().position(|old| old.key() == tab.key()) {
                nodes[new_index] = self.nodes[old_index].clone();
            }
        }
        if let Ok(model) = new.model.as_mut() {
            if let Some(key) = selected_key.as_ref() {
                let _ = model.select_key(key);
            }
            if let Some(key) = focused_key.as_ref() {
                let _ = model.focus_key(key);
            }
        }
        new.nodes = nodes;
        *self = new;
    }

    fn activate(&mut self, index: usize) -> Option<usize> {
        let model = self.model.as_mut().ok()?;
        if !matches!(model.select(index), NavigationAction::Activated(_)) {
            return None;
        }
        if let Some(node) = self.nodes.get(index) {
            node.request_focus();
        }
        Some(index)
    }

    fn focus(&mut self, index: usize) {
        if let Ok(model) = self.model.as_mut() {
            if matches!(model.focus(index), NavigationAction::FocusChanged(_))
                && let Some(node) = self.nodes.get(index)
            {
                node.request_focus();
            }
        }
    }

    pub(crate) fn handle_key(&mut self, key: NavigationKey) -> NavigationAction {
        let Ok(model) = self.model.as_mut() else {
            return NavigationAction::Ignored;
        };
        let action = model.handle_key(key);
        match action {
            NavigationAction::FocusChanged(index) | NavigationAction::Activated(index) => {
                if let Some(node) = self.nodes.get(index) {
                    node.request_focus();
                }
            }
            NavigationAction::Ignored
            | NavigationAction::Disabled(_)
            | NavigationAction::TabbedAway => {}
        }
        action
    }
}

fn build_selection_surface(orientation: Orientation, children: Vec<AnyWidget>) -> AnyWidget {
    match orientation {
        Orientation::Horizontal => Row::new().children(children).boxed(),
        Orientation::Vertical => Column::new().children(children).boxed(),
    }
}

fn build_tab_items<K: Clone + PartialEq + ToString + 'static>(
    tabs: &[Tab<K>],
    indices: impl IntoIterator<Item = usize>,
    selected: Option<usize>,
    focused: Option<usize>,
    nodes: &[FocusNode],
    handler: Rc<dyn Fn(ItemInteraction)>,
    prefix: &str,
    palette: NavigationPalette,
) -> Vec<AnyWidget> {
    indices
        .into_iter()
        .filter_map(|index| {
            let tab = tabs.get(index)?;
            let node = nodes.get(index)?.clone();
            let disabled = tab.is_disabled();
            let is_selected = selected == Some(index);
            let is_focused = focused == Some(index);
            let marker = if disabled {
                "× "
            } else if is_selected {
                "● "
            } else if is_focused {
                "› "
            } else {
                "  "
            };
            let label = format!("{marker}{}", tab.label());
            let item_key = format!("{prefix}:{}", tab.key().to_string());
            let background = if is_selected {
                palette.selected_background
            } else if is_focused {
                palette.focused_background
            } else {
                palette.normal_background
            };
            let foreground = if disabled {
                palette.disabled_foreground
            } else {
                palette.foreground
            };

            let key_handler = handler.clone();
            let focus_handler = handler.clone();
            let press_handler = handler.clone();
            let button = Button::new()
                .key(item_key.clone())
                .disabled(disabled)
                .on_press(move || press_handler(ItemInteraction::Activate(index)))
                .decoration(
                    BoxDecoration::new()
                        .background_color(background)
                        .border_radius(8.0),
                )
                .child(
                    Container::new()
                        .padding(LayoutSpacing::all(8))
                        .child(Text::new(label).text_style(TextStyle::new().color(foreground))),
                );
            let relay = NavigationKeyRelay::new()
                .on_key(move |key| {
                    if matches!(key, NavigationKey::Tab | NavigationKey::ShiftTab) {
                        false
                    } else {
                        key_handler(ItemInteraction::Key(key));
                        true
                    }
                })
                .child(button);
            Some(
                Focusable::new()
                    .node(node)
                    .key(item_key)
                    .focusable_when(move || !disabled)
                    .on_focus_change(move |gained| {
                        focus_handler(ItemInteraction::FocusChanged(index, gained))
                    })
                    .child(relay)
                    .boxed(),
            )
        })
        .collect()
}

fn apply_selection_interaction<K: Clone + PartialEq>(
    runtime: &mut SelectionRuntime<K>,
    interaction: ItemInteraction,
) -> Option<usize> {
    match interaction {
        ItemInteraction::Activate(index) => runtime.activate(index),
        ItemInteraction::Key(key) => match runtime.handle_key(key) {
            NavigationAction::Activated(index) => Some(index),
            _ => None,
        },
        ItemInteraction::FocusChanged(index, true) => {
            runtime.focus(index);
            None
        }
        ItemInteraction::FocusChanged(_, false) => None,
    }
}

fn selected_key<K: Clone>(runtime: &SelectionRuntime<K>, index: usize) -> Option<K> {
    runtime.tabs.get(index).map(|tab| tab.key().clone())
}

fn apply_menu_interaction<K: Clone + PartialEq>(
    runtime: &mut SelectionRuntime<K>,
    interaction: ItemInteraction,
    visible_count: usize,
    overflow_node: &FocusNode,
) -> Option<usize> {
    match interaction {
        ItemInteraction::Key(key) => match runtime.handle_key(key) {
            NavigationAction::FocusChanged(index) if index >= visible_count => {
                if let Some(last_visible) = visible_count.checked_sub(1) {
                    if let Ok(model) = runtime.model.as_mut() {
                        let _ = model.focus(last_visible);
                    }
                }
                overflow_node.request_focus();
                None
            }
            NavigationAction::Activated(index) if index >= visible_count => {
                overflow_node.request_focus();
                None
            }
            NavigationAction::Activated(index) => Some(index),
            NavigationAction::FocusChanged(_) => None,
            NavigationAction::Ignored
            | NavigationAction::Disabled(_)
            | NavigationAction::TabbedAway => None,
        },
        other => apply_selection_interaction(runtime, other),
    }
}

/// A focusable, keyboard-driven tab list.
pub struct TabBarWidget<K = String> {
    tabs: Vec<Tab<K>>,
    orientation: Orientation,
    wrap_navigation: bool,
    on_changed: Option<NavigationCallback<K>>,
}

impl<K> TabBarWidget<K> {
    /// Creates an empty builder. Supply items with [`Self::with_tabs`].
    #[inline]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            orientation: Orientation::Horizontal,
            wrap_navigation: true,
            on_changed: None,
        }
    }
}

impl<K> Default for TabBarWidget<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> TabBarWidget<K> {
    /// Creates a tab bar directly from its model items.
    #[inline]
    pub fn from_tabs(tabs: Vec<Tab<K>>) -> Self {
        Self::new().with_tabs(tabs)
    }

    /// Replaces the tab definitions while retaining matching focus nodes.
    #[inline]
    pub fn with_tabs(mut self, tabs: Vec<Tab<K>>) -> Self {
        self.tabs = tabs;
        self
    }

    /// Returns the configured tab definitions.
    #[inline]
    pub fn tabs(&self) -> &[Tab<K>] {
        &self.tabs
    }

    /// Sets the keyboard arrow axis.
    #[inline]
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Enables or disables wrapping when an arrow reaches an edge.
    #[inline]
    pub fn wrap_navigation(mut self, wrap: bool) -> Self {
        self.wrap_navigation = wrap;
        self
    }

    /// Registers a callback for activated tab keys.
    #[inline]
    pub fn on_changed(mut self, callback: impl Fn(K) + 'static) -> Self {
        self.on_changed = Some(Rc::new(callback));
        self
    }
}

pub struct TabBarWidgetState<K> {
    runtime: SelectionRuntime<K>,
    orientation: Orientation,
    on_changed: Option<NavigationCallback<K>>,
    updater: StateUpdater<Self>,
}

impl<K: Clone + PartialEq + ToString + 'static> StatefulWidget for TabBarWidget<K> {
    type State = TabBarWidgetState<K>;

    fn create_state(self) -> Self::State {
        TabBarWidgetState {
            runtime: SelectionRuntime::new(self.tabs, self.orientation, self.wrap_navigation),
            orientation: self.orientation,
            on_changed: self.on_changed,
            updater: StateUpdater::empty(),
        }
    }
}

impl<K: Clone + PartialEq + ToString + 'static> Widget for TabBarWidget<K> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "TabBarWidget", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "TabBarWidget"
    }
}

impl<K: Clone + PartialEq + ToString + 'static> PortableWidget for TabBarWidget<K> {}

impl<K: Clone + PartialEq + ToString + 'static> State<TabBarWidget<K>> for TabBarWidgetState<K> {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self)
    where
        Self: Sized,
    {
        self.runtime.adopt_config_from(new.runtime);
        self.orientation = new.orientation;
        self.on_changed = new.on_changed;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let Some(model) = self.runtime.model.as_ref().ok() else {
            return ErrorWidget::new(
                self.runtime
                    .model
                    .as_ref()
                    .err()
                    .expect("navigation model error should be present")
                    .to_string(),
            )
            .boxed();
        };
        let handler = {
            let updater = self.updater;
            Rc::new(move |interaction| {
                updater.set_state(move |state| {
                    if let Some(index) =
                        apply_selection_interaction(&mut state.runtime, interaction)
                    {
                        if let Some(key) = selected_key(&state.runtime, index) {
                            if let Some(callback) = state.on_changed.as_ref() {
                                callback(key);
                            }
                        }
                    }
                });
            })
        };
        let children = build_tab_items(
            &self.runtime.tabs,
            0..self.runtime.tabs.len(),
            Some(model.selected_index()),
            Some(model.focused_index()),
            &self.runtime.nodes,
            handler,
            "tabs",
            navigation_palette(ctx),
        );
        build_selection_surface(self.orientation, children)
    }
}

/// A drawer, rail, or bottom-navigation widget backed by [`NavigationMenu`].
pub struct NavigationMenuWidget<K = String> {
    surface: NavigationSurface,
    items: Vec<Tab<K>>,
    overflow: OverflowPolicy,
    item_extent: u32,
    on_changed: Option<NavigationCallback<K>>,
    on_overflow: Option<OverflowCallback<K>>,
}

impl<K> NavigationMenuWidget<K> {
    /// Creates a drawer-shaped empty builder. Supply items with [`Self::with_items`].
    #[inline]
    pub fn new() -> Self {
        Self {
            surface: NavigationSurface::Drawer,
            items: Vec::new(),
            overflow: OverflowPolicy::Scroll,
            item_extent: 56,
            on_changed: None,
            on_overflow: None,
        }
    }
}

impl<K> Default for NavigationMenuWidget<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> NavigationMenuWidget<K> {
    /// Creates a navigation surface from its model items.
    #[inline]
    pub fn from_items(surface: NavigationSurface, items: Vec<Tab<K>>) -> Self {
        Self::new().with_surface(surface).with_items(items)
    }

    /// Sets the drawer, rail, or bottom placement.
    #[inline]
    pub fn with_surface(mut self, surface: NavigationSurface) -> Self {
        self.surface = surface;
        self
    }

    /// Replaces navigation destinations.
    #[inline]
    pub fn with_items(mut self, items: Vec<Tab<K>>) -> Self {
        self.items = items;
        self
    }

    /// Returns the configured navigation destinations.
    #[inline]
    pub fn items(&self) -> &[Tab<K>] {
        &self.items
    }

    /// Sets the narrow-viewport policy.
    #[inline]
    pub fn overflow_policy(mut self, overflow: OverflowPolicy) -> Self {
        self.overflow = overflow;
        self
    }

    /// Sets the minimum logical extent reserved for one item.
    #[inline]
    pub fn item_extent(mut self, item_extent: u32) -> Self {
        self.item_extent = item_extent;
        self
    }

    /// Registers a callback for activated destinations.
    #[inline]
    pub fn on_changed(mut self, callback: impl Fn(K) + 'static) -> Self {
        self.on_changed = Some(Rc::new(callback));
        self
    }

    /// Registers a callback receiving keys hidden behind the overflow control.
    #[inline]
    pub fn on_overflow(mut self, callback: impl Fn(Vec<K>) + 'static) -> Self {
        self.on_overflow = Some(Rc::new(callback));
        self
    }

    /// Returns the configured placement.
    #[inline]
    pub fn surface(&self) -> NavigationSurface {
        self.surface
    }
}

pub struct NavigationMenuWidgetState<K> {
    runtime: SelectionRuntime<K>,
    overflow: OverflowPolicy,
    item_extent: u32,
    on_changed: Option<NavigationCallback<K>>,
    on_overflow: Option<OverflowCallback<K>>,
    overflow_node: FocusNode,
    updater: StateUpdater<Self>,
}

impl<K: Clone + PartialEq + ToString + 'static> StatefulWidget for NavigationMenuWidget<K> {
    type State = NavigationMenuWidgetState<K>;

    fn create_state(self) -> Self::State {
        let orientation = match self.surface {
            NavigationSurface::Drawer | NavigationSurface::Rail => Orientation::Vertical,
            NavigationSurface::Bottom => Orientation::Horizontal,
        };
        NavigationMenuWidgetState {
            runtime: SelectionRuntime::new(self.items, orientation, true),
            overflow: self.overflow,
            item_extent: self.item_extent,
            on_changed: self.on_changed,
            on_overflow: self.on_overflow,
            overflow_node: FocusNode::new(),
            updater: StateUpdater::empty(),
        }
    }
}

impl<K: Clone + PartialEq + ToString + 'static> Widget for NavigationMenuWidget<K> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "NavigationMenuWidget", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "NavigationMenuWidget"
    }
}

impl<K: Clone + PartialEq + ToString + 'static> PortableWidget for NavigationMenuWidget<K> {}

impl<K: Clone + PartialEq + ToString + 'static> State<NavigationMenuWidget<K>>
    for NavigationMenuWidgetState<K>
{
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self)
    where
        Self: Sized,
    {
        self.runtime.adopt_config_from(new.runtime);
        self.overflow = new.overflow;
        self.item_extent = new.item_extent;
        self.on_changed = new.on_changed;
        self.on_overflow = new.on_overflow;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let Some(model) = self.runtime.model.as_ref().ok() else {
            return ErrorWidget::new(
                self.runtime
                    .model
                    .as_ref()
                    .err()
                    .expect("navigation model error should be present")
                    .to_string(),
            )
            .boxed();
        };
        if self.item_extent == 0 {
            return ErrorWidget::new(NavigationError::InvalidItemExtent.to_string()).boxed();
        }

        let orientation = model.navigation_orientation();
        let window_extent = ctx.watch_window_metrics().logical_size();
        let extent = match orientation {
            Orientation::Horizontal if ctx.parent_size.width > 0.0 => ctx.parent_size.width,
            Orientation::Vertical if ctx.parent_size.height > 0.0 => ctx.parent_size.height,
            Orientation::Horizontal => window_extent.width,
            Orientation::Vertical => window_extent.height,
        };
        let visible_count =
            ((extent / self.item_extent as f32).floor() as usize).min(self.runtime.tabs.len());
        let visible_count = if self.overflow == OverflowPolicy::Menu {
            if visible_count < self.runtime.tabs.len() {
                visible_count.saturating_sub(1)
            } else {
                visible_count
            }
        } else {
            self.runtime.tabs.len()
        };
        let selected = Some(model.selected_index());
        let focused = Some(model.focused_index());
        let handler = {
            let updater = self.updater;
            let overflow_node = self.overflow_node.clone();
            Rc::new(move |interaction| {
                let overflow_node = overflow_node.clone();
                updater.set_state(move |state| {
                    if let Some(index) = apply_menu_interaction(
                        &mut state.runtime,
                        interaction,
                        visible_count,
                        &overflow_node,
                    ) {
                        if let Some(key) = selected_key(&state.runtime, index) {
                            if let Some(callback) = state.on_changed.as_ref() {
                                callback(key);
                            }
                        }
                    }
                });
            })
        };
        let mut children = build_tab_items(
            &self.runtime.tabs,
            0..visible_count,
            selected,
            focused,
            &self.runtime.nodes,
            handler,
            "menu",
            navigation_palette(ctx),
        );
        if visible_count < self.runtime.tabs.len() && self.overflow == OverflowPolicy::Menu {
            let hidden_keys = self.runtime.tabs[visible_count..]
                .iter()
                .map(|tab| tab.key().clone())
                .collect::<Vec<_>>();
            let label = format!("› More ({})", hidden_keys.len());
            let callback = self.on_overflow.clone();
            let node = self.overflow_node.clone();
            let press_keys = hidden_keys.clone();
            let press_callback = callback.clone();
            let button = Button::new()
                .on_press(move || {
                    if let Some(callback) = press_callback.as_ref() {
                        callback(press_keys.clone());
                    }
                })
                .decoration(
                    BoxDecoration::new()
                        .background_color(navigation_palette(ctx).focused_background)
                        .border_radius(8.0),
                )
                .child(
                    Container::new()
                        .padding(LayoutSpacing::all(8))
                        .child(Text::new(label)),
                );
            let key_keys = hidden_keys.clone();
            let overflow_button = Focusable::new()
                .node(node)
                .key("navigation:overflow")
                .child(
                    NavigationKeyRelay::new()
                        .on_key(move |key| {
                            if matches!(key, NavigationKey::Enter | NavigationKey::Space) {
                                if let Some(callback) = callback.as_ref() {
                                    callback(key_keys.clone());
                                }
                                true
                            } else {
                                false
                            }
                        })
                        .child(button),
                )
                .boxed();
            children.push(overflow_button);
        }
        build_selection_surface(orientation, children)
    }
}

/// A route-backed tab list that synchronizes its selected item with a
/// [`NavigatorController`].
pub struct RouteTabBarWidget<R: Route> {
    tabs: Vec<RouteTab<R>>,
    on_changed: Option<NavigationCallback<R>>,
}

impl<R: Route> RouteTabBarWidget<R> {
    /// Creates an empty route-tab builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            on_changed: None,
        }
    }

    /// Creates a route tab bar directly from route tabs.
    #[inline]
    pub fn from_tabs(tabs: Vec<RouteTab<R>>) -> Self {
        Self::new().tabs(tabs)
    }

    /// Replaces the route-tab definitions.
    #[inline]
    pub fn tabs(mut self, tabs: Vec<RouteTab<R>>) -> Self {
        self.tabs = tabs;
        self
    }

    /// Registers a callback in addition to the navigator push.
    #[inline]
    pub fn on_changed(mut self, callback: impl Fn(R) + 'static) -> Self {
        self.on_changed = Some(Rc::new(callback));
        self
    }
}

impl<R: Route> Default for RouteTabBarWidget<R> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RouteTabBarWidgetState<R: Route> {
    model: std::cell::RefCell<Result<RouteTabBar<R>, NavigationError>>,
    nodes: Vec<FocusNode>,
    on_changed: Option<NavigationCallback<R>>,
    updater: StateUpdater<Self>,
}

fn route_tabs_as_tabs<R: Route>(model: &RouteTabBar<R>) -> Vec<Tab<String>> {
    model
        .tabs()
        .iter()
        .map(|tab| Tab::new(tab.key().to_owned(), tab.label()).disabled(tab.is_disabled()))
        .collect()
}

fn apply_route_interaction<R: Route>(
    state: &mut RouteTabBarWidgetState<R>,
    interaction: ItemInteraction,
    navigator: Option<&NavigatorController<R>>,
) -> Option<R> {
    let mut model = state.model.borrow_mut();
    let model = model.as_mut().ok()?;
    let index = match interaction {
        ItemInteraction::Activate(index) => match model.activate(index) {
            crate::RouteActivation::Pushed(index) => index,
            crate::RouteActivation::Disabled(_) | crate::RouteActivation::InvalidIndex => {
                return None;
            }
        },
        ItemInteraction::Key(key) => match model.handle_key(key) {
            NavigationAction::Activated(index) => index,
            NavigationAction::FocusChanged(index) => {
                state.nodes.get(index).map(FocusNode::request_focus);
                return None;
            }
            _ => return None,
        },
        ItemInteraction::FocusChanged(index, true) => {
            let _ = model.focus(index);
            return None;
        }
        ItemInteraction::FocusChanged(_, false) => return None,
    };
    let route = model.route_for(index)?.clone();
    if let Some(navigator) = navigator {
        navigator.push(route.clone());
    }
    Some(route)
}

impl<R: Route> StatefulWidget for RouteTabBarWidget<R> {
    type State = RouteTabBarWidgetState<R>;

    fn create_state(self) -> Self::State {
        let nodes = self.tabs.iter().map(|_| FocusNode::new()).collect();
        let model = RouteTabBar::new(self.tabs);
        RouteTabBarWidgetState {
            model: std::cell::RefCell::new(model),
            nodes,
            on_changed: self.on_changed,
            updater: StateUpdater::empty(),
        }
    }
}

impl<R: Route> Widget for RouteTabBarWidget<R> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "RouteTabBarWidget", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "RouteTabBarWidget"
    }
}

impl<R: Route> PortableWidget for RouteTabBarWidget<R> {}

impl<R: Route> State<RouteTabBarWidget<R>> for RouteTabBarWidgetState<R> {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self)
    where
        Self: Sized,
    {
        let (old_keys, selected_route, focused_route) = {
            let model = self.model.borrow();
            match model.as_ref() {
                Ok(model) => (
                    model
                        .tabs()
                        .iter()
                        .map(|tab| tab.key().to_owned())
                        .collect::<Vec<_>>(),
                    model
                        .route_for(model.selected_index())
                        .map(|route| route.format()),
                    model
                        .route_for(model.focused_index())
                        .map(|route| route.format()),
                ),
                Err(_) => (Vec::new(), None, None),
            }
        };
        let old_nodes = std::mem::replace(&mut self.nodes, Vec::new());
        let mut new_model = new.model.into_inner();
        let new_nodes_len = new_model
            .as_ref()
            .map(|model| model.tabs().len())
            .unwrap_or(0);
        let mut nodes = (0..new_nodes_len)
            .map(|_| FocusNode::new())
            .collect::<Vec<_>>();
        if let Ok(new_model_ref) = new_model.as_ref() {
            for (new_index, new_tab) in new_model_ref.tabs().iter().enumerate() {
                if let Some(old_index) = old_keys.iter().position(|key| key == new_tab.key()) {
                    nodes[new_index] = old_nodes[old_index].clone();
                }
            }
        }
        if let Ok(model) = new_model.as_mut() {
            if let Some(route_format) = selected_route.as_ref()
                && let Some(index) = model
                    .tabs()
                    .iter()
                    .position(|tab| tab.route().format() == route_format.as_str())
            {
                let _ = model.select(index);
            }
            if let Some(route_format) = focused_route.as_ref()
                && let Some(index) = model
                    .tabs()
                    .iter()
                    .position(|tab| tab.route().format() == route_format.as_str())
            {
                let _ = model.focus(index);
            }
        }
        self.model = std::cell::RefCell::new(new_model);
        self.nodes = nodes;
        self.on_changed = new.on_changed;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let navigator = ctx
            .get_state::<NavigatorController<R>>()
            .map(|controller| controller.as_ref().clone());
        if let Some(navigator) = navigator.as_ref() {
            if let Ok(mut model) = self.model.try_borrow_mut() {
                let _ = model
                    .as_mut()
                    .map(|route_model| route_model.sync_from_navigator(navigator));
            }
        }
        let model = self.model.borrow();
        let Some(model) = model.as_ref().ok() else {
            let message = model
                .as_ref()
                .err()
                .expect("route model error should be present")
                .to_string();
            drop(model);
            return ErrorWidget::new(message).boxed();
        };
        let tabs = route_tabs_as_tabs(model);
        let handler = {
            let updater = self.updater;
            let navigator = navigator.clone();
            Rc::new(move |interaction| {
                let updater = updater;
                let navigator = navigator.clone();
                updater.set_state(move |state| {
                    if let Some(route) =
                        apply_route_interaction(state, interaction, navigator.as_ref())
                    {
                        if let Some(callback) = state.on_changed.as_ref() {
                            callback(route);
                        }
                    }
                });
            })
        };
        let children = build_tab_items(
            &tabs,
            0..tabs.len(),
            Some(model.selected_index()),
            Some(model.focused_index()),
            &self.nodes,
            handler,
            "route-tabs",
            navigation_palette(ctx),
        );
        build_selection_surface(Orientation::Horizontal, children)
    }
}

/// A tab list and retained page branch.
pub struct TabViewWidget<K = String> {
    tabs: Vec<Tab<K>>,
    pages: Vec<AnyWidget>,
    orientation: Orientation,
    wrap_navigation: bool,
    on_changed: Option<NavigationCallback<K>>,
}

impl TabViewWidget {
    /// Creates an empty builder. Supply pages with [`Self::from_pages`].
    #[inline]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            pages: Vec::new(),
            orientation: Orientation::Horizontal,
            wrap_navigation: true,
            on_changed: None,
        }
    }
}

impl Default for TabViewWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> TabViewWidget<K> {
    /// Creates a tab view from stable tab/page pairs.
    #[inline]
    pub fn from_pages(pages: Vec<(Tab<K>, AnyWidget)>) -> Self {
        let (tabs, page_widgets): (Vec<_>, Vec<_>) = pages.into_iter().unzip();
        Self {
            tabs,
            pages: page_widgets,
            orientation: Orientation::Horizontal,
            wrap_navigation: true,
            on_changed: None,
        }
    }

    /// Returns the configured tab definitions.
    #[inline]
    pub fn tabs(&self) -> &[Tab<K>] {
        &self.tabs
    }

    /// Returns the configured page widgets before this builder is mounted.
    #[inline]
    pub fn pages(&self) -> &[AnyWidget] {
        &self.pages
    }

    /// Sets the keyboard arrow axis for the tab header.
    #[inline]
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Enables or disables wrapping in the tab header.
    #[inline]
    pub fn wrap_navigation(mut self, wrap: bool) -> Self {
        self.wrap_navigation = wrap;
        self
    }

    /// Registers a callback for selected tab keys.
    #[inline]
    pub fn on_changed(mut self, callback: impl Fn(K) + 'static) -> Self {
        self.on_changed = Some(Rc::new(callback));
        self
    }
}

struct TabViewPage<K> {
    key: K,
    child: ChildBuilder,
}

struct TabViewRuntime<K> {
    tabs: Vec<Tab<K>>,
    pages: Vec<TabViewPage<K>>,
    model: Result<TabBar<K>, NavigationError>,
    nodes: Vec<FocusNode>,
}

fn apply_tab_view_interaction<K: Clone + PartialEq>(
    runtime: &mut TabViewRuntime<K>,
    interaction: ItemInteraction,
) -> Option<usize> {
    let model = runtime.model.as_mut().ok()?;
    match interaction {
        ItemInteraction::Activate(index) => {
            if matches!(model.select(index), NavigationAction::Activated(_)) {
                runtime.nodes.get(index).map(FocusNode::request_focus);
                Some(index)
            } else {
                None
            }
        }
        ItemInteraction::Key(key) => match model.handle_key(key) {
            NavigationAction::Activated(index) => {
                runtime.nodes.get(index).map(FocusNode::request_focus);
                Some(index)
            }
            NavigationAction::FocusChanged(index) => {
                runtime.nodes.get(index).map(FocusNode::request_focus);
                None
            }
            _ => None,
        },
        ItemInteraction::FocusChanged(index, true) => {
            let _ = model.focus(index);
            None
        }
        ItemInteraction::FocusChanged(_, false) => None,
    }
}

impl<K: Clone + PartialEq> TabViewRuntime<K> {
    fn new(
        tabs: Vec<Tab<K>>,
        pages: Vec<AnyWidget>,
        orientation: Orientation,
        wrap_navigation: bool,
    ) -> Self {
        let page_entries = tabs
            .iter()
            .zip(pages)
            .map(|(tab, child)| TabViewPage {
                key: tab.key().clone(),
                child: ChildBuilder::from_widget(child),
            })
            .collect();
        let model = TabBar::new(tabs.clone()).map(|bar| {
            bar.orientation(orientation)
                .wrap_navigation(wrap_navigation)
        });
        let nodes = tabs.iter().map(|_| FocusNode::new()).collect();
        Self {
            tabs,
            pages: page_entries,
            model,
            nodes,
        }
    }

    fn adopt_config_from(&mut self, mut new: Self) {
        let selected_key = self
            .model
            .as_ref()
            .ok()
            .and_then(|model| model.selected_key().cloned());
        let focused_key = self
            .model
            .as_ref()
            .ok()
            .and_then(|model| model.focused_key().cloned());
        let old_pages = std::mem::replace(&mut self.pages, Vec::new());
        for entry in &mut new.pages {
            if let Some(index) = old_pages.iter().position(|old| old.key == entry.key) {
                entry.child = old_pages[index].child.clone();
            }
        }
        let mut nodes = new
            .tabs
            .iter()
            .map(|_| FocusNode::new())
            .collect::<Vec<_>>();
        for (new_index, tab) in new.tabs.iter().enumerate() {
            if let Some(old_index) = self.tabs.iter().position(|old| old.key() == tab.key()) {
                nodes[new_index] = self.nodes[old_index].clone();
            }
        }
        if let Ok(model) = new.model.as_mut() {
            if let Some(key) = selected_key.as_ref() {
                let _ = model.select_key(key);
            }
            if let Some(key) = focused_key.as_ref() {
                let _ = model.focus_key(key);
            }
        }
        new.nodes = nodes;
        *self = new;
    }
}

pub struct TabViewWidgetState<K> {
    runtime: TabViewRuntime<K>,
    orientation: Orientation,
    on_changed: Option<NavigationCallback<K>>,
    updater: StateUpdater<Self>,
}

impl<K: Clone + PartialEq + ToString + 'static> StatefulWidget for TabViewWidget<K> {
    type State = TabViewWidgetState<K>;

    fn create_state(self) -> Self::State {
        TabViewWidgetState {
            runtime: TabViewRuntime::new(
                self.tabs,
                self.pages,
                self.orientation,
                self.wrap_navigation,
            ),
            orientation: self.orientation,
            on_changed: self.on_changed,
            updater: StateUpdater::empty(),
        }
    }
}

impl<K: Clone + PartialEq + ToString + 'static> Widget for TabViewWidget<K> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "TabViewWidget", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "TabViewWidget"
    }
}

impl<K: Clone + PartialEq + ToString + 'static> PortableWidget for TabViewWidget<K> {}

impl<K: Clone + PartialEq + ToString + 'static> State<TabViewWidget<K>> for TabViewWidgetState<K> {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self)
    where
        Self: Sized,
    {
        self.runtime.adopt_config_from(new.runtime);
        self.orientation = new.orientation;
        self.on_changed = new.on_changed;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let Some(model) = self.runtime.model.as_ref().ok() else {
            return ErrorWidget::new(
                self.runtime
                    .model
                    .as_ref()
                    .err()
                    .expect("tab view model error should be present")
                    .to_string(),
            )
            .boxed();
        };
        let handler = {
            let updater = self.updater;
            Rc::new(move |interaction| {
                updater.set_state(move |state| {
                    if let Some(index) = apply_tab_view_interaction(&mut state.runtime, interaction)
                    {
                        if let Some(key) =
                            state.runtime.tabs.get(index).map(|tab| tab.key().clone())
                        {
                            if let Some(callback) = state.on_changed.as_ref() {
                                callback(key);
                            }
                        }
                    }
                });
            })
        };
        let header = build_tab_items(
            &self.runtime.tabs,
            0..self.runtime.tabs.len(),
            Some(model.selected_index()),
            Some(model.focused_index()),
            &self.runtime.nodes,
            handler,
            "tab-view",
            navigation_palette(ctx),
        );
        let header = build_selection_surface(self.orientation, header);
        let page = self
            .runtime
            .pages
            .get(model.selected_index())
            .map(|page| page.child.clone().boxed())
            .unwrap_or_else(|| ErrorWidget::new("selected tab has no page").boxed());
        Column::new().children([header, page]).boxed()
    }
}

fn focus_model<K: Clone + PartialEq>(
    tabs: Vec<Tab<K>>,
    selected: usize,
    orientation: Orientation,
) -> Result<TabBar<K>, NavigationError> {
    let mut model = TabBar::new(tabs)?.orientation(orientation);
    let _ = model.select(selected);
    Ok(model)
}

/// A breadcrumb trail rendered as a focusable row.
pub struct BreadcrumbsWidget<K = String> {
    items: Vec<Breadcrumb<K>>,
    on_changed: Option<NavigationCallback<K>>,
}

impl<K> BreadcrumbsWidget<K> {
    /// Creates an empty breadcrumb builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            on_changed: None,
        }
    }

    /// Creates a breadcrumb widget from model items.
    #[inline]
    pub fn from_items(items: Vec<Breadcrumb<K>>) -> Self {
        Self::new().with_items(items)
    }

    /// Replaces the breadcrumb definitions.
    #[inline]
    pub fn with_items(mut self, items: Vec<Breadcrumb<K>>) -> Self {
        self.items = items;
        self
    }

    /// Returns the configured breadcrumb definitions.
    #[inline]
    pub fn items(&self) -> &[Breadcrumb<K>] {
        &self.items
    }

    /// Registers a callback for activated locations.
    #[inline]
    pub fn on_changed(mut self, callback: impl Fn(K) + 'static) -> Self {
        self.on_changed = Some(Rc::new(callback));
        self
    }
}

impl<K> Default for BreadcrumbsWidget<K> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BreadcrumbsWidgetState<K> {
    model: Result<Breadcrumbs<K>, NavigationError>,
    focus: Result<TabBar<K>, NavigationError>,
    nodes: Vec<FocusNode>,
    on_changed: Option<NavigationCallback<K>>,
    updater: StateUpdater<Self>,
}

fn breadcrumb_tabs<K: Clone + PartialEq>(model: &Breadcrumbs<K>) -> Vec<Tab<K>> {
    model
        .items()
        .iter()
        .map(|item| Tab::new(item.key().clone(), item.label()).disabled(item.is_disabled()))
        .collect()
}

fn apply_breadcrumb_interaction<K: Clone + PartialEq>(
    state: &mut BreadcrumbsWidgetState<K>,
    interaction: ItemInteraction,
) -> Option<K> {
    match interaction {
        ItemInteraction::Activate(index) => {
            let key = state.model.as_mut().ok()?.activate(index)?;
            if let Ok(focus) = state.focus.as_mut() {
                let _ = focus.select(index);
            }
            state.nodes.get(index).map(FocusNode::request_focus);
            Some(key)
        }
        ItemInteraction::Key(key) => {
            let action = state.focus.as_mut().ok()?.handle_key(key);
            match action {
                NavigationAction::FocusChanged(index) => {
                    state.nodes.get(index).map(FocusNode::request_focus);
                    None
                }
                NavigationAction::Activated(index) => {
                    let key = state.model.as_mut().ok()?.activate(index)?;
                    state.nodes.get(index).map(FocusNode::request_focus);
                    Some(key)
                }
                NavigationAction::Ignored
                | NavigationAction::Disabled(_)
                | NavigationAction::TabbedAway => None,
            }
        }
        ItemInteraction::FocusChanged(index, true) => {
            if let Ok(focus) = state.focus.as_mut() {
                let _ = focus.focus(index);
            }
            None
        }
        ItemInteraction::FocusChanged(_, false) => None,
    }
}

impl<K: Clone + PartialEq + ToString + 'static> StatefulWidget for BreadcrumbsWidget<K> {
    type State = BreadcrumbsWidgetState<K>;

    fn create_state(self) -> Self::State {
        let model = Breadcrumbs::new(self.items);
        let (focus, nodes) = match model.as_ref() {
            Ok(model) => (
                focus_model(
                    breadcrumb_tabs(model),
                    model.current_index(),
                    Orientation::Horizontal,
                ),
                model.items().iter().map(|_| FocusNode::new()).collect(),
            ),
            Err(error) => (Err(*error), Vec::new()),
        };
        BreadcrumbsWidgetState {
            model,
            focus,
            nodes,
            on_changed: self.on_changed,
            updater: StateUpdater::empty(),
        }
    }
}

impl<K: Clone + PartialEq + ToString + 'static> Widget for BreadcrumbsWidget<K> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "BreadcrumbsWidget", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "BreadcrumbsWidget"
    }
}

impl<K: Clone + PartialEq + ToString + 'static> PortableWidget for BreadcrumbsWidget<K> {}

impl<K: Clone + PartialEq + ToString + 'static> State<BreadcrumbsWidget<K>>
    for BreadcrumbsWidgetState<K>
{
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self)
    where
        Self: Sized,
    {
        let current_key = self
            .model
            .as_ref()
            .ok()
            .and_then(|model| model.items().get(model.current_index()))
            .map(|item| item.key().clone());
        let focused_key = self
            .focus
            .as_ref()
            .ok()
            .and_then(|focus| focus.focused_key().cloned());
        let old_keys = self
            .model
            .as_ref()
            .ok()
            .map(|model| {
                model
                    .items()
                    .iter()
                    .map(|item| item.key().clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let old_nodes = std::mem::replace(&mut self.nodes, Vec::new());
        self.model = new.model;
        self.focus = new.focus;
        self.nodes = self
            .model
            .as_ref()
            .ok()
            .map(|model| {
                model
                    .items()
                    .iter()
                    .enumerate()
                    .map(|(_, item)| {
                        old_keys
                            .iter()
                            .position(|old| old == item.key())
                            .and_then(|index| old_nodes.get(index).cloned())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(key) = current_key.as_ref()
            && let Ok(model) = self.model.as_mut()
            && let Some(index) = model.items().iter().position(|item| item.key() == key)
        {
            let _ = model.activate(index);
            if let Ok(focus) = self.focus.as_mut() {
                let _ = focus.select(index);
            }
        }
        if let Some(key) = focused_key.as_ref()
            && let Ok(focus) = self.focus.as_mut()
        {
            let _ = focus.focus_key(key);
        }
        self.on_changed = new.on_changed;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let Some(model) = self.model.as_ref().ok() else {
            return ErrorWidget::new(
                self.model
                    .as_ref()
                    .err()
                    .expect("breadcrumb model error should be present")
                    .to_string(),
            )
            .boxed();
        };
        let tabs = breadcrumb_tabs(model);
        let (selected, focused) = self
            .focus
            .as_ref()
            .map(|focus| (Some(model.current_index()), Some(focus.focused_index())))
            .unwrap_or((Some(model.current_index()), None));
        let handler = {
            let updater = self.updater;
            Rc::new(move |interaction| {
                updater.set_state(move |state| {
                    if let Some(key) = apply_breadcrumb_interaction(state, interaction) {
                        if let Some(callback) = state.on_changed.as_ref() {
                            callback(key);
                        }
                    }
                });
            })
        };
        build_selection_surface(
            Orientation::Horizontal,
            build_tab_items(
                &tabs,
                0..tabs.len(),
                selected,
                focused,
                &self.nodes,
                handler,
                "breadcrumbs",
                navigation_palette(ctx),
            ),
        )
    }
}

/// A keyboard-accessible ordered stepper.
pub struct StepperWidget<K = String> {
    steps: Vec<Step<K>>,
    orientation: Orientation,
    on_changed: Option<NavigationCallback<K>>,
}

impl<K> StepperWidget<K> {
    /// Creates an empty horizontal stepper builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            orientation: Orientation::Horizontal,
            on_changed: None,
        }
    }

    /// Creates a stepper from ordered model steps.
    #[inline]
    pub fn from_steps(steps: Vec<Step<K>>) -> Self {
        Self::new().with_steps(steps)
    }

    /// Replaces the ordered step definitions.
    #[inline]
    pub fn with_steps(mut self, steps: Vec<Step<K>>) -> Self {
        self.steps = steps;
        self
    }

    /// Returns the configured steps.
    #[inline]
    pub fn steps(&self) -> &[Step<K>] {
        &self.steps
    }

    /// Sets horizontal or vertical step layout.
    #[inline]
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Registers a callback for the newly active step key.
    #[inline]
    pub fn on_changed(mut self, callback: impl Fn(K) + 'static) -> Self {
        self.on_changed = Some(Rc::new(callback));
        self
    }
}

impl<K> Default for StepperWidget<K> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StepperWidgetState<K> {
    model: Result<Stepper<K>, NavigationError>,
    focus: Result<TabBar<K>, NavigationError>,
    orientation: Orientation,
    nodes: Vec<FocusNode>,
    on_changed: Option<NavigationCallback<K>>,
    updater: StateUpdater<Self>,
}

fn step_marker(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "○",
        StepStatus::Current => "●",
        StepStatus::Complete => "✓",
        StepStatus::Error => "!",
    }
}

fn step_tabs<K: Clone + PartialEq>(model: &Stepper<K>) -> Vec<Tab<K>> {
    model
        .steps()
        .iter()
        .map(|step| {
            Tab::new(
                step.key().clone(),
                format!("{} {}", step_marker(step.status_value()), step.label()),
            )
            .disabled(step.is_disabled())
        })
        .collect()
}

fn apply_stepper_interaction<K: Clone + PartialEq>(
    state: &mut StepperWidgetState<K>,
    interaction: ItemInteraction,
) -> Option<K> {
    match interaction {
        ItemInteraction::Activate(index) => {
            if !state.model.as_mut().ok()?.set_current(index) {
                return None;
            }
            if let Ok(focus) = state.focus.as_mut() {
                let _ = focus.select(index);
            }
            state.nodes.get(index).map(FocusNode::request_focus);
            state
                .model
                .as_ref()
                .ok()?
                .steps()
                .get(index)
                .map(|step| step.key().clone())
        }
        ItemInteraction::Key(key) => {
            let action = state.focus.as_mut().ok()?.handle_key(key);
            match action {
                NavigationAction::FocusChanged(index) => {
                    state.nodes.get(index).map(FocusNode::request_focus);
                    None
                }
                NavigationAction::Activated(index) => {
                    if !state.model.as_mut().ok()?.set_current(index) {
                        return None;
                    }
                    state.nodes.get(index).map(FocusNode::request_focus);
                    state
                        .model
                        .as_ref()
                        .ok()?
                        .steps()
                        .get(index)
                        .map(|step| step.key().clone())
                }
                NavigationAction::Ignored
                | NavigationAction::Disabled(_)
                | NavigationAction::TabbedAway => None,
            }
        }
        ItemInteraction::FocusChanged(index, true) => {
            if let Ok(focus) = state.focus.as_mut() {
                let _ = focus.focus(index);
            }
            None
        }
        ItemInteraction::FocusChanged(_, false) => None,
    }
}

impl<K: Clone + PartialEq + ToString + 'static> StatefulWidget for StepperWidget<K> {
    type State = StepperWidgetState<K>;

    fn create_state(self) -> Self::State {
        let model = Stepper::new(self.steps);
        let (focus, nodes) = match model.as_ref() {
            Ok(model) => (
                focus_model(step_tabs(model), model.current_index(), self.orientation),
                model.steps().iter().map(|_| FocusNode::new()).collect(),
            ),
            Err(error) => (Err(*error), Vec::new()),
        };
        StepperWidgetState {
            model,
            focus,
            orientation: self.orientation,
            nodes,
            on_changed: self.on_changed,
            updater: StateUpdater::empty(),
        }
    }
}

impl<K: Clone + PartialEq + ToString + 'static> Widget for StepperWidget<K> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "StepperWidget", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "StepperWidget"
    }
}

impl<K: Clone + PartialEq + ToString + 'static> PortableWidget for StepperWidget<K> {}

impl<K: Clone + PartialEq + ToString + 'static> State<StepperWidget<K>> for StepperWidgetState<K> {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self)
    where
        Self: Sized,
    {
        let current_key = self
            .model
            .as_ref()
            .ok()
            .and_then(|model| model.steps().get(model.current_index()))
            .map(|step| step.key().clone());
        let focused_key = self
            .focus
            .as_ref()
            .ok()
            .and_then(|focus| focus.focused_key().cloned());
        let old_keys = self
            .model
            .as_ref()
            .ok()
            .map(|model| {
                model
                    .steps()
                    .iter()
                    .map(|step| step.key().clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let old_nodes = std::mem::replace(&mut self.nodes, Vec::new());
        self.model = new.model;
        self.focus = new.focus;
        self.orientation = new.orientation;
        self.nodes = self
            .model
            .as_ref()
            .ok()
            .map(|model| {
                model
                    .steps()
                    .iter()
                    .map(|step| {
                        old_keys
                            .iter()
                            .position(|old| old == step.key())
                            .and_then(|index| old_nodes.get(index).cloned())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(key) = current_key.as_ref()
            && let Ok(model) = self.model.as_mut()
            && let Some(index) = model.steps().iter().position(|step| step.key() == key)
        {
            let _ = model.set_current(index);
            if let Ok(focus) = self.focus.as_mut() {
                let _ = focus.select(index);
            }
        }
        if let Some(key) = focused_key.as_ref()
            && let Ok(focus) = self.focus.as_mut()
        {
            let _ = focus.focus_key(key);
        }
        self.on_changed = new.on_changed;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let Some(model) = self.model.as_ref().ok() else {
            return ErrorWidget::new(
                self.model
                    .as_ref()
                    .err()
                    .expect("stepper model error should be present")
                    .to_string(),
            )
            .boxed();
        };
        let tabs = step_tabs(model);
        let focused = self.focus.as_ref().ok().map(TabBar::focused_index);
        let handler = {
            let updater = self.updater;
            Rc::new(move |interaction| {
                updater.set_state(move |state| {
                    if let Some(key) = apply_stepper_interaction(state, interaction) {
                        if let Some(callback) = state.on_changed.as_ref() {
                            callback(key);
                        }
                    }
                });
            })
        };
        build_selection_surface(
            self.orientation,
            build_tab_items(
                &tabs,
                0..tabs.len(),
                Some(model.current_index()),
                focused,
                &self.nodes,
                handler,
                "stepper",
                navigation_palette(ctx),
            ),
        )
    }
}

/// Converts framework key events into the model's platform-neutral commands.
fn map_navigation_key(key: &NamedKey, modifiers: &Modifiers) -> Option<NavigationKey> {
    match key {
        NamedKey::ArrowLeft => Some(NavigationKey::Left),
        NamedKey::ArrowRight => Some(NavigationKey::Right),
        NamedKey::ArrowUp => Some(NavigationKey::Up),
        NamedKey::ArrowDown => Some(NavigationKey::Down),
        NamedKey::Home => Some(NavigationKey::Home),
        NamedKey::End => Some(NavigationKey::End),
        NamedKey::Enter => Some(NavigationKey::Enter),
        NamedKey::Tab => Some(if modifiers.shift {
            NavigationKey::ShiftTab
        } else {
            NavigationKey::Tab
        }),
        NamedKey::Other(name)
            if name.eq_ignore_ascii_case("space") || name == " " || name == "Space" =>
        {
            Some(NavigationKey::Space)
        }
        _ => None,
    }
}

pub(crate) fn event_navigation_key(event: &ElementEvent) -> Option<NavigationKey> {
    match event {
        ElementEvent::KeyInput {
            key,
            action,
            modifiers,
        } if matches!(action, KeyAction::Pressed | KeyAction::Repeat) => {
            map_navigation_key(key, modifiers)
        }
        ElementEvent::CharInput {
            ch: ' ', action, ..
        } if matches!(action, KeyAction::Pressed | KeyAction::Repeat) => Some(NavigationKey::Space),
        ElementEvent::TextInput { text, action, .. }
            if text == " " && matches!(action, KeyAction::Pressed | KeyAction::Repeat) =>
        {
            Some(NavigationKey::Space)
        }
        _ => None,
    }
}

/// A small retained wrapper that receives keyboard events before its child.
struct NavigationKeyRelay<W = RequiredChild> {
    child: W,
    on_key: Rc<dyn Fn(NavigationKey) -> bool>,
}

impl NavigationKeyRelay {
    #[inline]
    fn new() -> Self {
        Self {
            child: RequiredChild,
            on_key: Rc::new(|_| false),
        }
    }

    #[inline]
    fn on_key(mut self, on_key: impl Fn(NavigationKey) -> bool + 'static) -> Self {
        self.on_key = Rc::new(on_key);
        self
    }

    #[inline]
    fn child<C: Widget>(self, child: C) -> NavigationKeyRelay<C> {
        NavigationKeyRelay {
            child,
            on_key: self.on_key,
        }
    }
}

impl<W: Widget + 'static> PortableWidget for NavigationKeyRelay<W> {}

impl<W: Widget + 'static> Widget for NavigationKeyRelay<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawNavigationKeyRelay {
            child: self.child.to_element(ctx),
            on_key: self.on_key,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "NavigationKeyRelay"
    }
}

struct RawNavigationKeyRelay {
    child: AnyElement,
    on_key: Rc<dyn Fn(NavigationKey) -> bool>,
}

impl VisitorElement for RawNavigationKeyRelay {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "NavigationKeyRelay"
    }
}

impl Rebuildable for RawNavigationKeyRelay {
    fn is_carry_state(&self) -> bool {
        self.child.is_carry_state()
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.child.with_rebuild_context(ctx, callback);
    }
}

impl Drawable for RawNavigationKeyRelay {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }
}

impl LayoutElement for RawNavigationKeyRelay {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl EventElement for RawNavigationKeyRelay {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event_navigation_key(event) {
            Some(key) if (self.on_key)(key) => EventResult::consumed(),
            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
