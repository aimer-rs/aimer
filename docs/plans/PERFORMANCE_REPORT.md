# Performance Report: High Resource Usage on Scroll Events

**Date:** 2026-08-29
**Target:** macOS native `website` release build from the current checkout
**Profiler:** Apple Instruments — Time Profiler and Activity Monitor

## Executive finding

The scroll event is the trigger for the CPU spike, but event decoding itself is
not the dominant cost. A wheel event queues work in the scroll smoother, the
next frame dispatches a `Scroll` event, and that event causes a redraw. During
the redraw, the page's widget tree is walked and painted again, including flex
layout, text preparation, draw-command encoding, and Metal presentation.

The primary framework-level cause is that the page content is not eligible for
full retained-paint replay:

1. `RawScrollableContainer::draw` calls
   `draw_child_with_retained_paint` on every frame.
2. That path checks `child.is_paint_stable()`.
3. `Drawable::is_paint_stable` conservatively defaults to `false`.
4. `RawContainer` has a `Drawable` implementation, but no stability override;
   it therefore uses the default. Its `draw` method also updates hit-test
   bounds, which is an observable side effect that makes blindly marking it
   stable unsafe.
5. Blog and vertical blog-detail scrolling place a `Container` directly under
   `Scrollable`; Home's scroll child is a flex tree containing multiple
   `Container` nodes. On desktop, the horizontal blog-detail layout instead
   enables scrolling inside `MarkdownViewer`, which still contains a large
   text/container subtree. The retained path consequently falls back to
   ordinary child drawing or to a partially dynamic island path instead of
   replaying a static layer.

This explains why CPU rises while scrolling inside Home, Blog, and the latest
blog. The event route adds work, but the measured hotspot is the redraw that
follows it.

## Reproduction and measurement

The profiled app was a symbolized `aarch64-apple-darwin` release build:

```text
CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_DEBUG=2 \
  cargo build -p website --target aarch64-apple-darwin --release
```

The executable was placed in a temporary uniquely named app bundle so
Instruments could attach to it. The workload was:

- Home: bottom, top, bottom, top;
- Blog: bottom, top, bottom, top;
- latest blog: bottom, top, bottom, top, including extra movement required by
  the longer detail page.

The single-scroll trace used for call-stack attribution contained one scroll
gesture and deliberately avoided `get_app_state` calls until after the gesture
had settled, so accessibility automation did not contaminate the event burst.

## Resource measurements

Activity Monitor measurements are process CPU percentages; 100% represents one
fully occupied logical CPU. The Time Profiler sample counts below are inclusive
statistical samples and overlap, so they must not be added together as a total.

| Scenario | CPU result | Memory result |
| --- | --- | --- |
| Home idle, settled | average **0.009%**, maximum **0.025%** | not separately recorded |
| Blog idle, settled | average **0.006%**, maximum **0.009%** | not separately recorded |
| One clean scroll | weighted average **0.777%** over 14.97 s; maximum **10.599%** in one 1 s bucket | footprint peaked at **184.97 MiB**; later settled near **88.9 MiB** |
| Full repeated Home → Blog → latest-blog workload | average **13.51%** over 51.20 s; maximum **46.08%** | footprint ranged from **50.16 MiB** to **208.31 MiB**, average about **131.48 MiB**; compressed memory was **0** |

Under the exact scripted order, the approximate active-scroll CPU peaks were
**38.05% on Home**, **33.96% on Blog**, and **46.08% on the latest detail**.
The latest detail was highest, consistent with its larger text/layout subtree,
but the same redraw mechanism is present on all three pages.

The idle-only trace is important: after a page settled, CPU returned to nearly
zero. A brief post-navigation spike reached about 23% in one bucket while the
route transition settled, but it did not continue. Native control flow is
`ControlFlow::Wait`, and deferred text preparation is configured not to keep a
settled page in a render loop. Persistent CPU while the app is genuinely idle
was therefore not reproduced in this release profile; a debug build, inspector,
or external automation should be profiled separately if it still shows that
behavior.

## Time Profiler evidence

The clean single-scroll trace's main event/render burst was around 5.02–5.27 s.
The highest source-line aggregates were:

| Hot path                                            |                 Inclusive samples | Source                                                                                               |
|-----------------------------------------------------|----------------------------------:|------------------------------------------------------------------------------------------------------|
| `ElementNode::draw` → `self.element.draw(ctx)`      |                               140 | [`element.rs:812`](../crates/aimer_widget/src/components/element.rs#L812)                            |
| `AnyElement::draw`                                  |                               108 | [`element.rs:1014`](../crates/aimer_widget/src/components/element.rs#L1014)                          |
| `WgpuApi::render_frame`                             |                                46 | [`wgpu_ctx.rs:306-308`](../aimer_quiver/src/render_ctx/wgpu_ctx.rs#L306-L308)                        |
| `WgpuApi::present`                                  |                                38 | [`wgpu_ctx.rs:362-370`](../aimer_quiver/src/render_ctx/wgpu_ctx.rs#L362-L370)                        |
| `Renderer::render_impl`                             |                                31 | [`renderer.rs:1433-1442`](../aimer_cupid/src/renderer.rs#L1433-L1442)                                |
| `TextPipelineV2::prepare` / glyph work              |                                17 | [`text_pipeline.rs:1612-1623`](../aimer_cupid/src/pipeline/text_pipeline.rs#L1612-L1623)             |
| `RawFlex::render_child`                             |                                15 | [`raw_flex.rs`](../crates/aimer_flex/src/flex/raw_flex.rs)                                           |
| flex `LayerOrder::visit`                            |                                14 | [`raw_flex.rs`](../crates/aimer_flex/src/flex/raw_flex.rs)                                           |
| `RawScrollableContainer::on_event` / child dispatch | 1 each in the sampled event stack | [`handle_scroll.rs:324-349`](../crates/aimer_scroll/src/scrollable/input/handle_scroll.rs#L324-L349) |

The event stack at the beginning of the burst was:

```text
MouseWheel
  -> handle_mouse_wheel
  -> scroll smoother queue + frame request
  -> begin_frame / dispatch_smoothed_scroll
  -> dispatch_element_event / EventDispatcher
  -> RawScrollableContainer::on_event
  -> apply_scroll_frame
  -> RedrawRequested
  -> WgpuApi::render_frame
  -> widget draw / flex / text / present
```

## Reconciliation optimization A/B baseline

Before changing the reconciliation implementation, the exact requested
workload was completed against the existing
`website/builds/macos/build/Debug/website.app` build. To prevent macOS from
reusing another `website` process with the same bundle identifier, the build
was copied to a temporary bundle with a unique executable name and profiled as
`website-reconciliation-baseline`.

Workload:

- Home: bottom, top, bottom, top;
- Blog: bottom, top, bottom, top;
- latest blog: bottom, top, bottom, top.

The valid Time Profiler recording ran for **20.724 s** and its process path was
the isolated copy of the current checkout. The exported stack data contained
the reconciliation paths `collect_matches`, `find_keyed_stateful`,
`element_children`, `carry_keyed_child_state`, and
`carry_unkeyed_child_state`, alongside the rendering paths. This is the
before-optimization attribution for the A/B comparison; the stack samples are
statistical and are not a CPU-percentage breakdown.

The fresh attached Activity Monitor recording covered **24.440 s**. The six
contiguous live samples in the interaction burst averaged **73.00% CPU**
(duration-weighted), peaked at **93.56% CPU**, and reported a physical
footprint of **222.27–232.52 MiB**. Across the whole recording, the process
consumed **5.161 s of CPU time**, or **21.12% of one core**. The burst and the
whole-recording ledger are reported separately because instrument startup and
idle time are included in the ledger.

Captured baseline traces:

```text
/private/tmp/website-reconciliation-baseline-time-v4.trace
/private/tmp/website-reconciliation-baseline-activity-v3.trace
```

The earlier launch-mode Activity Monitor attempt is retained for debugging
only; it failed before the first scroll completed:

```text
/private/tmp/website-reconciliation-baseline-activity-v1.trace
```

## Reconciliation optimization A/B result

The optimization was applied after the baseline capture:

- [`reconciliation_plan.rs:collect_matches`](../crates/aimer_widget/src/reconciliation_plan.rs#L201)
  now indexes old keyed siblings by `(key, element type, debug name)` and
  keeps source-order candidate cursors. Keyed matching changes from a sibling
  scan per new child to linear index construction plus lookups, while
  preserving duplicate-key and compatibility behavior.
- [`stateful.rs:KeyedStateIndex`](../crates/aimer_widget/src/widget/stateful.rs#L1118)
  builds one keyed-state index per carry scope instead of recursively scanning
  the old subtree for every keyed replacement. The state-revision tie-breaker
  remains unchanged.
- State carry now uses the canonical `structural_children` accessor, keeping
  reconciliation/state traversal consistent for containers such as
  `Scrollable`.

The rebuilt app was produced from the same website target with
`cargo build -p website --target aarch64-apple-darwin`, refreshed into the
existing `website/builds/macos/Libraries/libwebsite.a`, and relinked with the
existing macOS Xcode project. The post-optimization Time Profiler recording
ran for **23.463 s**. Its exported stack data no longer contained
`find_keyed_stateful` and did contain `KeyedStateIndex`; this confirms that the
new implementation was in the profiled binary. As with all Time Profiler
sample counts, this is attribution evidence rather than a CPU percentage.

The fresh post-optimization Activity Monitor recording covered **24.987 s**.
Its six contiguous interaction-burst samples averaged **68.17% CPU** and
peaked at **102.29% CPU**, with a physical footprint of
**188.83–196.86 MiB**. The whole-recording ledger consumed **4.874 s of CPU
time**, or **19.51% of one core**.

| Metric | Baseline | Optimized | Change |
| --- | ---: | ---: | ---: |
| Interaction-burst average CPU | 73.00% | 68.17% | −6.61% |
| Whole-recording CPU share | 21.12% | 19.51% | −7.63% |
| Interaction-burst peak CPU | 93.56% | 102.29% | +9.33% |
| Interaction-burst peak footprint | 232.52 MiB | 196.86 MiB | −35.66 MiB / −15.34% |

The average CPU and footprint improved, but the single-run CPU peak did not;
repeat runs are needed before treating peak CPU as a regression. The result
also confirms that reconciliation is not the main sustained scroll cost: the
remaining hot path is still page drawing/GPU submission identified above.

Post-optimization traces:

```text
/private/tmp/website-reconciliation-after-time-v3.trace
/private/tmp/website-reconciliation-after-activity-v2.trace
```

## Root-scoped path invalidation A/B (2026-08-30)

### Change

`EventDispatcher::synchronize_paths` now compares its cached generation with
`root.subtree_generation()` instead of the process-wide
`element_tree_generation()`. The private cache field is named
`indexed_subtree_generation` to make that scope explicit. The existing
`STABLE_SUBTREE_GENERATIONS` map and `ElementNode::subtree_generation()` remain
the source of the local value:

- [`element.rs:41-42`](../crates/aimer_widget/src/components/element.rs#L41)
- [`element.rs:646-661`](../crates/aimer_widget/src/components/element.rs#L646)
- [`element.rs:1431-1458`](../crates/aimer_widget/src/components/element.rs#L1431)
- [`element.rs:1741-1757`](../crates/aimer_widget/src/components/element.rs#L1741)

An unrelated branch can still advance the global generation, but a dispatcher
whose stable dispatch root did not change now keeps its path index. A root whose
local subtree generation changes still re-indexes. Unstable roots retain the
conservative behavior because `ElementNode::subtree_generation()` reports the
global generation for them.

### Workload and binary isolation

The exact requested sequence was run before and after the change:

- Home: bottom, top, bottom, top;
- Blog: bottom, top, bottom, top;
- latest blog: bottom, top, bottom, top.

Each run used a uniquely identified, ad-hoc-signed copy of the existing macOS
website build. Instruments attached to the verified process PID, avoiding the
same-bundle collision with the sibling `aimer-hot-reload-opt` checkout.

### Measurements

Activity Monitor values below come from the `activity-monitor-process-live`
table. “Active CPU” is the duration-weighted average of live samples at or
above 20% CPU, isolating the interaction burst. “Whole trace CPU” is the
Activity Monitor CPU ledger divided by the recording duration.

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Recording duration | 31.042 s | 34.386 s | not comparable across UI timing |
| CPU ledger | 14.29 s | 16.08 s | not comparable across duration |
| Whole-trace CPU share | 46.03% | 46.76% | +0.72 percentage points |
| Active interaction CPU | 72.95% | 69.99% | −2.96 points / −4.06% |
| Peak CPU | 102.05% | 110.58% | +8.53 points |
| Peak physical footprint | 237.36 MiB | 199.81 MiB | −37.55 MiB / −15.82% |

This single run shows a lower average CPU during the active interaction window
and a lower memory peak, but a higher peak CPU and slightly higher whole-trace
CPU share. It is evidence that the scoped cache is active, not conclusive proof
of a universal CPU improvement; repeat runs are needed because the interaction
duration and render scheduling vary.

Time Profiler confirmed the binaries and recorded 13,768 baseline samples and
13,617 post-change samples. The post-change trace contains the
`subtree_generation` frame while the baseline trace uses the old global lookup;
the sample counts are statistical and should not be treated as exact traversal
counts.

### Validation

The focused regression test was red before the implementation (`1` vs `0` for
the cached generation), then passed after it. The test verifies both behaviors:

- an unrelated global generation advance does not invalidate a stable root's
  path index;
- changing that root's subtree generation does invalidate it.

The serialized `aimer_widget` suite also passed:

```text
cargo test -p aimer_widget -- --test-threads=1
243 unit tests passed; 20 doctests passed, 10 ignored; 1 compile-fail doctest passed, 2 ignored.
```

The scroll crate regression suite also passed:

```text
cargo test -p aimer_scroll
168 unit tests passed; 2 ignored; 7 doctests passed; 2 ignored.
```

Additional traces for this A/B run:

```text
/private/tmp/website-subtree-baseline-attach.trace
/private/tmp/website-subtree-baseline-activity.trace
/private/tmp/website-subtree-after-v4.trace
/private/tmp/website-subtree-after-activity.trace
```

## Frame-coalesced event-path indexing A/B (2026-08-30)

### Change

The event path index now checks and rebuilds at most once per dispatcher per
event frame. `EventDispatcher` keeps a `paths_dirty` bit and the frame in which
it last checked the dispatch root. On the first dispatch of a managed frame it
samples `root.subtree_generation()`; a mismatch marks the index dirty and the
first dispatch performs the required full walk. Later events in the same frame
skip both the generation read and the walk. A root identity change remains an
immediate safety check. Direct callers that do not start managed frames retain
the previous per-dispatch fallback behavior.

The shared UI-thread epoch is advanced by `begin_event_frame()` after a frame
finishes, including the retry path. This lets the application dispatcher and
dispatchers owned by nested widgets such as `Scrollable` share the same frame
boundary:

- [`element.rs:50-94`](../crates/aimer_widget/src/components/element.rs#L50-L94)
- [`element.rs:1452-1459`](../crates/aimer_widget/src/components/element.rs#L1452-L1459)
- [`element.rs:1766-1808`](../crates/aimer_widget/src/components/element.rs#L1766-L1808)
- [`handler.rs:447-455`](../aimer_quiver/src/handler.rs#L447-L455)

The regression test counts tree visitation. It verifies that an unrelated
global generation change does not cause another walk, that a root-local change
is observed at the next frame, and that repeated dispatches in that frame still
walk only once ([`element.rs:2672-2713`](../crates/aimer_widget/src/components/element.rs#L2672-L2713)).

### Workload and binary isolation

The exact requested sequence was run against both isolated macOS app copies:

- Home: bottom, top, bottom, top;
- Blog: bottom, top, bottom, top;
- latest blog: bottom, top, bottom, top.

The before binary was the previous root-scoped implementation at
`/private/tmp/website-subtree-after-v4.app`; the after binary contains the
frame-coalesced implementation at `/private/tmp/website-frame-after-v1.app`.
Both were attached by their unique executable name, and each Activity Monitor
recording ran for 60 seconds. The workload was executed near the start of the
recording; the remaining time was left idle so the traces have the same fixed
observation window.

### Measurements

Values are from the `activity-monitor-process-live` table. “Active CPU” is the
duration-weighted average of samples at or above 20% CPU. The CPU ledger is
cumulative before attachment, so “CPU time” uses the delta between the first
and final live `cpu-total` samples; this avoids counting pre-recording work.

| Metric | Root-scoped before | Frame-coalesced after | Change |
| --- | ---: | ---: | ---: |
| Recording duration | 60.847 s | 60.835 s | effectively equal |
| Active sample duration | 11.495 s | 8.381 s | −27.08% |
| Active interaction CPU | 59.47% | 41.91% | −17.56 points / −29.53% |
| CPU time during trace | 7.006 s | 3.955 s | −3.051 s / −43.55% |
| Whole-trace CPU share | 11.51% | 6.50% | −5.01 points / −43.54% |
| Peak CPU | 85.84% | 82.63% | −3.21 points / −3.74% |
| Peak physical footprint | 195.95 MiB | 197.16 MiB | +1.20 MiB / +0.61% |

This single matched run shows a lower active CPU and CPU-time total after
coalescing, while peak memory was effectively flat. It is strong evidence that
repeated event dispatches were paying avoidable generation/index work, but it
is not a benchmark-quality universal speedup: the active burst lengths differ,
and Activity Monitor peaks are noisy. Repeated runs and instrumentation around
the actual index-walk count should follow before treating the result as a
release performance guarantee.

Captured artifacts:

```text
/private/tmp/website-frame-baseline-v1-activity.trace
/private/tmp/website-frame-baseline-v1-activity-toc.xml
/private/tmp/website-frame-baseline-v1-live.xml
/private/tmp/website-frame-baseline-v1-ledger.xml
/private/tmp/website-frame-after-v1-time.trace
/private/tmp/website-frame-after-v1-time-profile.xml
/private/tmp/website-frame-after-v1-activity.trace
/private/tmp/website-frame-after-v1-activity-toc.xml
/private/tmp/website-frame-after-v1-live.xml
/private/tmp/website-frame-after-v1-ledger.xml
```

### Validation

The focused regression test was first added against the old implementation;
before the frame API existed it failed at compile time because
`begin_event_frame` was not yet defined. After the implementation, it passed:

```text
cargo test -p aimer_widget dispatcher_reindexes_at_most_once_per_event_frame -- --nocapture
1 passed; 0 failed; 242 filtered out.
```

The relevant post-change suites also passed:

```text
cargo test -p aimer_widget -- --test-threads=1
243 unit tests passed; 20 doctests passed, 10 ignored; 1 compile-fail doctest passed, 2 ignored.

cargo test -p aimer_scroll
168 unit tests passed; 2 ignored; 7 doctests passed, 2 ignored.

cargo test -p aimer_quiver
135 unit tests passed; 7 doctests passed.
```

The website library was rebuilt for `aarch64-apple-darwin`, copied into the
existing `website/builds/macos/Libraries/libwebsite.a`, and the macOS project
was relinked successfully with `xcodebuild`.

The native event handler confirms that `MouseWheel` feeds the smoother and
requests a frame; `RedrawRequested` calls `app.render` rather than doing the
painting inline ([`event_handler.rs:56`](../aimer_quiver/src/handler/event_handler.rs#L56),
[`event_handler.rs:116-120`](../aimer_quiver/src/handler/event_handler.rs#L116-L120),
[`event_handler.rs:680-707`](../aimer_quiver/src/handler/event_handler.rs#L680-L707)).
The application then dispatches the smoothed scroll during `begin_frame`
([`handler.rs:402-425`](../aimer_quiver/src/handler.rs#L402-L425)), and rendering
walks the tree through `render_frame` and `build_frame`
([`handler.rs:1024-1027`](../aimer_quiver/src/handler.rs#L1024-L1027),
[`wgpu_ctx.rs:324-349`](../aimer_quiver/src/render_ctx/wgpu_ctx.rs#L324-L349)).

This makes the distinction clear: one scroll event is cheap to enqueue, while
the frame(s) it keeps alive are expensive.

## Why the retained scroll path is missed

`RawScrollableContainer::draw` performs scroll physics, notification, clipping,
visible-rect calculation, and then calls the retained-paint entry point
([`draw_scroll.rs:86-200`](../crates/aimer_scroll/src/scrollable/rendering/draw_scroll.rs#L86-L200)).
The retained implementation immediately tests stability. For an unstable child
it tries dynamic islands, then clears the cache and calls `self.child.draw` when
that partition is unavailable ([`raw_scroll.rs:475-490`](../crates/aimer_scroll/src/scrollable/core/raw_scroll.rs#L475-L490)).
The default `draw_paint_islands` also returns `false`
([`drawable.rs:25-55`](../crates/aimer_widget/src/components/drawable.rs#L25-L55)).

The relevant page structures are:

- Home: `Container` outside `Scrollable`, then a `Column` containing page
  sections ([`home_screen.rs:84-95`](../website/src/screen/home_screen.rs#L84-L95)).
- Blog: `Scrollable` directly contains a padded `Container`
  ([`blog.rs:34-43`](../website/src/screen/blog.rs#L34-L43)).
- Latest blog detail chooses the vertical layout on mobile, where `Scrollable`
  directly contains a padded `Container`; on desktop the horizontal layout
  makes `MarkdownViewer` scrollable instead
  ([`blog_detail.rs:153-161`](../website/src/screen/blog_detail.rs#L153-L161),
  [`blog_detail.rs:193-232`](../website/src/screen/blog_detail.rs#L193-L232)).

`RawFlex` can opt into stability only when all of its children are stable
([`raw_flex.rs:1140-1143`](../crates/aimer_flex/src/flex/raw_flex.rs#L1140-L1143)).
However, `RawContainer`'s `Drawable` implementation starts at
[`container.rs:262`](../crates/aimer_container/src/single_child/container.rs#L262)
and has no `is_paint_stable` override. Its inherited `false` is also justified
by the bounds mutation in [`container.rs:296-313`](../crates/aimer_container/src/single_child/container.rs#L296-L313).
Consequently, a container wrapper prevents the scroll child from being treated
as a fully replayable static paint subtree.

The scroll renderer itself documents the resulting cost: the visible rectangle
can put build, layout, shaping, highlighting, and glyph rasterization on the
frame where a line crosses the viewport boundary
([`draw_scroll.rs:176-182`](../crates/aimer_scroll/src/scrollable/rendering/draw_scroll.rs#L176-L182)).
That matches the clean trace's flex and text samples during movement.

## Reconciliation complexity

The suspicion about reconciliation is valid, with an important scope
distinction: reconciliation is rebuild-only, not part of every ordinary scroll
frame in the current implementation. `StatefulElement::rebuild_if_dirty`
returns early when the element is clean and its invalidation generation has not changed
([`stateful.rs:982-1009`](../crates/aimer_widget/src/widget/stateful.rs#L982-L1009)).
When a rebuild is required, it carries child state and then runs generated-tree
reconciliation ([`stateful.rs:1050-1059`](../crates/aimer_widget/src/widget/stateful.rs#L1050-L1059)).

### `Scrollable` is stateful, but its scroll state does not use the updater

`Scrollable<W>` does implement `StatefulWidget`; its state is
`ScrollableState<W>`, and the widget is created as a `StatefulElement`
([`scrollable.rs:406-433`](../crates/aimer_scroll/src/scrollable.rs#L406-L433)).
That state is therefore retained across reconciliation. The important detail is
that `ScrollableState::init_state` currently ignores the supplied
`StateUpdater` ([`scrollable.rs:478-480`](../crates/aimer_scroll/src/scrollable.rs#L478-L480)).

During scrolling, the live offset is held in `ScrollState` cells and the
scroll input/render paths mutate that state, request another animation frame,
and notify the optional scroll callback; they do not call
`StateUpdater::set_state` ([`controller.rs:548-570`](../crates/aimer_scroll/src/scrollable/state/controller.rs#L548-L570),
[`handle_scroll.rs:331-349`](../crates/aimer_scroll/src/scrollable/input/handle_scroll.rs#L331-L349)).
`StateUpdater::set_state` is the operation that queues a mutation, marks a
stateful element dirty, and requests a frame
([`stateful.rs:419-439`](../crates/aimer_widget/src/widget/stateful.rs#L419-L439)).
So the existence of a `Scrollable` state and updater does not, by itself, put
reconciliation on every scroll frame: the current scroll path redraws the
existing scroll container directly. An `on_scroll` callback can still mark an
ancestor dirty; Home does this only when its `SHOW_ICON` threshold changes,
which is the one-time rebuild observed in the clean trace.

If a variant of the implementation calls the `Scrollable` updater for every
offset change, the quadratic reconciliation paths below become directly
relevant to every scroll frame. That should be verified with a temporary
counter/signpost around `StateUpdater::set_state` and
`StatefulElement::rebuild_if_dirty` before attributing the sustained scroll CPU
to reconciliation.

### Structural reconciliation planner: O(n²) for keyed siblings

`collect_matches` scans `old_children` from the beginning for every keyed
`new_child` ([`reconciliation_plan.rs:201-247`](../crates/aimer_widget/src/reconciliation_plan.rs#L201-L247)).
For a sibling group of `k` keyed elements, the scan is `O(k²)` in the worst
case. Unkeyed positional matching is `O(k)` for that group. Across a tree the
cost is `O(N + Σ k²)`, with `O(N²)` worst-case behavior for a large flat keyed
group. `validate` and `apply_identities` are linear expected-time passes over
the match list ([`reconciliation_plan.rs:70-101`](../crates/aimer_widget/src/reconciliation_plan.rs#L70-L101),
[`reconciliation_plan.rs:128-133`](../crates/aimer_widget/src/reconciliation_plan.rs#L128-L133)).

### Stateful child carry: another quadratic path

`element_children` first appends `event_children`, then checks every child from
`visit_children` against the accumulated list using a linear `.any()` search
([`stateful.rs:1159-1170`](../crates/aimer_widget/src/widget/stateful.rs#L1159-L1170)).
`RawFlex` exposes the same child source through both views
([`raw_flex.rs:1220-1250`](../crates/aimer_flex/src/flex/raw_flex.rs#L1220-L1250)).
Therefore, a flex group with `k` children can cost `O(k²)` merely to build the
deduplicated child list during state carry, even when all children are unkeyed.

The keyed state pass has a separate repeated-search problem. Every keyed new
stateful element calls `find_keyed_stateful` against the same `old_root`
([`stateful.rs:1196-1220`](../crates/aimer_widget/src/widget/stateful.rs#L1196-L1220)),
and that function recursively scans the old tree
([`stateful.rs:1116-1144`](../crates/aimer_widget/src/widget/stateful.rs#L1116-L1144)).
With `K` keyed stateful nodes and an otherwise linear child enumerator, this is
`O(KN)`, or `O(N²)` when `K` grows with the tree. Because the search itself uses
the potentially `O(k²)` `element_children` operation, a contrived flat flex
list containing `O(N)` keyed stateful children can reach `O(N³)` in the current
composition. That is a worst-case bound, not the measured complexity of the
website's small number of keyed stateful nodes.

The clean scroll trace did capture `element_children`, `find_keyed_stateful`,
and keyed state-carry frames at the first Home threshold-crossing rebuild. The
trace showed this as a short rebuild stack, while the larger sampled aggregates
were widget drawing, flex/text work, and WGPU presentation. Thus:

- **Yes:** the reconciliation implementation contains real quadratic behavior,
  and a worse synthetic composition.
- **No:** it is not the explanation for sustained CPU on every Blog/Home scroll
  frame unless some callback marks the tree dirty on every frame.
- **For this workload:** Home's `SHOW_ICON` threshold rebuild pays this cost
  once; the sustained scroll spike remains the ordinary redraw/paint path
  described above.

## Secondary contributors

### Frame-synchronized smoothing

The smoother deliberately spreads one platform delta over multiple rendered
frames. `MomentumScroller::tick_with_dt` removes only a response-sized portion
of the remaining distance on each tick ([`scroll_utils.rs:66-105`](../aimer_quiver/src/handler/scroll_utils.rs#L66-L105)).
`DualScroller::tick` can emit one step per source for each rendered frame
([`scroll_classifier.rs:477-489`](../aimer_quiver/src/handler/scroll_classifier.rs#L477-L489)).
This is correct behavior, but it multiplies the redraw cost of one input pulse
when each redraw still walks and paints the page.

### Child-first event routing

`RawScrollableContainer::on_event` dispatches the event to its child before
applying the scroll frame ([`handle_scroll.rs:324-349`](../crates/aimer_scroll/src/scrollable/input/handle_scroll.rs#L324-L349)).
That can traverse the event tree and is a valid optimization target, especially
for deeply nested pages. It was not the dominant Time Profiler hotspot in the
clean trace: the event route appeared briefly, while widget draw and GPU work
occupied most of the event burst.

### Home threshold rebuild

Home's scroll callback toggles `SHOW_ICON` at the 150 px threshold and calls
`set_state` only when that boolean changes
([`home_screen.rs:50-67`](../website/src/screen/home_screen.rs#L50-L67)).
The clean event stack included the rebuild path, so the first threshold crossing
has an extra rebuild. Because the callback is edge-triggered, this is a
one-time transition cost, not the explanation for sustained scroll CPU.

### Text and first-visibility work

The renderer invokes `TextPipelineV2::prepare` whenever text or decoration
requests exist ([`renderer.rs:1433-1442`](../aimer_cupid/src/renderer.rs#L1433-L1442)).
The trace included glyph visibility/raster work and buffer uploads. Newly visible
blog lines can therefore cause a short-lived extra cost as they enter the
viewport. This is secondary to the retained-paint miss, but it explains why the
longer latest-blog detail peaks higher.

## Recommended next actions

1. Add temporary counters or signposts for `Scroll` events, smoother steps,
   `RedrawRequested`, `draw_scroll`, retained-layer reuse, retained-layer
   recording, dynamic-island fallback, full-child fallback, and
   `TextPipelineV2::prepare`. Also count reconciliation entry, child-list
   enumeration, keyed-state searches, and plan matches. The current traces
   prove the expensive path but do not expose retained-cache or reconciliation
   counts.
2. Make the paint/hit-test boundary explicit. A container whose geometry is
   updated during the draw pass cannot simply be declared stable. Separate
   bounds bookkeeping from paint, or introduce a conservative paint-only
   wrapper that delegates stability to its child only when its decoration and
   layout are immutable.
3. Reconciliation indexing is now implemented and measured above. Repeat the
   A/B run on a larger keyed list before making further changes to validate the
   asymptotic win under production-sized sibling counts.
4. Run an A/B profile with the scroll child made safely paint-stable. The
   expected signature of a successful change is: steady scrolling reuses a
   retained layer, `ElementNode::draw`/flex/text samples fall sharply, and CPU
   follows the GPU compositing cost rather than the whole page paint cost.
5. For long blog content, measure a retained static layer or a genuinely
   windowed list. Keep the current visible-rect and invalidation semantics; text,
   images, custom pipelines, and dynamic widgets may still require normal draw.
6. Only after the retained-paint A/B test, optimize child-first scroll routing
   or frame-request counts. The current code requests frames from several
   correct lifecycle points, while the platform requester is intended to
   coalesce them; count requested versus delivered frames before changing this
   behavior.

## Limitations

- Time Profiler is statistical and its inclusive samples overlap; the sample
  counts are hotspot indicators, not a CPU percentage breakdown.
- The full-workload page-to-peak mapping follows the scripted action order;
  exact phase boundaries were inferred from the timeline.
- Activity Monitor cannot identify which allocation caused the temporary
  footprint jump. An Allocations/Memory Graph run is needed to distinguish glyph
  caches, draw-list/retained-layer storage, GPU resources, and a true leak.
- Activity Monitor peak CPU is noisy in a single short run; the optimized peak
  was higher even though the duration-weighted average and CPU ledger fell.

## Captured Instruments artifacts

The raw traces remain in `/private/tmp` on the profiling machine:

```text
/private/tmp/website-time-profile-clean-scroll.trace
/private/tmp/website-time-profile-clean-scroll-table.xml
/private/tmp/website-activity-monitor-clean-scroll.trace
/private/tmp/website-activity-monitor-clean-scroll-live.xml
/private/tmp/website-time-profile-attached.trace
/private/tmp/website-activity-monitor.trace
/private/tmp/website-reconciliation-baseline-time-v4.trace
/private/tmp/website-reconciliation-baseline-activity-v3.trace
/private/tmp/website-reconciliation-after-time-v3.trace
/private/tmp/website-reconciliation-after-activity-v2.trace
/private/tmp/website-frame-baseline-v1-activity.trace
/private/tmp/website-frame-after-v1-time.trace
/private/tmp/website-frame-after-v1-activity.trace
/private/tmp/website-last-test-clean-20260830.trace
/private/tmp/website-last-test-clean-20260830-toc.xml
/private/tmp/website-last-test-clean-20260830-live.xml
/private/tmp/website-last-test-clean-20260830-ledger.xml
/private/tmp/website-last-test-clean-20260830-time.xml
```

## Final requested website traversal — Instruments run (2026-08-30)

### Run setup

The requested traversal was run against the existing macOS build at
`website/builds/macos/build/Debug/website.app`; it was not rebuilt for this
capture. Apple Instruments used a combined Time Profiler and Activity Monitor
recording attached to the verified `website` process (PID 86961).

The canvas does not expose an accessibility scroll element, so the test used
the app's native `Home` and `End` endpoint commands, which were verified to
move the actual `Scrollable` on Home and Blog:

- Home: bottom, top, bottom, top;
- Blog: bottom, top, bottom, top;
- latest blog: bottom, upward traversal, and bottom endpoint verification.

The latest-blog detail uses a nested `MarkdownViewer`/selection surface. Its
`End` bottom endpoint was verified and upward traversal was exercised, but the
final top endpoint was not independently observable in this capture because
the nested text surface consumed some `Home`/keyboard events. This is a test
observability limitation, not a claim that the scroll path failed.

### Measurements

The recording ran from `03:21:25.836` to `03:23:26.675` local time, for
`120.839 s`. Values below are from the exported
`activity-monitor-process-live` table; CPU values at or above 20% define the
active interaction samples.

| Metric | Result |
| --- | ---: |
| Live Activity Monitor rows | 116 |
| Active CPU samples | 9 |
| Active sample duration | 9.374 s |
| Active interaction CPU | 34.58% duration-weighted |
| Peak CPU | 45.24% |
| CPU time during trace | 4.872 s |
| Whole-trace CPU share | 4.03% |
| Physical footprint | 102.42–247.38 MiB; 116.27 MiB at the final sample |
| Real memory | 156.66–192.94 MiB; 157.39 MiB at the final sample |
| Time Profiler samples | 4,821 |

The highest CPU interval was a transient approximately 1.019-second sample at
`00:13.892`, reaching 45.24% CPU. The final live samples were below 0.1% CPU,
so this capture does not show sustained high CPU while the website was idle
after the interaction bursts. The Time Profiler export contains the trace
sample data, but website frames were mostly unsymbolicated addresses in this
existing build; it should not be used alone to attribute the burst to a
specific framework function.

### Conclusion

This run confirms that the resource cost is bursty during scroll/navigation
activity rather than continuously high on an idle Home or Blog screen. The
peak physical footprint reached 247.38 MiB during the workload and settled to
116.27 MiB at the end of the capture. These are single-run measurements and
are not a controlled A/B comparison with the earlier traces; repeat captures
with symbolicated binaries and explicit phase markers are required before
claiming a percentage improvement for the latest hit-test, path-storage, or
MouseRegion changes.
