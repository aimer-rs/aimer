use std::rc::Rc;

use aimer_input::TextEditingController;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, PortableWidget, State, StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::binary::ChangeCallback;
use crate::combo::{Combo, ComboConfig, Filter, QueryCallback, Typeahead, TriggerBuilder};
use crate::common::{Common, common_setters};
use crate::option::{ChoiceOption, OptionError, Options};
use crate::popup::RowBuilder;
use crate::semantics::{SelectionSemantics, SemanticRole};
use crate::visual::{OptionState, TriggerState};

/// A controlled single-choice combobox with a search field.
///
/// The trigger looks like a [`crate::Select`]. Opening it presents a popup with
/// a focused search field above the option list, and typing narrows the list.
/// Arrows move the highlight, Enter chooses it, and Escape or a press outside
/// closes the popup.
///
/// The selection is caller-owned like any other control's. The query is shared:
/// the control filters by what was typed straight away, reports it through
/// [`on_query_change`](Self::on_query_change), and takes a new
/// [`query`](Self::query) from the owner whenever the owner changes it — so
/// both a form that mirrors the query and one that never reads it behave.
///
/// By default options are filtered by a case-insensitive substring match on
/// their labels. Use [`filter`](Self::filter) to match differently, or
/// [`unfiltered`](Self::unfiltered) when the owner supplies options that
/// already match, as a server-driven search does, together with
/// [`loading`](Self::loading) while they are on their way.
///
/// # Examples
///
/// ```
/// use aimer_selection::{Autocomplete, ChoiceOption};
///
/// let fruit = Autocomplete::new()
///     .options([
///         ChoiceOption::new("apple", "Apple", 1),
///         ChoiceOption::new("apricot", "Apricot", 2),
///     ])
///     .placeholder("Pick a fruit")
///     .on_query_change(|query| println!("searching {query}"))
///     .on_change(|fruit| println!("chose {fruit}"));
/// ```
pub struct Autocomplete<T> {
    options: Options<T>,
    selected: Option<T>,
    query: Option<String>,
    placeholder: Rc<str>,
    search_placeholder: Rc<str>,
    empty_text: Option<Rc<str>>,
    loading: bool,
    filter: Filter<T>,
    common: Common,
    on_change: Option<ChangeCallback<T>>,
    on_query_change: Option<QueryCallback>,
    trigger: Option<TriggerBuilder>,
    rows: Option<RowBuilder<T>>,
}

impl<T> Default for Autocomplete<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Autocomplete<T> {
    /// Creates an autocomplete with no options, nothing selected, and an empty
    /// query.
    #[inline]
    pub fn new() -> Self {
        Self {
            options: Options::default(),
            selected: None,
            query: None,
            placeholder: Rc::from("Select…"),
            search_placeholder: Rc::from("Search…"),
            empty_text: None,
            loading: false,
            filter: Filter::Contains,
            common: Common::default(),
            on_change: None,
            on_query_change: None,
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
            Ok(autocomplete) => autocomplete,
            Err(error) => panic!("invalid autocomplete options: {error}"),
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

    /// Sets the search text. The popup adopts it whenever it differs from the
    /// last query the owner passed.
    #[inline]
    pub fn query(mut self, query: impl Into<String>) -> Self {
        self.query = Some(query.into());
        self
    }

    /// Sets the text the trigger shows while nothing is selected.
    #[inline]
    pub fn placeholder(mut self, placeholder: impl Into<Rc<str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the hint shown in the empty search field.
    #[inline]
    pub fn search_placeholder(mut self, placeholder: impl Into<Rc<str>>) -> Self {
        self.search_placeholder = placeholder.into();
        self
    }

    /// Sets the text shown when no option matches.
    #[inline]
    pub fn empty_text(mut self, text: impl Into<Rc<str>>) -> Self {
        self.empty_text = Some(text.into());
        self
    }

    /// Shows a loading row instead of the options while they are on their way.
    #[inline]
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Matches options with `matches(query, option)` instead of the default
    /// substring match.
    #[inline]
    pub fn filter<F>(mut self, matches: F) -> Self
    where
        F: Fn(&str, &ChoiceOption<T>) -> bool + 'static,
    {
        self.filter = Filter::Custom(Rc::new(matches));
        self
    }

    /// Shows every option whatever the query is. Use it when the owner supplies
    /// options that already match.
    #[inline]
    pub fn unfiltered(mut self) -> Self {
        self.filter = Filter::None;
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

    /// Registers the callback that receives the query as the user types.
    #[inline]
    pub fn on_query_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(String) + 'static,
    {
        self.on_query_change = Some(Rc::new(callback));
        self
    }

    common_setters!();

    /// Replaces the default trigger with a composition of your own.
    ///
    /// The closure runs on every rebuild and receives the selected label, the
    /// placeholder, and whether the popup is open. Opening, closing, keyboard,
    /// and focus behavior stay with the autocomplete.
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
    /// Pointer behavior stays with the autocomplete; the closure only draws the
    /// row.
    #[inline]
    pub fn option_builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(OptionState<'a, T>) -> W + 'static,
        W: Widget + 'static,
    {
        self.rows = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Returns a snapshot of the autocomplete for assistive technology.
    pub fn semantics(&self) -> SelectionSemantics {
        SelectionSemantics::new(SemanticRole::Autocomplete, &self.common).busy(self.loading)
    }
}

impl<T: Clone + PartialEq + 'static> StatefulWidget for Autocomplete<T> {
    type State = Combo<T>;

    fn create_state(self) -> Self::State {
        let initial = self.query.clone().unwrap_or_default();
        Combo::new(ComboConfig {
            options: self.options,
            selected: self.selected,
            on_change: self.on_change,
            common: self.common,
            placeholder: self.placeholder,
            empty_text: self.empty_text,
            loading: self.loading,
            trigger: self.trigger,
            rows: self.rows,
            search: Some(Typeahead {
                controller: TextEditingController::with_text(initial.clone()),
                query: initial,
                owner_query: self.query,
                placeholder: self.search_placeholder,
                filter: self.filter,
                on_query_change: self.on_query_change,
            }),
        })
    }
}

impl<T: Clone + PartialEq + 'static> State<Autocomplete<T>> for Combo<T> {
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

impl<T: Clone + PartialEq + 'static> Widget for Autocomplete<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Autocomplete", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Autocomplete"
    }
}

impl<T: 'static> PortableWidget for Autocomplete<T> {}
