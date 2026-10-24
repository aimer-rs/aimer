# Real dirty-region redraw without rebuilding the element tree

> Status: planned. This document is self-contained and defines the scope,
> implementation steps, correctness requirements, and performance acceptance
> for incremental dirty-region redraw in Aimer.

## Goal

Keep the mounted `Element` tree and apply localized visual updates without
calling the root `draw()` path. Rebuild only the smallest affected branch when
its declarative widget output changes, patch that branch's retained paint data,
and redraw only affected target pixels. The initial frame and any frame with
unknown invalidation or target state use the full-frame path.

The feature is internal to Aimer. Widget composition and ordinary user APIs
remain unchanged. Every supported target must implement the safe dirty-region
path for the shared supported paint contract. Unsupported operations use an
explicit full-frame fallback.

## Current design state

- The retained production element tree is recursively owned through erased
  `AnyElement` children. `ElementId`s are stable across compatible
  reconciliation.
- Dirty-path tracking currently lets a root rebuild walk prune clean branches;
  it does not provide direct `ElementId`-to-element lookup and does not bypass
  the root draw walk.
- Aimer has paint-stability contracts, retained paint replay, scene-node bounds
  and revisions, `DamageSet`, and a persistent-target renderer path. Packet and
  damage consumption still vary by presentation target.
- The historical Jaime baseline used an arm64 macOS Metal environment with an
  approximately 1104×768 window. The current Aimer MCP run 2 confirms a
  2300×1600 physical target at 2× scale and 120 Hz on an Apple M4 MacBook Pro;
  see [`PHASE0_BASELINE.md`](PHASE0_BASELINE.md) for the dated measurements.

## Update contract

1. A visual mutation queues an invalidation before root drawing. The record
   identifies one `ElementId`, its change kind, prior committed bounds, and
   relevant subtree, paint, layout, and resource revisions. Convert to
   `SceneNodeId` only at the compositor seam.
2. Maintain a generation-safe index from `ElementId` to its retained node and
   parent path. Update the index on insertion, compatible reconciliation,
   replacement, and removal. Discard or conservatively promote stale IDs.
3. For a localized update, use that index to process dirty branches without
   invoking root `draw()`. A declarative change may call `build()` on the
   smallest affected branch; clean elements retain their identity and state.
4. Reconstruct the current `BuildContext` for a dirty branch from its ancestor
   path without calling ancestor `draw()`. Refresh constraints, geometry,
   viewport, and inherited state through the relevant ancestor context hooks.
   If that context cannot be reconstructed safely, use the full path.
5. Service active frame-driven work independently of drawing. Maintain an
   indexed frame-update path for live animation/resource callbacks. Do not
   bypass root drawing when a required callback cannot be reached or safely
   separated from it; use the conservative live/full path instead.
6. After required branch rebuild/layout, compute new bounds. Damage covers old
   and new footprints plus intersecting background, clip, transform, ordering,
   and effect dependencies needed to match a full repaint. Insertions have no
   old bounds; removals have no new bounds.
7. Patch retained paint commands by scene node or subtree range, preserving
   paint order, resource lifetime, and dependencies. Unknown bounds, ordering,
   lifecycle work, resources, or target validity promote to full repaint before
   preserved target pixels are modified.
8. Keep layout, hit testing, focus, accessibility, input, and animation state
   current independently of retained paint. Paint reuse must not suppress
   required lifecycle or interaction work.

### Change handling

| Change | Required update |
| --- | --- |
| Paint-only value | Re-record the owning branch if needed; use its committed bounds. |
| Geometry or layout | Rebuild/layout the smallest dependent branch; then compute new bounds. |
| Insert, remove, or reorder | Update the retained node index and command order; damage affected old/new footprints and overlaps. |
| Transform, clip, or opacity | Update scene properties and damage old/new transformed bounds plus affected dependencies. |
| Resource readiness or dynamic callback | Run the indexed update and invalidate its owner; use full path if the owner or footprint is unknown. |
| Unknown invalidation or stale identity | Promote to full rebuild/repaint. |

## Implementation checklist

### Phase 0 — baseline and instrumentation

- [x] Preserve the full-repaint baseline and record build profile, backend,
  device, operating system, target size, device scale, and refresh rate. The
  current Aimer MCP run 2 records Debug/Metal on an Apple M4 MacBook Pro running
  macOS 27.2, with a 2300×1600 physical target, 2× scale, and 120 Hz; System
  Profiler exposes no separate Metal driver version.
- [x] Rebuild the existing reference app through Aimer MCP and attach the
  resource monitor to that managed run. Record its session, run, PID, and
  resource-recording ID with each measurement. For the current capture, the
  original c036 session was restarted as run 2 (PID 67417), recording
  `c036b99d-e0e9-4f68-a3ad-558070b6ae59_2`.
- [x] Capture controlled idle and scroll windows on the Aimer MCP managed
  session, with sample intervals, recording IDs, and bounds. Run 2 has both a
  15 s idle capture and a 15 s user-driven sidebar scroll capture (28 samples
  each); its automated CUA scroll window overlaps **26** resource samples.
  Run 3 has a final 15 s idle capture (28 samples). Bounded captures restored
  the 1000 ms default.
- [x] Record the automated scroll input start/end and pair its resource samples
  with the frame-report sequence range. The 76-action CUA window ran from
  17:02:20.675–17:02:35.811 UTC on run 2. Cursor pagination works when
  `aimer_get_logs` is called without `run_id`; sequence range 32197–33821
  contains 53 frame reports with scroll/smoothing counters. Log entries do not
  have per-report timestamps, so the exact input times and report sequence
  bracket are retained together in
  [`PHASE0_BASELINE.md`](PHASE0_BASELINE.md).
- [x] Re-enable the existing 30-frame Debug report for build, encode, present,
  rebuild, layout, paint, input, and redraw counters.
- [x] Define CPU frame time as the interval from immediately before the
  `MetalApi` retained build/rebuild walk through renderer command-buffer
  submission. It includes layout, paint, and CPU command encoding; it ends
  before drawable presentation or vsync waits.
- [x] Define GPU frame time as the earliest valid Metal `GPUStartTime` through
  the latest `GPUEndTime` among command buffers submitted for the renderer
  frame. Completion callbacks emit one sample after all frame submissions
  finish; drawable presentation and OS/vsync waits are excluded.

Phase 0 baseline and instrumentation are complete for the reference fixture.
The separately listed Release, 60 Hz, and expanded widget-workload comparisons
remain follow-up measurements; they are not claimed as performance acceptance.

### Phase 1 — invalidation source and direct node index

- [x] Add an internal invalidation record keyed by canonical `ElementId`, with
  change kind for paint, geometry/layout, structure, transform/order, resource,
  or unknown changes. Records retain the owner path, tree/rebuild/layout
  revisions, stable old/new frame-space bounds, and an optional device-pixel
  paint-damage hint.
- [x] Count invalidations queued/coalesced, direct node lookups, and stale IDs
  in the 30-frame report.
- [x] Reuse the generation-safe `ElementId` parent-path index maintained by
  `EventDispatcher`. It is synchronized before frame processing and after
  structural reconciliation, and each replacement index is built before it is
  committed.
- [x] Capture stable pre-walk and post-rebuild bounds and tree/rebuild/layout
  revisions. Validate every queued ID through the current root and generation;
  missing or unbounded owners use the full-frame fallback.
- [x] Connect state/style rebuilds and scroll state through `DirtySource`,
  layout changes through the layout-generation invalidation, and animation
  paint damage through the current drawing owner's ID. Unowned resource/full
  invalidations are recorded as `Unknown` and remain full-frame.
- [x] Coalesce repeated changes per `ElementId`. Conflicting change kinds
  promote the record to `Unknown`; unowned, stale, or unbounded records promote
  the frame to full damage.

Current producers expose tree, rebuild, and layout revisions. Per-element
resource epochs are not available, so the resource revision is recorded as
`None`; resource invalidations without a known owner stay on the full path.
The event-coordinate bounds are kept in the widget layer, while paint owners
may also attach the compositor's device-pixel damage hint.

#### Phase 1 implementation check — Aimer MCP run 5

- The original session `c036b99d-e0e9-4f68-a3ad-558070b6ae59` was rebuilt
  through Aimer MCP as run 5, PID `88180`. No separate Jaime session was
  launched.
- A native CUA loop sent **73** alternating sidebar scroll actions from
  **2026-09-30 18:08:55.389–18:09:10.437 UTC**. Recording
  `c036b99d-e0e9-4f68-a3ad-558070b6ae59_5` supplied **28** samples at 0.5 s
  within that window. Process CPU mean/p50/p95/max was
  **50.92/52.90/58.14/59.25%**; GPU was **15.03/15.53/17.84/18.49%**. The
  recorder restored the 1000 ms monitor interval afterward.
- `aimer_get_logs` without a `run_id` filter reported `newest_seq=41047`. The
  fetched pages after cursors **37577** and **40000** included reports that
  exercised the new queue and direct index lookup counters without stale IDs:

  | Log sequence | CPU mean / p95 bound | GPU mean / p95 bound | Scroll events / steps | Queued / coalesced | Index lookups / stale IDs |
  | ---: | ---: | ---: | ---: | ---: | ---: |
  | 37601 | 5.10 / ≤12.50 ms | 2.59 / ≤9.50 ms | 0.1 / 1.7 | 4.0 / 3.2 | 7.1 / 0.0 |
  | 40019 | 5.81 / ≤11.50 ms | 1.49 / ≤2.00 ms | 0.7 / 2.0 | 4.7 / 3.5 | 8.2 / 0.0 |
  | 40050 | 3.19 / ≤8.00 ms | 1.73 / ≤3.50 ms | 0.5 / 1.8 | 3.9 / 3.2 | 7.0 / 0.0 |

- Debug and optimized Release compile checks passed. Jaime remained running in
  the managed session; the visible scroll workload exercised the counters.

### Phase 2 — local branch update and frame lifecycle

- [ ] Process indexed dirty branches without invoking root `draw()` on valid
  localized frames. Rebuild only the smallest branch whose declarative output
  changed and preserve compatible runtime state.
- [ ] Count dirty branches rebuilt, clean branches skipped, root draws bypassed,
  context reconstructions, and command ranges patched.
- [ ] Reconstruct branch `BuildContext` by replaying the ancestor context hooks
  needed for inherited state and constraints, without calling ancestor
  `Drawable::draw` methods.
- [ ] Recompute layout for dirty geometry and its actual parent/sibling
  dependencies. Synchronize hit-test and focus geometry separately from paint.
- [ ] Add an indexed per-frame update path for active animation and resource
  callbacks. Require each live callback to report its owner and damage, or use
  the conservative root/full path.
- [ ] Retain paint commands by scene node or subtree range. Replace changed
  ranges while preserving order, clips, transforms, dependencies, and resource
  ownership.
- [ ] Handle compositor-only transform/opacity changes without widget building
  when paint and layout contracts prove that safe.

### Phase 3 — packet and renderer integration on every target

- [ ] Carry target dimensions, device scale, surface/renderer/context/resource
  generations, validity, damage, ordered scene dependencies, retained command
  references, and resource lifetimes in the owned frame packet.
- [ ] Route packet damage through every `aimer_quiver` presentation adapter:
  WGPU, Metal, OpenGL, Vulkan, DX12, H5 Canvas/portable-web, and native auto
  routing.
- [ ] For each region, clear only damaged pixels and replay all intersecting
  retained content and dependencies in original paint order. Preserve the
  initialized target between frames; final surface composition may remain
  full-surface.
- [ ] Report damaged pixels, target reuse, and full-frame fallback reasons.
- [ ] Reuse current `DamageSet` clipping/coalescing behavior as the initial
  policy: at most eight disjoint regions and full repaint at half-target area.
  Tune thresholds only from measurements.
- [ ] Keep full-frame fallback for first frame, resize/scale/context loss,
  invalid target, unknown damage, and unsupported MSAA, custom pipeline,
  backdrop, filter, shadow, or asynchronous-resource paths.
- [ ] Verify all supported targets build and that unsupported capabilities
  select the explicit full-frame fallback rather than using stale pixels.

### Phase 4 — correctness and regression coverage

- [ ] Unit-test invalidation coalescing, direct lookup, stale-ID rejection, and
  old/new bounds for paint changes, movement, resize, insertion, removal,
  reorder, and unknown geometry.
- [ ] Prove with counters that a localized update skips root `draw()`, leaves
  clean-branch build/draw counts unchanged, and processes only the dirty branch
  and declared layout dependencies.
- [ ] Test direct branch rebuild under nested inherited-state providers,
  changed parent constraints, viewport changes, and compatible reconciliation.
- [ ] Verify active animations and async/resource callbacks still advance and
  damage their owners during localized frames; verify unknown callbacks choose
  the conservative path.
- [ ] Add direct-vs-partial pixel comparisons for overlapping siblings,
  unchanged backgrounds, nested clips, transforms, opacity, text, images,
  resource readiness, animation, and effects.
- [ ] Cover first frame, target reuse, resize, device-scale change, context loss,
  and each conservative fallback.
- [ ] Verify hit testing, focus, accessibility, keyboard navigation, and input
  while paint commands are retained.
- [ ] Run renderer correctness tests on every supported backend and retain the
  direct full-frame path for unsupported capabilities.

## Benchmark and acceptance

### Synthetic localized-update workload

- [ ] Build a deterministic 1,024-node retained tree on a fixed 1104×768 target
  at fixed device scale. Change one leaf whose visual bounds cover 5% of the
  target; keep all other content unchanged.
- [ ] Compare candidate dirty redraw against a forced full-repaint baseline on
  the same arm64 macOS Metal device, driver, build profile, and workload. Warm
  up for 120 frames, then collect 1,000 measured frames per mode.
- [ ] Report p50/p95 CPU frame time, p50/p95 GPU time, dirty-pixel ratio,
  command-patch count, direct-lookup count, root-draw count, fallback count,
  allocations, and dropped frames.
- [ ] Make Debug the optimization acceptance profile. Require candidate p95
  frame CPU time and p95 GPU time to be each at least 75% lower than the
  same-profile full-repaint baseline. Capture optimized Release measurements as
  a cross-check.

### Jaime real-application regression

- [ ] Repeat the sidebar-only paging workload against the arm64 macOS Metal
  reference setup. Confirm the right content pane is not rebuilt, root-drawn,
  or re-recorded on sidebar-only frames.
- [ ] Compare visual output and report the same frame-work, damage, CPU, and GPU
  counters. Jaime is the real-app regression workload; the numeric 75% gate is
  defined by the synthetic localized-update benchmark.

## Completion criteria

- [ ] A valid localized update uses direct, generation-safe node lookup,
  reconstructs the required branch context, and bypasses root `draw()`.
- [ ] Clean element branches are neither rebuilt nor re-recorded; required
  lifecycle and interaction work still runs through the indexed update path.
- [ ] Partial output is pixel-equivalent to the full-frame reference across
  the correctness matrix, with full fallback for every unknown/unsupported
  case.
- [ ] Every supported target consumes the common damage packet correctly.
- [ ] The synthetic workload meets both 75% p95 CPU and 75% p95 GPU reduction
  gates on the reference target.
- [ ] Jaime confirms unchanged sibling content remains retained during
  sidebar-only paging, with no stale pixels or interaction regressions.
