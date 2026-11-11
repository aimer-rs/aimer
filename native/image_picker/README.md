# aimer_image_picker

Pick photos and videos.

- **[`ImagePicker`]** — a builder (`media`, `multiple`, `limit`) whose `pick()`
  resolves with the chosen items as [`PickedFile`]s, the handle
  `aimer_file_picker` returns (`name`, `mime_type`, `size`, `path`, async
  `read()`). `pick_one()` is the single-item shorthand.
- **[`MediaKind`]** — `Images` (default), `Videos`, or `ImagesAndVideos`.

Dismissing the picker is not an error: `pick()` resolves with no items.

```rust,no_run
use aimer_image_picker::{ImagePicker, MediaKind};

async fn choose() -> aimer_image_picker::PickResult<()> {
    let photos = ImagePicker::new()
        .media(MediaKind::Images)
        .multiple(true)
        .limit(9)
        .pick()
        .await?;
    for photo in photos {
        println!("{} ({:?})", photo.name(), photo.mime_type());
    }
    Ok(())
}
```

Availability: macOS, iOS 14+, Android 7+, Windows, Linux, Web

## Platforms

| Platform | Picker |
|----------|--------|
| iOS      | `PHPickerViewController` — out of process, no photo-library permission, selection limit and image/video filter enforced by the picker. Items are copies in the app's temporary directory. |
| Android  | the system Photo Picker (Android 13+; older devices through Google Play services), else the document browser — via `aimer_file_picker`'s bridge, see its README for the manifest entry |
| macOS, Windows, Linux, web | the platform's open dialog filtered to images and videos (`aimer_file_picker`) — these have no photo library to browse |

Because everything except iOS is the file picker with an image filter, the
failure modes, threading and web activation rules are the file picker's.

## Not here yet

Capturing a photo or video with the camera is not part of this first version: it
needs a capture UI and camera-permission strings, and is better as its own step.
