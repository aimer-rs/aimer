• The highest-return optimization point is the Aimer rasterizer, not SFNT parsing or shaping.

  Current measurements show:

  - Direct shaping: Aimer is about 2.18× faster.
  - Whole-run shaping: effectively tied.
  - Rasterization one-at-a-time: Aimer is 1.22× slower.
  - Rasterization in runs: Aimer is 1.39× slower.

  See the latest comparison in zodiac_plans/FONT_RASTERIZING_BASELINE.md:727.

  ### Priority 1 — Replace the scan-conversion algorithm

  The main hotspot is aimer_cupid/src/pipeline/text_pipeline/aimer_font/rasterize.rs:926.

  For every glyph it currently performs:

  1. Four vertical samples per row.
  2. Active-edge copying.
  3. Floating-point intersection calculations.
  4. Intersection sorting for every sample row.
  5. Per-pixel coverage calculations.

  The largest improvement would come from replacing this with an active-edge scan converter:

  - Maintain active edges incrementally between scanlines.
  - Update x positions instead of recalculating them.
  - Avoid sorting when edge order remains unchanged.
  - Accumulate coverage directly into spans.
  - Keep the 4× quality profile for smooth rendering.

  This is the optimization most likely to make Aimer faster than Swash.

  ### Priority 2 — Remove per-glyph temporary work

  aimer_cupid/src/pipeline/text_pipeline/aimer_font/rasterize.rs:572 currently:

  - Allocates a new bitmap with vec![0; bitmap_len].
  - Performs a second full bitmap scan with bitmap.iter().all(...).
  - May run an 8× fallback pass for blank glyphs.

  The blank check should become a flag returned directly from the scan converter. That removes one full
  bitmap traversal per glyph.

  Then add reusable bitmap buffers for temporary and evicted glyphs.

  ### Priority 3 — Batch cache operations

  aimer_cupid/src/pipeline/text_pipeline/aimer_font.rs:780 still calls the single-glyph rasterizer for every
  glyph.

  The current per-glyph path performs local cache access plus shared cache access for outlines and flattened
  edges. A batch API should:

  - Probe all outline keys under one shared lock.
  - Probe all flattened-path keys under one shared lock.
  - Rasterize only missing entries.
  - Avoid repeated Arc and Mutex operations.
  - Reuse one batch scratch structure.

  This matters especially because the benchmark creates a new rasterizer for each cold iteration.

  ### Priority 4 — Batch metric publication

  Every glyph currently calls crates/aimer_text/src/../aimer_cupid/src/pipeline/text_pipeline/
  glyph_metrics.rs:69 through the raster callback.

  That acquires the global write lock once per glyph. Replace it with:

  store_many(batch_metrics)

  and publish all metrics with one lock acquisition per raster batch.

  ### Priority 5 — Flatten outlines without intermediate vectors

  The CFF/TrueType path currently creates:

  contours -> Vec<Vec<Point>> -> Vec<Edge>

  For first-use rasterization, especially CFF/CFF2 and CJK, flatten directly into one reusable edge buffer.
  This will improve cold-start performance, but it is less important than the scan converter because
  flattened edges are already cached.

  ### Recommended next milestone

  Implement these in this order:

  1. Return has_coverage from the scan converter.
  2. Remove the full bitmap blank scan.
  3. Add direct row-edge iteration without copying into active_edges.
  4. Add batch shared-cache probes.
  5. Add batched metric publication.
  6. Replace the intersection-sort kernel with an incremental active-edge rasterizer.
  7. Re-run Latin, CJK, CFF, Arabic, and mixed-fallback benchmarks.

  The scan-converter rewrite is the point with the maximum performance potential. The other changes reduce
  overhead around it; they will help, but they cannot fully close the current rasterization gap alone.