#![doc = include_str!("../README.md")]

mod content;
mod picture;
mod window;

pub use aimer_platform::capture::Error as CaptureError;
pub use content::{Capture, CaptureHandle, CaptureRequest, Region, ShotError};
pub use picture::{EncodeError, Format, Image};
pub use window::{DEFAULT_TIMEOUT, screenshot};
