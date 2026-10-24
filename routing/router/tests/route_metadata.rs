extern crate self as aimer;

pub mod router { pub use aimer_router::*; }
pub mod widget { pub use aimer_widget::*; }

use aimer_router::{CrossFadeTransition, Route, Router, TransitionChoice, ZoomTransition};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyWidget, ErrorWidget, Widget};

#[derive(Clone, aimer_macro::Router)]
enum Derived {
    #[route("/")]
    #[transition(none)]
    Home,
    #[route("/detail/{id}")]
    #[transition(ZoomTransition::new())]
    #[view_level(2)]
    Detail { id: u32 },
    #[shell("/dashboard")]
    #[transition(CrossFadeTransition::new())]
    #[view_level(1)]
    Dashboard(Child),
}

#[aimer_widget::widget(Router)]
#[derive(Clone)]
enum Child {
    #[route("/")]
    Index,
    #[route("/reports")]
    #[transition(CrossFadeTransition::new())]
    #[view_level(3)]
    Reports,
}

impl Router for Derived {
    fn build(&self, _: &BuildContext) -> AnyWidget { ErrorWidget::new("derived route").boxed() }
}
impl Router for Child {
    fn build(&self, _: &BuildContext) -> AnyWidget { ErrorWidget::new("child route").boxed() }
}

#[test]
fn router_macros_compile_and_expose_transition_level_and_shell_metadata() {
    assert!(matches!(Derived::Home.navigation_transition(), TransitionChoice::Disabled));
    let detail = Derived::Detail { id: 42 };
    assert!(matches!(detail.navigation_transition(), TransitionChoice::Custom(_)));
    assert_eq!(detail.view_level(), 2);
    assert_eq!(detail.format(), "/detail/42");
    assert!(matches!(Child::Index.navigation_transition(), TransitionChoice::Inherit));
    assert!(matches!(Child::Reports.navigation_transition(), TransitionChoice::Custom(_)));
    assert_eq!(Child::Reports.view_level(), 3);
    assert_eq!(Child::Index.view_level(), 0);
    assert_eq!(Derived::Dashboard(Child::Index).shell_key(), Derived::Dashboard(Child::Reports).shell_key());
    assert!(Derived::Dashboard(Child::Index).shell_key().unwrap().ends_with("::Derived::Dashboard"));
    assert_eq!(Derived::Home.shell_key(), None);
}
