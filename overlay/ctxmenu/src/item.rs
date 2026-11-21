//! One verb of a context menu.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::svg::SvgScene;

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
        }
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
}
