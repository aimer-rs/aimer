use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, PortableWidget, State, StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::binary::ChangeCallback;
use crate::combo::{Combo, ComboConfig, TriggerBuilder};
use crate::common::{Common, common_setters};
use crate::option::{ChoiceOption, OptionError, Options};
use crate::popup::RowBuilder;
use crate::semantics::{SelectionSemantics, SemanticRole};
use crate::visual::{OptionState, TriggerState};

/// A controlled single-choice dropdown.
///
/// Tapping the trigger — or pressing Enter, Space, or an arrow key while it is
/// focused — opens a popup anchored under it. The popup takes the keyboard:
/// arrows, Home, and End move the highlight, Enter or Space chooses, and Escape
/// or a press outside closes it. Choosing an option calls
/// [`on_change`](Self::on_change) with its value and closes the popup; the
/// owner rebuilds the select with [`selected`](Self::selected). Choosing the
/// value that is already selected only closes.
///
/// Only the rows in view are built, so a select over thousands of options opens
/// as fast as one over a handful.
///
/// # Examples
///
/// ```
/// use aimer_selection::{ChoiceOption, Select};
///
/// let select = Select::new()
///     .options([
///         ChoiceOption::new("s", "Small", 1),
///         ChoiceOption::new("m", "Medium", 2),
///     ])
///     .selected(Some(2))
///     .label("Size")
///     .on_change(|size| assert_eq!(size, 1));
/// ```
pub struct Select<T> {
    options: Options<T>,
    selected: Option<T>,
    placeholder: Rc<str>,
    common: Common,
    on_change: Option<ChangeCallback<T>>,
    trigger: Option<TriggerBuilder>,
    rows: Option<RowBuilder<T>>,
}

impl<T> Default for Select<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Select<T> {
    /// Creates a select with no options, nothing selected, and no callback.
    #[inline]
    pub fn new() -> Self {
        Self {
            options: Options::default(),
            selected: None,
            placeholder: Rc::from("Select…"),
            common: Common::default(),
            on_change: None,
            trigger: None,
            rows: None,
        }
    }

    /// Sets the options, in display order.
    ///
    /// # Panics
    ///
    /// Panics if an option key is empty or repeated. Use
    /// [`try_options`](Self::try_options) for keys that come from data.
    #[inline]
    #[track_caller]
    pub fn options(self, options: impl IntoIterator<Item = ChoiceOption<T>>) -> Self {
        match self.try_options(options) {
            Ok(select) => select,
            Err(error) => panic!("invalid select options: {error}"),
        }
    }

    /// Sets the options, in display order, rejecting empty and repeated keys.
    pub fn try_options(
        mut self,
        options: impl IntoIterator<Item = ChoiceOption<T>>,
    ) -> Result<Self, OptionError> {
        self.options = Options::try_new(options)?;
        Ok(self)
    }

    /// Sets the caller-owned selection.
    #[inline]
    pub fn selected(mut self, selected: Option<T>) -> Self {
        self.selected = selected;
        self
    }

    /// Sets the text shown while nothing is selected.
    #[inline]
    pub fn placeholder(mut self, placeholder: impl Into<Rc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Registers the callback that receives the chosen value.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(T) + 'static,
    {
        self.on_change = Some(Rc::new(callback));
        self
    }

    common_setters!();

    /// Replaces the default trigger with a composition of your own.
    ///
    /// The closure runs on every rebuild and receives the selected label, the
    /// placeholder, and whether the popup is open. Opening, closing, keyboard,
    /// and focus behavior stay with the select.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_selection::Select;
    /// use aimer_text::Text;
    ///
    /// let select = Select::<u8>::new().builder(|state| Text::new(state.text().to_owned()));
    /// ```
    #[inline]
    pub fn builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(TriggerState<'a>) -> W + 'static,
        W: Widget + 'static,
    {
        self.trigger = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Replaces the default option row with a composition of your own.
    ///
    /// Pointer behavior stays with the select; the closure only draws the row.
    #[inline]
    pub fn option_builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(OptionState<'a, T>) -> W + 'static,
        W: Widget + 'static,
    {
        self.rows = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Returns a snapshot of the select for assistive technology.
    pub fn semantics(&self) -> SelectionSemantics {
        SelectionSemantics::new(SemanticRole::Select, &self.common)
    }
}

impl<T: Clone + PartialEq + 'static> StatefulWidget for Select<T> {
    type State = Combo<T>;

    fn create_state(self) -> Self::State {
        Combo::new(ComboConfig {
            options: self.options,
            selected: self.selected,
            on_change: self.on_change,
            common: self.common,
            placeholder: self.placeholder,
            empty_text: None,
            loading: false,
            trigger: self.trigger,
            rows: self.rows,
            search: None,
        })
    }
}

impl<T: Clone + PartialEq + 'static> State<Select<T>> for Combo<T> {
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

impl<T: Clone + PartialEq + 'static> Widget for Select<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Select", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Select"
    }
}

impl<T: 'static> PortableWidget for Select<T> {}
