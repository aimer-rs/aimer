//! Platforms with no live activities of their own.

use super::Backend;
use crate::LiveActivityError;

/// Always fails, so [`super::create`] falls back to the tray or reports
/// [`LiveActivityError::Unsupported`].
pub(super) fn create() -> Result<Box<dyn Backend>, LiveActivityError> {
    Err(LiveActivityError::Unsupported)
}
