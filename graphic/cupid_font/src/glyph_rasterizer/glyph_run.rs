//! Grouping of fresh glyphs into the units one scaler draws.
//!
//! Turning a glyph into coverage is preceded by work that belongs to the
//! *face at a size*, not to the glyph: mapping the font file, parsing its
//! tables, building the scaler and its hinting instance, and reading the
//! metrics the advances come from. Asking for one glyph at a time pays all of
//! it once per glyph. Asking for a run of glyphs that share a face and a size
//! pays it once for the run.
//!
//! Runs are also the unit of parallelism, so they are capped: a page of text
//! is usually one face at one size, and a single unbounded run would leave
//! every worker but one idle.

use super::GlyphKey;

/// Glyphs that share a face and a size, in the order they are drawn.
///
/// The invariant every consumer relies on: `keys` is non-empty, and every key
/// in it names the same `font_id` and is drawn at `font_size`. That is what
/// makes one resolved face and one scaler enough for the whole run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GlyphRun {
    pub(crate) font_size: f32,
    pub(crate) keys: Vec<GlyphKey>,
}

impl GlyphRun {
    /// The face every glyph of this run is drawn from.
    #[inline]
    pub(crate) fn font_id(&self) -> super::FontId {
        self.keys[0].font_id
    }
}

/// How many glyphs one run may hold.
///
/// The per-run setup it amortizes is around half a microsecond, so a few dozen
/// glyphs already reduce it to noise — while keeping enough runs for the
/// worker pool to spread a freshly scrolled paragraph or code block across its
/// threads. Raising this trades parallelism for a saving that is already spent.
pub(crate) const MAX_GLYPHS_PER_RUN: usize = 32;
/// Maximum sequential batch size used by the synchronous preload path.
///
/// Unlike worker preparation, this path does not need to preserve parallel
/// work stealing. A larger batch amortizes face lookup and cache insertion
/// overhead while the worker-facing limit above keeps parallel jobs balanced.
pub(crate) const SEQUENTIAL_MAX_GLYPHS_PER_RUN: usize = 128;

/// Groups `glyphs` into the runs a worker rasterizes.
///
/// This is [`group_into_runs`] at the length the renderer uses; the length is a
/// parameter there only so tests can state it.
#[inline]
pub(crate) fn glyph_runs(glyphs: Vec<(GlyphKey, f32)>) -> Vec<GlyphRun> {
    group_into_runs(glyphs, MAX_GLYPHS_PER_RUN)
}

/// Groups `glyphs` into runs sharing a face and a size.
///
/// Each element is a glyph key with the size it is drawn at; the size travels
/// beside the key because [`GlyphKey::size_tenths`] is quantized and the
/// rasterizer draws at the exact size the layout asked for.
///
/// Grouping is a sort, so glyphs are not returned in the order they were
/// collected. Nothing downstream depends on that order: each result carries
/// the key it belongs to.
pub(crate) fn group_into_runs(mut glyphs: Vec<(GlyphKey, f32)>, max_run: usize) -> Vec<GlyphRun> {
    debug_assert!(max_run > 0, "a run must be able to hold a glyph");

    glyphs.sort_unstable_by_key(|(key, font_size)| {
        (key.font_id, font_size.to_bits(), key.glyph_id, key.weight)
    });

    let mut runs: Vec<GlyphRun> = Vec::new();
    for (key, font_size) in glyphs {
        let joins_last = runs.last().is_some_and(|run| {
            run.keys.len() < max_run
                && run.font_id() == key.font_id
                && run.font_size.to_bits() == font_size.to_bits()
        });
        if joins_last {
            // The check above proved there is a last run.
            if let Some(run) = runs.last_mut() {
                run.keys.push(key);
            }
        } else {
            runs.push(GlyphRun {
                font_size,
                keys: vec![key],
            });
        }
    }

    runs
}

#[cfg(test)]
mod tests {
    use super::{GlyphKey, MAX_GLYPHS_PER_RUN, group_into_runs};

    fn key(font_id: u32, glyph_id: u16, font_size: f32) -> (GlyphKey, f32) {
        (GlyphKey::new(font_id, glyph_id, font_size), font_size)
    }

    #[test]
    fn nothing_to_rasterize_is_no_work() {
        assert!(group_into_runs(Vec::new(), MAX_GLYPHS_PER_RUN).is_empty());
    }

    #[test]
    fn glyphs_of_one_face_at_one_size_share_a_run() {
        let glyphs = (0..8).map(|index| key(0, index, 16.0)).collect();

        let runs = group_into_runs(glyphs, MAX_GLYPHS_PER_RUN);

        assert_eq!(runs.len(), 1, "one face at one size is one setup");
        assert_eq!(runs[0].keys.len(), 8);
        assert_eq!(runs[0].font_size, 16.0);
    }

    #[test]
    fn a_run_never_mixes_faces_or_sizes() {
        let glyphs = vec![
            key(0, 1, 16.0),
            key(7, 1, 16.0),
            key(0, 2, 18.0),
            key(7, 2, 18.0),
            key(0, 3, 16.0),
        ];

        let runs = group_into_runs(glyphs, MAX_GLYPHS_PER_RUN);

        for run in &runs {
            assert!(
                run.keys.iter().all(|key| key.font_id == run.font_id()
                    && key.size_tenths == (run.font_size * 10.0) as u32),
                "one scaler draws one face at one size"
            );
        }
        assert_eq!(
            runs.len(),
            4,
            "two faces at two sizes cannot share a scaler"
        );
    }

    #[test]
    fn a_long_stretch_is_split_so_workers_have_something_to_take() {
        let glyphs = (0..MAX_GLYPHS_PER_RUN as u16 * 3 + 1)
            .map(|index| key(0, index, 16.0))
            .collect();

        let runs = group_into_runs(glyphs, MAX_GLYPHS_PER_RUN);

        assert_eq!(runs.len(), 4);
        assert!(
            runs.iter().all(|run| run.keys.len() <= MAX_GLYPHS_PER_RUN),
            "an unbounded run would leave every worker but one idle"
        );
    }

    #[test]
    fn every_glyph_is_rasterized_exactly_once() {
        let glyphs: Vec<_> = (0..70u16)
            .map(|index| key(u32::from(index % 3), index, 12.0 + f32::from(index % 2)))
            .collect();
        let mut expected: Vec<_> = glyphs.iter().map(|(key, _)| *key).collect();
        expected.sort_unstable_by_key(|key| (key.font_id, key.glyph_id));

        let runs = group_into_runs(glyphs, MAX_GLYPHS_PER_RUN);

        let mut grouped: Vec<_> = runs.into_iter().flat_map(|run| run.keys).collect();
        grouped.sort_unstable_by_key(|key| (key.font_id, key.glyph_id));
        assert_eq!(grouped, expected);
    }
}
