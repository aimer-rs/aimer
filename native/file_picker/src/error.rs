//! Why a pick or a read did not produce files.

use std::fmt;
use std::io;

/// A shorthand for results in this crate.
pub type PickResult<T> = Result<T, PickError>;

/// Why a picker could not be shown or a file could not be read.
///
/// Dismissing the picker is **not** an error: the user chose nothing, and
/// [`FilePicker::pick`](crate::FilePicker::pick) resolves with no files.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PickError {
    /// The request itself is unusable, for example a selection limit of zero.
    InvalidRequest(&'static str),
    /// This target has no system picker, or no way to read what it returned.
    Unsupported,
    /// There is no window or activity to present the picker over.
    NoWindow,
    /// The Android bridge (`com.aimer.FilePickerBridge`) is not part of the app.
    BridgeMissing,
    /// Reading the picked file failed; carries the operating system's reason.
    Io(io::ErrorKind),
    /// The system picker reported a failure; carries its message.
    Platform(String),
    /// The platform dropped the request without answering.
    Interrupted,
}

impl fmt::Display for PickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(reason) => write!(f, "invalid picker request: {reason}"),
            Self::Unsupported => f.write_str("this platform has no system file picker"),
            Self::NoWindow => f.write_str("there is no window to present the picker over"),
            Self::BridgeMissing => {
                f.write_str("the Android bridge com.aimer.FilePickerBridge is not part of the app")
            }
            Self::Io(kind) => write!(f, "reading the picked file failed: {kind}"),
            Self::Platform(message) => write!(f, "the system picker failed: {message}"),
            Self::Interrupted => f.write_str("the picker was interrupted before it answered"),
        }
    }
}

impl std::error::Error for PickError {}

impl From<io::Error> for PickError {
    #[inline]
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_keep_their_kind() {
        let error = PickError::from(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(error, PickError::Io(io::ErrorKind::NotFound));
        assert!(error.to_string().contains("not found"));
    }
}
