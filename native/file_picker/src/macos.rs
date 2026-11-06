//! macOS: `NSOpenPanel`.
//!
//! The panel is shown asynchronously — as a sheet on the key window when there is
//! one, modeless otherwise — rather than with `runModal`, which would spin a
//! nested run loop inside the Venus task that is polling this future. The
//! completion block runs on the main thread, copies the chosen paths out, and
//! completes a Venus [`Completion`]; the awaiting task applies them in its own
//! frame phase.
//!
//! Dropping the future before the panel closes cancels the panel.

use std::cell::Cell;
use std::path::PathBuf;

use aimer_venus::{Completer, completion};
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSModalResponse, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSArray, NSString};

use crate::apple_types::content_types;
use crate::error::PickError;
use crate::future::{PickFuture, ready};
use crate::picked::PickedFile;
use crate::request::FilePicker;

pub(crate) fn pick(request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    let Some(mtm) = MainThreadMarker::new() else {
        return ready(Err(PickError::Platform(
            "the open panel can only be shown from the main thread".into(),
        )));
    };

    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(request.multiple);
    panel.setResolvesAliases(true);
    if let Some(title) = &request.title {
        panel.setMessage(Some(&NSString::from_str(title)));
    }
    let types = content_types(&request.filters);
    if !types.is_empty() {
        panel.setAllowedContentTypes(&NSArray::from_retained_slice(&types));
    }

    let (completer, completion) = completion::<Vec<PathBuf>>();
    let handler = {
        let panel = panel.clone();
        // `Fn`, not `FnOnce`: AppKit calls the block once, but the type says
        // otherwise, so the completer is taken out of a cell.
        let completer = Cell::new(Some(completer));
        RcBlock::new(move |response: NSModalResponse| {
            let Some(completer): Option<Completer<Vec<PathBuf>>> = completer.take() else {
                return;
            };
            let mut paths = Vec::new();
            if response == NSModalResponseOK {
                for url in panel.URLs().iter() {
                    if let Some(path) = url.path() {
                        paths.push(PathBuf::from(path.to_string()));
                    }
                }
            }
            completer.complete(paths);
        })
    };

    let app = NSApplication::sharedApplication(mtm);
    match app.keyWindow() {
        Some(window) => panel.beginSheetModalForWindow_completionHandler(&window, &handler),
        None => panel.beginWithCompletionHandler(&handler),
    }

    let mut open = OpenPanel {
        panel,
        finished: false,
    };
    Box::pin(async move {
        let paths = completion.await.map_err(|_| PickError::Interrupted)?;
        open.finish();
        Ok(paths.into_iter().map(PickedFile::from_path).collect())
    })
}

/// The panel while it is up; cancels it if the future is dropped first.
struct OpenPanel {
    panel: Retained<NSOpenPanel>,
    finished: bool,
}

impl OpenPanel {
    /// Marks the panel as closed. A method rather than a field write, so the
    /// future captures the whole guard and not just the flag: with only the flag
    /// captured the guard would drop, and cancel the panel, as `pick` returns.
    fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for OpenPanel {
    fn drop(&mut self) {
        if !self.finished {
            // SAFETY: `cancel:` takes an optional sender, which may be nil.
            unsafe { self.panel.cancel(None) };
        }
    }
}
