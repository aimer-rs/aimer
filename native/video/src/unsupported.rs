//! Platforms without a video backend: every load fails with
//! [`VideoError::Unsupported`], so call sites need no platform guard.

use crate::backend::PlatformVideo;
use crate::{VideoError, VideoSource};

pub(crate) fn open(_source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    Err(VideoError::Unsupported)
}
