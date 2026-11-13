//! Design A: `build` is an associated function that receives a [`StateView`].
//!
//! ```text
//! fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget
//! ```
//!
//! There is no `self` in `build`, and the state never stores its updater. `this`
//! derefs to the state, so fields are read directly (`this.count`), and it owns a
//! `'static` updater for everything that outlives the build: `this.callback(..)`
//! queues a mutation, `this.updater()` is the handle for a handler that must read
//! the state when it runs.
//!
//! The view borrows the state for this one call, so capturing `this` in a
//! `'static` closure does not compile: a handler cannot read a stale reference,
//! it has to go through the updater.

use crate::mock::*;

pub trait State<W: Widget>: Sized + 'static {
    /// Builds the state from the widget. Moves the props in, it does not clone them.
    fn create(widget: W) -> Self;

    /// Optional. Runs once, right after `create` and before the first build.
    /// Mutate `self` directly; there is nothing to queue and no updater yet.
    fn init_state(&mut self) {}

    /// A parent re-emitted the widget. Replaces `adopt_config_from(new: Self)`:
    /// the framework no longer builds a throwaway state on every parent rebuild.
    fn update_widget(&mut self, _widget: W) {}

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget;
}

pub struct StatefulElement<W, S> {
    mount: Mount<W, S>,
}

impl<W: Widget + 'static, S: State<W>> StatefulElement<W, S> {
    pub fn new(widget: W) -> Self {
        Self {
            mount: Mount::new(widget),
        }
    }

    pub fn reconcile(&mut self, widget: W) {
        self.mount.reconcile(widget, S::update_widget);
    }
}

impl<W: Widget + 'static, S: State<W>> Element for StatefulElement<W, S> {
    fn render(&mut self, ctx: &BuildContext) -> Node {
        let slot = self.mount.slot_or_create(S::create, S::init_state);
        slot.apply_pending();
        // The shared borrow lasts only while the widget tree is described. Mutations
        // queued by callbacks run at the next `apply_pending`, never during this build.
        let mut child = {
            let state = slot.state.borrow();
            let view = StateView::new(&*state, StateUpdater::new(slot.clone()));
            S::build(&view, ctx).to_element()
        };
        child.render(ctx)
    }

    fn is_dirty(&self) -> bool {
        self.mount.is_dirty()
    }
}

/// Public widget, private state. This `impl` is what `#[derive(StatefulWidget)]`
/// + `#[state(CounterState)]` would emit: the state is named in a body, never in a signature.
pub struct Counter {
    pub start: i32,
    pub step: i32,
}

struct CounterState {
    count: i32,
    step: i32,
}

impl Widget for Counter {
    fn to_element(self) -> Box<dyn Element> {
        Box::new(StatefulElement::<Self, CounterState>::new(self))
    }
}

impl State<Counter> for CounterState {
    fn create(widget: Counter) -> Self {
        Self {
            count: widget.start,
            step: widget.step,
        }
    }

    fn update_widget(&mut self, widget: Counter) {
        self.step = widget.step;
    }

    fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        Column::new()
            .child(Text::new(format!("count={}", this.count)))
            .child(Button::new(
                "+",
                this.callback(|state| state.count += state.step),
            ))
    }
}

pub struct Loader;

struct LoaderState {
    ready: bool,
    inits: u32,
}

impl Widget for Loader {
    fn to_element(self) -> Box<dyn Element> {
        Box::new(StatefulElement::<Self, LoaderState>::new(self))
    }
}

impl State<Loader> for LoaderState {
    fn create(_widget: Loader) -> Self {
        Self { ready: false, inits: 0 }
    }

    fn init_state(&mut self) {
        self.ready = true;
        self.inits += 1;
    }

    fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        Text::new(format!("ready={} inits={}", this.ready, this.inits))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    fn counter(step: i32) -> StatefulElement<Counter, CounterState> {
        StatefulElement::new(Counter { start: 1, step })
    }

    #[test]
    fn first_build_reads_props_from_the_widget() {
        assert_eq!(counter(2).render(&BuildContext).render(), "count=1 [+]");
    }

    #[test]
    fn press_is_deferred_until_the_next_build() {
        let mut element = counter(2);
        let tree = element.render(&BuildContext);
        assert!(!element.is_dirty());
        assert!(tree.press("+"));
        assert!(element.is_dirty());
        assert_eq!(element.render(&BuildContext).render(), "count=3 [+]");
        assert!(!element.is_dirty());
    }

    #[test]
    fn presses_in_one_frame_cost_one_build() {
        let mut element = counter(1);
        let tree = element.render(&BuildContext);
        tree.press("+");
        tree.press("+");
        assert_eq!(element.render(&BuildContext).render(), "count=3 [+]");
    }

    #[test]
    fn reconcile_keeps_runtime_state_and_adopts_new_props() {
        let mut element = counter(1);
        element.render(&BuildContext).press("+");
        assert_eq!(element.render(&BuildContext).render(), "count=2 [+]");
        element.reconcile(Counter {
            start: 100,
            step: 10,
        });
        assert_eq!(element.render(&BuildContext).render(), "count=2 [+]");
        element.render(&BuildContext).press("+");
        assert_eq!(element.render(&BuildContext).render(), "count=12 [+]");
    }

    #[test]
    fn queued_mutation_sees_the_props_current_when_it_runs() {
        let mut element = counter(1);
        element.render(&BuildContext).press("+");
        element.reconcile(Counter {
            start: 100,
            step: 10,
        });
        assert_eq!(element.render(&BuildContext).render(), "count=11 [+]");
    }

    #[test]
    fn reconcile_before_first_build_replaces_the_widget() {
        let mut element = counter(1);
        element.reconcile(Counter { start: 7, step: 1 });
        assert_eq!(element.render(&BuildContext).render(), "count=7 [+]");
    }

    #[test]
    fn init_state_mutates_self_once_before_the_first_build() {
        let mut element = Loader.to_element();
        assert_eq!(element.render(&BuildContext).render(), "ready=true inits=1");
        assert!(!element.is_dirty());
        assert_eq!(element.render(&BuildContext).render(), "ready=true inits=1");
    }

    /// A handler that must read the state when it runs, not when it was built,
    /// takes the `'static` updater out of the view.
    struct Logger {
        log: Rc<RefCell<Vec<i32>>>,
    }

    struct LoggerState {
        count: i32,
        log: Rc<RefCell<Vec<i32>>>,
    }

    impl Widget for Logger {
        fn to_element(self) -> Box<dyn Element> {
            Box::new(StatefulElement::<Self, LoggerState>::new(self))
        }
    }

    impl State<Logger> for LoggerState {
        fn create(widget: Logger) -> Self {
            Self { count: 0, log: widget.log }
        }

        fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
            let updater = this.updater();
            let log = this.log.clone();
            Column::new()
                .child(Button::new("+", this.callback(|state| state.count += 1)))
                .child(Button::new("log", move || {
                    log.borrow_mut().push(updater.read().count);
                }))
        }
    }

    #[test]
    fn a_handler_reads_the_applied_state_through_the_updater() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut element = StatefulElement::<Logger, LoggerState>::new(Logger { log: log.clone() });
        let tree = element.render(&BuildContext);

        tree.press("+");
        tree.press("log");
        assert_eq!(*log.borrow(), [0], "a queued mutation is not applied until the next build");

        element.render(&BuildContext);
        tree.press("log");
        assert_eq!(*log.borrow(), [0, 1], "the same handler now sees the applied state");
    }
}
