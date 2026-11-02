//! Phase 6 cross-script verification using only checked-in readable fonts.
//!
//! The matrix deliberately covers scripts that the repository's bundled
//! assets claim to support. System fallback and Apple-private color faces are
//! tested separately because their bytes and raster output are host-owned.

use super::glyph_rasterizer::{GlyphKey, GlyphRasterizer, RasterizedGlyph};
use super::text_layout::{ShapedText, layout_shaped_text, shape_text_styled};
use crate::font::{FontFamily, FontStyle, FontWeight};

const CJK_FONT: &[u8] = include_bytes!("../fonts/NotoSansJP-VariableFont_wght.ttf");

fn bundled_matrix_rasterizer() -> GlyphRasterizer {
    let mut rasterizer = GlyphRasterizer::primary_only();
    assert!(
        rasterizer.register_font_bytes(CJK_FONT.to_vec()).is_some(),
        "the checked-in CJK fallback must register for the Phase 6 matrix"
    );
    rasterizer
}

fn assert_source_ranges_are_valid(text: &str, shaped: &ShapedText, label: &str) {
    assert_eq!(shaped.text, text, "{label} changed the logical source text");
    for cluster in &shaped.clusters {
        let range = &cluster.text_range;
        assert!(
            range.start < range.end,
            "{label} emitted an empty source cluster: {range:?}"
        );
        assert!(
            text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
            "{label} split a UTF-8 scalar: {range:?}"
        );
        assert_eq!(
            text.get(range.clone()),
            Some(cluster.text.as_str()),
            "{label} lost the source text for {range:?}"
        );
        assert!(cluster.width.is_finite() && cluster.width >= 0.0);
        for glyph in &cluster.glyphs {
            assert_ne!(
                glyph.key.glyph_id, 0,
                "{label} resolved {:?} to .notdef",
                cluster.text
            );
        }
    }
}

fn bitmap_fingerprint(bitmap: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bitmap.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

fn raster_snapshot(glyph: &RasterizedGlyph) -> (u32, u32, i32, i32, u64) {
    (
        glyph.width,
        glyph.height,
        (glyph.offset_x * 1000.0).round() as i32,
        (glyph.offset_y * 1000.0).round() as i32,
        bitmap_fingerprint(&glyph.bitmap),
    )
}

#[test]
fn phase6_owned_font_matrix_preserves_clusters_and_rasterizes_supported_scripts() {
    let mut rasterizer = bundled_matrix_rasterizer();
    let cases = [
        ("Latin", "Aimer"),
        ("Greek", "Ελληνικά"),
        ("Cyrillic", "Русский"),
        ("Hebrew", "עברית"),
        ("Devanagari", "नमस्ते"),
        ("Thai", "สวัสดี"),
        ("Lao", "ສະບາຍດີ"),
        ("Khmer", "សួស្តីពិភពលោក"),
        ("CJK", "你好世界"),
        ("Combining", "e\u{301}"),
    ];

    for (label, text) in cases {
        let shaped = shape_text_styled(
            &mut rasterizer,
            text,
            24.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );
        assert!(!shaped.clusters.is_empty(), "{label} produced no clusters");
        assert_source_ranges_are_valid(text, &shaped, label);

        for cluster in &shaped.clusters {
            for glyph in &cluster.glyphs {
                let rendered = rasterizer.rasterize_key(glyph.key, 24.0);
                assert!(
                    !rendered.bitmap.is_empty(),
                    "{label} produced an empty bitmap for {:?}",
                    cluster.text
                );
                assert!(rendered.offset_x.is_finite() && rendered.offset_y.is_finite());
                assert!(rendered.advance_width.is_finite());
            }
        }
    }
}

#[test]
fn phase6_grayscale_raster_matrix_is_stable_across_device_scales_and_subpixel_phases() {
    let mut rasterizer = GlyphRasterizer::primary_only();
    let cases = [
        (16.0_f32, 1.0_f32, 0_u8, 0_u8),
        (16.0, 1.5, 2, 3),
        (16.0, 2.0, 4, 5),
        (24.0, 1.0, 7, 1),
        (24.0, 1.0, 0, 0),
    ];
    let actual = cases.map(|(logical_size, device_scale, subpixel_x, subpixel_y)| {
        let physical_size = logical_size * device_scale;
        let base = rasterizer.glyph_key_for_codepoint('A', physical_size);
        let key = GlyphKey {
            subpixel_x,
            subpixel_y,
            ..base
        };
        let glyph = rasterizer.rasterize_key(key, physical_size).clone();

        assert!(!glyph.is_color);
        assert_eq!(glyph.bitmap.len(), (glyph.width * glyph.height) as usize);
        assert!(glyph.bitmap.iter().any(|pixel| *pixel != 0));
        raster_snapshot(&glyph)
    });

    assert_eq!(
        actual,
        [
            (11, 12, 0, 0, 6_178_879_141_678_921_614),
            (16, 18, -250, -375, 1_722_626_256_821_631_889),
            (22, 24, -500, -625, 18_012_845_966_783_831_635),
            (16, 18, 125, -125, 5_544_611_616_453_397_900),
            (16, 18, 0, 0, 13_278_534_332_415_583_524),
        ]
    );
}

#[test]
fn phase6_baseline_boundaries_preserve_descenders_bearings_and_empty_glyphs() {
    let mut rasterizer = GlyphRasterizer::primary_only();

    let descender_key = rasterizer.glyph_key_for_codepoint('g', 24.0);
    let descender = rasterizer.rasterize_key(descender_key, 24.0).clone();
    assert!(
        !descender.bitmap.is_empty(),
        "the descender must remain visible"
    );
    assert!(
        descender.offset_y < 0.0,
        "the descender must retain its negative baseline bearing: {}",
        descender.offset_y
    );

    let bearing_keys = ['j', 'f', '(', 'A']
        .into_iter()
        .map(|codepoint| rasterizer.glyph_key_for_codepoint(codepoint, 24.0))
        .collect::<Vec<_>>();
    let bearings = bearing_keys
        .into_iter()
        .map(|key| rasterizer.rasterize_key(key, 24.0).clone())
        .collect::<Vec<_>>();
    assert!(
        bearings.iter().any(|glyph| glyph.offset_x < 0.0),
        "at least one punctuation/italic-side bearing must be allowed to extend left"
    );
    assert!(
        bearings
            .iter()
            .all(|glyph| glyph.offset_x.is_finite() && glyph.offset_y.is_finite())
    );

    let space_key = rasterizer.glyph_key_for_codepoint(' ', 24.0);
    let space = rasterizer.rasterize_key(space_key, 24.0).clone();
    assert!(space.bitmap.is_empty(), "space must stay an empty glyph");
    assert_eq!((space.width, space.height), (0, 0));
    assert!(
        space.advance_width > 0.0,
        "empty glyphs still advance the pen"
    );

    let shaped = shape_text_styled(
        &mut rasterizer,
        "g",
        24.0,
        FontFamily::SANS_SERIF,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    );
    let positioned = layout_shaped_text(&shaped, 0.0, 0.0, 0.0);
    assert_eq!(positioned.len(), 1);
    assert_eq!(
        positioned[0].y + positioned[0].height as f32,
        -shaped.clusters[0].glyphs[0].offset_y,
        "the bitmap bottom must stay anchored to the y=0 baseline"
    );
    assert!(
        positioned[0].y + positioned[0].height as f32 > 0.0,
        "a descender must be allowed below the baseline without clipping"
    );
}
