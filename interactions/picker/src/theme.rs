//! Semantic-token resolution shared by picker widget surfaces.

use aimer_provider::ProviderContext;
use aimer_style::{ThemeData, ThemeTokens};
use aimer_widget::base::BuildContext;

/// Resolves picker tokens from the most specific provider available.
///
/// `ThemeTokens` is preferred for component-level styling. `ThemeData` remains
/// a compatibility bridge for applications that provide the original core
/// palette, and the built-in light tokens keep a standalone picker usable in
/// a headless or otherwise unthemed tree.
pub(crate) fn tokens(ctx: &BuildContext) -> ThemeTokens {
    ctx.try_copied::<ThemeTokens>()
        .or_else(|| ctx.try_copied::<ThemeData>().map(|theme| theme.tokens()))
        .unwrap_or_else(ThemeTokens::light)
        .normalized()
}
