//! State and interaction models for route-aware navigation widgets.
//!
//! The models in this crate are deliberately independent of painting. They
//! retain tab/page state, make keyboard focus order explicit, and expose a
//! small route binding that an Aimer widget adapter can drive. Keeping those
//! responsibilities separate means a future native or portable renderer can
//! consume the same behavior without duplicating navigation policy.

mod model;
mod route;
mod widgets;

pub use model::*;
pub use route::*;
pub use widgets::*;

/// A named alias for a [`NavigationMenuWidget`] configured as a drawer.
pub type NavigationDrawerWidget<K = String> = NavigationMenuWidget<K>;

/// A named alias for a [`NavigationMenuWidget`] configured as a rail.
pub type NavigationRailWidget<K = String> = NavigationMenuWidget<K>;

/// A named alias for a [`NavigationMenuWidget`] configured for bottom
/// navigation.
pub type BottomNavigationWidget<K = String> = NavigationMenuWidget<K>;

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_widget::Widget;

    #[test]
    fn widget_adapters_keep_the_navigation_models_as_their_small_interface() {
        let tab_bar = TabBarWidget::from_tabs(vec![
            Tab::new("home", "Home"),
            Tab::new("settings", "Settings"),
        ]);
        assert_eq!(tab_bar.tabs().len(), 2);

        let menu = NavigationMenuWidget::from_items(
            NavigationSurface::Rail,
            vec![Tab::new("home", "Home"), Tab::new("settings", "Settings")],
        );
        assert_eq!(menu.surface(), NavigationSurface::Rail);

        let view = TabViewWidget::from_pages(vec![
            (
                Tab::new("home", "Home"),
                aimer_widget::ErrorWidget::new("home").boxed(),
            ),
            (
                Tab::new("settings", "Settings"),
                aimer_widget::ErrorWidget::new("settings").boxed(),
            ),
        ]);
        assert_eq!(view.pages().len(), 2);
    }

    #[test]
    fn widget_runtime_routes_keyboard_focus_and_tab_escape() {
        let mut runtime = crate::widgets::SelectionRuntime::new(
            vec![Tab::new("one", "One"), Tab::new("two", "Two")],
            Orientation::Horizontal,
            true,
        );

        assert_eq!(
            runtime.handle_key(NavigationKey::Right),
            NavigationAction::FocusChanged(1)
        );
        assert_eq!(
            runtime.handle_key(NavigationKey::Tab),
            NavigationAction::TabbedAway
        );
    }

    #[test]
    fn widget_runtime_maps_shift_tab_and_space_without_releasing() {
        use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};

        let shift_tab = ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        };
        let space = ElementEvent::CharInput {
            ch: ' ',
            action: KeyAction::Repeat,
            modifiers: Modifiers::default(),
        };
        assert_eq!(
            crate::widgets::event_navigation_key(&shift_tab),
            Some(NavigationKey::ShiftTab)
        );
        assert_eq!(
            crate::widgets::event_navigation_key(&space),
            Some(NavigationKey::Space)
        );
        assert_eq!(
            crate::widgets::event_navigation_key(&ElementEvent::KeyInput {
                key: NamedKey::Tab,
                action: KeyAction::Released,
                modifiers: Modifiers::default(),
            }),
            None
        );
    }

    #[test]
    fn tab_view_retains_each_page_while_selection_changes() {
        let mut view = TabView::new(vec![
            (Tab::new("home", "Home"), String::from("home state")),
            (
                Tab::new("settings", "Settings"),
                String::from("settings state"),
            ),
        ])
        .expect("valid tabs");

        assert_eq!(view.selected_key(), Some(&"home"));
        assert!(view.select_key(&"settings"));
        assert_eq!(view.selected_page(), Some(&"settings state".to_string()));

        view.page_mut(&"home")
            .expect("home exists")
            .push_str(" updated");
        assert!(view.select_key(&"home"));
        assert_eq!(
            view.selected_page(),
            Some(&"home state updated".to_string())
        );
    }

    #[test]
    fn keyboard_navigation_skips_disabled_items_and_handles_home_end_tab() {
        let mut tabs = TabBar::new(vec![
            Tab::new("one", "One"),
            Tab::new("two", "Two").disabled(true),
            Tab::new("three", "Three"),
        ])
        .expect("valid tabs");

        assert_eq!(
            tabs.handle_key(NavigationKey::Right),
            NavigationAction::FocusChanged(2)
        );
        assert_eq!(
            tabs.handle_key(NavigationKey::Home),
            NavigationAction::FocusChanged(0)
        );
        assert_eq!(
            tabs.handle_key(NavigationKey::End),
            NavigationAction::FocusChanged(2)
        );
        assert_eq!(
            tabs.handle_key(NavigationKey::Tab),
            NavigationAction::TabbedAway
        );
        assert_eq!(
            tabs.handle_key(NavigationKey::ShiftTab),
            NavigationAction::TabbedAway
        );
    }

    #[test]
    fn route_tabs_sync_deep_links_and_push_selected_routes() {
        let mut tabs = RouteTabBar::new(vec![
            RouteTab::new("home", "Home", TestRoute::Home),
            RouteTab::new("settings", "Settings", TestRoute::Settings),
        ])
        .expect("valid route tabs");

        assert_eq!(
            tabs.sync_from_route(&TestRoute::Settings),
            RouteSync::Selected(1)
        );
        assert_eq!(tabs.selected_index(), 1);

        let mut pushed = Vec::new();
        assert_eq!(
            tabs.activate_with_sender(0, |route| pushed.push(route)),
            RouteActivation::Pushed(0)
        );
        assert_eq!(pushed, vec![TestRoute::Home]);
        assert_eq!(tabs.sync_from_route(&TestRoute::Home), RouteSync::Unchanged);
        assert_eq!(
            tabs.sync_from_route(&TestRoute::Missing),
            RouteSync::NoMatch
        );
    }

    #[test]
    fn disabled_route_and_all_disabled_navigation_are_safe() {
        let mut tabs = RouteTabBar::new(vec![
            RouteTab::new("home", "Home", TestRoute::Home).disabled(true),
            RouteTab::new("settings", "Settings", TestRoute::Settings),
        ])
        .expect("one enabled tab remains");
        assert_eq!(tabs.activate(0), RouteActivation::Disabled(0));
        assert_eq!(tabs.activate(99), RouteActivation::InvalidIndex);

        assert_eq!(
            TabBar::new(vec![Tab::new("only", "Only").disabled(true)]),
            Err(NavigationError::AllItemsDisabled)
        );
    }

    #[test]
    fn layout_reports_overflow_without_hiding_disabled_policy() {
        let menu = NavigationMenu::new(
            NavigationSurface::Bottom,
            vec![
                Tab::new("one", "One"),
                Tab::new("two", "Two").disabled(true),
                Tab::new("three", "Three"),
            ],
        )
        .expect("valid menu");

        assert_eq!(
            menu.layout(LayoutConstraints::new(180, 100, OverflowPolicy::Menu))
                .expect("valid constraints"),
            NavigationLayout {
                visible_count: 1,
                hidden_count: 2,
                overflow_control: true,
            }
        );
        assert_eq!(menu.focus_order(), vec![0, 2]);
    }

    #[test]
    fn semantic_items_expose_role_focus_order_and_selected_state() {
        let mut bar = TabBar::new(vec![
            Tab::new("home", "Home"),
            Tab::new("settings", "Settings").disabled(true),
        ])
        .expect("valid tabs");
        bar.select(0);
        let semantics = bar.semantics();

        assert_eq!(semantics[0].role, SemanticRole::Tab);
        assert!(semantics[0].selected);
        assert!(semantics[0].focusable);
        assert!(!semantics[1].focusable);
        assert_eq!(bar.focus_order(), vec![0]);
    }

    #[test]
    fn disabled_route_sync_keeps_the_previous_selection() {
        let mut tabs = RouteTabBar::new(vec![
            RouteTab::new("home", "Home", TestRoute::Home),
            RouteTab::new("settings", "Settings", TestRoute::Settings).disabled(true),
        ])
        .expect("valid route tabs");

        assert_eq!(
            tabs.sync_from_route(&TestRoute::Settings),
            RouteSync::Disabled(1)
        );
        assert_eq!(tabs.selected_index(), 0);
    }

    #[test]
    fn navigation_surface_sets_vertical_orientation_and_stepper_stops_at_bounds() {
        let menu = NavigationMenu::new(
            NavigationSurface::Rail,
            vec![Tab::new("one", "One"), Tab::new("two", "Two")],
        )
        .expect("valid menu");
        assert_eq!(
            menu.selection().navigation_orientation(),
            Orientation::Vertical
        );

        let mut stepper = Stepper::new(vec![
            Step::new("one", "One"),
            Step::new("two", "Two").disabled(true),
            Step::new("three", "Three"),
        ])
        .expect("valid steps");
        assert!(stepper.next());
        assert_eq!(stepper.current_index(), 2);
        assert!(!stepper.next());
        assert!(stepper.previous());
        assert_eq!(stepper.current_index(), 0);
        assert!(!stepper.previous());
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestRoute {
        Home,
        Settings,
        Missing,
    }

    impl aimer_router::Route for TestRoute {
        fn parse(path: &str) -> Option<Self> {
            match path {
                "/" => Some(Self::Home),
                "/settings" => Some(Self::Settings),
                "/missing" => Some(Self::Missing),
                _ => None,
            }
        }

        fn format(&self) -> String {
            match self {
                Self::Home => "/".into(),
                Self::Settings => "/settings".into(),
                Self::Missing => "/missing".into(),
            }
        }
    }
}
