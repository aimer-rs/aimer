use crate::request::{self, Request, WindowCommand};

/// Names one window of the application, for as long as the application runs.
///
/// A window is asked for with [`NewWindow`](crate::NewWindow) or the
/// [`Window`](crate::Window) widget, and an id is how it is told what to do
/// afterwards. Ids are plain numbers: cheap to copy, to keep, and to compare. An
/// id never names a second window, so one that outlived its window is harmless
/// — everything done through it is ignored.
///
/// Everything here is a request. The event loop carries it out the next time it
/// runs, so a call returns before the window has changed. Call these on the UI
/// thread.
///
/// # Examples
///
/// ```no_run
/// use aimer_widget::{Widget, base::BuildContext};
/// use aimer_window::{NewWindow, WindowId};
///
/// fn show_inspector(ctx: &BuildContext) -> WindowId {
///     let inspector = NewWindow::new()
///         .title("Inspector")
///         .size(360, 480)
///         .inherit(ctx)
///         .builder(|_ctx| aimer_container::ZeroSizedBox)
///         .open();
///
///     inspector.set_title("Inspector - 3 items");
///     inspector
/// }
///
/// // Later, from anywhere on the UI thread:
/// # fn later(inspector: WindowId) {
/// if inspector.is_open() {
///     inspector.focus();
/// }
/// inspector.close();
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// The window the application started with.
    pub const MAIN: Self = Self(0);

    #[doc(hidden)]
    #[inline]
    pub(crate) const fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Whether the window exists right now.
    ///
    /// `false` before the event loop has created a window that was just asked
    /// for, and again once it has closed.
    pub fn is_open(self) -> bool {
        request::is_open(self)
    }

    /// Closes the window.
    ///
    /// If it was the last one, or the main one, the application's
    /// [`ExitPolicy`](crate::ExitPolicy) decides whether it ends.
    pub fn close(self) {
        self.command(WindowCommand::Close);
    }

    /// Brings the window to the front and gives it the keyboard.
    pub fn focus(self) {
        self.command(WindowCommand::Focus);
    }

    /// Replaces the title in the window's title bar.
    pub fn set_title(self, title: impl Into<String>) {
        self.command(WindowCommand::SetTitle(title.into()));
    }

    /// Asks for the window's content to be `width` by `height` logical pixels.
    ///
    /// The platform may refuse, for a window that is maximized or has a fixed
    /// size.
    pub fn resize(self, width: u32, height: u32) {
        self.command(WindowCommand::Resize(width, height));
    }

    /// Minimizes the window, or restores it.
    pub fn set_minimized(self, minimized: bool) {
        self.command(WindowCommand::SetMinimized(minimized));
    }

    /// Maximizes the window, or restores it.
    pub fn set_maximized(self, maximized: bool) {
        self.command(WindowCommand::SetMaximized(maximized));
    }

    fn command(self, command: WindowCommand) {
        request::push(Request::Command(self, command));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<(WindowId, WindowCommand)> {
        request::drain()
            .into_iter()
            .map(|request| match request {
                Request::Command(id, command) => (id, command),
                Request::Open(_) => panic!("unexpected open request"),
            })
            .collect()
    }

    #[test]
    fn every_operation_becomes_one_command_for_that_window() {
        request::drain();
        let id = request::next_id();

        id.close();
        id.focus();
        id.set_title("Title");
        id.resize(300, 200);
        id.set_minimized(true);
        id.set_maximized(false);

        assert_eq!(
            commands(),
            [
                (id, WindowCommand::Close),
                (id, WindowCommand::Focus),
                (id, WindowCommand::SetTitle("Title".to_owned())),
                (id, WindowCommand::Resize(300, 200)),
                (id, WindowCommand::SetMinimized(true)),
                (id, WindowCommand::SetMaximized(false)),
            ]
        );
    }

    #[test]
    fn ids_are_distinct_and_never_the_main_windows() {
        let first = request::next_id();
        let second = request::next_id();

        assert_ne!(first, second);
        assert_ne!(first, WindowId::MAIN);
        assert_ne!(second, WindowId::MAIN);
    }

    #[test]
    fn a_window_is_open_between_the_host_marking_it_so_and_gone() {
        let id = request::next_id();
        assert!(!id.is_open(), "a window just asked for does not exist yet");

        request::mark_open(id);
        assert!(id.is_open());

        request::mark_closed(id);
        assert!(!id.is_open());
    }
}
