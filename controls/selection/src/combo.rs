//! The retained state behind `Select` and `Autocomplete`.
//!
//! Both are a trigger that opens a popup of options. They differ only in that
//! an autocomplete's popup carries a search field and filters its options, so
//! they share one state and the autocomplete adds a [`Typeahead`].

use std::rc::Rc;

use aimer_input::TextEditingController;
use aimer_modal::{Anchor, AnchorHandle};
use aimer_text::Text;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyWidget, FocusNode, StateUpdater, Widget};

use crate::binary::ChangeCallback;
use crate::chrome::{control_shell, labeled_row};
use crate::common::Common;
use crate::indicator::{Indicator, Mark};
use crate::interaction::{Controlled, Handlers};
use crate::keys::Key;
use crate::option::{ChoiceOption, Options};
use crate::popup::{PopupModel, RowBuilder, Search, Visible};
use crate::theme::{label_style, tokens};
use crate::visual::TriggerState;

/// Receives the query an autocomplete's search field proposes.
pub(crate) type QueryCallback = Rc<dyn Fn(String)>;

/// Builds a trigger's content from its visual state.
pub(crate) type TriggerBuilder = Rc<dyn for<'a> Fn(TriggerState<'a>) -> AnyWidget>;

/// Decides whether an option matches a query.
pub(crate) type MatchFn<T> = Rc<dyn Fn(&str, &ChoiceOption<T>) -> bool>;

/// How an autocomplete narrows its options as the query changes.
pub(crate) enum Filter<T> {
    /// Case-insensitive substring match on the label.
    Contains,
    /// A caller-supplied predicate.
    Custom(MatchFn<T>),
    /// Every option, always: the caller supplies options that already match,
    /// as a server-driven search does.
    None,
}

impl<T> Clone for Filter<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Contains => Self::Contains,
            Self::Custom(matcher) => Self::Custom(Rc::clone(matcher)),
            Self::None => Self::None,
        }
    }
}

/// The search side of an autocomplete.
pub(crate) struct Typeahead<T> {
    /// The query the popup currently shows and filters by.
    pub(crate) query: String,
    /// The query the owner last passed down, to tell a changed owner query
    /// from a rebuild that merely repeats it.
    pub(crate) owner_query: Option<String>,
    pub(crate) placeholder: Rc<str>,
    pub(crate) filter: Filter<T>,
    pub(crate) on_query_change: Option<QueryCallback>,
    pub(crate) controller: TextEditingController,
}

/// Case-insensitive substring test that allocates only for non-ASCII text.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        let needle = needle.as_bytes();
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

impl<T> Typeahead<T> {
    /// The option indices that match the current query, or `None` when every
    /// option is shown.
    fn matches(&self, options: &[ChoiceOption<T>]) -> Option<Vec<usize>> {
        let query = self.query.trim();
        let matcher: &dyn Fn(&ChoiceOption<T>) -> bool = match &self.filter {
            Filter::None => return None,
            Filter::Contains if query.is_empty() => return None,
            Filter::Contains => &|option| contains_ignore_case(option.label(), query),
            Filter::Custom(matcher) => &|option| matcher(query, option),
        };
        Some(
            options
                .iter()
                .enumerate()
                .filter(|(_, option)| matcher(option))
                .map(|(index, _)| index)
                .collect(),
        )
    }
}

/// Everything a `Select` or `Autocomplete` is configured with.
pub(crate) struct ComboConfig<T: 'static> {
    pub(crate) options: Options<T>,
    pub(crate) selected: Option<T>,
    pub(crate) on_change: Option<ChangeCallback<T>>,
    pub(crate) common: Common,
    pub(crate) placeholder: Rc<str>,
    pub(crate) empty_text: Option<Rc<str>>,
    pub(crate) loading: bool,
    pub(crate) trigger: Option<TriggerBuilder>,
    pub(crate) rows: Option<RowBuilder<T>>,
    pub(crate) search: Option<Typeahead<T>>,
}

/// Retained state of a select or autocomplete.
pub struct Combo<T: 'static> {
    config: ComboConfig<T>,
    model: Rc<PopupModel<T>>,
    node: FocusNode,
    handlers: Handlers,
}

impl<T: Clone + PartialEq + 'static> Combo<T> {
    pub(crate) fn new(mut config: ComboConfig<T>) -> Self {
        let node = config.common.focus_node.take().unwrap_or_else(FocusNode::new);
        let model = PopupModel::new(AnchorHandle::new());
        let combo = Self {
            config,
            model,
            node,
            // Replaced in `init`, once the state has an updater.
            handlers: Handlers::new(StateUpdater::<Self>::empty()),
        };
        combo.model.set_options(combo.config.options.clone());
        combo.push_config();
        combo.recompute_visible();
        combo
    }

    pub(crate) fn init(&mut self, updater: StateUpdater<Self>) {
        self.handlers = Handlers::new(updater);
        self.model.set_owner_refresh(Rc::new(move || {
            let _ = updater.try_set_state(|_| {});
        }));
        if let Some(search) = &self.config.search {
            let on_input: Rc<dyn Fn(String)> =
                Rc::new(move |text| updater.set_state(move |combo| combo.typed(text)));
            self.model.set_search(Some(Search {
                controller: search.controller.clone(),
                on_input,
                placeholder: Rc::clone(&search.placeholder),
            }));
        }
    }

    pub(crate) fn adopt(&mut self, mut new: Self) {
        let highlighted_key = self.model.highlighted_key();
        let options_changed = !self.config.options.ptr_eq(&new.config.options);
        let mut query_changed = false;
        if let (Some(mine), Some(theirs)) = (&mut self.config.search, new.config.search.take()) {
            // A rebuild that repeats what the user just typed changes nothing.
            query_changed = theirs.owner_query != mine.owner_query
                && theirs.owner_query.as_deref() != Some(mine.query.as_str());
            if let Some(query) = theirs.owner_query.as_ref().filter(|_| query_changed) {
                mine.query = query.clone();
                if mine.controller.value().text() != query.as_str() {
                    mine.controller.set_text(query.clone());
                }
            }
            mine.owner_query = theirs.owner_query;
            mine.placeholder = theirs.placeholder;
            mine.filter = theirs.filter;
            mine.on_query_change = theirs.on_query_change;
        }
        if let Some(node) = new.config.common.focus_node.take() {
            self.node = node;
        }
        if new.config.common.disabled {
            self.handlers.clear_pointer();
            self.model.close();
        }
        let search = self.config.search.take();
        self.config = ComboConfig {
            search,
            ..new.config
        };
        if options_changed {
            self.model.set_options(self.config.options.clone());
        }
        self.push_config();
        if options_changed || query_changed {
            self.recompute_visible();
        }
        // An owner that rebuilds its options on every build hands over a fresh
        // list each time; the highlight follows its option, not its position.
        if let Some(key) = highlighted_key {
            self.model.restore_highlight(&key);
        }
        if self.model.is_open() {
            self.model.refresh_panel();
        }
    }

    /// Copies the configuration the open popup reads into the shared model.
    fn push_config(&self) {
        let config = &self.config;
        self.model.set_selected(config.selected.clone());
        self.model.set_on_change(config.on_change.clone());
        self.model.set_rows(config.rows.clone());
        self.model.set_loading(config.loading);
        if let Some(text) = &config.empty_text {
            self.model.set_empty_text(Rc::clone(text));
        }
    }

    fn recompute_visible(&self) {
        let Some(search) = &self.config.search else {
            return;
        };
        let visible = match search.matches(&self.config.options) {
            Some(indices) => Visible::Subset(indices),
            None => Visible::All,
        };
        self.model.set_visible(visible);
    }

    /// Applies text the user typed into the popup's search field.
    fn typed(&mut self, text: String) {
        let Some(search) = &mut self.config.search else {
            return;
        };
        search.query.clone_from(&text);
        let callback = search.on_query_change.clone();
        self.recompute_visible();
        self.model.refresh_panel();
        if let Some(callback) = callback {
            callback(text);
        }
    }

    pub(crate) fn render(&self, ctx: &BuildContext) -> AnyWidget {
        let tokens = tokens(ctx);
        let flags = self.handlers.flags();
        let config = &self.config;
        let disabled = config.common.disabled;
        let open = self.model.is_open();
        let label = config.common.label.as_deref();
        let error = config.common.error.as_deref();
        let selected_label = config.selected.as_ref().and_then(|selected| {
            config
                .options
                .position_of_value(selected)
                .map(|index| config.options[index].label())
        });
        let state = TriggerState::new(
            selected_label,
            &config.placeholder,
            label,
            error,
            open,
            flags,
            disabled,
        );
        let content = match &config.trigger {
            Some(builder) => builder(state),
            None => {
                let row = labeled_row(
                    &tokens,
                    Indicator::new(&tokens, Mark::Chevron { open }, flags, disabled, error.is_some()),
                    Some(state.text()),
                    disabled,
                    selected_label.is_none(),
                );
                match label {
                    Some(label) => aimer_flex::Column::new()
                        .horizontal_alignment(aimer_flex::BoxAlignment::Start)
                        .children([
                            Text::new(label.to_owned())
                                .text_style(label_style(&tokens, disabled, false))
                                .boxed(),
                            row,
                        ])
                        .boxed(),
                    None => row,
                }
            }
        };
        let shell = control_shell(&tokens, flags, disabled, error, content);
        let anchored = Anchor::new()
            .handle(self.model.anchor().clone())
            .box_child(shell);
        self.handlers.wrap(disabled, &self.node, anchored)
    }
}

impl<T: Clone + PartialEq + 'static> Controlled for Combo<T> {
    fn consumes_key(&self, key: Key) -> bool {
        !self.config.common.disabled
            && (key.is_activation() || matches!(key, Key::ArrowDown | Key::ArrowUp))
    }

    fn on_key(&mut self, key: Key) {
        if self.consumes_key(key) {
            self.model.open();
        }
    }

    fn on_tap(&mut self) {
        if self.config.common.disabled {
            return;
        }
        if self.model.is_open() {
            self.model.close();
        } else {
            self.model.open();
        }
    }
}

#[cfg(test)]
impl<T: Clone + PartialEq + 'static> Combo<T> {
    pub(crate) fn model(&self) -> &Rc<PopupModel<T>> {
        &self.model
    }

    pub(crate) fn query(&self) -> Option<&str> {
        self.config.search.as_ref().map(|search| search.query.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_ignores_ascii_case_without_allocating_a_lowercase_copy() {
        assert!(contains_ignore_case("Apricot", "apr"));
        assert!(contains_ignore_case("Apricot", "COT"));
        assert!(!contains_ignore_case("Apricot", "xyz"));
        assert!(contains_ignore_case("Apricot", ""));
        assert!(!contains_ignore_case("ap", "apricot"));
    }

    fn fruit() -> Options<i32> {
        Options::try_new([
            ChoiceOption::new("apple", "Apple", 1),
            ChoiceOption::new("apricot", "Apricot", 2),
            ChoiceOption::new("banana", "Banana", 3),
        ])
        .unwrap()
    }

    fn autocomplete(query: Option<&str>, filter: Filter<i32>) -> Combo<i32> {
        let initial = query.unwrap_or_default().to_owned();
        Combo::new(ComboConfig {
            options: fruit(),
            selected: None,
            on_change: None,
            common: Common::default(),
            placeholder: Rc::from("Pick"),
            empty_text: None,
            loading: false,
            trigger: None,
            rows: None,
            search: Some(Typeahead {
                controller: TextEditingController::with_text(initial.clone()),
                query: initial,
                owner_query: query.map(str::to_owned),
                placeholder: Rc::from("Search"),
                filter,
                on_query_change: None,
            }),
        })
    }

    #[test]
    fn the_initial_query_filters_the_options() {
        let combo = autocomplete(Some("ap"), Filter::Contains);
        assert_eq!(combo.model().len(), 2);
    }

    #[test]
    fn typing_filters_immediately_and_reports_the_query() {
        let mut combo = autocomplete(None, Filter::Contains);
        let reported = Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = Rc::clone(&reported);
        combo.config.search.as_mut().unwrap().on_query_change =
            Some(Rc::new(move |query| observed.borrow_mut().push(query)));
        assert_eq!(combo.model().len(), 3);

        combo.typed("ban".to_owned());

        assert_eq!(combo.model().len(), 1);
        assert_eq!(combo.query(), Some("ban"));
        assert_eq!(reported.borrow().as_slice(), &["ban".to_owned()]);
    }

    #[test]
    fn an_owner_that_mirrors_the_query_does_not_refilter_twice() {
        let mut combo = autocomplete(None, Filter::Contains);
        combo.typed("ap".to_owned());
        let highlighted_after_typing = combo.model().highlighted();
        combo.model().highlight(Some(1));

        // The owner echoes what was typed back into a rebuilt widget.
        combo.adopt(autocomplete(Some("ap"), Filter::Contains));

        assert_eq!(combo.query(), Some("ap"));
        assert_eq!(combo.model().len(), 2);
        assert_eq!(combo.model().highlighted(), Some(1), "an echoed query keeps the highlight");
        assert_eq!(highlighted_after_typing, Some(0));
    }

    #[test]
    fn an_owner_can_change_the_query_and_the_field_follows() {
        let mut combo = autocomplete(Some("ap"), Filter::Contains);

        combo.adopt(autocomplete(Some("ban"), Filter::Contains));

        assert_eq!(combo.query(), Some("ban"));
        assert_eq!(combo.model().len(), 1);
        let search = combo.config.search.as_ref().unwrap();
        assert_eq!(search.controller.value().text(), "ban");
    }

    #[test]
    fn an_owner_that_never_sets_a_query_leaves_what_was_typed_alone() {
        let mut combo = autocomplete(None, Filter::Contains);
        combo.typed("ap".to_owned());

        combo.adopt(autocomplete(None, Filter::Contains));

        assert_eq!(combo.query(), Some("ap"));
        assert_eq!(combo.model().len(), 2);
    }

    #[test]
    fn unfiltered_shows_every_option_whatever_the_query() {
        let mut combo = autocomplete(Some("zzz"), Filter::None);
        combo.typed("nothing matches".to_owned());
        assert_eq!(combo.model().len(), 3);
    }

    #[test]
    fn a_custom_filter_decides_what_matches() {
        let by_key: MatchFn<i32> = Rc::new(|query, option| option.key().starts_with(query));
        let mut combo = autocomplete(None, Filter::Custom(by_key));
        combo.typed("ap".to_owned());
        assert_eq!(combo.model().len(), 2);
        combo.typed("an".to_owned());
        assert_eq!(combo.model().len(), 0, "the label matches but the key does not");
    }

    #[test]
    fn replacing_the_options_refilters_by_the_current_query() {
        let mut combo = autocomplete(None, Filter::Contains);
        combo.typed("ap".to_owned());
        let mut next = autocomplete(None, Filter::Contains);
        next.config.options = Options::try_new([
            ChoiceOption::new("grape", "Grape", 4),
            ChoiceOption::new("apple", "Apple", 1),
        ])
        .unwrap();

        combo.adopt(next);

        assert_eq!(combo.model().len(), 2, "grape contains \"ap\" too");
    }

    #[test]
    fn a_select_has_no_query() {
        let combo = Combo::new(ComboConfig {
            options: fruit(),
            selected: Some(2),
            on_change: None,
            common: Common::default(),
            placeholder: Rc::from("Pick"),
            empty_text: None,
            loading: false,
            trigger: None,
            rows: None,
            search: None,
        });
        assert_eq!(combo.query(), None);
        assert_eq!(combo.model().len(), 3);
    }

    #[test]
    fn matching_falls_back_to_unicode_lowercasing() {
        assert!(contains_ignore_case("Ünïcode", "ÜNÏ"));
    }
}
