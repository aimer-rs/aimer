//! The option popup shared by `Select` and `Autocomplete`.
//!
//! A popup is presented through [`aimer_modal::Floating`], so it is never
//! clipped by an ancestor, closes on an outside press or `Escape`, and takes
//! the keyboard while it is open. Its content is a separate tree from the
//! control that opened it, so the two share a [`PopupModel`]: the control writes
//! the model whenever it is rebuilt, and the open panel reads it on every build.
//! Either side asks the other to rebuild through the two refresh hooks.
//!
//! The option list is windowed. Only the rows in view are built, which keeps
//! opening a popup over thousands of options independent of their number.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_container::{Container, SizedBox};
use aimer_decoration::BoxDecoration;
use aimer_flex::{BoxAlignment, Column, Row};
use aimer_input::TextEditingController;
use aimer_input::mouse_region::MouseRegion;
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::input::TextField;
use aimer_modal::{AnchorHandle, Floating, FloatingAlign, FloatingSide, ModalHandle, OverflowPolicy};
use aimer_scroll::{ScrollController, Scrollable};
use aimer_style::{LayoutSpacing, Spacing, TextStyle, ThemeTokens, apply_state_layer};
use aimer_text::Text;
use aimer_widget::base::{BuildContext, Vec2d};
use aimer_widget::{
    AnyElement, AnyWidget, FocusBehavior, FocusNode, Focusable, PortableWidget, State,
    StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::binary::ChangeCallback;
use crate::indicator::{Indicator, Mark};
use crate::interaction::Flags;
use crate::keys::{Key, KeyRelay, map_named_key};
use crate::nav;
use crate::option::{ChoiceOption, Options};
use crate::theme::{label_style, tokens};
use crate::visual::OptionState;

/// Builds one option row from its state.
pub(crate) type RowBuilder<T> = Rc<dyn for<'a> Fn(OptionState<'a, T>) -> AnyWidget>;

/// Fewest pixels a popup is wide, however narrow its trigger is.
const MIN_WIDTH: f32 = 180.0;
/// Most rows a popup shows before it scrolls.
const MAX_ROWS: usize = 8;
/// Row height before density scaling.
const ROW_HEIGHT: f32 = 32.0;
/// Search field height before density scaling.
const SEARCH_HEIGHT: f32 = 36.0;
/// Height of the loading and empty messages before density scaling.
const STATUS_HEIGHT: f32 = 32.0;

/// Which options a popup shows.
pub(crate) enum Visible {
    /// Every option, in order. Costs nothing to hold.
    All,
    /// The listed option indices, in display order.
    Subset(Vec<usize>),
}

/// The text field an autocomplete popup shows above its list.
pub(crate) struct Search {
    pub(crate) controller: TextEditingController,
    pub(crate) on_input: Rc<dyn Fn(String)>,
    pub(crate) placeholder: Rc<str>,
}

/// Closes the popup it was handed, from inside or outside.
///
/// The row handlers are built before `Floating::show` returns the modal handle,
/// so the handle is delivered to this shared slot afterwards.
#[derive(Default)]
pub(crate) struct Dismiss {
    handle: RefCell<Option<ModalHandle>>,
    open: Cell<bool>,
}

impl Dismiss {
    fn is_open(&self) -> bool {
        self.open.get()
            && self
                .handle
                .borrow()
                .as_ref()
                .is_none_or(ModalHandle::is_showing)
    }

    fn close(&self) {
        self.open.set(false);
        if let Some(handle) = self.handle.borrow_mut().take() {
            handle.dismiss();
        }
    }
}

/// State shared by a control and the popup it opens.
pub(crate) struct PopupModel<T: 'static> {
    options: RefCell<Options<T>>,
    visible: RefCell<Visible>,
    selected: RefCell<Option<T>>,
    on_change: RefCell<Option<ChangeCallback<T>>>,
    /// Position within the visible options.
    highlighted: Cell<Option<usize>>,
    loading: Cell<bool>,
    empty_text: RefCell<Rc<str>>,
    dismiss: Dismiss,
    panel_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    owner_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    anchor: AnchorHandle,
    rows: RefCell<Option<RowBuilder<T>>>,
    search: RefCell<Option<Search>>,
    scroll: ScrollController,
    /// Rows the list shows at once, for scroll-into-view.
    viewport_rows: Cell<usize>,
    row_height: Cell<f32>,
}

impl<T: 'static> PopupModel<T> {
    pub(crate) fn new(anchor: AnchorHandle) -> Rc<Self> {
        Rc::new(Self {
            options: RefCell::new(Options::default()),
            visible: RefCell::new(Visible::All),
            selected: RefCell::new(None),
            on_change: RefCell::new(None),
            highlighted: Cell::new(None),
            loading: Cell::new(false),
            empty_text: RefCell::new(Rc::from("No results")),
            dismiss: Dismiss::default(),
            panel_refresh: RefCell::new(None),
            owner_refresh: RefCell::new(None),
            anchor,
            rows: RefCell::new(None),
            search: RefCell::new(None),
            scroll: ScrollController::new(),
            viewport_rows: Cell::new(MAX_ROWS),
            row_height: Cell::new(ROW_HEIGHT),
        })
    }

    #[inline]
    pub(crate) fn anchor(&self) -> &AnchorHandle {
        &self.anchor
    }

    pub(crate) fn set_options(&self, options: Options<T>) {
        *self.options.borrow_mut() = options;
        *self.visible.borrow_mut() = Visible::All;
        self.clamp_highlight();
    }

    pub(crate) fn options(&self) -> Options<T> {
        self.options.borrow().clone()
    }

    pub(crate) fn set_visible(&self, visible: Visible) {
        *self.visible.borrow_mut() = visible;
        self.highlighted.set(self.first_available());
    }

    pub(crate) fn set_selected(&self, selected: Option<T>) {
        *self.selected.borrow_mut() = selected;
    }

    pub(crate) fn set_on_change(&self, on_change: Option<ChangeCallback<T>>) {
        *self.on_change.borrow_mut() = on_change;
    }

    pub(crate) fn set_rows(&self, rows: Option<RowBuilder<T>>) {
        *self.rows.borrow_mut() = rows;
    }

    pub(crate) fn set_search(&self, search: Option<Search>) {
        *self.search.borrow_mut() = search;
    }

    pub(crate) fn set_loading(&self, loading: bool) {
        self.loading.set(loading);
    }

    pub(crate) fn set_empty_text(&self, text: Rc<str>) {
        *self.empty_text.borrow_mut() = text;
    }

    pub(crate) fn set_owner_refresh(&self, refresh: Rc<dyn Fn()>) {
        *self.owner_refresh.borrow_mut() = Some(refresh);
    }

    #[cfg(test)]
    pub(crate) fn highlighted(&self) -> Option<usize> {
        self.highlighted.get()
    }

    /// The key of the highlighted option, which survives the option list being
    /// rebuilt where its position may not.
    pub(crate) fn highlighted_key(&self) -> Option<Rc<str>> {
        let index = self.option_index(self.highlighted.get()?)?;
        Some(self.options.borrow()[index].shared_key())
    }

    /// Moves the highlight back to the option with `key`, if it is still shown
    /// and available. Changes nothing otherwise, and never scrolls.
    pub(crate) fn restore_highlight(&self, key: &str) {
        let position = (0..self.len()).find(|&position| {
            self.option_index(position)
                .is_some_and(|index| self.options.borrow()[index].key() == key)
        });
        if let Some(position) = position.filter(|&position| self.is_available(position)) {
            self.highlighted.set(Some(position));
        }
    }

    /// Asks the open panel, if any, to rebuild.
    pub(crate) fn refresh_panel(&self) {
        let refresh = self.panel_refresh.borrow().clone();
        if let Some(refresh) = refresh {
            refresh();
        }
    }

    fn refresh_owner(&self) {
        let refresh = self.owner_refresh.borrow().clone();
        if let Some(refresh) = refresh {
            refresh();
        }
    }

    #[inline]
    pub(crate) fn is_open(&self) -> bool {
        self.dismiss.is_open()
    }

    pub(crate) fn close(&self) {
        self.dismiss.close();
        self.refresh_owner();
    }

    /// The number of visible rows.
    pub(crate) fn len(&self) -> usize {
        match &*self.visible.borrow() {
            Visible::All => self.options.borrow().len(),
            Visible::Subset(indices) => indices.len(),
        }
    }

    /// The option index shown at `position`.
    fn option_index(&self, position: usize) -> Option<usize> {
        match &*self.visible.borrow() {
            Visible::All => (position < self.options.borrow().len()).then_some(position),
            Visible::Subset(indices) => indices.get(position).copied(),
        }
    }

    fn is_available(&self, position: usize) -> bool {
        self.option_index(position)
            .is_some_and(|index| !self.options.borrow()[index].is_disabled())
    }

    fn first_available(&self) -> Option<usize> {
        nav::first(self.len(), |position| self.is_available(position))
    }

    fn clamp_highlight(&self) {
        let valid = self
            .highlighted
            .get()
            .filter(|&position| self.is_available(position));
        self.highlighted.set(valid);
    }

    /// Moves the highlight and keeps it in view. Returns whether it moved.
    pub(crate) fn highlight(&self, position: Option<usize>) -> bool {
        let position = position.filter(|&position| self.is_available(position));
        if position == self.highlighted.get() {
            return false;
        }
        self.highlighted.set(position);
        self.scroll_into_view();
        self.refresh_panel();
        true
    }

    pub(crate) fn step_highlight(&self, forward: bool) {
        let next = nav::step(self.len(), self.highlighted.get(), forward, |position| {
            self.is_available(position)
        });
        self.highlight(next);
    }

    pub(crate) fn highlight_edge(&self, last: bool) {
        let len = self.len();
        let edge = if last {
            nav::last(len, |position| self.is_available(position))
        } else {
            nav::first(len, |position| self.is_available(position))
        };
        self.highlight(edge);
    }

    fn scroll_into_view(&self) {
        let Some(position) = self.highlighted.get() else {
            return;
        };
        let row = self.row_height.get();
        let viewport = self.viewport_rows.get() as f32 * row;
        let top = position as f32 * row;
        let offset = self.scroll.offset();
        let target = if top < offset.y {
            top
        } else if top + row > offset.y + viewport {
            top + row - viewport
        } else {
            return;
        };
        self.scroll.jump_to(Vec2d {
            x: 0.0,
            y: target.max(0.0),
        });
    }
}

impl<T: Clone + PartialEq + 'static> PopupModel<T> {
    /// Opens the popup unless it is already open.
    pub(crate) fn open(self: &Rc<Self>) {
        if self.is_open() {
            return;
        }
        let selected = self.selected.borrow().clone();
        let selected_position = selected.and_then(|selected| {
            (0..self.len()).find(|&position| {
                self.option_index(position)
                    .is_some_and(|index| *self.options.borrow()[index].value() == selected)
            })
        });
        self.highlighted
            .set(selected_position.or_else(|| self.first_available()));
        self.dismiss.open.set(true);
        let handle = Floating::new()
            .anchor(self.anchor.clone())
            .side(FloatingSide::Bottom)
            .align(FloatingAlign::Start)
            .gap(4.0)
            .overflow(OverflowPolicy::Flip)
            .child(PopupPanel {
                model: Rc::clone(self),
            })
            .show();
        *self.dismiss.handle.borrow_mut() = Some(handle);
        self.refresh_owner();
    }

    /// Chooses the option shown at `position`, proposes its value, and closes
    /// the popup. Choosing the value that is already selected only closes.
    pub(crate) fn pick(&self, position: usize) {
        if !self.is_open() {
            return;
        }
        let Some(index) = self.option_index(position) else {
            return;
        };
        let (value, disabled) = {
            let options = self.options.borrow();
            (options[index].value().clone(), options[index].is_disabled())
        };
        if disabled {
            return;
        }
        let changed = self.selected.borrow().as_ref() != Some(&value);
        let on_change = self.on_change.borrow().clone();
        self.close();
        if let (true, Some(on_change)) = (changed, on_change) {
            on_change(value);
        }
    }

    /// Chooses the highlighted option, if there is one.
    pub(crate) fn pick_highlighted(&self) {
        if let Some(position) = self.highlighted.get() {
            self.pick(position);
        }
    }

    fn option_state<'a>(
        &self,
        options: &'a [ChoiceOption<T>],
        position: usize,
    ) -> Option<OptionState<'a, T>> {
        let index = self.option_index(position)?;
        let option = options.get(index)?;
        let selected = self.selected.borrow().as_ref() == Some(option.value());
        Some(OptionState::new(
            option,
            selected,
            self.highlighted.get() == Some(position),
        ))
    }
}

/// The content of an open popup.
pub(crate) struct PopupPanel<T: 'static> {
    model: Rc<PopupModel<T>>,
}

/// Retained state of an open popup.
pub struct PanelState<T: 'static> {
    model: Rc<PopupModel<T>>,
    node: FocusNode,
    /// Set once the state is live. A state built only to carry configuration
    /// into `adopt_config_from` shares the live state's model and must not
    /// announce a close when it is dropped.
    mounted: bool,
}

impl<T: 'static> Drop for PanelState<T> {
    /// A popup that was dismissed from outside — by the barrier, say — never
    /// told its control, so the control learns of it here.
    fn drop(&mut self) {
        if !self.mounted {
            return;
        }
        self.model.dismiss.open.set(false);
        *self.model.panel_refresh.borrow_mut() = None;
        self.model.refresh_owner();
    }
}

impl<T: Clone + PartialEq + 'static> StatefulWidget for PopupPanel<T> {
    type State = PanelState<T>;

    fn create_state(self) -> Self::State {
        PanelState {
            model: self.model,
            node: FocusNode::new(),
            mounted: false,
        }
    }
}

impl<T: Clone + PartialEq + 'static> State<PopupPanel<T>> for PanelState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.mounted = true;
        *self.model.panel_refresh.borrow_mut() = Some(Rc::new(move || {
            let _ = updater.try_set_state(|_| {});
        }));
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.model = Rc::clone(&new.model);
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let model = &self.model;
        let tokens = tokens(ctx);
        let row_height = tokens.density.target_size(ROW_HEIGHT);
        let len = model.len();
        let rows = len.min(MAX_ROWS);
        model.viewport_rows.set(rows.max(1));
        model.row_height.set(row_height);
        let width = model
            .anchor
            .bounds()
            .map_or(MIN_WIDTH, |bounds| bounds.width.max(MIN_WIDTH));

        // A `Container` left at `Auto` fills whatever space it is given, which
        // in an overlay is the whole window, so every part of the panel gets an
        // explicit height and the panel is the sum of them.
        let pad = tokens.spacing.x_small;
        let mut panel_height = 0.0;
        let mut children: Vec<AnyWidget> = Vec::with_capacity(2);
        if let Some(search) = &*model.search.borrow() {
            let field_height = tokens.density.target_size(SEARCH_HEIGHT);
            panel_height += field_height + 2.0 * pad;
            let on_input = Rc::clone(&search.on_input);
            let panel = Rc::clone(model);
            children.push(
                Container::new()
                    .padding(LayoutSpacing::all(Spacing::Px(tokens.spacing.x_small as u32)))
                    .child(
                        SizedBox::new().height(field_height).child(
                        TextField::new()
                            .controller(search.controller.clone())
                            .hint(std::sync::Arc::<str>::from(&*search.placeholder))
                            .auto_focus(true)
                            .on_changed(move |text: String| on_input(text))
                            // A field consumes the arrows and Escape itself, so
                            // the popup claims them through the field's filter.
                            .on_key(move |key, _modifiers| {
                                map_named_key(key).is_some_and(|key| on_key(&panel, key, true))
                            }),
                        ),
                    )
                    .boxed(),
            );
        }
        if model.loading.get() || len == 0 {
            let status_height = tokens.density.target_size(STATUS_HEIGHT);
            panel_height += status_height;
            let text = if model.loading.get() {
                "Loading…".to_owned()
            } else {
                model.empty_text.borrow().to_string()
            };
            children.push(status_text(&tokens, &text, status_height));
        } else {
            panel_height += row_height * rows as f32;
            children.push(self.list(&tokens, row_height, rows));
        }

        let body = Container::new()
            .width(width)
            .height(panel_height)
            .box_decoration(
                BoxDecoration::new()
                    .background_color(tokens.colors.surface)
                    .border_radius(tokens.shape.medium),
            )
            .child(
                Column::new()
                    
                    .children(children),
            );
        Focusable::new()
            .node(self.node.clone())
            .behavior(FocusBehavior::Auto)
            .child(body)
    }
}

/// Applies a key to an open popup. Returns whether the key was used.
fn on_key<T: Clone + PartialEq + 'static>(
    model: &Rc<PopupModel<T>>,
    key: Key,
    searchable: bool,
) -> bool {
    match key {
        key if key.is_next() && !matches!(key, Key::ArrowRight) => model.step_highlight(true),
        key if key.is_previous() && !matches!(key, Key::ArrowLeft) => model.step_highlight(false),
        // The overlay host would close on Escape too, but a text field consumes
        // it first, so the popup closes itself.
        Key::Escape => model.close(),
        Key::Home if !searchable => model.highlight_edge(false),
        Key::End if !searchable => model.highlight_edge(true),
        Key::Enter => model.pick_highlighted(),
        // A search field owns the space bar for typing.
        Key::Space if !searchable => model.pick_highlighted(),
        _ => return false,
    }
    true
}

fn status_text(tokens: &ThemeTokens, text: &str, height: f32) -> AnyWidget {
    Container::new()
        .height(height)
        .padding(LayoutSpacing::all(Spacing::Px(tokens.spacing.small as u32)))
        .child(
            Text::new(text.to_owned()).text_style(
                TextStyle::new()
                    .font_size(tokens.typography.label.font_size as u32)
                    .color(tokens.colors.outline),
            ),
        )
        .boxed()
}

impl<T: Clone + PartialEq + 'static> PanelState<T> {
    /// The windowed list of option rows.
    fn list(&self, tokens: &ThemeTokens, row_height: f32, rows: usize) -> AnyWidget {
        let model = Rc::clone(&self.model);
        let keys = Rc::clone(&self.model);
        let searchable = model.search.borrow().is_some();
        let tokens = *tokens;
        let options = model.options();
        let positions: Vec<usize> = (0..model.len()).collect();
        let list = Column::new()
            
            .list(positions)
            .item_extent(row_height)
            .builder(move |&position| row(&model, &options, &tokens, row_height, position));
        SizedBox::new()
            .height(row_height * rows as f32)
            .child(
                // The key handler sits inside the scroll view on purpose: a
                // focused region offers a key to its subtree innermost first,
                // and a scroll view would otherwise scroll on Up and Down
                // before the popup moved its highlight.
                Scrollable::new()
                    .controller(self.model.scroll.clone())
                    .child(
                        KeyRelay::new()
                            .on_key(move |key| on_key(&keys, key, searchable))
                            .child(list),
                    ),
            )
            .boxed()
    }
}

/// One option row: a leading tick, the label, and pointer handlers.
fn row<T: Clone + PartialEq + 'static>(
    model: &Rc<PopupModel<T>>,
    options: &Options<T>,
    tokens: &ThemeTokens,
    row_height: f32,
    position: usize,
) -> AnyWidget {
    let Some(state) = model.option_state(options, position) else {
        return SizedBox::new().height(row_height).boxed();
    };
    let content = match &*model.rows.borrow() {
        Some(builder) => builder(state),
        None => default_row(tokens, row_height, state),
    };
    if state.is_disabled() {
        return content;
    }
    let hover = Rc::clone(model);
    let tap = Rc::clone(model);
    MouseRegion::new()
        .on_hover_enter(move || {
            hover.highlight(Some(position));
        })
        .child(
            GestureDetector::new()
                .on_tap(move || tap.pick(position))
                .child(content),
        )
        .boxed()
}

fn default_row<T>(tokens: &ThemeTokens, row_height: f32, state: OptionState<'_, T>) -> AnyWidget {
    let highlighted = state.is_highlighted() && !state.is_disabled();
    let background = if highlighted {
        apply_state_layer(tokens.colors.surface, tokens.state.hover)
    } else {
        tokens.colors.surface
    };
    let tick_side = row_height * 0.5;
    let tick = if state.is_selected() {
        Indicator::new(tokens, Mark::Tick, Flags::default(), state.is_disabled(), false)
            .sized(tick_side)
            .boxed()
    } else {
        SizedBox::new().width(tick_side).height(tick_side).boxed()
    };
    Container::new()
        .height(row_height)
        .padding(LayoutSpacing::all(Spacing::Px(tokens.spacing.x_small as u32)))
        .color(background)
        .child(
            Row::new()
                .vertical_alignment(BoxAlignment::Center)
                .gaps(LayoutSpacing::all(Spacing::Px(tokens.spacing.small as u32)))
                .children([
                    tick,
                    Text::new(state.label().to_owned())
                        .text_style(label_style(tokens, state.is_disabled(), highlighted))
                        .boxed(),
                ]),
        )
        .boxed()
}

impl<T: Clone + PartialEq + 'static> Widget for PopupPanel<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "SelectionPopup", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SelectionPopup"
    }
}

impl<T: 'static> PortableWidget for PopupPanel<T> {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use aimer_attribute::BoxConstraint;
    use aimer_attribute::size::ResolvedSize;
    use aimer_widget::base::WindowHandle;
    use aimer_widget::{Drawable, LayoutElement};

    use super::*;

    fn model(options: &[(&str, &str, i32)]) -> Rc<PopupModel<i32>> {
        let model = PopupModel::new(AnchorHandle::new());
        model.set_options(
            Options::try_new(
                options
                    .iter()
                    .map(|&(key, label, value)| ChoiceOption::new(key, label, value)),
            )
            .unwrap(),
        );
        model
    }

    fn record(model: &PopupModel<i32>) -> Rc<RefCell<Vec<i32>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&log);
        model.set_on_change(Some(Rc::new(move |value| observed.borrow_mut().push(value))));
        log
    }

    #[test]
    fn opening_highlights_the_selection_else_the_first_available_option() {
        let model = model(&[("a", "A", 1), ("b", "B", 2), ("c", "C", 3)]);
        model.set_selected(Some(3));
        model.open();
        assert_eq!(model.highlighted(), Some(2));
        model.close();

        let model = model_with_disabled_first();
        model.open();
        assert_eq!(model.highlighted(), Some(1), "a disabled first row is skipped");
    }

    fn model_with_disabled_first() -> Rc<PopupModel<i32>> {
        let model = PopupModel::new(AnchorHandle::new());
        model.set_options(
            Options::try_new([
                ChoiceOption::new("a", "A", 1).disabled(true),
                ChoiceOption::new("b", "B", 2),
            ])
            .unwrap(),
        );
        model
    }

    #[test]
    fn picking_proposes_the_value_and_closes() {
        let model = model(&[("a", "A", 1), ("b", "B", 2)]);
        let log = record(&model);
        model.open();
        model.pick(1);
        assert_eq!(log.borrow().as_slice(), &[2]);
        assert!(!model.is_open());
    }

    #[test]
    fn picking_the_selected_value_only_closes() {
        let model = model(&[("a", "A", 1), ("b", "B", 2)]);
        let log = record(&model);
        model.set_selected(Some(2));
        model.open();
        model.pick(1);
        assert!(log.borrow().is_empty());
        assert!(!model.is_open());
    }

    #[test]
    fn a_disabled_option_cannot_be_picked() {
        let model = model_with_disabled_first();
        let log = record(&model);
        model.open();
        model.pick(0);
        assert!(log.borrow().is_empty());
        assert!(model.is_open(), "a refused pick leaves the popup open");
    }

    #[test]
    fn a_pick_after_the_popup_closed_does_nothing() {
        let model = model(&[("a", "A", 1)]);
        let log = record(&model);
        model.open();
        model.pick(0);
        model.pick(0);
        assert_eq!(log.borrow().as_slice(), &[1], "Enter and a submit must not double-fire");
    }

    #[test]
    fn the_highlight_wraps_and_skips_disabled_options() {
        let model = PopupModel::new(AnchorHandle::new());
        model.set_options(
            Options::try_new([
                ChoiceOption::new("a", "A", 1),
                ChoiceOption::new("b", "B", 2).disabled(true),
                ChoiceOption::new("c", "C", 3),
            ])
            .unwrap(),
        );
        model.open();
        assert_eq!(model.highlighted(), Some(0));
        model.step_highlight(true);
        assert_eq!(model.highlighted(), Some(2));
        model.step_highlight(true);
        assert_eq!(model.highlighted(), Some(0));
        model.step_highlight(false);
        assert_eq!(model.highlighted(), Some(2));
        model.highlight_edge(false);
        assert_eq!(model.highlighted(), Some(0));
        model.highlight_edge(true);
        assert_eq!(model.highlighted(), Some(2));
    }

    #[test]
    fn a_subset_shows_only_its_options_in_order() {
        let model = model(&[("a", "A", 1), ("b", "B", 2), ("c", "C", 3)]);
        let log = record(&model);
        model.set_visible(Visible::Subset(vec![2, 0]));
        assert_eq!(model.len(), 2);
        model.open();
        assert_eq!(model.highlighted(), Some(0));
        model.pick(0);
        assert_eq!(log.borrow().as_slice(), &[3], "position 0 of the subset is option 2");
    }

    #[test]
    fn keys_move_pick_and_are_left_alone_when_unused() {
        let model = model(&[("a", "A", 1), ("b", "B", 2)]);
        let log = record(&model);
        model.open();
        assert!(on_key(&model, Key::ArrowDown, false));
        assert_eq!(model.highlighted(), Some(1));
        assert!(!on_key(&model, Key::ArrowRight, false), "a popup has no use for Right");
        assert!(!on_key(&model, Key::ArrowLeft, false), "a popup has no use for Left");
        assert!(!on_key(&model, Key::Home, true), "a search field keeps Home for the caret");
        assert!(on_key(&model, Key::Enter, true));
        assert_eq!(log.borrow().as_slice(), &[2]);
    }

    #[test]
    fn escape_closes_the_popup_even_from_a_search_field() {
        let model = model(&[("a", "A", 1)]);
        let log = record(&model);
        model.open();
        assert!(on_key(&model, Key::Escape, true));
        assert!(!model.is_open());
        assert!(log.borrow().is_empty(), "closing proposes nothing");
    }

    #[test]
    fn space_picks_only_when_there_is_no_search_field() {
        let model = model(&[("a", "A", 1)]);
        let log = record(&model);
        model.open();
        assert!(!on_key(&model, Key::Space, true));
        assert!(log.borrow().is_empty());
        assert!(on_key(&model, Key::Space, false));
        assert_eq!(log.borrow().as_slice(), &[1]);
    }

    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 400.0,
                height: 800.0,
            },
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 400.0,
            max_height: 800.0,
        };
        ctx
    }

    #[tokio::test]
    async fn a_long_list_builds_only_the_rows_in_view() {
        let ctx = context();
        let model = PopupModel::new(AnchorHandle::new());
        model.set_options(
            Options::try_new((0..5000).map(|i| ChoiceOption::new(format!("k{i}"), format!("Option {i}"), i)))
                .unwrap(),
        );
        let built = Rc::new(Cell::new(0_u32));
        let counted = Rc::clone(&built);
        model.set_rows(Some(Rc::new(move |_state| {
            counted.set(counted.get() + 1);
            SizedBox::new().height(40.0).boxed()
        })));
        let state = PopupPanel {
            model: Rc::clone(&model),
        }
        .create_state();

        let element = state.build(&ctx).to_element(&ctx);
        // A windowed list has no rows until its first frame.
        for _ in 0..2 {
            let _ = element.layout(&ctx);
            element.update(&ctx);
        }

        assert!(
            (1..100).contains(&built.get()),
            "opening a 5000-option popup built {} rows, expected a window of them",
            built.get()
        );
        drop(state);
    }

    #[tokio::test]
    async fn the_panel_is_as_tall_as_its_rows_not_as_tall_as_the_window() {
        let ctx = context();
        let model = model(&[("a", "A", 1), ("b", "B", 2), ("c", "C", 3)]);
        let state = PopupPanel {
            model: Rc::clone(&model),
        }
        .create_state();

        let size = state.build(&ctx).to_element(&ctx).layout(&ctx);

        let row = ThemeTokens::light().density.target_size(ROW_HEIGHT);
        assert!(
            (size.height - 3.0 * row).abs() < 1.0,
            "three rows are {} tall, the panel is {}",
            3.0 * row,
            size.height
        );
    }

    #[tokio::test]
    async fn a_search_field_and_a_status_line_add_to_the_height_of_an_empty_panel() {
        let ctx = context();
        let model = model(&[("a", "A", 1)]);
        model.set_visible(Visible::Subset(Vec::new()));
        let state = PopupPanel {
            model: Rc::clone(&model),
        }
        .create_state();
        let status_only = state.build(&ctx).to_element(&ctx).layout(&ctx).height;

        model.set_search(Some(Search {
            controller: TextEditingController::new(),
            on_input: Rc::new(|_| {}),
            placeholder: Rc::from("Search"),
        }));
        let with_search = state.build(&ctx).to_element(&ctx).layout(&ctx).height;

        let tokens = ThemeTokens::light();
        assert!((status_only - tokens.density.target_size(STATUS_HEIGHT)).abs() < 1.0);
        assert!(with_search > status_only + tokens.density.target_size(SEARCH_HEIGHT) - 1.0);
        assert!(with_search < 400.0, "an empty panel must not fill the window: {with_search}");
    }

    #[tokio::test]
    async fn a_loading_popup_shows_no_rows_and_an_empty_one_says_so() {
        let ctx = context();
        let model = model(&[("a", "A", 1)]);
        let built = Rc::new(Cell::new(0_u32));
        let counted = Rc::clone(&built);
        model.set_rows(Some(Rc::new(move |_state| {
            counted.set(counted.get() + 1);
            SizedBox::new().height(40.0).boxed()
        })));
        model.set_loading(true);
        let state = PopupPanel {
            model: Rc::clone(&model),
        }
        .create_state();
        let _ = state.build(&ctx).to_element(&ctx).layout(&ctx);
        assert_eq!(built.get(), 0);

        model.set_loading(false);
        model.set_visible(Visible::Subset(Vec::new()));
        let _ = state.build(&ctx).to_element(&ctx).layout(&ctx);
        assert_eq!(built.get(), 0);
    }

    #[tokio::test]
    async fn arrow_keys_and_enter_drive_a_focused_popup() {
        use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
        use aimer_events::pointer::{PointerButton, PointerInfo};
        use aimer_widget::EventDispatcher;

        let ctx = context();
        let model = PopupModel::new(AnchorHandle::new());
        model.set_options(
            Options::try_new([
                ChoiceOption::new("a", "A", 1).disabled(true),
                ChoiceOption::new("b", "B", 2),
                ChoiceOption::new("c", "C", 3),
            ])
            .unwrap(),
        );
        let log = record(&model);
        model.open();
        let state = PopupPanel {
            model: Rc::clone(&model),
        }
        .create_state();
        let element = state.build(&ctx).to_element(&ctx);
        for _ in 0..2 {
            let _ = element.layout(&ctx);
            element.update(&ctx);
        }

        // Press the disabled first row: it focuses the panel without picking.
        let pointer = PointerInfo::mouse(Vec2d { x: 24.0, y: 10.0 }, PointerButton::Primary);
        let mut dispatcher = EventDispatcher::new();
        for event in [ElementEvent::PointerDown(pointer), ElementEvent::PointerUp(pointer)] {
            let _ = dispatcher.dispatch(element.as_ref(), pointer.pos, &event);
            element.update(&ctx);
        }
        assert!(log.borrow().is_empty());
        assert!(state.node.has_focus(), "pressing the panel must focus it");

        let mut press = |key| {
            // Keys go to the focus owner whatever the pointer is over.
            let result = dispatcher.dispatch(
                element.as_ref(),
                Vec2d { x: -50.0, y: -50.0 },
                &ElementEvent::KeyInput {
                    key,
                    action: KeyAction::Pressed,
                    modifiers: Modifiers::default(),
                },
            );
            element.update(&ctx);
            result
        };

        assert_eq!(model.highlighted(), Some(1));
        assert!(press(NamedKey::ArrowDown).is_consumed());
        assert_eq!(model.highlighted(), Some(2));
        assert!(press(NamedKey::ArrowUp).is_consumed());
        assert_eq!(model.highlighted(), Some(1));
        assert!(press(NamedKey::Enter).is_consumed());
        assert_eq!(log.borrow().as_slice(), &[2], "Enter chooses the highlighted option");
    }

    #[tokio::test]
    async fn a_focused_search_field_still_lets_the_arrows_and_enter_drive_the_popup() {
        use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
        use aimer_events::pointer::{PointerButton, PointerInfo};
        use aimer_widget::EventDispatcher;

        let ctx = context();
        let model = model(&[("a", "A", 1), ("b", "B", 2), ("c", "C", 3)]);
        let log = record(&model);
        let controller = TextEditingController::new();
        model.set_search(Some(Search {
            controller: controller.clone(),
            on_input: Rc::new(|_| {}),
            placeholder: Rc::from("Search"),
        }));
        model.open();
        let state = PopupPanel {
            model: Rc::clone(&model),
        }
        .create_state();
        let element = state.build(&ctx).to_element(&ctx);
        for _ in 0..2 {
            let _ = element.layout(&ctx);
            element.update(&ctx);
        }

        // Click into the search field, as a user would.
        let pointer = PointerInfo::mouse(Vec2d { x: 24.0, y: 22.0 }, PointerButton::Primary);
        let mut dispatcher = EventDispatcher::new();
        for event in [ElementEvent::PointerDown(pointer), ElementEvent::PointerUp(pointer)] {
            let _ = dispatcher.dispatch(element.as_ref(), pointer.pos, &event);
            element.update(&ctx);
        }
        assert!(log.borrow().is_empty(), "the click must land on the field, not on a row");

        let mut press = |key| {
            let result = dispatcher.dispatch(
                element.as_ref(),
                Vec2d { x: -50.0, y: -50.0 },
                &ElementEvent::KeyInput {
                    key,
                    action: KeyAction::Pressed,
                    modifiers: Modifiers::default(),
                },
            );
            element.update(&ctx);
            result
        };

        assert_eq!(model.highlighted(), Some(0));
        assert!(press(NamedKey::ArrowDown).is_consumed());
        assert_eq!(model.highlighted(), Some(1), "ArrowDown moves the popup highlight, not the caret");
        assert!(press(NamedKey::ArrowDown).is_consumed());
        assert!(press(NamedKey::ArrowUp).is_consumed());
        assert_eq!(model.highlighted(), Some(1));
        assert!(press(NamedKey::Enter).is_consumed());
        assert_eq!(log.borrow().as_slice(), &[2], "Enter chooses the highlighted option");
        assert!(!model.is_open());
    }

    /// Profile harness; run with `--run-ignored ignored-only popup_open`.
    #[ignore = "profile harness"]
    #[tokio::test]
    async fn profile_popup_open_500() {
        let ctx = context();
        let model = PopupModel::new(AnchorHandle::new());
        model.set_options(
            Options::try_new((0..500).map(|i| ChoiceOption::new(format!("k{i}"), format!("Option number {i}"), i)))
                .unwrap(),
        );
        let iterations = 20;
        let started = aimer_time::Instant::now();
        for _ in 0..iterations {
            let state = PopupPanel {
                model: Rc::clone(&model),
            }
            .create_state();
            let element = state.build(&ctx).to_element(&ctx);
            for _ in 0..2 {
                std::hint::black_box(element.layout(&ctx));
                element.update(&ctx);
            }
        }
        println!(
            "popup open 500 build+layout+update: {:.3} ms/iter over {iterations}",
            started.elapsed().as_secs_f64() * 1000.0 / iterations as f64
        );
    }
}
