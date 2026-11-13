//! The `stateful_v1` implementation as a consistency-suite [`Backend`].
//!
//! The suite describes a stateful widget as a neutral [`Spec`]; this adapter
//! maps one onto the API `stateful_v1` actually has (`StatefulWidget`, `State`,
//! `create_state`, `init_state(updater)`, `adopt_config_from(new)`), which is
//! the only place that mapping lives.

use std::any::Any;

use aimer_widget_core::base::BuildContext;
use aimer_widget_core::key::Key;
use aimer_widget_core::{AnyElement, Widget};

use super::stateful::{KeyedStateScope, State, StateUpdater, StatefulElement, StatefulWidget};
use crate::consistency::{Backend, Handle, Ops, Spec};

/// The backend: the copied stateful machinery in this module tree.
pub struct V1;

/// The widget half: just the props.
pub struct Adapter<Sp: Spec> {
    props: Sp::Props,
}

/// The state half: the spec's state, plus what the old API makes the state hold.
///
/// `props` is kept because `adopt_config_from` hands over a whole new *state*,
/// and the spec wants the props it was built from. `updater` is the field the
/// redesign removes; it is stored here so a spec never has to.
pub struct AdapterState<Sp: Spec> {
    state: Sp::State,
    props: Sp::Props,
    updater: StateUpdater<Self>,
}

impl<Sp: Spec> StatefulWidget for Adapter<Sp> {
    type State = AdapterState<Sp>;

    fn create_state(self) -> Self::State {
        AdapterState {
            state: Sp::create(&self.props),
            props: self.props,
            updater: StateUpdater::empty(),
        }
    }
}

impl<Sp: Spec> State<Adapter<Sp>> for AdapterState<Sp> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
        Sp::init(&mut self.state);
    }

    fn adopt_config_from(&mut self, new: Self) {
        Sp::adopt(&mut self.state, &new.props);
        self.props = new.props;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        Sp::build::<V1>(&self.state, Handle::new(V1Ops::<Sp>(self.updater)), ctx)
    }
}

/// The native updater, seen through the suite's [`Handle`].
struct V1Ops<Sp: Spec>(StateUpdater<AdapterState<Sp>>);

impl<Sp: Spec> Ops<Sp::State> for V1Ops<Sp> {
    fn set_state(&self, mutation: Box<dyn FnOnce(&mut Sp::State)>) {
        self.0.set_state(move |adapter| mutation(&mut adapter.state));
    }

    fn try_set_state(&self, mutation: Box<dyn FnOnce(&mut Sp::State)>) -> bool {
        self.0.try_set_state(move |adapter| mutation(&mut adapter.state)).is_some()
    }

    fn read(&self, read: &mut dyn FnMut(&Sp::State)) {
        self.0.read(|adapter| read(&adapter.state));
    }

    fn try_read(&self, read: &mut dyn FnMut(&Sp::State)) -> bool {
        self.0.try_read(|adapter| read(&adapter.state)).is_some()
    }

    fn has_state(&self) -> bool {
        self.0.has_state()
    }
}

impl Backend for V1 {
    const NAME: &'static str = "stateful_v1";

    fn key_scope() -> Box<dyn Any> {
        Box::new(KeyedStateScope::enter())
    }

    fn element<Sp: Spec>(
        props: Sp::Props,
        ctx: &BuildContext,
        name: &'static str,
        key: Option<Key>,
    ) -> AnyElement {
        StatefulElement::from_widget(Adapter::<Sp> { props }, ctx, name, key)
    }
}

#[cfg(test)]
mod suite {
    crate::consistency::consistency_suite!(super::V1);
}
