//! Bounded and fail-closed validation for browser or native file drops.
//!
//! Validation is metadata-only. It never opens, canonicalizes, follows, or
//! otherwise performs I/O on a path supplied by a platform. A batch is
//! accepted atomically: one invalid entry rejects the complete batch, so a
//! caller cannot accidentally process a valid prefix before discovering an
//! unsafe or oversized file.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Default maximum number of files in one drop batch.
pub const DEFAULT_MAX_FILES: usize = 32;
/// Default maximum size of one dropped file.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Default maximum size of one complete drop batch.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
/// Default maximum UTF-8 byte length of a dropped path.
pub const DEFAULT_MAX_PATH_BYTES: usize = 4096;

/// Metadata supplied by a file-drop adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDropEntry {
    path: PathBuf,
    size: u64,
    content_type: Option<String>,
    directory: bool,
    symlink: bool,
}

impl FileDropEntry {
    /// Creates a file candidate from a path and its reported byte size.
    #[inline]
    pub fn new(path: PathBuf, size: u64) -> Self {
        Self {
            path,
            size,
            content_type: None,
            directory: false,
            symlink: false,
        }
    }

    /// Adds the platform-reported MIME type, if one exists.
    #[inline]
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Marks this entry as a directory.
    #[inline]
    pub const fn directory(mut self, directory: bool) -> Self {
        self.directory = directory;
        self
    }

    /// Marks this entry as a symbolic link.
    #[inline]
    pub const fn symlink(mut self, symlink: bool) -> Self {
        self.symlink = symlink;
        self
    }

    /// The unmodified path supplied by the platform.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The reported file size in bytes.
    #[inline]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// The optional platform-reported MIME type.
    #[inline]
    pub fn content_type_value(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Whether the entry is marked as a directory.
    #[inline]
    pub const fn is_directory(&self) -> bool {
        self.directory
    }

    /// Whether the entry is marked as a symbolic link.
    #[inline]
    pub const fn is_symlink(&self) -> bool {
        self.symlink
    }
}

/// Why a file-drop batch was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDropRejectReason {
    /// No entries were supplied.
    EmptyBatch,
    /// The batch contains more entries than the configured bound.
    TooManyFiles,
    /// One entry exceeds the per-file byte bound.
    FileTooLarge,
    /// The accumulated batch exceeds the total byte bound.
    TotalTooLarge,
    /// The extension or MIME type is not in the configured allowlist.
    TypeNotAllowed,
    /// The path exceeds the configured byte bound.
    PathTooLong,
    /// The path contains a parent component.
    PathTraversal,
    /// The path is not valid Unicode metadata for this adapter.
    InvalidPath,
    /// The path contains a control character.
    PathControlCharacter,
    /// Directories are not accepted by the current policy.
    Directory,
    /// Symbolic links are not accepted by the current policy.
    Symlink,
}

impl fmt::Display for FileDropRejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyBatch => "file drop is empty",
            Self::TooManyFiles => "file drop contains too many files",
            Self::FileTooLarge => "file exceeds the per-file size limit",
            Self::TotalTooLarge => "file drop exceeds the total size limit",
            Self::TypeNotAllowed => "file type is not allowed",
            Self::PathTooLong => "file path is too long",
            Self::PathTraversal => "file path contains traversal",
            Self::InvalidPath => "file path is not valid metadata",
            Self::PathControlCharacter => "file path contains a control character",
            Self::Directory => "directories are not accepted",
            Self::Symlink => "symbolic links are not accepted",
        };
        f.write_str(message)
    }
}

/// A rejection that does not expose the untrusted path in its diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDropRejection {
    item_index: Option<usize>,
    reason: FileDropRejectReason,
}

impl FileDropRejection {
    /// The rejected item index, or `None` for a batch-wide failure.
    #[inline]
    pub const fn item_index(&self) -> Option<usize> {
        self.item_index
    }

    /// The bounded reason for rejection.
    #[inline]
    pub const fn reason(&self) -> FileDropRejectReason {
        self.reason
    }
}

/// The result of validating a complete drop batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDropOutcome {
    /// Every entry passed the configured bounds and security checks.
    Accepted(ValidatedFileDrop),
    /// No entry from the batch should be processed.
    Rejected(FileDropRejection),
}

impl FileDropOutcome {
    /// Returns the accepted batch, if validation succeeded.
    #[inline]
    pub fn accepted(self) -> Option<ValidatedFileDrop> {
        match self {
            Self::Accepted(batch) => Some(batch),
            Self::Rejected(_) => None,
        }
    }

    /// Returns the rejection, if validation failed.
    #[inline]
    pub fn rejection(&self) -> Option<&FileDropRejection> {
        match self {
            Self::Accepted(_) => None,
            Self::Rejected(rejection) => Some(rejection),
        }
    }
}

/// A validated batch safe for the caller to hand to its file-processing layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedFileDrop {
    files: Vec<FileDropEntry>,
    total_bytes: u64,
}

impl ValidatedFileDrop {
    /// The validated entries in platform order.
    #[inline]
    pub fn files(&self) -> &[FileDropEntry] {
        &self.files
    }

    /// The checked total size of all entries.
    #[inline]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Moves the validated entries out of the batch.
    #[inline]
    pub fn into_files(self) -> Vec<FileDropEntry> {
        self.files
    }
}

/// Bounds, allowlists, and path-safety policy for a file drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDropPolicy {
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_path_bytes: usize,
    allowed_extensions: Option<Vec<String>>,
    allowed_mime_types: Option<Vec<String>>,
    allow_directories: bool,
    allow_symlinks: bool,
}

impl Default for FileDropPolicy {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_path_bytes: DEFAULT_MAX_PATH_BYTES,
            allowed_extensions: None,
            allowed_mime_types: None,
            allow_directories: false,
            allow_symlinks: false,
        }
    }
}

impl FileDropPolicy {
    /// Creates the default bounded, file-only policy.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum number of entries in a batch.
    #[inline]
    pub const fn max_files(mut self, max_files: usize) -> Self {
        self.max_files = max_files;
        self
    }

    /// Sets the maximum size of one entry in bytes.
    #[inline]
    pub const fn max_file_bytes(mut self, max_file_bytes: u64) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    /// Sets the maximum aggregate size of a batch in bytes.
    #[inline]
    pub const fn max_total_bytes(mut self, max_total_bytes: u64) -> Self {
        self.max_total_bytes = max_total_bytes;
        self
    }

    /// Sets the maximum UTF-8 byte length of a candidate path.
    #[inline]
    pub const fn max_path_bytes(mut self, max_path_bytes: usize) -> Self {
        self.max_path_bytes = max_path_bytes;
        self
    }

    /// Requires one of the case-insensitive extensions in `extensions`.
    #[inline]
    pub fn allowed_extensions<I, S>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_extensions = Some(normalize_extensions(extensions));
        self
    }

    /// Requires one of the case-insensitive MIME types in `mime_types`.
    ///
    /// Parameters after `;` are ignored, so a browser value such as
    /// `image/png; charset=binary` is compared as `image/png`.
    #[inline]
    pub fn allowed_mime_types<I, S>(mut self, mime_types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_mime_types = Some(normalize_mime_types(mime_types));
        self
    }

    /// Whether directories should be accepted. The default is `false`.
    #[inline]
    pub const fn allow_directories(mut self, allow: bool) -> Self {
        self.allow_directories = allow;
        self
    }

    /// Whether symbolic links should be accepted. The default is `false`.
    #[inline]
    pub const fn allow_symlinks(mut self, allow: bool) -> Self {
        self.allow_symlinks = allow;
        self
    }

    /// Validates every entry and returns either the complete safe batch or one
    /// bounded rejection outcome.
    pub fn validate(&self, entries: &[FileDropEntry]) -> FileDropOutcome {
        if entries.is_empty() {
            return Self::rejected(None, FileDropRejectReason::EmptyBatch);
        }
        if entries.len() > self.max_files {
            return Self::rejected(None, FileDropRejectReason::TooManyFiles);
        }

        let mut total_bytes = 0_u64;
        for (index, entry) in entries.iter().enumerate() {
            if entry.is_directory() && !self.allow_directories {
                return Self::rejected(Some(index), FileDropRejectReason::Directory);
            }
            if entry.is_symlink() && !self.allow_symlinks {
                return Self::rejected(Some(index), FileDropRejectReason::Symlink);
            }
            if let Some(reason) = validate_path(entry.path(), self.max_path_bytes) {
                return Self::rejected(Some(index), reason);
            }
            if entry.size() > self.max_file_bytes {
                return Self::rejected(Some(index), FileDropRejectReason::FileTooLarge);
            }
            let Some(next_total) = total_bytes.checked_add(entry.size()) else {
                return Self::rejected(Some(index), FileDropRejectReason::TotalTooLarge);
            };
            if next_total > self.max_total_bytes {
                return Self::rejected(Some(index), FileDropRejectReason::TotalTooLarge);
            }
            if !self.type_allowed(entry) {
                return Self::rejected(Some(index), FileDropRejectReason::TypeNotAllowed);
            }
            total_bytes = next_total;
        }

        FileDropOutcome::Accepted(ValidatedFileDrop {
            files: entries.to_vec(),
            total_bytes,
        })
    }

    fn type_allowed(&self, entry: &FileDropEntry) -> bool {
        if let Some(extensions) = self.allowed_extensions.as_deref() {
            let extension = entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase);
            if extension.is_none_or(|extension| !extensions.contains(&extension)) {
                return false;
            }
        }
        if let Some(mime_types) = self.allowed_mime_types.as_deref() {
            let mime = entry
                .content_type_value()
                .and_then(normalize_mime)
                .map(|mime| mime.to_ascii_lowercase());
            if mime.is_none_or(|mime| !mime_types.contains(&mime)) {
                return false;
            }
        }
        true
    }

    #[inline]
    fn rejected(item_index: Option<usize>, reason: FileDropRejectReason) -> FileDropOutcome {
        FileDropOutcome::Rejected(FileDropRejection { item_index, reason })
    }
}

fn normalize_extensions<I, S>(extensions: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized = Vec::new();
    for extension in extensions {
        let extension = extension
            .as_ref()
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if !normalized.contains(&extension) {
            normalized.push(extension);
        }
    }
    normalized
}

fn normalize_mime_types<I, S>(mime_types: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized = Vec::new();
    for mime in mime_types {
        if let Some(mime) = normalize_mime(mime.as_ref())
            && !normalized.contains(&mime)
        {
            normalized.push(mime);
        }
    }
    normalized
}

fn normalize_mime(mime: &str) -> Option<String> {
    let mime = mime.split(';').next()?.trim().to_ascii_lowercase();
    (!mime.is_empty() && mime.contains('/') && !mime.chars().any(char::is_control)).then_some(mime)
}

fn validate_path(path: &Path, max_path_bytes: usize) -> Option<FileDropRejectReason> {
    let Some(text) = path.to_str() else {
        return Some(FileDropRejectReason::InvalidPath);
    };
    if text.len() > max_path_bytes {
        return Some(FileDropRejectReason::PathTooLong);
    }
    if text.chars().any(char::is_control) {
        return Some(FileDropRejectReason::PathControlCharacter);
    }
    if path.file_name().and_then(|name| name.to_str()).is_none() {
        return Some(FileDropRejectReason::InvalidPath);
    }
    for component in path.components() {
        match component {
            Component::ParentDir => return Some(FileDropRejectReason::PathTraversal),
            Component::Normal(value) if value.to_str().is_none() => {
                return Some(FileDropRejectReason::InvalidPath);
            }
            Component::Normal(_)
            | Component::CurDir
            | Component::RootDir
            | Component::Prefix(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(path: &str, size: u64) -> FileDropEntry {
        FileDropEntry::new(PathBuf::from(path), size).content_type("image/png")
    }

    fn policy() -> FileDropPolicy {
        FileDropPolicy::new()
            .max_files(2)
            .max_file_bytes(10)
            .max_total_bytes(15)
            .max_path_bytes(64)
            .allowed_extensions(["png"])
            .allowed_mime_types(["image/png"])
    }

    #[test]
    fn a_valid_batch_is_accepted_atomically() {
        let result = policy().validate(&[entry("photo.PNG", 8)]);

        let FileDropOutcome::Accepted(batch) = result else {
            panic!("the valid file should be accepted");
        };
        assert_eq!(batch.files().len(), 1);
        assert_eq!(batch.total_bytes(), 8);
        assert_eq!(batch.files()[0].path(), PathBuf::from("photo.PNG"));
    }

    #[test]
    fn file_count_size_and_total_limits_fail_closed() {
        let too_many =
            policy().validate(&[entry("a.png", 1), entry("b.png", 1), entry("c.png", 1)]);
        assert!(matches!(
            too_many,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::TooManyFiles
        ));

        let too_large = policy().validate(&[entry("a.png", 11)]);
        assert!(matches!(
            too_large,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::FileTooLarge
        ));

        let total_too_large = policy().validate(&[entry("a.png", 8), entry("b.png", 8)]);
        assert!(matches!(
            total_too_large,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::TotalTooLarge
        ));
    }

    #[test]
    fn type_limits_are_case_insensitive_and_missing_metadata_is_rejected() {
        let accepted = policy().validate(&[entry("a.PnG", 1)]);
        assert!(matches!(accepted, FileDropOutcome::Accepted(_)));

        let wrong_extension = policy().validate(&[entry("a.jpg", 1)]);
        assert!(matches!(
            wrong_extension,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::TypeNotAllowed
        ));

        let mut no_mime = FileDropEntry::new(PathBuf::from("a.png"), 1);
        no_mime = no_mime.content_type("");
        let missing_mime = policy().validate(&[no_mime]);
        assert!(matches!(
            missing_mime,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::TypeNotAllowed
        ));
    }

    #[test]
    fn traversal_directories_symlinks_and_long_paths_are_rejected_without_io() {
        let traversal = policy().validate(&[entry("../a.png", 1)]);
        assert!(matches!(
            traversal,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::PathTraversal
        ));

        let directory = policy().validate(&[entry("folder/a.png", 1).directory(true)]);
        assert!(matches!(
            directory,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::Directory
        ));

        let symlink = policy().validate(&[entry("a.png", 1).symlink(true)]);
        assert!(matches!(
            symlink,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::Symlink
        ));

        let long_name = format!("{}.png", "a".repeat(65));
        let long_path = policy().validate(&[entry(&long_name, 1)]);
        assert!(matches!(
            long_path,
            FileDropOutcome::Rejected(rejection)
                if rejection.reason() == FileDropRejectReason::PathTooLong
        ));
    }

    #[test]
    fn a_mixed_batch_never_returns_the_valid_prefix() {
        let result = policy().validate(&[entry("good.png", 1), entry("bad.jpg", 1)]);

        assert!(matches!(
            result,
            FileDropOutcome::Rejected(rejection)
                if rejection.item_index() == Some(1)
                    && rejection.reason() == FileDropRejectReason::TypeNotAllowed
        ));
    }
}
