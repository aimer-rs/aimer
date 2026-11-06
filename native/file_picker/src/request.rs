//! The picker builder.

use crate::backend;
use crate::error::PickError;
use crate::filter::FileFilter;
use crate::future::{PickFuture, ready};
use crate::picked::PickedFile;

/// Asks the user to choose files in the system's own picker.
///
/// The builder only describes the request; [`pick`](Self::pick) shows the
/// picker. Each platform presents what it has — `NSOpenPanel`, the Files
/// browser, the Storage Access Framework, `IFileOpenDialog`, the desktop portal,
/// an `<input type="file">` — so the look and the available locations are the
/// platform's, and so is the permission model: nothing is requested up front and
/// only the files the user picks become readable.
///
/// # Examples
///
/// ```no_run
/// use aimer_file_picker::{FileFilter, FilePicker};
///
/// async fn attach() -> aimer_file_picker::PickResult<()> {
///     let files = FilePicker::new()
///         .title("Attach files")
///         .filter(FileFilter::new("Documents").extension("pdf").extension("txt"))
///         .multiple(true)
///         .limit(5)
///         .pick()
///         .await?;
///     for file in files {
///         println!("{}", file.name());
///     }
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct FilePicker {
    pub(crate) title: Option<String>,
    pub(crate) filters: Vec<FileFilter>,
    pub(crate) multiple: bool,
    pub(crate) limit: Option<usize>,
}

impl FilePicker {
    /// Creates a picker for one file of any type.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the title or prompt, where the platform shows one (desktop; mobile
    /// pickers ignore it).
    #[inline]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Adds a filter. With none, every file is offered; with several, the union.
    #[inline]
    pub fn filter(mut self, filter: FileFilter) -> Self {
        if !filter.is_empty() {
            self.filters.push(filter);
        }
        self
    }

    /// Adds several filters; see [`FilePicker::filter`].
    #[inline]
    pub fn filters(mut self, filters: impl IntoIterator<Item = FileFilter>) -> Self {
        for filter in filters {
            self = self.filter(filter);
        }
        self
    }

    /// Whether the user may choose more than one file. The default is one.
    #[inline]
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Caps a multiple selection at `limit` files.
    ///
    /// Platforms with a native cap (the iOS and Android photo pickers) enforce it
    /// in the picker; elsewhere the result is truncated. `0` is rejected by
    /// [`pick`](Self::pick) with [`PickError::InvalidRequest`]. It has no effect
    /// unless [`multiple`](Self::multiple) is on.
    #[inline]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// The title, if one was set.
    #[inline]
    pub fn title_text(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The filters, in the order they were added.
    #[inline]
    pub fn filter_list(&self) -> &[FileFilter] {
        &self.filters
    }

    /// The most files the user may choose: `1` unless
    /// [`multiple`](Self::multiple) is on, then the [`limit`](Self::limit) if
    /// there is one.
    #[inline]
    pub fn max_selection(&self) -> Option<usize> {
        if self.multiple { self.limit } else { Some(1) }
    }

    fn validate(&self) -> Result<(), PickError> {
        match self.limit {
            Some(0) => Err(PickError::InvalidRequest("the selection limit must be at least 1")),
            _ => Ok(()),
        }
    }

    /// Shows the picker and resolves with the chosen files.
    ///
    /// An empty vector means the user dismissed the picker; that is not an
    /// error. Dropping the future before it resolves dismisses the picker where
    /// the platform lets the app do so.
    ///
    /// Call it from a Venus task on the UI thread. In a browser the picker only
    /// opens while the page has *transient user activation*, so start the task
    /// from the click or tap that asked for it.
    pub fn pick(self) -> PickFuture<Vec<PickedFile>> {
        if let Err(error) = self.validate() {
            return ready(Err(error));
        }
        let max = self.max_selection();
        let platform = backend::pick(self);
        Box::pin(async move {
            let mut files = platform.await?;
            // Not every picker can stop the user at the cap, so enforce it.
            if let Some(max) = max {
                files.truncate(max);
            }
            Ok(files)
        })
    }

    /// Shows a single-file picker and resolves with the file, or `None` if the
    /// user dismissed it.
    pub fn pick_one(self) -> PickFuture<Option<PickedFile>> {
        let pick = self.multiple(false).pick();
        Box::pin(async move { Ok(pick.await?.into_iter().next()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picker_chooses_one_file_by_default() {
        assert_eq!(FilePicker::new().max_selection(), Some(1));
    }

    #[test]
    fn multiple_is_unbounded_until_a_limit_is_set() {
        let picker = FilePicker::new().multiple(true);
        assert_eq!(picker.max_selection(), None);
        assert_eq!(picker.limit(3).max_selection(), Some(3));
    }

    #[test]
    fn a_limit_does_not_widen_a_single_pick() {
        assert_eq!(FilePicker::new().limit(4).max_selection(), Some(1));
    }

    #[test]
    fn empty_filters_are_not_added() {
        let picker = FilePicker::new()
            .filter(FileFilter::new("nothing"))
            .filter(FileFilter::images());
        assert_eq!(picker.filter_list().len(), 1);
        assert_eq!(picker.filter_list()[0].name(), "Images");
    }

    #[test]
    fn a_zero_limit_is_rejected() {
        use std::task::{Context, Poll, Waker};

        let mut future = FilePicker::new().multiple(true).limit(0).pick();
        match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Err(PickError::InvalidRequest(_))) => {}
            other => panic!("expected an invalid request, got {other:?}"),
        }
    }
}
