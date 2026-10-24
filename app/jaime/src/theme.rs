use aimer::style::{
    AnimatedTheme, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontWeight, Stroke,
    TextDecoration, TextStyle, ThemeData,
};
use aimer::{Color, Colors, MarkdownTheme, SpanStyle, Widget};

/// The single visual theme used by Jaime's showcase shell and regular examples.
///
/// Examples should read this through [`ThemeData::of`] or use [`app_theme`] when
/// constructing a page before it has a build context. Keeping the palette here
/// gives the application one seam for changing its visual language without
/// scattering color literals across every example.
pub fn app_theme() -> ThemeData {
    ThemeData::dark()
        .primary_color(Color::Rgb(255, 107, 82))
        .on_primary_color(Color::WHITE)
        .background_color(Color::Rgb(25, 17, 16))
        .on_background_color(Color::Rgb(237, 226, 222))
        .surface_color(Color::Rgb(42, 28, 26))
        .on_surface_color(Color::Rgb(237, 226, 222))
}

/// Provides Jaime's application theme to a standalone example.
///
/// The showcase mounts this provider once at the application root. Individual
/// launch functions use the same helper so an example remains runnable by
/// itself without losing access to the shared theme.
pub fn provide<W: Widget + 'static>(child: W) -> impl Widget {
    AnimatedTheme::new()
        .data(app_theme())
        .child(child)
}

/// Derives the quieter text color from the active semantic foreground.
#[inline]
pub fn muted_text(theme: &ThemeData) -> Color {
    theme
        .on_background_color
        .with_alpha(0.72)
}

/// Derives a separator color that remains visible without competing with text.
#[inline]
pub fn divider(theme: &ThemeData) -> Color {
    theme
        .primary_color
        .with_alpha(0.32)
}

/// Derives a slightly raised surface for nested controls and sample cards.
#[inline]
pub fn raised_surface(theme: &ThemeData) -> Color {
    theme
        .surface_color
        .lighten(0.08)
}

/// Derives a subdued surface for fields and secondary samples.
#[inline]
pub fn recessed_surface(theme: &ThemeData) -> Color {
    theme
        .background_color
        .lighten(0.06)
}

/// Creates the normal decoration shared by Jaime's text inputs.
#[inline]
pub fn input_decoration(theme: &ThemeData) -> BoxDecoration {
    BoxDecoration::new()
        .background_color(recessed_surface(theme))
        .border(input_border(divider(theme), Stroke::Px(1.0)))
        .border_radius(10)
}

/// Creates the hover decoration shared by Jaime's text inputs.
#[inline]
pub fn input_hover_decoration(theme: &ThemeData) -> BoxDecoration {
    BoxDecoration::new()
        .background_color(raised_surface(theme))
        .border(input_border(
            theme
                .primary_color
                .with_alpha(0.64),
            Stroke::Px(1.0),
        ))
        .border_radius(10)
}

/// Creates the focused decoration shared by Jaime's text inputs.
#[inline]
pub fn input_focus_decoration(theme: &ThemeData) -> BoxDecoration {
    BoxDecoration::new()
        .background_color(raised_surface(theme))
        .border(input_border(theme.primary_color, Stroke::Px(1.5)))
        .border_radius(10)
}

/// Converts a themed [`Color`] into the legacy cursor-color representation.
#[inline]
pub fn input_cursor_color(theme: &ThemeData) -> Colors {
    Colors::Custom(
        theme
            .primary_color
            .as_u32(),
    )
}

fn input_border(color: Color, stroke: Stroke) -> BoxBorder {
    BoxBorder::all(
        BorderSlice::new()
            .style(BorderStyle::Solid)
            .stroke(stroke)
            .color(color),
    )
}

/// Creates the Markdown renderer theme from the same semantic palette.
pub fn markdown_theme() -> MarkdownTheme {
    let theme = app_theme();
    let body = TextStyle::new()
        .font_size(16)
        .color(theme.on_surface_color);
    let markdown_defaults = MarkdownTheme::default();

    markdown_defaults
        .body(body)
        .headings([
            body.font_size(32)
                .font_weight(FontWeight::Bolder),
            body.font_size(28)
                .font_weight(FontWeight::Bolder),
            body.font_size(24)
                .font_weight(FontWeight::Bolder),
            body.font_size(20)
                .font_weight(FontWeight::Bolder),
            body.font_size(18)
                .font_weight(FontWeight::Bolder),
            body.font_size(16)
                .font_weight(FontWeight::Bolder),
        ])
        .blockquote(body.color(muted_text(&theme)))
        .code_block(
            markdown_defaults
                .code_block
                .color(theme.on_surface_color)
                .font_size(14),
        )
        .inline_code(
            markdown_defaults
                .inline_code
                .background_color(recessed_surface(&theme)),
        )
        .link(
            SpanStyle::new()
                .color(theme.primary_color)
                .text_decoration(TextDecoration::Underline),
        )
        .link_hover_color(
            theme
                .primary_color
                .lighten(0.22),
        )
        .code_background(recessed_surface(&theme))
        .quote_background(theme.surface_color)
        .rule_color(divider(&theme))
        .table_header_background(theme.surface_color)
        .table_cell_background(recessed_surface(&theme))
        .keyword_color(
            theme
                .primary_color
                .lighten(0.10),
        )
        .string_color(theme.primary_color)
        .comment_color(muted_text(&theme))
        .number_color(
            theme
                .primary_color
                .lighten(0.22),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_theme_defines_one_branded_semantic_palette() {
        let theme = app_theme();

        assert_eq!(theme.primary_color, Color::Rgb(255, 107, 82));
        assert_eq!(theme.background_color, Color::Rgb(25, 17, 16));
        assert_eq!(theme.surface_color, Color::Rgb(42, 28, 26));
        assert_eq!(theme.on_background_color, theme.on_surface_color);
    }

    #[test]
    fn derived_roles_keep_the_palette_tied_to_the_active_theme() {
        let theme = app_theme();

        assert_eq!(
            divider(&theme),
            theme
                .primary_color
                .with_alpha(0.32)
        );
        assert_eq!(
            muted_text(&theme),
            theme
                .on_background_color
                .with_alpha(0.72)
        );
        assert_eq!(
            raised_surface(&theme),
            theme
                .surface_color
                .lighten(0.08)
        );
        assert_eq!(
            recessed_surface(&theme),
            theme
                .background_color
                .lighten(0.06)
        );
    }

    #[test]
    fn input_decorations_use_the_active_theme_roles() {
        let theme = app_theme();
        let normal = input_decoration(&theme);
        let hover = input_hover_decoration(&theme);
        let focus = input_focus_decoration(&theme);

        assert_eq!(
            normal
                .background_color
                .get(),
            Some(recessed_surface(&theme))
        );
        assert_eq!(
            normal
                .border
                .left
                .color,
            divider(&theme)
        );
        assert_eq!(
            hover
                .background_color
                .get(),
            Some(raised_surface(&theme))
        );
        assert_eq!(
            focus
                .border
                .left
                .color,
            theme.primary_color
        );
        assert_eq!(
            input_cursor_color(&theme).as_u32(),
            theme
                .primary_color
                .as_u32()
        );
    }

    #[test]
    fn markdown_theme_uses_the_same_semantic_foreground_and_surfaces() {
        let theme = app_theme();
        let markdown = markdown_theme();
        let defaults = MarkdownTheme::default();

        assert_eq!(markdown.body.color, theme.on_surface_color);
        assert_eq!(markdown.code_background, recessed_surface(&theme));
        assert_eq!(markdown.table_cell_background, recessed_surface(&theme));
        assert_eq!(
            markdown
                .code_block
                .font_family,
            defaults
                .code_block
                .font_family
        );
        assert_eq!(
            markdown
                .inline_code
                .font_family,
            defaults
                .inline_code
                .font_family
        );
    }
}
