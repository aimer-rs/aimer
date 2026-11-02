//! Theme lookups and the colors every control derives from them.

use aimer_provider::ProviderContext;
use aimer_style::{TextStyle, ThemeData, ThemeTokens, apply_state_layer};
use aimer_widget::base::{BuildContext, Color};

use crate::interaction::Flags;

/// The active theme's tokens, or the light defaults outside a themed tree.
#[inline]
pub(crate) fn tokens(ctx: &BuildContext) -> ThemeTokens {
    ctx.try_copied::<ThemeData>()
        .map(|theme| theme.tokens())
        .unwrap_or_else(ThemeTokens::light)
}

/// Lays the hover, press, or disabled state layer over `base`.
///
/// Press wins over hover; disabled wins over both.
pub(crate) fn with_state_layer(
    tokens: &ThemeTokens,
    base: Color,
    flags: Flags,
    disabled: bool,
) -> Color {
    if disabled {
        apply_state_layer(base, tokens.state.disabled)
    } else if flags.pressed {
        apply_state_layer(base, tokens.state.pressed)
    } else if flags.hovered {
        apply_state_layer(base, tokens.state.hover)
    } else {
        base
    }
}

/// The background of a control's shell.
#[inline]
pub(crate) fn surface_color(tokens: &ThemeTokens, flags: Flags, disabled: bool) -> Color {
    with_state_layer(tokens, tokens.colors.surface, flags, disabled)
}

/// The text style of a control's label.
pub(crate) fn label_style(tokens: &ThemeTokens, disabled: bool, highlighted: bool) -> TextStyle {
    let color = if disabled {
        apply_state_layer(tokens.colors.on_surface, tokens.state.disabled)
    } else if highlighted {
        tokens.colors.primary
    } else {
        tokens.colors.on_surface
    };
    TextStyle::new()
        .font_size(tokens.typography.body.font_size as u32)
        .color(color)
}
