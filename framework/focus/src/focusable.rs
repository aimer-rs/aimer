//! How a region takes part in keyboard focus.
//!
//! This is the policy half of a focusable region, and the only half that owes
//! nothing to the widget tree: a behavior answers two questions — may this
//! region be given focus at all, and does it ask for focus the moment it
//! appears — which is exactly what the host needs to gather it as a
//! [`FocusCandidate`](crate::FocusCandidate). The widget that carries the
//! behavior lives in `aimer_widget`, which depends on this crate; keeping the
//! policy here is what lets that dependency stay one-way.

use aimer_utils::callback::{Callback, VoidParamedFunction};

/// A callback reporting a change of focus ownership.
///
/// The argument is `true` when focus was gained and `false` when it was lost,
/// so one handler can drive both halves of an appearance — a border, a caret, a
/// highlight — without the caller having to keep two.
pub type FocusCallback = VoidParamedFunction<bool>;

/// How a region participates in keyboard focus.
///
/// Focus has exactly two dimensions worth naming, and this enumerates the
/// useful combinations of them:
///
/// | behavior | offered as a target | asks for focus on arrival |
/// |----------|---------------------|---------------------------|
/// | [`Auto`](Self::Auto)       | yes | yes |
/// | [`OnPress`](Self::OnPress) | yes | no  |
/// | [`Ignore`](Self::Ignore)   | no  | no  |
///
/// The default is [`OnPress`](Self::OnPress): a region that can be focused but
/// does not seize the keyboard from whatever already had it.
///
/// # Examples
///
/// ```
/// use aimer_focus::FocusBehavior;
///
/// // A search field at the top of a page that opens ready to be typed into.
/// assert!(FocusBehavior::Auto.is_autofocus());
///
/// // A decorative panel: focus passes straight through it.
/// assert!(!FocusBehavior::Ignore.is_focusable());
///
/// assert_eq!(FocusBehavior::default(), FocusBehavior::OnPress);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum FocusBehavior {
    /// Takes focus as soon as the region enters the tree.
    ///
    /// This is the search field of a page that opens ready to be typed into.
    /// Use it for the one region a screen is *about*: several automatic regions
    /// on one screen are a race, and the last one resolved wins.
    Auto,
    /// Becomes the focus owner when pressed, or when reached with `Tab`.
    ///
    /// The default, and what an ordinary field, button or list row wants: the
    /// keyboard follows the user rather than the tree.
    #[default]
    OnPress,
    /// Never a focus target.
    ///
    /// The region is not offered to the focus manager, so a press cannot focus
    /// it, `Tab` skips it, and even
    /// [`FocusNode::request_focus`](crate::FocusNode::request_focus) on its node
    /// has nothing to grant. Only *this* region is hidden — focusable
    /// descendants of an ignored region are offered as usual, which is what
    /// makes it a way to spell "not itself a control" rather than "inert".
    Ignore,
}

/// Decides, each time it is asked, whether a region is a focus target.
///
/// [`FocusBehavior`] is a decision taken when a region is described, which is
/// the right shape for the overwhelming majority of them. A few regions are
/// eligible only while something else is true — a selection owns the keyboard
/// exactly as long as the selection exists — and that condition turns over
/// without anything rebuilding the region, so it cannot be spelled as a fixed
/// behavior. A gate is asked afresh every time the tree gathers its focus
/// targets, and answering `false` hides the region exactly the way
/// [`FocusBehavior::Ignore`] does.
///
/// An unset gate means "no extra condition", so nothing is called for the
/// regions that do not use one.
pub type FocusGate = Callback<(), bool>;

impl FocusBehavior {
    /// Returns whether a region with this behavior is offered as a focus
    /// target.
    #[inline]
    pub const fn is_focusable(self) -> bool {
        !matches!(self, Self::Ignore)
    }

    /// Returns whether a region with this behavior asks for focus on first
    /// attachment.
    #[inline]
    pub const fn is_autofocus(self) -> bool {
        matches!(self, Self::Auto)
    }
}

#[cfg(test)]
mod tests {
    use super::FocusBehavior;

    /// The default has to be the behavior that changes nothing about who owns
    /// the keyboard: a region that stole focus merely by being built would make
    /// every list, every route change and every rebuild a focus event.
    #[test]
    fn the_default_is_focusable_without_claiming_focus() {
        let behavior = FocusBehavior::default();

        assert!(behavior.is_focusable());
        assert!(!behavior.is_autofocus());
    }

    #[test]
    fn an_automatic_region_is_a_target_that_asks_for_focus() {
        assert!(FocusBehavior::Auto.is_focusable());
        assert!(FocusBehavior::Auto.is_autofocus());
    }

    #[test]
    fn an_ignored_region_is_no_target_at_all() {
        assert!(!FocusBehavior::Ignore.is_focusable());
        assert!(!FocusBehavior::Ignore.is_autofocus());
    }
}
