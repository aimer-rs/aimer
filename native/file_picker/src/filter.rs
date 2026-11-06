//! Which files a picker offers.

use crate::mime::{extension_for_mime, mime_for_extension};

/// A named set of file types, as a picker's type drop-down shows it.
///
/// A filter is written with extensions (`"png"`), MIME types (`"image/png"`,
/// `"image/*"`), or both; each platform uses the vocabulary it understands and
/// the crate translates the other through a small built-in table. A picker with
/// several filters offers the union.
///
/// # Examples
///
/// ```
/// use aimer_file_picker::FileFilter;
///
/// let documents = FileFilter::new("Documents").extension("pdf").extension(".TXT");
/// assert_eq!(documents.extension_list(), ["pdf", "txt"]);
/// assert_eq!(documents.resolved_mime_types(), ["application/pdf", "text/plain"]);
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileFilter {
    name: String,
    extensions: Vec<String>,
    mime_types: Vec<String>,
}

impl FileFilter {
    /// Creates an empty filter with the label a picker may show for it.
    #[inline]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            extensions: Vec::new(),
            mime_types: Vec::new(),
        }
    }

    /// Every image format the crate knows, including `image/*`.
    pub fn images() -> Self {
        Self::new("Images")
            .mime_type("image/*")
            .extensions(["png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "avif", "bmp", "tiff"])
    }

    /// Common video formats, including `video/*`.
    pub fn videos() -> Self {
        Self::new("Videos")
            .mime_type("video/*")
            .extensions(["mp4", "m4v", "mov", "webm", "mkv", "avi"])
    }

    /// Common audio formats, including `audio/*`.
    pub fn audio() -> Self {
        Self::new("Audio")
            .mime_type("audio/*")
            .extensions(["mp3", "m4a", "wav", "flac", "ogg", "aac"])
    }

    /// Adds an extension. A leading `.` or `*.` and the case are ignored; an
    /// empty or duplicate extension is dropped.
    pub fn extension(mut self, extension: impl AsRef<str>) -> Self {
        let extension = extension.as_ref().trim();
        let extension = extension.strip_prefix("*.").unwrap_or(extension);
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        if !extension.is_empty() && !self.extensions.contains(&extension) {
            self.extensions.push(extension);
        }
        self
    }

    /// Adds several extensions; see [`FileFilter::extension`].
    pub fn extensions<I>(mut self, extensions: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        for extension in extensions {
            self = self.extension(extension);
        }
        self
    }

    /// Adds a MIME type such as `application/pdf` or a wildcard such as
    /// `image/*`. Text without a `/` is not a MIME type and is dropped, as are
    /// duplicates.
    pub fn mime_type(mut self, mime_type: impl AsRef<str>) -> Self {
        let mime_type = mime_type.as_ref().trim().to_ascii_lowercase();
        let well_formed = mime_type
            .split_once('/')
            .is_some_and(|(kind, subtype)| !kind.is_empty() && !subtype.is_empty());
        if well_formed && !self.mime_types.contains(&mime_type) {
            self.mime_types.push(mime_type);
        }
        self
    }

    /// The label for this filter.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The extensions, lowercase and without a dot.
    #[inline]
    pub fn extension_list(&self) -> &[String] {
        &self.extensions
    }

    /// The MIME types and wildcards added explicitly.
    #[inline]
    pub fn mime_types(&self) -> &[String] {
        &self.mime_types
    }

    /// Whether the filter accepts nothing in particular.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty() && self.mime_types.is_empty()
    }

    /// Every MIME type and wildcard the filter stands for: the explicit ones,
    /// then those of its extensions the table knows.
    pub fn resolved_mime_types(&self) -> Vec<String> {
        let mut resolved = self.mime_types.clone();
        for extension in &self.extensions {
            if let Some(mime) = mime_for_extension(extension)
                && !resolved.iter().any(|known| known == mime)
            {
                resolved.push(mime.to_owned());
            }
        }
        resolved
    }

    /// Every extension the filter stands for: the explicit ones, then those of
    /// its exact (non-wildcard) MIME types the table knows.
    pub fn resolved_extensions(&self) -> Vec<String> {
        let mut resolved = self.extensions.clone();
        for mime in &self.mime_types {
            if let Some(extension) = extension_for_mime(mime)
                && !resolved.iter().any(|known| known == extension)
            {
                resolved.push(extension.to_owned());
            }
        }
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_normalized() {
        let filter = FileFilter::new("x").extensions([".PNG", "*.jpg", "  gif ", "", ".", "png"]);
        assert_eq!(filter.extension_list(), ["png", "jpg", "gif"]);
    }

    #[test]
    fn malformed_mime_types_are_dropped() {
        let filter = FileFilter::new("x")
            .mime_type("Image/PNG")
            .mime_type("png")
            .mime_type("/png")
            .mime_type("image/")
            .mime_type("image/png");
        assert_eq!(filter.mime_types(), ["image/png"]);
    }

    #[test]
    fn mime_types_are_resolved_from_extensions_without_duplicates() {
        let filter = FileFilter::new("x").mime_type("image/png").extensions(["png", "jpg", "zzz"]);
        assert_eq!(filter.resolved_mime_types(), ["image/png", "image/jpeg"]);
    }

    #[test]
    fn extensions_are_resolved_from_exact_mime_types_only() {
        let filter = FileFilter::new("x").mime_type("application/pdf").mime_type("image/*");
        assert_eq!(filter.resolved_extensions(), ["pdf"]);
    }

    #[test]
    fn presets_carry_wildcards_and_extensions() {
        let images = FileFilter::images();
        assert!(images.mime_types().contains(&"image/*".to_owned()));
        assert!(images.extension_list().contains(&"heic".to_owned()));
        assert!(!images.is_empty());
        assert!(FileFilter::new("none").is_empty());
    }
}
