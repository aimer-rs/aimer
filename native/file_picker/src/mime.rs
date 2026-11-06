//! A small extension ↔ MIME table.
//!
//! Pickers report a MIME type where the platform knows one and the file name
//! otherwise; filters are written with extensions but Android and the web want
//! MIME types. The table covers the formats applications ask users for. It is a
//! hint, not a sniffer: an unknown extension maps to nothing.

/// Extension (lowercase, no dot) and MIME type. The first entry for a MIME type
/// is its preferred extension.
const TABLE: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("svg", "image/svg+xml"),
    ("ico", "image/x-icon"),
    ("mp4", "video/mp4"),
    ("m4v", "video/x-m4v"),
    ("mov", "video/quicktime"),
    ("webm", "video/webm"),
    ("mkv", "video/x-matroska"),
    ("avi", "video/x-msvideo"),
    ("mp3", "audio/mpeg"),
    ("m4a", "audio/mp4"),
    ("wav", "audio/wav"),
    ("flac", "audio/flac"),
    ("ogg", "audio/ogg"),
    ("aac", "audio/aac"),
    ("pdf", "application/pdf"),
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("csv", "text/csv"),
    ("html", "text/html"),
    ("json", "application/json"),
    ("xml", "application/xml"),
    ("zip", "application/zip"),
    ("gz", "application/gzip"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
];

/// The MIME type for a file extension, compared without regard to case.
pub fn mime_for_extension(extension: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        .map(|&(_, mime)| mime)
}

/// The preferred extension for an exact MIME type (no wildcards).
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|(_, known)| known.eq_ignore_ascii_case(mime))
        .map(|&(extension, _)| extension)
}

/// The MIME type of a file name, from its extension.
pub(crate) fn mime_for_name(name: &str) -> Option<&'static str> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    mime_for_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_mime_types_ignoring_case() {
        assert_eq!(mime_for_extension("PNG"), Some("image/png"));
        assert_eq!(mime_for_extension("jpeg"), Some("image/jpeg"));
        assert_eq!(mime_for_extension("nope"), None);
    }

    #[test]
    fn mime_types_map_back_to_their_preferred_extension() {
        assert_eq!(extension_for_mime("image/jpeg"), Some("jpg"));
        assert_eq!(extension_for_mime("IMAGE/PNG"), Some("png"));
        assert_eq!(extension_for_mime("image/*"), None);
    }

    #[test]
    fn names_without_a_real_extension_have_no_mime() {
        assert_eq!(mime_for_name("photo.JPG"), Some("image/jpeg"));
        assert_eq!(mime_for_name("archive.tar.gz"), Some("application/gzip"));
        assert_eq!(mime_for_name(".png"), None);
        assert_eq!(mime_for_name("README"), None);
    }
}
