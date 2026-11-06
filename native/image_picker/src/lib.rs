#![doc = include_str!("../README.md")]

mod media;
mod picker;

#[cfg(target_os = "ios")]
mod ios;

pub use aimer_file_picker::{PickError, PickFuture, PickResult, PickedFile};
pub use media::MediaKind;
pub use picker::ImagePicker;
