//! Files the app is opened with.
//!
//! Drop a file on the app's icon, choose Jaime in "Open With", or double-click a file
//! of a kind `Aimer.toml` declares under `[[documents]]`, and it arrives here. The
//! handler is installed by [`install_open_files`] before the app runs, so the file
//! that *launched* the app is seen too; the log lives outside the page for the same
//! reason as the shortcuts page's: the file belongs to the application, not to
//! whichever page is showing.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use aimer::events::file_open;
use aimer::style::{FontWeight, LayoutSpacing, Spacing, TextStyle, Theme, ThemeData};
use aimer::*;

/// Files kept in the log.
const LOG_LINES: usize = 20;

aimer_tls::thread_local! {
    static OPENED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// The page that is showing the log, if one is.
    static PAGE: Cell<Option<StateUpdater<OpenedFilesExampleState>>> = const { Cell::new(None) };
}

/// Notes `paths` as opened and redraws the page showing them.
fn record(paths: &[PathBuf]) {
    OPENED.with_borrow_mut(|log| {
        for path in paths {
            if log.len() == LOG_LINES {
                log.remove(0);
            }
            log.push(describe(path));
        }
    });
    // A page that has gone away rejects the update, which is the right outcome.
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

/// What the page shows for a path: its name with what it is and how big, and the
/// folder it is in on a line of its own, since a path is one long word that does
/// not wrap.
fn describe(path: &Path) -> (String, String) {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    let what = match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => "a folder".to_owned(),
        Ok(meta) => format!("{} bytes", meta.len()),
        Err(error) => format!("cannot read: {error}"),
    };
    let folder = path.parent().map_or_else(String::new, |parent| parent.display().to_string());
    (format!("{name}  ({what})"), folder)
}

/// Installs the handler for the files the app is opened with. Call it on the UI
/// thread before the app runs: the file that launches the app is delivered as soon
/// as the loop resumes, and a handler registered later would find it waiting but
/// never be offered it.
pub fn install_open_files() {
    // Lasts as long as the app does.
    std::mem::forget(file_open::register(|paths: &[PathBuf]| {
        record(paths);
        true
    }));
}

/// Builds the page without starting an application.
pub fn opened_files_example() -> impl Widget {
    OpenedFilesExample
}

#[derive(StatefulWidget)]
struct OpenedFilesExample;

struct OpenedFilesExampleState;

impl State<OpenedFilesExample> for OpenedFilesExampleState {
    fn create(widget: OpenedFilesExample) -> Self {
        OpenedFilesExampleState
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        PAGE.set(Some(this.updater()));
        let theme = ThemeData::copied(ctx);
        let line = |text: String, color, size| {
            Text::new(text).wrapped().text_style(TextStyle::new().font_size(size).color(color)).boxed()
        };
        let muted = crate::theme::muted_text(&theme);
        let opened: Vec<AnyWidget> = OPENED.with_borrow(|log| {
            if log.is_empty() {
                vec![line("Nothing yet. Drop a file on the app's icon.".to_owned(), muted, 15)]
            } else {
                log.iter()
                    .rev()
                    .flat_map(|(what, folder)| {
                        [line(what.clone(), theme.on_background_color, 16), line(folder.clone(), muted, 12)]
                    })
                    .collect()
            }
        });

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::all(Spacing::Px(12)))
                    .children(
                        [
                            Text::new("Opened files")
                                .text_style(
                                    TextStyle::new()
                                        .font_size(28)
                                        .font_weight(FontWeight::Bold)
                                        .color(theme.on_background_color),
                                )
                                .boxed(),
                            line(
                                "Drop a file on the app's icon, choose it in \"Open With\", or open a .txt, \
                                 .md or .rs file with it. Each arrives here."
                                    .to_owned(),
                                crate::theme::muted_text(&theme),
                                16,
                            ),
                        ]
                        .into_iter()
                        .chain(opened)
                        .collect::<Vec<_>>(),
                    ),
            )
    }
}
