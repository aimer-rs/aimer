//! One verb of a context menu.

use std::borrow::Cow;
use std::rc::Rc;

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
