//! Swapchain format policy for the native Vulkan backend.
//!
//! The policy is generic over the offered format type because `ash` only
//! builds for Linux and Android, while the choice should be testable on every
//! host.

/// How the swapchain treats the 8-bit color values a shader writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub(crate) enum SurfaceFormatClass {
    /// An 8-bit `*_SRGB` format in the sRGB non-linear color space: the
    /// hardware encodes linear shader output.
    Srgb8,
    /// An 8-bit `*_UNORM` format in the sRGB non-linear color space: values
    /// are stored exactly as written.
    Unorm8,
    /// Anything else, including formats in another color space.
    Other,
}

/// Picks the swapchain format from what the surface offers, in driver order.
///
/// With `prefer_srgb` the hardware encodes linear shader output, which is what
/// the shaders assume everywhere but Android. Android's shaders write
/// already-encoded sRGB values (`surface_is_srgb == 2`), so an `*_SRGB` target
/// would encode them a second time and wash every color out; it asks for
/// `*_UNORM` instead. If no preferred format exists the surface's first format
/// is used.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub(crate) fn select_surface_format<T: Copy>(
    offered: &[T],
    prefer_srgb: bool,
    classify: impl Fn(T) -> SurfaceFormatClass,
) -> Option<T> {
    let wanted = if prefer_srgb {
        SurfaceFormatClass::Srgb8
    } else {
        SurfaceFormatClass::Unorm8
    };
    offered
        .iter()
        .copied()
        .find(|format| classify(*format) == wanted)
        .or_else(|| offered.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    // (name, class): the name stands in for the driver's format enum.
    type Offer = (&'static str, SurfaceFormatClass);

    fn classify(offer: Offer) -> SurfaceFormatClass {
        offer.1
    }

    const SRGB: Offer = ("rgba8_srgb", SurfaceFormatClass::Srgb8);
    const UNORM: Offer = ("rgba8_unorm", SurfaceFormatClass::Unorm8);
    const OTHER: Offer = ("r5g6b5", SurfaceFormatClass::Other);

    #[test]
    fn srgb_preference_skips_an_earlier_unorm_format() {
        // Typical Android driver order: UNORM first, SRGB also available.
        let picked = select_surface_format(&[UNORM, SRGB, OTHER], true, classify);
        assert_eq!(picked, Some(SRGB));
    }

    #[test]
    fn unorm_preference_skips_an_earlier_srgb_format() {
        let picked = select_surface_format(&[SRGB, UNORM], false, classify);
        assert_eq!(picked, Some(UNORM));
    }

    #[test]
    fn falls_back_to_the_first_format_when_nothing_preferred_is_offered() {
        let picked = select_surface_format(&[OTHER, SRGB], false, classify);
        assert_eq!(picked, Some(OTHER));
    }

    #[test]
    fn empty_offer_selects_nothing() {
        assert_eq!(select_surface_format(&[], true, classify), None);
    }
}
