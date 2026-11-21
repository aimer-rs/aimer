//! One verb of a context menu.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::svg::SvgScene;
use aimer_widget::{AnyWidget, Widget};

use crate::dismiss::ContextMenuDismiss;

/// A single row of a context menu.
///
/// An item is a label, whether it can be chosen, and what choosing it does.
/// The action is optional: a menu may instead take one callback for all of its
/// rows and tell them apart by index, which is what a menu built from a
/// computed list of verbs usually wants.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_ctxmenu::ContextMenuItem;
///
/// let copied = Rc::new(Cell::new(false));
/// let flag = Rc::clone(&copied);
/// let copy = ContextMenuItem::new("Copy").on_select(move || flag.set(true));
///
/// assert_eq!(copy.label(), "Copy");
/// assert!(copy.is_enabled());
///
/// let paste = ContextMenuItem::new("Paste").enabled(false);
/// assert!(!paste.is_enabled());
/// ```
#[derive(Clone)]
pub struct ContextMenuItem {
    label: Cow<'static, str>,
    enabled: bool,
    on_select: Option<Rc<dyn Fn()>>,
    hint: Option<Cow<'static, str>>,
    separated: bool,
    icon: Option<Arc<SvgScene>>,
    widget: Option<WidgetRow>,
}

/// What a row is that is a widget: how tall it is, how wide it needs the menu to be at least, and how it is made.
#[derive(Clone)]
struct WidgetRow {
    height: f32,
    min_width: f32,
    build: Rc<dyn Fn(ContextMenuDismiss) -> AnyWidget>,
}

impl ContextMenuItem {
    /// Creates an enabled item labelled `label`.
    ///
    /// A `&'static str` is borrowed rather than copied, which is what every
    /// fixed verb — `Copy`, `Select All` — costs here: nothing.
    #[inline]
    pub fn new(label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            on_select: None,
            hint: None,
            separated: false,
            icon: None,
            widget: None,
        }
    }

    /// Creates a row whose content is a widget, `height` logical pixels tall and as wide as the menu: a strip of buttons, a header, a row of
    /// colours, a toggle. The widget takes its own presses and everything else that happens on it, as it does anywhere; the menu does not
    /// mark the row or choose it, and does not close because of what is done on it. A widget that is to close the menu does so with the
    /// handle it is given.
    ///
    /// `build` makes the widget each time the menu is built, because a widget is made once for the place it is shown in. A row of this
    /// kind is for a list: a pill, whose items are words side by side, leaves it out.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::ZeroSizedBox;
    /// use aimer_ctxmenu::ContextMenuItem;
    ///
    /// let row = ContextMenuItem::widget(32.0, |_dismiss| ZeroSizedBox);
    /// assert!(row.is_widget());
    /// ```
    pub fn widget<W: Widget + 'static>(height: f32, build: impl Fn(ContextMenuDismiss) -> W + 'static) -> Self {
        Self {
            widget: Some(WidgetRow { height, min_width: 0.0, build: Rc::new(move |dismiss| build(dismiss).boxed()) }),
            ..Self::new("")
        }
    }

    /// Sets how wide the menu is to be at least for this row, in logical pixels, which is for a widget that cannot be narrower. The rows that
    /// are words keep to the width their longest needs, and the menu is as wide as the widest of them all.
    #[inline]
    pub fn min_width(mut self, min_width: f32) -> Self {
        if let Some(widget) = &mut self.widget {
            widget.min_width = min_width.max(0.0);
        }
        self
    }

    /// Whether the content of the row is a widget.
    #[inline]
    pub fn is_widget(&self) -> bool {
        self.widget.is_some()
    }

    /// How tall the widget of the row is, in logical pixels, if the content of the row is one.
    #[inline]
    pub fn widget_height(&self) -> Option<f32> {
        self.widget.as_ref().map(|widget| widget.height)
    }

    /// How wide the menu is to be at least for the widget of the row: none for a row that is not a widget, or that asked for none.
    #[inline]
    pub fn widget_min_width(&self) -> f32 {
        self.widget.as_ref().map_or(0.0, |widget| widget.min_width)
    }

    /// Makes the widget of the row, to be given the handle that closes the menu it is in. None for a row that is not a widget.
    pub fn build_widget(&self, dismiss: ContextMenuDismiss) -> Option<AnyWidget> {
        self.widget.as_ref().map(|widget| (widget.build)(dismiss))
    }

    /// Sets whether the item can be chosen.
    ///
    /// A disabled item is still drawn, dimmed, and still swallows the press
    /// that lands on it: a menu whose rows moved as they became available
    /// would be chosen wrongly.
    #[inline]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the hint written at the end of the row, dimmer than the label: the shortcut that does the same, such as `⌘V`. It is painted by
    /// a list, which gives the rows room for the widest one; a pill, which is as wide as its labels, has none.
    #[inline]
    pub fn hint(mut self, hint: impl Into<Cow<'static, str>>) -> Self {
        let hint = hint.into();
        self.hint = (!hint.is_empty()).then_some(hint);
        self
    }

    /// Sets a line between this row and the one above it, which sets the rows below apart as a group. The first row has nothing above it,
    /// and a pill, whose items stand side by side, has its own hairlines.
    #[inline]
    pub fn separated(self) -> Self {
        self.separator_before(true)
    }

    /// Sets whether there is a line between this row and the one above it: see [`separated`](Self::separated).
    #[inline]
    pub fn separator_before(mut self, separated: bool) -> Self {
        self.separated = separated;
        self
    }

    /// Sets the icon painted before the label. It is a scene that is already the colour it is to be, and is drawn a square of the style's
    /// [`icon_size`](crate::ContextMenuStyle::icon_size). A list with an icon on any row keeps the room for one on every row, so the labels
    /// stay in line.
    #[inline]
    pub fn icon(mut self, icon: Arc<SvgScene>) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The hint written at the end of the row, if it has one.
    #[inline]
    pub fn hint_text(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Whether there is a line between this row and the one above it.
    #[inline]
    pub const fn has_separator(&self) -> bool {
        self.separated
    }

    /// The icon painted before the label, if it has one.
    #[inline]
    pub fn icon_scene(&self) -> Option<&Arc<SvgScene>> {
        self.icon.as_ref()
    }

    /// The label painted for this item.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Sets what choosing this item does.
    #[inline]
    pub fn on_select(mut self, on_select: impl Fn() + 'static) -> Self {
        self.on_select = Some(Rc::new(on_select));
        self
    }

    /// Whether the item can be chosen.
    #[inline]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether choosing this item runs an action of its own.
    #[inline]
    pub fn has_action(&self) -> bool {
        self.on_select.is_some()
    }

    /// Runs this item's own action, if it has one.
    pub(crate) fn run(&self) {
        if let Some(on_select) = &self.on_select {
            on_select();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_enabled_until_it_is_told_otherwise() {
        assert!(ContextMenuItem::new("Copy").is_enabled());
        assert!(!ContextMenuItem::new("Copy").enabled(false).is_enabled());
    }

    #[test]
    fn an_owned_label_is_accepted_as_readily_as_a_static_one() {
        let dynamic = ContextMenuItem::new(format!("Search for {}", "rust"));

        assert_eq!(dynamic.label(), "Search for rust");
    }

    #[test]
    fn an_item_runs_the_action_it_was_given() {
        let ran = Rc::new(std::cell::Cell::new(0));
        let counter = Rc::clone(&ran);
        let item = ContextMenuItem::new("Copy").on_select(move || counter.set(counter.get() + 1));

        assert!(item.has_action());
        item.run();

        assert_eq!(ran.get(), 1);
    }

    #[test]
    fn an_item_without_an_action_runs_nothing_and_does_not_panic() {
        let item = ContextMenuItem::new("Copy");

        assert!(!item.has_action());
        item.run();
    }
}

#[cfg(test)]
mod parts_tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn an_item_has_no_hint_no_separator_and_no_icon_until_it_is_given_one() {
        let item = ContextMenuItem::new("Copy");
        assert_eq!(item.hint_text(), None);
        assert!(!item.has_separator());
        assert!(item.icon_scene().is_none());
    }

    #[test]
    fn a_hint_is_the_shortcut_written_at_the_end_of_the_row() {
        let item = ContextMenuItem::new("Paste").hint("\u{2318}V");
        assert_eq!(item.hint_text(), Some("\u{2318}V"));
        let owned = ContextMenuItem::new("Paste").hint(String::from("Ctrl+V"));
        assert_eq!(owned.hint_text(), Some("Ctrl+V"), "a text that is built is as good as one that is written");
        assert_eq!(ContextMenuItem::new("Paste").hint("").hint_text(), None, "no words is no hint, and leaves no room for one");
    }

    #[test]
    fn a_separator_is_a_line_before_the_row_that_sets_it_apart_from_those_above() {
        let item = ContextMenuItem::new("Rename").separated();
        assert!(item.has_separator());
        assert!(!item.separator_before(false).has_separator(), "and it can be taken away");
    }

    #[test]
    fn an_icon_is_a_scene_that_is_drawn_before_the_label() {
        let scene = Arc::new(aimer_cupid::svg::parse_svg(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6"/></svg>"#).expect("a scene"));
        let item = ContextMenuItem::new("Rename").icon(Arc::clone(&scene));
        assert!(Arc::ptr_eq(item.icon_scene().expect("an icon"), &scene));
    }

    #[test]
    fn the_new_parts_are_kept_when_an_item_is_copied_and_do_not_change_what_it_does() {
        let item = ContextMenuItem::new("Copy").hint("\u{2318}C").separated().enabled(false);
        let copy = item.clone();
        assert_eq!((copy.label(), copy.hint_text(), copy.has_separator(), copy.is_enabled()), ("Copy", Some("\u{2318}C"), true, false));
    }

    #[test]
    fn a_widget_row_has_a_widget_for_its_content_and_a_height_of_its_own() {
        let item = ContextMenuItem::widget(36.0, |_| aimer_container::ZeroSizedBox);
        assert!(item.is_widget());
        assert_eq!(item.widget_height(), Some(36.0));
        assert_eq!(item.widget_min_width(), 0.0, "it asks for no width unless it says so");
        assert_eq!(item.label(), "", "its content is the widget, and not words");
        assert!(!ContextMenuItem::new("Copy").is_widget());
        assert_eq!(ContextMenuItem::new("Copy").widget_height(), None);
    }

    #[test]
    fn a_widget_row_can_ask_for_a_width_and_be_set_apart_like_any_row() {
        let item = ContextMenuItem::widget(24.0, |_| aimer_container::ZeroSizedBox).min_width(180.0).separated();
        assert_eq!(item.widget_min_width(), 180.0);
        assert!(item.has_separator());
    }

    #[test]
    fn a_widget_is_made_each_time_it_is_asked_for_and_is_given_the_way_to_close_the_menu() {
        use std::cell::Cell;
        let made = Rc::new(Cell::new(0));
        let count = Rc::clone(&made);
        let item = ContextMenuItem::widget(24.0, move |dismiss| {
            count.set(count.get() + 1);
            // What a button in the row does: closes the menu.
            dismiss.dismiss();
            aimer_container::ZeroSizedBox
        });
        let dismiss = ContextMenuDismiss::new();
        assert!(item.build_widget(dismiss.clone()).is_some());
        assert!(dismiss.was_asked_to_dismiss(), "the widget was given this menu's handle");
        let copy = item.clone();
        assert!(copy.build_widget(ContextMenuDismiss::new()).is_some());
        assert_eq!(made.get(), 2, "a row that is built again makes its widget again: a widget is not shared between two menus");
        assert!(ContextMenuItem::new("Copy").build_widget(ContextMenuDismiss::new()).is_none());
    }

    #[test]
    fn what_a_widget_row_does_not_have_is_not_there_to_be_asked() {
        let item = ContextMenuItem::widget(24.0, |_| aimer_container::ZeroSizedBox);
        assert!(!item.has_action(), "it is not a row that is chosen: its widget takes the press");
        assert_eq!((item.hint_text(), item.icon_scene().is_none()), (None, true));
    }
}
