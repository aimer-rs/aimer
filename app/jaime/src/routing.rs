#![allow(clippy::items_after_test_module)]

#[macro_use]
pub mod home;
#[macro_use]
pub mod profile;
#[macro_use]
pub mod setting;

use aimer::macros::Router;
use aimer::router::{CrossFadeTransition, DynamicTransitions, NavigationContext,
    NavigationTransition, Navigator, Outlet, Router, Shell, StatefulShell,
    ViewLevelTransitions, ZoomTransition};
use aimer::*;

use crate::routing::home::HomeWidget;
use crate::routing::profile::ProfilePage;
use crate::routing::setting::SettingPage;

#[derive(Router)]
#[derive(Clone, Debug, PartialEq)]
pub enum AppRouting {
    #[route("/")]
    #[transition(none)]
    Home,
    #[route("/profile/{name}", name = "profile")]
    #[transition(ZoomTransition::new())]
    #[view_level(2)]
    Profile { name: String },
    #[route("/search?q={q}&page={page}", name = "search")]
    #[transition(CrossFadeTransition::new())]
    #[view_level(1)]
    Search { q: String, page: u32 },
    #[route("/settings")]
    #[transition(ViewLevelTransitions::new())]
    #[view_level(1)]
    Settings,
    #[route("/login", name = "login")]
    Login,
    #[route("/admin", name = "admin")]
    #[redirect(guard = "admin_guard")]
    Admin,
    #[shell("/dashboard", name = "dashboard")]
    Dashboard(DashRoute),
}

/// Child routes rendered inside the dashboard shell's `Outlet`.
#[derive(Router)]
#[derive(Clone, Debug, PartialEq)]
pub enum DashRoute {
    #[route("/")]
    Overview,
    #[route("/reports")]
    #[view_level(1)]
    Reports,
}

impl Router for DashRoute {
    fn build(&self, _: &BuildContext) -> AnyWidget {
        match self {
            DashRoute::Overview => HomeWidget {}.boxed(),
            DashRoute::Reports => HomeWidget {}.boxed(),
        }
    }
}

/// Per-branch routes for the tabbed stateful shell. Each variant is one tab
/// that keeps its own independent navigation history.
#[derive(Router)]
#[derive(Clone, Debug, PartialEq)]
pub enum TabRoute {
    #[route("/feed")]
    Feed,
    #[route("/notifications")]
    Notifications,
    #[route("/profile")]
    Profile,
}

impl Router for TabRoute {
    fn build(&self, _: &BuildContext) -> AnyWidget {
        match self {
            TabRoute::Feed => HomeWidget {}.boxed(),
            TabRoute::Notifications => HomeWidget {}.boxed(),
            TabRoute::Profile => HomeWidget {}.boxed(),
        }
    }
}

/// Persistent tab-shell frame: a layout containing the `Outlet` where the
/// active branch's top route renders. A real app would add a bottom nav bar
/// here whose buttons call
/// `StatefulShellController::<TabRoute>::of(ctx).go_branch(i)`.
fn tab_frame(_: &BuildContext) -> AnyWidget {
    Container::new()
        .child(Outlet)
        .boxed()
}

/// Builds the widget for a given tab route (each `TabRoute` is itself a
/// widget).
fn tab_child(route: TabRoute) -> AnyWidget {
    route.boxed()
}

/// Pure redirect decision for the guarded `/admin` route: unauthenticated users
/// are sent to `/login`. Extracted from `admin_guard` so it can be unit tested
/// without a live `BuildContext`.
fn admin_redirect_decision(authenticated: bool) -> Option<AppRouting> {
    if authenticated {
        None
    } else {
        Some(AppRouting::Login)
    }
}

/// Guard hook wired into the generated `Route::redirect` for
/// `AppRouting::Admin`. In a real app this would read auth state from the
/// context.
fn admin_guard(_route: &AppRouting, _ctx: &BuildContext) -> Option<AppRouting> {
    admin_redirect_decision(false)
}

impl Router for AppRouting {
    fn build(&self, _: &BuildContext) -> AnyWidget {
        match self {
            AppRouting::Home => HomeWidget {}.boxed(),
            AppRouting::Settings => SettingPage {}.boxed(),
            AppRouting::Profile { name } => ProfilePage::new(name.clone()).boxed(),
            AppRouting::Search { .. } => HomeWidget {}.boxed(),
            AppRouting::Login => HomeWidget {}.boxed(),
            AppRouting::Admin => HomeWidget {}.boxed(),
            AppRouting::Dashboard(child) => {
                let child = child.clone();
                let active = child.clone();
                // Persistent shell frame: a Container wrapping the Outlet where
                // the active dashboard child route renders.
                Shell::new(Container::new().child(Outlet), move |_| {
                    child
                        .clone()
                        .boxed()
                })
                .route(active)
                .transition(DynamicTransitions::new(|navigation: &NavigationContext<'_, '_, DashRoute>| {
                    if matches!(navigation.to, DashRoute::Reports) {
                        ZoomTransition::new().resolve(navigation)
                    } else {
                        CrossFadeTransition::new().resolve(navigation)
                    }
                }))
                .boxed()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use aimer::router::Route;

    use super::*;

    #[test]
    fn round_trip_unit_and_named_path() {
        assert_eq!(AppRouting::Home.format(), "/");
        assert_eq!(AppRouting::parse("/"), Some(AppRouting::Home));

        let profile = AppRouting::Profile {
            name: "alice".to_string(),
        };
        assert_eq!(profile.format(), "/profile/alice");
        assert_eq!(AppRouting::parse("/profile/alice"), Some(profile));

        assert_eq!(AppRouting::Settings.format(), "/settings");
        assert_eq!(AppRouting::parse("/settings"), Some(AppRouting::Settings));
    }

    #[test]
    fn round_trip_query_params() {
        let search = AppRouting::Search {
            q: "foo".to_string(),
            page: 2,
        };
        assert_eq!(search.format(), "/search?page=2&q=foo");
        assert_eq!(AppRouting::parse("/search?q=foo&page=2"), Some(search));
    }

    #[test]
    fn unknown_path_returns_none() {
        assert_eq!(AppRouting::parse("/does/not/exist"), None);
    }

    #[test]
    fn admin_guard_redirects_when_unauthenticated() {
        assert_eq!(admin_redirect_decision(false), Some(AppRouting::Login));
        assert_eq!(admin_redirect_decision(true), None);
    }

    #[test]
    fn nested_shell_round_trip() {
        // Child "home" (`/`) collapses to just the shell prefix.
        let overview = AppRouting::Dashboard(DashRoute::Overview);
        assert_eq!(overview.format(), "/dashboard");
        assert_eq!(
            AppRouting::parse("/dashboard"),
            Some(AppRouting::Dashboard(DashRoute::Overview))
        );

        // Nested child appends under the shell prefix.
        let reports = AppRouting::Dashboard(DashRoute::Reports);
        assert_eq!(reports.format(), "/dashboard/reports");
        assert_eq!(
            AppRouting::parse("/dashboard/reports"),
            Some(AppRouting::Dashboard(DashRoute::Reports))
        );

        // Unknown nested child does not match.
        assert_eq!(AppRouting::parse("/dashboard/unknown"), None);
    }

    #[test]
    fn name_reports_declared_names() {
        assert_eq!(AppRouting::Home.name(), None);
        assert_eq!(
            AppRouting::Profile {
                name: "x".to_string()
            }
            .name(),
            Some("profile")
        );
        assert_eq!(
            AppRouting::Search {
                q: "x".to_string(),
                page: 1
            }
            .name(),
            Some("search")
        );
    }

    #[test]
    fn resolve_named_builds_route_from_params() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), "bob".to_string());
        assert_eq!(
            AppRouting::resolve_named("profile", &params),
            Some(AppRouting::Profile {
                name: "bob".to_string()
            })
        );

        let mut params = HashMap::new();
        params.insert("q".to_string(), "rust".to_string());
        params.insert("page".to_string(), "3".to_string());
        assert_eq!(
            AppRouting::resolve_named("search", &params),
            Some(AppRouting::Search {
                q: "rust".to_string(),
                page: 3
            })
        );

        assert_eq!(AppRouting::resolve_named("unknown", &params), None);
    }

    #[test]
    fn stateful_shell_starts_each_branch_with_own_stack() {
        let shell = StatefulShell::<TabRoute>::new(
            vec![TabRoute::Feed, TabRoute::Notifications, TabRoute::Profile],
            tab_frame,
            tab_child,
        );
        assert_eq!(shell.active, 0);
        assert_eq!(shell.branches.len(), 3);
        assert_eq!(shell.branches[0], vec![TabRoute::Feed]);
        assert_eq!(shell.branches[1], vec![TabRoute::Notifications]);
        assert_eq!(shell.branches[2], vec![TabRoute::Profile]);
    }
}

pub fn state_router() {
    AimerApp::start(crate::theme::provide(state_router_widget()))
}

/// Builds the generated-route showcase without starting an application.
pub fn state_router_widget() -> impl Widget {
    Navigator::<AppRouting>::new(AppRouting::Home, |route| route.boxed())
        .transition(ViewLevelTransitions::new())
}

/// Launch the tabbed stateful-shell demo: three branches (Feed, Notifications,
/// Profile), each keeping its own independent navigation history.
///
/// ## This is not working yet, I will update it
pub fn tab_shell_app() {
    AimerApp::start(crate::theme::provide(StatefulShell::<TabRoute>::new(
        vec![TabRoute::Feed, TabRoute::Notifications, TabRoute::Profile],
        tab_frame,
        tab_child,
    ).transition(CrossFadeTransition::new())))
}
