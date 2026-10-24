//! Phase 0 snapshots for the Aimer-owned font path.
//!
//! The snapshots intentionally use fonts checked into the repository. Host
//! system-font discovery is not part of the deterministic baseline and would
//! make a baseline depend on the machine running the test.

use super::glyph_rasterizer::{GlyphRasterizer, RasterizedGlyph};
use super::text_layout::{
    FontMetrics, ShapedText, TextLayoutOptions, layout_paragraph_with_shaper, layout_shaped_text,
    shape_text_styled,
};
use crate::font::{FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight};

const CJK_FONT: &[u8] = include_bytes!("../../../fonts/NotoSansJP-VariableFont_wght.ttf");
const LATIN_FONT: &[u8] = include_bytes!("../../../fonts/JetBrainsMono-Regular.ttf");
const RTL_FONT: &[u8] = include_bytes!("../../../fonts/GoogleSans-Regular.ttf");

fn baseline_rasterizer() -> (GlyphRasterizer, FontFamily) {
    let family = FontRegistry::register(FontRegistration {
        family: "aimer-phase0-baseline-latin",
        bytes: LATIN_FONT,
        weight: FontWeight::Normal,
        style: FontStyle::Normal,
    })
    .expect("the checked-in Latin baseline font must register");
    let mut rasterizer = GlyphRasterizer::primary_only();
    assert!(
        rasterizer.register_font_bytes(CJK_FONT.to_vec()).is_some(),
        "the checked-in CJK baseline font must be readable"
    );
    (rasterizer, family)
}

fn milli(value: f32) -> i32 {
    (value * 1000.0).round() as i32
}

fn bitmap_fingerprint(bitmap: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bitmap.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

fn glyph_snapshot(glyph: &RasterizedGlyph) -> (u32, u32, i32, i32, i32, u64) {
    (
        glyph.width,
        glyph.height,
        milli(glyph.offset_x),
        milli(glyph.offset_y),
        milli(glyph.advance_width),
        bitmap_fingerprint(&glyph.bitmap),
    )
}

fn shaped_snapshot(
    shaped: &ShapedText,
    latin_font_id: u32,
    cjk_font_id: u32,
) -> Vec<(String, u8, u16, i32, u32, u32, i32, i32)> {
    shaped
        .clusters
        .iter()
        .flat_map(|cluster| {
            cluster.glyphs.iter().map(|glyph| {
                let face = if glyph.key.font_id == latin_font_id {
                    0
                } else if glyph.key.font_id == cjk_font_id {
                    1
                } else {
                    u8::MAX
                };
                (
                    cluster.text.clone(),
                    face,
                    glyph.key.glyph_id,
                    milli(glyph.advance),
                    glyph.width,
                    glyph.height,
                    milli(glyph.offset_x),
                    milli(glyph.offset_y),
                )
            })
        })
        .collect()
}

#[test]
fn phase0_golden_snapshot() {
    let (mut rasterizer, family) = baseline_rasterizer();
    let latin = shape_text_styled(
        &mut rasterizer,
        "Aimer",
        16.0,
        family,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    );
    let cjk = shape_text_styled(
        &mut rasterizer,
        "あ你",
        16.0,
        family,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    );
    let combining = shape_text_styled(
        &mut rasterizer,
        "e\u{301}",
        16.0,
        family,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    );
    let mixed = shape_text_styled(
        &mut rasterizer,
        "A你B",
        15.5,
        family,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    );

    let latin_key = latin.clusters[0].glyphs[0].key;
    let cjk_key = cjk.clusters[0].glyphs[0].key;
    let latin_font_id = latin_key.font_id;
    let cjk_font_id = cjk_key.font_id;
    let latin_bitmap = rasterizer.rasterize_key(latin_key, 16.0).clone();
    let cjk_bitmap = rasterizer.rasterize_key(cjk_key, 16.0).clone();
    let latin_layout = layout_shaped_text(&latin, 0.0, 0.0, 0.0);
    let mixed_layout = layout_shaped_text(&mixed, 0.25, 0.5, 0.0);
    let rtl = layout_paragraph_with_shaper(
        "שלום",
        RTL_FONT,
        0,
        FontMetrics::new(12.0, -4.0, 2.0),
        TextLayoutOptions::new(16.0, 3.25, 1.5, 200.0),
    );
    let mut clipped_options = TextLayoutOptions::new(16.0, 0.0, 0.0, 0.0);
    clipped_options.max_height = 20.0;
    let clipped = layout_paragraph_with_shaper(
        "Ag\nnext",
        LATIN_FONT,
        0,
        FontMetrics::new(12.0, -4.0, 2.0),
        clipped_options,
    );

    rasterizer.reset_shape_call_count();
    rasterizer.reset_rasterize_call_count();
    let _ = shape_text_styled(
        &mut rasterizer,
        "Aimer",
        16.0,
        family,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    );
    let warm_shape_calls = rasterizer.shape_call_count();
    let warm_rasterize_calls = rasterizer.rasterize_call_count();
    let warm_bitmap_bytes = rasterizer.bitmap_cache_bytes();
    assert_eq!(
        shaped_snapshot(&latin, latin_font_id, cjk_font_id),
        vec![
            ("A".to_owned(), 0, 1, 9_600, 9, 12, 0, 0),
            ("i".to_owned(), 0, 255, 9_600, 8, 13, 1_000, 0),
            ("m".to_owned(), 0, 282, 9_600, 9, 9, 0, 0),
            ("e".to_owned(), 0, 225, 9_600, 8, 10, 1_000, -1_000,),
            ("r".to_owned(), 0, 320, 9_600, 8, 9, 1_000, 0),
        ]
    );
    assert_eq!(
        shaped_snapshot(&cjk, latin_font_id, cjk_font_id),
        vec![
            ("あ".to_owned(), 1, 1_203, 16_000, 14, 14, 1_000, -1_000,),
            ("你".to_owned(), 1, 2_578, 16_000, 16, 16, 0, -2_000),
        ]
    );
    assert_eq!(
        shaped_snapshot(&combining, latin_font_id, cjk_font_id),
        vec![
            ("e\u{301}".to_owned(), 0, 225, 9_600, 8, 10, 1_000, -1_000,),
            ("e\u{301}".to_owned(), 0, 1_643, 0, 4, 3, -6_000, 10_000,),
        ]
    );
    assert_eq!(
        shaped_snapshot(&mixed, latin_font_id, cjk_font_id),
        vec![
            ("A".to_owned(), 0, 1, 9_300, 9, 12, 0, 0),
            ("你".to_owned(), 1, 2_578, 15_500, 15, 15, 0, -2_000),
            ("B".to_owned(), 0, 26, 9_300, 8, 12, 1_000, 0),
        ]
    );
    assert_eq!(
        glyph_snapshot(&latin_bitmap),
        (9, 12, 0, 0, 9_600, 7_064_966_629_923_364_211,)
    );
    assert_eq!(
        glyph_snapshot(&cjk_bitmap),
        (14, 14, 1_000, -1_000, 16_000, 9_874_997_834_247_042_479,)
    );
    assert_eq!(
        latin_layout
            .iter()
            .map(|glyph| (milli(glyph.x), milli(glyph.y), glyph.width, glyph.height))
            .collect::<Vec<_>>(),
        vec![
            (0, -12_000, 9, 12),
            (10_600, -13_000, 8, 13),
            (19_200, -9_000, 9, 9),
            (29_800, -9_000, 8, 10,),
            (39_400, -9_000, 8, 9),
        ]
    );
    assert_eq!(
        mixed_layout
            .iter()
            .map(|glyph| (milli(glyph.x), milli(glyph.y), glyph.width, glyph.height))
            .collect::<Vec<_>>(),
        vec![
            (250, -11_500, 9, 12),
            (9_550, -12_500, 15, 15),
            (26_050, -11_500, 8, 12),
        ]
    );
    assert_eq!(
        rtl.glyphs
            .iter()
            .map(|glyph| (
                glyph.glyph_id,
                milli(glyph.x),
                milli(glyph.y),
                milli(glyph.advance)
            ))
            .collect::<Vec<_>>(),
        vec![
            (2_176, 3_250, 1_500, 11_440),
            (2_153, 14_690, 1_500, 3_936),
            (2_172, 18_626, 1_500, 9_040),
            (2_197, 27_666, 1_500, 13_200),
        ]
    );
    assert_eq!(
        rtl.lines
            .iter()
            .map(|line| (
                line.text_range.clone(),
                milli(line.baseline),
                milli(line.width)
            ))
            .collect::<Vec<_>>(),
        vec![(0..8, 1_500, 37_616)]
    );
    assert_eq!(
        clipped
            .glyphs
            .iter()
            .map(|glyph| (glyph.source.clone(), milli(glyph.x), milli(glyph.y)))
            .collect::<Vec<_>>(),
        vec![("A".to_owned(), 0, 0), ("g".to_owned(), 9_600, 0)]
    );
    assert_eq!(
        clipped
            .lines
            .iter()
            .map(|line| (
                line.text_range.clone(),
                line.glyph_range.clone(),
                milli(line.baseline)
            ))
            .collect::<Vec<_>>(),
        vec![(0..2, 0..2, 0), (3..7, 2..2, 18_000)]
    );
    assert_eq!(
        (
            warm_shape_calls,
            warm_rasterize_calls,
            warm_bitmap_bytes,
            rasterizer.cached_glyph_count(),
        ),
        (1, 0, 1_338, 11,)
    );
    assert_eq!(combining.clusters.len(), 1);
    assert_eq!(mixed.clusters.len(), 3);
    assert_eq!(mixed_layout.len(), 3);
    assert_eq!(rtl.lines.len(), 1);
    assert_eq!(clipped.lines.len(), 2);
    assert_eq!(
        clipped.lines[1].glyph_range.start,
        clipped.lines[1].glyph_range.end
    );
    assert!(latin_layout.iter().all(|glyph| glyph.y.is_finite()));
    assert!(mixed_layout.iter().all(|glyph| glyph.y.is_finite()));
}
