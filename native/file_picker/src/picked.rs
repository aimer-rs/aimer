//! A file the user chose.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::PickError;
use crate::future::{PickFuture, ready};
use crate::mime::mime_for_name;

/// Where the bytes of a picked file live. Only some platforms have a path.
pub(crate) enum Source {
    /// A file system path: desktop, and iOS where the system hands back a copy
    /// inside the app's sandbox.
    Path(PathBuf),
    /// A `content://` URI from Android's Storage Access Framework or Photo
    /// Picker. It has no path; the bridge reads it through `ContentResolver`.
    #[cfg(target_os = "android")]
    Uri(String),
    /// A browser `File`. It has no path; the browser reads it.
    #[cfg(target_arch = "wasm32")]
    Blob(web_sys::File),
}

/// A handle to a file the user chose in a system picker.
///
/// What the platform grants is access to *that file*, not a path: on Android it
/// is a content URI, in a browser a `File` object, on iOS a copy in the app's
/// sandbox. So the handle describes the file ([`name`](Self::name),
/// [`mime_type`](Self::mime_type), [`size`](Self::size)) and reads it
/// ([`read`](Self::read)) without ever promising a path. [`path`](Self::path) is
/// there for the platforms that do have one.
///
/// Access to a picked file lasts as long as the app's process.
///
/// # Examples
///
/// ```no_run
/// use aimer_file_picker::{FileFilter, FilePicker};
///
/// async fn import() -> aimer_file_picker::PickResult<()> {
///     if let Some(file) = FilePicker::new()
///         .filter(FileFilter::new("Notes").extension("txt"))
///         .pick_one()
///         .await?
///     {
///         let bytes = file.read().await?;
///         println!("{} is {} bytes", file.name(), bytes.len());
///     }
///     Ok(())
/// }
/// ```
pub struct PickedFile {
    name: String,
    mime_type: Option<String>,
    size: Option<u64>,
    source: Source,
}

impl PickedFile {
    /// Describes the file at `path`.
    ///
    /// The name is the path's file name, the MIME type comes from its extension
    /// when the crate knows it, and the size from the file system when the file
    /// can be inspected. Nothing is opened.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(&path).ok().map(|meta| meta.len());
        Self {
            mime_type: mime_for_name(&name).map(str::to_owned),
            name,
            size,
            source: Source::Path(path),
        }
    }

    /// Builds a handle from what a platform reported. Only the platforms whose
    /// files have no path (Android, the browser) build handles this way.
    #[cfg_attr(not(any(target_os = "android", target_arch = "wasm32", test)), allow(dead_code))]
    pub(crate) fn from_parts(
        name: String,
        mime_type: Option<String>,
        size: Option<u64>,
        source: Source,
    ) -> Self {
        let mime_type = mime_type
            .filter(|mime| !mime.is_empty())
            .or_else(|| mime_for_name(&name).map(str::to_owned));
        Self {
            name,
            mime_type,
            size,
            source,
        }
    }

    /// The file's display name, with its extension.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The MIME type the platform reported, or one guessed from the extension.
    #[inline]
    pub fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }

    /// The size in bytes, when the platform said.
    #[inline]
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// The file system path, on the platforms where the file has one.
    ///
    /// `None` on Android and in a browser. On iOS the path is a copy owned by
    /// the app, not the user's original.
    #[inline]
    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            Source::Path(path) => Some(path),
            #[cfg(any(target_os = "android", target_arch = "wasm32"))]
            _ => None,
        }
    }

    /// Reads the whole file.
    ///
    /// The read happens off the UI thread where there is a thread to run it on
    /// (Venus's offload pool), and in the browser through `Blob.arrayBuffer`.
    /// Outside a Venus runtime it reads in place, which is only suitable for
    /// tests and tools.
    pub fn read(&self) -> PickFuture<Vec<u8>> {
        match &self.source {
            Source::Path(path) => read_path(path.clone()),
            #[cfg(target_os = "android")]
            Source::Uri(uri) => crate::android::read(uri.clone()),
            #[cfg(target_arch = "wasm32")]
            Source::Blob(file) => crate::web::read(file.clone()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_path(path: PathBuf) -> PickFuture<Vec<u8>> {
    // Clone only when a pool exists: the fallback reads in place.
    match aimer_venus::spawn_blocking({
        let path = path.clone();
        move || std::fs::read(path)
    }) {
        Some(offloaded) => Box::pin(async move { offloaded.await.map_err(PickError::from) }),
        None => ready(std::fs::read(path).map_err(PickError::from)),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_path(_path: PathBuf) -> PickFuture<Vec<u8>> {
    ready(Err(PickError::Unsupported))
}

impl fmt::Debug for PickedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PickedFile")
            .field("name", &self.name)
            .field("mime_type", &self.mime_type)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use super::*;

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                return value;
            }
        }
    }

    fn scratch(name: &str, contents: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aimer_file_picker_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("the scratch file is writable");
        path
    }

    #[test]
    fn from_path_describes_the_file() {
        let path = scratch("photo.JPG", b"12345");
        let file = PickedFile::from_path(&path);
        assert_eq!(file.name(), "photo.JPG");
        assert_eq!(file.mime_type(), Some("image/jpeg"));
        assert_eq!(file.size(), Some(5));
        assert_eq!(file.path(), Some(path.as_path()));
    }

    #[test]
    fn a_missing_file_has_no_size_but_still_a_name() {
        let file = PickedFile::from_path("/no/such/dir/notes.txt");
        assert_eq!(file.name(), "notes.txt");
        assert_eq!(file.size(), None);
        assert_eq!(file.mime_type(), Some("text/plain"));
    }

    #[test]
    fn a_reported_mime_type_beats_the_guess() {
        let file = PickedFile::from_parts(
            "clip.mp4".into(),
            Some("video/quicktime".into()),
            None,
            Source::Path(PathBuf::from("clip.mp4")),
        );
        assert_eq!(file.mime_type(), Some("video/quicktime"));

        let blank = PickedFile::from_parts(
            "clip.mp4".into(),
            Some(String::new()),
            None,
            Source::Path(PathBuf::from("clip.mp4")),
        );
        assert_eq!(blank.mime_type(), Some("video/mp4"));
    }

    #[test]
    fn read_returns_the_contents() {
        let path = scratch("data.bin", b"hello");
        assert_eq!(block_on(PickedFile::from_path(path).read()), Ok(b"hello".to_vec()));
    }

    #[test]
    fn reading_a_missing_file_reports_io() {
        let result = block_on(PickedFile::from_path("/no/such/file.txt").read());
        assert_eq!(result, Err(PickError::Io(std::io::ErrorKind::NotFound)));
    }
}
