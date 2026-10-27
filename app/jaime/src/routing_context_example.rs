//! Jaime's route-child context composition example.
//!
//! The provider deliberately wraps the `Navigator`, rather than a route
//! branch. This makes the same app-wide value available to direct route pages
//! and to pages rendered through a persistent `Shell`/`Outlet` pair.

use std::time::Duration;

use aimer::animation::{AnimatedSwitcher, curves};
use aimer::macros::Router;
use aimer::router::{Navigator, Outlet, Router, Shell};
use aimer::style::{AnimatedTheme, Theme, ThemeData};
use aimer::{
    AimerApp, AnyWidget, BuildContext, Container, Provider, ProviderContext, StatelessWidget, Text,
    Widget, router,
};

#[derive(Clone, Copy)]
struct RoutingContextValue;

#[derive(Router)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextRoute {
    #[route("/")]
    Home,
    #[route("/details")]
    Details,
    #[shell("/dashboard")]
    Dashboard(ContextChildRoute),
}

#[derive(Router)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextChildRoute {
    #[route("/")]
    Overview,
    #[route("/reports")]
    Reports,
}

#[derive(StatelessWidget)]
struct ContextRoutePageWidget {
    title: &'static str,
}

impl ContextRoutePageWidget {
    fn new(title: &'static str) -> Self {
        Self { title }
    }
}

impl Router for ContextRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        match self {
            Self::Home => ContextRoutePageWidget::new("Direct route: home").boxed(),
            Self::Details => ContextRoutePageWidget::new("Direct route: details").boxed(),
            Self::Dashboard(child) => {
                let child = *child;
                Shell::new(Container::new().child(Outlet), move |_| child.boxed()).boxed()
            }
        }
    }
}

impl Router for ContextChildRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        let title = match self {
            Self::Overview => "Shell route: overview",
            Self::Reports => "Shell route: reports",
        };
        ContextRoutePageWidget::new(title).boxed()
    }
}

impl aimer::StatelessWidget for ContextRoutePageWidget {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::of(ctx);
        let scope = if ProviderContext::try_read::<RoutingContextValue>(ctx).is_some() {
            "provider scope: available"
        } else {
            "provider scope: missing"
        };
        Container::new()
            .color(theme.background_color)
            .child(
                AnimatedSwitcher::new(
                    Duration::from_millis(180),
                    curves::EASE_IN_OUT,
                    Text::new(format!("{} — {}", self.title, scope)),
                )
                .child_key(self.title),
            )
    }
}

/// Builds the context-composition example without starting an application.
pub fn routing_context_example() -> impl Widget {
    AnimatedTheme::new()
        .data(ThemeData::light())
        .child(
            Provider::new()
                .create(|| RoutingContextValue)
                .child(Navigator::new(ContextRoute::Home, |route| route.boxed())),
        )
}

/// Runs the route-child context example with Jaime's application theme.
pub fn start_routing_context_example() {
    AimerApp::start(routing_context_example());
}

/// Deliberately invalid example used to demonstrate the source-located missing
/// shell diagnostic: an `Outlet` must have a `Shell` ancestor.
pub fn missing_shell_diagnostic_example() -> impl Widget {
    Container::new().child(Outlet)
}
