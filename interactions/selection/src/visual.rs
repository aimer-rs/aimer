//! What a control's `builder` closure is told about the control.

use crate::checkbox::CheckboxValue;
use crate::interaction::Flags;

/// A short-lived view of one control, passed by value to its `builder`.
///
/// It carries the controlled value and the interaction flags the control
/// paints from, so a builder can compose ordinary widgets for any state without
/// holding a reference into the control. The control keeps pointer, keyboard,
/// focus, disabled, and callback behavior around whatever the builder returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualState<'a, V> {
    value: V,
    label: Option<&'a str>,
    error: Option<&'a str>,
    flags: Flags,
    disabled: bool,
}

impl<'a, V: Copy> VisualState<'a, V> {
    #[inline]
    pub(crate) fn new(
        value: V,
        label: Option<&'a str>,
        error: Option<&'a str>,
        flags: Flags,
        disabled: bool,
    ) -> Self {
        Self {
            value,
            label,
            error,
            flags,
            disabled,
        }
    }

    /// Returns the controlled value.
    #[inline]
    pub fn value(self) -> V {
        self.value
    }

    /// Returns the label configured on the control.
    #[inline]
    pub fn label(self) -> Option<&'a str> {
        self.label
    }

    /// Returns the validation error configured on the control.
    #[inline]
    pub fn error(self) -> Option<&'a str> {
        self.error
    }

    /// Returns whether the control is disabled.
    #[inline]
    pub fn is_disabled(self) -> bool {
        self.disabled
    }

    /// Returns whether the pointer is over the control.
    #[inline]
    pub fn is_hovered(self) -> bool {
        self.flags.hovered
    }

    /// Returns whether a press is in progress on the control.
    #[inline]
    pub fn is_pressed(self) -> bool {
        self.flags.pressed
    }

    /// Returns whether the control owns keyboard focus.
    #[inline]
    pub fn is_focused(self) -> bool {
        self.flags.focused
    }

    /// Returns whether a focus ring should be drawn, which is the case for
    /// keyboard focus but not for focus gained by a click.
    #[inline]
    pub fn is_focus_visible(self) -> bool {
        self.flags.focus_visible()
    }
}

impl VisualState<'_, CheckboxValue> {
    /// Returns whether the checkbox is checked.
    #[inline]
    pub fn is_checked(self) -> bool {
        self.value == CheckboxValue::Checked
    }

    /// Returns whether the checkbox is indeterminate.
    #[inline]
    pub fn is_indeterminate(self) -> bool {
        self.value == CheckboxValue::Indeterminate
    }
}

impl VisualState<'_, bool> {
    /// Returns whether the switch is on, or the radio selected.
    #[inline]
    pub fn is_on(self) -> bool {
        self.value
    }
}

/// The visual state a [`crate::Checkbox`] `builder` receives.
pub type CheckboxVisualState<'a> = VisualState<'a, CheckboxValue>;

/// The visual state a [`crate::Switch`] or [`crate::Radio`] `builder` receives.
pub type ToggleVisualState<'a> = VisualState<'a, bool>;

/// A short-lived view of one option row, passed to a list control's
/// `option_builder`.
///
/// The row's pointer and keyboard behavior stays with the control; the builder
/// only decides what the row looks like.
#[derive(Debug)]
pub struct OptionState<'a, T> {
    option: &'a crate::ChoiceOption<T>,
    selected: bool,
    highlighted: bool,
}

impl<T> Clone for OptionState<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for OptionState<'_, T> {}

impl<'a, T> OptionState<'a, T> {
    #[inline]
    pub(crate) fn new(option: &'a crate::ChoiceOption<T>, selected: bool, highlighted: bool) -> Self {
        Self {
            option,
            selected,
            highlighted,
        }
    }

    /// Returns the option this row shows.
    #[inline]
    pub fn option(self) -> &'a crate::ChoiceOption<T> {
        self.option
    }

    /// Returns the option's display label.
    #[inline]
    pub fn label(self) -> &'a str {
        self.option.label()
    }

    /// Returns whether this option is the controlled selection.
    #[inline]
    pub fn is_selected(self) -> bool {
        self.selected
    }

    /// Returns whether this row is the keyboard or pointer highlight.
    #[inline]
    pub fn is_highlighted(self) -> bool {
        self.highlighted
    }

    /// Returns whether the option cannot be chosen.
    #[inline]
    pub fn is_disabled(self) -> bool {
        self.option.is_disabled()
    }
}

/// A short-lived view of a `Select` or `Autocomplete` trigger, passed to its
/// `builder`.
///
/// The control keeps opening, closing, keyboard, and focus behavior around
/// whatever the builder returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerState<'a> {
    selected_label: Option<&'a str>,
    placeholder: &'a str,
    label: Option<&'a str>,
    error: Option<&'a str>,
    open: bool,
    flags: Flags,
    disabled: bool,
}

impl<'a> TriggerState<'a> {
    #[inline]
    pub(crate) fn new(
        selected_label: Option<&'a str>,
        placeholder: &'a str,
        label: Option<&'a str>,
        error: Option<&'a str>,
        open: bool,
        flags: Flags,
        disabled: bool,
    ) -> Self {
        Self {
            selected_label,
            placeholder,
            label,
            error,
            open,
            flags,
            disabled,
        }
    }

    /// Returns the label of the selected option, if there is one.
    #[inline]
    pub fn selected_label(self) -> Option<&'a str> {
        self.selected_label
    }

    /// Returns the placeholder shown while nothing is selected.
    #[inline]
    pub fn placeholder(self) -> &'a str {
        self.placeholder
    }

    /// Returns what the trigger shows by default: the selected label, or the
    /// placeholder when nothing is selected.
    #[inline]
    pub fn text(self) -> &'a str {
        self.selected_label.unwrap_or(self.placeholder)
    }

    /// Returns the label configured on the control.
    #[inline]
    pub fn label(self) -> Option<&'a str> {
        self.label
    }

    /// Returns the validation error configured on the control.
    #[inline]
    pub fn error(self) -> Option<&'a str> {
        self.error
    }

    /// Returns whether the popup is open.
    #[inline]
    pub fn is_open(self) -> bool {
        self.open
    }

    /// Returns whether the control is disabled.
    #[inline]
    pub fn is_disabled(self) -> bool {
        self.disabled
    }

    /// Returns whether the pointer is over the trigger.
    #[inline]
    pub fn is_hovered(self) -> bool {
        self.flags.hovered
    }

    /// Returns whether a press is in progress on the trigger.
    #[inline]
    pub fn is_pressed(self) -> bool {
        self.flags.pressed
    }

    /// Returns whether the trigger owns keyboard focus.
    #[inline]
    pub fn is_focused(self) -> bool {
        self.flags.focused
    }

    /// Returns whether a focus ring should be drawn.
    #[inline]
    pub fn is_focus_visible(self) -> bool {
        self.flags.focus_visible()
    }
}
