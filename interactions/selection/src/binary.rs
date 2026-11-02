//! The retained state behind `Checkbox`, `Switch`, and `Radio`.
//!
//! The three controls differ only in the mark they paint and in what a tap
//! proposes, so they share one state. `V` is the controlled value a `builder`
//! sees, and `P` is the payload of `on_change`.

use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyWidget, FocusNode, StateUpdater};

use crate::chrome::{control_shell, labeled_row};
use crate::common::Common;
use crate::indicator::{Indicator, Mark};
use crate::interaction::{Controlled, Handlers};
use crate::keys::Key;
use crate::theme::tokens;
use crate::visual::VisualState;

/// Receives the value a control proposes. Runs synchronously on the UI thread.
pub(crate) type ChangeCallback<P> = Rc<dyn Fn(P)>;

/// Builds a control's content from its visual state.
pub(crate) type VisualBuilder<V> = Rc<dyn for<'a> Fn(VisualState<'a, V>) -> AnyWidget>;

/// Retained state of a checkbox, switch, or radio.
pub struct Binary<V: 'static, P: 'static> {
    value: V,
    mark: Mark,
    /// What a tap proposes; `None` when there is nothing to propose.
    proposal: Option<P>,
    on_change: Option<ChangeCallback<P>>,
    common: Common,
    builder: Option<VisualBuilder<V>>,
    node: FocusNode,
    handlers: Handlers,
}

impl<V: Copy + 'static, P: Clone + 'static> Binary<V, P> {
    pub(crate) fn new(
        value: V,
        mark: Mark,
        proposal: Option<P>,
        on_change: Option<ChangeCallback<P>>,
        mut common: Common,
        builder: Option<VisualBuilder<V>>,
    ) -> Self {
        let node = common.focus_node.take().unwrap_or_else(FocusNode::new);
        Self {
            value,
            mark,
            proposal,
            on_change,
            common,
            builder,
            node,
            // Replaced in `init`, once the state has an updater.
            handlers: Handlers::new(StateUpdater::<Self>::empty()),
        }
    }

    pub(crate) fn init(&mut self, updater: StateUpdater<Self>) {
        self.handlers = Handlers::new(updater);
    }

    pub(crate) fn adopt(&mut self, mut new: Self) {
        self.value = new.value;
        self.mark = new.mark;
        self.proposal = new.proposal;
        self.on_change = new.on_change;
        self.builder = new.builder;
        if let Some(node) = new.common.focus_node.take() {
            self.node = node;
        }
        if new.common.disabled {
            self.handlers.clear_pointer();
        }
        self.common = new.common;
    }

    pub(crate) fn render(&self, ctx: &BuildContext) -> AnyWidget {
        let tokens = tokens(ctx);
        let flags = self.handlers.flags();
        let disabled = self.common.disabled;
        let label = self.common.label.as_deref();
        let error = self.common.error.as_deref();
        let content = match &self.builder {
            Some(builder) => builder(VisualState::new(self.value, label, error, flags, disabled)),
            None => labeled_row(
                &tokens,
                Indicator::new(&tokens, self.mark, flags, disabled, error.is_some()),
                label,
                disabled,
                false,
            ),
        };
        let shell = control_shell(&tokens, flags, disabled, error, content);
        self.handlers.wrap(disabled, &self.node, shell)
    }

    /// Proposes the pending value, if any, to the owner.
    fn propose(&self) {
        if self.common.disabled {
            return;
        }
        if let (Some(on_change), Some(proposal)) = (&self.on_change, &self.proposal) {
            on_change(proposal.clone());
        }
    }
}

impl<V: Copy + 'static, P: Clone + 'static> Controlled for Binary<V, P> {
    fn consumes_key(&self, key: Key) -> bool {
        !self.common.disabled && key.is_activation()
    }

    fn on_key(&mut self, key: Key) {
        if key.is_activation() {
            self.propose();
        }
    }

    fn on_tap(&mut self) {
        self.propose();
    }
}
