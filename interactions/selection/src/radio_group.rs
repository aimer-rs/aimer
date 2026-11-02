use std::cell::Cell;
use std::rc::Rc;

use aimer_flex::{BoxAlignment, Column};
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::mouse_region::MouseRegion;
use aimer_style::{LayoutSpacing, Spacing};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, FocusNode, PortableWidget, State, StateUpdater, StatefulElement,
    StatefulWidget, Widget,
};

use crate::binary::ChangeCallback;
use crate::chrome::{control_shell, labeled_row};
use crate::common::{Common, common_setters};
use crate::indicator::{Indicator, Mark};
use crate::interaction::{Controlled, Flags, Handlers};
use crate::keys::Key;
use crate::nav;
use crate::option::{ChoiceOption, OptionError, Options};
use crate::popup::RowBuilder;
use crate::semantics::{SelectionSemantics, SemanticRole};
use crate::theme::tokens;
use crate::visual::OptionState;

/// A controlled group of mutually exclusive radio options.
///
/// The group is one tab stop. Arrow keys move the focus ring between options
/// and propose each option they land on, `Home` and `End` move the ring
/// without proposing, and Enter or Space proposes the option under the ring.
/// Disabled options are skipped. Selection stays with the owner: the group
/// calls [`on_change`](Self::on_change) and the owner rebuilds it with
/// [`selected`](Self::selected).
///
/// # Examples
///
/// ```
/// use aimer_selection::{ChoiceOption, RadioGroup};
///
/// let plan = RadioGroup::new()
///     .options([
///         ChoiceOption::new("basic", "Basic", "basic"),
///         ChoiceOption::new("pro", "Pro", "pro"),
///     ])
///     .selected(Some("basic"))
///     .label("Plan")
///     .on_change(|plan| assert_eq!(plan, "pro"));
/// ```
pub struct RadioGroup<T> {
    options: Options<T>,
    selected: Option<T>,
    common: Common,
    on_change: Option<ChangeCallback<T>>,
    rows: Option<RowBuilder<T>>,
}

impl<T> Default for RadioGroup<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> RadioGroup<T> {
    /// Creates an empty group with nothing selected.
    #[inline]
    pub fn new() -> Self {
        Self {
            options: Options::default(),
            selected: None,
            common: Common::default(),
            on_change: None,
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
            Ok(group) => group,
            Err(error) => panic!("invalid radio group options: {error}"),
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

    /// Registers the callback that receives the proposed value.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(T) + 'static,
    {
        self.on_change = Some(Rc::new(callback));
        self
    }

    common_setters!();

    /// Replaces the default circle and label of each option with a composition
    /// of your own.
    ///
    /// The closure runs once per visible option on every rebuild.
    /// `state.is_highlighted()` is true under the pointer or the keyboard ring.
    /// Pointer, keyboard, and focus behavior stay with the group.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_selection::RadioGroup;
    /// use aimer_text::Text;
    ///
    /// let group = RadioGroup::<u8>::new().builder(|state| Text::new(state.label().to_owned()));
    /// ```
    #[inline]
    pub fn builder<F, W>(mut self, builder: F) -> Self
    where
        F: for<'a> Fn(OptionState<'a, T>) -> W + 'static,
        W: Widget + 'static,
    {
        self.rows = Some(Rc::new(move |state| builder(state).boxed()));
        self
    }

    /// Returns a snapshot of the group for assistive technology.
    pub fn semantics(&self) -> SelectionSemantics {
        SelectionSemantics::new(SemanticRole::RadioGroup, &self.common)
    }
}

/// Retained state of a radio group.
pub struct Group<T: 'static> {
    options: Options<T>,
    selected: Option<T>,
    common: Common,
    on_change: Option<ChangeCallback<T>>,
    rows: Option<RowBuilder<T>>,
    node: FocusNode,
    handlers: Handlers,
    /// The option under the keyboard ring.
    focused: Option<usize>,
    /// The option under the pointer, shared with the row handlers.
    hovered: Rc<Cell<Option<usize>>>,
    updater: StateUpdater<Self>,
}

impl<T: Clone + PartialEq + 'static> Group<T> {
    fn is_available(&self, index: usize) -> bool {
        self.options
            .get(index)
            .is_some_and(|option| !option.is_disabled())
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.options
            .iter()
            .position(|option| option.value() == selected && !option.is_disabled())
    }

    /// The option the ring is on, falling back to the selection and then to the
    /// first available option, which is where a group is entered.
    fn ring_index(&self) -> Option<usize> {
        self.focused
            .filter(|&index| self.is_available(index))
            .or_else(|| self.selected_index())
            .or_else(|| nav::first(self.options.len(), |index| self.is_available(index)))
    }

    /// Proposes the option at `index` unless it is the current selection.
    fn activate(&mut self, index: usize) {
        if self.common.disabled || !self.is_available(index) {
            return;
        }
        self.focused = Some(index);
        let value = self.options[index].value();
        if self.selected.as_ref() == Some(value) {
            return;
        }
        if let Some(on_change) = &self.on_change {
            on_change(value.clone());
        }
    }

    fn move_ring(&mut self, forward: bool) -> Option<usize> {
        let index = nav::step(self.options.len(), self.ring_index(), forward, |index| {
            self.is_available(index)
        })?;
        self.focused = Some(index);
        Some(index)
    }

    pub(crate) fn new(
        options: Options<T>,
        selected: Option<T>,
        mut common: Common,
        on_change: Option<ChangeCallback<T>>,
        rows: Option<RowBuilder<T>>,
    ) -> Self {
        let node = common.focus_node.take().unwrap_or_else(FocusNode::new);
        Self {
            options,
            selected,
            common,
            on_change,
            rows,
            node,
            handlers: Handlers::new(StateUpdater::<Self>::empty()),
            focused: None,
            hovered: Rc::new(Cell::new(None)),
            updater: StateUpdater::empty(),
        }
    }

    fn init(&mut self, updater: StateUpdater<Self>) {
        self.handlers = Handlers::new(updater);
        self.updater = updater;
    }

    fn adopt(&mut self, mut new: Self) {
        if let Some(node) = new.common.focus_node.take() {
            self.node = node;
        }
        if new.common.disabled {
            self.handlers.clear_pointer();
            self.hovered.set(None);
        }
        self.options = new.options;
        self.selected = new.selected;
        self.common = new.common;
        self.on_change = new.on_change;
        self.rows = new.rows;
        self.focused = self.focused.filter(|&index| self.is_available(index));
    }

    fn render(&self, ctx: &BuildContext) -> AnyWidget {
        let tokens = tokens(ctx);
        let flags = self.handlers.flags();
        let disabled = self.common.disabled;
        let error = self.common.error.as_deref();
        let ring = flags.focus_visible().then(|| self.ring_index()).flatten();
        let hovered = self.hovered.get();
        let positions: Vec<usize> = (0..self.options.len()).collect();
        let options = self.options.clone();
        let selected = self.selected.clone();
        let rows = self.rows.clone();
        let updater = self.updater;
        let hover_cell = Rc::clone(&self.hovered);
        let row_tokens = tokens;
        let list = Column::new()
            .horizontal_alignment(BoxAlignment::Start)
            .gaps(LayoutSpacing::all(Spacing::Px(tokens.spacing.small as u32)))
            .list(positions)
            .builder(move |&index| {
                let option = &options[index];
                let is_selected = selected.as_ref() == Some(option.value());
                let highlighted = ring == Some(index) || hovered == Some(index);
                let row_disabled = disabled || option.is_disabled();
                let content = match &rows {
                    Some(builder) => builder(OptionState::new(option, is_selected, highlighted)),
                    None => labeled_row(
                        &row_tokens,
                        Indicator::new(
                            &row_tokens,
                            Mark::Radio {
                                selected: is_selected,
                            },
                            Flags {
                                hovered: hovered == Some(index),
                                focused: ring == Some(index),
                                keyboard: ring == Some(index),
                                ..Flags::default()
                            },
                            row_disabled,
                            false,
                        ),
                        Some(option.label()),
                        row_disabled,
                        false,
                    ),
                };
                if row_disabled {
                    return content;
                }
                let enter_cell = Rc::clone(&hover_cell);
                let exit_cell = Rc::clone(&hover_cell);
                MouseRegion::new()
                    .on_hover_enter(move || set_hover(&enter_cell, updater, Some(index)))
                    .on_hover_exit(move || {
                        if exit_cell.get() == Some(index) {
                            set_hover(&exit_cell, updater, None);
                        }
                    })
                    .child(
                        GestureDetector::new()
                            .on_tap(move || updater.set_state(move |group| group.activate(index)))
                            .child(content),
                    )
                    .boxed()
            });
        let shell = control_shell(&tokens, Flags::default(), disabled, error, list.boxed());
        self.handlers.wrap_keys(disabled, &self.node, shell)
    }
}

fn set_hover<T: Clone + PartialEq + 'static>(
    cell: &Cell<Option<usize>>,
    updater: StateUpdater<Group<T>>,
    hovered: Option<usize>,
) {
    if cell.get() != hovered {
        cell.set(hovered);
        updater.set_state(|_| {});
    }
}

impl<T: Clone + PartialEq + 'static> Controlled for Group<T> {
    fn consumes_key(&self, key: Key) -> bool {
        !self.common.disabled
            && (key.is_activation()
                || key.is_next()
                || key.is_previous()
                || matches!(key, Key::Home | Key::End))
    }

    fn on_key(&mut self, key: Key) {
        if key.is_next() || key.is_previous() {
            if let Some(index) = self.move_ring(key.is_next()) {
                self.activate(index);
            }
        } else if matches!(key, Key::Home | Key::End) {
            let len = self.options.len();
            let edge = if key == Key::End {
                nav::last(len, |index| self.is_available(index))
            } else {
                nav::first(len, |index| self.is_available(index))
            };
            self.focused = edge.or(self.focused);
        } else if key.is_activation() {
            if let Some(index) = self.ring_index() {
                self.activate(index);
            }
        }
    }

    fn on_tap(&mut self) {}
}

impl<T: Clone + PartialEq + 'static> StatefulWidget for RadioGroup<T> {
    type State = Group<T>;

    fn create_state(self) -> Self::State {
        Group::new(
            self.options,
            self.selected,
            self.common,
            self.on_change,
            self.rows,
        )
    }
}

impl<T: Clone + PartialEq + 'static> State<RadioGroup<T>> for Group<T> {
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

impl<T: Clone + PartialEq + 'static> Widget for RadioGroup<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "RadioGroup", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "RadioGroup"
    }
}

impl<T: 'static> PortableWidget for RadioGroup<T> {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn group(selected: Option<i32>) -> (Group<i32>, Rc<RefCell<Vec<i32>>>) {
        let proposed = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&proposed);
        let options = Options::try_new([
            ChoiceOption::new("a", "A", 1).disabled(true),
            ChoiceOption::new("b", "B", 2),
            ChoiceOption::new("c", "C", 3),
        ])
        .unwrap();
        let group = Group::new(
            options,
            selected,
            Common::default(),
            Some(Rc::new(move |value| observed.borrow_mut().push(value))),
            None,
        );
        (group, proposed)
    }

    #[test]
    fn the_ring_starts_on_the_selection_else_the_first_available_option() {
        assert_eq!(group(Some(3)).0.ring_index(), Some(2));
        assert_eq!(group(None).0.ring_index(), Some(1), "option A is disabled");
        assert_eq!(group(Some(1)).0.ring_index(), Some(1), "a disabled selection is not a ring stop");
    }

    #[test]
    fn arrows_move_and_propose_while_home_and_end_only_move() {
        let (mut group, proposed) = group(None);
        group.on_key(Key::ArrowDown);
        assert_eq!(proposed.borrow().as_slice(), &[3]);
        group.on_key(Key::ArrowDown);
        assert_eq!(proposed.borrow().as_slice(), &[3, 2], "wraps past the disabled first option");

        group.on_key(Key::End);
        group.on_key(Key::Home);
        assert_eq!(proposed.borrow().len(), 2);
        assert_eq!(group.ring_index(), Some(1));
    }

    #[test]
    fn activating_the_selection_or_a_disabled_option_proposes_nothing() {
        let (mut group, proposed) = group(Some(2));
        group.activate(1);
        group.activate(0);
        group.activate(9);
        assert!(proposed.borrow().is_empty());
        group.activate(2);
        assert_eq!(proposed.borrow().as_slice(), &[3]);
    }

    #[test]
    fn a_disabled_group_consumes_no_keys_and_proposes_nothing() {
        let (mut group, proposed) = group(None);
        group.common.disabled = true;
        assert!(!group.consumes_key(Key::ArrowDown));
        group.activate(1);
        assert!(proposed.borrow().is_empty());
    }

    #[test]
    fn a_rebuild_keeps_the_ring_only_while_its_option_stays_available() {
        let (mut group, _) = group(None);
        group.on_key(Key::End);
        assert_eq!(group.focused, Some(2));
        let (next, _) = self::group(None);
        group.adopt(next);
        assert_eq!(group.focused, Some(2));

        let mut shorter = self::group(None).0;
        shorter.options = Options::try_new([ChoiceOption::new("a", "A", 1)]).unwrap();
        group.adopt(shorter);
        assert_eq!(group.focused, None);
    }
}
