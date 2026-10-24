use aimer_color::prelude::Color;

/// Composites a declared solid surface over its inherited paint background.
///
/// Transparent represents an unknown background. This metadata follows solid
/// surfaces; it does not sample images, gradients, or backdrop pixels.
#[doc(hidden)]
#[inline]
pub fn composite_background(foreground: Option<Color>, background: Color) -> Color {
    let Some(foreground) = foreground else {
        return background;
    };
    let [alpha, r, g, b] = foreground.as_u32().to_be_bytes();
    if alpha == 0 {
        return background;
    }
    let [ba, br, bg, bb] = background.as_u32().to_be_bytes();
    if alpha == 255 || ba == 0 {
        return foreground;
    }
    let front = u32::from(alpha) * 255;
    let back = u32::from(ba) * (255 - u32::from(alpha));
    let total = front + back;
    let channel = |value: u8, below: u8| {
        ((u32::from(value) * front + u32::from(below) * back + total / 2) / total) as u8
    };
    Color::Rgba(
        channel(r, br),
        channel(g, bg),
        channel(b, bb),
        ((total + 127) / 255) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_background_metadata_uses_source_over_alpha() {
        assert_eq!(composite_background(None, Color::WHITE), Color::WHITE);
        assert_eq!(
            composite_background(Some(Color::Transparent), Color::WHITE),
            Color::WHITE
        );
        assert_eq!(
            composite_background(Some(Color::BLACK), Color::WHITE),
            Color::BLACK
        );
        assert_eq!(
            composite_background(Some(Color::Rgba(0, 0, 0, 192)), Color::WHITE),
            Color::Rgb(63, 63, 63)
        );
        assert_eq!(
            composite_background(
                Some(Color::Rgba(255, 0, 0, 128)),
                Color::Rgba(0, 0, 255, 128)
            ),
            Color::Rgba(170, 0, 85, 192)
        );
    }
}
