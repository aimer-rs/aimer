# Font rasterizing plan

This plan covers replacing the current third-party font-processing stack with
an Aimer-owned font parser, shaper, rasterizer, and fallback system while
keeping the existing WGPU glyph-atlas renderer. The replacement is controlled
by the Cargo feature `aimer-font`; after the Phase 6 production cutover it is
enabled by default by `aimer_cupid` and the downstream canvas/text consumers,
while the feature remains available for explicit forwarding and portable
profile selection.

The important distinction is that text shaping and glyph rasterization are
different operations. Shaping chooses glyph IDs and positions; rasterization
turns those glyphs into pixels. A complete from-scratch implementation must
own both.

## Decision

The target is an Aimer-owned portable font stack for standard TTF/OTF/TTC
fonts. The existing glyph atlas, cache, clipping, and WGPU quad path remain the
rendering backend.

### Feature boundary

- The public `aimer` feature `aimer-font` forwards to
  `aimer_cupid/aimer-font`.
- `aimer_cupid` exposes the same feature for direct engine users and enables it
  in its default production profile together with `apple-core-text`.
- `--no-default-features --features aimer-font` selects the portable Aimer
  profile without Apple Core Text or implicit Apple system fallback.
- `aimer-font-compare` additionally selects the legacy swash rasterizer for
  side-by-side measurements while `aimer-font` keeps the Aimer-owned path.
- New parser, shaper, rasterizer, and feature-specific tests belong behind
  `#[cfg(feature = "aimer-font")]` until the replacement is complete.
- The feature path is self-supplied-font first: registered or bundled font
  bytes are the source of truth. It does not promise OS font discovery or
  Core Text fallback.

### Initial support contract

The first `aimer-font` implementation is intentionally bounded:

- Input containers are standalone TTF/OTF and TTC; the first parser milestone
  accepts TrueType `glyf`/`loca` outlines, while CFF/CFF2 support remains an
  explicit parser milestone before those faces are claimed as supported.
- The first script gate is Latin, combining marks, Han, Hiragana/Katakana, and
  Hangul. RTL text is part of the baseline contract; Arabic, Indic, and other
  complex-script shaping remains a later shaping milestone.
- The first raster output is monochrome 8-bit coverage for the R8 atlas. Color
  glyphs, variable axes, and Apple-private glyph data are deferred.
- The initial hinting policy is unhinted outlines. TrueType instruction
  execution requires a separate quality comparison before it is enabled.
- Initial resource limits are 128 MiB per font blob, 64 MiB per table, 64 faces
  per TTC, and 128 MiB of bitmap coverage per rasterizer. The existing
  process-wide metric cache remains bounded at 16,384 entries.

These limits are the Phase 0 contract for the parser and cache work. Raising a
limit or expanding the first script/feature gate requires a new baseline.

### Frozen pipeline contracts

- Loading begins with [`FontRegistration`](../aimer_cupid/src/font.rs) and
  produces an immutable registered face. `FontFamily` selects a family;
  `FontId` identifies a concrete face; registration owns a copy of the input
  bytes and never depends on a screen or GPU object.
- Shaping produces glyph IDs, source clusters, advances, offsets, direction,
  and runs through [`ShapedGlyph`](../aimer_cupid/src/pipeline/text_pipeline/text_layout.rs)
  and [`ParagraphLayout`](../aimer_cupid/src/pipeline/text_pipeline/text_layout.rs).
  Layout and interaction consume those values without querying a rasterizer.
- Rasterization consumes a [`GlyphKey`](../aimer_cupid/src/pipeline/text_pipeline/glyph_rasterizer.rs)
  and returns the existing `RasterizedGlyph` contract: bitmap, dimensions,
  bearings, advance, and color state. Empty coverage is valid when metrics are
  still meaningful.
- GPU preparation consumes only glyph keys, rasterized pixels, and atlas
  regions. Font parsing, shaping, and bitmap generation remain CPU work and
  must not receive WGPU handles.

Exact support for every Apple system font is a separate compatibility decision.
Some Apple fonts contain private tables that are not documented as ordinary
OpenType outlines. A strict no-platform-renderer build cannot promise those
fonts. The practical options are:

- Bundle and control the supported fonts, with no Apple rendering dependency.
- Keep Core Text as an explicit optional fallback for Apple-private fonts.

Core Text must not be part of the portable renderer contract if the goal is a
fully independent renderer.

## Current state

| Concern | Current implementation | Target |
| --- | --- | --- |
| Feature selection | Aimer-owned backend is feature-controlled | `aimer_cupid` defaults to `aimer-font`; `aimer-font-compare` is the explicit legacy raster comparison profile |
| Font discovery and matching | `fontique`, platform discovery, and local fallback logic | Aimer-owned collection, matching, and fallback resolver |
| Font parsing and metrics | `skrifa` | Aimer-owned safe SFNT parser |
| Text shaping | `harfrust` | Aimer-owned Unicode/OpenType shaper |
| Bidi and line breaking | `unicode_bidi` and `unicode_linebreak` | Aimer-owned Unicode layout layer, or explicitly retained as non-font dependencies |
| Standard-font rasterization | `swash` plus the local outline path | Aimer-owned scaler and scan converter |
| Apple-private glyphs | Core Text temporary bitmap fallback | Bundled supported fonts, or an optional Core Text compatibility backend |
| Glyph caching | Aimer glyph and metric caches | Keep and adapt to the new rasterizer |
| GPU rendering | Aimer R8/RGBA glyph atlases and WGPU quads | Keep unchanged except for format and cache-contract fixes |

Relevant seams are [`GlyphRasterizer`](../aimer_cupid/src/pipeline/text_pipeline/glyph_rasterizer.rs),
[`text_layout`](../aimer_cupid/src/pipeline/text_pipeline/text_layout.rs),
[`glyph_outline`](../aimer_cupid/src/pipeline/text_pipeline/glyph_outline.rs),
[`core_text_raster`](../aimer_cupid/src/pipeline/text_pipeline/core_text_raster.rs),
and [`glyph_atlas`](../aimer_cupid/src/pipeline/text_pipeline/glyph_atlas.rs).

## Goals

- [ ] Under `aimer-font`, render supported standard fonts without `swash`, `skrifa`, or another
      third-party font engine.
- [ ] Own font loading, face selection, glyph mapping, shaping, outline
      extraction, rasterization, fallback, and glyph metrics.
- [ ] Preserve the existing `GlyphKey`/`RasterizedGlyph` and atlas contracts
      where they are sound, limiting downstream migration.
- [ ] Support deterministic output for bundled fonts across platforms.
- [ ] Keep rasterization off the UI thread and preserve bounded memory use.
- [ ] Make unsupported font features explicit instead of silently producing
      shifted, missing, or offscreen glyphs.

## Non-goals

- [ ] Reimplement the WGPU text shader or glyph atlas without a measured need.
- [ ] Promise exact visual parity with Apple's private system fonts in a strict
      portable build.
- [ ] Add every OpenType feature before the basic parser and rasterizer are
      validated.
- [ ] Treat a temporary bitmap as a screen position; it is only an intermediate
      representation before atlas upload.

## Required capabilities

### Font file parser

The parser must be bounds-checked, allocation-aware, and safe for untrusted
font bytes. The first supported format set should include:

- standalone TTF and OTF files;
- TrueType collections (`.ttc`) and face selection;
- `cmap` formats needed for Unicode, including format 4, 12, and 14;
- naming, units-per-em, ascender, descender, line-gap, and bounding-box data;
- horizontal metrics and glyph advances;
- TrueType `glyf`/`loca` outlines;
- CFF and CFF2 outlines;
- OpenType table directory validation, checked offsets, checked lengths, and
      overflow-resistant arithmetic.

Later parser phases must cover:

- variable-font axes and deltas (`fvar`, `gvar`, `HVAR`, `VVAR`, `MVAR`);
- TrueType hinting tables and bytecode, if hinting is part of the quality
      target;
- color glyph data (`COLR`/`CPAL`, `sbix`, `CBDT`/`CBLC`, and SVG);
- vertical metrics and vertical substitutions for CJK.

### Unicode and shaping

The shaper must operate on Unicode text rather than treating one codepoint as
one independently positioned glyph. It needs:

- UTF-8 decoding and stable byte-to-cluster mapping;
- grapheme segmentation;
- script and language-run segmentation;
- Unicode bidirectional ordering (UAX #9);
- line-break opportunities (UAX #14);
- combining-mark behavior and mark attachment;
- glyph substitution (GSUB), including ligatures and contextual forms;
- glyph positioning (GPOS), including kerning, mark positioning, and cursive
      attachment;
- Arabic, Indic, Southeast Asian, Hebrew, and other complex-script behavior;
- CJK punctuation, variation selectors, vertical forms, and language-specific
      glyph choices;
- emoji variation selectors, ZWJ sequences, and cluster-preserving hit testing.

Shaping output must preserve, for every glyph:

- glyph ID and source cluster;
- x/y offset and advance;
- direction and visual-run order;
- the source range used by selection, caret, links, and accessibility.

### Font fallback and matching

The resolver must own:

- Unicode coverage checks;
- family, style, weight, stretch, and variable-axis matching;
- fallback-run construction without splitting combining sequences;
- Chinese Simplified, Traditional, Japanese, and Korean selection;
- emoji and color-font preference;
- missing-glyph behavior and advance preservation;
- deterministic bundled-font precedence;
- lazy loading and release of large CJK faces.

System-font discovery and font-file parsing are separate concerns. A path found
by the operating system is useful only when the face bytes are in a format the
Aimer parser can decode.

### Outline scaler and rasterizer

The Aimer rasterizer must implement:

- TrueType quadratic curves;
- CFF/CFF2 cubic curves;
- contour winding and empty-contour handling;
- scale, transform, and device-pixel-ratio conversion;
- grayscale coverage generation into the atlas's R8 representation;
- color-glyph compositing into the RGBA representation;
- subpixel positioning and stable rounding rules;
- bearings, bitmap bounds, ascent/descent, and advance metrics;
- a documented hinting policy: implement TrueType hinting or explicitly use
      unhinted outlines with a measured quality target;
- deterministic behavior at fractional sizes and fractional pen positions.

The rasterizer result must continue to provide the equivalent of:

```text
bitmap, width, height, offset_x, offset_y, advance_width, is_color
```

`offset_x` and `offset_y` are part of placement correctness. The low-level text
API uses a baseline origin, so tests must cover ascenders at `y = 0`, descenders,
negative bearings, and glyphs whose bitmap is intentionally empty.

### Layout and interaction

The layout layer must use the same shaped advances and metrics as painting for:

- wrapping and line breaking;
- baseline and line-height calculation;
- ellipsis and truncation;
- alignment and justification;
- caret placement and hit testing;
- selection rectangles and grapheme navigation;
- text decoration bounds;
- mixed-font and mixed-direction runs;
- optional vertical CJK layout.

### Cache and GPU integration

The new stack must preserve the existing performance model:

- cache font metadata and table lookups;
- cache glyph IDs and shaped runs with explicit invalidation keys;
- cache glyph metrics independently from coverage bitmaps;
- rasterize a face's pending glyphs in batches;
- pack monochrome glyphs into the R8 atlas and color glyphs into the RGBA atlas;
- upload only newly rasterized glyphs;
- evict coverage under a memory budget without losing valid metrics;
- make cache keys include font face, glyph ID, size, variation state, weight,
      transform, and subpixel phase where those affect output;
- keep all mutable rasterization state worker-local or synchronized with an
      explicit ownership rule.

## Implementation phases

### Phase 0 — Define the contract and baseline

- [x] Decide that “from scratch” excludes third-party font engines first;
      existing Unicode segmentation, bidi, and line-break crates may remain
      temporarily until equivalent Aimer-owned replacements are justified.
- [x] Define the initial supported-font policy: `aimer-font` consumes
      registered or bundled TTF/OTF/TTC bytes; readable system fonts and an
      Apple compatibility backend are separate follow-up policies.
- [x] Reserve the `aimer-font` feature boundary in the root crate and
      `aimer_cupid`; default builds remain unchanged during migration.
- [x] Define the first supported font features, scripts, color formats, hinting
      policy, and maximum font/table/cache sizes in the initial support
      contract above.
- [x] Freeze the interfaces between font loading, shaping, rasterization, and
      GPU preparation in the pipeline contracts above.
- [x] Record baseline glyph metrics, shaped positions, bitmap fingerprints,
      cache behavior, and available host timings in
      [`FONT_RASTERIZING_BASELINE.md`](FONT_RASTERIZING_BASELINE.md). The GPU
      benchmark was attempted and reported that this host has no adapter.
- [x] Add golden tests for Latin, CJK, combining marks, RTL text, mixed-font
      runs, fractional sizes, and baseline clipping in
      [`phase0_baseline.rs`](../aimer_cupid/src/pipeline/text_pipeline/phase0_baseline.rs).

Phase 0 is complete for the repository contract. A GPU-equipped host still
needs to rerun the resize and scroll benchmarks before any frame-time claim is
made.

### Phase 1 — Implement the safe SFNT reader

- [x] Add an internal `aimer_font` module with checked big-endian readers and
      table-directory access.
- [x] Implement TTF/OTF/TTC face loading and `cmap` lookup.
- [x] Implement names, metrics, glyph advances, and font coverage queries.
- [x] Implement bounds-checked TrueType `glyf`/`loca` outline extraction,
      including simple contours and translated/scaled composite glyphs.
- [x] Implement bounded CFF/CFF2 Type 2 outline extraction, including cubic
      paths, local/global subroutines, FDArray/FDSelect selection, and explicit
      rejection of CFF2 variation `blend` until axis coordinates are supported.
- [x] Reject malformed or oversized tables deterministically.
- [x] Add cargo-fuzz harnesses for table-directory/cmap/name input and
      TrueType composite plus CFF/CFF2 outline input. The harnesses cap each
      iteration at 4 MiB and exercise the same checked parser entry points.
- [x] Compare the Aimer reader with read-only `skrifa` on the checked-in
      JetBrains face: global metrics, every glyph advance, selected cmap
      mappings, and glyph bounds match at the unscaled/default instance.

The portable PostScript outline slice is static-instance support. CFF2
variation stores, `vsindex`/`blend`, and variable metric deltas remain in the
variable-font phase; those inputs fail explicitly rather than being rendered
with an implicit or mismatched instance.

### Phase 2 — Implement standard-font rasterization

- [x] Build an Aimer outline representation independent of the current helper
      implementation.
- [x] Implement curve flattening or direct scan conversion with deterministic
      coverage rules.
- [x] Produce the exact `RasterizedGlyph` metrics required by layout and atlas
      upload.
- [x] Add fractional-size and fractional-position image tests.
- [x] Measure unhinted output against the quality target before deciding whether
      to implement TrueType instruction execution.
- [x] Route simple standard-font glyphs through the new rasterizer while
      retaining the old path behind a comparison feature during migration.

The Phase 2 implementation uses static `glyf`/`loca` and CFF/CFF2 outlines,
normalizes them into one Aimer path representation, flattens curves with a
bounded tolerance, and scan-converts them with an 8x8 even-odd sample grid.
`aimer-font-compare` selects the legacy swash path for A/B checks. The current
path is intentionally unhinted. The quality gate is covered by
`phase2_unhinted_output_meets_reference_quality_target` in
`glyph_rasterizer.rs`, which compares the Aimer output with the legacy hinted
swash output for 50 checked-in JetBrains Mono Latin samples and 40 checked-in
Noto Sans JP CJK samples at 9, 12, 16, 24, and 32 px. The target is no missing
coverage, no more than a one-pixel bound difference, and mean absolute coverage
error no greater than `0.30`; the observed errors are `0.0580` (Latin) and
`0.0156` (CJK), with zero missing samples and maximum bound differences of one
and zero pixels respectively. Advances are also checked against the reference.
This closes Phase 2 without a TrueType instruction interpreter: the remaining
pixel differences are the expected small-size hinting divergence, not an
offscreen or metric failure. Instruction execution remains a future quality
option if a product requirement later demands exact platform hinting.

### Phase 3 — Own font resolution and fallback

- [x] Implement the first feature-gated internal face-validation and
      family/style/weight metadata path.
- [x] Add coverage-index caches and language-aware CJK fallback.
- [x] Add deterministic bundled-font registration, a Japanese-only bundled
      lane, and language-specific Chinese/Korean fallback lanes.
- [x] Preserve the existing font IDs and invalidate all dependent metric,
      shaping, and bitmap caches when a registration is removed or replaced.
- [x] Add mixed Latin/CJK/emoji fallback tests, including combining sequences.
- [x] Load platform-independent fallback discovery one script lane at a time,
      with stable per-lane ids and release/reload support.

Phase 3 is complete for font resolution and fallback ownership. The existing
`FontRegistry` API validates registered faces through the Aimer reader under
`aimer-font`; face coverage, color/outline metadata, design weight, advances,
line metrics, fallback glyph decodability, and cache invalidation all follow
the same face identity. Local rasterizers keep lazy cmap and script answers in
face-scoped coverage indexes, while process-wide fallback faces keep a
separate immutable decodability index. Both key script answers by the complete
`ScriptRequirement` value, so hash collisions cannot make a CJK run reuse
another run's face. `ScriptRequirement` retains the effective Japanese,
Chinese, or Korean hint, and Apple fallback cascades put that language first
before device preferences and the generic CJK backstop.

The bundled policy is deliberately explicit: Noto Sans JP is the only
portable CJK asset checked into this repository, and it is eligible only for a
Japanese run. Chinese and Korean runs load only their own script lane and use
an explicitly registered face or the language-aware platform cascade; they
never silently borrow Japanese glyph forms. No proprietary Apple font is
copied into the project. Shipping portable Chinese Simplified/Traditional or
Korean binaries is an asset/licensing decision for the later portable-font
packaging work in Phase 5, rather than a hidden substitution in this phase.

Registered-family snapshots carry a registry revision and refresh lazily when
a new deterministic registration appears, including when the rasterizer was
constructed before that registration. `GlyphRasterizer::release_fallbacks`
releases discovered faces and all local cmap, metric, bitmap, and shaping
state derived from them; explicit runtime registrations survive, and a later
lookup reloads the same stable face id. `FontRegistry::replace` keeps a family
variant's deterministic face id while publishing new bytes, and
`FontRegistry::remove` removes a variant or its last family entry. Live
rasterizers compare the registry revision before family, metric, preparation,
and rasterization reads; changed or removed face ids invalidate local bitmaps,
advances, cmap and script coverage answers, platform/design metadata, shaping
state, and the shared glyph metrics table.

Fallback discovery is now per script lane rather than one broad first-miss
sweep. Each lane has a stable id range, snapshots preserve the loaded-lane
set, and release/reload keeps ids deterministic. The regression matrix covers
mixed Latin, Chinese Han, Japanese, Korean, emoji, and combining sequences;
Latin/combining stay on the primary face, language lanes do not cross-select
the Japanese bundle, and no produced glyph is `.notdef` or an empty bitmap.

### Phase 4 — Implement shaping and Unicode layout

- [x] Implement grapheme, script, language, and directional run segmentation.
      - [x] First feature-gated slice: consume UAX #9 visual level runs,
            split them at grapheme/script boundaries, preserve logical source
            ranges, and pass the resolved direction/script into shaping.
- [x] Implement the required GSUB, GPOS, and GDEF subset for the first scripts.
      - [x] Add checked GDEF class lookup, GSUB `liga`/`clig` ligatures
            (including extension lookups), and GPOS pair positioning for the
            Latin feature path; malformed tables fall back to HarfRust.
- [x] Add Latin kerning and ligatures, then Arabic and Indic shaping, then
      Southeast Asian and other complex scripts.
      - [x] First Latin slice: route ASCII runs through the Aimer shaper under
            `aimer-font`, with Google Sans glyph/cluster/advance reference
            coverage for `office` and `AV`.
      - [x] First Arabic slice: route joining-form `isol`/`init`/`medi`/`fina`
            substitutions from the `arab` script through Aimer; unsupported
            Arabic mark forms remain on the compatibility path.
      - [x] Add Arabic `rlig`/`liga` ligature substitution after joining-form
            selection, preserving the first source cluster and the ligature
            advance.
      - [x] Add checked GPOS mark-to-base, mark-to-mark (`mkmk`), and cursive
            entry/exit (`curs`) positioning, including extension-lookup
            guards; unsupported anchor formats remain on the compatibility
            path.
      - [x] Add checked Arabic chaining-context substitutions from `calt`,
            including backtrack/input/lookahead matching, format 1/2/3
            coverage/class forms, extension guards, and nested single-
            substitution records.
      - [x] Add the first owned Indic shaping slice behind `aimer-font` for
            Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu,
            Kannada, Malayalam, and Sinhala. It recognizes modern `*2` and
            legacy OpenType script tags, reorders pre-base vowel signs,
            applies the standard Indic GSUB feature order, and supports
            checked single, multiple, ligature, extension, and contextual
            substitutions.
      - [x] Add checked Indic GPOS mark-to-base/mark-to-mark attachment and
            pair kerning, including cluster inheritance for combining marks
            and the post-base offset convention used by the shared layout
            representation. Unsupported lookup or anchor formats continue to
            return to the compatibility shaper.
      - [x] Add the first owned Southeast Asian slice behind `aimer-font` for
            Thai, Lao, Khmer, and Myanmar script detection. Thai/Lao/Myanmar
            pre-base ordering, Khmer subscript-RA (“leg”) reordering, checked
            common GSUB/context/extension forms, mark-to-base/mark-to-mark
            attachment, and pair kerning now share the validated layout
            readers; unsupported lookup forms continue to return to the
            compatibility shaper.
- [x] Add bidi reordering, line breaking, cluster mapping, caret geometry, and
      selection geometry under one layout contract.
      - [x] Add the first feature-gated paragraph contract: visual-order bidi
            runs, source cluster ranges, direction-aware caret geometry,
            hit-testing, and multi-line selection rectangles now derive from
            the same positioned glyph/run/line data.
      - [x] Keep grapheme and ligature clusters indivisible for caret and
            selection operations, and fix the paragraph pen advance so visual
            glyph positions no longer collapse to one x-coordinate.
      - [x] Retain the Aimer production layout's source-aware interaction view
            beside its renderer glyphs in the layout cache, and route simple
            selectable text through the shared caret, hit-test, and selection
            geometry.
      - [x] Compose the shared interaction view across rich spans, spacing,
            transforms, custom line boxes, hard breaks, and ellipsis; retain
            the legacy one-pixel hard-break selection marker at the element
            adapter boundary.
- [x] Add CJK variation selectors, punctuation, language forms, and vertical
      substitutions where supported.
      - [x] Consume cmap format-14 Unicode variation sequences in the owned
            CJK path as one source cluster, selecting the non-default glyph
            and preserving the base advance.
      - [x] Select checked CJK `locl` language-system lookups for Chinese,
            Japanese, and Korean runs, and prefer `vrt2` over `vert` in the
            explicit vertical-substitution mode.
      - [x] Parse checked `vhea`/`vmtx` metrics and optional `VORG` origins,
            deriving non-`VORG` origins from the used glyph outline lazily and
            caching them per glyph.
      - [x] Carry top-to-bottom vertical advances and origins through the
            owned shaping/rasterizer representation, and apply checked `vkrn`
            pair adjustments after vertical metrics.
      - [x] Add the public paragraph writing-mode/request API, vertical column
            wrapping, and vertical caret, hit-test, and selection geometry.
      - [x] Apply checked `vpal` substitutions after `vrt2`/`vert` and before
            final vertical metrics, with a valid-SFNT HarfRust comparison for
            glyph IDs and advances.
- [x] Compare shaped glyph IDs and positions against the current shaper and
      reference test data before removing `harfrust`.
      - [x] Lock the first Latin glyph IDs, clusters, and positions against
            checked-in reference output; broader script comparison remains.
      - [x] Add `aimer-font-compare` Arabic shaping coverage against HarfRust
            for joining forms, ligatures, marks, mark-to-mark, cursive, and
            contextual substitutions; normalize RTL order and record the
            intentional mark-cluster and cursive-metric differences.
      - [x] Compare the owned vertical CJK feature sequence against HarfRust
            on the same checked SFNT, including `vrt2`/`vpal`, vertical
            advances, and source glyph order.
      - [x] Compare owned Indic Devanagari shaping against HarfRust using
            Google Sans for pre-base `कि` and the combining/half-form sequence
            `नमस्ते`, covering glyph IDs, clusters, advances, and offsets.
      - [x] Compare owned Thai, Lao, and Khmer shaping against HarfRust using
            Google Sans, covering glyph IDs, byte clusters, advances, offsets,
            Khmer preposed subscript-RA (“leg”) clusters, and contextual
            ligature output. Myanmar routing also has an explicit
            missing-coverage fallback regression test.

#### Phase 4 close-out — 2026-09-01

Phase 4 is complete for the bounded portable shaping and Unicode-layout
contract. The `aimer-font` path now owns grapheme/script/direction run
segmentation, checked SFNT/OpenType shaping for the supported Latin, Arabic,
Indic, Thai, Lao, Khmer, Myanmar, and CJK subsets, paragraph layout,
vertical-CJK layout, cluster-aware interaction geometry, and fallback-aware
glyph preparation. Unsupported script forms return to the compatibility
shaper at the run boundary instead of producing partial or displaced output.

The Khmer close-out regression uses the demo's 32 px showcase text,
`សួស្តីពិភពលោក`, including the U+200B separator. It locks the owned glyph
sequence, raster dimensions, bitmap bytes, and visible-pixel presence. The
same text is also compared against HarfRust through the `aimer-font-compare`
feature, including Khmer pre-base vowels and COENG/subscript legs.

The following remain intentional compatibility fallbacks and are not Phase 4
portable-support claims: full Myanmar syllable behavior beyond the bounded
GSUB/GPOS slice, script-specific language systems not selected by the current
contract, and vertical Arabic or other complex-script writing modes. They are
separate follow-up milestones and do not invalidate the completed Phase 4
contract.

Close-out verification:

```text
cargo test -p aimer_cupid --features aimer-font --lib -- --test-threads=1
cargo test -p aimer_cupid --features aimer-font-compare --lib \
  compares_southeast_asian_shaping_with_harfrust -- --nocapture --test-threads=1
cargo test -p aimer_cupid --lib -- --test-threads=1
cargo check -p aimer_cupid --bin cupid --features aimer-font
git diff --check
```

The vertical writing-mode milestone is complete for the current CJK contract.
`TextWritingMode::VerticalRl` is carried through `TextLayoutOptions`,
`TextDrawRequest`, shaping-cache keys, layout-cache keys, worker preparation,
and renderer placement. Vertical requests use `bounds_height` as the
top-to-bottom column extent and `bounds_width` as the right-to-left column
area; the width remains in the cache identity even when a request does not
wrap because it determines the rightmost column origin. Horizontal requests
retain their previous key and placement behavior.

The Aimer-owned paragraph and cached layout paths now share the same column
progression, newline handling, break-opportunity checks, glyph offsets, and
source ranges. The Aimer interaction view exposes vertical caret, hit-test,
and selection rectangles beside the renderer glyphs. ASCII vertical runs use
HarfRust's top-to-bottom direction when the owned CJK subset does not claim
them, while CJK faces require checked `vhea`/`vmtx` metrics and use `VORG` or
the bounded outline-derived origin fallback. `vpal` is optional and is only
applied when the face advertises the feature.

Regression coverage includes vertical cache identity, paragraph and cached
column wrapping, vertical interaction geometry, `vpal` ordering, vertical
metric fallback, Indic pre-base reordering, contextual GSUB formats, mark
attachment, cluster inheritance, and HarfRust comparisons. The Indic and first
Southeast Asian milestones are complete for their bounded OpenType subsets.
Full Myanmar syllable behavior, script-specific language systems beyond the
implemented feature forms, and vertical Arabic/other complex-script writing
modes remain on the compatibility shaper until their complete contracts are
defined.

### Phase 5 — Variable, color, and Apple-specific fonts

Phase 5 starts with the portable variable-font weight slice. Readable faces
that expose `fvar`/`gvar` `wght` data now receive the requested OpenType weight
directly, and that selected instance is retained in shaped glyph keys,
advance/metric cache keys, outline-cache keys, flattened-edge keys, and bitmap
cache/atlas identity. Static faces remain on the shared neutral key. Apple
platform-only faces keep their existing companion-weight adjustment because
their instance is still selected by the platform compatibility path.

- [x] Select and cache the readable `wght` variation instance for Aimer-owned
      faces, with regular/bold cache identity regression coverage.
- [x] Read checked `HVAR`/`VVAR` item-variation stores, including compressed
      delta-set maps, normalized regions, advance metrics, side bearings, and
      vertical origins for the selected `wght` instance. The rasterizer now
      applies HVAR advance/left-bearing deltas and vertical shaping applies
      VVAR advance/origin deltas.
- [x] Expose arbitrary variation-axis coordinates and include the complete
      normalized coordinate identity in every glyph, metric, outline,
      flattened-edge, bitmap, atlas, and shaped-output key. Aimer interns
      clamped F2DOT14 coordinates per shared face, keeps the zero id on the
      existing weight-only fast path, and routes non-zero instances through
      `gvar`, HVAR/VVAR, shaping, and rasterization. The public variation
      entry points are feature-gated; unsupported/invalid requests retain the
      ordinary compatibility behavior.
- [x] Implement owned `COLR` v0 layer rasterization and the default `CPAL`
      palette. Layer outlines reuse the checked TrueType/CFF path, coverage is
      composited in source-over order into straight RGBA8, and the result uses
      the existing color atlas without a platform renderer. Malformed tables,
      COLR v1, and color faces without a supported vector layer decline the
      owned path and retain the compatibility fallback.
- [x] Add owned bitmap/color glyph formats (`sbix` and `CBDT`/`CBLC`) where
      their decoder and memory policy are explicitly supported. The checked
      strike indexes and baseline placement are Aimer-owned; encoded PNG/JPEG/
      TIFF payloads use the existing bounded image codec and are decoded only
      for the requested glyph.
- [x] Add owned SVG glyph documents (`SVG `) for bounded, lazy, plain SVG
      documents containing solid filled paths. The OpenType index is checked
      without retaining XML, each requested document is parsed once into
      Aimer-owned commands, and SVG's y-down coordinates are mirrored into the
      existing y-up coverage rasterizer. Even-odd and non-zero fills are
      supported; gradients, images, text, strokes, filters, masks, animation,
      compressed SVGZ, and other unsupported effects retain compatibility
      fallback.
- [x] Define the strict portable behavior for Apple-private tables such as
      `hvgl` and `emjc`. The checked SFNT directory records direct private
      tags, and the checked `sbix` index records an `emjc` graphic type, without
      interpreting either private payload. Private-only outline/color faces
      decline the owned path and use the optional platform compatibility
      renderer; a platform without that backend receives the normal empty
      glyph with its shaped advance. Public tables remain eligible when a face
      also carries a private table.
- [x] If exact Apple system-font support is required, isolate Core Text behind
      an optional compatibility backend and keep its bitmap conversion out of
      the portable renderer. `apple-core-text` preserves the migration
      default, while `--no-default-features --features aimer-font` removes
      Apple discovery, Core Text rasterization, and their optional bindings;
      `aimer-font-core-text` opts both Aimer fonts and that compatibility path
      back in explicitly.
- [x] If no platform fallback is allowed, document unsupported Apple faces and
      ship equivalent bundled fonts instead. The portable contract rejects
      implicit Apple discovery and treats private-only `hvgl`/`emjc` faces as
      unsupported; applications must provide licensed readable replacements
      through `FontRegistration` or bundled assets.

#### Portable bundled-font policy

The no-platform profile is selected with:

```text
cargo build -p aimer_cupid --no-default-features --features aimer-font
```

On Apple, it accepts only the primary/bundled faces and application-owned
registered TTF/OTF/TTC bytes; Apple system fallback is deliberately absent. A
private Apple file is not copied into the application as a portable font:
`hvgl` outline data and `emjc` color-strike data remain opaque, and a
private-only face produces no coverage while preserving its shaped advance.
Non-Apple system fallback remains the existing migration behavior until Phase
6 replaces that resolver, so products needing cross-platform reproducibility
should use the same bundled assets on every target.

The repository currently ships these readable replacement assets:

| Asset | Portable role | Coverage policy |
| --- | --- | --- |
| `aimer_cupid/fonts/GoogleSans-Regular.ttf` | Primary Latin/common face | Default sans-serif text and metrics |
| `aimer_cupid/fonts/JetBrainsMono-Regular.ttf` | Generic monospace face | Monospace text |
| `aimer_cupid/fonts/NotoSansJP-VariableFont_wght.ttf` | Lazy CJK fallback | Japanese-oriented Han/kana fallback with `wght` instances |

These assets are not a claim that every language has a bundled equivalent.
Products requiring Korean, Myanmar, Arabic, Indic, Southeast Asian, emoji, or
other script coverage must add a licensed font with the required readable
outline/color tables and register every required weight/style variant before
rendering. The release checklist for each added asset is: validate its
TTF/OTF/TTC container, verify cmap coverage and script shaping samples, compare
weight/metrics against the product reference, and run raster golden tests at
the supported sizes and device scales. Apple system fonts are never vendored;
their licenses and private table formats make them unsuitable as portable
replacement assets.

### Phase 6 — Integrate and close the owned font path

- [x] Make `GlyphRasterizer` use the Aimer parser, shaper, resolver, and
      rasterizer in production.
      - [x] Promote the verified Aimer path to the default profiles of
            `aimer_canvas` and `aimer_text`; `--no-default-features` retains
            the feature-off compatibility test profile.
- [x] Verify that layout, drawing, clipping, baseline placement, selection, and
      accessibility consume the same metrics.
      - [x] `aimer_text` now transfers the painted paragraph's source ranges,
            line boxes, baselines, and horizontal writing mode into the Aimer
            interaction layout when `aimer-font` is enabled; the selectable
            text path consumes that shared snapshot for hit testing and caret
            geometry.
      - [x] Carry the complete local-to-physical canvas transform into that
            snapshot. Pointer hit testing, glyph hover detection, caret bounds,
            and paragraph bounds now use the inverse/forward affine mapping,
            so scaled and rotated text keeps interaction aligned with paint.
      - [x] Publish `aimer_text::TextAccessibilitySnapshot` from the same
            `TextInteractionLayout`. It retains logical UTF-8 source ranges,
            visual bidi clusters, line metrics, transformed bounds, caret
            geometry, and selection rectangles for host accessibility adapters;
            malformed ranges and singular transforms are rejected.
      - [x] Retain the shared interaction layout for rich text even when the
            element is not selectable, so accessibility does not need a second
            shaping or fallback pass.
      - [x] Map `TextAccessibilitySnapshot` into a host-owned
            `aimer_accessibility::SemanticNode`/`SemanticTree` with a stable
            caller-supplied `NodeId`, the complete accessible text, and the
            transformed paragraph bounds. Keep source-aware cluster, caret,
            and selection geometry on the snapshot instead of manufacturing
            synthetic semantic children.
- [x] Audit the legacy font engines and keep them only for explicit
      compatibility paths. `skrifa`, `harfrust`, and `swash` are still directly
      used by the feature-off compatibility backend, comparison helpers, and
      last-resort fallback code; `fontique` is still used for non-Apple system
      discovery. They are therefore intentionally retained rather than labeled
      unused. Fully owned runs do not enter those paths.
- [x] Remove migration-only dependency weight. The direct `rayon` dependency
      is gone, shader validation `naga` is dev-only, and Cupid's image decoder
      enables only PNG/JPEG/TIFF instead of the broad default codec bundle.
- [x] Keep platform FFI only when it is an explicitly selected compatibility
      backend. `apple-core-text` is optional and is absent from the portable
      `--no-default-features --features aimer-font` profile.

## Verification

- [x] Test valid and malformed TTF, OTF, TTC, CFF, variable, and color-font
      inputs through the checked parser/rasterizer fixtures and fuzz harnesses;
      unsupported table forms decline safely.
- [x] Test Latin, CJK, Arabic, Hebrew, Indic, Southeast Asian, combining-mark,
      emoji, RTL, and mixed-script paragraphs.
      - [x] Add a deterministic bundled-font matrix for Latin, Greek, Cyrillic,
            Hebrew, Devanagari, Thai, Lao, Khmer, CJK, and combining marks;
            verify source-cluster boundaries and non-empty Aimer raster output.
      - [x] Add host-specific fallback coverage for Arabic, emoji, Korean, and
            Myanmar, or register licensed readable replacement faces for those
            scripts before treating them as portable owned-font coverage.
            - [x] Add an Apple/Core Text-gated regression matrix for those four
                  host-owned samples; it verifies non-`.notdef` glyph IDs and
                  visible fallback bitmaps without promoting them to the
                  portable bundle.
- [x] Compare glyph IDs, advances, offsets, line metrics, clusters, and visual
      order against recorded reference output in
      [`FONT_RASTERIZING_BASELINE.md`](./FONT_RASTERIZING_BASELINE.md).
- [x] Add pixel-golden tests for grayscale and color glyphs at multiple sizes,
      device scales, transforms, and subpixel phases.
      - [x] Lock the owned COLR v0 compositor to deterministic RGBA pixel
            fingerprints at 16, 24, and 32 px using a checked synthetic SFNT
            fixture; the deterministic color fixture is intentionally separate
            from the host-owned Apple color-font path.
      - [x] Lock the owned grayscale path across 1×, 1.5×, and 2× physical
            sizes plus distinct x/y subpixel phases, including exact bitmap
            dimensions, bearings, and fingerprints.
      - [x] Verify a translated/scaled glyph box uses the same pixel snapping
            and clip intersection rules for partially visible and fully
            offscreen geometry.
- [x] Test `y = 0`, clipping, negative bearings, descenders, empty glyphs, and
      offscreen culling so temporary rasterization is not confused with screen
      placement.
      - [x] Cover transformed partial/offscreen clip decisions and direct
            descender/negative-bearing/empty-glyph boundary samples.
- [x] Test fallback stability when fonts are registered, unloaded, replaced,
      or discovered lazily.
- [x] Fuzz font parsing and composite glyph handling under bounded memory.
- [x] Benchmark cold rasterization, shaped-run reuse, atlas hits, atlas misses,
      large CJK pages, and worker-thread contention; retain the measurements in
      [`FONT_RASTERIZING_BASELINE.md`](./FONT_RASTERIZING_BASELINE.md).
- [x] Verify no UI-thread font parsing or bitmap generation remains on the
      measured draw/submit hot path; preparation owns the CPU font work and the
      renderer consumes prepared glyph/atlas data.

## Phase 6 exit criteria

- [x] Supported portable standard-font runs use the Aimer-owned parser, shaper,
      resolver, and rasterizer. Unsupported Apple-private payloads may use the
      optional Apple bridge, but no third-party font engine remains in Cupid.
- [x] All scripts covered by the current asset policy pass the shaping, layout,
      interaction, and image test matrix; other scripts require a registered
      licensed readable face or the selected host compatibility backend.
- [x] Cache memory and frame-time targets are recorded against the baseline;
      any compatibility fallback remains visible in the comparison profile.
- [x] The Apple system-font policy is explicit: either an optional Core Text
      compatibility path is enabled, or unsupported private fonts are replaced
      by bundled fonts.
- [x] No visible glyph displacement is attributable to mismatched baseline,
      bitmap offsets, atlas coordinates, or stale metrics in the checked
      placement, clipping, fallback, and accessibility regressions.

### Phase 6 closure record — 2026-09-02

Phase 6 is closed for the current owned-font contract. The Aimer path is the
default production path, the interaction/accessibility snapshot shares the
painted metrics, fallback lifecycle is covered, and the portable Apple-private
font policy is explicit.

The dependency cleanup is complete for the font stack. `naga` remains only as
a dev-dependency for WGSL validation; the direct `rayon` entry was removed;
and the `image` dependency no longer enables unused default codecs or its
parallel feature. The legacy font engines and their compatibility/comparison
paths (`skrifa`, `harfrust`, `fontique`, and `swash`) were removed. The only
remaining optional font backend is `apple-core-text`, which is limited to
Apple discovery and private-glyph raster bridging around the Aimer-owned
parser, shaper, and rasterizer.

## Postscript — remove the `aimer-font` feature and legacy engines — 2026-09-02

This postscript supersedes the earlier Phase 5/6 wording that described
feature-off compatibility engines. The migration is now complete:

- [x] Remove `aimer-font`, `aimer-font-compare`, and
      `aimer-font-core-text`; Aimer parsing, shaping, fallback resolution, and
      rasterization are unconditional in Cupid.
- [x] Remove the legacy `skrifa`, `harfrust`, `fontique`, and `swash`
      dependencies and their compatibility/comparison code.
- [x] Keep `apple-core-text` as the only optional font feature. It is limited
      to Apple font discovery and the private-glyph bridge; it does not select
      a third-party text engine.
- [x] Keep `--no-default-features` as the portable Aimer profile. It disables
      the Apple bridge while retaining the same owned font implementation.
- [x] Record the last release comparison and the final Aimer-only timings in
      [`FONT_RASTERIZING_BASELINE.md`](./FONT_RASTERIZING_BASELINE.md).

The default Apple Cupid suite passes (`511 passed; 5 ignored`) and the
portable Aimer suite passes. The final source and manifest sweep reports no
removed feature or legacy engine names outside the historical plan records.
