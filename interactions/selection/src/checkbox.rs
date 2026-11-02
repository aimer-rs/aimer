use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, PortableWidget, State, StateUpdater, StatefulElement,
    StatefulWidget, Widget,
};

use crate::binary::{Binary, ChangeCallback, VisualBuilder};
use crate::common::{Common, common_setters};
use crate::indicator::{Glyph, Mark};
use crate::semantics::{SelectionSemantics, SemanticRole};
use crate::visual::CheckboxVisualState;

/// The controlled value of a [`Checkbox`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckboxValue {
    /// The checkbox is not selected.
    #[default]
    Unchecked,
    /// The checkbox is selected.
    Checked,
    /// The checkbox represents a mixed value owned by its parent.
    Indeterminate,
}

impl CheckboxValue {
    /// The value a tap proposes: `Checked` becomes `Unchecked`, and anything
    /// else becomes `Checked`.
    #[inline]
    pub fn toggled(self) -> Self {
        match self {
            Self::Unchecked | Self::Indeterminate => Self::Checked,
            Self::Checked => Self::Unchecked,
        }
    }

    #[inline]
    fn glyph(self) -> Glyph {
        match self {
            Self::Unchecked => Glyph::None,
            Self::Checked => Glyph::Check,
            Self::Indeterminate => Glyph::Dash,
        }
    }
}

impl From<bool> for CheckboxValue {
    #[inline]
    fn from(checked: bool) -> Self {
        if checked { Self::Checked } else { Self::Unchecked }
    }
}

/// A controlled checkbox with an explicit tri-state transition policy.
///
/// A checkbox never changes its own value. A tap or an activation key calls
/// [`on_change`](Self::on_change) with the proposed value, and the owner
/// rebuilds the checkbox with [`value`](Self::value). `Indeterminate` resolves
/// to `Checked`, and `Checked` to `Unchecked`.
///
/// Use [`builder`](Self::builder) to replace the default box and label with any
/// composition of widgets while keeping the interaction contract.
///
/// # Examples
///
/// ```
/// use aimer_selection::{Checkbox, CheckboxValue};
///
/// let checkbox = Checkbox::new()
///     .value(CheckboxValue::Indeterminate)
///     .label("Select all")
///     .on_change(|proposed| assert_eq!(proposed, CheckboxValue::Checked));
/// ```
pub struct Checkbox {
    value: CheckboxValue,
    common: Common,
    on_change: Option<ChangeCallback<CheckboxValue>>,
    builder: Option<VisualBuilder<CheckboxValue>>,
}

impl Default for Checkbox {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Checkbox {
    /// Creates an unchecked checkbox with no callback or label.
    #[inline]
    pub fn new() -> Self {
        Self {
            value: CheckboxValue::Unchecked,
            common: Common::default(),
            on_change: None,
            builder: None,
        }
    }

    /// Sets the caller-owned value.
    #[inline]
    pub fn value(mut self, value: CheckboxValue) -> Self {
        self.value = value;
        self
    }

    /// Registers the callback that receives proposed values.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(CheckboxValue) + 'static,
    {
        self.on_change = Some(Rc::new(callback));
        self
    }

    common_setters!();

    /// Replaces the default box and label with a composition of your own.
    ///
    /// The closure runs on every rebuild and receives the value, label, and
    /// interaction flags. Pointer, keyboard, focus, disabled, and callback
    /// behavior stay with the checkbox.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::Container;
    /// use aimer_flex::Row;
    /// use aimer_selection::Checkbox;
    /// use aimer_text::Text;
    /// use aimer_widget::Widget;
    ///
    /// let checkbox = Checkbox::new().label("Accept").builder(|state| {
    ///     let mark = if state.is_checked() { "✓" } else { " " };
    ///     Row::new().children([
    ///         Container::new().child(Text::new(mark)).boxed(),
    ///         Text::new(state.label().unwrap_or_default().to_owned()).boxed(),
    ///     ])
    /// });
    /// ```
    #[inline]
    pub fn builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(CheckboxVisualState<'a>) -> W + 'static,
        W: Widget + 'static,
    {
        self.builder = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Replaces the default visual with one retained child.
    ///
    /// Use [`builder`](Self::builder) when the content must follow the state.
    #[inline]
    pub fn child<W: Widget + 'static>(self, child: W) -> Self {
        let child = ChildBuilder::from_widget(child);
        self.builder(move |_| child.clone())
    }

    /// Replaces the default visual with `child` and erases the checkbox's type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }

    /// Returns a snapshot of the checkbox for assistive technology.
    pub fn semantics(&self) -> SelectionSemantics {
        SelectionSemantics::new(SemanticRole::Checkbox, &self.common).checked(self.value)
    }
}

type CheckboxState = Binary<CheckboxValue, CheckboxValue>;

impl StatefulWidget for Checkbox {
    type State = CheckboxState;

    fn create_state(self) -> Self::State {
        Binary::new(
            self.value,
            Mark::Box {
                check: self.value.glyph(),
            },
            Some(self.value.toggled()),
            self.on_change,
            self.common,
            self.builder,
        )
    }
}

impl State<Checkbox> for CheckboxState {
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

impl Widget for Checkbox {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Checkbox", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Checkbox"
    }
}

impl PortableWidget for Checkbox {}
