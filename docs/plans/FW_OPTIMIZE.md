# Framework optimization plan

## Decision

Profile and improve framework-level hot paths independently of SIMD. The goal is to remove avoidable tree work first,
while preserving the distinct semantics of layout, drawing, hit testing, focus, and event dispatch.

This work is useful on its own. Its measurements also establish whether the numeric kernels left after framework
optimization are significant enough to justify the SIMD work tracked in [SIMD_CALCULATION.md](SIMD_CALCULATION.md).

## Goals

- [x] Measure layout, reconciliation, drawing, hit testing, focus, and event dispatch separately on representative
  small, medium, and large trees.
- [x] Reduce unnecessary full-tree walks, allocations, and dynamic dispatch on measured hot paths.
- [ ] Preserve the existing public widget and element interfaces unless a separate API decision approves a change.
- [ ] Preserve event, hit-test, focus, lifecycle, ordering, and retained-state behavior.
- [ ] Verify every optimization with both focused tests and end-to-end frame measurements.

## Non-goals

- [ ] Implement explicit SIMD kernels; those remain in
  [SIMD_CALCULATION.md](SIMD_CALCULATION.md).
- [ ] Replace the widget tree or dynamic dispatch with a vector API.
- [ ] Add a spatial index before bounds/window culling and profiling establish that it is necessary.
- [ ] Change event or hit-test semantics merely to make traversal cheaper.

## Phase 0 — Baseline and profiling

- [x] Define representative small, medium, and large widget trees, including eager children, windowed lists, layered
  siblings, focusable controls, and stateful descendants.
- [x] Capture cold layout, cached layout, scrolling, resizing, animation, pointer hit testing, focus traversal, and
  event-dispatch profiles.
- [x] Record p50/p95 CPU time, allocations, frame time, and binary-size impact using a reproducible release-profile
  command and fixed workloads.
- [ ] Separate framework work from platform, canvas, GPU submission, and child application work in the measurements.
- [x] Identify the top measured costs and define an exit threshold for each proposed optimization before implementing
  it.

### Initial baseline progress

The first executable baseline is available at
[`aimer_laboratory/examples/framework_baseline.rs`](../aimer_laboratory/examples/framework_baseline.rs). It drives the
real headless frame loop and sends resize and pointer events through the production handlers. It reports the p50/p95 of
each round's average operation time, plus allocations per operation. The workloads use a fixed 1,150 × 800 headless
display and are run with:

```text
cargo run -p aimer_laboratory --example framework_baseline --release
```

The pre-optimization release run recorded 7,199,312 bytes for the example executable and these results:

| Workload                                  |   p50 CPU |   p95 CPU | allocations/op |
|-------------------------------------------|----------:|----------:|---------------:|
| Eager column, 32 rows, cold frame         |  20.58 us | 224.38 us |          16.00 |
| Eager column, 32 rows, cached frame       |   9.34 us |   9.77 us |           2.00 |
| Eager column, 256 rows, cold frame        |  41.62 us | 128.79 us |          48.14 |
| Eager column, 256 rows, cached frame      |  27.96 us |  28.13 us |           4.00 |
| Eager column, 2,048 rows, cold frame      | 281.08 us | 544.50 us |         279.00 |
| Eager column, 2,048 rows, cached frame    | 221.55 us | 222.92 us |           7.00 |
| Windowed list, 120,000 rows, cold frame   |  10.75 us | 106.00 us |          89.14 |
| Windowed list, 120,000 rows, cached frame |   3.45 us |   4.38 us |           2.00 |
| Eager column, 256 rows, resize            |  30.64 us |  30.83 us |           7.00 |
| Eager column, 2,048 rows, resize          | 246.03 us | 247.16 us |          10.00 |
| Windowed list, 120,000 rows, resize       |   3.76 us |   3.82 us |           4.00 |
| Eager column, 32 rows, pointer move       |   1.61 us |   1.69 us |           0.00 |
| Eager column, 256 rows, pointer move      |  11.47 us |  11.87 us |           3.00 |
| Eager column, 2,048 rows, pointer move    |  90.41 us |  94.43 us |           6.00 |
| Windowed list, 120,000 rows, pointer move |   1.86 us |   1.90 us |           1.00 |

The initial run covered cold and cached frames, resizing, pointer hit testing, and layered sibling workloads. The
expanded matrix below adds named scrolling, animation, event-dispatch, focusable-control, and stateful-descendant
workloads. A framework-only traversal workload isolates dispatcher cost from widget and canvas work, although platform,
canvas, and GPU submission are not yet independently instrumented. The eager 2,048-row pointer path was the first clear
bounds/window-culling target, while the windowed list provides the bounded traversal comparison.

### Phase 0 matrix extension

The same release command and fixed display were rerun after the structural focus changes. These additional rows complete
the named workload matrix; the focus traversal rows remain recorded in the optimization section below.

| Workload                                           |     p50 CPU |     p95 CPU | allocations/op |
|----------------------------------------------------|------------:|------------:|---------------:|
| Scrollable column, 256 rows, scroll + frame        |    13.32 us |    13.92 us |           5.02 |
| Scrollable column, 2,048 rows, scroll + frame      |    18.04 us |    18.51 us |           5.02 |
| Windowed list, 120,000 rows, scroll + frame        |     7.63 us |     8.15 us |           4.34 |
| Focusable column, 32 controls, cached frame        |     3.52 us |     3.83 us |           1.00 |
| Focusable column, 256 controls, cached frame       |     4.08 us |     4.42 us |           1.00 |
| Focusable column, 2,048 controls, cached frame     |     4.20 us |     4.58 us |           1.00 |
| Stateful column, 32 buttons, cached frame          |     5.49 us |     5.71 us |           1.00 |
| Stateful column, 256 buttons, cached frame         |     6.80 us |     7.08 us |           1.00 |
| Stateful column, 2,048 buttons, cached frame       |     6.68 us |     6.91 us |           1.00 |
| Animation framework shell, one child, cached frame |     0.63 us |     0.64 us |           7.00 |
| Animated column, 256 rows, cached frame            |   206.28 us |   209.09 us |          87.00 |
| Animated column, 2,048 rows, cached frame          | 2,984.73 us | 3,025.88 us |       1,148.00 |
| Framework event dispatch, 32 nodes                 |     0.44 us |     0.53 us |           0.00 |
| Framework event dispatch, 256 nodes                |     2.72 us |     3.02 us |           3.00 |
| Framework event dispatch, 2,048 nodes              |    29.00 us |    31.16 us |           6.00 |

The executable size for this matrix run is 7,315,424 bytes. For each new optimization, retain the change only when the
named hot workload improves by at least 10% at p50 or removes at least one allocation per operation without regressing a
corresponding correctness or retained-state test. The 2,048-row animated column is a mixed framework/application
workload: its one-child framework shell is 0.63 us, while rebuilding 2,048 application rows dominates the 2,984.73 us
result. It is therefore tracked separately from the next framework-only target.

### First adopted optimization — hidden-flex bounds culling

The first red-green change uses the finite constraint of a hidden-overflow flex as its visible main-axis range when no
ancestor supplies `visible_rect`. Visible-overflow flexes keep their full child set, and explicit ancestor windows keep
using their existing range. Regression coverage is in `aimer_flex`'s lazy-flex tests.

The same release benchmark was rerun after the change:

| Workload                          | Before p50 | After p50 | Before allocs/op | After allocs/op |
|-----------------------------------|-----------:|----------:|-----------------:|----------------:|
| Eager 2,048-row cached frame      |  221.55 us |   6.02 us |             7.00 |            2.00 |
| Eager 2,048-row resize            |  246.03 us |  62.17 us |            10.00 |            5.00 |
| Eager 2,048-row pointer move      |   90.41 us |   1.73 us |             6.00 |            1.00 |
| Windowed 120,000-row pointer move |    1.86 us |   1.82 us |             1.00 |            1.00 |

The large eager pointer path fell by about 98% and the cached frame by about 97%, while the already-windowed pointer
path stayed within measurement noise. This establishes bounds/window culling as a worthwhile optimization and keeps the
remaining focus, event, animation, and stateful-tree work open.

### Second adopted optimization — generation-keyed layer order

Layered sibling workloads use `Align::layer` with reverse input order so the paint sort cannot be skipped. `RawFlex` now
caches the sorted visible range by element-tree generation and painted range, while `Stack` caches the full child
permutation by element-tree generation and child count. Default-layer flex ranges iterate directly without allocating an
order table. Stack normal and reverse paint order, ascending hit-test order, and flex event-child behavior remain
unchanged.

The same release benchmark was rerun after the change. The layered flex result is within timing noise but removes one
cached-frame allocation; the full-stack workload shows the larger sibling-group resource improvement:

| Workload                                 | Before p50 | After p50 | Before p95 | After p95 | Before allocs/op | After allocs/op |
|------------------------------------------|-----------:|----------:|-----------:|----------:|-----------------:|----------------:|
| Layered column, 2,048 rows, cached frame |    4.63 us |   4.70 us |    4.80 us |   4.80 us |             2.00 |            1.00 |
| Layered stack, 2,048 rows, cached frame  |  156.57 us | 149.33 us |  179.57 us | 151.94 us |             3.00 |            1.00 |
| Layered stack, 2,048 rows, resize        |  176.98 us | 170.14 us |  177.62 us | 172.64 us |             3.00 |            1.00 |
| Layered stack, 2,048 rows, pointer move  |  125.56 us | 124.48 us |  128.78 us | 126.83 us |             8.00 |            6.00 |

This retains the cache for its repeatable allocation reduction and the stable stack improvement; a spatial index remains
a separate decision for after the focus and event workloads are measured.

### Third adopted optimization — structural focus traversal reuse

Focus traversal and reconciliation previously combined `event_children` and
`visit_children` into a fresh deduplicated `SmallVec` at every structural node. The new
`EventElement::structural_children` seam preserves that union as the default, while common flex, stack, grid,
single-child, positioned, focusable, and scroll elements expose their canonical structural children directly. Focus
candidate collection now streams through that seam, the dispatcher reuses candidate-buffer capacity while still
reevaluating every focus gate, and saved-path lookup selects a child without materializing the whole sibling list.
Pointer and event child semantics remain on their existing accessors.

The framework-only profile uses the same seven rounds and 64 measured Tab operations as the earlier profile. It isolates
a retained root with one focusable leaf per node:

| Workload                     |  Before p50 | After p50 |  Before p95 | After p95 | Before allocs/op | After allocs/op |
|------------------------------|------------:|----------:|------------:|----------:|-----------------:|----------------:|
| Focus traversal, 32 nodes    |     1.40 us |   0.92 us |     1.50 us |   0.93 us |             7.00 |            6.00 |
| Focus traversal, 256 nodes   |    37.12 us |   6.81 us |    38.49 us |   8.87 us |            19.00 |           15.00 |
| Focus traversal, 2,048 nodes | 1,707.95 us |  70.67 us | 1,715.54 us |  72.02 us |            31.00 |           24.00 |

The large focus traversal fell by about 96% at p50 and p95, with 7 fewer allocator operations per Tab. The remaining
allocations are isolated in the framework-only profile for follow-up; candidate values are deliberately not cached
because focus gates are required to be reevaluated on every gather. The final release benchmark executable is 7,249,072
bytes, compared with 7,199,312 bytes in the pre-optimization run; that size also includes the added layered and
framework-only benchmark workloads.

### Fourth adopted optimization — reverse hit-test child visitation

Routed pointer dispatch used to collect each element's hit-test children into a temporary `SmallVec` and pop them in
reverse paint order. That spills for large sibling groups even when the retained container already has indexable
storage.
`EventElement::hit_test_children_reversed` now preserves the old allocating default for custom elements, while flex,
stack, grid, and the benchmark's large structural container visit their children directly in dispatch order. Pointer
ordering, capture, focus-on-press, and drag follow-up behavior remain unchanged.

The release matrix was rerun after this change:

| Workload                              | Before p50 | After p50 | Before p95 | After p95 | Before allocs/op | After allocs/op |
|---------------------------------------|-----------:|----------:|-----------:|----------:|-----------------:|----------------:|
| Framework event dispatch, 32 nodes    |    0.44 us |   0.36 us |    0.58 us |   0.37 us |             0.00 |            0.00 |
| Framework event dispatch, 256 nodes   |    3.48 us |   2.68 us |    3.59 us |   2.69 us |             3.00 |            0.00 |
| Framework event dispatch, 2,048 nodes |   30.71 us |  21.56 us |   31.07 us |  22.60 us |             6.00 |            0.00 |
| Animation framework shell, one child  |    0.63 us |   0.60 us |    0.64 us |   0.62 us |             7.00 |            7.00 |

The large event path improved by about 30% at p50 and removed all measured per-operation allocations. The latest matrix
executable is 7,315,520 bytes; the small size increase is benchmark-harness code for the direct-order fixture.

### Fifth adopted optimization — stable Flex child measurement reuse

Measured Flex tables now retain compact identity and subtree-revision metadata only when every direct child explicitly
reports generation-independent sizing. After an unrelated generated subtree advances the global tree generation, a
stable table compares those revisions and reuses the existing measurements without rebuilding every child. A changed
identity or revision, any flexible child, or a non-stable child keeps the existing full remeasurement and redistribution
path. Revision state stays outside the erased element payload so inline element storage does not grow.

The focused regression uses 256 fixed-size children and advances an unrelated tree generation between size queries:

| Workload                                      | Before child measurements | After child measurements |
|-----------------------------------------------|--------------------------:|-------------------------:|
| Stable 256-child Flex after unrelated rebuild |                       256 |                        0 |

The end-to-end release workload keeps a fixed-size Flex column beside an animation-driven sibling, so the animation
advances the tree generation while the column remains unchanged:

| Workload                              | After p50 | After p95 | allocations/op |
|---------------------------------------|----------:|----------:|---------------:|
| Stable column + animation, 256 rows   |   3.57 us |   3.65 us |          10.00 |
| Stable column + animation, 2,048 rows |  11.28 us |  11.48 us |          10.00 |

The final matrix executable is 7,315,616 bytes. The dynamic-child regression remains green: children that resize
themselves do not opt in and still trigger the existing remeasurement path. The fixed-size workload is therefore the
accepted stable-Flex target; broader dynamic child subtrees remain conservative until they expose an equivalent revision
contract.

### Website follow-up — remove eager flex wrapping from the blog archive

The website blog archive used `OverflowBehavior::Wrap` on its vertical
`Column`. That mode is for flex-child wrapping, not text wrapping; it materializes and measures the complete child list
on every draw because each line break depends on preceding children. The archive's text widgets already request
`TextOverflow::Wrap`, so removing the column-level mode preserves the intended text behavior and restores normal
scroll-window culling.

The benchmark fixture now compares the old wrapped-column shape with the fixed plain-column shape under the same native
headless scroll workload:

| Workload                  |  Old p50 | Fixed p50 | Old allocations/op | Fixed allocations/op |
|---------------------------|---------:|----------:|-------------------:|---------------------:|
| 256-row column, release   | 14.90 us |  13.16 us |               9.02 |                 5.02 |
| 2,048-row column, release | 50.54 us |  19.58 us |              12.02 |                 5.02 |

The website change is in `website/src/screen/blog.rs`; the corresponding regression workload is retained in
`aimer_laboratory/examples/framework_baseline.rs`.

### Sixth adopted optimization — reuse full-parent `Align` contexts

The layered stack workload was still spending most of its cached-frame time inside repeated `Align` wrappers. When an
aligned child already fills the parent and receives the same zero-minimum, parent-sized constraint, `Align`
now reuses the existing `BuildContext` instead of cloning it and translating the canvas by zero. Canvas save/restore
remains in place so a child that leaves canvas state behind cannot affect its siblings. The guarded path leaves non-full
children and all non-zero alignment offsets unchanged.

The release benchmark crossed the 10% retention threshold on the measured stack paths:

| Workload                                | Before p50 | After p50 |       Change | allocations/op |
|-----------------------------------------|-----------:|----------:|-------------:|---------------:|
| Layered stack, 2,048 rows, cached frame |  148.77 us | 130.90 us | 12.0% faster |           1.00 |
| Layered stack, 2,048 rows, resize       |  178.09 us | 152.02 us | 14.6% faster |           1.00 |

The debug run also completed successfully; its post-change p50 was 2,174.62 us for the 2,048-row cached stack and
2,372.30 us for resize. Pointer hit-testing remains separate at 102.02 us in release because this change only removes
draw-context overhead. That remaining stack hit-test cost needs a position-aware spatial/indexing decision and is not
folded into this change.

### Seventh adopted optimization — bounded pointer descent

Pointer dispatch now checks an element's retained screen bounds before walking its hit-test children. A known-outside
subtree returns immediately; elements that do not report bounds remain candidates, so transparent wrappers and unknown
transforms keep their previous conservative behavior. `Positioned`
records bounds for identity and translation-only painting and deliberately leaves scale and rotation bounds unknown
rather than risking an under-cull. Focus traversal, broadcast delivery, captured-pointer routing, and ordinary
event-child order are unchanged. `EventElement` now has additive defaulted position-aware hooks; existing
implementations retain their behavior without needing to implement them.

The benchmark adds sparse positioned siblings to show the work removed by the early return. The comparison is against
the same release fixture with the bounds cache present but without bounded-subtree pruning:

| Workload                                          | Before p50 | After p50 | Before p95 | After p95 | allocations/op |
|---------------------------------------------------|-----------:|----------:|-----------:|----------:|---------------:|
| Sparse layered stack, 256 rows, pointer move      |   11.84 us |   1.71 us |   12.30 us |   1.73 us |           0.00 |
| Sparse layered stack, 2,048 rows, pointer move    |   94.27 us |  12.75 us |   99.14 us |  12.85 us |           0.00 |
| Full-area layered stack, 2,048 rows, pointer move |  103.16 us | 102.75 us |  105.80 us | 105.06 us |           0.00 |

The sparse 2,048-row path is 86.5% faster in release while the full-area stack remains within measurement noise. The
final debug run completed with a 93.28 us p50 for the sparse 2,048-row pointer path. This is still an O (n)
bounds check over the retained sibling list; a spatial or interval index is a separate future step if measured workloads
show that scan itself becoming the dominant cost.

### Eighth adopted optimization — retained sparse-stack y index

The sparse stack still paid the O (n) bounded-descent scan after the seventh optimization. `RawStackElement` now lazily
retains a 64-bin y-range index in painted topmost-first order. A pointer query visits only the bin candidates; unknown
bounds remain in every bin, out-of-range queries use the unknown-only fallback, and estimated-dense groups use the exact
existing traversal. The index is retired when the stack draws new child bounds and also keys itself to the element-tree
generation and child count, so layout and structural changes cannot reuse stale candidates.

The position-aware hooks are additive and defaulted, so existing custom
`EventElement` implementations continue to compile and use their original traversal. The regression tests cover topmost
ordering, unknown bounds, and rebuilding after bounds invalidation.

| Workload                                          | Before p50 | After p50 | Before p95 | After p95 | allocations/op |
|---------------------------------------------------|-----------:|----------:|-----------:|----------:|---------------:|
| Sparse layered stack, 256 rows, pointer move      |    1.71 us |   0.13 us |    1.73 us |   0.14 us |           0.00 |
| Sparse layered stack, 2,048 rows, pointer move    |   12.75 us |   0.32 us |   12.85 us |   0.32 us |           0.00 |
| Full-area layered stack, 2,048 rows, pointer move |  102.75 us | 102.90 us |  105.06 us | 103.57 us |           0.00 |

The sparse 2,048-row path is about 40x faster in release, with the debug p50 falling from 93.28 us to 3.43 us. The
full-area path remains within benchmark noise and keeps the exact fallback; dense detection also avoids allocating bin
contents for that case.

### Ninth adopted optimization — reuse the scalar flex share buffer

`FlexLayout::build` used to allocate a second vector for the shares produced by flex-weight distribution. The measured
scalar path now reuses its existing weight table in place. Negative non-flex sentinels remain intact, and negative zero
records a zero-weight flex child so finite and unbounded constraints keep their prior behavior. The change is private to
layout implementation and does not make the framework API SIMD-aware.

The expanded fixture mixes regular rows with `Expanded` children and measures the complete layout path:

| Workload                                |     Before p50/p95 |      After p50/p95 | Before allocs/op | After allocs/op |
|-----------------------------------------|-------------------:|-------------------:|-----------------:|----------------:|
| Expanded column, 256 rows, cold frame   |   51.54 / 55.12 us |   50.29 / 52.50 us |            49.43 |           47.43 |
| Expanded column, 256 rows, resize       |   20.29 / 21.12 us |   20.84 / 21.17 us |            14.00 |           12.00 |
| Expanded column, 2,048 rows, cold frame | 160.21 / 168.71 us | 149.75 / 170.08 us |           215.00 |          213.00 |
| Expanded column, 2,048 rows, resize     |  85.98 / 117.13 us |   84.78 / 85.63 us |            14.00 |           12.00 |

The 2,048-row cold p50 improves by 6.5%; more importantly, all four expanded workloads remove two allocations per
operation. The allocation reduction meets the retention gate even where timing is within noise. Explicit SIMD remains a
separate next step in [SIMD_CALCULATION.md](SIMD_CALCULATION.md).

### Measured candidate — fused size-table construction (not retained)

The next scalar candidate fused uniform-size detection with varying offset construction after the first disagreement. It
improved the isolated varying 2,048-entry constructor, but the complete frame path did not cross the 10% CPU threshold
and allocations were unchanged:

| Workload                                            |     Before p50/p95 |  Candidate p50/p95 | allocations/op |
|-----------------------------------------------------|-------------------:|-------------------:|---------------:|
| Varying size column, 2,048 rows, release cold frame |   57.46 / 64.88 us |   54.00 / 65.42 us |  21.00 / 21.00 |
| Varying size column, 2,048 rows, release resize     |   40.47 / 41.16 us |   36.96 / 38.49 us |  15.00 / 15.00 |
| Varying size column, 2,048 rows, debug cold frame   | 565.71 / 576.50 us | 550.71 / 566.71 us |  21.00 / 21.00 |

The candidate was removed to keep the production path simple. Its focused profile remains in `aimer_flex` for future
larger-table measurements; the next numeric target should be selected from a separately measured domain.

### Tenth adopted optimization — bounded GPU image-cache lifetime

The native image pipeline previously retained every uploaded GPU texture until the caller explicitly removed it.
Reconstructible hashed `LoadImage` textures now carry a last-used frame and are reclaimed after 120 idle frames or when
older entries are needed to bring the cache back under its 128 MiB soft byte budget. Textures used by the current frame
are protected, eviction happens after queue submission, and explicitly addressed uploads remain pinned because the
framework cannot reconstruct their source bytes.

The canvas keeps only compact intrinsic-size metadata for an evicted ID and marks it unavailable through an atomic flag.
A retained file, asset, or network image widget notices the cache generation change, drops its stale ID, and lets its
provider reload the source. The provider-side `Loaded` record is also discarded when it observes an unavailable ID;
decoded RGBA bytes were already released after the original upload, so the static source maps retain only keys,
dimensions, and IDs between reloads.

The focused eviction-policy and stale-metadata regressions pass in `aimer_cupid`. This change is a resource-lifetime
optimization rather than a CPU-speed claim.

The follow-up profile is `aimer_cupid/examples/image_scroll_memory_profile.rs`. It renders four unique 256x256 RGBA8
images per frame for 180 scrolling frames, then keeps the final viewport visible for 121 more frames. The release run
reported a peak logical image cache of 480 textures / 125,829,120 bytes and a settled cache of 4 textures / 1,048,576
bytes. These are the renderer's logical RGBA texture bytes, not a substitute for the operating system's physical GPU
memory counter; the result confirms that old entries are reclaimed while the visible viewport remains cached.

### Eleventh adopted optimization — stop native off-screen text redraw loops

`TextPipelineV2` prepares visible text first and uses a small budget to prepare text ahead of the viewport. The native
presenter previously converted an unfinished off-screen preparation tail into another animation-frame request. That kept
the macOS page rendering after it had visually settled on the image section: the baseline five-second native sample
contained 202
`AimerApplicationHandler::render` samples and repeated WGPU command-encoding stacks.

The native presenter now treats deferred off-screen text as advisory. A later real frame from scrolling, input,
animation, or another state change prepares whatever has become visible; the WASM presenter is unchanged. In the rebuilt
native app, the image section remained visible while Activity Monitor reported 0.0% CPU and 0.0% GPU, and a five-second
post-change sample found no render-loop symbols while the process was sleeping.

### Twelfth adopted optimization — skip repeated clean subtree rebuild walks

Every retained self-rebuilding `StatelessElement` and `StatefulElement` now records the rebuild-invalidation generation
at which it last visited its descendants. A clean element returns immediately when that generation is still current. The
generation advances when a real build consumer, state updater, portable state mutation, or async completion wakes the
tree; recursive explicit dirty marking is coalesced into one advance. New elements use a sentinel so their first
retained walk is never skipped. Pure wrappers clear their local dirty flag after forwarding the walk.

This is a clean-propagation guard, not a claim that an intentionally dirty tree is cheap. The next optimization adds a
separate precise dirty-subtree index without overloading this generation.

The phase profile now includes a same-process old-walk reference beside the generation-gated path:

| Workload                       | Gated p50 / p95 | Old walk p50 / p95 |
|--------------------------------|----------------:|-------------------:|
| Clean propagation, 32 nodes    |  0.03 / 0.05 us |     0.43 / 0.44 us |
| Clean propagation, 256 nodes   |  0.01 / 0.07 us |     1.58 / 2.74 us |
| Clean propagation, 2,048 nodes |  0.01 / 0.04 us |   12.20 / 12.82 us |

The gated path stays effectively constant while the old path continues to walk every retained child. In the same run,
the deliberately dirty 2,048-node reconciliation path measured 2,828.74 us p50 / 3,037.71 us p95; the clean guard does
not hide that work or change its correctness semantics.

### Thirteenth adopted optimization — index precise dirty subtrees

The invalidation generation remains a wake-up guard; it is not used as a dirty subtree index. Built-in self-rebuilding
elements now share an internal dirty source with their build consumer or state updater. After a retained walk, the
source records its root-relative `ElementId` path, and a thread-local reference-counted index marks only the ancestors
of currently dirty sources. During the next rebuild walk, clean siblings whose paths are absent from that index are
skipped. The normal erased-element `draw` entry point performs this prepass as well, so native frame dispatch uses the
same index. A dirty source still forces traversal of its own descendants, because its rebuild can change the shape below
it.

Structural tree changes, asynchronous/portable invalidations, custom dirty markers without a tracked source, and
incomplete walks invalidate the index and fall back to the previous conservative traversal. Keyed state adoption
transfers the dirty source with the live state so its path is not orphaned. Multiple dirty sources share ancestor counts
and release them independently after rebuilding; independent subtree-root walks invalidate the relative paths and
conservatively re-index the full retained root.

The focused retained-tree regression workload measured visits rather than CPU:

| Workload                                                 |                              Before |                                                                                After |
|----------------------------------------------------------|------------------------------------:|-------------------------------------------------------------------------------------:|
| One intentionally dirty wrapper beside one clean sibling |               1 clean-sibling visit |                                                               0 clean-sibling visits |
| Two dirty siblings, then only the first dirty            | clean sibling visited on both walks | clean sibling skipped on both walks; second dirty sibling skipped on the second walk |

The first row is the red-before/green-after regression assertion; the second row is covered by the shared-ancestor
reference-counting test. Stateful state updater marking and the custom-marker fallback have separate regression tests.

### Fourteenth adopted optimization — generation-keyed layout invalidation

An erased retained root used to call `invalidate_layout` recursively through every descendant before the next resize
layout. The layout caches already describe their inputs, so the framework now advances one layout-invalidation
generation at the erased `ElementNode` boundary. `LayoutCache`, `FlexLayoutCache`, the grid layout table, and prepared
paragraph layouts include that generation in their cache keys. A grid drops entries from older generations lazily when a
new layout is stored, preserving multiple constraint variants within one frame without retaining stale variants across
resizes. Callers that own a concrete raw element directly keep its existing local invalidation behavior.

The focused regression verifies that an erased root advances the marker without invoking a child invalidation walk, and
the cache regression verifies that the marker retires a cached measurement. The release phase profile now measures the
marker independently:

| Workload                                   | Before p50 / p95 |  After p50 / p95 | Before allocs/op | After allocs/op |
|--------------------------------------------|-----------------:|-----------------:|-----------------:|----------------:|
| Layout, 2,048 nodes (invalidate + measure) | 61.50 / 64.22 us | 46.14 / 49.07 us |                — |               — |
| Layout invalidation marker, 2,048 nodes    |                — |   0.00 / 0.02 us |                — |               — |
| Eager column, 2,048 rows (resize)          | 65.73 / 66.35 us | 50.63 / 51.70 us |             5.00 |            4.00 |
| Layered column, 2,048 rows (resize)        | 64.46 / 69.67 us | 51.32 / 53.39 us |             6.00 |            4.00 |

The combined 2,048-node layout phase is about 25% faster at p50, while the invalidation-only phase is effectively
constant. The cached-frame path remains at one allocation per operation; this change only retires layout state and does
not alter the normal clean-frame cache behavior.

## Phase 1 — UI framework optimization gate

This phase removes avoidable framework work before SIMD is evaluated. A faster numeric kernel must improve the complete
frame path, not only hide unrelated tree overhead.

- [x] Profile layout, reconciliation, drawing, hit testing, focus, and event dispatch separately on representative
  small, medium, and large trees.
- [x] Skip repeated clean self-rebuild subtree walks with a coalesced invalidation generation while preserving
  dirty-producer wakeups.
- [x] Add precise per-subtree dirty tracking for intentionally dirty trees; the current generation is a wake-up guard,
  not a dirty-subtree index.
- [x] Give structural traversal a direct, deduplicated child seam instead of repeatedly combining `event_children` and
  `visit_children`; preserve their distinct event and hit-test semantics.
- [x] Cache layer-sorted child order before considering a spatial index for large sibling groups.
- [x] Add bounds/window culling for hidden flex constraints and declared windows without changing event-child or
  visible-overflow semantics.
- [x] Reuse traversal scratch and layout results; measure allocations and dynamic-dispatch overhead on the hot paths.
- [x] Route large hit-test sibling groups in reverse storage order so pointer dispatch does not allocate a temporary
  child buffer.
- [x] Stop pointer descent at known-bounded subtrees that exclude the pointer; preserve unknown-bound and
  focus/broadcast behavior.
- [x] Add a retained sparse-stack y index after bounded descent proves that the remaining sibling scan is significant;
  preserve exact dense fallback.
- [x] Improve stable Flex paths so unchanged flex children do not trigger avoidable full remeasurement, while preserving
  correct redistribution.
- [x] Reuse the scalar flex share buffer after measuring the expanded layout path; preserve zero-weight and
  unbounded-layout behavior.
- [x] Bound reconstructible GPU image textures by idle lifetime and byte budget; preserve explicit-ID uploads and reload
  source-backed widgets after eviction.
- [x] Replace erased-tree recursive layout invalidation with a generation marker; key every framework layout cache to
  that marker and preserve direct concrete invalidation behavior.
- [x] Re-run end-to-end frame profiles after each framework optimization and document which costs remain significant
  enough to pursue.

## Acceptance criteria

The framework optimization work is complete only when:

- [x] Every adopted optimization has a before/after measurement on a named representative workload.
- [ ] Layout, reconciliation, drawing, hit testing, focus, event dispatch, lifecycle, and retained-state tests remain
  green.
- [ ] Traversal changes preserve the separate child sets required by events, hit testing, focus, and structural
  inspection.
- [ ] No optimization is retained when its end-to-end result is within measured noise unless it reduces a documented
  resource or maintenance cost.
- [ ] The remaining numeric hot paths and their measured frame share are recorded for the SIMD plan.

## Phase 2 — Native scroll-frame optimization

The native website now settles to near-zero work when it is left alone, but a full scroll traversal still produces a
large transient load. The baseline below was collected from `aimer run --target macos` with Activity Monitor filtered to
the main `website` process. Each arrow represents one full native scroll traversal; the samples were taken shortly after
the viewport reached the endpoint.

| Screen      |  CPU range |  GPU range | Activity Monitor Mem range | Real Mem range |
|-------------|-----------:|-----------:|---------------------------:|---------------:|
| Home        | 70.4–78.6% |  0.0–34.7% |             183.4–224.9 MB | 166.5–167.1 MB |
| Blog        | 35.1–73.5% | 12.4–21.2% |             212.2–213.1 MB | 172.7–173.0 MB |
| Latest post | 72.0–83.8% |   4.4–7.0% |             217.0–217.8 MB | 182.4–183.2 MB |

The final latest-post idle sample, after 2.5 seconds without input, was 0.1% CPU, 0.0% GPU, 114.3 MB in the Activity
Monitor Mem column, and 182.7 MB Real Mem. This separates transient scroll/render work from a permanently active idle
loop; the stable Real Mem ranges also do not indicate an unbounded allocation leak in this short test.

The current scroll path requests another animation frame when a scroll step is applied, enters the scroll container's
draw method, resolves content and layout state, and redraws the child tree into a fresh frame command stream. The
existing visible-rectangle culling and layout caches reduce some of that work, but they do not yet make offset-only
scrolling a retained paint operation. The phase therefore optimizes the scroll frame itself, not the already-completed
resize invalidation work from Phase 1.

### Optimization sequence

- [ ] Instrument native scroll frames with frame duration, frames per gesture, visited/drawn nodes, draw-command count,
  text-layout/cache misses, image/texture uploads, and allocations. Separate UI-thread recording from raster/GPU
  presentation where the platform exposes the boundary.

The first native-debug probe now records frame build/encode/present timing, scroll input and moved-frame counts,
drawn-node and draw-command totals, text cache hits/misses, and image draw/upload counts. Allocation counts and
glyph-shaping misses remain pending until the low-overhead frame counters identify whether they are material costs.

- [ ] Coalesce scroll deltas and redraw requests so one pending request produces at most one frame per display tick;
  preserve momentum, gesture phases, overscroll recovery, and final scroll callbacks.

Native wheel input now uses the platform frame-synchronized requester instead of issuing a separate direct redraw. This
lets wheel input share the existing pending-request gate with momentum frames; the existing momentum scroller already
folds accumulated deltas into at most one step per channel per tick. Pointer/touch redraw paths and the native
frame-count effect remain pending measurement.

- [x] Separate offset-only scrolling from layout. Reuse content size, viewport size, and layout results when the
  constraint/tree generation is unchanged; changing the scroll offset must not invalidate unrelated layout state.
  `RawScrollableContainer` now retains a scroll-local layout snapshot keyed by constraints, parent size, scale, viewport
  configuration, and layout/tree generations, deliberately excluding the live scroll offset. A focused regression test
  confirms offset-only frames reuse content measurement while constraint changes retire the snapshot.
- [x] Push known-bounds viewport culling ahead of dynamic draw dispatch so complete off-screen subtrees are skipped;
  preserve event-child, hit-test, focus, structural-inspection, and unknown-bounds semantics. Clipping boundaries now
  reject off-screen child dispatch through `BuildContext::is_rect_visible`; non-clipping/unknown-bound paths remain
  conservative.
- [x] Reuse or retain draw-command storage for static subtrees and apply scroll translation/clip without rebuilding
  unchanged content; invalidate retained paint data on structural, style, text, image, and scale changes.
- [x] Optimize text and image work only where the frame counters show misses: prefetch near-viewport content, reuse
  stable texture IDs, and keep decoded/GPU caches bounded without evicting visible content.
- [x] Re-run the home, blog, and latest-post traversal matrix in native debug and release profiles, including settled
  idle samples, and record CPU, GPU, and memory changes.
- [ ] Add frame-time and allocation captures to the same traversal matrix so the dominant active-scroll cost can be
  named against the display budget.

### Native CPU/GPU/memory acceptance matrix — 2026-08-27

The matrix was run from `website` with `aimer run --target macos --no-tui` and
`aimer run --target macos --release --no-tui`. Each screen used top-to-bottom, bottom-to-top, top-to-bottom, and
bottom-to-top traversals. Activity Monitor
was sampled against the exact packaged `website` PID while scroll requests were active; the post-pass column is the
sample taken after the helper's five-second wait. The ranges below summarize the four traversals for each screen.

| Profile | Screen      | Peak active CPU | Peak active GPU | Peak Activity Monitor Mem |  Peak Real Mem | Post-pass sample (CPU / GPU / Activity Mem / Real Mem) |
|---------|-------------|----------------:|----------------:|--------------------------:|---------------:|-------------------------------------------------------:|
| Debug   | Home        |      64.8–82.4% |       0.0–12.9% |            182.6–193.6 MB | 130.7–137.7 MB |      0.0% / 4.0–6.5% / 82.8–95.8 MB / 130.5–131.7 MB |
| Debug   | Blog        |      70.9–72.9% |      10.3–26.8% |            181.0–183.3 MB | 140.4–140.8 MB |         0.0% / 5.9–8.0% / 83.3–83.8 MB / 140.4–140.7 MB |
| Debug   | Latest post |      71.6–82.8% |      21.4–38.5% |            181.4–182.1 MB | 149.8–150.6 MB | 0.0–77.9% / 9.0–37.5% / 86.7–141.3 MB / 150.1–150.3 MB |
| Release | Home        |           0.0%* |           0.0%* |                   71.1 MB |       110.2 MB |                    0.0% / 0.0% / 71.1 MB / 110.2 MB |
| Release | Blog        |           0.0%* |           0.0%* |                   71.1 MB |       110.2 MB |                    0.0% / 0.0% / 71.1 MB / 110.2 MB |
| Release | Latest post |           0.0%* |           0.0%* |                   71.1 MB |       110.2 MB |                    0.0% / 0.0% / 71.1 MB / 110.2 MB |

The latest-post first and third downward passes still had deferred work at the five-second sample. An additional six
one-second idle samples then stabilized at 0.0–0.1% CPU, 0.0% GPU, 78.8 MB Activity Monitor Mem, and 150.2 MB Real Mem.
The repeated traversal captures therefore show quiescent settled behavior and no short-run unbounded memory growth,
while debug active scrolling still has a substantial transient CPU/GPU cost. Release CPU/GPU values marked `*`
were 0.0 at Activity Monitor's displayed sampling resolution; they should not be interpreted as literal zero work.

This closes the Phase 2 CPU/GPU/memory measurement item. Frame time, allocation counts, and a causal before/after
determination for the dominant active-scroll cost remain open until the low-overhead counters are captured.

The relevant native validation passed with
`cargo test -p aimer_scroll -p aimer_assets -p aimer_cupid -p aimer_widget -p aimer_quiver` (including the
retained-command, cache-bound, culling, and frame-stat regressions). A separate `cargo test --workspace --all-features`
attempt remains blocked by 36 portable-state trait errors in `jaime`; that failure is outside this native acceptance
capture.

### Fifteenth adopted optimization — cull clipped subtrees before child dispatch

`BuildContext::is_rect_visible` now provides one allocation-free, edge-inclusive intersection check for local known
rectangles. Missing or invalid viewport/bounds data remains conservative and visible. Existing flex and wrapped-flex
culling use the helper, while clipping boundaries now use it before entering their erased child: `Container` tests its
full inner clip (not only the child's nominal content bounds), `Resizable` tests its clip, and `Scrollable` tests its
viewport. This skips complete off-screen child subtrees without changing their physics, bounds, or retained state.

The focused native-free regressions record the work removed at the dispatch boundary: an off-screen unknown-bounds child
receives `0` draw calls instead of `1`, while the same child receives `1` when its clipping parent is visible.
Structural/event/hit-test child views remain available in the off-screen container test, and no-viewport/invalid-bound
inputs stay visible by contract. Native CPU/GPU traversal measurements remain part of the final Phase 2 matrix.

### Sixteenth adopted optimization — retain static scroll paint and rebase it at draw time

Native scroll containers now opt into a retained local draw stream only when the child explicitly reports that its paint
is stable and side-effect-free. The first eligible draw records the child once into a private canvas, snapshots the
supported draw commands, and replays that snapshot into subsequent frame lists. Replay rebases local transforms against
the current scroll transform, so offset changes update translation and clipping without rebuilding or re-layouting the
unchanged child. The retained stream keeps large text/image/SVG payloads shared through their existing reference-counted
handles; only the small command records are copied into the recycled frame list.

The cache key includes the scroll layout snapshot, child subtree generation, rebuild generation, texture-cache epoch,
and scale. Structural, style, text, image/upload, eviction, and scale changes therefore retire stale paint before
replay. Dynamic, interactive, async, rich-text, image-loading, custom-command, and otherwise unsupported subtrees retain
the conservative direct-draw fallback. A focused native-free regression verifies that offset-only drawing records once
while a scale change records again; the existing culling and event/focus paths are unchanged.

### Seventeenth adopted optimization — make text/image preparation miss-driven and bounded

The scroll cache extent now supplies a directional near-viewport region (with a bounded lead and a short retention
window)
to text/image preparation. Visible requests remain mandatory, while offscreen text preparation is submitted only when
the layout cache actually misses; a hit-only offscreen request does not spend shaping/layout work again. This keeps
prefetch useful for content approaching the viewport without turning every scroll frame into speculative text work.

Source-backed decoded image entries now use access stamps, a 64 MiB per-source-cache byte budget, a 512-entry bound, and
a cold-access threshold. Only cold, non-loading entries are eligible for removal; visible/loading work is protected, and
an active working set may temporarily exceed the byte target until a safe candidate exists. The native GPU cache
continues to bound reconstructible textures by byte budget and idle lifetime while protecting current draw references
and explicit-ID textures. Reused image IDs remain stable, and replacing an explicit-ID payload advances the texture
epoch so retained paint cannot replay stale pixels. Focused cache tests cover cold eviction, visible/loading protection,
and access-metadata cleanup.

### Phase 2 acceptance gate

This phase is complete only when:

- [ ] Scroll frame time is measured against the display refresh budget and the dominant UI/raster/GPU cost is named.
- [ ] The selected optimization reduces the measured dominant cost without changing scroll, event, focus, or culling
  behavior.
- [x] Repeated top-to-bottom and bottom-to-top traversals do not show unbounded memory growth, and visible images/text
  remain correct after cache reuse or eviction.
- [x] Native debug and release results, settled idle usage, focused tests, and relevant workspace tests are recorded.

## Phase 3 — Compositor-style retained scroll layers

Phase 2 removed unnecessary layout and paint preparation, but an active native scroll can still replay and encode a large
retained command stream. The next step is to make offset-only scrolling a compositing operation: stable content is
rasterized once into a bounded GPU layer or tile set, then moved with a transform and viewport clip while the offset
changes. This phase must preserve the conservative direct-render fallback for dynamic, interactive, loading, animated, and
otherwise ineligible subtrees.

### Phase 3 goals

- Make the common static-scroll frame submit only the visible retained layer/tile draws plus the scroll transform and clip.
- Keep layout, reconciliation, text shaping, image decoding, and paint recording out of offset-only frames.
- Keep memory bounded with visible-tile protection, a small near-viewport lead, and idle reclamation.
- Preserve event-child, hit-test, focus, lifecycle, accessibility/structural inspection, nested-scroll, and scale-change
  semantics independently of the retained visual layer.

### Optimization sequence

- [x] Capture a Phase 3 active-scroll baseline with per-frame build/encode/present time, draw-command count, allocations,
  dropped-frame budget checks, and memory. The reproducible native headless runner covers 60 Hz and 120 Hz budgets; the
  real-surface Activity Monitor pass supplies sampled CPU/GPU/resident-memory ranges.
- [x] Define retained-layer eligibility and ownership. Reuse the Phase 2 stability and generation keys, and exclude
  interactive, animated, async/loading, custom-command, rich-text, and unknown-side-effect subtrees from the layer path.
- [x] Add a bounded offscreen render target for eligible static scroll content. Small content uses one renderer-owned
  layer capped at 8,192 px per side and 64 MiB; larger content uses a viewport-plus-lead tile set instead of one
  oversized texture. Tiles are 1,024 px and selection is capped at 64 tiles per frame.
- [x] On offset-only frames, update only the retained layer/tile transforms, viewport clip, and layer/tile selection. Do
  not rebuild or copy the subtree's individual draw commands into the frame list.
- [x] Support dirty tile updates for structural, style, text, image, upload, scale, and device changes. Known local paint
  invalidations retain element identities per tile and re-rasterize only affected tiles; structural/tree-generation,
  geometry, scale, upload/texture, device, and unknown-producer changes conservatively retire the complete tile set.
- [x] Composite dynamic islands separately above the retained layer, preserving their normal event and draw paths. The
  implementation is deliberately narrow: an eager, non-wrapped flex with a stable prefix and dynamic suffix can retain
  the prefix in one compositor-safe layer; dynamic, interleaved, windowed, oversized, clipped/effectful, and otherwise
  unsupported subtrees fall back to direct drawing.
- [x] Reuse existing texture IDs and bounded GPU-cache policy for retained layers/tiles. Protect visible/in-flight layers,
  reclaim cold layers after the scroll settles, and avoid per-frame allocation churn in layer/tile selection or transform
  setup.
- [x] Coalesce scroll input and frame requests around the display tick. Native frame-ready requests use a pending gate;
  headless redraw requests use the same one-bit coalescing contract, and both report accepted/coalesced/display-tick
  counts. The frame source is not kept alive by an already-pending wake-up.
- [x] Re-run the Home, Blog, and latest-post traversal matrix in native debug and release profiles. Record active and
  settled CPU/GPU/memory, frame time, draw count, allocation count, tile-cache growth, and visual correctness where the
  selected surface exposes those counters; retain the direct-path result for ineligible production content.

### Eighteenth adopted optimization — dynamic islands and display-tick frame coalescing

`RetainedChildElement` now forwards the retained-paint contract instead of hiding the child behind the builder proxy.
`RawFlex` exposes a conservative stable-prefix/dynamic-suffix partition for eager, non-wrapped children. The scroll owner
records and replays the stable prefix, then draws the live suffix through the ordinary element path, so dynamic state,
input, event geometry, and lifecycle behavior remain owned by the live tree. If the partition cannot prove ordering or
compositor safety, the whole child uses the existing direct fallback.

Frame-ready scheduling now has an explicit one-pending-request gate. A second request before delivery increments the
coalesced counter instead of adding another wake-up, and delivery increments the display-tick counter. The headless
window uses the same contract for deterministic tests. These counters describe scheduling pressure, not a promise that
each accepted request produces a distinct rendered frame.

### Phase 3 native acceptance measurements — 2026-08-27

The reproducible CPU-side acceptance command is:

```text
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-phase3-target cargo run -p aimer_laboratory \
  --example native_phase3_acceptance --features aimer/frame-stats
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-phase3-target cargo run -p aimer_laboratory \
  --example native_phase3_acceptance --release --features aimer/frame-stats
```

The runner performs four directions (`down`, `up`, `down`, `up`) with five wheel steps per direction. Active timing
includes event delivery plus native headless build/paint. `wall_us` is p50/p95; all workloads reported zero samples over
both the 60 Hz (16,666.667 µs) and 120 Hz (8,333.333 µs) budgets. Every workload reported `requests=21/19/20`
(`accepted/coalesced/display_ticks`) and settled with no pending redraw.

| Profile / workload | Active wall p50/p95 µs | Build ms | Commands / nodes / retained | Allocs / frame |
| --- | ---: | ---: | ---: | ---: |
| Debug Home / static | 69.92 / 90.08 | 0.066 | 239 / 43 / 0 | 5.25 |
| Debug Blog / static | 48.50 / 66.08 | 0.045 | 239 / 43 / 0 | 5.25 |
| Debug Latest / dynamic islands | 27.46 / 37.67 | 0.025 | 44 / 9 / 1 | 6.90 |
| Release Home / static | 5.88 / 31.96 | 0.008 | 239 / 43 / 0 | 5.25 |
| Release Blog / static | 5.21 / 42.62 | 0.007 | 239 / 43 / 0 | 5.25 |
| Release Latest / dynamic islands | 2.71 / 7.92 | 0.003 | 44 / 9 / 1 | 6.90 |

The current main-based rerun keeps the dynamic synthetic workload at one retained layer, but the all-static Home and
Blog probes report zero retained layers and replay 239 commands across 43 visited nodes. This is because the current
`RawFlex` contract deliberately rejects whole-tree stability and its island partition requires a dynamic suffix; the
static-only retained-layer gate therefore remains open. The focused regression still verifies that a stable prefix is
painted once while a dynamic suffix is painted again after an offset-only draw.

The final local rerun reported zero over-budget samples at both refresh budgets, `requests=21/19/20` for every active
workload, and `pending=false` after settling. The headless runner exposed RSS in this environment: active runs grew by
at most 400 KiB in debug and 224 KiB in release, while settled runs were flat or within 16 KiB. This acceptance fixture
uses probe rectangles rather than text, so its `text misses/frame=0` counters are not a text-workload measurement.
The text-specific safety path is covered separately: plain `RawTextWidget` paragraphs are retained through
`LayoutCache`, while rich/selectable/link-bearing and shadowed text remain on direct dynamic drawing paths.
The main-based text scroll profile measured 53.83/60.67 us (p50/p95) debug and 10.75/12.79 us release, with 206
commands, 35 nodes, zero retained layers, 15 text commands, and zero text-cache misses per frame. The release Cupid
pipeline benchmarks measured 112.324 us average and 157.5 us worst for warmed four-times-viewport scrolling; mixed-script
warm cache hits averaged 297 ns, while cold repeated preparation averaged 928.488 us with 389.603 us shaping,
29.526 us layout, 143.620 us glyph work, and 60.469 us instance building.

The real macOS website was also launched with `aimer run -d macos --no-tui` and `--release --no-tui`. Activity Monitor
was filtered to `website` while each screen followed the same four-direction traversal. These are sampled ranges across
the four passes, not frame-time measurements:

| Profile / screen | CPU average range / peak | GPU average range / peak | Activity-Monitor memory | Resident memory |
| --- | ---: | ---: | ---: | ---: |
| Debug Home | 34.0–64.3% / 70.8% | 3.7–29.2% / 30.9% | 78.4–188.6 MB | 138.4–139.0 MB |
| Debug Blog | 45.6–56.8% / 63.8% | 2.8–18.4% / 21.0% | 174.1–176.1 MB | 146.5–146.6 MB |
| Debug Latest | 43.4–68.8% / 75.3% | 8.4–27.4% / 28.4% | 80.8–178.3 MB | 153.5–156.3 MB |
| Release Home | 35.8–61.6% / 67.1% | 8.7–25.8% / 27.0% | 85.3–223.8 MB | 134.9–164.2 MB |
| Release Blog | 37.0–53.9% / 83.7% | 4.4–19.2% / 20.9% | 116.9–215.7 MB | 175.3–176.8 MB |
| Release Latest | 46.9–62.0% / 84.2% | 7.3–24.7% / 26.9% | 121.6–219.6 MB | 183.2–185.8 MB |

The real website remains intentionally conservative: its mixed interactive, async, image, and Markdown content did not
qualify for the current one-layer dynamic-island partition. The debug frame report consequently observed
`build=0.21 ms`, `encode=5.22 ms`, `present=0.07 ms`, `nodes/frame=139`, `commands/frame=721`, and
`retained-layers/frame=0`. This confirms why Activity Monitor still shows meaningful active-scroll CPU/GPU: the production
surface is still submitting the direct command stream. The headless runner cannot expose actual GPU encode/present or
RSS (`ps` is unavailable in the sandbox), so those real-surface costs remain a follow-up measurement target.

### Phase 3 acceptance gate

This phase is complete only when:

- [ ] Static offset-only scrolling no longer replays the full retained command stream on the representative production
  website. The synthetic native workload proves the retained path; the real website currently reports zero retained layers
  and therefore needs a broader safe partition/annotation before this gate can close.
- [x] The measured headless active frames stay within both 60 Hz and 120 Hz budgets with zero over-budget samples. The
  real debug frame report also stayed below the 60 Hz encode budget; Activity Monitor percentages are recorded separately.
- [x] Dynamic and unsupported subtrees retain correct layout, scroll, event, hit-test, focus, lifecycle, and visual
  behavior through the direct-render fallback, covered by focused dynamic-island and existing scroll tests.
- [x] Retained layers invalidate correctly after structural/style/text/image/scale/device changes and never show stale
  pixels after cache reuse or eviction, covered by the retained-paint and dirty-tile tests.
- [ ] Retained tile memory and allocations remain bounded across the real repeated matrix. Resident memory was nearly flat
  in the Activity Monitor samples, but the headless sandbox could not collect RSS or a GPU-cache byte trace.
- [x] The settled headless app requests no continuing redraw (`pending=false`); debug/release native and headless results,
  focused tests, and relevant crate tests are recorded above.

Final focused validation in the optimization worktree passed `cargo test -p aimer_widget -p aimer_text -p aimer_scroll
--lib` (272, 213, and 177 tests respectively), the retained paint-island regression, and
`cargo check -p aimer_container --lib`; `git diff --check` is clean. The workspace-wide `--all-features` run remains
blocked by the repository's pre-existing 60 `jaime` portable-state trait errors and missing `aimer_legacy` benchmark
packages; the container unit-test command also encounters the existing dev-dependency feature-unification `BuildCanvas`
mismatch in `aimer_scroll`.
