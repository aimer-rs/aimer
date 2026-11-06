//! Targets without a system picker.

use crate::error::PickError;
use crate::future::{PickFuture, ready};
use crate::picked::PickedFile;
use crate::request::FilePicker;

pub(crate) fn pick(_request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    ready(Err(PickError::Unsupported))
}
