//! Try `aimer_file_picker` and `aimer_image_picker` with the system pickers.
//!
//! Each button opens the platform's own picker and lists what came back: the
//! file's name, MIME type, size, and path (where the platform has one), then the
//! first bytes read through `PickedFile::read`. Dismissing a picker is reported
//! as "nothing chosen", not as a failure.
//!
//! Platform notes:
//!
//! * **Android** needs `FilePickerBridge.kt` and `PickerActivity.kt` from
//!   `native/file_picker/android/` in the app's `com.aimer` package, plus the
//!   `PickerActivity` manifest entry; without them the error line says so.
//! * **The browser** only opens a picker from a click, which a button press is.
//! * **iOS** hands back copies of the chosen files, so the path shown is inside
//!   the app's sandbox.

use std::future::Future;
use std::pin::Pin;

use aimer::file_picker::{FileFilter, FilePicker, PickError, PickedFile};
use aimer::image_picker::{ImagePicker, MediaKind};
use aimer::input::button::Button;
use aimer::style::{FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, Theme, ThemeData};
use aimer::*;

/// Lines of the log kept on screen.
const MAX_LOG_LINES: usize = 40;
/// Files larger than this are listed but not read, so the page never holds a
/// whole video in memory just to show a few bytes.
const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
/// Bytes of a file shown as hex.
const PREVIEW_BYTES: usize = 8;

/// Builds the picker page without starting an application.
pub fn file_picker_example() -> impl Widget {
    FilePickerExample::new()
}

/// Starts the picker page as a standalone Jaime application.
pub fn start_file_picker_example() {
    AimerApp::start(crate::theme::provide(FilePickerExample::new()));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    OneFile,
    SeveralFiles,
    Documents,
    Photos,
    PhotosAndVideos,
}

impl Operation {
    fn title(self) -> &'static str {
        match self {
            Self::OneFile => "Any file",
            Self::SeveralFiles => "Up to 5 files",
            Self::Documents => "PDF / text",
            Self::Photos => "Photos (up to 9)",
            Self::PhotosAndVideos => "Photos & videos",
        }
    }

    /// Shows the picker this operation stands for.
    async fn pick(self) -> Result<Vec<PickedFile>, PickError> {
        match self {
            Self::OneFile => FilePicker::new().title("Pick a file").pick().await,
            Self::SeveralFiles => FilePicker::new().title("Pick up to five files").multiple(true).limit(5).pick().await,
            Self::Documents => {
                FilePicker::new()
                    .title("Pick a document")
                    .filter(FileFilter::new("Documents").extensions(["pdf", "txt", "md"]))
                    .pick()
                    .await
            }
            Self::Photos => ImagePicker::new().multiple(true).limit(9).pick().await,
            Self::PhotosAndVideos => {
                ImagePicker::new().media(MediaKind::ImagesAndVideos).multiple(true).pick().await
            }
        }
    }
}

#[derive(StatefulWidget)]
struct FilePickerExample;

impl FilePickerExample {
    #[inline]
    fn new() -> Self {
        Self
    }
}

struct FilePickerExampleState {
    log: Vec<String>,
    busy: bool,
}

impl FilePickerExampleState {
    fn push(&mut self, line: String) {
        if self.log.len() == MAX_LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(line);
    }
}

/// The handler for one button press.
///
/// A button takes its closure on the first press, so this is built afresh on
/// every build; marking the page busy rebuilds it, which re-arms the buttons.
fn operation_callback(
    operation: Operation,
    updater: StateUpdater<FilePickerExampleState>,
) -> impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + 'static>> {
    move || {
        Box::pin(async move {
            if updater.read_state().busy {
                return;
            }
            updater.set_state(|state| {
                state.busy = true;
                state.log.clear();
            });

            let lines = match operation.pick().await {
                Ok(files) if files.is_empty() => vec!["Nothing chosen: the picker was dismissed.".to_owned()],
                Ok(files) => {
                    let mut lines = vec![format!("✓ {} chosen", files.len())];
                    for file in &files {
                        lines.extend(describe(file).await);
                    }
                    lines
                }
                Err(error) => vec![format!("✗ {error}")],
            };
            updater.set_state(move |state| {
                for line in lines {
                    state.push(line);
                }
                state.busy = false;
            });
        })
    }
}

/// What the page shows for one chosen file.
async fn describe(file: &PickedFile) -> Vec<String> {
    let mut lines = vec![format!(
        "• {} ({}, {})",
        file.name(),
        file.mime_type().unwrap_or("unknown type"),
        file.size().map_or_else(|| "unknown size".to_owned(), format_size),
    )];
    lines.push(match file.path() {
        Some(path) => format!("    path: {}", path.display()),
        None => "    path: none on this platform; read through the handle".to_owned(),
    });
    if file.size().is_some_and(|size| size > MAX_READ_BYTES) {
        lines.push("    read: skipped, larger than 16 MiB".to_owned());
        return lines;
    }
    lines.push(match file.read().await {
        Ok(bytes) => format!("    read: {} bytes, starts {}", bytes.len(), hex_prefix(&bytes)),
        Err(error) => format!("    read failed: {error}"),
    });
    lines
}

/// `1.5 MiB` style sizes.
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The first bytes as hex, with an ellipsis when there are more.
fn hex_prefix(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "(empty)".to_owned();
    }
    let shown: Vec<String> = bytes.iter().take(PREVIEW_BYTES).map(|byte| format!("{byte:02x}")).collect();
    let more = if bytes.len() > PREVIEW_BYTES { " …" } else { "" };
    format!("{}{more}", shown.join(" "))
}

impl State<FilePickerExample> for FilePickerExampleState {
    fn create(widget: FilePickerExample) -> Self {
        FilePickerExampleState {
            log: vec!["Press a button to open the system picker.".to_owned()],
            busy: false,
        }
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let operations = [
            Operation::OneFile,
            Operation::SeveralFiles,
            Operation::Documents,
            Operation::Photos,
            Operation::PhotosAndVideos,
        ];

        Scrollable::new()
            .vertical_scroll_bar(None)
            .axis(ScrollAxis::Vertical)
            .child(
                Container::new()
                    .color(theme.background_color)
                    .padding(LayoutSpacing::all(Spacing::Px(32)))
                    .child(
                        Column::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(12)))
                            .children([
                                Text::new("File & image pickers")
                                    .text_style(
                                        TextStyle::new()
                                            .font_size(28)
                                            .font_weight(FontWeight::Bold)
                                            .color(theme.on_background_color),
                                    )
                                    .boxed(),
                                Text::new(
                                    "Open the platform's own picker. Nothing is requested up front: only the \
                                     files you choose become readable.",
                                )
                                .wrapped()
                                .text_style(
                                    TextStyle::new()
                                        .font_size(16)
                                        .color(crate::theme::muted_text(&theme)),
                                )
                                .boxed(),
                                Row::new()
                                    .gaps(LayoutSpacing::all(Spacing::Px(12)))
                                    .children(operations.map(|operation| {
                                        action(
                                            operation.title(),
                                            &theme,
                                            this.busy,
                                            operation_callback(operation, this.updater()),
                                        )
                                    }))
                                    .boxed(),
                                Column::new()
                                    .children(
                                        this.log
                                            .iter()
                                            .map(|line| {
                                                Text::new(line.clone())
                                                    .wrapped()
                                                    .text_style(
                                                        TextStyle::new()
                                                            .font_size(15)
                                                            .color(theme.on_background_color),
                                                    )
                                                    .boxed()
                                            })
                                            .collect::<Vec<_>>(),
                                    )
                                    .boxed(),
                            ]),
                    ),
            )
    }
}

/// A filled button that is inert while a picker is open.
fn action<F, Fut>(label: &'static str, theme: &ThemeData, busy: bool, on_press: F) -> AnyWidget
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    Container::new()
        .color(if busy {
            crate::theme::muted_text(theme)
        } else {
            theme.primary_color
        })
        .padding(LayoutSpacing::all(Spacing::Px(12)))
        .box_child(
            Button::new()
                .disabled(busy)
                .on_press_async(on_press)
                .box_child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .color(theme.on_background_color)
                                .font_weight(FontWeight::Bold),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_builds_without_opening_a_picker() {
        let mut app = AimerApp::start_headless(crate::theme::provide(FilePickerExample::new()));

        app.pump_frames(3);
    }

    #[test]
    fn sizes_are_formatted_in_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024 * 1024), "3072.0 GiB");
    }

    #[test]
    fn hex_previews_are_bounded() {
        assert_eq!(hex_prefix(&[]), "(empty)");
        assert_eq!(hex_prefix(&[0x89, 0x50]), "89 50");
        assert_eq!(hex_prefix(&[0; 9]), "00 00 00 00 00 00 00 00 …");
    }

    #[test]
    fn the_log_keeps_only_the_most_recent_lines() {
        let mut state = FilePickerExample::new().create_state::<FilePickerExampleState>();

        for n in 0..MAX_LOG_LINES + 5 {
            state.push(format!("line {n}"));
        }

        assert_eq!(state.log.len(), MAX_LOG_LINES);
        assert_eq!(state.log.first().unwrap(), "line 5");
    }

    #[test]
    fn registering_a_callback_is_safe_before_dispatch() {
        let _callback = operation_callback(Operation::OneFile, StateUpdater::empty());
    }
}
