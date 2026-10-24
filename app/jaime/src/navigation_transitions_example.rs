//! Interactive previews of navigation effects and persistent shell outlets.
//!
//! Each preview owns its history. Route metadata can opt out of a host's
//! fallback, and shell child transitions leave the surrounding frame retained.

use std::time::Duration;

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::router::{
    CrossFadeTransition, DynamicTransitions, NavigationAction, NavigationContext,
    NavigationTransition, Navigator, NavigatorController, Outlet, Router, Shell,
    StatefulShell, StatefulShellController, ViewLevelTransitions, ZoomTransition,
};
use aimer::style::*;
use aimer::*;

use crate::theme;

const PREVIEW_DURATION: Duration = Duration::from_millis(450);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Router)]
enum PreviewRoute {
    #[route("/")]
    Home,
    #[route("/details")]
    #[view_level(1)]
    Details,
    #[route("/inspect")]
    #[view_level(2)]
    Inspect,
    #[route("/instant")]
    #[transition(none)]
    Instant,
}

impl PreviewRoute {
    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home · level 0",
            Self::Details => "Details · level 1",
            Self::Inspect => "Inspect · level 2",
            Self::Instant => "Instant · animation disabled",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Home => Color::Rgb(36, 62, 91),
            Self::Details => Color::Rgb(77, 44, 88),
            Self::Inspect => Color::Rgb(27, 78, 67),
            Self::Instant => Color::Rgb(94, 61, 32),
        }
    }
}

#[widget(Stateless)]
struct PreviewPage {
    route: PreviewRoute,
}

impl PreviewPage {
    #[inline]
    fn new() -> Self { Self { route: PreviewRoute::Home } }

    #[inline]
    fn route(mut self, route: PreviewRoute) -> Self { self.route = route; self }
}

impl Router for PreviewRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        PreviewPage::new().route(*self).boxed()
    }
}

impl StatelessWidget for PreviewPage {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let navigator = NavigatorController::<PreviewRoute>::of(ctx);
        let can_pop = navigator.can_pop();
        let reset = navigator.clone();
        let back = navigator.clone();
        let destinations = [
            ("Details", PreviewRoute::Details),
            ("Inspect", PreviewRoute::Inspect),
            ("Instant", PreviewRoute::Instant),
        ].map(|(label, route)| {
            let navigator = navigator.clone();
            action_button(label, route == self.route, move || navigator.push(route), ctx)
        });
        page_surface(self.route.color(), Column::new()
            .gaps(LayoutSpacing::new().bottom(12))
            .children([
                page_title(self.route.title()),
                page_caption("Push a page, then use Back to see the return transition."),
                Row::new().gaps(LayoutSpacing::new().right(8)).children(destinations).boxed(),
                Row::new().gaps(LayoutSpacing::new().right(8)).children([
                    action_button("Back", !can_pop, move || back.pop(), ctx),
                    action_button("Reset", !can_pop, move || reset.clear(), ctx),
                ]).boxed(),
            ]))
    }
}

fn preview_navigator() -> Navigator<PreviewRoute> {
    Navigator::new(PreviewRoute::Home, |route| route.boxed())
}

// Selection happens once per navigation. The resolved built-in effect owns
// frame sampling; the selector never builds a page or runs on each frame.
fn dynamic_transition(duration: Duration) -> impl NavigationTransition<PreviewRoute> {
    DynamicTransitions::new(move |navigation: &NavigationContext<'_, '_, PreviewRoute>| {
        if navigation.action == NavigationAction::Pop {
            CrossFadeTransition::new().duration(duration).resolve(navigation)
        } else {
            ZoomTransition::new().duration(duration).resolve(navigation)
        }
    })
}

#[widget(Router)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellRoute {
    #[shell("/dashboard")]
    Dashboard(ShellChildRoute),
}

#[widget(Router)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellChildRoute {
    #[route("/")]
    Overview,
    #[route("/reports")]
    #[transition(ZoomTransition::new().duration(PREVIEW_DURATION))]
    Reports,
}

impl Router for ShellRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        let Self::Dashboard(child) = *self;
        Shell::new(TransitionShellFrame::new(), move |_| child.boxed())
            .route(child)
            .transition(CrossFadeTransition::new().duration(PREVIEW_DURATION))
            .boxed()
    }
}

impl Router for ShellChildRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        let (title, caption, color) = match self {
            Self::Overview => ("Overview outlet", "Inherits the shell's CrossFadeTransition.", PreviewRoute::Home.color()),
            Self::Reports => ("Reports outlet", "This route overrides the fallback with ZoomTransition.", PreviewRoute::Details.color()),
        };
        page_surface(color, Column::new().gaps(LayoutSpacing::new().bottom(12))
            .children([page_title(title), page_caption(caption)])).boxed()
    }
}

#[widget(Stateless)]
struct TransitionShellFrame;

impl TransitionShellFrame {
    #[inline]
    fn new() -> Self { Self }
}

impl StatelessWidget for TransitionShellFrame {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let navigator = NavigatorController::<ShellRoute>::of(ctx);
        let back = navigator.clone();
        let [overview, reports] = [("Overview", ShellChildRoute::Overview), ("Reports", ShellChildRoute::Reports)]
            .map(|(label, child)| {
                let navigator = navigator.clone();
                action_button(label, false, move || {
                    let next = ShellRoute::Dashboard(child);
                    if navigator.current_route() != next { navigator.push(next); }
                }, ctx)
            });
        Column::new().gaps(LayoutSpacing::new().bottom(10)).children([
            Text::new("Shell frame · stays in place").boxed(),
            Row::new().gaps(LayoutSpacing::new().right(8)).children([
                overview, reports,
                action_button("Shell back", false, move || back.pop(), ctx),
            ]).boxed(),
            Expanded::new().child(Outlet).boxed(),
        ])
    }
}

#[widget(Router)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BranchRoute {
    #[route("/feed/{page}")]
    Feed { page: u32 },
    #[route("/profile/{page}")]
    Profile { page: u32 },
}

#[widget(Stateless)]
struct BranchPage {
    route: BranchRoute,
}

impl BranchPage {
    #[inline]
    fn new() -> Self { Self { route: BranchRoute::Feed { page: 0 } } }

    #[inline]
    fn route(mut self, route: BranchRoute) -> Self { self.route = route; self }
}

impl Router for BranchRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        BranchPage::new().route(*self).boxed()
    }
}

impl StatelessWidget for BranchPage {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let controller = StatefulShellController::<BranchRoute>::of(ctx);
        let branch = controller.active_branch();
        let back = controller.clone();
        let (name, page, next, color) = match self.route {
            BranchRoute::Feed { page } => ("Feed", page, BranchRoute::Feed { page: page.saturating_add(1) }, PreviewRoute::Home.color()),
            BranchRoute::Profile { page } => ("Profile", page, BranchRoute::Profile { page: page.saturating_add(1) }, PreviewRoute::Inspect.color()),
        };
        page_surface(color, Column::new().gaps(LayoutSpacing::new().bottom(12)).children([
            Text::new(format!("{name} · page {page}"))
                .text_style(TextStyle::new().font_size(26).color(Color::WHITE)).boxed(),
            page_caption("Switch branches and return: each branch keeps its own history."),
            Row::new().gaps(LayoutSpacing::new().right(8)).children([
                action_button("Next page", false, move || controller.push_in_branch(branch, next), ctx),
                action_button("Branch back", back.branch_len(branch) <= 1, move || back.pop_in_branch(branch), ctx),
            ]).boxed(),
        ]))
    }
}

#[widget(Stateless)]
struct TransitionBranchFrame;

impl TransitionBranchFrame {
    #[inline]
    fn new() -> Self { Self }
}

impl StatelessWidget for TransitionBranchFrame {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let controller = StatefulShellController::<BranchRoute>::of(ctx);
        let profile = controller.clone();
        Column::new().gaps(LayoutSpacing::new().bottom(10)).children([
            Text::new("Stateful shell frame · separate branch histories").boxed(),
            Row::new().gaps(LayoutSpacing::new().right(8)).children([
                action_button("Feed", false, move || controller.go_branch(0), ctx),
                action_button("Profile", false, move || profile.go_branch(1), ctx),
            ]).boxed(),
            Expanded::new().child(Outlet).boxed(),
        ])
    }
}

fn action_button(label: &'static str, disabled: bool, action: impl Fn() + 'static, ctx: &BuildContext) -> AnyWidget {
    let theme = ThemeData::of(ctx);
    Button::new().disabled(disabled).on_press(action)
        .decoration(BoxDecoration::new().background_color(theme.primary_color).border_radius(8))
        .hover_decoration(BoxDecoration::new().background_color(theme.primary_color.lighten(0.12)).border_radius(8))
        .box_child(Container::new().padding(LayoutSpacing::all(9)).child(
            Text::new(label).text_style(TextStyle::new().font_size(14).color(theme.on_primary_color))))
}

fn page_title(title: &'static str) -> AnyWidget {
    Text::new(title).text_style(TextStyle::new().font_size(26)
        .font_weight(FontWeight::Bold).color(Color::WHITE)).boxed()
}

fn page_caption(caption: &'static str) -> AnyWidget {
    Text::new(caption).wrapped().text_style(TextStyle::new().font_size(14).color(Color::WHITE)).boxed()
}

fn page_surface(color: Color, child: impl Widget) -> impl Widget {
    Container::new().width(Dimension::Percent(100.0)).height(Dimension::Percent(100.0))
        .color(color).padding(LayoutSpacing::all(18)).child(child)
}

fn preview_card(title: &'static str, description: &'static str, preview: impl Widget + 'static, theme: &ThemeData) -> AnyWidget {
    Container::new().padding(LayoutSpacing::all(16))
        .box_decoration(BoxDecoration::new().background_color(theme.surface_color).border_radius(12))
        .child(Column::new().gaps(LayoutSpacing::new().bottom(10)).children([
            Text::new(title).text_style(TextStyle::new().font_size(20)
                .font_weight(FontWeight::Bold).color(theme.on_surface_color)).boxed(),
            Text::new(description).wrapped().text_style(TextStyle::new().font_size(14)
                .color(theme::muted_text(theme))).boxed(),
            Container::new().height(260).child(preview).boxed(),
        ])).boxed()
}

/// Builds six independent previews: four effects, a routed shell, and branches.
///
/// Navigate to Details and Inspect, then Back. Instant demonstrates
/// `#[transition(none)]`; the shell previews keep their frame stationary.
pub fn navigation_transitions_example() -> impl Widget {
    NavigationTransitionsExample::new()
}

#[widget(Stateless)]
/// Transition comparisons with independently configurable preview timing.
pub(crate) struct NavigationTransitionsExample {
    duration: Duration,
}

impl NavigationTransitionsExample {
    /// Creates the page with 450 ms effects.
    #[inline]
    pub(crate) fn new() -> Self { Self { duration: PREVIEW_DURATION } }

    /// Sets timing for the four effect previews. Shell routes use their own timing.
    #[inline]
    pub(crate) fn duration(mut self, duration: Duration) -> Self { self.duration = duration; self }
}

impl StatelessWidget for NavigationTransitionsExample {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::of(ctx);
        let duration = self.duration;
        Scrollable::new().axis(ScrollAxis::Vertical).child(
            Column::new().gaps(LayoutSpacing::new().bottom(18)).children([
                preview_card("CrossFadeTransition", "Both pages blend over 450 ms.",
                    preview_navigator().transition(CrossFadeTransition::new().duration(duration)), &theme),
                preview_card("ZoomTransition", "The incoming page grows from its center; Back reverses the zoom.",
                    preview_navigator().transition(ZoomTransition::new().duration(duration)), &theme),
                preview_card("ViewLevelTransitions", "Home (0), Details (1), and Inspect (2) slide according to their view level.",
                    preview_navigator().transition(ViewLevelTransitions::new().duration(duration)), &theme),
                preview_card("DynamicTransitions", "Push uses zoom; Back uses cross-fade. Selection runs once per navigation.",
                    preview_navigator().transition(dynamic_transition(duration)), &theme),
                preview_card("Shell / Outlet", "Overview inherits cross-fade. Reports declares a route-specific zoom override.",
                    Navigator::new(ShellRoute::Dashboard(ShellChildRoute::Overview), |route| route.boxed())
                        .transition(ZoomTransition::new().duration(PREVIEW_DURATION)), &theme),
                preview_card("StatefulShell / Outlet", "Cross-fade between Feed and Profile, with a separate stack for each branch.",
                    StatefulShell::new(
                        vec![BranchRoute::Feed { page: 0 }, BranchRoute::Profile { page: 0 }],
                        |_| TransitionBranchFrame::new().boxed(), |route| route.boxed(),
                    ).transition(CrossFadeTransition::new().duration(PREVIEW_DURATION)), &theme),
            ]),
        )
    }
}

/// Runs the navigation-transition page with Jaime's shared theme.
pub fn start_navigation_transitions_example() {
    AimerApp::start(theme::provide(navigation_transitions_example()));
}
