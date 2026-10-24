# Aimer framework `PaintIsolated` and damage-region repaint plan

> Status: implementation in progress. Phases 0-3 are complete. Phase 4 now
> includes the persistent target and a conservative single-region repaint
> path; the owned frame packet, complete capability coverage, diagnostics,
> generic framework correctness suite, and Jaime acceptance remain.
>
> Scope: Aimer framework. Jaime is the first reproducible workload and acceptance fixture, not the owner of the fix.
> Native desktop is the first target; the existing full-frame path remains the correctness fallback.
>
> Phase 0 record: [`PHASE0_BASELINE.md`](PHASE0_BASELINE.md). The record is
> intentionally preliminary; it documents the first debug/release reproduction
> and the remaining measurement gaps before implementation acceptance.

This plan addresses a framework rendering problem discovered in Jaime: an offset-only interaction can cause an
unchanged visual subtree to be visited, re-recorded, and repainted during a new frame. The solution has two connected
framework Modules:

- `PaintIsolated` retains a subtree's paint recording and avoids repeating child paint work.
- `DamageRegion` repaint retains the initialized render target, repaints only damaged pixels, and composites retained
  layers into the result.

Jaime supplies the first regression case and real performance measurement; it must not receive a one-off compositor
implementation.

## Decision

The current evidence does not prove that Aimer rebuilds the entire widget tree on every scroll. The retained element tree
already makes rebuilding dirty-scoped, and Jaime's sidebar offset is held in `ScrollState` rather than selection state.

The expensive path is more likely:

1. An interaction tick requests another frame.
2. `FrameDrawer` walks and draws the retained root for that frame.
3. `CupidCanvas::begin_frame` clears the frame command list and the main target is cleared.
4. An unchanged visual subtree is visited, encoded, and rasterized again.

The framework fix is therefore:

- Introduce a reusable internal `PaintIsolated` Module at the widget/canvas Seam.
- Introduce a reusable `DamageRegion`/`DamageRenderer` Module at the canvas/renderer Seam.
- Let widget developers provide a conservative paint-retention contract through `Drawable`.
- Let framework-owned Modules choose natural visual seams and install `PaintIsolated` internally.
- Record a child once and replay its retained layer while only an independent sibling, offset, or transform changes.
- Preserve a valid render target between frames and repaint only the normalized damaged regions.
- Invalidate or fall back to normal drawing whenever paint is dynamic, unsafe, or unknown.
- Keep layout, hit testing, focus, accessibility, and lifecycle processing independent from cached paint.

Jaime is the first adopter because its sidebar and content pane make the independent-subtree case observable. The
framework Modules must also work for other containers, scroll views, overlays, drawers, static panels, and future
rendering clients.

## Ownership and public Interface

The retention mechanism belongs to Aimer and widget developers, not to ordinary widget users:

- Widget developers implement the internal `Drawable` paint contract; its conservative default remains `false`.
- Framework-owned Modules such as `Scrollable`, `Drawer`, `Overlay`, or a framework `ContentPane` choose the visual
  Seams that may retain paint.
- The `PaintIsolated` Implementation owns cache keys, invalidation, layer lifetime, resource tracking, and fallback.
- The `DamageRegion`/`DamageRenderer` Implementation owns damaged-region normalization, persistent-target validity,
  partial clear/load ordering, and full-frame promotion.
- Ordinary widget-user composition remains unchanged. Users write normal `Scrollable`, `Row`, `Column`, and content
  widgets without adding `PaintIsolated`, `Cached`, `DamageRegion`, dirty rectangles, or renderer options.
- A future semantic user hint is optional and must not expose GPU/layer mechanics; it is not part of this implementation.

This gives callers a small Interface and gives the framework a deep Module with high Leverage: one implementation can
serve many framework-owned Modules and tests while keeping cache complexity local.

## Evidence to preserve

Relevant implementation and existing measurements:

- [`aimer_quiver/src/handler.rs`](../aimer_quiver/src/handler.rs): the retained root is reused, but `root.draw` is
  called for every requested frame.
- [`aimer_cupid/src/canvas.rs`](../aimer_cupid/src/canvas.rs): frame drawing starts from a cleared command list and
  supports recording and retained-layer replay; the target lifecycle still needs persistent damage-aware handling.
- [`crates/aimer_widget/src/components/drawable.rs`](../crates/aimer_widget/src/components/drawable.rs):
  `Drawable::is_paint_stable` defaults to `false` and forbids hidden paint side effects.
- [`crates/aimer_scroll/src/scrollable/core/raw_scroll.rs`](../crates/aimer_scroll/src/scrollable/core/raw_scroll.rs):
  A scroll-specific retained-paint implementation already records snapshots, replays layers, and falls back for
  unstable children.
- [`crates/aimer_widget/src/paint_isolated.rs`](../crates/aimer_widget/src/paint_isolated.rs): the initial shared
  framework seam records/replays stable full layers and falls back conservatively.
- [`aimer_cupid/src/damage_region.rs`](../aimer_cupid/src/damage_region.rs): the backend-independent damage model
  normalizes device-pixel bounds, coalesces regions, and promotes conservatively.
- [`crates/aimer_container/src/single_child/container.rs`](../crates/aimer_container/src/single_child/container.rs):
  `RawContainer` currently updates hit-test bounds during `draw`, so it cannot be marked stable blindly.
- [`jaime/src/showcase.rs`](../jaime/src/showcase.rs): Jaime provides the first sibling-pane regression workload;
  sidebar scrolling and right-side content are independent visual regions.
- [`FW_OPTIMIZE.md`](FW_OPTIMIZE.md): existing retention and culling are useful foundations, but production content
  has not consistently qualified for retained paint.
- [`PERFORMANCE_REPORT.md`](PERFORMANCE_REPORT.md): identifies paint stability and hit-test-bound bookkeeping as the
  important safety seam.

### Interpretation of the reverted widget-rebuild attempt

The reverted rebuild-locality change had no visible effect because it addressed a secondary cost. Even when the widget
tree is clean, the current frame pipeline can still traverse and repaint the whole visual surface. Measurements must
separate build, layout, paint, command encoding, GPU work, and frame count instead of calling all of them “rebuild”.

## Target behavior

```text
Initial frame or target invalidation:
  build/layout -> PaintIsolated records layers -> full target repaint -> present

Independent sibling/offset-only frame:
  update live subtree -> collect damage -> partial clear/load -> repaint damaged layers -> present

Valid PaintIsolated child inside an undamaged region:
  skip child paint recording -> keep its retained layer -> composite it when required

Child content/style/resource/geometry change:
  invalidate PaintIsolated -> union old/new bounds and dependencies -> record -> repaint damage

Unsafe, unknown, resized, or lost target:
  promote to full direct repaint
```

The root element may still be traversed. `PaintIsolated` prevents unnecessary child recording; `DamageRegion` prevents
unnecessary target repaint. The success condition is that an independent offset-only update does not rebuild or record an
unchanged child and does not rasterize unchanged target pixels. Eliminating the root walk is a later optimization.

## Goals

- Make independent offset-only, transform-only, and sibling-interaction frames cheaper across Aimer.
- Stop normal child paint and command recording when `PaintIsolated` has a valid retained layer.
- Preserve a valid render target and repaint only the `DamageRegion` produced by changed layers, bounds, clips, and
  effects.
- Preserve exact pixels, z-order, clipping, hit testing, focus, accessibility, and input behavior.
- Generalize the existing scroll retained-paint logic instead of maintaining parallel cache implementations.
- Make dynamic content and unsupported paint conservatively fall back to the direct path.
- Provide framework-level counters and tests that prove when isolation is useful.
- Keep Jaime sidebar-only scrolling below 20% average process CPU in the defined native release/profile workload.
- Keep native optimization opt-in until framework and Jaime acceptance passes.

## Framework-level definition of done

This work is complete only when the behavior is provided by reusable Aimer Modules and does not depend on Jaime types,
layout assumptions, or a Jaime-specific renderer branch. Jaime is an acceptance fixture, not the implementation seam.

- [ ] The framework owns an immutable frame packet that carries target validity and generations, device geometry,
  damage regions, ordered layer metadata, retained-layer references, and resource lifetimes through raster submission.
- [ ] Every visual invalidation that can change a target pixel records enough old/new geometry and dependency data to
  derive conservative damage. Unknown provenance promotes to a full repaint.
- [ ] Partial repaint loads a persistent target, clears only damaged device pixels, and replays every intersecting
  dependency in paint order, including unchanged layers behind the changed content.
- [ ] The renderer can process multiple regions and all supported paint paths, or explicitly promotes unsupported paths
  before touching the persistent target.
- [ ] Unaffected content is isolated on both sides of the pipeline: it is not rebuilt or re-recorded unnecessarily on
  the framework path, and its target pixels are not rasterized again.
- [ ] Generic framework tests compare partial output with an equivalent full repaint across geometry, ordering,
  resources, effects, target lifecycle, and interaction-related invalidation.
- [ ] Framework diagnostics expose why a frame was partial, skipped, or promoted, and release-profile measurements
  separate build, layout, paint, encode, GPU, and present costs.
- [ ] Jaime demonstrates the framework behavior with an unchanged content pane during sidebar scrolling, including the
  agreed below-20%-CPU workload target.

The first implementation is allowed to replay more commands than strictly necessary inside a damage region, but it is
not allowed to claim full completion while the frame packet, invalidation coverage, target lifecycle, or fallback
reasoning is implicit.

## Non-goals

- Do not claim that every frame rebuilds the entire widget tree without counters.
- Do not change selection, event routing, focus, or accessibility semantics.
- Do not add public compositor parameters to ordinary widgets in the first iteration.
- Do not expose `PaintIsolated`, cache keys, retained layers, damage rectangles, or renderer policy to ordinary widget
  users in the first iteration.
- Do not blindly mark `RawContainer` or arbitrary custom widgets paint-stable.
- Do not introduce a spatial `SceneIndex`, complex effect expansion, or other global indexing before the basic
  `DamageRegion` repaint path is correct and measured.
- Do not use partial repaint without explicit persistent-target validity and a full-repaint fallback.
- Do not remove existing full-frame, dynamic-island, or scroll-retained fallbacks.
- Do not make wasm or unsupported renderers depend on partial presentation.
- Do not optimize frame count by dropping frames that contain visible scroll or animation progress.

## Framework Module and Seam

### Module: `PaintIsolated`

`PaintIsolated` is an internal deep Module with one narrow Interface. It owns the decision between normal child paint and
retained paint:

- Input: a retained child element, its layout bounds, clip, transform, and invalidation generation.
- Retained state: cached layer or tiles, device scale, content bounds, resource generations, and validity state.
- Layout Interface: preserve the child element and delegate geometry work.
- Interaction Interface: preserve hit testing, focus, accessibility, and lifecycle behavior.
- Paint Interface: record the child when invalid; otherwise replay the retained layer.
- Fallback Interface: report the reason retention is unavailable and use normal child drawing.

The external widget Interface does not expose GPU textures, damage rectangles, or renderer-specific details. A renderer
Adapter below the widget/canvas Seam owns the retained surface and frame-packet lifetime.

### `PaintIsolated` lifecycle

```text
cache missing or invalid:
  record child paint -> validate snapshot -> store retained layer -> draw layer

cache valid and only an independent region changed:
  skip child paint recording -> replay retained layer

child dynamic, unsafe, or invalidation unknown:
  bypass cache -> use normal child.draw path
```

`PaintIsolated` is a paint policy owned by the framework/widget developer, not a promise made by the widget user. The
Module must not depend on user-supplied dirty rectangles or renderer-specific cache handles.

### Widget-developer contract

Keep [`Drawable::is_paint_stable`](../crates/aimer_widget/src/components/drawable.rs) as the internal capability:

- [ ] `true` means paint can be recorded once and replayed under a different transform without observable side effects.
- [ ] `false` remains the safe default for custom and dynamic elements.
- [ ] A stable result must not update event/hit-test geometry, advance animation/input state, start async work, or depend
  on the current viewport/cursor.
- [ ] Structural, style, text, image, scale, resource, and geometry changes remain invalidation responsibilities of the
  owning `PaintIsolated` Module.
- [ ] Wrappers forward the contract only when every relevant child and effect satisfies it.

### Module: `DamageRegion` repaint

Add a second internal deep Module named `DamageRegion`/`DamageRenderer` at the canvas/renderer Seam. It operates on a
persistent initialized target, not on a newly cleared empty target:

- Damage set: finite device-pixel, half-open rectangles with normalization, union, intersection, clipping, coalescing,
  and full-frame promotion thresholds.
- Damage tracker: consumes invalidation provenance from `PaintIsolated`, old/new layer bounds, clips, transforms,
  ordering changes, resource changes, and effect expansion.
- Persistent target: owns the initialized color target/view, validity state, size/scale key, and bounded lifetime.
- Owned frame packet: freezes target identity, device geometry, damage, ordered layer dependencies, retained-layer
  references, and resource lifetimes for the raster thread.
- Region renderer: for each damaged region, clears only that region, reloads preserved pixels, and replays every ordered
  layer that can affect it, including unchanged background layers behind a changed foreground.
- Presentation Adapter: composites/presents the target and promotes to the existing full-frame path when the target or
  dependency state is invalid.

The external widget Interface does not expose damage rectangles. Widgets report state/paint invalidation through normal
framework mechanisms; `DamageRegion` derives screen damage from retained layer metadata.

### Relationship between the Modules

```text
Widget invalidation
    -> PaintIsolated records/reuses child layer
    -> DamageTracker derives old/new damaged regions
    -> DamageRenderer partially clears and repaints persistent target
    -> compositor presents
```

`PaintIsolated` answers “which child paint must be recorded again?” `DamageRegion` answers “which target pixels must be
repainted?” Either Module may fall back independently, but a partial target repaint is never allowed to depend on a
missing or uninitialized layer.

## Framework integration model

The framework, not each application, selects the first isolation Seams:

```text
framework-owned visual Module
├── live child/region
└── PaintIsolated(child)
    ├── retained layer/tile cache
    └── direct fallback

frame invalidations/bounds
    -> DamageRegion tracker
    -> persistent target + ordered region repaint
    -> full-frame fallback when invalid
```

Initial adopters:

- `Scrollable`: migrate its current retained-paint cache to the shared `PaintIsolated` Implementation while preserving
  culling, retained tiles, dynamic islands, and scroll-specific transforms.
- Independent sibling regions: allow framework-owned layout/pane Modules to place `PaintIsolated` around a child whose
  bounds and paint contract are independently invalidated.
- `DamageRenderer`: consume layer invalidations and old/new bounds, preserve the target between frames, and repaint only
  damaged regions while replaying all intersecting layers in paint order.
- Other built-in Modules: add one at a time only where profiling shows parent and child repaint at different times.

Do not automatically create a retained layer for every stable node or a damage region for every event. Layer memory,
recording cost, ordering, effects, target validity, and dynamic invalidation must be considered by the framework placement
policy.

## Phase 0 — framework baseline and Jaime reproduction

- [x] Add debug-only counters for retained-tree rebuild visits/prunes and
  stateful/stateless build callbacks.
- [x] Add framework-level `PaintIsolated` counters for candidate, record,
  replay, invalidation, fallback, and retained-tile work. The counters are
  emitted by the existing scroll retention seam, so ordinary widget users do
  not configure them.
- [x] Add a full-frame damage baseline: full-frame clears and pixels are
  recorded, while partial frames, damage regions, merges, target reuse, and
  promotions remain zero until the persistent-target `DamageRenderer` is
  integrated.
- [x] Add framework debug-only counters/signposts for scroll and offset events,
  state updates, smoothing steps, redraw requests, delivered frame wakes,
  retained-element layout and paint calls, routed hit-test visits, and
  `root.draw` calls. The counters are sampled into the existing 30-frame
  report; they are measurement-only and do not change rendering behavior.
- [x] Keep command recording, retained-layer command, text-cache, image-draw,
  and build/encode/present phase counters in the same frame report.

## Phase 1 — repair the paint contract

`RawContainer` currently records hit-test bounds during `draw`. Changing its stability answer without moving that side
effect would make cached paint stale for interaction geometry.

- [x] Add the internal `Drawable::paint` and `Drawable::sync_paint_geometry` seams. Retained recording now uses
  `paint`, while normal `draw` remains the lifecycle/rebuild entry point.
- [x] Separate `RawContainer` hit-test bounds and child geometry synchronization from its visual paint path.
  `RawContainer` remains conservatively non-stable until a complete adopter-specific proof exists.
- [x] Keep `RawFlex` non-stable because its normal draw maintains layout/window/reconciliation and hit-test state. Its
  stable paint-island callback now synchronizes geometry live and records through `paint`.
- [x] Add focused coverage for stable replay, unstable fallback, invalidated child, changed clip/transform/resource
  owner keys, and replay-time transform geometry.
- [x] Add regression coverage proving `RawContainer`, `RawFlex`, and effect surfaces do not become stable accidentally.
- [x] Verify built-in stable wrappers (`Opacity`, `Scalable`, `AspectRatio`, `Expanded`, `CustomShape`, stateless and
  retained-child wrappers) forward paint-only work; backdrop/effect surfaces (`Glass`, `Liquid`) reject retention.
- [x] Audit current production stability opt-ins: immutable text/no-op primitives are the only leaf opt-ins; image, SVG,
  custom-pipeline, and effect surfaces remain conservative, while retained-command validation rejects upload, rich-text,
  and custom payloads.
- [x] Add a GPU readback oracle comparing direct and retained primitive output across two scroll-like translations and
  an active clip (`tests/paint_isolated.rs`).
- [x] Add a GPU readback oracle for a texture-backed image after an explicit upload, including translated clipped
  composition (`tests/paint_isolated.rs`).
- [x] Add retained-scroll regressions for same-ID image replacement and dirty stable-content re-recording.
- [x] Expand the pixel oracle to SVG/font-backed and animation-transition cases before enabling additional framework
  isolation seams. The oracle also caught and fixed the retained-layer premultiplied-alpha image path: renderer-owned
  layers now identify their premultiplied source encoding instead of being decoded as straight-alpha images.

Do not annotate a whole application subtree as stable. `PaintIsolated` must earn retention from the widget-developer
contract or use a separately verified paint-only Adapter.

## Phase 2 — extract the shared `PaintIsolated` Implementation

- [x] Add the initial framework-level Module and move stable full-layer recording/replay behind its small Interface. The
      dynamic-island and retained-tile paths now use the shared cache-validity Module while retaining their distinct
      recording and placement policies.
- [x] Extract the shared cache key/validity machinery from `aimer_scroll` without duplicating the tile/dynamic policy.
      `aimer_widget::PaintCache` now owns recorded keys, tracked element identities, invalidation-epoch checks, and
      cache retirement; Scrollable supplies only the policy-specific contract comparisons.
- [x] Reuse `fork_for_recording`, `retained_snapshot`, `draw_retained_layer`, and existing retained-layer memory limits.
- [x] Record the stable child into a renderer-owned layer on first paint or after invalidation.
- [x] Replay a valid layer without invoking normal child paint or command recording.
- [x] Keep the child element available for rebuild, layout, and interaction processing.
- [x] Track every content/rebuild generation, layout/bounds, device scale, clip, transform, and resource generation through the shared `PaintContract`; `Scrollable` supplies the complete contract while its live scroll translation remains composition-only, and element paint-invalidation tracking remains the conservative fallback.
- [x] Keep the retained layer alive through the current frame command ownership model.
- [x] Bound memory; cache absence or failed recording falls back to direct paint.
- [x] Keep the direct path available through the conservative stability/size checks.
- [x] Record isolation record/replay/invalidation/fallback counters; reason-specific fallback diagnostics remain.

The shared Module is the framework Implementation. `aimer_scroll` and other adopters provide only placement policy and
scroll/transform details through their small Interfaces.

## Phase 3 — migrate existing scroll retention

- [x] Make `Scrollable` use the shared `PaintIsolated` Module for the stable full-child case, with the dynamic-island
      and retained-tile paths using the shared `PaintCache` validity seam while keeping their distinct policies.
- [x] Preserve scroll culling, visible-range behavior, scroll offset transforms, tile overlap, and child ordering.
      Coverage now includes dynamic-row culling and composed translation, retained-tile overlap windows, and a
      dynamic-before-stable ordering fallback in `crates/aimer_scroll/src/scrollable/core/raw_scroll.rs`.
- [x] Verify that offset-only scroll changes the cache composition state but not the child paint generation.
      The regression verifies one child `paint` record, zero normal `draw` calls, and live geometry synchronization
      at both scroll offsets.
- [x] Verify that unstable children still use dynamic islands or direct drawing exactly as before.
      Existing dynamic-island coverage plus the mixed-order fallback regression keep the conservative direct path.
- [x] Remove duplicate cache invalidation logic only after shared and existing paths produce identical output.
      Stable layers, dynamic islands, and retained tiles now retire through the shared `PaintCache` validity seam; the
      existing output/order and invalidation regressions remain green.

## Phase 4 — implement core damage-region repaint

- [x] Define and test a deterministic device-pixel `DamageSet` with half-open rectangles, finite-value normalization,
  clipping, union, coalescing, and full-frame promotion thresholds.
- [x] Add a persistent target Module that retains the initialized color target/view between frames.
      `aimer_cupid::persistent_target::PersistentTarget` now owns the texture/view lifecycle and is used by the
      existing material-frame target path; a newly created target remains invalid until a complete paint finishes.
- [x] Include target size, device scale, surface identity, renderer/context generation, and validity in the target key.
      `PersistentTargetKey` carries all of those fields, while resource reuse deliberately ignores only validity so an
      invalid target can retain its allocation and request a full repaint.
- [x] Make the first frame, resize, scale change, surface recreation, context loss, and unknown damage use a full repaint.
      `PersistentTargetState::full_repaint_reason` classifies each conservative promotion, and
      `TargetEnsureResult::requires_full_repaint` keeps the full-clear path mandatory until a valid partial repaint
      packet exists.
- [x] Derive damage from `PaintIsolated` invalidations, old/new layer bounds, clip/transform changes, ordering changes,
  resource readiness, and conservative effect expansion.
      `DamageTracker` normalizes `DamageLayerChange` records, transforms and clips old/new footprints, expands bounded
      effects, and promotes unknown geometry or ordering changes; `PaintIsolated` submits those transitions with the
      frame `DrawList` while preserving its record/replay behavior.
- [x] For the supported partial path, clear the single coalesced damage region with a replace-blend scissor, load
  preserved pixels from the persistent target, and replay the complete resolved command stream in paint order under
  that scissor so unchanged layers behind the changed layer are included. Disjoint regions, MSAA, custom/backdrop
  commands, and untracked or unknown damage conservatively use the full path until their validity seams are ready.
- [x] Preserve unchanged target pixels; never filter the current draw list and leave omitted pixels blank. The scene
  target is persistent across swap-chain frames, while the final surface composite remains full-surface.
- [ ] Emit an owned `FramePacket` containing target validity and generations, actual device scale and surface/context
  identity, damage set, ordered layer metadata, retained-layer references, and resource lifetime information safe for
  the raster thread. The current renderer derives damage directly inside `render_frame` and still uses placeholder
  scale/identity values, so this is a required framework seam rather than a Jaime integration detail.
- [ ] Feed real device scale, surface identity, renderer/context generations, and resource generations into the packet
  and target key; invalidate the persistent target when any of those values change.
- [ ] Add ordered layer metadata and dependency queries so the region renderer can process only layers intersecting each
  damage region while preserving all required background, clip, transform, and effect dependencies. The current safe
  slice replays the complete command stream under one scissor; that proves pixels but does not yet minimize command
  preparation.
- [ ] Support multiple damage regions and define measured coalescing/area thresholds. Until then, promote disjoint or
  uncertain regions to the existing full-frame path.
- [ ] Extend the partial capability matrix to MSAA, custom pipelines, backdrop reads, filters, shadows, and async
  resources, or promote each unsupported path before modifying preserved target pixels.
- [ ] Promote to the existing full-frame path when memory, target validity, bounds, ordering, effects, resource
  readiness, or renderer capabilities are uncertain.
- [ ] Add diagnostics for damage area, full-frame promotions, partial/full clears, target reuse, and fallback reasons.

The first damage implementation may repaint the full layer set inside each damaged region. A spatial index is not needed
until measurements show that layer intersection queries are material, but ordered layer metadata and dependency
correctness are required before command filtering is enabled.

Implementation order for the remaining framework work: `FramePacket` ownership, real device geometry and generations,
ordered layer/dependency metadata, multiple-region and renderer capability handling, then diagnostics. Only after those
generic seams are covered should Jaime be used for the independent-sibling acceptance run.

## Phase 5 — validate framework adopters, then prove the independent-sibling case in Jaime

- [ ] Add at least one generic framework adopter/test fixture that places `PaintIsolated` around an independently
  invalidated sibling region without importing Jaime code or assumptions.
- [ ] Add a private Jaime/framework `ContentPane` Module only as the acceptance adapter that places `PaintIsolated`
  around the right content region.
- [ ] Keep ordinary application/widget-user composition unchanged; `PaintIsolated` is internal to the
  framework/widget Implementation.
- [ ] Keep the sidebar's existing `Scrollable` responsible for scrolling, culling, tile selection, and dynamic rows.
- [ ] Ensure a sidebar offset changes neither the content paint generation nor the content layer's device bounds.
- [ ] Ensure sidebar hover, pressed, focus, and selection visuals remain live where required.
- [ ] Confirm cached content is composited in the same order and clip as the direct path.
- [ ] Keep selection changes explicit: invalidate affected sidebar visuals and content once, then replay content on later
  sidebar-only frames.
- [ ] Do not add a provider/state split unless Phase 0 proves an unnecessary ancestor rebuild.

Jaime validates the framework Seam. It must not grow a Jaime-only cache, renderer path, or invalidation protocol.

## Phase 6 — invalidation and fallback policy

| Change | Required action |
| --- | --- |
| Independent offset/transform only | Replay `PaintIsolated`; update live region/composition state. |
| Child local state change | Invalidate and re-record `PaintIsolated` or a smaller isolated child. |
| Text/style/theme change | Invalidate affected Module and dependent text/resource generations. |
| Image/font/async resource readiness | Invalidate the dependent retained layer. |
| Animation, cursor, viewport, backdrop, or dynamic effect | Keep affected subtree live or use direct fallback. |
| Layout, bounds, clip, transform, device scale, or resize | Invalidate and re-record with new geometry. |
| Insert/remove/reorder or unknown ordering/effect dependency | Invalidate; use direct rendering if bounds/order are unknown. |
| MSAA or unsupported renderer path | Use the existing compatible fallback. |

Rules:

- [ ] Never clear or replace a cached layer with an empty result merely because recording was skipped.
- [ ] Never reuse a layer across an untracked resource, transform, clip, or ordering change.
- [ ] If old/new bounds or dependencies cannot be determined conservatively, use direct rendering.
- [ ] Keep invalidation separate from user input and event routing.
- [ ] Include the fallback reason in diagnostics so an ineffective optimization is explainable.

## Phase 7 — framework correctness tests

- [ ] Add a direct-vs-`PaintIsolated` pixel oracle for stable primitive content.
- [x] Add deterministic tests for the initial `DamageSet` normalization, union, clipping, coalescing, and full-frame
      promotion model.
- [ ] Test persistent-target load/clear behavior so an omitted region can never become blank or stale.
- [ ] Test first record, cache replay, invalidation, fallback, cache destruction, and renderer/context recreation.
- [ ] Test nested isolated Modules, changed parent transform, changed clip, opacity, z-order, and sibling overlap.
- [ ] Test damage repaint with unchanged background layers behind changed foreground layers.
- [ ] Test stable text, images, retained children, layout wrappers, custom drawing, async resources, animation, and
  effects.
- [ ] Verify hit-test bounds, focus, accessibility, keyboard navigation, and input routing while paint is cached.
- [ ] Verify feature-off and wasm builds retain the existing direct behavior.

## Phase 8 — Jaime regression and performance acceptance

- [ ] Compare direct and `PaintIsolated` output after slow scroll, fast scroll, momentum, reversal, selection, hover,
  focus, resize, scale, theme, and window occlusion.
- [ ] During offset-only sidebar scrolling, content build count does not increase.
- [ ] After the initial record, content normal-paint and command-record counts remain zero except for documented
  invalidation.
- [ ] Content retained-layer replay occurs without content re-rasterization.
- [ ] Sidebar rows update at the required scroll cadence.
- [ ] Offset-only frames reuse a valid persistent target and do not perform an unnecessary full-surface clear.
- [ ] Damage sets include old/new bounds and dependent clips/effects, while excluding the unchanged content pane in the
  Jaime sidebar-scroll case.
- [ ] Every damaged region is repainted in correct layer order, including required unchanged background layers.
- [ ] In the defined Jaime workload, sustained sidebar-only scrolling averages below 20% process CPU; report peak and
  percentile CPU separately.
- [ ] Frame CPU time, command count, GPU work, memory, and dropped frames improve or remain within the agreed budget.
- [ ] A real Jaime workload and at least one non-Jaime framework workload demonstrate the result.
- [ ] No stale pixels, ordering changes, interaction regressions, or unbounded cache growth occur.

Focused validation commands after implementation:

```bash
cargo test -p aimer_scroll
cargo test -p aimer_widget
cargo test -p aimer_cupid
cargo test -p aimer_quiver
cargo test -p jaime --lib
```

Run the workspace suite after the focused tests pass. Add feature-specific commands only after the opt-in feature is
wired into the manifests.

## Phase 9 — optional damage-region extensions

Only after the core `PaintIsolated` and `DamageRegion` Modules pass correctness and performance acceptance:

- [ ] Measure whether replaying retained layers still spends material time on unchanged screen regions.
- [ ] Add spatial indexing, advanced effect expansion, MSAA support, and multi-window policy one at a time.
- [ ] Keep `PaintIsolated` as the reusable subtree-level Seam even if a global compositor is later added.
- [ ] Keep direct rendering as the fallback for wasm, unsupported effects, unknown bounds, and resource uncertainty.

This ordering makes `PaintIsolated` and `DamageRegion` the framework solution and Jaime the evidence source. The
widget-user Interface stays simple, widget developers provide the safe paint contract, and renderer Adapters hide
retention and target details. Further compositor complexity is justified only by measured remaining work, not by the
assumption that a root draw call means a full widget rebuild.
