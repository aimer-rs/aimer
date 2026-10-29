//! `#[aimer::assets]` showcase: typed handles for the files declared in
//! `Aimer.toml`, with the font loaded lazily on first use.

use aimer::style::{BoxFit, LayoutSpacing, Spacing, TextStyle, ThemeData};
use aimer::{
    AimerApp, AnyWidget, AssetImage, BundledAsset, Column, Container, Dimension,
    FontFamily, FontWeight, Row, SizedBox, SvgAsset, Text, Widget,
};

use crate::{Assets, theme};

/// One line of the "what the macro generated" table.
struct AssetRow {
    constant: &'static str,
    handle: BundledAsset,
}

/// Rows describing the `[assets]` handles used by the page. A font is listed
/// separately because it is a `BundledFont`, not a `BundledAsset`.
fn asset_rows() -> [AssetRow; 3] {
    [
        AssetRow {
            constant: "Assets::MY_IMAGE",
            handle: Assets::MY_IMAGE,
        },
        AssetRow {
            constant: "Assets::CAT_SVGREPO_COM",
            handle: Assets::CAT_SVGREPO_COM,
        },
        AssetRow {
            constant: "Assets::JAIME_MD",
            handle: Assets::JAIME_MD,
        },
    ]
}

/// The text describing a handle: its const, manifest path, and detected kind.
fn describe(row: &AssetRow) -> String {
    format!(
        "{}  →  {}  ({:?})",
        row.constant,
        row.handle.path(),
        row.handle.kind()
    )
}

fn heading(text: &'static str, app_theme: &ThemeData) -> AnyWidget {
    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(18)
                .font_weight(FontWeight::Bold)
                .color(app_theme.on_background_color),
        )
        .boxed()
}

/// Builds the bundled-assets showcase without starting an application.
pub fn bundled_assets_example() -> impl Widget {
    let app_theme = theme::app_theme();
    let muted = theme::muted_text(&app_theme);

    let mut children: Vec<AnyWidget> = vec![
        heading("Typed handles from Aimer.toml", &app_theme),
        SizedBox::new().height(6).boxed(),
        Text::new("#[aimer::assets] turns [assets] and [fonts] into consts. A missing file is a compile error.")
            .text_style(TextStyle::new().font_size(14).color(muted))
            .boxed(),
        SizedBox::new().height(14).boxed(),
    ];
    for row in asset_rows() {
        children.push(
            Text::new(describe(&row))
                .text_style(
                    TextStyle::new()
                        .font_family(Assets::JETBRAINS_MONO.into())
                        .font_size(13)
                        .color(app_theme.primary_color),
                )
                .boxed(),
        );
    }

    children.extend([
        SizedBox::new().height(24).boxed(),
        heading("Image and SVG", &app_theme),
        SizedBox::new().height(10).boxed(),
        Row::new()
            .children([
                Container::new()
                    .width(Dimension::Px(240.0))
                    .child(AssetImage::new(Assets::MY_IMAGE).fit(BoxFit::FitWidth))
                    .boxed(),
                SizedBox::new().width(24).boxed(),
                SvgAsset::new(Assets::CAT_SVGREPO_COM)
                    .width(120)
                    .height(120)
                    .boxed(),
            ])
            .boxed(),
        SizedBox::new().height(24).boxed(),
        heading("Font, loaded on first read", &app_theme),
        SizedBox::new().height(6).boxed(),
        Text::new("Converting Assets::JETBRAINS_MONO into a FontFamily reads and registers it once; if that fails the text falls back to sans-serif.")
            .text_style(TextStyle::new().font_size(14).color(muted))
            .boxed(),
        SizedBox::new().height(10).boxed(),
        Text::new("The quick brown fox jumps over the lazy dog.\n0123456789  {}[]() =>")
            .text_style(
                TextStyle::new()
                    .font_family(FontFamily::from(Assets::JETBRAINS_MONO))
                    .font_size(22)
                    .color(app_theme.on_background_color),
            )
            .boxed(),
    ]);

    Container::new()
        .color(app_theme.background_color)
        .padding(LayoutSpacing::all(Spacing::Px(32)))
        .child(Column::new().children(children))
}

pub fn start_bundled_assets_example() {
    AimerApp::start(theme::provide(bundled_assets_example()))
}

#[cfg(test)]
mod tests {
    use aimer::AssetKind;

    use super::*;

    #[test]
    fn the_macro_generated_a_handle_for_every_manifest_entry_the_page_uses() {
        assert_eq!(Assets::MY_IMAGE.path(), "assets/my_image.png");
        assert_eq!(Assets::MY_IMAGE.kind(), AssetKind::Image);
        assert_eq!(Assets::CAT_SVGREPO_COM.kind(), AssetKind::Svg);
        assert_eq!(Assets::JAIME_MD.kind(), AssetKind::Other);
        assert_eq!(
            Assets::JETBRAINS_MONO.path(),
            "assets/JetBrainsMono-Regular.ttf"
        );
    }

    #[test]
    fn rows_describe_const_path_and_kind() {
        let lines: Vec<String> = asset_rows().iter().map(describe).collect();

        assert_eq!(
            lines[0],
            "Assets::MY_IMAGE  →  assets/my_image.png  (Image)"
        );
        assert!(lines[1].ends_with("(Svg)"), "{}", lines[1]);
    }

    #[test]
    fn the_bundled_font_loads_lazily_from_the_project_directory() {
        // `cargo test` runs from the crate root, where the manifest paths resolve.
        assert_eq!(Assets::JETBRAINS_MONO.registered(), None);

        let family = FontFamily::from(Assets::JETBRAINS_MONO);

        assert_ne!(family, FontFamily::SANS_SERIF);
        assert_eq!(Assets::JETBRAINS_MONO.registered(), Some(family));
    }
}
