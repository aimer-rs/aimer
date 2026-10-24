//! Elements that only hand a rebuild mark on say so.
//!
//! A mark that reaches an element the dirty-path index knows nothing about is
//! treated as coming from an unknown producer, and the index is thrown away: the
//! next pass walks the whole window. Placing a retained child again marks it, a
//! stateful widget that rebuilds every frame (a focus ring easing in, a caret
//! blinking) does that every frame, and so an animated builder or a column among
//! its children cost a walk of the page per frame. Both only forward the mark to
//! their children, so they declare it with `forwards_marks_only`.

use super::*;

use std::time::Duration;

use aimer_animation::{AnimatedBuilder, AnimationController, curves};

/// A widget that reports what the element built for `inner` says about marks.
struct Report<W> {
    inner: W,
    forwards: Rc<Cell<Option<bool>>>,
}

impl<W: Widget> Widget for Report<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let element = self.inner.to_element(ctx);
        self.forwards.set(Some(element.forwards_marks_only()));
        element
    }
}

impl<W: Widget> aimer_widget::PortableWidget for Report<W> {}

fn forwards_marks_only(inner: impl Widget + 'static) -> bool {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let forwards = Rc::new(Cell::new(None));
    let mut app = AimerApp::start_headless_with(
        Report { inner, forwards: forwards.clone() },
        HeadlessOptions { size: PhysicalSize::new(100, 100), scale_factor: 1.0 },
    );
    // The widget is built by the first frame.
    app.pump_frames(2);
    forwards.get().expect("the widget was built")
}

#[test]
fn an_animated_builder_only_forwards_marks() {
    let controller = AnimationController::new(Duration::from_millis(200), curves::LINEAR);
    assert!(forwards_marks_only(AnimatedBuilder::new(controller, |_| {
        SizedBox::new().width(Dimension::Px(4.0)).height(Dimension::Px(4.0))
    })));
}

#[test]
fn a_column_only_forwards_marks() {
    assert!(forwards_marks_only(Column::new().children([
        SizedBox::new().width(Dimension::Px(4.0)).height(Dimension::Px(4.0)).boxed(),
    ])));
}

#[test]
fn a_stateful_widget_does_not_claim_to_only_forward_marks() {
    // It is the source of rebuilds the index tracks, so it must stay judged.
    assert!(!forwards_marks_only(TextField::new()));
}
