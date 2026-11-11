//! A tray icon standing in for a live activity.
//!
//! Windows and Linux have no live activity surface, so an application that
//! opts in gets a tray icon whose tooltip carries the text of what is running.
//! The icon exists only while at least one activity does.

use std::cell::RefCell;

use aimer_tray::{Tray, TrayError, TrayHandle, TrayIcon};

use super::Backend;
use crate::{LiveActivity, LiveActivityError, LiveActivityId};

/// The longest tooltip, in characters. The Windows notification area keeps 127
/// characters and a terminator; the other platforms take at least that much.
const TOOLTIP_CAPACITY: usize = 127;

/// The running activities, oldest first, each as the line shown for it.
#[derive(Default)]
struct Board {
    lines: Vec<(LiveActivityId, String)>,
}

impl Board {
    /// Sets the line of `id`, keeping its place when it already runs.
    fn upsert(&mut self, id: LiveActivityId, line: String) {
        match self.lines.iter_mut().find(|(existing, _)| *existing == id) {
            Some(entry) => entry.1 = line,
            None => self.lines.push((id, line)),
        }
    }

    /// Removes `id`; `false` when it was not running.
    fn remove(&mut self, id: LiveActivityId) -> bool {
        let before = self.lines.len();
        self.lines.retain(|(existing, _)| *existing != id);
        self.lines.len() != before
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// One line per activity, cut to [`TOOLTIP_CAPACITY`] characters with an
    /// ellipsis when it does not fit.
    fn tooltip(&self) -> String {
        let mut text = String::new();
        for (index, (_, line)) in self.lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(line);
        }
        if text.chars().count() <= TOOLTIP_CAPACITY {
            return text;
        }
        let mut cut: String = text.chars().take(TOOLTIP_CAPACITY - 1).collect();
        cut.push('\u{2026}');
        cut
    }
}

struct State {
    board: Board,
    handle: Option<TrayHandle>,
}

pub(super) struct TrayBackend {
    icon: TrayIcon,
    state: RefCell<State>,
}

impl TrayBackend {
    pub(super) fn new(icon: TrayIcon) -> Self {
        Self { icon, state: RefCell::new(State { board: Board::default(), handle: None }) }
    }
}

fn from_tray(error: TrayError) -> LiveActivityError {
    match error {
        TrayError::Unsupported => LiveActivityError::Unsupported,
        TrayError::MissingIcon => LiveActivityError::Platform("the tray icon is missing".into()),
        TrayError::Platform(reason) => LiveActivityError::Platform(reason),
    }
}

impl Backend for TrayBackend {
    fn show(&self, activity: &LiveActivity) -> Result<(), LiveActivityError> {
        let mut state = self.state.borrow_mut();
        state.board.upsert(activity.id, activity.summary());
        let tooltip = state.board.tooltip();
        match state.handle.as_mut() {
            Some(handle) => handle.set_tooltip(&tooltip),
            None => {
                let built = Tray::new().icon(self.icon.clone()).tooltip(tooltip).build();
                match built {
                    Ok(handle) => state.handle = Some(handle),
                    Err(error) => {
                        // Nothing is on screen, so nothing may be remembered.
                        state.board.remove(activity.id);
                        return Err(from_tray(error));
                    }
                }
            }
        }
        Ok(())
    }

    fn end(&self, id: LiveActivityId) -> Result<(), LiveActivityError> {
        let mut state = self.state.borrow_mut();
        if !state.board.remove(id) {
            return Ok(());
        }
        if state.board.is_empty() {
            // Dropping the handle removes the icon.
            state.handle = None;
        } else {
            let tooltip = state.board.tooltip();
            if let Some(handle) = state.handle.as_mut() {
                handle.set_tooltip(&tooltip);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: LiveActivityId = LiveActivityId(1);
    const B: LiveActivityId = LiveActivityId(2);

    #[test]
    fn upsert_keeps_the_place_of_a_running_activity() {
        let mut board = Board::default();
        board.upsert(A, "first".into());
        board.upsert(B, "second".into());
        board.upsert(A, "first again".into());
        assert_eq!(board.tooltip(), "first again\nsecond");
    }

    #[test]
    fn remove_reports_whether_it_was_running() {
        let mut board = Board::default();
        board.upsert(A, "x".into());
        assert!(board.remove(A));
        assert!(!board.remove(A));
        assert!(board.is_empty());
        assert_eq!(board.tooltip(), "");
    }

    #[test]
    fn a_long_tooltip_is_cut_with_an_ellipsis() {
        let mut board = Board::default();
        board.upsert(A, "a".repeat(200));
        let tooltip = board.tooltip();
        assert_eq!(tooltip.chars().count(), TOOLTIP_CAPACITY);
        assert!(tooltip.ends_with('\u{2026}'));
    }

    #[test]
    fn a_tooltip_at_the_limit_is_kept_whole() {
        let mut board = Board::default();
        board.upsert(A, "\u{e9}".repeat(TOOLTIP_CAPACITY));
        assert_eq!(board.tooltip().chars().count(), TOOLTIP_CAPACITY);
        assert!(!board.tooltip().ends_with('\u{2026}'));
    }

    #[test]
    fn tray_errors_map_to_live_activity_errors() {
        assert_eq!(from_tray(TrayError::Unsupported), LiveActivityError::Unsupported);
        assert_eq!(
            from_tray(TrayError::Platform("no watcher".into())),
            LiveActivityError::Platform("no watcher".into())
        );
    }
}
