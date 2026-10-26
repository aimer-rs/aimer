//! A parsed operating-system version.

use std::fmt::{Display, Formatter};

/// An operating-system version reduced to its numeric `major.minor.patch`
/// components, so two versions can be compared instead of string-matched.
///
/// Platforms report versions in different shapes — `14.5.1`, `10.0.22631`,
/// Android's `14`, a distribution's `24.04` — and [`OsVersion::parse`] reads the
/// leading numeric components of any of them. A missing component is `0`, which
/// makes `14` and `14.0.0` equal.
///
/// # Examples
///
/// ```
/// use aimer_device::OsVersion;
///
/// let version = OsVersion::parse("17.4.1");
/// assert!(version >= OsVersion::new(17, 0, 0));
/// assert_eq!(version.to_string(), "17.4.1");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OsVersion {
    /// The major component.
    pub major: u32,
    /// The minor component, `0` when the platform reports none.
    pub minor: u32,
    /// The patch component, `0` when the platform reports none.
    pub patch: u32,
}

impl OsVersion {
    /// Creates a version from its components.
    #[inline]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Reads the leading `major[.minor[.patch]]` of `text`.
    ///
    /// Parsing stops at the first component that is not a number, so trailing
    /// text such as the `-beta` of `15.0-beta` or the build of `10.0.22631.3447`
    /// is ignored. Text with no leading number yields [`OsVersion::default`].
    pub fn parse(text: &str) -> Self {
        let mut parts = [0u32; 3];
        for (slot, component) in parts.iter_mut().zip(text.split('.')) {
            let digits = component.bytes().take_while(u8::is_ascii_digit).count();
            // An empty or overflowing component ends the version; what was read
            // before it stands.
            let Ok(value) = component[..digits].parse() else {
                break;
            };
            *slot = value;
            // `15.0-beta`: text after the digits means no further component.
            if digits != component.len() {
                break;
            }
        }
        Self::new(parts[0], parts[1], parts[2])
    }
}

impl Display for OsVersion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_and_partial_versions() {
        assert_eq!(OsVersion::parse("14.5.1"), OsVersion::new(14, 5, 1));
        assert_eq!(OsVersion::parse("14.5"), OsVersion::new(14, 5, 0));
        assert_eq!(OsVersion::parse("14"), OsVersion::new(14, 0, 0));
        assert_eq!(OsVersion::parse("24.04"), OsVersion::new(24, 4, 0));
    }

    #[test]
    fn ignores_trailing_text_and_extra_components() {
        assert_eq!(OsVersion::parse("15.0-beta"), OsVersion::new(15, 0, 0));
        assert_eq!(OsVersion::parse("10.0.22631.3447"), OsVersion::new(10, 0, 22631));
        assert_eq!(OsVersion::parse("13.2 (a)"), OsVersion::new(13, 2, 0));
    }

    #[test]
    fn text_without_a_leading_number_is_zero() {
        assert_eq!(OsVersion::parse(""), OsVersion::default());
        assert_eq!(OsVersion::parse("unknown"), OsVersion::default());
        assert_eq!(OsVersion::parse(".5"), OsVersion::default());
        // Overflowing components must not panic.
        assert_eq!(OsVersion::parse("99999999999.1").major, 0);
    }

    #[test]
    fn orders_by_component() {
        assert!(OsVersion::parse("10.15.7") < OsVersion::parse("11.0"));
        assert!(OsVersion::parse("14") == OsVersion::parse("14.0.0"));
    }
}
