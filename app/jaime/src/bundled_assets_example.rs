//! `#[aimer::assets]` showcase: typed handles for the files declared in
//! `Aimer.toml`, with the font loaded lazily on first use.

use aimer::style::{BoxFit, LayoutSpacing, Spacing, TextStyle, ThemeData};
use aimer::{
    AimerApp, AnyWidget, AssetImage, BundleAsset, Column, Container, Dimension,
    FontFamily, FontWeight, Row, SizedBox, SvgAsset, Text, Widget,
};

use crate::{Assets, theme};

/// One line of the "what the macro generated" table.
struct AssetRow {
    constant: &'static str,
    /// The `Bundle*` type the macro chose for the entry.
    bundle: &'static str,
    path: &'static str,
}

/// Describes `asset` for the table. Generic over [`BundleAsset`], which is what
/// lets one helper take every kind of bundled file.
fn row(constant: &'static str, bundle: &'static str, asset: impl BundleAsset) -> AssetRow {
    AssetRow {
        constant,
        bundle,
        path: asset.path(),
    }
}

/// Rows describing the handles used by the page, one per kind of file.
fn asset_rows() -> [AssetRow; 4] {
    [
        row("Assets::MY_IMAGE", "BundleImage", Assets::MY_IMAGE),
        row("Assets::CAT_SVGREPO_COM", "BundleSvg", Assets::CAT_SVGREPO_COM),
        row("Assets::JAIME_MD", "BundleDocuments", Assets::JAIME_MD),
        row("Assets::JETBRAINS_MONO", "BundleFont", Assets::JETBRAINS_MONO),
    ]
}

/// The text describing a handle: its const, manifest path, and generated type.
fn describe(row: &AssetRow) -> String {
    format!("{}  →  {}  ({})", row.constant, row.path, row.bundle)
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
                        .font_family(Assets::JETBRAINS_MONO)
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
        Text::new("Passing Assets::JETBRAINS_MONO to TextStyle::font_family reads and registers it once; if that fails the text falls back to sans-serif.")
            .text_style(TextStyle::new().font_size(14).color(muted))
            .boxed(),
        SizedBox::new().height(10).boxed(),
        Text::new("The quick brown fox jumps over the lazy dog.\n0123456789  {}[]() =>")
            .text_style(
                TextStyle::new()
                    .font_family(Assets::JETBRAINS_MONO)
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
    use aimer::{BundleDocuments, BundleFont, BundleImage, BundleSvg};

    use super::*;

    /// Each entry of `Aimer.toml` became the `Bundle*` type of its extension. The
    /// annotations are the test: they do not compile for any other type.
    #[test]
    fn the_macro_generated_the_bundle_type_of_each_file() {
        let image: BundleImage = Assets::MY_IMAGE;
        let svg: BundleSvg = Assets::CAT_SVGREPO_COM;
        let document: BundleDocuments = Assets::JAIME_MD;
        let font: BundleFont = Assets::JETBRAINS_MONO;

        assert_eq!(image.path(), "assets/my_image.png");
        assert_eq!(svg.path(), "assets/cat-svgrepo-com.svg");
        assert_eq!(document.path(), "assets/JAIME.md");
        assert_eq!(font.path(), "assets/JetBrainsMono-Regular.ttf");
    }

    #[test]
    fn rows_describe_const_path_and_generated_type() {
        let lines: Vec<String> = asset_rows().iter().map(describe).collect();

        assert_eq!(
            lines[0],
            "Assets::MY_IMAGE  →  assets/my_image.png  (BundleImage)"
        );
        assert!(lines[1].ends_with("(BundleSvg)"), "{}", lines[1]);
        assert!(lines[2].ends_with("(BundleDocuments)"), "{}", lines[2]);
        assert!(lines[3].ends_with("(BundleFont)"), "{}", lines[3]);
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
