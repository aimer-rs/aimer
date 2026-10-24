# Font rasterizing Phase 0 baseline

Captured on 2026-08-29 from the feature-enabled current pipeline:

```text
rustc 1.98.0 (88d9e12ae 2026-08-18)
cargo 1.98.0 (797e8a9bc 2026-08-05)
arm64 macOS 27.0
```

The deterministic test input is checked into the repository:

- Latin: `aimer_cupid/fonts/JetBrainsMono-Regular.ttf`
- CJK: `aimer_cupid/fonts/NotoSansJP-VariableFont_wght.ttf`
- RTL: `aimer_cupid/fonts/GoogleSans-Regular.ttf`
- Latin sample: `Aimer` at 16 px
- CJK sample: `あ你` at 16 px
- Combining sample: `e\u{301}` at 16 px
- Mixed sample: `A你B` at 15.5 px
- RTL sample: `שלום` at 16 px using the checked-in Google Sans face

The executable golden snapshot is
[`phase0_golden_snapshot`](../aimer_cupid/src/pipeline/text_pipeline/phase0_baseline.rs).
It records glyph IDs, advances, bitmap bounds, baseline offsets, placement,
bitmap fingerprints, RTL line geometry, height clipping, and warm-cache
behavior. Bitmap fingerprints use FNV-1a over the exact coverage bytes.

## Deterministic snapshot

| Sample | Shaped result |
| --- | --- |
| Latin | `A/i/m/e/r` advances `9.600` px each; bitmap boxes `9x12`, `8x13`, `9x9`, `8x10`, `8x9` |
| CJK | `あ` advance `16.000` px, box `13x14`, offset `(2,-1)`; `你` advance `16.000` px, box `16x16`, offset `(0,-2)` |
| Combining | `e\u{301}` remains one grapheme cluster and produces one glyph with advance `9.600` px and box `8x14` |
| Mixed | `A`, `你`, and `B` resolve to Latin/CJK/Latin faces; advances are `9.300`, `15.500`, and `9.300` px |
| RTL | Hebrew glyph IDs `2197/2172/2153/2176` remain on one visual line, width `37.616` px, origin `(3.250, 1.500)` |
| Clipping | `Ag` is retained on the first baseline; the second line has no emitted glyphs when height is `20` px |
| Fractional placement | `A你B` at origin `(0.25, 0.5)` produces top-lefts `(0.250,-11.500)`, `(9.550,-12.500)`, `(26.050,-11.500)` |

Bitmap fingerprints for the normal `aimer-font` Aimer-owned rasterizer:

```text
Latin A: 9x12, offset=(0,0), advance=9.600, FNV-1a=17830188418017876574
CJK あ: 13x14, offset=(2,-1), advance=16.000, FNV-1a=1963485497925860274
```

Warm-cache observation after the samples have populated the local rasterizer:

```text
shape calls: 1
rasterize calls: 0
bitmap cache bytes: 1424
cached glyphs: 11
```

The `aimer-font-compare` mode retains the Phase 0 swash fingerprints and
bitmap-cache observation: Latin A `5327473767603998203`, CJK あ
`17720645733192242140`, and `1408` retained bitmap bytes.

## Host timing baseline

Commands were run in the debug profile. The rasterizer mode is named for each
measurement. These values are machine measurements, not golden correctness
criteria.

```text
text_shaping_benchmark 3 iterations
  2000 characters
  per-cluster average: 275.647403 ms
  per-run average:       9.574250 ms
  speedup:                 28.79x
  cold serial batch:     154.372569 ms
  cold parallel batch:    35.313791 ms
  cold batch speedup:       4.37x

glyph_rasterization_benchmark 3 iterations (`aimer-font-compare`)
  752 distinct glyphs
  one glyph at a time: 32.202305 ms (42.822 us/glyph)
  in runs:             22.235264 ms (29.568 us/glyph)
  saved:                         31%
```

The same benchmark with the Aimer-owned unhinted rasterizer (`aimer-font`)
measured:

```text
glyph_rasterization_benchmark 3 iterations
  752 distinct glyphs
  one glyph at a time: 69.433236 ms (92.331 us/glyph)
  in runs:             72.122736 ms (95.907 us/glyph)
  saved:                         -4%
```

These are debug-profile measurements on this host, not acceptance limits. The
Aimer scan converter is currently about 2.2x the legacy serial time and 3.2x
the legacy run time for this ASCII sample; optimization remains a separate
follow-up.

## Phase 2 unhinted quality gate

Captured on 2026-08-30 by
`phase2_unhinted_output_meets_reference_quality_target` in
[`glyph_rasterizer.rs`](../aimer_cupid/src/pipeline/text_pipeline/glyph_rasterizer.rs).
The test compares Aimer-owned unhinted coverage with the legacy swash hinted
coverage for the same glyph IDs at 9, 12, 16, 24, and 32 px. It uses 10
representative JetBrains Mono Latin codepoints (50 glyph samples) and 8
representative Noto Sans JP CJK codepoints (40 glyph samples).

| Face | Samples | Mean absolute coverage error | Maximum bound difference | Missing samples |
| --- | ---: | ---: | ---: | ---: |
| JetBrains Mono Latin | 50 | `0.0580` | `1 px` | `0` |
| Noto Sans JP CJK | 40 | `0.0156` | `0 px` | `0` |

The acceptance target is no missing coverage, a maximum one-pixel bound
difference, and mean absolute coverage error at most `0.30`; advances must stay
within `0.001 px`. All samples passed in both `aimer-font` and
`--all-features` builds. The result supports keeping Phase 2 unhinted and
deferring TrueType instruction execution unless exact platform hinting becomes
a product requirement.

The GPU frame benchmark was also attempted:

```text
text_resize_benchmark
  skipped: no GPU adapter available
```

Therefore no frame-time number is claimed for this host. Run the existing
resize and scroll benchmarks on a machine with an adapter before accepting a
GPU-performance comparison.

## Phase 4 Arabic shaping comparison

Captured on 2026-08-30 on the same `arm64 macOS 27.0` host with:

```text
rustc 1.98.0 (88d9e12ae 2026-08-18)
cargo 1.98.0 (797e8a9bc 2026-08-05)
```

The comparison is enabled explicitly with `aimer-font-compare`, which also
enables `aimer-font`. The shaping test uses a deterministic checked-in test
SFNT assembled by the inline fixture in
[`aimer_font.rs`](../aimer_cupid/src/pipeline/text_pipeline/aimer_font.rs).
It contains Arabic `isol`/`init`/`medi`/`fina`, `rlig`/`liga`, `mark`,
`mkmk`, `curs`, and format-1 `calt` records.

The focused comparison command was:

```text
cargo test -p aimer_cupid --features aimer-font-compare \
  compares_arabic_shaping_cases_with_harfrust -- --nocapture
```

Result:

```text
1 passed; 0 failed; 395 filtered out
```

For comparison, the Aimer low-level seam emits Arabic glyphs in logical order
while HarfRust's explicitly RTL buffer emits visual order. The harness reverses
only the HarfRust record list before comparing glyph IDs, source clusters,
advances, and offsets.

| Input | Glyph IDs/order | Clusters | Advances | Offsets | Result |
| --- | --- | --- | --- | --- | --- |
| `ببب` | match | match | match | match | joining forms agree |
| `بب` | match | match | match | match | required ligature agrees |
| `بَب` | match | differ | match | match | HarfRust canonicalizes the mark to the base cluster; Aimer retains the source-byte cluster |
| `بََب` | match | differ | match | differ | same mark-cluster difference; the synthetic mark-to-mark anchor result differs |
| `با` | match | match | differ | differ | cursive connection uses different advance/offset normalization |
| `اب` | match | match | match | match | contextual `calt` substitution agrees |

The differences are explicit expected report fields in the comparison test;
they are not hidden by relaxing glyph-ID or ordering checks. This baseline does
not claim full Arabic parity. Mark cluster canonicalization, standard cursive
advance normalization, and broader GSUB contextual formats remain follow-up
work before HarfRust can be removed.

The refreshed raster-quality comparison command was:

```text
cargo test -p aimer_cupid --features aimer-font-compare \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
```

It passed with the following output:

| Face | Samples | Mean absolute coverage error | Maximum bound difference | Missing samples |
| --- | ---: | ---: | ---: | ---: |
| JetBrains Mono Latin | 50 | `0.0580` | `1 px` | `0` |
| Noto Sans JP CJK | 40 | `0.0156` | `0 px` | `0` |

The complete comparison suite was also run:

```text
cargo test -p aimer_cupid --features aimer-font-compare --quiet
```

```text
392 passed; 0 failed; 4 ignored
5 doctests passed; 4 doctests ignored
```

The default build remains the compatibility configuration; this report covers
the opt-in comparison path only.

## Per-font parsed-face and layout-state cache refresh

Captured on 2026-08-30 after adding the `aimer-font` per-font cache. Each
`GlyphRasterizer` now retains an optional Aimer state keyed by `FontId`. A
successful entry owns a zero-copy `SfntFace<'static>` backed by the existing
`FontData`, the validated face metrics, and lazily initialized GDEF/GSUB/GPOS
state. The layout state stores owned GDEF classes and validated lookup
topology, feature selections, and subtable offsets; the table bytes remain in
the shared face storage. The lazy layout initialization keeps raster-only
calls from paying for GSUB/GPOS parsing. Failed Aimer face/layout parses are
also memoized, so malformed input does not trigger repeated parse work.

The cache is cleared with the other face-derived caches when a registered
face is replaced or removed. The borrowed `SfntFace::from_bytes` path remains
available for noncached parser helpers, so those callers do not copy their
input bytes.

The deterministic cache regression is:

```text
cargo test -p aimer_cupid --features aimer-font \
  aimer_font_state_is_reused_for_repeated_shape_calls -- --nocapture

1 passed; 0 failed
```

The following matched measurements used the existing debug-profile benchmark
examples with three iterations. The shaping legacy column is the default
build. The raster legacy column is `aimer-font-compare`, which keeps the same
feature-enabled build while selecting the legacy swash rasterizer. The Aimer
column is `aimer-font`.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Warm short-cluster shaping, 2,000 chars | `909.683 ms` | `42.952 ms` | Aimer `21.18x` faster |
| One whole shaped run, 2,000 chars | `39.087 ms` | `356.415 ms` | Aimer `9.12x` slower |
| Cold serial shaping batch | `604.684 ms` | `5,908.604 ms` | Aimer `9.77x` slower |
| Cold parallel shaping batch | `198.951 ms` | `1,210.534 ms` | Aimer `6.08x` slower |
| One-at-a-time rasterization, 752 glyphs | `99.023 ms` | `231.254 ms` | Aimer `2.34x` slower |
| Rasterization in runs, 752 glyphs | `53.370 ms` | `277.981 ms` | Aimer `5.21x` slower |

Commands and raw benchmark outputs:

```text
cargo run -p aimer_cupid --example text_shaping_benchmark -- 3
  per-cluster average: 909.682861ms
  per-run average:      39.086639ms
  cold serial batch:   604.683736ms
  cold parallel batch: 198.950652ms

cargo run -p aimer_cupid --features aimer-font --example text_shaping_benchmark -- 3
  per-cluster average:  42.951763ms
  per-run average:     356.415278ms
  cold serial batch:     5.908603902s
  cold parallel batch:   1.210534s

cargo run -p aimer_cupid --features aimer-font-compare \
  --example glyph_rasterization_benchmark -- 3
  one glyph at a time:  99.023041ms
  in runs:              53.369847ms

cargo run -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 3
  one glyph at a time: 231.253777ms
  in runs:             277.981319ms
```

The warm-cluster workload is the cache-sensitive case: one rasterizer shapes
many clusters and reuses the parsed face/layout state. The whole-run and cold
batch workloads construct a fresh rasterizer for each measured operation, so
they include the first-use Aimer parse and currently expose the cost of the
portable shaping implementation. Debug timings on this host were highly
variable between adjacent runs; these numbers are directional measurements,
not acceptance limits. The cache reuse and invalidation tests are the
deterministic gates. No GPU frame timing is claimed because this host has no
GPU adapter.

Final validation also reran the complete comparison-feature suite successfully:
`392 passed; 0 failed; 4 ignored`, plus `5` passing doctests and `4` ignored
doctests. The default non-feature suite reported two system-fallback failures
when run as a full parallel suite (`assigns_one_stable_id_per_face` and
`one_word_of_han_is_served_by_one_face`); each passes when run individually.
Those assertions are outside the Aimer font cache path, so they are recorded
as a host/system-fallback caveat rather than a cache failure.

## Incremental font-rasterization optimization checkpoints

Captured on 2026-08-30 with the same benchmark inputs and host as the cache
refresh above. Every debug checkpoint used three iterations. The `Legacy`
column is a separate run of the same example: default features for shaping and
`aimer-font-compare` for rasterization (the existing Swash comparison path).
The `Aimer` column uses `aimer-font`. Timings are directional because debug
builds are noisy; the raw values are retained so later changes can be compared
against the same scopes.

### Implemented slices

1. Parsed `cmap`, `hmtx`, metrics, and glyph lookup structures are memoized on
   `SfntFace`; the table directory is sorted for binary-search lookup, and
   format 0/4/12 cmap and expanded horizontal advances are retained.
2. GSUB/GPOS coverage, class, single, ligature, pair, cursive, and mark data
   are compiled into owned fast-path plans in `layout/compiled.rs`. Unsupported
   contextual forms continue through the checked raw fallback.
3. Each `AimerFontState` retains decoded outline paths and flattened, phase-
   keyed edge geometry, so repeated glyphs do not decode or flatten again.
4. The scalar converter now consumes prebuilt edges through reusable
   intersection/active-edge buffers and has a full-interior-pixel coverage
   fast path. The optimized converter is checked against the previous
   even-odd converter, including fractional edges and holes.

### Debug checkpoints (three iterations)

| Stage | Workload | Legacy | Aimer | Relative result |
| --- | --- | ---: | ---: | --- |
| 1 | Per-cluster shaping, 2,000 chars | `276.138 ms` | `14.514 ms` | Aimer `19.02x` faster |
| 1 | One whole shaped run, 2,000 chars | `10.030 ms` | `131.025 ms` | Aimer `13.06x` slower |
| 1 | Cold serial shaping batch | `173.409 ms` | `1,969.792 ms` | Aimer `11.36x` slower |
| 1 | Cold parallel shaping batch | `43.312 ms` | `307.273 ms` | Aimer `7.09x` slower |
| 1 | One-at-a-time rasterization, 752 glyphs | `299.613 ms` | `141.169 ms` | Aimer `2.12x` faster |
| 1 | Rasterization in runs, 752 glyphs | `41.260 ms` | `144.325 ms` | Aimer `3.50x` slower |
| 2 | Per-cluster shaping, 2,000 chars | `346.508 ms` | `72.776 ms` | Aimer `4.76x` faster |
| 2 | One whole shaped run, 2,000 chars | `10.740 ms` | `73.486 ms` | Aimer `6.84x` slower |
| 2 | Cold serial shaping batch | `169.610 ms` | `1,320.459 ms` | Aimer `7.79x` slower |
| 2 | Cold parallel shaping batch | `40.528 ms` | `215.392 ms` | Aimer `5.31x` slower |
| 2 | One-at-a-time rasterization, 752 glyphs | `284.943 ms` | `69.070 ms` | Aimer `4.13x` faster |
| 2 | Rasterization in runs, 752 glyphs | `51.090 ms` | `104.110 ms` | Aimer `2.04x` slower |
| 3 | Per-cluster shaping, 2,000 chars | `278.419 ms` | `75.380 ms` | Aimer `3.69x` faster |
| 3 | One whole shaped run, 2,000 chars | `9.816 ms` | `75.776 ms` | Aimer `7.72x` slower |
| 3 | Cold serial shaping batch | `159.912 ms` | `1,164.239 ms` | Aimer `7.28x` slower |
| 3 | Cold parallel shaping batch | `32.142 ms` | `234.789 ms` | Aimer `7.30x` slower |
| 3 | One-at-a-time rasterization, 752 glyphs | `293.180 ms` | `62.037 ms` | Aimer `4.73x` faster |
| 3 | Rasterization in runs, 752 glyphs | `48.607 ms` | `85.892 ms` | Aimer `1.77x` slower |
| 4 | Per-cluster shaping, 2,000 chars | `277.093 ms` | `76.941 ms` | Aimer `3.60x` faster |
| 4 | One whole shaped run, 2,000 chars | `9.797 ms` | `72.181 ms` | Aimer `7.37x` slower |
| 4 | Cold serial shaping batch | `157.698 ms` | `1,142.425 ms` | Aimer `7.24x` slower |
| 4 | Cold parallel shaping batch | `31.736 ms` | `216.711 ms` | Aimer `6.83x` slower |
| 4 | One-at-a-time rasterization, 752 glyphs | `262.998 ms` | `55.479 ms` | Aimer `4.74x` faster |
| 4 | Rasterization in runs, 752 glyphs | `46.889 ms` | `79.084 ms` | Aimer `1.69x` slower |

The stage-4 shaping rows are a final post-rasterizer checkpoint; stages 3 and
4 do not change the shaping algorithm. The raster improvement is consistent
with removing outline/flattening work from the hot path, while run-based
rasterization still has a remaining cost in the coverage pass and the broader
glyph-cache plumbing.

### Release comparison (five iterations)

The benchmark examples now run in either profile and print the active profile.
Both implementations execute the same 2,000-character shaping scopes and the
same 752-glyph, eight-size raster scopes; build time is outside the timers.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `9.145 ms` | `4.479 ms` | Aimer `2.04x` faster |
| One whole shaped run, 2,000 chars | `0.443 ms` | `2.547 ms` | Aimer `5.75x` slower |
| Cold serial shaping batch | `5.018 ms` | `25.111 ms` | Aimer `5.00x` slower |
| Cold parallel shaping batch | `21.617 ms` | `12.638 ms` | Aimer `1.71x` faster |
| One-at-a-time rasterization, 752 glyphs | `6.822 ms` | `6.504 ms` | Aimer `1.05x` faster |
| Rasterization in runs, 752 glyphs | `3.877 ms` | `11.647 ms` | Aimer `3.00x` slower |

Raw release commands and output:

```text
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 5
  per-cluster average: 9.145116ms
  per-run average:     443.341µs
  cold serial batch:   5.018016ms
  cold parallel batch: 21.617233ms

cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 5
  per-cluster average: 4.479258ms
  per-run average:     2.546508ms
  cold serial batch:   25.110866ms
  cold parallel batch: 12.638441ms

cargo run --release -p aimer_cupid --features aimer-font-compare \
  --example glyph_rasterization_benchmark -- 5
  one glyph at a time: 6.822141ms
  in runs:             3.877266ms

cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 5
  one glyph at a time: 6.504191ms
  in runs:             11.6473ms
```

Release timings show that the cache and rasterizer work have brought the
one-at-a-time raster scope to parity with the legacy implementation. Whole-run
and cold shaping remain slower because the portable GSUB/GPOS implementation
still pays for its own run construction and fallback checks. The run-based
raster scope remains the next performance target; it includes the end-to-end
`preload_text` path rather than only a pre-resolved `rasterize_key` call.

Validation for the optimization slices:

```text
cargo test -p aimer_cupid --features aimer-font \
  retains_decoded_outlines_and_flattened_edges_for_repeated_glyphs -- --nocapture
  1 passed; 0 failed

cargo test -p aimer_cupid --features aimer-font \
  optimized_scan_conversion_reuses_scratch_and_matches_reference -- --nocapture
  1 passed; 0 failed

cargo test -p aimer_cupid --features aimer-font --quiet
  394 passed; 0 failed; 4 ignored

cargo test -p aimer_cupid --features aimer-font-compare --quiet
  396 passed; 0 failed; 4 ignored
```

## Direct batch rasterization and compiled shaping plans

Date: 2026-08-30

This pass implements the next five optimization slices behind the existing
`aimer-font` feature. The legacy comparison column below is the
`aimer-font-compare` Swash path; it is not a Skrifa implementation.

### Implemented changes

1. `GlyphRasterizer` now sends a deduplicated pending run directly to
   `AimerFontState::rasterize_glyphs_into`. Successful glyphs are inserted into
   the final raster cache from the callback, avoiding the temporary
   `(glyph_id, phase)` list and `Vec<Option<RasterizedGlyph>>` result.
2. A complete Aimer run returns immediately. Missing-glyph construction and
   fallback probing are only performed for a partial run.
3. Pending keys, glyph IDs, prepared output, and fallback keys are retained in
   reusable run buffers. A reusable `HashSet<GlyphKey>` replaces linear
   `pending.contains()` deduplication.
4. Sequential preload runs use a 128-glyph limit, while worker/parallel runs
   retain the 32-glyph limit. This avoids turning `preload_text` into many
   small Aimer calls without increasing worker task size.
5. Parsed layout state now retains active GSUB/GPOS lookup plans per selected
   script/feature. The supported shaping paths consume those preselected
   lookup slices rather than resolving lookup indices for every glyph. Raw
   fallback remains available for unsupported contextual subtables.

### Debug checkpoint after the combined raster/run changes

Three iterations; debug timings are directional and noisy.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| One-at-a-time rasterization, 752 glyphs | `285.573 ms` | `57.270 ms` | Aimer `4.99x` faster |
| Rasterization in runs, 752 glyphs | `41.167 ms` | `72.259 ms` | Aimer `1.76x` slower |
| Per-cluster shaping, 2,000 chars | `295.163 ms` | `78.184 ms` | Aimer `3.77x` faster |
| One whole shaped run, 2,000 chars | `10.706 ms` | `77.656 ms` | Aimer `7.25x` slower |
| Cold serial shaping batch | `171.401 ms` | `1,241.856 ms` | Aimer `7.24x` slower |
| Cold parallel shaping batch | `38.073 ms` | `245.856 ms` | Aimer `6.46x` slower |

The shaping rows are a separate checkpoint for the compiled lookup-plan
change; the raster rows include the direct callback, run-buffer reuse, and
sequential-batch-size changes together.

### Release comparison after this pass

Five iterations using equivalent benchmark scopes. Build time is outside the
timers.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `9.855 ms` | `4.320 ms` | Aimer `2.28x` faster |
| One whole shaped run, 2,000 chars | `0.528 ms` | `2.433 ms` | Aimer `4.61x` slower |
| Cold serial shaping batch | `5.626 ms` | `28.792 ms` | Aimer `5.12x` slower |
| Cold parallel shaping batch | `15.853 ms` | `11.101 ms` | Aimer `1.43x` faster |
| One-at-a-time rasterization, 752 glyphs | `5.677 ms` | `6.711 ms` | Aimer `1.18x` slower |
| Rasterization in runs, 752 glyphs | `3.411 ms` | `8.140 ms` | Aimer `2.39x` slower |

Raw release outputs:

```text
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 5
  per-cluster average: 9.85505ms
  per-run average:     528.35µs
  cold serial batch:   5.626ms
  cold parallel batch: 15.853375ms

cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 5
  per-cluster average: 4.319791ms
  per-run average:     2.43345ms
  cold serial batch:   28.792183ms
  cold parallel batch: 11.100583ms

cargo run --release -p aimer_cupid --features aimer-font-compare \
  --example glyph_rasterization_benchmark -- 5
  one glyph at a time: 5.677475ms
  in runs:             3.411158ms

cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 5
  one glyph at a time: 6.711083ms
  in runs:             8.139749ms
```

The direct batch path reduced the debug one-at-a-time raster workload by about
5x and improved the release run workload versus the previous Aimer checkpoint
(`11.647 ms` to `8.140 ms`). The release end-to-end run remains slower than the
legacy path, so the next optimization should focus on the cache/preload scope
and coverage rasterization rather than adding more shaping lookup work.

### Validation

```text
cargo test -p aimer_cupid --features aimer-font \
  batched_font_rasterization_matches_individual_rasterization -- --nocapture
  1 passed; 0 failed

cargo test -p aimer_cupid --features aimer-font \
  compiles_layout_fast_paths_for_a_cached_face -- --nocapture
  1 passed; 0 failed

cargo test -p aimer_cupid --features aimer-font --quiet
  395 passed; 0 failed; 4 ignored

cargo test -p aimer_cupid --features aimer-font-compare \
  one_word_of_han_is_served_by_one_face -- --nocapture
  1 passed; 0 failed
```

The complete `aimer-font-compare` suite reached `396 passed; 1 failed;
4 ignored`; the one failure was the existing host/system-fallback assertion
that Han characters were split across system faces. Its focused rerun passed,
so it is recorded as host fallback nondeterminism rather than attributed to
the Aimer rasterizer changes. `git diff --check` passed.

## Hot-path optimization pass — 2026-08-30

This pass targets the remaining release gaps in whole-run shaping, cold serial
shaping, and the 752-glyph rasterization run. Every checkpoint below used
release builds and 100 iterations. The legacy column is the same Cupid
compatibility path with `aimer-font` disabled; the Aimer column enables
`aimer-font`.

### Implemented slices

1. Added `GlyphRasterizer::preload_text_into`, a synchronous callback API that
   exposes cached glyphs by reference. The owned `preload_text` API still
   returns cloned glyphs for compatibility, while `TextPipelineV2` and the
   raster benchmark use the streaming path. This removes a bitmap clone and a
   second allocation for every glyph consumed immediately by the atlas.
2. Reused the first primary-face cmap result in
   `glyph_key_for_codepoint_at_weight` instead of performing the same cached
   lookup twice.
3. Kept the earlier Aimer hot-path work in the measured build: bounded strong
   parsed-face/layout caches, compiled pair/ligature plans, pair-adjustment
   memoization, shared outline/flattened-edge caches, row edge buckets,
   precomputed inverse slopes, reusable scan-converter scratch, and the
   sequential 128-glyph preload batch.
4. Kept the final fast raster profile at `CURVE_FLATTEN_TOLERANCE = 0.125`
   and `SAMPLE_GRID = 2`. If that fast pass produces a blank bitmap from a
   non-empty outline, the converter retries that glyph at 8x8 so tiny curved
   CFF/TrueType glyphs do not disappear. This remains a deliberate
   quality/performance tradeoff, not a claim that 2x2 coverage is visually
   identical to higher supersampling.
5. Kept mixed-face preload correct: homogeneous runs use the large sequential
   batch directly, while text containing primary and fallback faces is grouped
   before rasterization so one scaler never receives glyphs from another face.

### Stage checkpoints

The first row is the checkpoint before this pass's streaming change. Its run
scope included the owned `Vec<(GlyphKey, RasterizedGlyph)>` result and bitmap
clones. The next rows use the same streamed callback scope on both paths.

| Raster stage | Legacy one | Aimer one | Legacy run | Aimer run |
| --- | ---: | ---: | ---: | ---: |
| Before streaming, owned result | `1.031022 ms` | `0.879549 ms` | `1.071348 ms` | `1.209596 ms` |
| Streamed preload callback | `0.932556 ms` | `0.765633 ms` | `1.126710 ms` | `1.133567 ms` |
| Reused primary cmap lookup | `0.932825 ms` | `0.761318 ms` | `1.155527 ms` | `1.100800 ms` |

The streaming checkpoint brought the run path to practical parity. The
100-iteration cmap checkpoint measured Aimer at about `1.05x` faster for the
run (`1.101 ms` vs `1.156 ms`) and about `1.23x` faster one glyph at a time
(`0.761 ms` vs `0.933 ms`). The higher-iteration final confirmation below is
the authoritative result after the correctness fixes.

The following are intermediate raster-profile experiments measured in
100-iteration release runs. The `0.25` and `0.5` curve-tolerance values were
rejected after the complete synthetic CFF/TrueType tests exposed blank or
unstable micro-glyphs; the final implementation uses `0.125` plus the blank
bitmap retry described above.

| Change checkpoint | Aimer run |
| --- | ---: |
| Row edge candidates and shared raster caches | `4.841227 ms` |
| Precomputed inverse slopes | `4.647511 ms` |
| Non-saturating coverage accumulator | `4.476665 ms` |
| Curve tolerance `0.25` | `4.333414 ms` |
| Curve tolerance `0.5` | `4.251900 ms` |
| `SAMPLE_GRID = 4` | `2.065970 ms` |
| `SAMPLE_GRID = 2` | `1.180885 ms` |
| Streamed preload callback | `1.133567 ms` |
| Primary cmap lookup reuse | `1.100800 ms` |

### Rejected optimization experiments

Each of these was implemented and measured in release mode, then removed when
it did not improve the run or added unnecessary risk:

| Experiment | Aimer streamed run result | Decision |
| --- | ---: | --- |
| Direct ASCII glyph-id table | `0.868–0.917 ms` in two 500-iteration samples, versus `0.836 ms` before it | Removed; slower/noisy |
| Batch hmtx visitor/slice loop | `0.869–0.875 ms` in 500-iteration samples | Removed; no repeatable gain |
| Bulk shared outline/flattened prefetch | `1.039897 ms` in 500 iterations | Removed; duplicate local hash probes outweighed lock savings |

The final code retains only changes with passing behavior coverage and a
repeatable or scope-correct performance benefit.

### Final shaping comparison

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.537350 ms` | `0.449968 ms` | Aimer `10.08x` faster |
| One whole shaped run, 2,000 chars | `0.239931 ms` | `0.220015 ms` | Aimer `1.09x` faster |
| Direct font-ID shaped run | `0.092283 ms` | `0.042060 ms` | Aimer `2.19x` faster |
| Cold serial shaping batch | `4.245331 ms` | `2.666572 ms` | Aimer `1.59x` faster |
| Cold parallel shaping batch | `10.894843 ms` | `11.109405 ms` | Effectively tied |

The direct shaped-run result improved from the original user checkpoint of
`2.547 ms` to `0.220 ms` for Aimer. Cold serial shaping improved from
`25.111 ms` to `2.667 ms`. Parallel cold shaping remains within scheduler
noise on this host.

### Raster quality gate

With the final `0.125` curve tolerance, 2x2 sample grid, and blank-bitmap
retry, the existing reference comparison passed:

```text
Phase 2 Latin: 50 glyph samples, mean absolute coverage error 0.0758,
  max edge error 1 px, missing 0
Phase 2 CJK: 40 glyph samples, mean absolute coverage error 0.0500,
  max edge error 0 px, missing 0
```

The gate is a bounded regression check against the reference rasterizer; it
does not remove the visual tradeoff of using fewer coverage samples. If a
product surface needs smoother antialiasing, raise the grid and remeasure the
run budget before changing the constant globally.

### Commands and validation

```text
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 100
  per-cluster average: 449.968us
  per-run average:     220.015us
  direct font-id run:  42.06us
  cold serial batch:   2.666572ms
  cold parallel batch: 11.109405ms

cargo run --release -p aimer_cupid \
  --example text_shaping_benchmark -- 100
  per-cluster average: 4.53735ms
  per-run average:     239.931us
  direct font-id run:  92.283us
  cold serial batch:   4.245331ms
  cold parallel batch: 10.894843ms

cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
  one glyph at a time: 781.046us
  in runs:             823.443us

cargo run --release -p aimer_cupid \
  --example glyph_rasterization_benchmark -- 500
  one glyph at a time: 908.174us
  in runs:             823.038us
```

The benchmark emitted the existing workspace manifest warnings and the
pre-existing unused `layout_paragraph` warning; no benchmark or compiler
error occurred.

### Final correctness-corrected confirmation

After restoring the curve tolerance, adding the blank-bitmap retry for tiny
curved outlines, grouping mixed-face preloads, and rejecting the unproductive
ASCII-table, hmtx-visitor, and shared-prefetch experiments, the final raster
comparison used 500 iterations:

| Workload | Legacy | Aimer | Result |
| --- | ---: | ---: | --- |
| One-at-a-time rasterization, 752 glyphs | `0.908174 ms` | `0.781046 ms` | Aimer `1.16x` faster |
| Streamed rasterization in runs, 752 glyphs | `0.823038 ms` | `0.823443 ms` | Effectively tied (`0.05%` delta) |

This means Aimer now wins the one-at-a-time raster scope and is at parity in
the streamed run scope on this host. The run difference is below normal host
noise; it should not be presented as a universal speedup without a broader
machine/font corpus.

The Aimer-specific raster, shaping, cache, and layout tests are green. The
full feature-enabled suite still sees the same two host-dependent fallback
assertions:

```text
cargo test -p aimer_cupid --features aimer-font --quiet
  395 passed; 2 failed; 4 ignored
  failures: assigns_one_stable_id_per_face,
            one_word_of_han_is_served_by_one_face
```

Both failures pass when run individually on this host, and are outside the
Aimer raster path. The comparison-feature suite has the same caveat:

```text
cargo test -p aimer_cupid --features aimer-font-compare --quiet
  396 passed; 2 failed; 4 ignored
  failures: assigns_one_stable_id_per_face,
            one_word_of_han_is_served_by_one_face
```

Those failures are outside the Aimer raster path and are the same fallback
nondeterminism recorded in the earlier baseline.

## Fresh legacy versus `aimer-font` comparison — 2026-08-31

A fresh release comparison was run against the current checkout. Each command
used the benchmark's existing scope and iteration count, and each profile was
launched twice. The values below are the mean of those two benchmark launches;
build time is outside the measured timers.

The primary legacy baseline is the default build with `aimer-font` disabled.
The Aimer profile enables only the `aimer-font` feature.

### Shaping comparison

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.425 ms` | `0.382 ms` | Aimer `11.57x` faster |
| One whole shaped run, 2,000 chars | `0.834 ms` | `0.792 ms` | Aimer `1.05x` faster; effectively tied |
| Direct shaping run | `94.6 us` | `43.4 us` | Aimer `2.18x` faster |
| Direct font-ID shaping run | `94.1 us` | `43.0 us` | Aimer `2.19x` faster |
| Cold serial shaping batch | `13.185 ms` | `11.916 ms` | Aimer `1.11x` faster |
| Cold parallel shaping batch | `8.034 ms` | `8.672 ms` | Legacy `1.08x` faster; near parity |

### Rasterization comparison

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| One-at-a-time rasterization, 752 glyphs | `0.902 ms` | `1.099 ms` | Legacy `1.22x` faster |
| Streamed rasterization in runs, 752 glyphs | `0.837 ms` | `1.161 ms` | Legacy `1.39x` faster |

### Commands

```text
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 100
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 100
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
```

## First-use rasterization optimization — batch reservation and `glyf` decode — 2026-08-31

The first-use path was the remaining Aimer raster bottleneck. Two focused
slices are now retained behind the existing `aimer-font` feature:

1. `rasterize_face_glyphs_into` reserves the face-local outline and flattened
   glyph maps for the incoming batch, avoiding repeated hash-map growth while
   a fresh run is decoded.
2. Simple TrueType outlines decode both axes into one point buffer and move
   those points into their contour vectors. This removes the separate X/Y
   coordinate vectors, the temporary combined point vector, and the per-
   contour point copies while preserving the decoded `GlyphOutline` contract.

### Stage-by-stage release measurements

The existing benchmark was run in separate release processes with 752 glyphs.
The streamed run is the useful first-use scope; the one-at-a-time timer runs
after it and is therefore cache/order sensitive.

| Stage | Aimer in runs, 1 iteration | Legacy control | Result |
| --- | ---: | ---: | --- |
| Before batch reservation | `6.149958 ms` | `1.599917 ms` | Legacy `3.84x` faster |
| After batch reservation | `2.972167 ms` | `3.227417 ms` | Aimer `1.09x` faster in this sample |
| After single-buffer `glyf` decode | `5.304750 ms` | `1.560791 ms` | Legacy `2.95x` faster; cold sample noisy |

The batch reservation measurement is a clear improvement in the immediate
before/after sample. The single-buffer decoder reduces the number of heap
allocations and copies on the TrueType path, but the repeated process-level
cold timing was noisy and did not establish a separate wall-time win. It is
retained as a lower-allocation, quality-neutral refactor rather than claimed
as a benchmark speedup.

The final 500-iteration samples were:

```text
Aimer:
one glyph at a time: 121.569 us
in runs:             119.410 us
key preparation:     6.334 us

Legacy:
one glyph at a time: 6.107722 ms
in runs:             1.673859 ms
key preparation:     13.410 us
```

The warmed streamed sample therefore measured Aimer `14.02x` faster than the
legacy control. As recorded in earlier sections, this comparison is affected
by process-wide cache warming and should not be generalized from one host.

### Quality and verification

Quality remained unchanged:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

The focused simple-outline, composite-outline, scan-conversion,
quality, and batch-rasterization tests passed. The full feature-enabled
`aimer_cupid` library run completed with `414 passed; 2 failed; 4 ignored`.
The two failures were the existing host-dependent fallback-ID/Han-face tests:
`assigns_one_stable_id_per_face` and `one_word_of_han_is_served_by_one_face`.
The comparison-feature check and `git diff --check` passed.

```text
cargo test -p aimer_cupid --features aimer-font \
  aimer_font::tests::extracts
cargo test -p aimer_cupid --features aimer-font \
  aimer_font::tests::rejects
cargo test -p aimer_cupid --features aimer-font scan_conversion
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test -p aimer_cupid --features aimer-font \
  batch_rasterization_matches_scalar_glyph_results
cargo test -p aimer_cupid --features aimer-font --lib
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo check -p aimer_cupid --features aimer-font-compare
git diff --check
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 500
```

## Next raster performance slice — direct edge flattening — 2026-08-31

The next cold-path slice removes temporary geometry allocations without
changing the active-edge coverage algorithm:

1. TrueType command storage now reserves from the decoded contour sizes.
2. Outline flattening keeps one reusable contour scratch buffer and emits
   `Edge` values directly at contour close, removing the temporary
   `Vec<Vec<Point>>` contour tree and its per-contour allocations.
3. Compact row indexing reuses its first-pass count array as the second-pass
   write cursor instead of cloning one cursor vector per flattened glyph.

The direct flattening regression test checks the exact closed edge sequence
for a square outline. Existing hole, overlap, slanted-edge, active-edge, and
reference-quality tests continue to exercise the coverage behavior.

### Release comparison

The benchmark was run in separate release processes with the same 752-glyph
workload. The immediate Aimer before-sample was `7.921542 ms` for the in-runs
scope; the post-change sample was `6.149958 ms`, a measured `22.4%` reduction
for that first-use run. The one-at-a-time scope is intentionally run after
the in-runs scope, so it is not a clean cold-start measurement.

Post-change paired output:

```text
Aimer, 1 iteration:
one glyph at a time: 407.25 us
in runs:             6.149958 ms
key preparation:     28.208 us

Legacy, 1 iteration:
one glyph at a time: 4.320625 ms
in runs:             1.599917 ms
key preparation:     10.417 us

Aimer, 500 iterations:
one glyph at a time: 118.747 us
in runs:             152.262 us
key preparation:     6.293 us

Legacy, 500 iterations:
one glyph at a time: 3.902328 ms
in runs:             833.873 us
key preparation:     6.641 us
```

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| One-at-a-time rasterization, 752 glyphs, warm sample | `3.902328 ms` | `0.118747 ms` | Aimer `32.87x` faster* |
| Streamed rasterization in runs, 752 glyphs, warm sample | `0.833873 ms` | `0.152262 ms` | Aimer `5.47x` faster |
| Streamed rasterization in runs, 752 glyphs, first-use sample | `1.599917 ms` | `6.149958 ms` | Legacy `3.84x` faster |

\* The benchmark intentionally measures one-at-a-time after the streamed
scope, so that row should be treated as a warmed/order-sensitive comparison.
The streamed scope is the more useful comparison for this allocation slice.

### Quality and verification

The quality output remained:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

The focused direct-flattening test, scan-conversion tests, quality test, and
batch-rasterization test passed. The full feature-enabled `aimer_cupid`
library run passed with `416 passed; 0 failed; 4 ignored`. The comparison
feature check and `git diff --check` also passed; only the existing unused
`layout_paragraph` warning remains.

```text
cargo test -p aimer_cupid --features aimer-font \
  direct_flattening_emits_the_expected_closed_edges
cargo test -p aimer_cupid --features aimer-font scan_conversion
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test -p aimer_cupid --features aimer-font \
  batch_rasterization_matches_scalar_glyph_results
cargo test -p aimer_cupid --features aimer-font --lib
cargo check -p aimer_cupid --features aimer-font-compare
git diff --check
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 500
```

The current result is that Aimer's direct and per-cluster shaping paths are
substantially faster, while whole-run and parallel shaping are effectively at
parity. The legacy Swash rasterizer remains faster for both one-at-a-time and
streamed run rasterization in this measurement, making batched coverage
rasterization the next Aimer performance target. Results are host-dependent
and should be rechecked across a broader font corpus before treating them as
general speed claims.

## Priority 1 — Cached scanline coverage spans — 2026-08-31

The scan-conversion hot path now builds a per-flattened-glyph scanline plan
lazily and stores it behind `OnceLock`. The plan contains clipped non-zero
winding coverage spans and can be replayed without rebuilding intersections,
sorting crossings, or scanning the bitmap to discover whether coverage was
written. First use combines plan construction with the initial fill. The
existing incremental active-edge converter remains available for the small
glyph high-quality retry path and as the reference-compatible fallback.

The focused regression test compares the cached-plan output with an independent
scan-conversion reference for a slanted outline containing a hole. The existing
scan-conversion and Phase 2 quality tests remain green.

### Warm release rasterization comparison

This is a sequential comparison using the existing 752-glyph benchmark with
500 iterations. The two profiles were launched separately after compilation.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| One-at-a-time rasterization, 752 glyphs | `0.923035 ms` | `0.562271 ms` | Aimer `1.64x` faster |
| Streamed rasterization in runs, 752 glyphs | `0.815295 ms` | `0.571172 ms` | Aimer `1.43x` faster |

The warm path now beats the legacy rasterizer in both measured scopes on this
host. First-use plan construction still has a cost: in the single-iteration
run scope the legacy result was `3.009291 ms` versus `3.582625 ms` for Aimer.
That first-use comparison is affected by the benchmark's order and shared
cache warming, so it is not a clean cold-start isolation; it does show that
the next optimization should target plan construction and first-use cache
work rather than the replay path.

### Quality check

The current Phase 2 reference-quality test passed with:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

The CJK value is within the test threshold and should continue to be watched
when changing span quantization or antialiasing. The measurement is
host/font-cache dependent and is not a pixel-perfect promise for every font.

### Verification commands

```text
cargo test -p aimer_cupid --features aimer-font scan_conversion
cargo test -p aimer_cupid --features aimer-font \
  precomputed_scanline_plan_matches_reference_for_slanted_holes
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid \
  --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
```

Results: the focused scan-conversion tests passed, the full Aimer library
suite passed (`412 passed; 0 failed; 4 ignored`), and the comparison-feature
check completed successfully with only the existing unused-function warning.

## Next raster performance slice — bitmap replay and batched advances — 2026-08-31

The next measured bottlenecks were addressed in three retained stages:

1. A flattened glyph now retains a bounded final alpha bitmap in an
   `Arc<OnceLock<_>>`. Repeated requests copy the completed pixels directly
   instead of replaying every coverage span and renormalizing the bitmap. The
   cache is limited to `64 KiB` per glyph; larger glyphs keep the normal
   correctness path without expanding the shared cache uncontrollably.
2. First-use conversion now uses the active-edge sweep directly. Existing
   crossings are recomputed from the sample coordinate rather than advanced
   by accumulated floating-point deltas, avoiding antialiasing boundary drift.
   The row-to-edge index was also compacted from one `Vec` per row into offset
   and index arrays, reducing row allocations and indirection.
3. Aimer batch rasterization acquires the validated `hmtx` advance slice once
   and computes the font scale once for the run. Each glyph then reuses the
   already-read advance while entering the same outline and coverage caches.

The new regression tests compare the active-edge output with the independent
coverage reference and compare the batch output with scalar rasterization.

### Final release comparison

These are sequential launches of the existing benchmark with 752 glyphs and
500 iterations. The benchmark now also reports key preparation separately;
the existing one-at-a-time and in-runs scopes are unchanged. The Aimer final
run was:

```text
one glyph at a time: 133.618 us (177 ns per glyph)
in runs:             141.088 us (187 ns per glyph)
key preparation:     6.254 us (8 ns per glyph)
```

The paired legacy run was:

```text
one glyph at a time: 916.382 us (1.218 us per glyph)
in runs:             816.084 us (1.085 us per glyph)
key preparation:     10.109 us (13 ns per glyph)
```

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| One-at-a-time rasterization, 752 glyphs | `0.916382 ms` | `0.133618 ms` | Aimer `6.86x` faster |
| Streamed rasterization in runs, 752 glyphs | `0.816084 ms` | `0.141088 ms` | Aimer `5.78x` faster |

For the one-iteration first-use scope, the final Aimer run measured
`3.278416 ms` and the paired legacy run measured `1.510333 ms`. This benchmark
creates a fresh rasterizer per iteration but intentionally allows the Aimer
process-wide flattened cache to warm across iterations; therefore the 500-run
values represent the steady-state shared-cache path, while the one-iteration
values expose first-use setup. The cold path is still the next optimization
target: outline decoding, flattening, and first-use cache population dominate
more than scanline replay now does.

One run-level shared-map prefetch/publish experiment was measured and removed:
it produced `0.169911 ms` one-at-a-time and `0.163342 ms` in runs, slower than
the retained `0.133618 ms` and `0.141088 ms` results. It is not part of the
current implementation.

### Quality and verification

The current quality output is unchanged from the accepted threshold:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

Focused scan-conversion, active-edge, bitmap-cache, batch-rasterization, and
quality tests passed. The feature-enabled library run completed with
`413 passed; 2 failed; 4 ignored`; the two failures were the existing
host-dependent fallback-ID/Han-face tests (`assigns_one_stable_id_per_face`
and `one_word_of_han_is_served_by_one_face`). The comparison-feature check
completed successfully with only the existing unused-function warning.

```text
cargo test -p aimer_cupid --features aimer-font scan_conversion
cargo test -p aimer_cupid --features aimer-font \
  active_edge_coverage_matches_reference_for_a_slanted_outline
cargo test -p aimer_cupid --features aimer-font \
  rasterized_coverage_is_cached_and_replayed_exactly
cargo test -p aimer_cupid --features aimer-font \
  batch_rasterization_matches_scalar_glyph_results
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid \
  --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
```

## First-use rasterization slice — cached `loca` offsets — 2026-08-31

The next first-use hotspot was the TrueType glyph-range lookup. Before this
slice, every glyph request revalidated the complete `loca` table length,
constructed a reader, and decoded the two offsets surrounding that glyph. A
validated `SfntFace` now parses the offset array once into a face-local
`OnceLock<Result<Vec<usize>, SfntError>>`. Subsequent outline requests use the
cached entries directly. The cache also remembers a malformed-table error,
while the selected glyph's ordering and `glyf`-bounds checks remain in the
per-glyph path so malformed-font behavior is not broadened or hidden.

This is intentionally a small first-use slice. It does not change outline
flattening, scan conversion, hinting, or antialiasing, and the existing
outline and malformed-SFNT tests remain the correctness oracle.

### Release comparison

The benchmark uses 752 distinct glyph requests. Single-iteration samples were
launched in separate processes three times per implementation; the table uses
the median. The benchmark's `in runs` scope is the useful first-use signal.
Its `one glyph at a time` scope runs after the streamed preload in the same
iteration and therefore represents a warmed glyph path rather than a pure
cold start.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| First-use streamed rasterization, 752 glyphs | `2.865042 ms` median | `5.851584 ms` median | Legacy `2.04x` faster |
| Warm one-at-a-time rasterization, 752 glyphs | `7.011334 ms` median | `0.273167 ms` median | Aimer `25.67x` faster |
| 500-iteration streamed rasterization, 752 glyphs | `0.905905 ms` | `0.168289 ms` | Aimer `5.38x` faster |
| 500-iteration one-at-a-time rasterization, 752 glyphs | `3.982860 ms` | `0.129288 ms` | Aimer `30.81x` faster |

The cold streamed path is still slower than the legacy path because outline
decoding, curve flattening, and first-use coverage construction dominate the
newly reduced `loca` work. The repeated-run result remains strongly ahead once
the face-local and flattened caches are warm. The absolute values are
host-dependent; the three-sample median is recorded to make the cold result
less sensitive to process scheduling.

### Quality and verification

The accepted quality output is unchanged:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

The focused outline, malformed-input, batch-rasterization, scan-conversion,
and quality tests passed. The full feature-enabled library suite passed with
`416 passed; 0 failed; 4 ignored`. The comparison-feature check also passed,
with only the existing unused `layout_paragraph` warning.

```text
cargo test -p aimer_cupid --features aimer-font aimer_font::tests::extracts
cargo test -p aimer_cupid --features aimer-font --lib
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 500
```

The next first-use target is now the initial outline/flattening and coverage
population work, not table-offset decoding.

## First-use outline, flattening, and coverage allocation slice — 2026-08-31

The remaining first-use work was reduced in three retained stages behind the
existing `aimer-font` path:

1. TrueType curve flattening now emits `Edge` values directly while adaptive
   subdivision runs. It no longer allocates a temporary vector of flattened
   contour points and then walks that vector again to create edges. The
   subdivision order and tolerance are unchanged.
2. A decoded TrueType outline now stores one contiguous `Vec<OutlinePoint>`
   plus half-open contour ranges. Simple glyph decoding no longer copies the
   point buffer into one allocation per contour; composite glyphs transform
   component points into the same contiguous representation. Point-bound
   validation is folded into the existing point-to-command walk.
3. First coverage population now converts the built bitmap directly into the
   bounded `Arc<[u8]>` cache and copies only the `Vec<u8>` returned to the text
   pipeline. This removes the temporary cloned bitmap allocation while
   retaining exact replay for later requests.

### Stage-by-stage release measurements

All measurements use the existing 752-glyph benchmark in separate release
processes. The `in runs` scope is the first-use batch. The benchmark's
one-at-a-time scope runs after that batch in the same iteration, so it is a
warmed scalar path. The values are process- and host-dependent; these samples
are retained as directional evidence rather than a universal performance
claim.

| Stage | First-use Aimer `in runs` | Paired legacy `in runs` | Warm Aimer `in runs` | Paired legacy warm `in runs` |
| --- | ---: | ---: | ---: | ---: |
| Before this slice, after cached `loca` offsets | `5.851584 ms` median | `2.865042 ms` median | `0.168289 ms` | `0.905905 ms` |
| Direct curve-to-edge emission | `3.602125 ms` | `3.148375 ms` | `0.148340 ms` | `0.960819 ms` |
| Contiguous outline points and fused bounds | `6.020584 ms` | `3.063625 ms` | `0.136964 ms` | not re-run in this stage |
| Direct bounded coverage-cache publication | `3.276916 ms` | `1.543000 ms` | `0.150042 ms` | `0.816684 ms` |

The final warm streamed result is `5.44x` faster for Aimer. The final cold
streamed result is still `2.12x` slower than legacy in this paired sample;
outline parsing, first flattening, and first coverage population remain more
expensive than the already-optimized replay path. The intermediate cold
measurements fluctuate substantially, so the allocation reductions are kept
because they remove concrete copies and preserve the output oracle, not
because one isolated wall-clock sample can prove a universal gain.

For the same final 500-iteration process pair, the warmed one-at-a-time scope
was `0.117841 ms` for Aimer versus `3.848659 ms` for legacy, or Aimer `32.66x`
faster. As above, that scope is deliberately after streamed preload and is not
a pure first-use measurement.

### Quality and verification

The quality target remains unchanged:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

The focused curve, outline, coverage-cache, scan-conversion, and quality tests
passed. The comparison-feature check passed with only the existing unused
`layout_paragraph` warning. The parallel full owned-font library run completed with
`414 passed; 2 failed; 4 ignored`; the failures are the unrelated,
host-dependent `assigns_one_stable_id_per_face` and
`one_word_of_han_is_served_by_one_face` fallback tests. Each failure passes
when run alone, and the serial full library run completed with `416 passed; 0
failed; 4 ignored`.

```text
cargo test -p aimer_cupid --features aimer-font \
  direct_flattening_emits_the_expected_closed_edges
cargo test -p aimer_cupid --features aimer-font \
  rasterized_coverage_is_cached_and_replayed_exactly
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 1
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark \
  --features aimer-font-compare -- 500
```

The next measurable first-use target is the per-glyph cache plumbing around
outline construction—especially shared-cache locking and result publication—
followed by any remaining TrueType variation decode cost. The raster geometry
and coverage quality path is now allocation-reduced without relaxing its
reference checks.

## Current overall legacy versus `aimer-font` release snapshot — 2026-08-31

This snapshot compares the current checkout in separate release processes. The
legacy profile disables `aimer-font`; the Aimer profile enables only the
`aimer-font` feature. The shaping benchmark uses 100 iterations and the glyph
rasterization benchmark uses 500 iterations.

### Shaping and rasterization comparison

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.337548 ms` | `0.311889 ms` | Aimer `13.91x` faster |
| One whole shaped run, 2,000 chars | `0.820514 ms` | `0.794961 ms` | Aimer `1.03x` faster; near parity |
| Direct shaping run | `96.445 us` | `38.739 us` | Aimer `2.49x` faster |
| Direct font-ID shaping run | `96.676 us` | `38.172 us` | Aimer `2.53x` faster |
| Cold serial shaping batch | `13.649170 ms` | `11.901060 ms` | Aimer `1.15x` faster |
| Cold parallel shaping batch | `7.310494 ms` | `9.142432 ms` | Legacy `1.25x` faster |
| Rasterization one glyph at a time, 752 glyphs | `968.026 us` | `121.131 us` | Aimer `7.99x` faster |
| Streamed rasterization in runs, 752 glyphs | `826.474 us` | `124.244 us` | Aimer `6.65x` faster |
| Glyph-key preparation, 752 glyphs | `9.412 us` | `7.396 us` | Aimer `1.27x` faster |

The 500-iteration rasterization values represent the warmed process-wide
cache path. They are not pure first-use measurements. In the separate fresh
first-use sample recorded above, streamed rasterization was `3.276916 ms` for
Aimer versus `1.543000 ms` for legacy, so legacy was `2.12x` faster during
initial outline decoding, flattening, and coverage population.

Overall, Aimer now leads the common serial shaping scopes and the warmed
rasterization scopes. Legacy still leads pure first-use rasterization and the
current parallel cold-shaping sample. The accepted raster quality target is
unchanged: Latin mean absolute coverage error `0.0616`, CJK `0.1793`, maximum
edge error `1 px`, and missing glyphs `0`.

### Reproduction commands

```text
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 100
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 100
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
```

The values are host- and scheduler-dependent directional measurements; the
relative cold/warm distinction is more important than the precision of any
single wall-clock sample.

## Phase 4 parallel cold-start shaping — 2026-08-31

The next performance milestone targeted the remaining cold-parallel shaping
gap while keeping all standard-font work behind `aimer-font`:

1. `ParsedAimerFont` now exposes an immutable `Arc` seed for worker contexts.
   `FontSnapshot` prewarms the primary face and its compiled `LayoutState`
   before the shaping batch enters Rayon. Each worker creates only its local
   mutable `AimerFontState`/scratch wrapper; face parsing, GSUB/GPOS parsing,
   and shared outline ownership are not repeated per worker.
2. The fixed primary face has a process-wide `OnceLock` handle. Fresh direct
   rasterizers avoid the parsed-face map read lock after the first primary
   request, and primary line metrics use that same handle. Registered and
   system fallback faces retain the bounded read/write cache and lazy loading.
3. The existing duplicated Aimer-state initialization branches were joined
   behind `ensure_aimer_font_cached`. The primary fast path is selected by the
   stable primary font id; fallback behavior is unchanged.

The worker handoff has a regression test,
`worker_context_starts_with_a_prewarmed_primary_aimer_face`, and the existing
worker output tests still compare shaped clusters, layout geometry, and
rasterized bitmap dimensions/coverage.

### Stage measurements

The release shaping benchmark uses 2,000 characters and 500 iterations. Legacy
and Aimer are run in separate processes. The first paired run below was made
after the primary handle/metric fast path was retained; the public benchmark's
direct `GlyphRasterizer` scopes do not exercise `FontSnapshot`, so the snapshot
handoff itself is validated by the worker regression/output tests rather than
isolated by that benchmark.

| Stage | Whole run | Cold serial batch | Cold parallel batch |
| --- | ---: | ---: | ---: |
| Retained prewarm + primary fast path, default Rayon | Legacy `903.593 us` / Aimer `768.701 us` | Legacy `14.825494 ms` / Aimer `12.787916 ms` | Legacy `7.281814 ms` / Aimer `7.320946 ms` |
| Retained path, bounded Rayon (`RAYON_NUM_THREADS=4`) | Legacy `837.641 us` / Aimer `775.369 us` | Legacy `13.996457 ms` / Aimer `12.552714 ms` | Legacy `4.856014 ms` / Aimer `4.678518 ms` |

The bounded four-worker result makes Aimer `1.08x` faster for a whole shaped
run, `1.12x` faster for the cold serial batch, and `1.04x` faster for the cold
parallel batch. A one-worker confirmation was also close to the serial result:
legacy `14.075652 ms` versus Aimer `12.548581 ms` for the cold batch. Rayon
width matters because the benchmark's cold-parallel scope creates a fresh
rasterizer per input; it is not the same as the production `BatchExecutor`
worker-reuse path.

One read-mostly-cache experiment changed the shared decoded-outline and
flattened-edge locks from `Mutex` to `RwLock`. It was measured and rejected:
the paired default-width result was legacy `7.544262 ms` versus Aimer
`7.888402 ms`, worse than the retained implementation. The source remains
mutex-based, avoiding a speculative synchronization change.

The same final bounded-four-worker release pair reported these surrounding
scopes:

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.501351 ms` | `206.156 us` | Aimer `21.83x` faster |
| Direct shaping run | `96.611 us` | `40.288 us` | Aimer `2.40x` faster |
| Direct font-ID shaping run | `96.944 us` | `40.842 us` | Aimer `2.37x` faster |
| Rasterization one glyph at a time, 752 glyphs | `1.001863 ms` | `125.821 us` | Aimer `7.96x` faster |
| Streamed rasterization in runs, 752 glyphs | `860.376 us` | `144.044 us` | Aimer `5.97x` faster |
| Glyph-key preparation, 752 glyphs | `9.645 us` | `6.623 us` | Aimer `1.46x` faster |

These values are directional, host-dependent release measurements. They show
that the remaining cold-parallel gap is no longer an inherent Aimer shaping
deficit at the production-sized four-worker width; scheduler width and the
benchmark's intentionally fresh local contexts dominate the variance.

### Quality and verification

The raster output oracle remains unchanged:

```text
Latin: 50 glyph samples, mean absolute coverage error 0.0616,
       maximum edge error 1 px, missing 0
CJK:   40 glyph samples, mean absolute coverage error 0.1793,
       maximum edge error 1 px, missing 0
```

The serial feature-enabled library suite passed with `417 passed; 0 failed; 4
ignored`. A default parallel run still reproduces the two known host-sensitive
fallback-test failures (`assigns_one_stable_id_per_face` and
`one_word_of_han_is_served_by_one_face`); both pass in the serial suite and
are unrelated to this cache handoff. The comparison feature and legacy build
checks passed; the comparison check retains the existing unused
`layout_paragraph` warning.

```text
cargo test -p aimer_cupid --features aimer-font \
  worker_context_starts_with_a_prewarmed_primary_aimer_face -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo check -p aimer_cupid
cargo check -p aimer_cupid --features aimer-font-compare
RAYON_NUM_THREADS=4 cargo run --release -p aimer_cupid \
  --example text_shaping_benchmark -- 500
RAYON_NUM_THREADS=4 cargo run --release -p aimer_cupid \
  --features aimer-font --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
git diff --check
```

## Persistent shaping worker pool — 2026-08-31

The production shaping and glyph-run preparation path now has a persistent,
manually configured native worker pool:

1. `BatchExecutor` starts a bounded worker set once and shuts it down through
   `Drop`; the worker threads and their thread-local scaler caches survive
   between preparation batches.
2. Owned shaping and glyph jobs cross the worker boundary as one shared
   `Arc<[IndexedJob]>`, so the persistent path does not clone glyph-run payloads
   per batch. Each active worker creates one context and claims jobs through
   the existing atomic cursor.
3. The pool uses a generation barrier and `Condvar`, not a per-batch receiver
   mutex or boxed closure queue. All workers wake for one batch generation,
   report one result vector, and the caller retains the existing order/key
   validation before committing anything.
4. Worker panics and failed preparation still produce an incomplete result set,
   which is rejected before cache or atlas state is committed. Small batches
   and WASM use direct iteration; the borrowed scoped executor remains only as
   a test-only comparison baseline.

The layout stage now crosses the same owned boundary. `TextPipelineV2` stores
shaped results as `Arc<ShapedText>`, resolves each layout job to a shared shaped
allocation after shaping completes, and transfers an owned layout slice to the
persistent workers. A cache hit therefore increments an `Arc` reference count
instead of copying the shaped clusters, while multiple wrapping widths for one
span share the same shaped result.

### Stage measurements

The ignored release microbenchmark uses 100 iterations of 16 shaping jobs with
four workers. It compares the same `BatchExecutor` workload against the old
scoped executor in separate optimized processes. Values are
`persistent / scoped`:

| Stage | Legacy | Aimer | Result |
| --- | ---: | ---: | --- |
| Initial owned task queue | `31.380750 ms / 28.462833 ms` | `22.317791 ms / 23.851875 ms` | rejected for legacy; Aimer `1.07x` faster |
| Final generation barrier | `18.211167 ms / 24.704709 ms` | `20.979458 ms / 23.725042 ms` | legacy `1.36x` faster; Aimer `1.13x` faster |
| Owned layout boundary | `2.525500 ms / 5.803167 ms` | `2.021000 ms / 4.157750 ms` | legacy `2.30x` faster; Aimer `2.06x` faster |

The first task-queue design was measured and rejected because its boxed task,
per-worker channel, and job-copy overhead made the legacy path `10.25%`
slower. Moving to the shared generation barrier and transferring the owned job
slice without a copy made the final persistent path `26.28%` faster for legacy
and `11.58%` faster for `aimer-font` in this repeated shaping scope. The owned
layout boundary was then measured with the same 100 iterations and 16 jobs,
each job sharing one `Arc<ShapedText>`: it reduced layout executor time by
`56.48%` for legacy and `51.39%` for `aimer-font` versus recreating scoped
workers. These are executor microbenchmarks; frame-level results remain
scheduler- and workload-dependent.

### Quality and verification

The change only transfers already-shaped data between workers; shaping,
fallback, glyph geometry, coverage, and atlas-compositing behavior remain
unchanged. The quality oracle remains:

```text
Phase 2 Latin: 50 glyph samples, mean absolute coverage error 0.0616,
               max edge error 1 px, missing 0
Phase 2 CJK:   40 glyph samples, mean absolute coverage error 0.1793,
               max edge error 1 px, missing 0
```

The no-feature library suite passed with `350 passed; 0 failed; 5 ignored` and
the `aimer-font` suite passed with `430 passed; 0 failed; 5 ignored`. The
focused executor suite passed with `11 passed; 0 failed; 1 ignored`; the cache
ownership suite passed with `3 passed`. Both feature modes compiled; the
existing unused `layout_paragraph` warning remains. The next performance check
should measure a real `TextPipelineV2::prepare` frame with mixed fallback faces
to quantify the end-to-end effect of the owned shaping, layout, and glyph
boundaries.

```text
cargo test -p aimer_cupid preparation_batch --lib -- --nocapture
cargo test -p aimer_cupid --features aimer-font preparation_batch --lib -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo test -p aimer_cupid --lib -- --test-threads=1
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test --release -p aimer_cupid persistent_executor_shaping_benchmark \
  --lib -- --ignored --nocapture
cargo test --release -p aimer_cupid --features aimer-font \
  persistent_executor_shaping_benchmark --lib -- --ignored --nocapture
cargo check -p aimer_cupid
cargo check -p aimer_cupid --features aimer-font-compare
git diff --check
```

The next high-value optimization should profile production `BatchExecutor`
jobs with mixed scripts and multiple active faces, then decide whether to
prewarm only the faces selected by the batch (rather than eagerly parsing all
loaded fallbacks). The current primary-only policy preserves lazy CJK/emoji
startup and should remain the default until that measurement exists.

## Manual configured text workers — 2026-08-31

Rayon was removed from `aimer_cupid`. `BatchExecutor` now uses a safe,
manually configured native worker set built with `std::thread::scope`:

1. Native worker width remains bounded by `available_parallelism() - 1`, with
   a maximum of four workers on desktop and two on iOS/Android. Batches below
   the existing four-job threshold remain serial.
2. A parallel batch shares only an atomic next-job counter and a result
   channel. Each worker creates one context and reuses it for the complete
   batch; per-worker result vectors avoid a lock on every prepared result.
3. Results are merged through the existing order/key validation, so worker
   completion order cannot change the committed output and a failed job still
   exposes no partial result set.
4. The benchmark's four-worker cold-parallel helper uses the same manual
   scheduling model, and Aimer no longer declares or calls Rayon directly.

This is deliberately a scoped configured worker pool rather than a persistent
global pool. The current executor accepts borrowed jobs and callbacks (the
layout callback also borrows the owning pipeline); a persistent generic pool
would require cloning large text/glyph inputs or introducing unsafe borrowed
task lifetimes. The scoped design keeps the existing API and context reuse
without either tradeoff. It does pay thread-startup cost once per parallel
batch, which is visible in the cold-parallel measurement.

### Equivalent four-worker release comparison

The historical Rayon row is the bounded `RAYON_NUM_THREADS=4` measurement
already recorded in the Phase 4 section. The manual row uses the same
2,000-character benchmark and 500 iterations, with `PARALLEL_WORKERS = 4`.
Values are `legacy / aimer-font`.

| Workload | Bounded Rayon | Manual workers | Manual delta: legacy / Aimer |
| --- | ---: | ---: | ---: |
| One whole shaped run | `837.641 us / 775.369 us` | `840.021 us / 790.982 us` | `+0.28% / +2.01%` |
| Cold serial shaping batch | `13.996457 ms / 12.552714 ms` | `14.072032 ms / 12.597336 ms` | `+0.54% / +0.36%` |
| Cold parallel shaping batch | `4.856014 ms / 4.678518 ms` | `5.278379 ms / 4.703046 ms` | `+8.70% / +0.52%` |

The manual executor keeps Aimer essentially at the previous Rayon result for
the cold-parallel scope and remains slightly faster than legacy in the paired
manual run. Legacy loses more on that scope because the benchmark starts a
fresh rasterizer per input and therefore exposes the manual worker startup
overhead directly. This benchmark is a controlled parallel scheduling
comparison; the production `BatchExecutor` behavior is additionally covered
by its focused executor tests because the public shaping example does not
expose the private pipeline batch object.

### Current manual-worker legacy versus Aimer snapshot

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.542940 ms` | `216.513 us` | Aimer `20.98x` faster |
| One whole shaped run, 2,000 chars | `840.021 us` | `790.982 us` | Aimer `1.06x` faster |
| Direct shaping run | `96.351 us` | `42.436 us` | Aimer `2.27x` faster |
| Direct font-ID shaping run | `97.060 us` | `42.134 us` | Aimer `2.30x` faster |
| Cold serial shaping batch | `14.072032 ms` | `12.597336 ms` | Aimer `1.12x` faster |
| Cold parallel shaping batch | `5.278379 ms` | `4.703046 ms` | Aimer `1.12x` faster |

The rasterizer and shaping quality paths were not changed by this worker
replacement. The existing quality oracle remains Latin mean absolute coverage
error `0.0616`, CJK `0.1793`, maximum edge error `1 px`, and missing glyphs
`0`. The feature-enabled library suite passed with `417 passed; 0 failed; 4
ignored`; the only warning is the existing unused `layout_paragraph` helper.

### Verification

```text
cargo test -p aimer_cupid preparation_batch -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 500
git diff --check
```

## CJK Unicode variation-selector shaping — 2026-08-31

The next Phase 4 slice wires the checked cmap format-14 reader into the owned
shaping path for bounded CJK variation-sequence runs:

1. Parsed cmap state now retains sorted format-14 selector records, default
   Unicode ranges, and non-default glyph mappings. The records and entries are
   bounded before allocation and use binary search during lookup.
2. A malformed optional format-14 subtable is retained as a localized error;
   ordinary base cmap lookup remains usable, while a variation lookup safely
   declines with the checked error.
3. Aimer consumes `base + variation-selector` as one glyph cluster, selects
   the non-default glyph when declared, ignores the selector's own advance,
   and keeps the base glyph's horizontal metrics. An undeclared selector falls
   back to the base glyph.
4. Only CJK runs containing valid variation sequences and no unsupported
   script/layout content are claimed. Plain CJK text, language-specific forms,
   and vertical substitutions continue through the compatibility shaper.

### Verification

The focused feature tests passed:

```text
maps_a_unicode_variation_sequence_through_a_format14_cmap ... ok
keeps_base_cmap_usable_when_an_optional_format14_table_is_malformed ... ok
shapes_a_cjk_unicode_variation_sequence_through_the_owned_path ... ok
```

The implementation remains behind `aimer-font`; no-feature builds do not
compile or use this owned shaping path. The remaining CJK Phase 4 work is
language-specific (`locl`) and vertical (`vert`/`vrt2`) substitution/layout,
followed by broader cross-shaper comparison coverage.

## Warm-cache and steady-state preparation reuse — 2026-08-31

The next preparation stage adds a generation-guarded retained-frame cache to
`TextPipelineV2`. It compares the complete render input exactly — including
request geometry, text, colors, spans, shadows, clipping, decorations,
viewport, and atlas layout generations — before deciding that the existing
instance lists and request ranges can be rendered again. This deliberately
does not use a hash-only shortcut, so a collision cannot display stale text.

When the inputs are unchanged, `prepare` now skips request analysis, layout
key construction, shaping, layout, glyph preparation, atlas planning,
instance assembly, atlas uploads, and instance uploads. The retained cache is
not published for a frame that still has deferred ahead-of-view work, so
scroll preparation continues on the next identical frame.

Changed frames also reuse the visible span-key/range vectors, glyph
descriptor vectors, deduplication set, and atlas capacity-plan descriptors.
If a change affects only position, color, clipping, or another instance field,
the atlas planners are not rerun when the descriptor set and atlas generation
are unchanged. Max-size atlas repacks now advance the atlas generation even
when the underlying texture object is retained, invalidating stale UVs safely.

The benchmark adds an alternating one-request position edit to distinguish a
true unchanged-frame hit from a warm frame that still needs instance rebuild.

### Release comparison

The same 29 visible mixed-script requests were run for 20 iterations in each
mode. Pipeline construction was outside the timer. Values are
`legacy / aimer-font`.

| Scope | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Cold first sample | `239.400000 ms` | `255.618959 ms` | initialization noise |
| Cold repeat average | `1.268550 ms` | `849.427 us` | Aimer `1.49x` faster |
| Cold p50 | `1.240708 ms` | `845.167 us` | Aimer `1.47x` faster |
| Cold p95 | `1.562667 ms` | `1.032083 ms` | Aimer `1.52x` faster |
| Warm unchanged repeat average | `263 ns` | `261 ns` | parity |
| Warm unchanged profile average | `193 ns` | `229 ns` | sub-microsecond parity |
| Warm alternating single-request edit | `42.989 us` | `42.978 us` | parity |

The unchanged-frame profile reports `cache_hit=true` and zero submitted jobs,
zero atlas descriptors, and zero GPU upload time. The alternating edit reports
`cache_hit=false`, but its atlas planning stage is only the descriptor
comparison (`616 ns` legacy / `631 ns` Aimer in this sample); no capacity plan
or atlas upload is needed. Its total remains about `43 us` because rebuilding
the instance lists and writing the changed vertex data are still required.

### Quality and verification

The cache stores exact request snapshots and checks all render-affecting
fields, including nested rich-text spans. Atlas generations cover both texture
growth and same-texture repacking. Existing culling, z-order, fallback,
shaping, atlas, CJK, Arabic, Myanmar, Korean, and raster-quality behavior is
unchanged.

```text
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
434 passed; 0 failed; 5 ignored

cargo test -p aimer_cupid --lib -- --test-threads=1
353 passed; 0 failed; 5 ignored

cargo test -p aimer_cupid --lib prepared_frame_cache -- --nocapture
2 passed; 0 failed

cargo check -p aimer_cupid --example text_prepare_mixed_benchmark
cargo check -p aimer_cupid --features aimer-font \
  --example text_prepare_mixed_benchmark
cargo run -p aimer_cupid --release --example text_prepare_mixed_benchmark
cargo run -p aimer_cupid --features aimer-font --release \
  --example text_prepare_mixed_benchmark
git diff --check
```

The next measurable target after this slice is partial-change preparation:
reuse unchanged request instance ranges and upload only changed contiguous
instance spans, while continuing to preserve request-order rendering and
z-order semantics.

## Repeated-cold `TextPipelineV2::prepare` profiling and cache work — 2026-08-31

The end-to-end preparation path now has an opt-in stage profile exposed by
`TextPipelineV2::prepare_profiled`. The normal `prepare` path does not start
timers or allocate profile state. The profile separates request analysis,
layout-key construction, owner-side fallback resolution, immutable font
snapshot construction, shaping, layout, glyph preparation, atlas planning and
population, instance assembly, and GPU uploads.

The measured hot spots were addressed with these changes:

1. Fallback resolution is warmed once on the owner for each complete text
   span, using the same grapheme boundaries, script requirement, language, and
   effective weight as glyph-key construction. Resolved codepoints are cached
   by `(codepoint, script requirement, weight)` and transferred in the worker
   snapshot. The system fallback store also caches the selected glyph beside
   its stable face id. Dynamic Apple fallback faces remain in the original
   adoption-order chain; an aggressive shortcut was rejected because it
   changed Japanese/Chinese face selection and weight behavior.
2. Aimer face metadata and parsed layout state are shared through the existing
   immutable face snapshot. The embedded Japanese fallback record is created
   once per process and its immutable font bytes are shared between fresh
   rasterizers. Cache invalidation still clears derived answers when registry
   faces or fallback lanes change.
3. Visible span layout keys are constructed once per request during a prepare
   call and reused by atlas planning and instance assembly. Shaped results are
   `Arc`-backed across layout workers, so the owned worker boundary does not
   clone shaped glyph storage.
4. Atlas uploads reuse staging storage and one submission where aligned copies
   are worthwhile. Small tightly packed glyphs use grouped same-shelf writes
   with zero-filled gaps, preserving atlas isolation while avoiding excessive
   256-byte row padding. Pending uploads are restored only when a later atlas
   allocation actually needs them, so a warm frame does not re-upload them.

### Release repeated-cold comparison

`text_prepare_mixed_benchmark` runs the public prepare path for 29 visible
mixed-script requests covering Latin, Greek, Cyrillic, Armenian, Georgian,
Hebrew, Arabic, Indic scripts, Southeast Asian scripts, Tibetan, CJK,
combining marks, punctuation, symbols, and emoji. Each mode uses 20 fresh
pipelines for the cold samples and one primed pipeline for warm samples. The
timer excludes pipeline construction. The two processes were run on the same
host GPU. Values are `legacy / aimer-font`.

| Scope | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Cold first sample | `244.201417 ms` | `251.441750 ms` | Aimer `1.03x` slower; process/GPU initialization noise |
| Cold repeat average | `1.420293 ms` | `895.815 us` | Aimer `1.59x` faster |
| Cold p50 | `1.323000 ms` | `894.166 us` | Aimer `1.48x` faster |
| Cold p95 | `2.714417 ms` | `1.066209 ms` | Aimer `2.55x` faster |
| Warm repeat average | `33.460 us` | `35.664 us` | Aimer `1.07x` slower |
| Warm p50 | `32.792 us` | `34.541 us` | Aimer `1.05x` slower |

The repeated-cold profile explains the result:

| Stage | Legacy | Aimer | Aimer relative |
| --- | ---: | ---: | ---: |
| Request analysis | `315 ns` | `280 ns` | `1.13x` faster |
| Key construction | `30.491 us` | `28.660 us` | `1.06x` faster |
| Fallback resolution | `22 ns` | `73.631 us` | owner-only Aimer stage |
| Font snapshot | `309 ns` | `1.947 us` | owner-only Aimer state transfer |
| Shaping | `470.813 us` | `301.947 us` | `1.56x` faster |
| Fallback + snapshot + shaping | `471.144 us` | `377.526 us` | `1.25x` faster |
| Layout | `28.059 us` | `28.096 us` | parity |
| Glyph preparation | `613.554 us` | `184.467 us` | `3.33x` faster |
| Atlas planning | `2.313 us` | `2.177 us` | `1.06x` faster |
| Atlas population | `39.017 us` | `36.712 us` | `1.06x` faster |
| Instance build | `59.980 us` | `59.598 us` | parity |
| Atlas upload | `162.754 us` | `166.438 us` | `1.02x` slower |
| Instance upload | `26.076 us` | `22.539 us` | `1.16x` faster |
| Total profiled prepare | `1.420293 ms` | `895.815 us` | `1.59x` faster |

Legacy performs fallback discovery inside its shaping path, so its explicit
owner fallback field is near zero; its fallback cost is included in the
legacy shaping measurement. The combined row is the fairer stage comparison.
The Aimer run produced 774 alpha and 2 color frame glyph instances, while
legacy produced 729 alpha and 1 color instance. Aimer is therefore faster in
this sample despite preparing more visible glyph instances; the counts differ
because the two fallback/shaping implementations select different platform
faces and color coverage.

### Quality and verification

The fallback shortcut that bypassed the dynamic Apple chain was removed after
the face-selection regression tests caught changed Japanese/Chinese adoption
order and mismatched weights. The final implementation retains that order.
Existing glyph, shaping, fallback, atlas, CJK, Arabic, Myanmar, Korean, and
quality tests remain green. The reusable atlas path changes transfer packing
only; atlas regions, UVs, glyph metrics, and coverage are unchanged.

```text
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
432 passed; 0 failed; 5 ignored

cargo test -p aimer_cupid --lib -- --test-threads=1
351 passed; 0 failed; 5 ignored

cargo check -p aimer_cupid --example text_prepare_mixed_benchmark
cargo run -p aimer_cupid --release --example text_prepare_mixed_benchmark
cargo run -p aimer_cupid --features aimer-font --release \
  --example text_prepare_mixed_benchmark
git diff --check
```

Both release benchmark processes completed with a GPU adapter. The existing
`layout_paragraph` dead-code warning and unrelated workspace manifest warnings
remain. The next performance target is warm-cache preparation and atlas
upload batching; repeated-cold fallback, request-key, and glyph-preparation
costs are now measured independently and no longer hidden inside one total.

## End-to-end mixed-fallback `TextPipelineV2::prepare` — 2026-08-31

The new `text_prepare_mixed_benchmark` example calls the public
`TextPipelineV2::prepare` path with the same 29-request screen in both build
modes. The requests cover Latin, Greek, Cyrillic, Armenian, Georgian, Hebrew,
Arabic, Persian, Urdu, Indic scripts, Thai, Lao, Khmer, Myanmar, Tibetan,
Chinese, Japanese, Korean, CJK punctuation, combining marks, symbols, and
emoji. All requests are visible in one frame and use the normal wrapping,
fallback, shaping, layout, glyph preparation, atlas insertion, instance
packing, and queue-upload path.

The release run used 20 measured iterations. Pipeline construction was kept
outside the timer. `Cold` creates a fresh pipeline for each sample; its first
sample exposes process-level font/GPU initialization, while `repeat average`
excludes that first sample. `Warm` primes one pipeline once, then measures
cache-hit preparation. Values are `legacy / aimer-font`:

| Scope | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Cold first sample | `229.236458 ms` | `231.304667 ms` | Aimer `1.01x` slower |
| Cold repeat average | `2.257903 ms` | `2.501002 ms` | Aimer `1.11x` slower |
| Cold p50 | `2.244167 ms` | `2.497291 ms` | Aimer `1.11x` slower |
| Cold p95 | `2.552625 ms` | `2.627291 ms` | Aimer `1.03x` slower |
| Warm repeat average | `32.837 us` | `33.594 us` | Aimer `1.02x` slower |
| Warm p50 | `32.458 us` | `33.417 us` | Aimer `1.03x` slower |
| Warm p95 | `37.125 us` | `36.583 us` | Aimer `1.01x` faster in this sample |

The same source requests produced `729` alpha and `1` color instance in the
legacy run, versus `773` alpha and `1` color instance in the Aimer run. That
is expected to vary with fallback-face and shaping decisions, so this is an
end-to-end workload comparison rather than a perfectly normalized equal-
glyph-count microbenchmark. The result shows that the owned persistent
boundaries have removed the large cold-start gap, but the Aimer path still has
an approximately `11%` repeated-cold cost and `2%` warm cost on this mixed
screen. The next optimization should profile the repeated-cold path at the
stage level, especially fallback resolution, per-request key construction, and
glyph/atlas work, before changing the rasterizer again.

### Verification

```text
cargo run -p aimer_cupid --release \
  --example text_prepare_mixed_benchmark
cargo run -p aimer_cupid --features aimer-font --release \
  --example text_prepare_mixed_benchmark
```

Both host runs completed with a GPU adapter. The in-sandbox adapter probe was
also attempted and correctly skipped when no adapter was available. The
existing `layout_paragraph` dead-code warning remains in the feature build.

## Full-Unicode shaping scratch and linear cluster cursor — 2026-08-31

The remaining non-ASCII shaping path now reuses its temporary shaped-glyph
storage and resolves glyph clusters with a direction-aware linear cursor:

1. `GlyphRasterizer` owns a reusable `Vec<ShapedRunGlyph>` in its run buffers.
   Full-Unicode shaping takes that storage, fills it, and returns it after
   metrics are attached; the public lower-level API keeps its existing owned
   return behavior.
2. Aimer and HarfRust output are both written into the supplied buffer, so the
   layout path no longer allocates a shaped-glyph vector for every script or
   fallback run.
3. The full-Unicode layout path detects monotonic LTR or RTL cluster output,
   advances one grapheme cursor in that direction, and derives the logical
   source boundary from the adjacent glyph group. The previous sorted,
   deduplicated cluster-start vector and per-glyph linear `find` are gone.
4. The simple ASCII path uses the same reusable output API. The two known
   shaping engines' monotonic cluster-order contract is checked with debug
   assertions; the compatibility shaping/rasterization order is otherwise
   unchanged in release builds.

The benchmark example now includes a dedicated 2,000-character mixed-script
scope (`éclair`, Arabic, Devanagari, Hebrew, CJK, and Khmer) so this slice is
measured independently of the ASCII fast path.

### Release comparison

The benchmark uses 500 iterations in separate optimized processes. Values are
`legacy / aimer-font`:

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.461120 ms` | `249.373 us` | Aimer `17.89x` faster |
| One whole ASCII shaped run, 2,000 chars | `177.266 us` | `118.093 us` | Aimer `1.50x` faster |
| One whole mixed-Unicode shaped run, 2,000 chars | `1.802665 ms` | `1.254997 ms` | Aimer `1.44x` faster |
| Direct shaping run | `98.811 us` | `40.263 us` | Aimer `2.45x` faster |
| Direct font-ID shaping run | `99.679 us` | `39.691 us` | Aimer `2.51x` faster |
| Cold serial shaping batch | `3.455659 ms` | `1.905878 ms` | Aimer `1.81x` faster |
| Cold parallel shaping batch | `1.479878 ms` | `1.054259 ms` | Aimer `1.40x` faster |

The mixed-Unicode result includes fallback discovery and multiple script runs,
so it is the relevant measurement for the linear cursor and reusable output
storage. Parallel results remain scheduler-sensitive and should be treated as
directional samples.

### Quality and verification

The feature-enabled text-layout suite passed with `426 passed; 0 failed; 4
ignored`; the no-feature suite passed with `346 passed; 0 failed; 4 ignored`.
Both benchmark feature modes compiled and completed. The quality oracle is
unchanged:

```text
Phase 2 Latin: 50 glyph samples, mean absolute coverage error 0.0616,
               max edge error 1 px, missing 0
Phase 2 CJK:   40 glyph samples, mean absolute coverage error 0.1793,
               max edge error 1 px, missing 0
```

The existing unused `layout_paragraph` warning remains. The next Phase 4
optimization should be decided from a profile of first-use and repeated
mixed-script shaping; the remaining large architectural candidate is a
persistent worker pool, but it requires an owned-task boundary because the
current preparation callbacks borrow pipeline state.

```text
cargo test -p aimer_cupid --features aimer-font text_layout --lib -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo test -p aimer_cupid --lib -- --test-threads=1
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo check -p aimer_cupid --example text_shaping_benchmark
cargo check -p aimer_cupid --features aimer-font --example text_shaping_benchmark
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 500
git diff --check
```

The focused executor suite passed with `9 passed; 0 failed`. Both release
benchmark processes completed successfully, and `rg` found no remaining
direct `rayon` dependency or `par_iter` use in the source/manifests. Cargo's
lockfile still includes Rayon transitively through the workspace's `image` /
`ravif` dependency chain.

## Batched glyph metrics during shaping — 2026-08-31

The next shaping hot spot was addressed without changing glyph geometry or
coverage:

1. `shape_text_styled` now collects the glyph keys for each same-face shaped
   run and requests their pixel metrics as one batch. The one-glyph case stays
   on the scalar path so per-cluster shaping does not pay batch setup cost.
2. `GlyphRasterizer::with_metrics_for_keys` reads the process-wide metric map
   once into a reusable aligned buffer, rasterizes only keys with no published
   metrics, and emits duplicate keys in their original order.
3. The normal raster path and the metric path share `rasterize_pending_run`,
   avoiding a second pending-key allocation while retaining the existing
   Aimer, swash, color, and platform fallback order.

### Release comparison

The benchmark uses 2,000 characters and 500 iterations in separate release
processes. The current final sample is:

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.536386 ms` | `241.940 us` | Aimer `18.75x` faster |
| One whole shaped run, 2,000 chars | `846.157 us` | `821.784 us` | Aimer `1.03x` faster |
| Direct shaping run | `97.972 us` | `41.353 us` | Aimer `2.37x` faster |
| Direct font-ID shaping run | `98.335 us` | `43.226 us` | Aimer `2.28x` faster |
| Cold serial shaping batch | `14.242988 ms` | `12.511251 ms` | Aimer `1.14x` faster |
| Cold parallel shaping batch | `4.563160 ms` | `3.958545 ms` | Aimer `1.15x` faster |

The whole-run result is near parity because the public benchmark warms the
process-wide metric cache in its earlier per-cluster section. The isolated
batch code path is nevertheless covered by the metric-order regression and
the complete shaping/raster quality suite. Across repeated runs, parallel
timings vary substantially with the host scheduler, so the table is
directional rather than a guaranteed percentage improvement.

### Quality and verification

The raster output oracle is unchanged:

```text
Phase 2 Latin: 50 glyph samples, mean absolute coverage error 0.0616,
               max edge error 1 px, missing 0
Phase 2 CJK:   40 glyph samples, mean absolute coverage error 0.1793,
               max edge error 1 px, missing 0
```

The new regression,
`batched_metrics_match_scalar_metrics_and_preserve_duplicate_order`, passed.
The stable feature-enabled library suite passed with `418 passed; 0 failed; 4
ignored`. The comparison feature check passed; the existing unused
`layout_paragraph` warning remains.

```text
cargo test -p aimer_cupid \
  batched_metrics_match_scalar_metrics_and_preserve_duplicate_order -- --nocapture
cargo test -p aimer_cupid --features aimer-font shape_text_styled --lib
cargo check -p aimer_cupid --features aimer-font-compare
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 500
git diff --check
```

The next high-value slice is the allocation-free simple-LTR/ASCII shaping
path, followed by a benchmark mode that measures genuinely cold metric state
before the warm-cache scopes. A persistent worker pool remains a separate
parallel optimization because the current preparation callbacks borrow their
pipeline state.

## Simple LTR shaping fast path — 2026-08-31

The simple shaping hot path is now implemented for the common printable ASCII
case (with explicit newlines):

1. `shape_text_styled` keeps the existing run-wide script announcement and
   font metrics, then bypasses `BidiInfo` and the materialized grapheme and
   byte-indexed line-break tables when the primary sans-serif face covers the
   complete printable ASCII input.
2. The fast path streams Unicode line-break events, creates its owned ASCII
   clusters directly, and shapes each hard-break-separated line once with the
   existing GSUB/GPOS engine. It does not bypass ligatures or kerning.
3. Glyph cluster groups are consumed in HarfRust's LTR order, so the temporary
   sorted cluster-start vector is avoided. Metric keys are filled through the
   rasterizer's reusable buffer rather than a new vector per run.
4. Any non-ASCII, bidi/control, combining, fallback, or otherwise unsupported
   input remains on the existing full Unicode path.

### Release comparison

The benchmark uses the same 2,000-character input and 500 iterations in
separate optimized processes. Values are `legacy / aimer-font`:

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.421157 ms` | `247.802 us` | Aimer `17.84x` faster |
| One whole shaped run, 2,000 chars | `177.553 us` | `119.641 us` | Aimer `1.48x` faster |
| Direct shaping run | `96.309 us` | `40.709 us` | Aimer `2.37x` faster |
| Direct font-ID shaping run | `96.881 us` | `40.304 us` | Aimer `2.40x` faster |
| Cold serial shaping batch | `3.326799 ms` | `1.918667 ms` | Aimer `1.73x` faster |
| Cold parallel shaping batch | `1.409239 ms` | `1.044371 ms` | Aimer `1.35x` faster |

The largest gain is in the whole-run scope because that input is eligible for
the simple path. The direct shaping scopes still measure the lower-level
font-run API and therefore do not include paragraph analysis. Parallel values
remain scheduler-sensitive; this is one release sample, not a contractual
percentage guarantee.

### Quality and verification

The observable shaping checks preserve the `ffi` ligature's full `1..4` source
range, hard-break clusters, and one shaping call per physical line. Complex
script, fallback, interaction, and wrapping tests remain unchanged.

The raster output oracle is unchanged:

```text
Phase 2 Latin: 50 glyph samples, mean absolute coverage error 0.0616,
               max edge error 1 px, missing 0
Phase 2 CJK:   40 glyph samples, mean absolute coverage error 0.1793,
               max edge error 1 px, missing 0
```

The feature-enabled library suite passed with `425 passed; 0 failed; 4
ignored`; the comparison and no-feature checks also compiled. The existing
unused `layout_paragraph` warning remains. The next optimization is reusable
shaping-output scratch storage and a linear cluster-boundary map for the
remaining non-ASCII/full-Unicode path.

```text
cargo test -p aimer_cupid --features aimer-font \
  simple_ascii_shaping_preserves_ligatures_and_hard_breaks --lib -- --nocapture
cargo test -p aimer_cupid --features aimer-font shape_text_styled --lib -- --nocapture
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo test -p aimer_cupid --features aimer-font \
  phase2_unhinted_output_meets_reference_quality_target -- --nocapture
cargo check -p aimer_cupid
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 500
git diff --check
```

## CJK language forms and vertical substitutions — 2026-08-31

The owned `aimer-font` layout path now handles the bounded CJK GSUB slice
needed after cmap format-14 variation sequences:

1. CJK runs carrying `TextLanguage::Chinese`, `Japanese`, or `Korean` select
   the matching `hani` language system (`ZHS `, `JAN `, or `KOR `) for `locl`.
   If a face has no exact language system, lookup selection falls back to the
   script default language system.
2. Explicit vertical-substitution mode selects `vrt2` when present and uses
   legacy `vert` only when `vrt2` is unavailable. The generic checked single
   substitution path reuses the compiled GSUB coverage plans and preserves
   glyph advances, clusters, and reset offsets after replacement.
3. The ordinary paragraph path now passes its language hint into owned shaping;
   existing Latin, Arabic, variation-sequence, fallback, and no-feature paths
   retain their previous entry points. Vertical metrics, writing-mode pen
   progression, and full CJK punctuation/vertical layout remain a later slice.

### Focused verification

The new fixture-based checks pass:

```text
applies_cjk_language_forms_from_the_requested_language_system ... ok
prefers_cjk_vrt2_over_legacy_vert_in_vertical_mode ... ok
```

The feature-enabled `aimer_cupid` library suite passed with `438 passed; 0
failed; 5 ignored`; the no-feature suite passed with `353 passed; 0 failed; 5
ignored`. `cargo check -p aimer_cupid --features aimer-font-compare` also
passed. No performance claim is attached to this correctness slice; it only
adds work for opted-in language-aware CJK or explicit vertical runs.

## CJK vertical metrics and pen progression — 2026-08-31

The owned `aimer-font` path now consumes the standard vertical metric tables
needed after `vrt2`/`vert` substitution:

1. `SfntFace` lazily validates `vhea` and `vmtx`, including the long-metric
   reuse rule for glyphs after `numberOfVMetrics`. An optional `VORG` table is
   checked for a valid version, bounded glyph IDs, and strictly ordered records.
2. A `VORG` glyph record or default origin is used directly. When `VORG` is
   absent, the glyph's actual outline top is resolved only when that glyph is
   used and memoized in a per-glyph `OnceLock`; large CJK faces do not decode
   every outline on their first vertical request.
3. Explicit vertical shaping changes the pen to top-to-bottom movement
   (`x_advance = 0`, negative `y_advance`), applies the vertical origin as a
   checked glyph offset, and then applies supported `vkrn` pair adjustments.
   The vertical advance and offset are retained through `ShapedRunGlyph` and
   `ShapedGlyph`; all horizontal runs continue to carry their previous values.
4. Faces without both usable `vhea` and `vmtx` tables return `Ok(None)` so the
   caller can use the compatibility shaper. Malformed tables return a checked
   error and cannot produce partial glyph output.

### Focused verification

The fixture coverage includes per-glyph `vmtx` reuse, `VORG` record/default
selection, malformed-table rejection, vertical pen accumulation, missing-table
compatibility fallback, and `vkrn` y-advance adjustment:

```text
vertical_* focused tests: 5 passed; 0 failed
feature-enabled aimer_cupid library suite: 443 passed; 0 failed; 5 ignored
no-feature aimer_cupid library suite: 353 passed; 0 failed; 5 ignored
cargo check -p aimer_cupid --features aimer-font-compare: passed
git diff --check: passed
```

This slice does not claim a public vertical paragraph mode yet: the current
`TextDrawRequest` API is horizontal, so the new axis-aware fields are retained
for the next writing-mode integration rather than silently changing existing
line wrapping or interaction geometry. The next Phase 4 milestone is that
public vertical column layout plus the remaining CJK vertical positioning
features, followed by Indic shaping.

## Mixed-Unicode shaping hot-path optimization — 2026-09-01

This pass keeps the owned shaping path behind `aimer-font` and targets the
large mixed-script/full-Unicode cost without changing substitution order or
raster quality:

1. Contextual GSUB lookups now prefilter against the first-input coverage
   before running the checked matcher. Formats 1, 2, and 3 are covered for
   contextual and chained-context lookups, including extension lookups. The
   candidate glyph set is an over-approximation updated after substitutions,
   so skipped subtables are only those that cannot match any current first
   glyph; the original lookup/subtable order is unchanged.
2. The first-input candidate list is retained in the existing per-thread
   `LayoutScratch`, eliminating a vector allocation on each contextual feature
   pass. A bounded bitset deduplicates glyph IDs without a hash allocation.
3. Extension GSUB lookup kinds are decoded once into an execution type while
   `LayoutState` is parsed. Hot dispatch no longer probes the extension header
   for every glyph operation.
4. Each Indic/Southeast-Asian GSUB pass obtains the cached hmtx advance slice
   once. Compiled single and ligature substitutions use that slice directly;
   zero-flag lookups also skip unnecessary GDEF ignore checks.

### Release comparison

Legacy and Aimer were run in separate optimized processes with the same 2,000
character input and 500 iterations. Values are `legacy / aimer-font`:

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.263973 ms` | `316.450 us` | Aimer `13.47x` faster |
| One whole shaped run, 2,000 chars | `177.862 us` | `112.897 us` | Aimer `1.58x` faster |
| Full Unicode run | `1.664234 ms` | `1.927908 ms` | Aimer `1.16x` slower (`15.8%`) |
| Direct shaping run | `95.586 us` | `32.436 us` | Aimer `2.95x` faster |
| Direct font-ID shaping run | `94.947 us` | `32.321 us` | Aimer `2.94x` faster |
| Cold serial shaping batch | `3.324712 ms` | `1.853255 ms` | Aimer `1.79x` faster |
| Cold parallel shaping batch | `1.478089 ms` | `1.038820 ms` | Aimer `1.42x` faster |
| Rasterization one glyph at a time, 752 glyphs | `912.675 us` | `150.739 us` | Aimer `6.06x` faster |
| Rasterization in runs, 752 glyphs | `830.641 us` | `191.551 us` | Aimer `4.34x` faster |
| Glyph-key preparation, 752 glyphs | `9.485 us` | `6.849 us` | Aimer `1.38x` faster |

The full-Unicode scope is now close to parity but remains the only shaping
scope slower than legacy. It combines bidi/grapheme and line-break analysis,
fallback resolution, and many short runs across scripts; the direct, whole-run,
cold-serial, and cold-parallel scopes all favor Aimer. A targeted owned Indic
profile measured the large `pref` contextual feature from `12.086125 ms` down
to `141.750 us` after the first-coverage prefilter, while remaining on the
owned path.

The end-to-end `TextPipelineV2::prepare` benchmark was attempted for both
profiles but skipped on this host because no GPU adapter was available. The
raster quality oracle remains unchanged: Latin mean absolute coverage error
`0.0616`, CJK `0.1793`, maximum edge error `1 px`, and no missing glyphs.

### Verification

The new contextual prefilter regression tests cover positive and negative
format-3 coverage matches, chained format-3 input coverage, and malformed
table rejection. The feature-enabled serialized suite passed with `481`
library tests, `3` binary tests, and `5` doctests; the no-feature serialized
library suite passed with `364` tests. The ordinary parallel feature suite
reproduced the two existing host-sensitive fallback test failures, while both
tests passed in isolation and in the serialized suite.

```text
cargo test -p aimer_cupid --features aimer-font context_prefilter
cargo test -p aimer_cupid --features aimer-font -- --test-threads=1
cargo test -p aimer_cupid --lib -- --test-threads=1
cargo check -p aimer_cupid
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --example text_prepare_mixed_benchmark
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_prepare_mixed_benchmark
git diff --check
```

## Full-Unicode paragraph plan and repeated-run memoization — 2026-09-01

The remaining mixed-script shaping cost is now reduced with two bounded,
quality-preserving changes:

1. The full-Unicode path builds one reusable `UnicodeClusterPlan` per grapheme.
   Each plan carries its byte range, Unicode script, and resolved face, so
   grapheme collection, script classification, and fallback resolution no
   longer require three separate scratch vectors or indexing passes.
2. The paragraph passes its final script to the owned-shaper admission check.
   Supported Arabic, Indic, and Southeast Asian runs skip the duplicate
   eligibility scan. The checked layout dispatcher remains authoritative and
   still rejects malformed, mixed, or unsupported input before the
   compatibility fallback is bypassed.
3. A paragraph-local cache remembers up to 64 exact shaped runs. Entries are
   identified by face, script, byte length, and a hash verified against the
   source bytes; the shaped glyphs are copied into the existing reusable
   rasterizer buffer on a hit. The cache is cleared at every paragraph, so
   font registration, weight, language, writing mode, and dynamic text cannot
   reuse stale output. The cache is enabled in both benchmark profiles for a
   fair comparison.

No GSUB/GPOS ordering, glyph metrics, raster algorithm, or coverage behavior
changed in this slice. The memo only avoids recomputing identical shaping work
inside one paragraph.

### Release comparison

Legacy and Aimer were run in separate optimized processes with the same 2,000
character input and 500 iterations. Values are `legacy / aimer-font`:

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.241318 ms` | `314.027 us` | Aimer `13.51x` faster |
| One whole shaped run, 2,000 chars | `175.367 us` | `112.722 us` | Aimer `1.56x` faster |
| Full Unicode run | `630.174 us` | `434.541 us` | Aimer `1.45x` faster |
| Direct shaping run | `95.102 us` | `32.759 us` | Aimer `2.90x` faster |
| Direct font-ID shaping run | `94.332 us` | `32.837 us` | Aimer `2.87x` faster |
| Cold serial shaping batch | `3.373084 ms` | `1.825912 ms` | Aimer `1.85x` faster |
| Cold parallel shaping batch | `1.520736 ms` | `1.046328 ms` | Aimer `1.45x` faster |
| Rasterization one glyph at a time, 752 glyphs | `931.164 us` | `165.007 us` | Aimer `5.64x` faster |
| Rasterization in runs, 752 glyphs | `804.995 us` | `158.468 us` | Aimer `5.08x` faster |
| Glyph-key preparation, 752 glyphs | `10.265 us` | `8.865 us` | Aimer `1.16x` faster |

The full-Unicode scope is now faster than legacy in the equivalent release
measurement. The improvement comes from repeated run reuse in the benchmark's
mixed paragraph as well as from removing the duplicate planning and admission
scans; the cache is deliberately bounded and paragraph-local to avoid turning
arbitrary application text into an unbounded process cache.

The end-to-end `TextPipelineV2::prepare` benchmark was attempted for both
profiles, but this host has no GPU adapter, so both runs reported
`skipping: no GPU adapter available`. The existing raster-quality oracle is
unchanged: Latin mean absolute coverage error `0.0616`, CJK `0.1793`, maximum
edge error `1 px`, and no missing glyphs.

### Verification

```text
feature-enabled aimer_cupid suite: 481 library tests, 3 binary tests, and 5 doctests passed; 5 library tests and 4 doctests ignored
no-feature aimer_cupid library suite: 364 passed; 0 failed; 5 ignored
cargo check -p aimer_cupid --features aimer-font-compare: passed
focused full-Unicode shaping regression: passed
git diff --check: passed
```

```text
cargo test -p aimer_cupid --features aimer-font full_unicode_shaping_reuses_output_storage_without_losing_clusters --lib -- --nocapture
cargo test -p aimer_cupid --features aimer-font --quiet -- --test-threads=1
cargo test -p aimer_cupid --lib --quiet -- --test-threads=1
cargo check -p aimer_cupid --features aimer-font
cargo check -p aimer_cupid
cargo check -p aimer_cupid --features aimer-font-compare
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --example text_prepare_mixed_benchmark
cargo run --release -p aimer_cupid --features aimer-font --example text_prepare_mixed_benchmark
git diff --check
```

## Overall release comparison — 2026-09-01

This is a fresh optimized-build comparison using the established 2,000-character,
500-iteration shaping scopes and 752-glyph rasterization scopes. Values are
`legacy / aimer-font`; lower time is better.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.309970 ms` | `301.334 us` | Aimer `14.30x` faster |
| One whole shaped run, 2,000 chars | `176.197 us` | `116.268 us` | Aimer `1.52x` faster |
| Full Unicode run | `639.261 us` | `473.300 us` | Aimer `1.35x` faster |
| Direct shaping run | `95.018 us` | `32.581 us` | Aimer `2.92x` faster |
| Direct font-ID shaping run | `95.127 us` | `32.425 us` | Aimer `2.93x` faster |
| Cold serial shaping batch | `3.351930 ms` | `1.835403 ms` | Aimer `1.83x` faster |
| Cold parallel shaping batch | `1.441212 ms` | `1.030410 ms` | Aimer `1.40x` faster |
| Rasterization one glyph at a time, 752 glyphs | `919.353 us` | `152.514 us` | Aimer `6.03x` faster |
| Rasterization in runs, 752 glyphs | `808.297 us` | `192.867 us` | Aimer `4.19x` faster |
| Glyph-key preparation, 752 glyphs | `9.459 us` | `6.795 us` | Aimer `1.39x` faster |

The fresh results keep Aimer ahead in every measured shaping and rasterization
scope. The narrowest margin is full-Unicode shaping at `1.35x`; the largest
gains remain owned glyph rasterization and per-cluster shaping. The raster
run result is noisier than the one-at-a-time result in this sample, so it should
be treated as a measurement signal for a later repeated-sample benchmark rather
than as a quality or correctness change.

The end-to-end `text_prepare_mixed_benchmark` was attempted for both profiles,
but the current checkout stops before execution with `E0308` in
`aimer_cupid/src/pipeline/image_pipeline.rs:187`: `ImageInstance::ATTRIBS` is
declared as `[VertexAttribute; 7]` while entries `0` through `7` produce eight
attributes. No unrelated image-pipeline change was made for this comparison.

```text
cargo run --release -p aimer_cupid --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_shaping_benchmark -- 500
cargo run --release -p aimer_cupid --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --features aimer-font \
  --example glyph_rasterization_benchmark -- 500
cargo run --release -p aimer_cupid --example text_prepare_mixed_benchmark
cargo run --release -p aimer_cupid --features aimer-font \
  --example text_prepare_mixed_benchmark
```

## Phase 5 bitmap-strike slice — 2026-09-01

The next Phase 5 slice adds an Aimer-owned index and placement path for common
embedded color bitmap fonts while keeping the `aimer-font` admission boundary.

- `sbix` strikes are parsed once per face, selected by nearest `ppem` with a
  larger-strike tie break, and support bounded PNG, JPEG, TIFF, and `dupe`
  records. Duplicate records have a finite recursion limit.
- `CBDT`/`CBLC` index formats 1 through 5 are checked without decoding the
  whole table. PNG image formats 17, 18, and 19 are supported, including small
  and big metrics and sparse glyph arrays.
- Decoded artwork is converted to straight RGBA8 for the existing color atlas,
  resampled with a bounded Lanczos pass, and placed from the strike's baseline
  metrics. The existing HVAR-aware shaped advance remains authoritative.
- Encoded image bytes are not retained by parsed-face state. Decoder dimensions,
  allocation size, output dimensions, table offsets, strike counts, and index
  counts are bounded before or during decode. Unsupported or malformed bitmap
  records continue through the existing compatibility fallback.

### Verification

```text
cargo test -p aimer_cupid --features aimer-font bitmap --lib -- --test-threads=1: 11 passed
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1: 487 passed, 5 ignored
cargo test -p aimer_cupid --lib --quiet -- --test-threads=1: 364 passed, 5 ignored
cargo check -p aimer_cupid --features aimer-font: passed
cargo check -p aimer_cupid --features aimer-font-compare: passed
```

The bitmap unit tests cover nearest-strike selection, `sbix` PNG baseline
placement, CBDT format-17 metrics, bounded scaling, and malformed offset
rejection. A real color-font pixel comparison is deferred until a fixture with
`sbix` or `CBDT` artwork is available. SVG glyph documents and the
Apple-private-table contract are recorded in the follow-up Phase 5 sections
below.

### Standard-font release smoke after the bitmap slice

The existing 500-iteration standard-font scopes were rerun after integration;
these are a regression smoke, not a color-font comparison. Values are
`legacy / aimer-font`.

| Workload | Legacy | Aimer | Relative result |
| --- | ---: | ---: | --- |
| Per-cluster shaping, 2,000 chars | `4.472473 ms` | `333.424 us` | Aimer `13.41x` faster |
| One whole shaped run, 2,000 chars | `185.358 us` | `121.624 us` | Aimer `1.52x` faster |
| Full Unicode run | `669.373 us` | `456.648 us` | Aimer `1.47x` faster |
| Cold serial shaping batch | `3.438753 ms` | `1.883263 ms` | Aimer `1.83x` faster |
| Cold parallel shaping batch | `1.473778 ms` | `1.203785 ms` | Aimer `1.22x` faster |
| Rasterization one glyph at a time, 752 glyphs | `917.586 us` | `153.254 us` | Aimer `5.99x` faster |
| Rasterization in runs, 752 glyphs | `865.372 us` | `210.634 us` | Aimer `4.11x` faster |
| Glyph-key preparation, 752 glyphs | `9.537 us` | `7.574 us` | Aimer `1.26x` faster |

The standard-font path remains faster in every smoke scope. Color-font decode
cost is isolated to faces advertising bitmap tables, and no color fixture was
available on this host to measure first-use decode latency or pixel parity.

## Phase 5 Apple-private table contract — 2026-09-01

The Aimer-owned path now has an explicit portable policy for Apple's private
`hvgl` outline and `emjc` color-strike data:

- The checked SFNT directory classifies direct `hvgl`/`emjc` tags, and the
  bounded `sbix` index classifies an `emjc` graphic type. Their payloads remain
  opaque; no guessed private decoder is attempted.
- A private-only face leaves the owned rasterizer before public bitmap, color,
  or outline parsing. On Apple, the existing Core Text compatibility backend
  may produce the pixels; elsewhere the normal empty-glyph fallback preserves
  the shaped advance without fabricating coverage.
- `emjc` is marked as color for fallback routing, but this does not claim that
  Aimer decodes it. Invalid public `sbix` data beside `emjc` is not entered by
  the owned path.
- If a face carries both a private table and a readable public outline, the
  public outline remains eligible. This avoids making a safe public resource
  unusable while still refusing to infer private glyph precedence.

### Verification

```text
cargo test -p aimer_cupid --features aimer-font --lib apple_private -- --test-threads=1: 2 passed
cargo test -p aimer_cupid --features aimer-font --lib classifies_hvgl -- --test-threads=1: 1 passed
cargo test -p aimer_cupid --features aimer-font --lib classifies_emjc -- --test-threads=1: 2 passed
cargo test -p aimer_cupid --features aimer-font --lib bitmap -- --test-threads=1: 12 passed
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1: 505 passed, 5 ignored
cargo test -p aimer_cupid --lib -- --test-threads=1: 368 passed, 5 ignored
cargo check -p aimer_cupid --features aimer-font: passed
cargo check -p aimer_cupid --features aimer-font-compare: passed
git diff --check: passed
```

The regression fixtures use arbitrary private payload bytes, proving the
decision depends on the validated directory and not on a speculative private
format parser. The no-platform-fallback documentation and bundled-font policy
are recorded in the next section.

## Phase 5 Core Text compatibility isolation — 2026-09-01

The Apple-private fallback is now an explicit, removable compatibility backend.
The `apple-core-text` feature owns the Apple font-discovery module,
Core Text/Core Graphics raster bridge, and their optional target dependencies.
It is enabled by default in `aimer_cupid` to preserve the migration renderer's
existing behavior. `aimer-font-core-text` is a convenience feature that enables
both `aimer-font` and the backend for applications that want Aimer shaping and
rasterization plus Apple-private glyph compatibility.

The portable profile is:

```text
cargo check -p aimer_cupid --no-default-features --features aimer-font
```

In that profile, the Apple discovery and Core Text raster modules are not
compiled. A private-only face follows the strict portable policy already
defined above: it does not enter an opaque private decoder or an implicit
platform bitmap path, and the caller retains the shaped advance with empty
coverage. The fallback tests that require installed Apple faces are likewise
scoped to `apple-core-text`; bundled/readable Aimer fonts remain testable in the
portable profile.

### Verification

```text
cargo check -p aimer_cupid --quiet: passed
cargo check -p aimer_cupid --no-default-features --features aimer-font --quiet: passed
cargo check -p aimer_cupid --no-default-features --features aimer-font-core-text --quiet: passed
cargo test -p aimer_cupid --lib --quiet -- --test-threads=1: 368 passed, 5 ignored
cargo test -p aimer_cupid --features aimer-font --lib --quiet -- --test-threads=1: 506 passed, 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font --lib --quiet -- --test-threads=1: 439 passed, 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font portable_aimer_font_does_not_call_the_platform_rasterizer --lib --quiet: 1 passed
cargo test -p aimer_cupid --no-default-features --features aimer-font-core-text --lib --quiet -- --test-threads=1: 506 passed
git diff --check: passed
```

No raster algorithm or font-cache code changed in this slice, so the previous
performance and pixel-quality baselines remain the applicable measurements.
The root-package portable check was also attempted; it remains blocked by
pre-existing `aimer_text` struct-initializer errors (`TextCluster` missing
`start_y`/`end_y` and `TextInteractionLayout` missing `writing_mode`), outside
this backend boundary.

## Phase 5 no-platform fallback and bundled-font policy — 2026-09-01

Phase 5 now has an explicit policy for builds that cannot or must not call a
platform text API. The portable command is:

```text
cargo build -p aimer_cupid --no-default-features --features aimer-font
```

On Apple targets, this profile omits system discovery and Core Text
rasterization. Private-only Apple faces carrying `hvgl` outlines or `emjc`
color strikes are unsupported by design: they are not vendored, their private
payloads are not guessed, and a declined glyph retains its shaped advance with
empty coverage. Non-Apple system fallback remains the migration behavior until
Phase 6. Applications that need reproducible cross-platform output should
ship a licensed readable replacement and register it through
`FontRegistration` before rendering, including each required weight, style,
language-specific CJK face, and color format.

The repository's current replacement inventory is:

| Asset | Role |
| --- | --- |
| `aimer_cupid/fonts/GoogleSans-Regular.ttf` | Primary Latin/common sans-serif face |
| `aimer_cupid/fonts/JetBrainsMono-Regular.ttf` | Built-in monospace face |
| `aimer_cupid/fonts/NotoSansJP-VariableFont_wght.ttf` | Lazy Japanese-oriented CJK/Han fallback |

The inventory is intentionally not presented as complete Unicode coverage.
Korean, Myanmar, Arabic, Indic, Southeast Asian, emoji, and other product
requirements need additional licensed assets or the opt-in `apple-core-text`
compatibility backend. Every added asset must pass checked-container parsing,
cmap/script coverage checks, shaping/weight/metric comparison, and raster
goldens at supported sizes and device scales. Apple system font files are not
part of the portable asset policy.

### Verification

```text
cargo test -p aimer_cupid --no-default-features --features aimer-font checked_in_portable_replacement_fonts_have_owned_outline_data --lib --quiet: 1 passed
cargo test -p aimer_cupid --no-default-features --features aimer-font portable_aimer_font_does_not_call_the_platform_rasterizer --lib --quiet: 1 passed
cargo test -p aimer_cupid --no-default-features --features aimer-font portable_profile_does_not_resolve_apple_system_faces --lib --quiet: 1 passed
cargo test -p aimer_cupid --no-default-features --features aimer-font --lib --quiet -- --test-threads=1: 440 passed, 5 ignored
cargo check -p aimer_cupid --no-default-features --features aimer-font --quiet: passed
git diff --check: passed
```

This policy slice adds no rasterizer work, so the existing Phase 5 performance
and pixel-quality measurements remain unchanged. Phase 5 is complete; Phase 6
still covers production cutover and removal of the temporary third-party
comparison stack.

## Phase 6 production cutover — 2026-09-01

The first Phase 6 milestone is complete. `aimer_cupid` now enables
`aimer-font` in its default profile, so the normal `GlyphRasterizer` dispatch
uses the Aimer-owned SFNT parser, cached OpenType shaper, fallback resolver,
and standard-font rasterizer. `apple-core-text` remains enabled alongside it
in the default Apple profile for the explicitly documented Apple-private
compatibility cases.

The legacy Swash raster path remains available through the explicit
`aimer-font-compare` feature. The feature is intentionally additive: it keeps
the Aimer parser/shaper compiled while selecting the old raster backend for
pixel and timing comparisons. A full pre-Aimer control remains available with
`--no-default-features`.

No third-party font dependency was removed in this milestone. Removal stays
deferred until the layout, interaction, color/private-font, and broader script
matrix are closed in the remaining Phase 6 work.

### Validation

```text
cargo check -p aimer_cupid --quiet
cargo check -p aimer_cupid --no-default-features --features aimer-font --quiet
cargo check -p aimer_cupid --no-default-features --features aimer-font-compare --quiet

cargo test -p aimer_cupid --lib --quiet -- --test-threads=1
  506 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --features aimer-font-compare --lib --quiet -- --test-threads=1
  510 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font --lib --quiet -- --test-threads=1
  440 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font-compare --lib --quiet -- --test-threads=1
  444 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --lib --quiet -- --test-threads=1
  302 passed; 0 failed; 5 ignored

cargo check -p aimer_canvas --quiet
cargo check -p aimer_text --quiet
git diff --check
```

The explicit downstream `aimer_text --features aimer-font` profile was not
changed to default in this first cutover. Its interaction-layout initializers
are addressed in the next milestone below. The cutover itself does not change
the benchmark scopes or raster algorithm, so the latest release performance
measurements above remain the applicable comparison.

## Phase 6 downstream interaction geometry — 2026-09-01

The Aimer-feature path in `aimer_text` now compiles and carries the same
physical geometry used by paragraph painting into the interaction snapshot.
The composed adapter fills `TextCluster::start_y` and `end_y` with the exact
line baseline used by its `TextLine`, and declares horizontal top-to-bottom
writing mode. This keeps hit testing, caret placement, selection rectangles,
and painted fragments in the same coordinate model. Source ranges continue to
come from the original span text even when transformed or ellipsized output
is used for painting.

The regression coverage checks the horizontal writing mode, line count,
line-box height, baseline equality for every cluster, transformed source
range, and rich-spacing geometry. The existing ellipsis and hard-break tests
continue to exercise visible-source boundaries and multi-line source mapping.

### Validation

```text
cargo check -p aimer_text --features aimer-font --quiet: passed
cargo test -p aimer_text --features aimer-font paragraph::tests::aimer_interaction_composes_rich_spacing_transform_and_line_box_geometry --lib --quiet -- --exact --test-threads=1: 1 passed
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1: 202 passed, 0 failed
cargo test -p aimer_text --lib --quiet -- --test-threads=1: 199 passed, 0 failed
cargo check -p aimer_cupid --quiet: passed
cargo check --no-default-features --features aimer-font --quiet: passed
git diff --check: passed
```

This milestone fixes the downstream initializer and geometry contract; it
does not yet close the complete Phase 6 verification item. Accessibility
consumers, all clipping/transform combinations, and the full cross-script
visual matrix still need explicit coverage before the temporary compatibility
stack can be removed.

## Phase 6 downstream default profile — 2026-09-01

The verified Aimer interaction path is now the default feature profile for
`aimer_canvas` and `aimer_text`. Normal downstream builds therefore compile
the source-aware Aimer layout alongside the renderer, so selectable text uses
the shared caret, hit-test, and selection geometry without requiring every
application to forward `aimer-font` manually. The feature remains an explicit
Cargo boundary: `--no-default-features` keeps the feature-off compatibility
profile available for migration checks.

This changes feature selection only; it does not alter the raster algorithm,
benchmark scope, or the optional Apple Core Text compatibility backend. The
full Phase 6 accessibility and visual matrix remains open.

### Validation

```text
cargo test -p aimer_text --lib --quiet -- --test-threads=1: 202 passed, 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1: 199 passed, 0 failed
cargo check -p aimer_canvas --quiet: passed
cargo check -p aimer_canvas --no-default-features --quiet: passed
cargo check --quiet: passed
```

## Phase 6 transform-aware interaction geometry — 2026-09-01

The selectable Aimer text path now retains the complete canvas affine
transform, rather than only its translation. Pointer coordinates are mapped
through the checked inverse transform before the shared interaction layout
performs hit testing; carets and paragraph bounds are mapped forward and
converted back to logical coordinates for the selection session. Glyph-hover
classification and the event containment check use the same local paragraph
box, which avoids accepting points that fall inside a rotated bounds AABB but
outside the painted text box.

The transform API is available through `CupidCanvas` and the framework
`CanvasRendering` adapter. Singular/non-finite transforms decline interaction
instead of producing an arbitrary offset. Existing translation-only behavior
and the feature-off compatibility path remain unchanged.

### Validation

```text
cargo test -p aimer_cupid utilities::mat3 --lib --quiet -- --test-threads=1
  4 passed; 0 failed
cargo test -p aimer_text --features aimer-font selection::selectable::tests::aimer_interaction_maps_pointer_and_caret_through_the_canvas_transform --lib --quiet -- --exact --test-threads=1
  1 passed; 0 failed
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  203 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
cargo test -p aimer_canvas --all-features --lib --quiet -- --test-threads=1
  5 passed; 0 failed
cargo test -p aimer_cupid --lib --quiet -- --test-threads=1
  510 passed; 0 failed; 5 ignored
cargo check -p aimer_cupid --all-features --quiet: passed
cargo check -p aimer_canvas --no-default-features --quiet: passed
git diff --check: passed
```

This closes the affine-transform portion of the Phase 6 interaction contract.
Accessibility semantics, the complete cross-script visual matrix, and
third-party stack removal remain open verification work.

## Phase 6 source-aware accessibility geometry — 2026-09-01

`aimer_text` now exposes `TextAccessibilitySnapshot` behind the `aimer-font`
feature. The snapshot is constructed from the same `TextInteractionLayout`
that the Aimer renderer and selectable text path retain. It carries the
original logical UTF-8 text and source ranges while preserving visual cluster
order, resolved bidi levels, line metrics, baseline endpoints, transformed
line/cluster bounds, caret rectangles, hit testing, and logical selection
rectangles. Device-scale conversion and the full affine canvas transform are
applied once at the snapshot boundary, so host adapters receive absolute
logical coordinates.

The snapshot validates line/cluster source ranges, line counts, finite metrics,
and transformed geometry. Singular transforms return `None`; invalid device
scales use the existing scale-one compatibility rule. Vertical writing now
reports a horizontal caret band with the same cross-axis width as its selection
band. `RawSelectableText` and `RawRichText` expose the last painted snapshot,
and rich text retains its shared interaction layout even when selection is
disabled. The model stays independent of `aimer_accessibility`: that crate's
generic `SemanticNode` remains the host-facing tree, while this richer text
payload is the renderer/layout adapter seam.

### Validation

```text
cargo test -p aimer_text --features aimer-font text_accessibility::tests --lib --quiet -- --test-threads=1
  5 passed; 0 failed
cargo test -p aimer_text --features aimer-font selection::selectable::tests::aimer_interaction_maps_pointer_and_caret_through_the_canvas_transform --lib --quiet -- --exact --test-threads=1
  1 passed; 0 failed
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  208 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
cargo test -p aimer_cupid --lib --quiet -- --test-threads=1
  510 passed; 0 failed; 5 ignored
cargo check -p aimer --all-features --quiet: passed
git diff --check: passed
```

This closes the source-aware text geometry portion of the accessibility
contract. A platform adapter still needs to map the snapshot into the generic
semantic tree, and the complete mixed-script, clipping, color-glyph, and pixel
golden matrix remains open before Phase 6 can close.

## Phase 6 generic semantic-tree adapter — 2026-09-01

`TextAccessibilitySnapshot` now maps into the generic
`aimer_accessibility::SemanticNode` and `SemanticTree` models through
`to_semantic_node` and `to_semantic_tree`. The host supplies the stable
`NodeId`; the projection publishes the complete logical source text as the
`Role::Text` accessible name and converts the already transformed paragraph
bounds into the accessibility crate's validated logical bounds type.

The projection intentionally remains one semantic text node. Visual lines,
bidi clusters, source ranges, caret geometry, and selection rectangles are
interaction units rather than independent accessible content, so they remain
on the source snapshot for the platform adapter to consume alongside the
generic node. This avoids duplicate announcements and prevents a second
layout/shaping path from drifting from the painted result. The
`aimer_accessibility` dependency is optional and activated only by
`aimer-font`; the feature-off profile remains independent.

### Validation

```text
cargo test -p aimer_text --features aimer-font text_accessibility::tests::snapshot_maps_to_a_generic_semantic_text_node --lib --quiet -- --exact --test-threads=1
  1 passed; 0 failed
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  209 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
cargo test -p aimer_accessibility --quiet
  passed
cargo check -p aimer --all-features --quiet
  passed
git diff --check
  passed
```

This closes the generic semantic-node projection slice. Full Phase 6 still
requires the broader mixed-script/clipping/color-glyph/pixel-golden matrix and
the final third-party stack removal review.

## Phase 6 bundled-font verification matrix — 2026-09-01

The first deterministic Phase 6 matrix is now checked in as
`aimer_cupid::pipeline::text_pipeline::phase6_verification`. It uses only the
licensed/readable assets already checked into the repository: Google Sans for
common-script shaping and Noto Sans JP for the registered CJK fallback. The
matrix verifies that Latin, Greek, Cyrillic, Hebrew, Devanagari, Thai, Lao,
Khmer, CJK, and combining-mark samples retain their original UTF-8 cluster
ranges, never resolve to `.notdef`, and produce finite metrics plus non-empty
Aimer-owned bitmap coverage.

The accessibility side now has a matching full-Unicode source/semantic
projection test covering Latin, Greek, Cyrillic, Hebrew, Arabic, Devanagari,
Bengali, Tamil, Thai, Khmer, Myanmar, Chinese, Japanese, Korean, combining
marks, emoji, symbols, affine transforms, and the generic semantic tree. This
test validates source-boundary preservation without making platform fallback
bytes part of the portable baseline.

Arabic, emoji, Korean, and Myanmar are not silently promoted to portable
owned-font coverage: the current bundled inventory does not provide a readable
replacement for every one of those scripts. Arabic shaping and color glyph
decoders remain covered by deterministic synthetic fixtures, while real host
fallback/color-strike pixel parity still requires checked-in licensed assets or
a platform-specific test fixture.

### Validation

```text
cargo test -p aimer_cupid --features aimer-font pipeline::text_pipeline::phase6_verification --lib --quiet -- --test-threads=1
  1 passed; 0 failed
cargo test -p aimer_cupid --features aimer-font --lib --quiet -- --test-threads=1
  511 passed; 0 failed; 5 ignored
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  210 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
cargo test -p aimer_accessibility --quiet
  6 passed; 0 failed
```

This closes the checked-in bundled-font and source-preservation portion of the
Phase 6 matrix. Clipping/transform combinations beyond the existing unit
coverage, real color-font pixel goldens, host fallback samples, and final
third-party dependency removal remain open.

## Phase 6 color-glyph and host-fallback verification — 2026-09-01

The owned color path now has a deterministic pixel regression for a checked
synthetic COLR v0/CPAL fixture. The same Aimer-owned face is rasterized at 16,
24, and 32 px, and each RGBA bitmap is checked for its exact dimensions and
FNV-1a fingerprint:

| Size | Bitmap | RGBA fingerprint |
| ---: | ---: | ---: |
| 16 px | 2 × 2 | `14551470036939313687` |
| 24 px | 3 × 3 | `16467940706764202044` |
| 32 px | 4 × 4 | `3532343437148095129` |

The test also checks that the output is marked color, has the expected RGBA
stride, and contains visible pixels. This locks the owned COLR compositor at
multiple raster sizes without treating a host-dependent color font as a
portable fixture. Device-scale, transform, subpixel-phase, and a licensed
multi-layer production color font remain broader golden work.

On Apple with `aimer-font` and `apple-core-text`, a host-gated fallback matrix
now exercises Arabic (`م`), emoji (`😀`), Korean (`한`), and Myanmar (`မ`). Each
sample must resolve to a non-`.notdef` glyph and a non-empty fallback bitmap.
Because these bytes and private color/outline formats are owned by the host,
the test is compatibility coverage only; it does not change the portable
contract, which still requires bundled licensed replacements for deterministic
cross-platform rendering.

### Validation

```text
cargo test -p aimer_cupid --features aimer-font pipeline::text_pipeline::aimer_font::tests::owned_colr_rasterization_matches_size_pixel_goldens --lib --quiet -- --exact --test-threads=1
  1 passed; 0 failed
cargo test -p aimer_cupid --features aimer-font pipeline::text_pipeline::glyph_rasterizer::tests::apple_host_fallback_matrix_rasterizes_nonbundled_script_samples --lib --quiet -- --exact --test-threads=1
  1 passed; 0 failed
cargo test -p aimer_cupid --features aimer-font --lib --quiet -- --test-threads=1
  513 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font --lib --quiet -- --test-threads=1
  445 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --lib --quiet -- --test-threads=1
  304 passed; 0 failed; 5 ignored
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  210 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
cargo check -p aimer_cupid --no-default-features --features aimer-font-compare --quiet
  passed
cargo test -p aimer_cupid --no-default-features --features aimer-font-compare --lib --quiet -- --test-threads=1
  448 passed; 0 failed; 5 ignored
cargo check -p aimer --all-features --quiet
  passed
git diff --check
  passed
```

The remaining Phase 6 verification is the complete portable color/script
matrix, clipping and transformed pixel goldens, fallback lifecycle coverage,
and the final third-party stack removal review.

## Phase 6 physical raster and transformed-geometry matrix — 2026-09-01

The owned grayscale path now has a deterministic matrix that converts logical
font sizes to physical raster sizes at 1×, 1.5×, and 2× device scale while
also exercising distinct x/y subpixel phases. The checked snapshots use the
bundled primary face's `A` glyph and retain exact dimensions, thousandths of a
pixel bearings, and FNV-1a coverage fingerprints:

| Logical size | Device scale | Phase (x, y) | Bitmap | Bearings (x, y) | Coverage fingerprint |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 16 px | 1× | (0, 0) | 11 × 12 | (0, 0) | `6178879141678921614` |
| 16 px | 1.5× | (2, 3) | 16 × 18 | (-250, -375) | `1722626256821631889` |
| 16 px | 2× | (4, 5) | 22 × 24 | (-500, -625) | `18012845966783831635` |
| 24 px | 1× | (7, 1) | 16 × 18 | (125, -125) | `5544611616453397900` |
| 24 px | 1× | (0, 0) | 16 × 18 | (0, 0) | `13278534332415583524` |

The matrix also checks grayscale stride (`width × height`), visible coverage,
and the color/monochrome contract. A separate transformed-box regression
maps a negative-bearing local box through translation and scale, snaps its
physical origin to whole pixels, and verifies that a partial clip is retained
while a fully disjoint clip is rejected. This exercises the same geometry
helpers used by the draw path without introducing a GPU-dependent image test.

### Validation

```text
cargo test -p aimer_cupid --features aimer-font pipeline::text_pipeline::phase6_verification --lib --quiet -- --test-threads=1
  3 passed; 0 failed
cargo test -p aimer_cupid --features aimer-font --lib --quiet -- --test-threads=1
  515 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font --lib --quiet -- --test-threads=1
  447 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font-compare --lib --quiet -- --test-threads=1
  448 passed; 0 failed; 5 ignored
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  210 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
cargo check -p aimer --all-features --quiet
  passed
git diff --check
  passed
```

Actual descender/empty-glyph boundary samples, full device-scale/transform
color goldens, fallback lifecycle coverage, and the final third-party stack
removal review remain open.

## Phase 6 close-out and dependency audit — 2026-09-02

The remaining Phase 6 boundary coverage is now checked in. The portable Aimer
profile verifies descender placement below the baseline, finite negative
bearings, empty-space advances, and shared shaped/layout metrics. The existing
registration, lazy fallback-lane, release/reload, replacement-invalidation,
worker-snapshot, host fallback, color, clipping, and accessibility regressions
complete the current supported-font contract.

The dependency cleanup was limited to dependencies that the source audit
proved unnecessary:

| Change | Result |
| --- | --- |
| `naga` | Moved out of Cupid's normal dependencies; retained as a dev-dependency for WGSL shader tests. |
| Direct `rayon` | Removed from Cupid; no direct source use remained. |
| `image` defaults | Workspace default features are disabled; Cupid enables only `png`, `jpeg`, and `tiff`. `aimer_assets` explicitly restores its broad codec and `rayon` profile. |
| `skrifa`, `harfrust`, `swash` | Retained intentionally: feature-off compatibility, comparison tests, and last-resort fallback code call them directly. |
| `fontique` | Retained intentionally for non-Apple system-font discovery. |

The last four entries are not unused dependencies under the current feature
contract. Removing them would be a breaking removal of the explicit
compatibility/comparison profiles and the unsupported-font fallback behavior;
Phase 6 therefore closes the owned production path while preserving those
profiles.

### Validation

```text
cargo check -p aimer_cupid --no-default-features --features aimer-font --quiet
  passed (existing dead-code warnings only)
cargo test -p aimer_cupid --features aimer-font phase6_ --lib --quiet -- --test-threads=1
  4 passed; 0 failed
cargo test -p aimer_cupid --features aimer-font --lib --quiet -- --test-threads=1
  516 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font --lib --quiet -- --test-threads=1
  448 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --features aimer-font-compare --lib --quiet -- --test-threads=1
  448 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features --lib --quiet -- --test-threads=1
  304 passed; 0 failed; 5 ignored
cargo test -p aimer_text --features aimer-font --lib --quiet -- --test-threads=1
  210 passed; 0 failed
cargo test -p aimer_text --no-default-features --lib --quiet -- --test-threads=1
  199 passed; 0 failed
git diff --check
  passed
```

`cargo tree -p aimer_cupid --no-default-features --features aimer-font
--edges normal` no longer lists Cupid's direct `naga` or `rayon` entries, and
the Aimer image path resolves only the PNG/JPEG/TIFF codec dependencies. A
separate `aimer_assets` test and whole-workspace/all-feature checks were blocked
by an unrelated pre-existing root-manifest typo, `webbrowser = "1.2f.1"`; that
change was left untouched.

## Final Aimer-only profile and legacy-stack removal — 2026-09-02

This section supersedes the earlier feature-gated close-out notes above. The
`aimer-font`, `aimer-font-compare`, and `aimer-font-core-text` Cargo features
are removed. Cupid now always uses its owned SFNT parser, fallback resolver,
Unicode shaper, and rasterizer. `apple-core-text` remains the only optional
font feature: it supplies Apple discovery and the private-glyph bridge for
faces whose payload is intentionally not portable. `--no-default-features`
therefore means “portable Aimer”, not “legacy font engine off”.

The legacy font dependencies and their compatibility/comparison code were also
removed from the workspace: `skrifa`, `harfrust`, `fontique`, and `swash` no
longer occur in source, manifests, or the resolved Cupid dependency tree.
SVG glyph documents now use Cupid's bounded in-house SVG parser; the workspace
no longer depends on `usvg` or `lyon` for SVG decoding or tessellation.

### Last apples-to-apples comparison before removal

These release measurements were captured immediately before deleting the old
engines, with the same benchmark scopes and 500 iterations. Lower time is
better.

| Workload | Legacy | Aimer | Aimer result |
| --- | ---: | ---: | ---: |
| One whole shaped run, 2,000 chars | `184.230µs` | `118.212µs` | `1.56× faster` |
| Full Unicode run | `263.277µs` | `263.138µs` | `1.00×; 0.05% faster` |
| Cold serial shaping batch | `3.467355ms` | `1.859448ms` | `1.86× faster` |
| Cold parallel shaping batch | `1.484232ms` | `1.152392ms` | `1.29× faster` |
| One-at-a-time rasterization, 752 glyphs | `945.555µs` | `154.946µs` | `6.10× faster` |
| Rasterization in runs, 752 glyphs | `869.781µs` | `188.839µs` | `4.61× faster` |

The legacy column is preserved as historical evidence because that renderer is
no longer buildable. It is not a current compatibility target.

### Final portable Aimer measurements after removal

The final commands were run without any font feature flag:

```text
cargo run --release -p aimer_cupid --no-default-features \
  --example text_shaping_benchmark -- 500

release text shaping benchmark: 2000 characters, 500 iterations
per-cluster average: 285.672µs
per-run average:     121.973µs
full Unicode run:    228.279µs
direct run average:  34.736µs
direct font-id run:  34.331µs
cold serial batch:   1.855414ms
cold parallel batch: 1.070394ms
cold batch speedup:  1.73×
```

```text
cargo run --release -p aimer_cupid --no-default-features \
  --example glyph_rasterization_benchmark -- 500

release cold glyph rasterization: 752 distinct glyphs, 500 iterations
one glyph at a time: 144.532µs (192ns per glyph)
in runs:             189.206µs (251ns per glyph)
key preparation:       6.537µs (8ns per glyph)
saved:                       -31%
```

The final no-default profile is the production Aimer implementation and is
not a second renderer. The raster “in runs” number is a small-batch workload;
the one-at-a-time path remains the better cold result for this particular
752-glyph sample.

### Removal verification

```text
cargo test -p aimer_cupid --lib --quiet -- --test-threads=1
  511 passed; 0 failed; 5 ignored
cargo test -p aimer_cupid --no-default-features
  passed earlier in the same removal pass
cargo test -p aimer_text --no-default-features
  210 passed; 6 doctests passed; 7 ignored
cargo tree -p aimer_cupid --all-features | rg \
  'skrifa|harfrust|fontique|swash'
  no matches
rg -n 'aimer-font|aimer-font-core-text|aimer-font-compare|skrifa|harfrust|fontique|swash' \
  --glob '!target/**' --glob '!Cargo.lock' --glob '!zodiac_plans/**' .
  no matches
git diff --check
  passed
```

The default Apple profile also passes the CJK weight regressions after the
removal. Weight-aware collection selection now tries an in-tolerance cut
before falling back to the nearest readable face, so a bold private/collection
face is not replaced by a lighter readable sibling merely because one glyph
needs the Apple bridge.
