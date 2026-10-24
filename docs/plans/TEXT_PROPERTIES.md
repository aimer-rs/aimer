# Text properties plan

This plan covers the text properties that are not yet available through the
public `Text`/`RichText` interfaces:

- `text-transform`
- `line-height`
- `letter-spacing`
- `word-spacing`
- `text-indent`
- `text-shadow`

The implementation must keep one canonical value for each property. `Text`
may expose ergonomic forwarding methods, but it should not duplicate the same
state in both `Text` and `TextStyle`.

## Current state

The current text model is split between a run style and paragraph settings:

| Property | Current location | Status |
| --- | --- | --- |
| Font family, size, style, weight | `TextStyle` | Implemented |
| Color and background color | `TextStyle` | Implemented |
| Text decoration | `TextStyle` / `SpanStyle` | Implemented; keep it there |
| Text alignment | `Text` / `RichText` | Implemented |
| Text overflow | `TextStyle` and `RichText` override | Implemented |
| `text-transform` | `TextStyle` / `SpanStyle` | Implemented before paragraph shaping with source-range preservation |
| `line-height` | `Text` / `RichText` / `Paragraph` | Implemented as `Normal`, `Px`, or `Factor` |
| `letter-spacing` | `TextStyle` / `SpanStyle` | Implemented in measured and painted grapheme advances |
| `word-spacing` | `TextStyle` / `SpanStyle` | Implemented at whitespace graphemes in layout and painting |
| `text-indent` | `Text` / `RichText` / `Paragraph` | Implemented for the first line; negative values are hanging indents |
| `text-shadow` | `TextStyle` / `SpanStyle` and native glyph pipeline | Implemented as paint-only glyph shadow data |

Relevant existing seams are [`TextStyle`](../crates/aimer_style/src/style/text_style.rs),
[`SpanStyle`](../crates/aimer_text/src/text_span.rs),
[`Text`](../crates/aimer_text/src/text.rs), and
[`Paragraph`](../crates/aimer_text/src/paragraph.rs).

## Text source ownership

`TextSource` accepts a static string, `String`, `Rc<str>`, `ShareRef<str>`, or
`ShareRef<String>`. The `ShareRef` variants retain shared ownership without
copying the selected bytes; `to_rc()` remains the explicit allocation boundary
for the current selection API.

### Decision: canonical shared text handle

Use a text-facing `ShareRef<T: ?Sized>` as the canonical shared-source
interface. `TextSource` stores `ShareRef<str>` by value:

```rust
enum TextSource {
    Static(&'static str),
    Shared(Rc<str>),
    ShareRef(ShareRef<str>),
}
```

The existing generic `SharedRef<Owner, Field, Select>` remains the lower-level
owning projection mechanism. Adapters from `Shared::project()` and
`SharedRef::from_rc()` produce the text-facing `ShareRef<str>`; arbitrary
`SharedRef<Owner, Field, Select>` values are not stored directly in the
non-generic `TextSource` enum.

`ShareRef<String>` is a constructor-level convenience only. It must be
converted to `ShareRef<str>` through a no-copy projection, so `TextSource` has
one canonical string target and does not need separate `String` and `str`
variants. Cloning a `ShareRef<str>` clones its ownership handle, never the
string contents.

The current `TextSource::to_rc()` method remains an explicit boundary
conversion. It may allocate when a projected `ShareRef<str>` must be handed to
an API that requires `Rc<str>`. Eliminating that allocation is a later
selection-system change: selection storage must accept and retain the shared
text handle directly.

Implementation invariants to verify:

- Does `ShareRef<String>` convert to a `ShareRef<str>` view without cloning?
- Can `TextSource` retain the `ShareRef` directly, or does it need a dedicated
  shared-source variant?
- Does `to_rc()` clone the shared storage or allocate only at the selection
  boundary, as the current static-string path does?
- Do `ShareRef` values guarantee immutable string data for the lifetime of the
  retained handle?

Implementation checklist:

- [x] Locate or define the public `ShareRef<T>` type and document its `Clone`,
      `Deref`, `AsRef`, and unsized-string behavior.
- [x] Add `From<ShareRef<str>> for TextSource`.
- [x] Add `From<ShareRef<String>> for TextSource`, preserving the shared handle
      rather than converting through an avoidable allocation.
- [x] Extend `TextSource::as_str`, `to_rc`, `Display`, `Debug`, equality, and
      cloning to cover both shared-reference forms.
- [x] Keep portable encoding content-based: encode the string bytes, never the
      process-local sharing identity.
- [x] Test the `TextSource` conversion seam used by `Text::new` and `Text::text`,
      clone behavior, content equality, lifetime/ownership guarantees, and
      `to_rc()` behavior.
- [x] Update the `TextSource` documentation example and public re-exports.

## Design direction

### Run-level style

Keep properties that affect glyph runs in `TextStyle`, with optional overrides
in `SpanStyle`:

- `text_transform`
- `letter_spacing`
- `word_spacing`
- `text_decoration`
- `text_shadow`

`TextDecoration` remains part of `TextStyle`. `RichText` must be able to apply
different decoration, spacing, transformation, and shadow values to different
spans.

Use the following initial defaults and units:

- `TextTransform::None`.
- Letter and word spacing of `0.0` logical pixels.
- No text shadow.
- Finite numeric values only; reject NaN and infinity in portable lowering.

Start with one optional `TextShadow` value so `TextStyle` can retain its cheap
copy semantics. If multiple CSS-style shadows become a requirement, evaluate
an immutable shared slice before introducing an allocating `Vec` into every
style value.

### Paragraph-level layout

Keep properties that affect the whole text block at the paragraph seam:

- `line_height`
- `text_indent`
- existing `text_align`
- existing `text_overflow`

The initial implementation may store these directly on `Text` and `RichText`
to limit migration scope. If both widgets need the same behavior, introduce a
small paragraph-layout value rather than duplicating layout rules.

`line-height` must have explicit semantics before it becomes public. Prefer
distinct forms such as `Normal`, an absolute logical-pixel value, and a font
size factor rather than an undocumented `Value(f32)` unit.

`text-indent` applies to the first line of a paragraph, not independently to
each `TextSpan`. Define the behavior of negative values before implementation;
supporting them as hanging indents is preferable if the layout math remains
clear.

### Public `Text` interface

Add forwarding methods only after the underlying model exists. The expected
ergonomic surface is:

```rust
Text::new("Aimer")
    .text_transform(TextTransform::Uppercase)
    .letter_spacing(0.5)
    .word_spacing(2.0)
    .line_height(LineHeight::Normal)
    .text_indent(12.0)
    .text_shadow(TextShadow::new())
```

Use descriptive names such as `text_shadow` and `text_transform`; plain
`shadow` or `transform` is too ambiguous in a widget framework. Each method
must update the canonical style or paragraph value rather than add a second
field.

`RichText` needs equivalent base-style and paragraph methods. `SpanStyle`
needs run-level overrides where the property has meaningful per-span
semantics.

## Implementation phases

### Phase 1 — Define the value model

- [x] Add `TextTransform` to `aimer_style` with documented Unicode behavior.
- [x] Add `TextShadow` with offset, blur, and color fields.
- [x] Add letter and word spacing to `TextStyle` and `SpanStyle`.
- [x] Add text transformation and shadow to `TextStyle` and `SpanStyle`.
- [x] Define the public line-height representation and units.
- [x] Add paragraph-level line-height and text-indent values.
- [x] Document defaults, finite-value validation, negative-value behavior, and
      whether each property inherits into child spans.

### Phase 2 — Thread properties through shaping and layout

- [x] Thread transformed text and spacing-adjusted advances through paragraph
      layout. Transform changes the text handed to the existing shaping/measure
      cache; spacing is an inter-grapheme paragraph advance and therefore does
      not duplicate a shaped-text cache entry.
- [x] Apply `text-transform` before shaping while retaining source-to-rendered
      ranges for selection, hit-testing, and links.
- [x] Apply letter spacing to glyph/run advances and word spacing only at the
      defined word boundaries.
- [x] Make wrapping, measured widths, ellipsis, grapheme geometry, selection
      regions, and decoration widths use the same adjusted layout.
- [x] Apply line-height when computing baselines, line boxes, paragraph height,
      and the final line's geometry.
- [x] Apply text-indent to the first line before wrapping and alignment offsets.
- [x] Ensure `Text`, `RichText`, and selectable text use the same paragraph
      calculations.

Spacing must not be applied only during painting: doing so would make measured
widths, wrapping, caret positions, and drawn glyphs disagree.

### Phase 3 — Add glyph shadows

- [x] Extend the text draw request and text pipeline with shadow information.
- [x] Paint each shadow before the normal glyphs, respecting transforms, clips,
      opacity, and visible-range culling.
- [x] Decide whether blur is implemented by the existing GPU text path or by a
      dedicated glyph-shadow pipeline; do not reuse rectangular `BoxShadow`
      rendering for glyph outlines.
- [x] Keep paint-only shadow data out of layout cache keys while carrying it in
      the draw request; the existing glyph atlas and shaping caches remain
      reusable because shadow does not change glyph geometry.

### Phase 4 — Portable schema and compatibility

- [x] Extend the versioned `TextStyle` value codec for the new run-level
      fields.
- [x] Preserve decoding of the existing style format and write a new version
      when the wire payload changes.
- [x] Update the `RichText` portable span-style codec and its bounded limits.
- [x] Add portable paragraph properties for line-height and text-indent if they
      are not represented by the existing `Text`/`RichText` properties.
- [x] Update host materialization, validation, encoded-size limits, and schema
      documentation.
- [x] Keep unsupported custom values rejected explicitly rather than silently
      falling back.

### Phase 5 — Ergonomic builders and documentation

- [x] Add `Text` forwarding builders for all supported new properties.
- [x] Add equivalent `RichText` and `SpanStyle` builders where appropriate.
- [x] Keep the existing `TextStyle` and `TextDecoration` APIs source-compatible
      unless a migration is explicitly planned.
- [x] Add examples showing simple text styling and per-span rich-text styling.

## Verification

- [x] Test defaults and builder composition for every new value.
- [x] Test Unicode transformations, including casing that changes the number
      of source characters, without breaking selection or links.
- [x] Test letter and word spacing for empty text, whitespace, combining marks,
      wrapping, and mixed spans.
- [x] Test normal, absolute, and factor-based line heights for one-line and
      multi-line paragraphs.
- [x] Test positive and negative text indentation and alignment interaction.
- [x] Test shadow ordering, clipping, opacity, transforms, and empty/transparent
      shadows.
- [x] Add portable encode/decode round trips, malformed payload tests, version
      compatibility tests, and encoded-size limit tests.
- [x] Run focused `aimer_style`, `aimer_text`, and portable-schema tests before
      the full workspace suite.

# Phase 6 — Add an All-Properties Example in the `jaime` Crate

The example is a visual regression fixture for the six properties in this
plan. It must use only the public `aimer`/`aimer_style`/`aimer_text` APIs and
must remain useful when a property is changed later. The example is not a
second implementation of text layout and must not reach into raw paragraph,
shaping, or renderer types.

## Example entry point

- [x] Add `jaime/src/text_properties_example.rs` with a
      `start_text_properties_example()` function following the existing Jaime
      showcase modules.
- [x] Register the module and start function in `jaime/src/main.rs`. Keep the
      current default demo unchanged; make the text-properties showcase
      launchable by switching the same single entry-point call used by the
      other examples.
- [x] Build the page as a vertically scrollable, constrained showcase so
      wrapping, line height, indentation, and shadows remain visible on small
      windows as well as wide desktop windows.
- [x] Use a consistent card/label/sample layout and short explanations of the
      units and semantics. Do not add a new dependency or require a new image
      asset for the showcase.
- [x] Keep the sample data deterministic and include ordinary Latin text,
      whitespace, an explicit newline, combining marks, and non-Latin text.
      Include at least one case whose transformed display can have a different
      number of rendered characters than its source.

## Sample matrix

Use one baseline sample with the default style, then make each property’s
effect easy to compare against that baseline. The exact enum variants and
builder names must follow the public value model finalized in Phases 1 and 5.

| Property | Showcase cases | What the example must make visible |
| --- | --- | --- |
| `text-transform` | Default plus every supported transform variant, using source text that includes mixed case and Unicode | The source string stays identifiable, the rendered casing changes, and a length-changing Unicode case does not corrupt wrapping or span boundaries |
| `line-height` | `Normal`, an absolute logical-pixel value, and a font-size factor on the same multi-line text | Baseline distance, paragraph height, and the final line’s placement change without changing glyph size |
| `letter-spacing` | `0.0`, a positive value, and a negative finite value on a long sample with punctuation and combining marks | Glyph/run advances, wrapping, and the measured width change together; spacing is not painted as a post-layout offset |
| `word-spacing` | `0.0`, a positive value, and a negative finite value on a sentence with repeated spaces and punctuation | Word gaps change while intra-word letter spacing does not, and wrap points move consistently with the displayed text |
| `text-indent` | `0.0`, a positive first-line indent, and a negative/hanging indent on a constrained multi-line paragraph | Only the first line is affected, subsequent lines retain the paragraph alignment, and the paragraph’s measured bounds remain correct |
| `text-shadow` | No shadow plus a clearly visible offset/blur/color shadow on a contrasting surface | The glyph shadow is painted before the glyphs, follows the text’s opacity/clip/transform, and is visibly distinct from a rectangular `BoxShadow` |

- [x] Add an existing-property baseline for font, color, alignment, overflow,
      and decoration so the new values can be compared without implying that
      those properties are part of the new implementation scope.
- [x] Show paragraph-level values on `Text` and on `RichText`, including an
      alignment comparison for the indented paragraph.
- [x] Show run-level values on `TextStyle` and at least two contrasting
      `TextSpan` styles. The rich-text sample must demonstrate that spans can
      override transformation, spacing, decoration, and shadow independently
      while inheriting the base style for values they do not set.
- [x] Include one selectable rich-text sample containing a transformed span
      and a link. The layout tests verify source-based ranges; the visual
      selection/link check remains in the manual section below.
- [x] Keep each sample label next to the value being demonstrated; do not rely
      on color alone to communicate a difference.

## Manual acceptance checks

The code and build checks are complete. These visual checks remain pending
until the Jaime showcase is launched in an interactive window.

- [ ] Launch the showcase from the `jaime` crate and inspect it at a wide
      window, a narrow window, and a window shorter than the full page.
- [ ] Confirm all six properties have a visibly different non-default case and
      that the page remains scrollable without clipped cards or overlapping
      labels.
- [ ] Confirm transformed, spaced, indented, and shadowed text has the same
      layout/paint result when it wraps and when it contains an explicit
      newline.
- [ ] Confirm the rich-text sample preserves per-span color/decoration and
      link interaction while the new properties are active.
- [ ] Confirm the shadow sample has no shadow when the value is absent or
      transparent, and that the shadow does not enlarge layout merely because
      it is painted outside the glyphs.
- [ ] Check the sample on every platform that the `jaime` showcase is intended
      to support; record any renderer-specific limitation instead of hiding it
      with a platform-specific fallback.

## Build and maintenance checks

- [x] Run `cargo check -p jaime` after wiring the example and run the focused
      text/style tests before the full workspace suite.
- [x] Keep helper functions small and data-driven so adding another supported
      transform or line-height form requires adding a case rather than copying
      an entire widget tree.
- [x] Add comments only for the source-to-rendered Unicode example and other
      non-obvious manual-test intent; keep ordinary layout code self-explanatory.
- [x] Update this plan’s checklist if a public property is intentionally
      deferred or if a platform cannot render a case, including the reason and
      the replacement verification.

The phase is complete when the showcase is checked into `jaime`, can be
launched through the existing example switch, visibly covers every supported
text property, exercises both `Text` and `RichText`, and passes the documented
build and manual checks.



## Non-goals

- [x] Do not move `TextDecoration` out of `TextStyle`.
- [x] Do not add independent duplicate fields to `Text` merely to make the
      builder look flat.
- [x] Do not implement spacing by changing only the final draw positions.
- [x] Do not treat box shadows as text shadows.
- [x] Do not change text input behavior until the shared shaping/layout contract
      is defined; editable fields need their own caret and composition review.
