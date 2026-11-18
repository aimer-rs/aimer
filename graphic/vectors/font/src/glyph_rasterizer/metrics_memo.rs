//! The pixel boxes of the glyphs shaped lately, found without hashing.
//!
//! Positioning a run needs the box of each of its glyphs, and a text is a few dozen distinct
//! glyphs shaped some hundreds of times. The process-wide metrics store and the rasterizer's own
//! glyph cache each answer by hashing the whole key, and a run asks both for every glyph, so
//! most of what shaping a line of plain text costs is those lookups returning what they returned
//! for the line before. This is a small direct-mapped table in front of them: a glyph hashes to
//! one slot, and the slot either holds that glyph or does not. A collision costs the slot, not
//! correctness, since a miss goes on to the stores as before.

use super::GlyphKey;
use crate::glyph_metrics::GlyphMetrics;

/// A power of two, so a hash picks a slot with a mask. A page of text has far fewer distinct
/// glyphs than this.
const SLOTS: usize = 2048;

pub(super) struct MetricsMemo {
    slots: Vec<Option<(GlyphKey, GlyphMetrics)>>,
    /// How many boxes were found here, for the tests.
    #[cfg(test)]
    pub(super) hits: usize,
}

impl Default for MetricsMemo {
    fn default() -> Self {
        Self {
            slots: vec![None; SLOTS],
            #[cfg(test)]
            hits: 0,
        }
    }
}

#[inline]
fn slot_of(key: &GlyphKey) -> usize {
    // A multiply and a shift of every field that tells two glyphs apart.
    let mut hash = u64::from(key.font_id);
    hash = (hash << 16 | u64::from(key.glyph_id)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    hash = (hash ^ (u64::from(key.size_tenths) << 24 | u64::from(key.weight) << 8 | u64::from(key.subpixel_x) << 4 | u64::from(key.subpixel_y)))
        .wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    hash = (hash ^ u64::from(key.variation_id)).wrapping_mul(0x1656_67B1_9E37_79F9);
    (hash >> 40) as usize & (SLOTS - 1)
}

impl MetricsMemo {
    /// The box remembered for `key`, if it is the glyph its slot holds.
    #[inline]
    pub(super) fn get(&mut self, key: &GlyphKey) -> Option<GlyphMetrics> {
        let found = self.slots[slot_of(key)].as_ref().filter(|(held, _)| held == key).map(|(_, metrics)| *metrics);
        #[cfg(test)]
        if found.is_some() {
            self.hits += 1;
        }
        found
    }

    /// Remembers `metrics` for `key`, in place of whatever held its slot.
    #[inline]
    pub(super) fn put(&mut self, key: GlyphKey, metrics: GlyphMetrics) {
        self.slots[slot_of(&key)] = Some((key, metrics));
    }

    /// Forgets everything, for when the faces the keys name may have changed.
    pub(super) fn clear(&mut self) {
        self.slots.fill(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(width: u32) -> GlyphMetrics {
        GlyphMetrics { width, height: 12, offset_x: 1.0, offset_y: -2.0, advance_width: 9.5, is_color: false }
    }

    #[test]
    fn a_glyph_is_found_where_it_was_put_and_not_when_anything_about_it_differs() {
        let mut memo = MetricsMemo::default();
        let key = GlyphKey::new(1, 40, 16.0);
        memo.put(key, metrics(8));

        assert_eq!(memo.get(&key), Some(metrics(8)));
        assert_eq!(memo.get(&GlyphKey::new(2, 40, 16.0)), None, "another face");
        assert_eq!(memo.get(&GlyphKey::new(1, 41, 16.0)), None, "another glyph");
        assert_eq!(memo.get(&GlyphKey::new(1, 40, 17.0)), None, "another size");
        assert_eq!(memo.get(&key.weighted(700)), None, "another weight");
        assert_eq!(memo.get(&key.with_variation_id(3)), None, "another variation");
        let mut shifted = key;
        shifted.subpixel_x = 2;
        assert_eq!(memo.get(&shifted), None, "another subpixel phase");
    }

    #[test]
    fn two_glyphs_that_share_a_slot_never_answer_for_each_other() {
        let mut memo = MetricsMemo::default();
        // Find two keys with one slot among many.
        let mut seen = std::collections::HashMap::new();
        let (first, second) = (0..10_000u16)
            .map(|glyph| GlyphKey::new(1, glyph, 16.0))
            .find_map(|key| seen.insert(slot_of(&key), key).map(|earlier| (earlier, key)))
            .expect("ten thousand glyphs fill two thousand slots");

        memo.put(first, metrics(1));
        memo.put(second, metrics(2));

        assert_eq!(memo.get(&second), Some(metrics(2)));
        assert_eq!(memo.get(&first), None, "its slot was taken, so it is asked of the stores again");
    }

    #[test]
    fn clearing_forgets_every_glyph() {
        let mut memo = MetricsMemo::default();
        let key = GlyphKey::new(1, 40, 16.0);
        memo.put(key, metrics(8));
        memo.clear();
        assert_eq!(memo.get(&key), None);
    }

    /// What shaping a text gives must not depend on whether the boxes came from the memo, the
    /// stores behind it, or the rasterizer: the same boxes in the same places, to the pixel.
    #[test]
    fn shaping_gives_the_same_boxes_whether_or_not_the_memo_holds_them() {
        use crate::font::{FontFamily, FontStyle, FontWeight};
        use crate::text_layout::shape_text_styled;

        let text = "Hello, world! Wavy AVA fi ffl.\nBr\u{fb}l\u{e9}e \u{43f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{4f60}\u{597d} \u{3053}\u{3093}\u{306b}\u{3061}\u{306f} \u{1f680}\n\n last";
        let mut rasterizer = super::super::GlyphRasterizer::new();
        let shape = |rasterizer: &mut super::super::GlyphRasterizer| {
            let shaped = shape_text_styled(rasterizer, text, 16.0, FontFamily::SANS_SERIF, FontWeight::Normal, FontStyle::Normal, None);
            shaped
                .clusters
                .iter()
                .map(|cluster| format!("{:?} {:?} {} {:?}", cluster.text, cluster.text_range, cluster.width, cluster.glyphs))
                .collect::<Vec<_>>()
        };

        let cold = shape(&mut rasterizer);
        let hits = rasterizer.metrics_memo.hits;
        let from_the_memo = shape(&mut rasterizer);
        assert!(rasterizer.metrics_memo.hits > hits, "the second shaping used the memo");
        rasterizer.metrics_memo.clear();
        let from_the_stores = shape(&mut rasterizer);

        assert_eq!(from_the_memo, cold);
        assert_eq!(from_the_stores, cold);
    }
}
