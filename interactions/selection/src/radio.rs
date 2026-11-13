use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, State, StateView, StateUpdater, StatefulElement, Widget};

use crate::binary::{Binary, ChangeCallback, VisualBuilder};
use crate::common::{Common, common_setters};
use crate::indicator::Mark;
use crate::semantics::{SelectionSemantics, SemanticRole};
use crate::visual::ToggleVisualState;

/// One controlled radio button.
///
/// A radio proposes its value only while it is not selected. Selection stays
/// with the owner: [`selected`](Self::selected) is set on rebuild, and
/// [`on_change`](Self::on_change) reports the value to adopt. To manage a whole
/// exclusive set, use [`crate::RadioGroup`].
///
/// # Examples
///
/// ```
/// use aimer_selection::Radio;
///
/// let radio = Radio::new("pro")
///     .selected(false)
///     .label("Pro")
///     .on_change(|plan| assert_eq!(plan, "pro"));
/// ```
pub struct Radio<T> {
    value: T,
    selected: bool,
    common: Common,
    on_change: Option<ChangeCallback<T>>,
    builder: Option<VisualBuilder<bool>>,
}

impl<T> Radio<T> {
    /// Creates an unselected radio for `value`.
    #[inline]
    pub fn new(value: T) -> Self {
        Self {
            value,
            selected: false,
            common: Common::default(),
            on_change: None,
            builder: None,
        }
    }

    /// Sets whether this radio is the selected one.
    #[inline]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Registers the callback that receives this radio's value when it is
    /// activated while unselected.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(T) + 'static,
    {
        self.on_change = Some(Rc::new(callback));
        self
    }

    common_setters!();

    /// Replaces the default circle and label with a composition of your own.
    ///
    /// The closure runs on every rebuild; `state.is_on()` is the selected flag.
    /// Pointer, keyboard, focus, disabled, and callback behavior stay with the
    /// radio.
    #[inline]
    pub fn builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(ToggleVisualState<'a>) -> W + 'static,
        W: Widget + 'static,
    {
        self.builder = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Returns a snapshot of the radio for assistive technology.
    pub fn semantics(&self) -> SelectionSemantics {
        SelectionSemantics::new(SemanticRole::Radio, &self.common).selected(self.selected)
    }
}

type RadioState<T> = Binary<bool, T>;

impl<T: Clone + 'static> State<Radio<T>> for RadioState<T> {
    fn create(widget: Radio<T>) -> Self {
        Binary::new(
            widget.selected,
            Mark::Radio {
                selected: widget.selected,
            },
            (!widget.selected).then_some(widget.value),
            widget.on_change,
            widget.common,
            widget.builder,
        )
    }

    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.init(updater);
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.adopt(new);
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        this.render(ctx)
    }
}

impl<T: Clone + 'static> Widget for Radio<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, RadioState<T>>(self, ctx, "Radio", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Radio"
    }
}

