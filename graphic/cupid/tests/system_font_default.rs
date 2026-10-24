//! Checks the target-specific shipped font profiles: native builds use a host
//! face by default, while wasm builds keep a readable generic face locally
//! because browsers do not expose font files to Cupid.

#[cfg(all(not(feature = "bundled-fonts"), not(target_arch = "wasm32")))]
#[test]
fn default_profile_uses_a_host_font_for_basic_latin() {
    assert!(aimer_cupid::font::bundled_monospace_bytes().is_empty());

    let mut rasterizer = aimer_cupid::glyph_rasterizer::GlyphRasterizer::new();
    let key = rasterizer.glyph_key_for_codepoint('A', 16.0);
    assert_ne!(
        key.glyph_id, 0,
        "the host should provide a basic Latin face"
    );

    let glyph = rasterizer.rasterize_key(key, 16.0);
    assert!(
        glyph.width > 0 && glyph.height > 0 && !glyph.bitmap.is_empty(),
        "the host font's Latin glyph should rasterize"
    );
}

#[cfg(target_arch = "wasm32")]
#[test]
fn wasm_default_profile_rasterizes_basic_latin() {
    let mut rasterizer = aimer_cupid::glyph_rasterizer::GlyphRasterizer::new();
    let key = rasterizer.glyph_key_for_codepoint('A', 16.0);
    assert_ne!(key.glyph_id, 0, "wasm needs an embedded basic Latin face");

    let glyph = rasterizer.rasterize_key(key, 16.0);
    assert!(
        glyph.width > 0 && glyph.height > 0 && !glyph.bitmap.is_empty(),
        "the wasm default face's Latin glyph should rasterize"
    );
}
