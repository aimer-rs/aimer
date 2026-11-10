//! The picture shown in the system tray.

use std::fmt;

/// An RGBA picture for the tray, straight (non-premultiplied) alpha.
///
/// The tray scales the picture to whatever size the platform draws, so give it
/// a square picture of at least 32 by 32 pixels; 64 by 64 stays sharp on dense
/// displays.
///
/// # Examples
///
/// ```
/// use aimer_tray::TrayIcon;
///
/// // A 2x2 opaque red square.
/// let icon = TrayIcon::from_rgba(2, 2, [255, 0, 0, 255].repeat(4)).unwrap();
/// assert_eq!((icon.width(), icon.height()), (2, 2));
///
/// // Three bytes of a four-byte pixel is not a picture.
/// assert!(TrayIcon::from_rgba(1, 1, vec![0, 0, 0]).is_err());
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct TrayIcon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Why [`TrayIcon::from_rgba`] refused its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconError {
    /// The width or the height is zero.
    Empty,
    /// The byte count is not `width * height * 4`.
    SizeMismatch { expected: usize, actual: usize },
}

impl fmt::Display for IconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the tray icon has no pixels"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "the tray icon needs {expected} bytes of RGBA but got {actual}")
            }
        }
    }
}

impl std::error::Error for IconError {}

impl TrayIcon {
    /// Wraps `width * height` RGBA pixels, row by row from the top.
    ///
    /// # Errors
    ///
    /// Returns [`IconError`] if either side is zero or `rgba` is not exactly
    /// `width * height * 4` bytes.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, IconError> {
        if width == 0 || height == 0 {
            return Err(IconError::Empty);
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(IconError::Empty)?;
        if rgba.len() != expected {
            return Err(IconError::SizeMismatch { expected, actual: rgba.len() });
        }
        Ok(Self { width, height, rgba })
    }

    /// Width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, `width * height * 4` bytes of straight RGBA.
    #[inline]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl fmt::Debug for TrayIcon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrayIcon").field("width", &self.width).field("height", &self.height).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_size() {
        let icon = TrayIcon::from_rgba(3, 2, vec![0; 24]).unwrap();
        assert_eq!(icon.rgba().len(), 24);
    }

    #[test]
    fn rejects_zero_sides() {
        assert_eq!(TrayIcon::from_rgba(0, 4, vec![]), Err(IconError::Empty));
        assert_eq!(TrayIcon::from_rgba(4, 0, vec![]), Err(IconError::Empty));
    }

    #[test]
    fn rejects_wrong_length() {
        assert_eq!(
            TrayIcon::from_rgba(2, 2, vec![0; 15]),
            Err(IconError::SizeMismatch { expected: 16, actual: 15 })
        );
    }

    #[test]
    fn rejects_overflowing_dimensions() {
        assert_eq!(TrayIcon::from_rgba(u32::MAX, u32::MAX, vec![]), Err(IconError::Empty));
    }
}
