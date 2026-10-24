//! The one error type of the persistence layer.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use aimer_storage::StorageError;

use crate::value::DecodeError;

/// An asynchronous persistence operation.
///
/// Like [`StorageFuture`](aimer_storage::StorageFuture) it is not `Send`: a
/// Venus task awaits it on the UI thread and keeps its `Rc` captures.
pub type PersistFuture<T> = Pin<Box<dyn Future<Output = Result<T, PersistError>> + 'static>>;

/// Why a persistence operation failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistError {
    /// The storage layer failed: I/O, quota, an invalid name, no runtime.
    Storage(StorageError),
    /// Stored bytes are not a valid encoding of the requested type.
    Decode(DecodeError),
    /// The data was written by a newer version of the app than this one.
    ///
    /// It is reported rather than read or overwritten: loading it would guess at
    /// a layout this build does not know, and saving over it would destroy data
    /// the newer build still needs.
    NewerVersion {
        /// The version found in storage.
        found: u32,
        /// The newest version this build understands.
        supported: u32,
    },
}

impl fmt::Display for PersistError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "storage failed: {error:?}"),
            Self::Decode(error) => write!(formatter, "stored data is invalid: {error}"),
            Self::NewerVersion { found, supported } => write!(
                formatter,
                "stored data is version {found}, newer than the supported version {supported}"
            ),
        }
    }
}

impl Error for PersistError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Storage(_) | Self::NewerVersion { .. } => None,
        }
    }
}

impl From<StorageError> for PersistError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<DecodeError> for PersistError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}
