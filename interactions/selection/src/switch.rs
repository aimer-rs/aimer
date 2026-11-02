use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, PortableWidget, State, StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::binary::{Binary, ChangeCallback, VisualBuilder};
use crate::common::{Common, common_setters};
use crate::indicator::Mark;
use crate::semantics::{SelectionSemantics, SemanticRole};
use crate::visual::ToggleVisualState;

/// A controlled on/off switch.
///
/// A switch never changes its own value: a tap or an activation key calls
/// [`on_change`](Self::on_change) with the opposite value, and the owner
/// rebuilds the switch with [`value`](Self::value).
///
/// # Examples
///
/// ```
/// use aimer_selection::Switch;
///
/// let switch = Switch::new()
///     .value(false)
///     .label("Notifications")
///     .on_change(|on| assert!(on));
/// ```
pub struct Switch {
    value: bool,
    common: Common,
    on_change: Option<ChangeCallback<bool>>,
    builder: Option<VisualBuilder<bool>>,
}

impl Default for Switch {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Switch {
    /// Creates an off switch with no callback or label.
    #[inline]
    pub fn new() -> Self {
        Self {
            value: false,
            common: Common::default(),
            on_change: None,
            builder: None,
        }
    }

    /// Sets the caller-owned value.
    #[inline]
    pub fn value(mut self, value: bool) -> Self {
        self.value = value;
        self
    }

    /// Registers the callback that receives proposed values.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(bool) + 'static,
    {
        self.on_change = Some(Rc::new(callback));
        self
    }

    common_setters!();

    /// Replaces the default track and label with a composition of your own.
    ///
    /// The closure runs on every rebuild and receives the value, label, and
    /// interaction flags. Pointer, keyboard, focus, disabled, and callback
    /// behavior stay with the switch.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_selection::Switch;
    /// use aimer_text::Text;
    ///
    /// let switch = Switch::new().builder(|state| Text::new(if state.is_on() { "On" } else { "Off" }));
    /// ```
    #[inline]
    pub fn builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(ToggleVisualState<'a>) -> W + 'static,
        W: Widget + 'static,
    {
        self.builder = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Returns a snapshot of the switch for assistive technology.
    pub fn semantics(&self) -> SelectionSemantics {
        SelectionSemantics::new(SemanticRole::Switch, &self.common).checked(self.value.into())
    }
}

type SwitchState = Binary<bool, bool>;

impl StatefulWidget for Switch {
    type State = SwitchState;

    fn create_state(self) -> Self::State {
        Binary::new(
            self.value,
            Mark::Switch { on: self.value },
            Some(!self.value),
            self.on_change,
            self.common,
            self.builder,
        )
    }
}

impl State<Switch> for SwitchState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.init(updater);
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.adopt(new);
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        self.render(ctx)
    }
}

impl Widget for Switch {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Switch", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Switch"
    }
}

impl PortableWidget for Switch {}
