# SIMD calculation plan

## Decision

Use SIMD selectively inside numeric kernels. Do not make the `Widget`,
`Element`, reconciliation, or event interfaces SIMD-aware.

The framework contains two different kinds of work:

- Data-parallel numeric work, where SIMD may help: sizes, offsets, weights,
  interpolation, geometry, colors, and image data.
- Irregular tree work, where SIMD is usually the wrong abstraction: dynamic
  child measurement, trait-object dispatch, retained-child traversal, state,
  input, focus, routing, providers, and text shaping.

The scalar implementation remains the correctness baseline for every target.
An optimized kernel is adopted only after a benchmark proves that it improves a
real frame path.

## Goals

- [x] Identify the numeric kernels that consume enough data to justify SIMD.
- [x] Preserve the existing public widget and element interfaces.
- [x] Keep scalar fallbacks for unsupported CPU and WebAssembly targets.
- [x] Preserve layout behavior, ordering, precision, and invalid-input handling.
- [x] Measure CPU time, allocations, frame time, and code-size impact before and
      after each optimization.

## Non-goals

- [ ] SIMD-ify every loop in the framework.
- [ ] Replace tree traversal or dynamic widget dispatch with a vector API.
- [ ] Use `target-cpu=native` for distributable binaries.
- [ ] Add unsafe architecture intrinsics before a scalar benchmark establishes a
      meaningful target.
- [ ] Move GPU work onto the CPU; the renderer already delegates parallel work
      to GPU pipelines where appropriate.

## Candidate areas

### 1. Flex layout

Primary target: the pure numeric part of `aimer_flex`.

#### Existing optimization baseline

Before evaluating SIMD, measure the algorithmic optimizations that the current
Flex implementation already provides:

- **Virtualized windows:** `FlexList` materializes and draws only the visible
  child range instead of building the entire list.
- **Declared item extent:** when every item extent is supplied, the list can
  calculate its total size arithmetically without measuring any child first.
- **Estimated item extent:** when no extent is supplied, the list can measure
  one visible child, predict the remaining extent, and lazily correct rows as
  they become visible.
- **Uniform stride storage:** when children resolve to the same size, the
  layout stores one size and a stride, then computes each offset as
  `index * stride` instead of storing every offset.
- **Visible-range lookup:** uniform layouts locate the visible range with
  arithmetic; varying layouts use binary search over their offsets.
- **Conditional work:** flex-weight storage is allocated only when flex
  children exist, and layer sorting runs only when a child requests a
  non-default layer.
- **Layout reuse:** the measured layout is shared by the size and draw passes
  and cached while its constraints and scale remain unchanged.

These are algorithmic optimizations, not SIMD. They are the baseline for every
benchmark, and they often matter more than vectorizing the final weight-share
calculation. In particular, a container with `Expanded` children currently
needs conservative full remeasurement because each child's share depends on
the other flex children; profile that cost before optimizing the arithmetic
kernel.

- [x] Benchmark `distribute_flex_space` in
      [flex_child.rs](crates/aimer_flex/src/flex/flex_child.rs), including zero,
      negative, sparse, uniform, and large weight arrays.
- [x] Benchmark size scanning and uniform detection in
      [flex_layout.rs](crates/aimer_flex/src/flex/flex_layout.rs).
- [x] Separate child measurement from numeric table construction. Calls to
      `child.computed_size(...)` remain scalar and dynamic.
- [ ] Consider structure-of-arrays storage for main-axis sizes, cross-axis
      sizes, and flex weights if profiling shows that `ResolvedSize`'s current
      array-of-structures layout limits vectorization.
- [ ] Keep prefix-offset accumulation scalar unless a benchmark shows that a
      tiled/prefix-scan implementation pays for its complexity.
- [ ] Treat wrapping in [wrap_layout.rs](crates/aimer_flex/src/flex/wrap_layout.rs)
      as branch-heavy and dependency-heavy; optimize it only after simpler
      kernels are measured.

### First scalar data-flow result — reuse flex share storage

The first measured flex seam is now isolated behind the private
`distribute_flex_space_in_place` helper. `FlexLayout::build` reuses its existing
weight table for the computed shares instead of allocating a second `Vec`.
Negative entries remain non-flex sentinels, and negative zero marks a
zero-weight flex child so the unbounded and finite-constraint paths preserve
their previous sizing behavior. This is scalar preparation for a possible
kernel; it does not add SIMD intrinsics or change a public API.

The ignored direct profile covers zero, sparse, uniform, and weighted arrays.
The current release scalar reference results are:

| Input | p50 | p95 |
| --- | ---: | ---: |
| Zero weights, 32 entries | 0.138 us | 0.358 us |
| Sparse weights, 256 entries | 0.954 us | 1.110 us |
| Uniform weights, 2,048 entries | 11.609 us | 13.630 us |
| Weighted weights, 2,048 entries | 9.274 us | 9.550 us |

The end-to-end release fixture adds mixed regular and `Expanded` children so
the allocation decision is measured through real layout construction:

| Workload | Before p50/p95 | After p50/p95 | Before allocs/op | After allocs/op |
| --- | ---: | ---: | ---: | ---: |
| Expanded column, 256 rows, cold frame | 51.54 / 55.12 us | 50.29 / 52.50 us | 49.43 | 47.43 |
| Expanded column, 256 rows, resize | 20.29 / 21.12 us | 20.84 / 21.17 us | 14.00 | 12.00 |
| Expanded column, 2,048 rows, cold frame | 160.21 / 168.71 us | 149.75 / 170.08 us | 215.00 | 213.00 |
| Expanded column, 2,048 rows, resize | 85.98 / 117.13 us | 84.78 / 85.63 us | 14.00 | 12.00 |

The 2,048-row cold p50 improves by 6.5% and every expanded workload removes
two allocations per operation, so the scalar change passes the resource gate
even where CPU timing is within noise. The next SIMD decision should compare
the remaining size-table scan and total-weight reduction against this
end-to-end cost before introducing an explicit vector kernel.

### Size-table follow-up — measured but not retained

The ignored `profile_size_table_construction` profile now measures the compact
uniform-stride path and the varying full-offset path without child or canvas
work. A candidate fused uniformity detection with offset construction after
the first disagreement. Its direct release p50 for a varying 2,048-entry
table fell from 17.092 us to 12.171 us, and its debug p50 fell from 57.611 us
to 36.649 us.

The complete layout workload did not meet the framework retention gate, and it
did not change allocations:

| Workload | Before p50/p95 | Candidate p50/p95 | allocations/op |
| --- | ---: | ---: | ---: |
| Varying size column, 2,048 rows, release cold frame | 57.46 / 64.88 us | 54.00 / 65.42 us | 21.00 / 21.00 |
| Varying size column, 2,048 rows, release resize | 40.47 / 41.16 us | 36.96 / 38.49 us | 15.00 / 15.00 |
| Varying size column, 2,048 rows, debug cold frame | 565.71 / 576.50 us | 550.71 / 566.71 us | 21.00 / 21.00 |
| Varying size column, 2,048 rows, debug resize | 392.50 / 394.00 us | 387.53 / 391.89 us | 15.00 / 15.00 |

The release p50 gains are below the 10% end-to-end threshold and are not
consistent across uniform tables or p95 measurements, so the production
refactor was rejected. The profile and direct-size fixtures remain as the
baseline for a future larger-table workload; explicit SIMD is not justified by
this candidate alone.

### Child measurement boundary — scalar data flow retained

`FlexLayout::build` now makes the dynamic/numeric boundary explicit without
adding another numeric allocation or changing the layout algorithm:

- `measure_children` calls `child.computed_size(...)` for regular children and
  records flex factors in the existing contiguous table;
- `resolve_flex_space` receives only scalar extents and the flex table, then
  resolves shares in place without borrowing a child;
- `measure_flex_children` applies those shares and keeps the remaining dynamic
  `child.computed_size(...)` calls scalar;
- `from_sizes` consumes the resulting `Vec<ResolvedSize>` as a pure numeric
  table-construction step.

The `aimer_flex` regression suite remains green after the split (82 passed, 2
ignored). This is SIMD preparation and a data-flow refactor, not a claimed
performance improvement; the existing numeric profiles remain the baseline for
the next contiguous-buffer decision.

### 2. Scroll and animation math

- [x] Profile scroll physics and overscroll calculations in
      `crates/aimer_scroll/src/scrollable/physics/`.
- [x] Profile curve evaluation and scalar interpolation in
      `crates/aimer_animation/src/primitives/`.
- [x] Profile keyframe lookup and collection-level animation ticks.
- [x] Evaluate collection-level batching; keep stateful controller/event
      transitions scalar until an existing caller justifies a batch API.
- [x] Preserve the framework's current floating-point tolerances and frame
      scheduling behavior while changing the lookup implementation.

### Scroll and animation result — scalar paths retained

The ignored profiles measure spring integration, the bounded drag-velocity
history, curve transforms, and scalar interpolation without widget or event
dispatch work. Release results are:

| Kernel | Representative input | p50 | p95 |
| --- | --- | ---: | ---: |
| Spring integration | 120 Hz frame | 0.018 us | 0.018 us |
| Spring integration | 50 ms stutter frame | 0.070 us | 0.073 us |
| Velocity history | 1 sample | 0.006 us | 0.006 us |
| Velocity history | 24 samples | 0.099 us | 0.101 us |
| Curve transform | cubic-bezier | 0.139 us | 0.143 us |
| Curve transform | FastOutSlowIn | 0.140 us | 0.147 us |
| Scalar interpolation | `f32` | 0.004 us | 0.004 us |
| Scalar interpolation | four-component tuple | 0.006 us | 0.009 us |

The debug profile is also sub-microsecond for every measured operation: the
50 ms spring frame is 0.113 us p50, the 24-sample history query is 0.569 us,
and cubic-bezier evaluation is 0.266 us. The existing native end-to-end
fixture reports 17.80 us p50 for a 2,048-row scroll frame and 0.60 us for the
one-child animation shell in release; the corresponding debug values are
239.90 us and 6.85 us.

These loops do not expose a useful SIMD seam. Spring integration is a
state-dependent recurrence with at most six sub-steps per frame, velocity
history is a chronological ring of at most 24 samples, and a controller
evaluates one curve and one value per tick. Vectorizing them would require a
new batch API and would add complexity at a call site whose measured cost is
already negligible. The scalar implementations therefore remain the
cross-target reference; no production change was retained from this profile.

### Keyframe and collection animation result — binary lookup, scalar ticks

`KeyframeAnimation::at` originally scanned every adjacent pair from the start
of the table. It now uses a lower-bound search over the already sorted fractions.
The early endpoint checks remain unchanged; a dedicated test also preserves the
old first-interval behavior for duplicate fractions and the last-value fallback
for `NaN` progress.

The direct lookup profile reports p50 / p95 microseconds per sample:

| Keyframes | Debug before | Debug after | Release before | Release after |
| ---: | ---: | ---: | ---: | ---: |
| 2 | 0.112 / 0.115 | 0.046 / 0.047 | 0.006 / 0.006 | 0.010 / 0.014 |
| 8 | 0.214 / 0.227 | 0.067 / 0.078 | 0.008 / 0.013 | 0.014 / 0.015 |
| 32 | 0.412 / 0.481 | 0.104 / 0.108 | 0.020 / 0.021 | 0.020 / 0.020 |
| 256 | 1.667 / 2.254 | 0.145 / 0.148 | 0.101 / 0.104 | 0.027 / 0.030 |
| 2,048 | 11.654 / 11.777 | 0.177 / 0.187 | 0.621 / 0.759 | 0.037 / 0.042 |

`ParallelAnimation::tick` and `StaggeredAnimation::tick` were profiled at 1,
8, 64, and 256 controllers. At 256 controllers, the release p50 / p95 is
1.620 / 1.879 us for parallel ticks and 1.280 / 1.420 us for staggered ticks;
the debug values are 8.059 / 8.068 us and 10.166 / 10.296 us respectively.
Both methods advance each controller through a stateful `LocalCell` and collect
a new result vector on every call. The repository has no production call site
for these collection APIs, so adding a reuse-output API would expand the public
surface without an end-to-end consumer, and controller state transitions do not
form an independent SIMD batch. The collection paths therefore remain scalar
and unchanged.

### 3. Geometry, colors, and CPU-side render preparation

- [x] Profile representative repeated matrix, rectangle, transform, color, and
      clip math in the CPU rect-instance preparation path; inspect the related
      `aimer_attribute`, `aimer_color`, `aimer_space`, and `aimer_cupid` types
      for independent batches.
- [x] Look for batches of plain numeric values before introducing a SIMD type.
- [x] Keep canvas/FFI calls and GPU command submission scalar and ordered.
- [x] Validate that no SIMD representation crosses the GPU/FFI seam; existing
      instance sizes, alignment, ownership, and thread-affinity contracts are
      unchanged.

### Geometry/render-preparation result — cache transform scales

The ignored `profile_rect_render_preparation` profile exercises the private
CPU path that turns a fill command into one or two complete `RectInstance`
values. It includes transformed corners, per-axis scale lengths, borders,
outlines, clip packing, color packing, and alpha application. The original
helper recomputed the same two scale lengths for every rectangle even though
the active transform changes only when a transform command is encountered.

`Renderer::render` now caches those scale lengths beside the current transform
and reuses them for fill rectangles, clips, text decorations, and shadows. The
change is scalar, keeps the GPU instance representation and public APIs intact,
and does not change allocations:

| Workload | Debug before | Debug after | Release before | Release after |
| --- | ---: | ---: | ---: | ---: |
| Identity, unclipped, 256 rects | 0.303 / 0.343 us | 0.240 / 0.242 us | 0.018 / 0.019 us | 0.017 / 0.017 us |
| Scaled, clipped, 1,024 rects | 0.317 / 0.435 us | 0.246 / 0.248 us | 0.018 / 0.019 us | 0.017 / 0.018 us |
| Rotated, outlined, 2,048 rects | 0.513 / 0.519 us | 0.365 / 0.368 us | 0.021 / 0.025 us | 0.019 / 0.023 us |

Values are p50 / p95 microseconds per prepared rectangle. The direct release
gain is 6–10% and the debug gain is 21–29%, while the larger transformed case
also removes two square-root calculations from every rectangle. The supporting
attribute and space types are individual geometry values, not existing
contiguous batches, and color storage is already one packed word. No explicit
SIMD kernel is justified here; the cached scalar data flow is retained, and
GPU/FFI submission remains ordered and scalar.

### 4. Image and bulk data operations

- [x] Profile image conversion, blending, and pixel-format operations before
      adding a kernel.
- [x] Prefer existing optimized image/GPU paths when they already dominate the
      operation.
- [x] Provide scalar handling for tails, empty buffers, unaligned data, and
      unsupported formats.

### Image and bulk-data result — retain existing conversion and upload paths

The native image path accepts validated RGBA8 data. An in-limit image returns
a borrowed `Cow`, so it does not copy pixels before the GPU upload. An oversized
image is copied once into the `image` crate's `RgbaImage` and resized with its
Lanczos3 implementation. The wasm fallback uses the local scalar nearest-neighbor
loop; its odd-dimension case is profiled as well.

The ignored `profile_image_bulk_operations` profile reports p50 / p95
microseconds per call in isolated single-test runs:

| Operation | Debug | Release |
| --- | ---: | ---: |
| Invalid/empty placeholder | 0.055 / 0.057 | 0.019 / 0.021 |
| Borrowed 256×256 RGBA8 | 0.051 / 0.051 | 0.004 / 0.004 |
| Lanczos3 256×128 → 128×64 | 15,172 / 15,364 | 231 / 235 |
| Lanczos3 512×256 → 128×64 | 48,510 / 51,050 | 648 / 650 |
| Lanczos3 1024×512 → 256×128 | 201,253 / 202,905 | 2,596 / 2,637 |
| Scalar nearest, 513×257 → 129×65 | 133 / 134 | 5.5 / 5.8 |

The resize path is the only measured image conversion that reaches a
millisecond-scale cost. It already delegates to a maintained image library,
and replacing Lanczos3 with a private SIMD filter would change quality while
optimizing a path that is not part of ordinary in-limit rendering. The nearest
fallback is compiled for wasm (and tested on native); its output buffer,
per-row source mapping, and scalar pixel tails are small and self-contained.

Blending is not a CPU bulk operation: images use `Rgba8Unorm` textures and
`PREMULTIPLIED_ALPHA_BLENDING` in the GPU render pipeline. The public image
upload boundary is RGBA8-only, so there is no independent CPU pixel-format
conversion batch to vectorize. Native file and asset decoding is already moved
off the render thread; the render-thread work is validation, optional resize,
command capture, and the non-blocking GPU upload.

The bulk profiles cover both the per-frame instance gate and image command
capture. `FrameUpload` uses contiguous slice comparison and
`extend_from_slice`; `DrawList` owns the borrowed input by copying it once into
the retained command. Their representative p50 / p95 results are:

| Operation | Debug | Release |
| --- | ---: | ---: |
| `FrameUpload` compare, 256 KiB | 4.99 / 5.76 | 4.20 / 5.35 |
| `FrameUpload` copy, 256 KiB | 3.19 / 3.20 | 2.80 / 3.43 |
| `FrameUpload` compare, 4 MiB | 76.15 / 77.61 | 73.29 / 74.63 |
| `FrameUpload` copy, 4 MiB | 50.81 / 51.48 | 49.23 / 53.44 |

The image command profile also found a correctness and data-flow issue:
`DefaultHashBuilder::default()` creates a new random seed for every hasher, so
identical calls to `load_image` previously received different texture IDs.
The ID now uses one process-stable builder, and identical hashed loads already
queued in the same draw list share the first owned byte buffer. Explicit-ID
loads still enqueue every call because they are the update path.

For two identical 4 MiB hashed loads made before either draw, p50 / p95 was:

| Workload | Debug before | Debug after | Release before | Release after |
| --- | ---: | ---: | ---: | ---: |
| Duplicate image command capture | 461.06 / 487.06 | 224.88 / 233.17 | 752.21 / 894.27 | 395.31 / 462.38 |

The p50 copy reduction is about 51% in debug and 47% in release, while the
behavioral fix also lets repeated frames reuse the same GPU texture ID. These
are compiler/library bulk operations rather than framework numeric kernels.
The image-command copy remains a one-time ownership transfer, not the
per-frame `DrawImage` operation. No explicit SIMD kernel is retained for this
area; the scalar validation, deterministic ID, deduplication, and GPU paths
remain the cross-target baseline.

Phase 3 is complete without adding a second SIMD kernel. The scroll,
animation, geometry, color, and image profiles either expose too little
independent work or already delegate their bulk work to maintained libraries
and GPU pipelines. Canvas/FFI submission stays scalar and ordered; no SIMD
buffer crosses that boundary, so the existing instance sizes, alignments,
ownership, and thread-affinity contracts remain unchanged.

## Kernel design

Keep a small private seam between framework policy and numeric implementation:

```text
Widget/Element policy
        |
        v
validated, contiguous numeric inputs
        |
        +--> scalar kernel
        |
        +--> optional SIMD kernel
        |
        v
validated numeric result
```

- [x] Keep kernel inputs simple and contiguous: slices, lengths, strides, and
      plain numeric values.
- [x] Make kernels deterministic and independently testable without a window,
      canvas, or widget tree.
- [x] Prefer compiler auto-vectorization first. The release profile already
      enables optimization and LTO.
- [x] If explicit SIMD is justified, add target-specific implementations behind
      a private dispatch layer and retain the scalar implementation.
- [x] Use runtime feature detection or compile-time target selection only where
      the deployment target guarantees the required instructions.
- [x] Keep `unsafe` limited to the smallest intrinsic wrapper and document its
      alignment, bounds, and target-feature invariants.

## Target support

- [x] Native desktop: provide a scalar baseline and optional SSE2/AVX2 or NEON
      paths where supported by the deployment policy.
- [ ] Android/iOS: validate the selected ARM SIMD path on the minimum supported
      devices; do not assume desktop CPU features.
- [x] WebAssembly: keep a non-SIMD build and an explicitly opted-in `simd128`
      build. Do not make SIMD a requirement for browsers that lack support.
- [x] Avoid making `portable_simd` a default dependency while it remains an
      unstable Rust API.

### Phase 4 result — target dispatch and fallback matrix

Phase 4's source rollout and compile-time matrix are complete. The private dispatch in
[`flex_child.rs`](../crates/aimer_flex/src/flex/flex_child.rs) selects baseline
NEON on release AArch64 and baseline SSE2 on release x86_64. Debug builds,
unsupported architectures, WebAssembly, and the `force-scalar` control all
compile the scalar implementation. The same parity test is compiled against
both normal and forced-scalar configurations, so the control is not a second
algorithm with a separate behavior contract.

The reproducible target matrix is kept in
[`simd_target_checks.sh`](../scripts/simd_target_checks.sh):

| Target/configuration | Expected implementation | Check |
| --- | --- | --- |
| x86_64 macOS release | SSE2 kernel | `cargo check` |
| AArch64 macOS release | NEON kernel | `cargo check` |
| AArch64 iOS release | NEON kernel | `cargo check` |
| AArch64 iOS simulator release | NEON kernel | `cargo check` |
| AArch64 Android release (opt-in) | NEON kernel | `SIMD_CHECK_ANDROID=1 bash scripts/simd_target_checks.sh` |
| wasm32 without `simd128` | scalar fallback | `cargo check` |
| wasm32 with `simd128` target feature | scalar fallback, compatibility build remains buildable | `cargo check` |
| debug or `force-scalar` | scalar reference | parity test |

The matrix was checked on `rustc 1.98.0 (88d9e12ae 2026-08-18)` for host
`aarch64-apple-darwin`, with release `opt-level=3`, LTO, and one codegen unit.
The native AArch64 host run executes the selected NEON path and compares it
with the scalar reference using exact sign/classification checks and a finite
four-ULP tolerance. The final layout fixture,
`selected_flex_dispatch_preserves_layout_positions_and_sizes`, compares
selected-dispatch and scalar sizes, offsets, and total extent on the executable
native host; the other target checks are compile-only. Cross-target checks
verify that the dispatch arms compile without changing the public widget or
element API. The WebAssembly
`+simd128` build is a compatibility check, not a WebAssembly SIMD kernel or
Cargo feature. An Android target check was attempted separately but the sandbox could not download the uncached
`combine 4.6.8` registry package; the AArch64 source arm is otherwise the
same baseline-NEON configuration as iOS. The iOS and simulator checks are
compile-only here; runtime and device power measurements remain external.

Native release size remains 7,530,768 bytes for the headless benchmark
executable. The remaining mobile battery/thermal measurement requires a
minimum iOS/Android device or profiler and is intentionally not inferred from
desktop or compile-only results.

## Implementation phases

Framework-wide optimization is tracked separately in
[FW_OPTIMIZE.md](FW_OPTIMIZE.md). Its measurements inform the SIMD candidate
gate but are not part of this plan.

### Phase 0 — Baseline and profiling

- [x] Add a focused benchmark for flex distribution, including zero, sparse,
      uniform, weighted, and large inputs.
- [x] Add a focused benchmark for size-table construction.
- [x] Add focused benchmarks for scroll physics and animation primitives.
- [x] Add a focused benchmark for image conversion and bulk-data operations.
- [x] Add a focused benchmark for wrapping.
- [x] Measure representative small, medium, and large child counts.
- [x] Record p50/p95 CPU time, allocations, frame time, and binary size.
- [x] Capture profiles for cold layout, cached layout, scrolling, resizing, and
      animated layout.

### Phase 0 result — baseline and profiling complete

Phase 0 was completed on 2026-08-27. The focused wrapping profile is
`flex::wrap_layout::wrap_tests::profile_wrap_layout` in
[`wrap_layout.rs`](crates/aimer_flex/src/flex/wrap_layout.rs); it exercises both
row and column wrapping at 32, 256, and 2,048 children. The existing ignored
numeric profiles cover flex distribution, size-table construction, spring and
velocity physics, curve and interpolation math, keyframe lookup, collection
animation ticks, rect preparation, image conversion, frame-upload bulk data,
and retained image-command capture.

The wrapping profile reports p50 / p95 microseconds per direct
`compute_wrap_layout` call. Output allocation and offset/line construction are
included; child measurement, widget reconciliation, and canvas work are not.

| Case | Debug p50 / p95 | Release p50 / p95 |
| --- | ---: | ---: |
| Row, 32 children | 3.408 / 4.862 us | 0.297 / 0.392 us |
| Row, 256 children | 11.414 / 21.468 us | 1.644 / 1.826 us |
| Row, 2,048 children | 65.745 / 68.127 us | 14.893 / 16.393 us |
| Column, 32 children | 1.080 / 1.125 us | 0.075 / 0.198 us |
| Column, 256 children | 6.859 / 6.982 us | 0.987 / 1.294 us |
| Column, 2,048 children | 53.656 / 55.529 us | 9.055 / 10.974 us |

The complete headless frame baseline is
[`framework_baseline.rs`](../aimer_laboratory/examples/framework_baseline.rs).
It uses a fixed 1,150 × 800 surface, seven rounds, and 64 measured operations
per round. The reported time is elapsed CPU-side work for the headless frame
driver (not OS process CPU time and not native GPU time); allocations are
counted by a benchmark-local recording allocator.

The representative current release matrix is:

| Workload | p50 / p95 | allocations/op |
| --- | ---: | ---: |
| Eager column, 32 rows, cold frame | 53.96 / 350.00 us | 16.57 |
| Eager column, 256 rows, cold frame | 65.17 / 101.71 us | 43.14 |
| Eager column, 2,048 rows, cold frame | 210.08 / 271.17 us | 268.00 |
| Eager column, 2,048 rows, cached frame | 7.27 / 8.46 us | 1.00 |
| Eager column, 2,048 rows, resize | 49.52 / 50.96 us | 4.00 |
| Scrollable column, 2,048 rows, scroll + frame | 11.29 / 11.77 us | 5.02 |
| Scrollable wrapped column, 2,048 rows, scroll + frame | 47.48 / 49.36 us | 12.02 |
| Windowed list, 120,000 rows, cached frame | 3.66 / 3.81 us | 1.00 |
| Windowed list, 120,000 rows, scroll + frame | 6.82 / 6.91 us | 4.34 |
| Animated column, 2,048 rows, cached frame | 3,163.99 / 3,326.59 us | 1,148.00 |

The same matrix was run in debug and release. The debug run covers the same
workload families and also emits per-frame diagnostics; its direct wrapping
results are recorded above. The large animated-column number is intentionally
kept as a mixed application/framework baseline: the one-child animation shell
is 8.10 / 8.22 us in debug and 0.70 / 0.77 us in release, so rebuilding 2,048
application rows—not SIMD-friendly animation arithmetic—is the dominant cost.

The benchmark executables were measured after each profile build:

| Profile | Executable | Size |
| --- | --- | ---: |
| Debug | `framework_baseline` | 63,576,720 bytes |
| Release (`opt-level=3`, LTO, one codegen unit, stripped symbols) | `framework_baseline` | 7,530,768 bytes |

The reproducible commands are:

```text
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase0-target \
  cargo run -p aimer_laboratory --example framework_baseline
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase0-target \
  cargo run -p aimer_laboratory --example framework_baseline --release
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase0-target \
  cargo test -p aimer_flex profile_wrap_layout -- --ignored --nocapture
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase0-target \
  cargo test -p aimer_flex profile_wrap_layout --release -- --ignored --nocapture
```

Phase 0 establishes that explicit SIMD is not yet justified for the measured
domains. Wrapping has a serial line-break dependency and sorting/branching;
scroll physics is a short stateful recurrence; animation ticks update stateful
controllers one at a time; geometry already benefits from scalar transform
reuse; and image resize/upload is delegated to the image library and GPU.
Flex distribution remains the only numeric area with a sufficiently large
contiguous input. The retained scalar in-place path is the Phase 0 baseline;
Phase 1 and Phase 2 below measure whether that seam benefits from a target
kernel without making the widget or element layers SIMD-aware.

### Phase 1 — SIMD-friendly data flow

- [x] Remove the avoidable flex-share allocation from the measured layout hot
      path; keep the remaining numeric allocations under measurement.
- [x] Separate dynamic child measurement from pure numeric post-processing.
- [x] Evaluate contiguous numeric buffers against the profile; retain the
      existing `Vec<f32>` flex table and do not add a parallel `ResolvedSize`
      structure when its memory cost is not justified.
- [x] Confirm that the scalar flex refactor is behaviorally identical with the
      `aimer_flex` regression suite and the expanded end-to-end fixture.

### Phase 2 — First optimized kernel

- [x] Implement and benchmark the best-supported candidate, expected to be flex
      weight distribution or a size reduction.
- [x] Compare compiler auto-vectorization against an explicit kernel.
- [x] Add scalar/SIMD parity tests with tolerances appropriate to each result.
- [x] Compare the candidate with the same-workload end-to-end frame profile;
      keep the kernel only when it does not regress the real frame path.

### Phase 1/2 result — contiguous flex data and release kernel

Phase 1 is complete. `FlexLayout::build` now keeps dynamic child measurement
in `measure_children` and `measure_flex_children`, while
`resolve_flex_space` receives only the existing contiguous `Vec<f32>` table.
The table is reused in place, so there is no second share vector and no new
structure-of-arrays allocation. A `ResolvedSize` SoA was measured as a future
possibility but is not justified by the current frame profiles.

Phase 2's first kernel is the private share-application pass in
[`flex_child.rs`](../crates/aimer_flex/src/flex/flex_child.rs). The positive
weight reduction remains scalar to preserve its summation order. In optimized
native builds, four share lanes use baseline AArch64 NEON or x86_64 SSE2
instructions; the scalar implementation is selected for debug builds,
unsupported targets, and the `force-scalar` benchmark feature. Tails use the
scalar path, negative regular-child sentinels are preserved, and zero/NaN
markers retain their negative-zero representation.

The scalar reference and selected kernel were profiled over 31 rounds with
preallocated inputs, a complete-buffer `black_box` barrier, and alternating
measurement order between cases. These are p50 / p95 microseconds per in-place
operation on the AArch64 native host:

| Input | Scalar reference | Optimized kernel | p50 change |
| --- | ---: | ---: | ---: |
| Zero weights, 32 entries | 0.022 / 0.028 us | 0.022 / 0.022 us | 0.0% faster |
| Sparse weights, 256 entries | 0.395 / 0.431 us | 0.311 / 0.345 us | 21.3% faster |
| Uniform weights, 2,048 entries | 3.276 / 3.464 us | 2.718 / 2.857 us | 17.0% faster |
| Weighted weights, 2,048 entries | 3.036 / 3.216 us | 2.717 / 2.887 us | 10.5% faster |

The scalar reference is the same release build with ordinary compiler
optimization; it does not use `target-cpu=native`. The debug profile keeps the
scalar dispatch because forced intrinsics made the 2,048-entry kernel slower
in debug builds. The release parity test covers empty input, one- and
three-entry tails, exact four-lane input, mixed negative/zero/negative-zero
sentinels, NaN markers, zero remaining space, and weighted inputs with a
four-ULP finite tolerance plus exact sign/classification checks.

The end-to-end control uses the same `framework_baseline` executable in
release, once with the default kernel and once with
`--features force-scalar`. The paired runs reported:

| Workload | Scalar p50/p95 | Optimized p50/p95 | allocations |
| --- | ---: | ---: | ---: |
| Expanded column, 256 rows, cold frame | 44.96 / 46.83 us | 44.54 / 50.71 us | 47.43 / 47.43 |
| Expanded column, 256 rows, resize | 21.00 / 21.41 us | 21.03 / 21.25 us | 9.00 / 9.00 |
| Expanded column, 2,048 rows, cold frame | 161.12 / 173.79 us | 160.46 / 181.21 us | 213.00 / 213.00 |
| Expanded column, 2,048 rows, resize | 77.17 / 79.01 us | 76.94 / 77.55 us | 9.00 / 9.00 |

The full frame is dominated by child dispatch, measurement, and table
construction, so the kernel's direct speedup is not an app-wide scroll-speed
claim. The paired frame result is allocation-neutral and has no consistent
material regression; the explicit kernel is retained as a private,
target-selected release optimization, while future kernels must show a
similarly controlled end-to-end benefit before adoption. The scalar path
remains available for debug, WebAssembly, and the benchmark control.

Reproduce the direct comparison with:

```text
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase2-target \
  cargo test -p aimer_flex profile_distribute_flex_space_variants --release -- --ignored --nocapture
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase2-target \
  cargo run -p aimer_laboratory --example framework_baseline --release
env CARGO_TARGET_DIR=/private/tmp/aimer-framework-opt-simd-phase2-target \
  cargo run -p aimer_laboratory --example framework_baseline --release --features force-scalar
```

### Phase 3 — Additional numeric domains

- [x] Apply the same measurement gate to scroll physics and animation math.
- [x] Apply it to geometry/color/image operations only when they show up in
      profiles.
- [x] Keep each kernel's optimization independent so one unsupported target does
      not disable unrelated framework features.

### Phase 4 — Cross-target rollout

- [x] Add native feature dispatch and scalar fallback tests.
- [x] Add WebAssembly SIMD and non-SIMD build checks where supported by the
      toolchain.
- [x] Verify selected-dispatch versus scalar layout sizes and offsets within
      documented floating-point tolerances on the executable native host, and
      compile every target dispatch arm.
- [x] Record native release code size and keep startup/battery work separate
      from the low-level kernel gate.
- [ ] Measure startup, battery, and thermal impact on minimum supported mobile
      devices.

### Phase 5 — Adoption and maintenance

- [x] Document the benchmark that justifies each SIMD kernel.
- [x] Keep a scalar reference implementation beside each optimized kernel.
- [x] Re-run the benchmark suite when layout data structures or target support
      changes.
- [x] Remove an optimized path if it no longer provides measurable benefit.

### Phase 5 result — reproducible adoption policy

The benchmark and target-check entry points are now maintained beside the
workspace:

- [`simd_benchmark_suite.sh`](../scripts/simd_benchmark_suite.sh) reruns the
  focused flex, scroll, animation, geometry/image, and paired native frame
  profiles.
- [`simd_target_checks.sh`](../scripts/simd_target_checks.sh) reruns normal
  and forced-scalar parity plus the native and WebAssembly target matrix.

The scalar implementation stays next to every explicit kernel, and the plan
records the direct and end-to-end evidence required to retain it. If a future
layout-data or toolchain change removes the direct gain or introduces a real
frame regression, the release dispatch must be removed or narrowed back to the
scalar path rather than preserved for architectural symmetry.

The full suite completed on 2026-08-27 in the release profile: flex, scroll,
animation, geometry/image, and paired native-frame profiles all passed. The
suite reports the direct flex-kernel gain on the measured large inputs, but the
paired frame control remains allocation-neutral and does not satisfy the
end-to-end improvement gate. That gate is intentionally left open until a
future workload shows a material whole-frame improvement rather than a
microbenchmark-only win.

## Acceptance criteria

The SIMD work is complete only when:

- [x] Every optimized kernel has a representative benchmark and scalar parity
      tests.
- [ ] The end-to-end frame path improves on a measured workload, not just a
      microbenchmark.
- [x] All supported targets retain a working scalar fallback.
- [x] Layout positions, sizes, hit testing, scrolling, animation timing, and
      rendering order remain correct.
- [x] No public widget or element API requires callers to know about SIMD.
- [x] The final documentation records the selected targets, dispatch strategy,
      benchmark result, and known numerical limitations.
