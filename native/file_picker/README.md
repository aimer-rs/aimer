# aimer_file_picker

The operating system's own file picker, as a future.

- **[`FilePicker`]** — a builder (`title`, `filter`, `multiple`, `limit`) whose
  `pick()` shows the platform's open dialog and resolves with the chosen files.
  `pick_one()` is the single-file shorthand.
- **[`PickedFile`]** — a handle to a chosen file: `name`, `mime_type`, `size`,
  an optional `path`, and an async `read()`. It never promises a path, because
  Android and the browser do not have one to give.
- **[`FileFilter`]** — a named set of extensions and MIME types
  (`FileFilter::images()`, `videos()`, `audio()` are presets). Each platform
  gets the vocabulary it understands; a small built-in table translates.

Dismissing the picker is not an error: `pick()` resolves with no files.
Operations return non-`Send` futures meant for a Venus task on the UI thread, and
the result is applied in that task's frame phase. The crate has no threads or
runtime of its own; blocking system calls use Venus's offload pool.

```rust,no_run
use aimer_file_picker::{FileFilter, FilePicker};

async fn import() -> aimer_file_picker::PickResult<()> {
    let files = FilePicker::new()
        .filter(FileFilter::new("Documents").extension("pdf").extension("txt"))
        .multiple(true)
        .pick()
        .await?;
    for file in files {
        let bytes = file.read().await?;
        println!("{}: {} bytes", file.name(), bytes.len());
    }
    Ok(())
}
```

Availability: macOS, iOS, Android 7+, Windows, Linux, Web

## Platforms

| Platform | Picker                                   | Chosen file is            | `path()` |
|----------|------------------------------------------|---------------------------|----------|
| macOS    | `NSOpenPanel` (sheet on the key window)  | the original              | yes      |
| iOS      | `UIDocumentPickerViewController`, `asCopy` | a copy in the app sandbox | yes      |
| Android  | Storage Access Framework / Photo Picker  | a `content://` URI        | no       |
| Windows  | `IFileOpenDialog` on the offload pool    | the original              | yes      |
| Linux    | `xdg-desktop-portal` FileChooser (D-Bus) | the original (or the portal's document mount in a sandbox) | yes |
| Web      | hidden `<input type="file">`             | a browser `File`          | no       |
| others   | none — `PickError::Unsupported`          |                           |          |

No permission is requested up front on any platform; only the files the user
picks become readable.

### Android setup

Rust cannot receive `onActivityResult`, so the app ships two small Kotlin files
from this crate's `android/` directory, in its `com.aimer` package:
`FilePickerBridge.kt` and `PickerActivity.kt`. Declare the activity:

```xml
<activity
    android:name="com.aimer.PickerActivity"
    android:exported="false"
    android:excludeFromRecents="true"
    android:theme="@android:style/Theme.Translucent.NoTitleBar" 
/>
```

Nothing is called from the app's own activity. Without the bridge a pick fails
with `PickError::BridgeMissing`; without the manifest entry, with
`PickError::Platform("PickerActivityMissing: ...")`.

When every filter is a whole image or video family (`image/*`, `video/*`) and the
device is Android 13 or newer, the system Photo Picker is shown instead of the
document browser, and `limit` is enforced in the picker. Any finer filter, or an
extension the MIME table does not know, uses the document browser (an unknown
extension offers every file rather than hide files the user asked for).

### Browser

The dialog only opens while the page has transient user activation, so start the
task from the click or tap that asked for it. Browsers that predate the `input`
`cancel` event leave the future pending when the dialog is dismissed; dropping it
cleans up.

### Dropping a pending future

Dismisses the picker on macOS, iOS and Android. Windows and the Linux portal show
a modal dialog that cannot be closed from outside; the result is discarded when
the user closes it.

## Not here yet

Choosing folders and saving files are not part of this first version.
