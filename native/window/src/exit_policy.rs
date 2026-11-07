/// What closing a window does to the application.
///
/// The default, [`ExitPolicy::LastWindowClosed`], is what a single-window
/// application always did: when its window closes, it ends.
///
/// # Examples
///
/// ```
/// use aimer_window::ExitPolicy;
///
/// // A menu-bar application lives on after its last window is closed.
/// let policy = ExitPolicy::Never;
/// assert_ne!(policy, ExitPolicy::default());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExitPolicy {
    /// The application ends when its last window has closed.
    #[default]
    LastWindowClosed,
    /// The application ends when its first window — the one it started with —
    /// closes, and closes every other window with it.
    MainWindowClosed,
    /// Closing windows never ends the application, not even the last one. It
    /// ends when it is asked to ([`quit`](crate::quit)) or the process is told
    /// to stop.
    Never,
}

/// What the host does after one or more windows finished closing.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterClose {
    /// Nothing: other windows carry on.
    Nothing,
    /// Close every window that is left; the application is ending.
    CloseRemaining,
    /// End the event loop.
    Exit,
}

impl ExitPolicy {
    /// Decides what follows windows closing, given how many are left and
    /// whether the main window was among those that closed.
    #[doc(hidden)]
    pub fn after_close(self, remaining: usize, main_closed: bool) -> AfterClose {
        match self {
            Self::Never => AfterClose::Nothing,
            Self::LastWindowClosed if remaining == 0 => AfterClose::Exit,
            Self::LastWindowClosed => AfterClose::Nothing,
            Self::MainWindowClosed if remaining == 0 => AfterClose::Exit,
            Self::MainWindowClosed if main_closed => AfterClose::CloseRemaining,
            Self::MainWindowClosed => AfterClose::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_policy_ends_the_application_with_its_last_window() {
        assert_eq!(ExitPolicy::default(), ExitPolicy::LastWindowClosed);
        assert_eq!(ExitPolicy::default().after_close(0, true), AfterClose::Exit);
        assert_eq!(
            ExitPolicy::default().after_close(1, false),
            AfterClose::Nothing
        );
    }

    #[test]
    fn closing_the_main_window_alone_does_not_end_the_last_window_policy() {
        assert_eq!(
            ExitPolicy::LastWindowClosed.after_close(2, true),
            AfterClose::Nothing
        );
    }

    #[test]
    fn the_main_window_policy_takes_the_others_with_it() {
        assert_eq!(
            ExitPolicy::MainWindowClosed.after_close(2, true),
            AfterClose::CloseRemaining
        );
        assert_eq!(
            ExitPolicy::MainWindowClosed.after_close(0, true),
            AfterClose::Exit
        );
        assert_eq!(
            ExitPolicy::MainWindowClosed.after_close(2, false),
            AfterClose::Nothing,
            "closing a secondary window ends nothing"
        );
        assert_eq!(
            ExitPolicy::MainWindowClosed.after_close(0, false),
            AfterClose::Exit,
            "nothing is left to keep the application alive"
        );
    }

    #[test]
    fn a_keep_running_application_outlives_every_window() {
        for (remaining, main_closed) in [(0, true), (0, false), (3, true), (3, false)] {
            assert_eq!(
                ExitPolicy::Never.after_close(remaining, main_closed),
                AfterClose::Nothing
            );
        }
    }
}
