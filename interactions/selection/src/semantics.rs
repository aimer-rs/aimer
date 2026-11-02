use crate::checkbox::CheckboxValue;
use crate::common::Common;

/// The role a selection control plays for assistive technology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticRole {
    /// A binary or tri-state checkbox.
    Checkbox,
    /// A binary switch.
    Switch,
    /// One radio option.
    Radio,
    /// A group of mutually exclusive radio options.
    RadioGroup,
    /// A select control.
    Select,
    /// A queryable select control.
    Autocomplete,
}

/// A platform-neutral description of a selection control.
///
/// It is independent of the renderer, so `aimer_accessibility` can adapt it
/// when that crate is present. It describes configuration only; transient
/// hover and press are not accessibility state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionSemantics {
    role: SemanticRole,
    label: Option<String>,
    enabled: bool,
    checked: Option<CheckboxValue>,
    selected: Option<bool>,
    busy: bool,
    error: Option<String>,
}

impl SelectionSemantics {
    pub(crate) fn new(role: SemanticRole, common: &Common) -> Self {
        Self {
            role,
            label: common.label.clone(),
            enabled: !common.disabled,
            checked: None,
            selected: None,
            busy: false,
            error: common.error.clone(),
        }
    }

    pub(crate) fn checked(mut self, checked: CheckboxValue) -> Self {
        self.checked = Some(checked);
        self
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub(crate) fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Returns the semantic role.
    #[inline]
    pub fn role(&self) -> SemanticRole {
        self.role
    }

    /// Returns the accessible label, if one was supplied.
    #[inline]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Returns whether the control accepts input.
    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Returns the checked or tri-state value, when the role supports it.
    #[inline]
    pub fn checked_value(&self) -> Option<CheckboxValue> {
        self.checked
    }

    /// Returns the selected state, when the role supports it.
    #[inline]
    pub fn is_selected(&self) -> Option<bool> {
        self.selected
    }

    /// Returns whether the control is waiting for asynchronous options.
    #[inline]
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Returns the validation error, if any.
    #[inline]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
