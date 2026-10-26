//! Which way up the window, and the device, are.

/// The shape of a rectangle: taller than wide, or wider than tall.
///
/// This is about the *window*, not the hardware. A desktop window dragged wide
/// is [`Landscape`](Self::Landscape), a phone held upright but split-screened
/// beside another app may be either, and a tablet on a table is whatever its
/// window currently measures. For which way the physical device is turned, see
/// [`DeviceOrientation`].
///
/// # Examples
///
/// ```
/// use aimer_screen::Orientation;
///
/// assert_eq!(Orientation::of_size(390.0, 844.0), Orientation::Portrait);
/// assert_eq!(Orientation::of_size(844.0, 390.0), Orientation::Landscape);
/// // A square is not wider than it is tall.
/// assert_eq!(Orientation::of_size(500.0, 500.0), Orientation::Portrait);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Orientation {
    /// At least as tall as wide.
    Portrait,
    /// Wider than tall.
    Landscape,
}

impl Orientation {
    /// The orientation of a `width` by `height` rectangle.
    #[inline]
    pub fn of_size(width: f32, height: f32) -> Self {
        if width > height {
            Self::Landscape
        } else {
            Self::Portrait
        }
    }

    /// Returns `true` for [`Orientation::Portrait`].
    #[inline]
    pub const fn is_portrait(self) -> bool {
        matches!(self, Self::Portrait)
    }

    /// Returns `true` for [`Orientation::Landscape`].
    #[inline]
    pub const fn is_landscape(self) -> bool {
        matches!(self, Self::Landscape)
    }
}

/// Which way the physical device is turned.
///
/// Named by where the top of the device points, using UIKit's vocabulary:
/// [`LandscapeLeft`](Self::LandscapeLeft) is the device turned a quarter
/// counter-clockwise from upright, so its top points left, and
/// [`LandscapeRight`](Self::LandscapeRight) is a quarter clockwise. The variants
/// are in counter-clockwise order from [`PortraitUp`](Self::PortraitUp).
///
/// Only devices that rotate — phones, tablets, and a browser on either — have
/// one; see [`device_orientation`](crate::device_orientation).
///
/// # Examples
///
/// ```
/// use aimer_screen::{DeviceOrientation, Orientation};
///
/// assert_eq!(DeviceOrientation::LandscapeLeft.orientation(), Orientation::Landscape);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeviceOrientation {
    /// Upright, top of the device up.
    PortraitUp,
    /// Turned a quarter counter-clockwise: the top points left.
    LandscapeLeft,
    /// Upside down.
    PortraitDown,
    /// Turned a quarter clockwise: the top points right.
    LandscapeRight,
}

impl DeviceOrientation {
    /// The four orientations in counter-clockwise order from upright.
    const COUNTER_CLOCKWISE: [Self; 4] = [
        Self::PortraitUp,
        Self::LandscapeLeft,
        Self::PortraitDown,
        Self::LandscapeRight,
    ];

    /// Whether the device is held upright or on its side.
    #[inline]
    pub const fn orientation(self) -> Orientation {
        match self {
            Self::PortraitUp | Self::PortraitDown => Orientation::Portrait,
            Self::LandscapeLeft | Self::LandscapeRight => Orientation::Landscape,
        }
    }

    /// Reads Android's `Display.getRotation()` together with the display's
    /// current size in pixels.
    ///
    /// The rotation is relative to the device's *natural* orientation, which is
    /// portrait on a phone and landscape on many tablets, so the same `0` means
    /// different things; the size reveals which this is. `None` for a rotation
    /// outside `0..=3`.
    pub fn from_android_rotation(rotation: i32, width: u32, height: u32) -> Option<Self> {
        let rotation = usize::try_from(rotation).ok().filter(|&rotation| rotation < 4)?;
        // At a quarter turn the display's width and height swap, so the natural
        // shape is read from whichever of them is the long side then.
        let natural_is_landscape = if rotation % 2 == 0 {
            width > height
        } else {
            height > width
        };
        // A natural-landscape device sits one step round the circle from upright.
        let start = usize::from(natural_is_landscape);
        Some(Self::COUNTER_CLOCKWISE[(start + rotation) % 4])
    }

    /// Reads UIKit's `UIDeviceOrientation` raw value.
    ///
    /// `None` for the values that name no direction — unknown, face up and face
    /// down — because a device lying flat says nothing about which way the
    /// interface is turned.
    pub fn from_ui_device_orientation(raw: isize) -> Option<Self> {
        match raw {
            1 => Some(Self::PortraitUp),
            2 => Some(Self::PortraitDown),
            3 => Some(Self::LandscapeLeft),
            4 => Some(Self::LandscapeRight),
            _ => None,
        }
    }

    /// Reads a browser's `screen.orientation.type`.
    ///
    /// `None` for a value the Screen Orientation API does not define.
    pub fn from_screen_orientation_type(kind: &str) -> Option<Self> {
        match kind {
            "portrait-primary" => Some(Self::PortraitUp),
            "portrait-secondary" => Some(Self::PortraitDown),
            "landscape-primary" => Some(Self::LandscapeLeft),
            "landscape-secondary" => Some(Self::LandscapeRight),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceOrientation::*;

    #[test]
    fn a_window_wider_than_tall_is_landscape() {
        assert_eq!(Orientation::of_size(800.0, 600.0), Orientation::Landscape);
        assert_eq!(Orientation::of_size(600.0, 800.0), Orientation::Portrait);
        assert_eq!(Orientation::of_size(600.0, 600.0), Orientation::Portrait);
        assert_eq!(Orientation::of_size(0.0, 0.0), Orientation::Portrait);
        assert!(Orientation::Landscape.is_landscape());
        assert!(Orientation::Portrait.is_portrait());
    }

    #[test]
    fn a_phone_rotates_counter_clockwise_through_the_android_rotations() {
        // Natural portrait: the display is taller than wide at rotations 0 and 2.
        assert_eq!(DeviceOrientation::from_android_rotation(0, 1080, 2400), Some(PortraitUp));
        assert_eq!(DeviceOrientation::from_android_rotation(1, 2400, 1080), Some(LandscapeLeft));
        assert_eq!(DeviceOrientation::from_android_rotation(2, 1080, 2400), Some(PortraitDown));
        assert_eq!(DeviceOrientation::from_android_rotation(3, 2400, 1080), Some(LandscapeRight));
    }

    #[test]
    fn a_landscape_tablet_starts_on_its_side() {
        // Natural landscape: wider than tall at rotations 0 and 2.
        assert_eq!(DeviceOrientation::from_android_rotation(0, 2560, 1600), Some(LandscapeLeft));
        assert_eq!(DeviceOrientation::from_android_rotation(1, 1600, 2560), Some(PortraitDown));
        assert_eq!(DeviceOrientation::from_android_rotation(2, 2560, 1600), Some(LandscapeRight));
        assert_eq!(DeviceOrientation::from_android_rotation(3, 1600, 2560), Some(PortraitUp));
    }

    #[test]
    fn an_android_rotation_out_of_range_is_unknown() {
        assert_eq!(DeviceOrientation::from_android_rotation(4, 1080, 2400), None);
        assert_eq!(DeviceOrientation::from_android_rotation(-1, 1080, 2400), None);
    }

    #[test]
    fn uikit_orientations_map_and_flat_ones_do_not() {
        assert_eq!(DeviceOrientation::from_ui_device_orientation(1), Some(PortraitUp));
        assert_eq!(DeviceOrientation::from_ui_device_orientation(2), Some(PortraitDown));
        assert_eq!(DeviceOrientation::from_ui_device_orientation(3), Some(LandscapeLeft));
        assert_eq!(DeviceOrientation::from_ui_device_orientation(4), Some(LandscapeRight));
        for raw in [0, 5, 6, 7, -1] {
            assert_eq!(DeviceOrientation::from_ui_device_orientation(raw), None, "{raw}");
        }
    }

    #[test]
    fn browser_orientation_types_map() {
        let read = DeviceOrientation::from_screen_orientation_type;
        assert_eq!(read("portrait-primary"), Some(PortraitUp));
        assert_eq!(read("portrait-secondary"), Some(PortraitDown));
        assert_eq!(read("landscape-primary"), Some(LandscapeLeft));
        assert_eq!(read("landscape-secondary"), Some(LandscapeRight));
        assert_eq!(read("sideways"), None);
        assert_eq!(read(""), None);
    }

    #[test]
    fn a_device_orientation_knows_whether_it_is_on_its_side() {
        assert!(PortraitUp.orientation().is_portrait());
        assert!(PortraitDown.orientation().is_portrait());
        assert!(LandscapeLeft.orientation().is_landscape());
        assert!(LandscapeRight.orientation().is_landscape());
    }
}
