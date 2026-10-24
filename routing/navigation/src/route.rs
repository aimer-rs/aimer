use aimer_router::{NavigatorController, Route};

use crate::{NavigationAction, NavigationError, NavigationSemantics, Tab, TabBar};

/// The result of synchronizing a route-driven tab bar with a route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteSync {
    /// A matching route changed the selected item.
    Selected(usize),
    /// A matching route was already selected.
    Unchanged,
    /// The route is not represented by this navigation surface.
    NoMatch,
    /// The route is represented, but its navigation item is disabled.
    Disabled(usize),
}

/// The result of trying to activate a route-driven item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteActivation {
    /// The route was selected and sent to the navigator.
    Pushed(usize),
    /// The item is disabled and no route was sent.
    Disabled(usize),
    /// No item exists at the requested index.
    InvalidIndex,
}

/// A labeled tab associated with one router route.
pub struct RouteTab<R: Route> {
    tab: Tab<String>,
    route: R,
}

impl<R: Route> RouteTab<R> {
    /// Creates an enabled route tab.
    pub fn new(key: impl Into<String>, label: impl Into<String>, route: R) -> Self {
        Self {
            tab: Tab::new(key.into(), label),
            route,
        }
    }

    /// Marks the route tab as disabled.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.tab = self.tab.disabled(disabled);
        self
    }

    /// Returns the stable tab key.
    #[inline]
    pub fn key(&self) -> &str {
        self.tab.key()
    }

    /// Returns the visible tab label.
    #[inline]
    pub fn label(&self) -> &str {
        self.tab.label()
    }

    /// Returns the associated route.
    #[inline]
    pub fn route(&self) -> &R {
        &self.route
    }

    /// Returns whether this route tab is disabled.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.tab.is_disabled()
    }
}

/// A tab selection model synchronized with an [`aimer_router`] navigator.
pub struct RouteTabBar<R: Route> {
    tabs: Vec<RouteTab<R>>,
    bar: TabBar<String>,
}

impl<R: Route> RouteTabBar<R> {
    /// Creates a route tab bar and rejects duplicate keys.
    pub fn new(tabs: Vec<RouteTab<R>>) -> Result<Self, NavigationError> {
        let bar = TabBar::new(
            tabs.iter()
                .map(|tab| Tab::new(tab.key().to_owned(), tab.label()).disabled(tab.is_disabled()))
                .collect(),
        )?;
        Ok(Self { tabs, bar })
    }

    /// Returns route tabs in their stable order.
    #[inline]
    pub fn tabs(&self) -> &[RouteTab<R>] {
        &self.tabs
    }

    /// Returns the current selected index.
    #[inline]
    pub fn selected_index(&self) -> usize {
        self.bar.selected_index()
    }

    /// Returns the index currently targeted by keyboard focus.
    #[inline]
    pub fn focused_index(&self) -> usize {
        self.bar.focused_index()
    }

    /// Returns the selected route tab.
    #[inline]
    pub fn selected(&self) -> &RouteTab<R> {
        &self.tabs[self.selected_index()]
    }

    /// Selects a route tab without pushing it. This is useful when handling a
    /// browser back/forward event in a host that already changed the route.
    #[inline]
    pub fn select(&mut self, index: usize) -> NavigationAction {
        self.bar.select(index)
    }

    /// Moves focus without changing the route selected by the bar.
    #[inline]
    pub fn focus(&mut self, index: usize) -> NavigationAction {
        self.bar.focus(index)
    }

    /// Activates an item locally without sending a route to a host.
    pub fn activate(&mut self, index: usize) -> RouteActivation {
        let Some(tab) = self.tabs.get(index) else {
            return RouteActivation::InvalidIndex;
        };
        if tab.is_disabled() {
            return RouteActivation::Disabled(index);
        }
        let _ = self.bar.select(index);
        RouteActivation::Pushed(index)
    }

    /// Returns a route for an item index without changing selection.
    #[inline]
    pub fn route_for(&self, index: usize) -> Option<&R> {
        self.tabs.get(index).map(RouteTab::route)
    }

    /// Synchronizes selection from a deep link, browser back/forward event, or
    /// any other route change by comparing the router's canonical format.
    pub fn sync_from_route(&mut self, route: &R) -> RouteSync {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.route().format() == route.format())
        else {
            return RouteSync::NoMatch;
        };
        if self.tabs[index].is_disabled() {
            return RouteSync::Disabled(index);
        }
        if self.selected_index() == index {
            RouteSync::Unchanged
        } else {
            let _ = self.bar.select(index);
            RouteSync::Selected(index)
        }
    }

    /// Synchronizes selection from the current route in a live navigator.
    pub fn sync_from_navigator(&mut self, navigator: &NavigatorController<R>) -> RouteSync {
        let route = navigator.current_route();
        self.sync_from_route(&route)
    }

    /// Activates an item and sends its cloned route to an arbitrary host. The
    /// callback seam keeps this model testable without constructing a live UI.
    pub fn activate_with_sender(&mut self, index: usize, push: impl FnOnce(R)) -> RouteActivation {
        let Some(tab) = self.tabs.get(index) else {
            return RouteActivation::InvalidIndex;
        };
        if tab.is_disabled() {
            return RouteActivation::Disabled(index);
        }
        let route = tab.route().clone();
        let _ = self.bar.select(index);
        push(route);
        RouteActivation::Pushed(index)
    }

    /// Activates an item by pushing it through Aimer's current navigator.
    pub fn activate_with(
        &mut self,
        index: usize,
        navigator: &NavigatorController<R>,
    ) -> RouteActivation {
        self.activate_with_sender(index, |route| navigator.push(route))
    }

    /// Handles keyboard focus and activation without pushing the selected
    /// route. A host can use [`Self::activate_with_sender`] for the activation
    /// callback once it receives [`NavigationAction::Activated`].
    #[inline]
    pub fn handle_key(&mut self, key: crate::NavigationKey) -> NavigationAction {
        self.bar.handle_key(key)
    }

    /// Returns route-aware semantic items.
    #[inline]
    pub fn semantics(&self) -> Vec<NavigationSemantics> {
        self.bar.semantics()
    }
}
