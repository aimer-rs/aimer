//! A small route-driven navigation example for Jaime.
//!
//! W17 registers this page in the shared showcase. The example uses the
//! public router API and keeps navigation state in the route controller.

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::navigation::{
    Breadcrumb, BreadcrumbsWidget, NavigationMenuWidget, NavigationSurface, OverflowPolicy,
    RouteTab, RouteTabBarWidget, Step, StepStatus, StepperWidget, Tab, TabBarWidget, TabViewWidget,
};
use aimer::router::{Navigator, NavigatorController, Router};
use aimer::{AnyWidget, BuildContext, Column, StatelessWidget, Text, Widget};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NavigationRoute {
    Home,
    Profile,
    Settings,
}

impl NavigationRoute {
    fn next(self) -> Self {
        match self {
            Self::Home => Self::Profile,
            Self::Profile => Self::Settings,
            Self::Settings => Self::Home,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Profile => "Profile",
            Self::Settings => "Settings",
        }
    }
}

impl aimer::router::Route for NavigationRoute {
    fn parse(path: &str) -> Option<Self> {
        match path {
            "/" => Some(Self::Home),
            "/profile" => Some(Self::Profile),
            "/settings" => Some(Self::Settings),
            _ => None,
        }
    }

    fn format(&self) -> String {
        match self {
            Self::Home => "/".to_owned(),
            Self::Profile => "/profile".to_owned(),
            Self::Settings => "/settings".to_owned(),
        }
    }
}

#[widget(Stateless)]
struct NavigationPage {
    route: NavigationRoute,
}

impl NavigationPage {
    fn new(route: NavigationRoute) -> Self {
        Self { route }
    }
}

impl aimer::StatelessWidget for NavigationPage {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let route = self.route;
        let next = route.next();
        let navigator = NavigatorController::<NavigationRoute>::of(ctx);
        Column::new().children(vec![
            Text::new(format!("Route: {}", route.title())).boxed(),
            Text::new("Route-backed tabs").boxed(),
            RouteTabBarWidget::from_tabs(vec![
                RouteTab::new("home", "Home", NavigationRoute::Home),
                RouteTab::new("profile", "Profile", NavigationRoute::Profile),
                RouteTab::new("settings", "Settings", NavigationRoute::Settings),
            ])
            .boxed(),
            Text::new("Navigation rail").boxed(),
            NavigationMenuWidget::from_items(
                NavigationSurface::Rail,
                vec![
                    Tab::new("overview", "Overview"),
                    Tab::new("activity", "Activity"),
                    Tab::new("reports", "Reports"),
                    Tab::new("billing", "Billing"),
                ],
            )
            .overflow_policy(OverflowPolicy::Menu)
            .item_extent(64)
            .boxed(),
            Text::new("Persistent tab pages").boxed(),
            TabViewWidget::from_pages(vec![
                (
                    Tab::new("summary", "Summary"),
                    Text::new("The summary page stays retained when another tab is selected.")
                        .boxed(),
                ),
                (
                    Tab::new("details", "Details"),
                    Text::new("The details page uses the same retained-child seam.").boxed(),
                ),
            ])
            .boxed(),
            BreadcrumbsWidget::from_items(vec![
                Breadcrumb::new("home", "Home"),
                Breadcrumb::new("section", "Section"),
                Breadcrumb::new("current", "Current").current(true),
            ])
            .boxed(),
            StepperWidget::from_steps(vec![
                Step::new("one", "Choose").status(StepStatus::Complete),
                Step::new("two", "Configure").status(StepStatus::Current),
                Step::new("three", "Finish"),
            ])
            .boxed(),
            TabBarWidget::from_tabs(vec![
                Tab::new("local-home", "Local Home"),
                Tab::new("local-settings", "Local Settings"),
            ])
            .boxed(),
            Button::new()
                .on_press(move || navigator.push(next))
                .child(Text::new(format!("Go to {}", next.title())))
                .boxed(),
        ])
    }
}

impl Router for NavigationRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        NavigationPage::new(*self).boxed()
    }
}

fn build_route(route: NavigationRoute) -> AnyWidget {
    NavigationPage::new(route).boxed()
}

/// Builds a route-driven navigation page without starting the application.
pub fn navigation_example() -> impl Widget {
    Navigator::new(NavigationRoute::Home, build_route)
}

/// Starts the standalone navigation example.
pub fn start_navigation_example() {
    aimer::AimerApp::start(navigation_example());
}
