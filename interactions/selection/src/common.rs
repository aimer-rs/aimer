//! Configuration every control shares, and the builder methods that set it.

use aimer_widget::FocusNode;

/// Label, disabled flag, validation error, and focus node of one control.
#[derive(Default)]
pub(crate) struct Common {
    pub(crate) label: Option<String>,
    pub(crate) disabled: bool,
    pub(crate) error: Option<String>,
    pub(crate) focus_node: Option<FocusNode>,
}

/// Adds the shared builder methods to a control whose `common` field is a
/// [`Common`].
macro_rules! common_setters {
    () => {
        /// Sets the label drawn beside the control and exposed to assistive
        /// technology.
        #[inline]
        pub fn label(mut self, label: impl Into<String>) -> Self {
            self.common.label = Some(label.into());
            self
        }

        /// Sets whether the control ignores pointer and keyboard input.
        ///
        /// A disabled control takes no focus and proposes no value.
        #[inline]
        pub fn disabled(mut self, disabled: bool) -> Self {
            self.common.disabled = disabled;
            self
        }

        /// Shows `error` under the control and marks it invalid.
        #[inline]
        pub fn error(mut self, error: impl Into<String>) -> Self {
            self.common.error = Some(error.into());
            self
        }

        /// Uses `node` as the control's focus target, so the application can
        /// move focus to it or ask whether it has focus.
        #[inline]
        pub fn focus_node(mut self, node: aimer_widget::FocusNode) -> Self {
            self.common.focus_node = Some(node);
            self
        }
    };
}

pub(crate) use common_setters;
