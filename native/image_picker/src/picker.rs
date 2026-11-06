//! The picker builder.

use aimer_file_picker::{FilePicker, PickError, PickFuture, PickedFile};

use crate::media::MediaKind;

/// Asks the user to choose photos or videos.
///
/// On iOS this is the system photo picker (`PHPickerViewController`): it runs out
/// of process, shows the whole library without asking for photo-library
/// permission, and only the items the user taps ever reach the app. On Android
/// it is the system Photo Picker where the device has one (Android 13 and up,
/// and older devices through Google Play services) and the document browser
/// otherwise. On macOS, Windows, Linux and the web — which have no photo
/// library to browse — it is the platform's open dialog filtered to images and
/// videos, so one code path serves every target.
///
/// Chosen items come back as [`PickedFile`]s, the same handle the file picker
/// returns. On iOS they are copies in the app's temporary directory; the system
/// clears that directory, so move anything to keep.
///
/// # Examples
///
/// ```no_run
/// use aimer_image_picker::{ImagePicker, MediaKind};
///
/// async fn choose_photos() -> aimer_image_picker::PickResult<()> {
///     let photos = ImagePicker::new()
///         .media(MediaKind::Images)
///         .multiple(true)
///         .limit(9)
///         .pick()
///         .await?;
///     for photo in photos {
///         let bytes = photo.read().await?;
///         println!("{}: {} bytes", photo.name(), bytes.len());
///     }
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct ImagePicker {
    pub(crate) media: MediaKind,
    pub(crate) multiple: bool,
    pub(crate) limit: Option<usize>,
}

impl ImagePicker {
    /// Creates a picker for one image.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Chooses what to offer: images, videos, or both. The default is images.
    #[inline]
    pub fn media(mut self, media: MediaKind) -> Self {
        self.media = media;
        self
    }

    /// Whether the user may choose more than one item. The default is one.
    #[inline]
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Caps a multiple selection at `limit` items, in the picker where the
    /// platform supports it and by truncating the result otherwise. `0` is
    /// rejected with [`PickError::InvalidRequest`]. It has no effect unless
    /// [`multiple`](Self::multiple) is on.
    #[inline]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// The media on offer.
    #[inline]
    pub fn media_kind(&self) -> MediaKind {
        self.media
    }

    /// The most items the user may choose: `1` unless
    /// [`multiple`](Self::multiple) is on, then the [`limit`](Self::limit) if
    /// there is one.
    #[inline]
    pub fn max_selection(&self) -> Option<usize> {
        if self.multiple { self.limit } else { Some(1) }
    }

    /// The same request as a file picker with the matching filters.
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    pub(crate) fn file_picker(&self) -> FilePicker {
        let mut picker = FilePicker::new().filters(self.media.filters()).multiple(self.multiple);
        if let Some(limit) = self.limit {
            picker = picker.limit(limit);
        }
        picker
    }

    /// Shows the picker and resolves with the chosen items.
    ///
    /// An empty vector means the user dismissed the picker; that is not an
    /// error. Dropping the future before it resolves dismisses the picker where
    /// the platform lets the app do so. Call it from a Venus task on the UI
    /// thread; in a browser, from the click or tap that asked for it.
    pub fn pick(self) -> PickFuture<Vec<PickedFile>> {
        if self.limit == Some(0) {
            return Box::pin(std::future::ready(Err(PickError::InvalidRequest(
                "the selection limit must be at least 1",
            ))));
        }
        self.pick_on_platform()
    }

    /// Shows a single-item picker and resolves with the item, or `None` if the
    /// user dismissed it.
    pub fn pick_one(self) -> PickFuture<Option<PickedFile>> {
        let pick = self.multiple(false).pick();
        Box::pin(async move { Ok(pick.await?.into_iter().next()) })
    }

    #[cfg(target_os = "ios")]
    fn pick_on_platform(self) -> PickFuture<Vec<PickedFile>> {
        crate::ios::pick(&self)
    }

    #[cfg(not(target_os = "ios"))]
    fn pick_on_platform(self) -> PickFuture<Vec<PickedFile>> {
        self.file_picker().pick()
    }
}

#[cfg(test)]
mod tests {
    use std::task::{Context, Poll, Waker};

    use super::*;

    #[test]
    fn a_picker_chooses_one_image_by_default() {
        let picker = ImagePicker::new();
        assert_eq!(picker.media_kind(), MediaKind::Images);
        assert_eq!(picker.max_selection(), Some(1));
    }

    #[test]
    fn the_limit_only_applies_to_multiple_selection() {
        assert_eq!(ImagePicker::new().limit(4).max_selection(), Some(1));
        assert_eq!(ImagePicker::new().multiple(true).limit(4).max_selection(), Some(4));
        assert_eq!(ImagePicker::new().multiple(true).max_selection(), None);
    }

    #[test]
    fn the_file_picker_carries_media_filters_and_selection() {
        let request = ImagePicker::new().media(MediaKind::ImagesAndVideos).multiple(true).limit(5).file_picker();
        let names: Vec<_> = request.filter_list().iter().map(|filter| filter.name()).collect();
        assert_eq!(names, ["Images", "Videos"]);
        assert_eq!(request.max_selection(), Some(5));
    }

    #[test]
    fn a_zero_limit_is_rejected() {
        let mut future = ImagePicker::new().multiple(true).limit(0).pick();
        match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Err(PickError::InvalidRequest(_))) => {}
            other => panic!("expected an invalid request, got {other:?}"),
        }
    }
}
