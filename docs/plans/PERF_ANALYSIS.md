# Overall Performance Bottoleneck

 ### 1. Every pointer event still walks the whole routed tree — the per-event CPU core

 EventDispatcher::route → dispatch_routed_event → dispatch_routed_event_inner
 (crates/aimer_widget/src/components/element.rs:2360). Flex/grid/stack prune by painted range + bounds
 (raw_flex.rs:1291, stack.rs:340), which is good, but the walk visits every painted child whose bounds contain
 the cursor, at every level, for every event.

 - PointerMove is rarely consumed, so the stopped early-exit almost never fires — the walk bottoms out over the
   full overlapping set every time.
 - Winit feeds CursorMoved at input rate (only a sub-pixel filter in event_handler.rs:314), so a screen with a
   few thousand visible elements = hundreds of thousands of contains()/pos_start_end() virtual calls per second
   while the cursor merely crosses the window.
 - There is no spatial index (R-tree/quadtree) and no reuse of the previous event's hit chain; each event
   recomputes it from scratch.

 This is the biggest remaining lever. Options: (a) cache the per-pointer hit chain between consecutive moves and
 revalidate against cached bounds (cheap rejection before a full walk), (b) add a coarse screen-space
 grid/interval index for very wide lists, (c) for moves, deliver to the topmost chain only — you'd need a hover
 contract for overlapping widgets, but that's what the fast frameworks do.

 ### 2. The generation system still collapses to global for real roots → O(N) reindex + focus walk once per frame
 under any animation

 - LayoutElement::is_layout_stable defaults to false (layout_element.rs:66) and
   StatefulElement/StatelessElement/RawFlex don't override it, so for the actual app root subtree_generation()
   falls back to the global element_tree_generation() (element.rs:~604).
 - So when any widget rebuilt during the last frame — an animation tick, a hover-triggered rebuild, a counter —
   the first event of the next epoch hits synchronize_paths dirty and runs:
     - index_element_links — full-tree walk, N HashMap inserts (std SipHash) into path_indices
       (element.rs:~1994);
     - collect_focus_candidates — second full-tree walk with FocusNode Rc clones per focusable
       (synchronize_focus, element.rs:~2048);
     - then the routed walk from #1.

 Your epoch coalescing bounded this to once per frame (previously per event), but it's still O(N) work per frame
 during any animation, and rebuilds that changed only state (a label's text) still invalidate the structural
 index. reconciliation_plan.rs already computes matches; a cheap follow-up: skip advance_element_tree_generation
 when the plan proves the tree is path-equivalent (same child counts, same keys/types at every level), or record
 subtree_generation stamps for non-layout-stable roots so other windows' rebuilds stop invalidating this one.

 ### 3. Captured-move resolution is O(sibling index) per event

 resolve_element_path → structural_child_at (element.rs:1322) linear-scans structural_children per level on every
 captured move/up (drags, scroll gestures). The owner of a drag is usually shallow (scrollable), but dragging a
 row in a large list — owner at index 5000 — costs ~5000 sibling visits per mouse move. Same for resolve_owner on
 every keystroke while a deep field is focused.

 Fix: during index_element_links you already visit every element — store the resolved child reference in the
 arena (raw pointer + element_id re-validation, safe because the tree is immutable between reindex and dispatch
 on the UI thread), or add an EventElement::nth_structural_child O(1) accessor for Vec-backed containers.

 ### 4. Nested dispatch boundaries multiply the walk

 Every MouseRegion (buttons, hoverables) calls context.dispatch_child (mouse_region.rs:~288) → dispatch_nested →
 a fresh routed walk of its subtree per event, plus its own bounds/hover checks. Nested regions = O(depth ×
 subtree) per move. The single-dispatcher sharing removed per-instance index maps (good) but not the duplicated
 walks. Consider flattening: hit-test the whole tree once in the dispatcher and hand each boundary its
 pre-resolved child path.

 ### 5. Debug-build bookkeeping (if you measure in debug, this is the blackhole)

 - record_paint_element inserts into a thread-local HashSet per drawn element per frame when paint tracking is
   active (scroll content draws — raw_scroll.rs:154), ungated in release.
 - ElementNode::draw → record_draw_traversal, REBUILD_PATH RefCell Vec push per visited node per frame.
 - Recommend measuring with the frame-stats feature / release build to separate these from the real costs.

 ### 6. Minor, worth folding in while you're here

 - EventDispatcher::dispatch → settle_focus_requests calls synchronize_paths a second time per event after route
   already did (gated, but redundant).
 - path_indices reindex uses std HashMap SipHash — hashbrown is already a workspace dep (aimer_focus uses it).
 - Default structural_children dedups event∪visual children with an O(k²) pointer scan per element during reindex
   (event_element.rs:301).
 - cancel_captures allocates a HashSet per Cancel — rare, fine.

 Priority: #1 (per-event walk) is the CPU black hole during interaction; #2 is the per-frame O(N) churn during
 animation; #3 fixes drags/typing in big lists; #5 explains debug-build perception.

 Want me to (a) add instrumentation to count element visits per event/frame so you can confirm these on your
 actual app, or (b) start implementing #2's path-equivalence skip or #3's arena-resolved pointers?
 
 
##  Proves

**Audit date:** 2026-08-30

**Method:** current-source call tracing, complexity inspection, and serial unit
tests. Source inspection proves control flow and asymptotic behavior; the
claimed CPU percentages and event rates require a fresh Instruments trace and
are not inferred from unit-test timings.

| Item | Result |
| --- | --- |
| 1. Routed pointer walk | **Partially confirmed** |
| 2. Generation and focus invalidation | **Confirmed for ordinary unstable roots; mitigated for stable roots** |
| 3. Captured-path resolution and per-element path allocation | **Allocation claim disproven; sibling-scan claim confirmed** |
| 4. Nested dispatch boundaries | **Dispatch mechanism confirmed; exact multiplied cost is workload-dependent** |
| 5. Debug/paint bookkeeping | **Partially confirmed** |
| 6. Minor costs | **Mixed: three confirmed, one disproven** |

### 1. Routed pointer walk

The core walk is present. `EventDispatcher::route` sends uncaptured pointer
events to `dispatch_routed_event`, and `dispatch_routed_event_inner` rejects an
outside parent, asks the element for position-aware children, and recursively
dispatches every child it receives. See
[`element.rs:1799-1870`](../crates/aimer_widget/src/components/element.rs#L1799)
and
[`element.rs:2368-2429`](../crates/aimer_widget/src/components/element.rs#L2368).

The original wording is too strong in three ways:

- Captured move/up/exit events take the `dispatch_captured` branch and resolve
  only the saved path; they do not run the full hit-test walk
  ([`element.rs:1861-1866`](../crates/aimer_widget/src/components/element.rs#L1861)).
- The walk is not an unconditional whole-tree walk. `RawFlex`, `Stack`, and
  `RawGrid` provide position-aware hit-test implementations that apply painted
  ranges, retained bounds, or the stack's y-index:
  [`raw_flex.rs:1292-1345`](../crates/aimer_flex/src/flex/raw_flex.rs#L1292),
  [`stack.rs:461-480`](../crates/aimer_space/src/space/stack.rs#L461), and
  [`raw_grid.rs:714-739`](../crates/aimer_grid/src/grid/raw_grid.rs#L714).
- There is no general previous-hit-chain cache or R-tree/quadtree in
  `EventDispatcher`; its retained fields are capture ownership and the
  structural path index
  ([`element.rs:1531-1542`](../crates/aimer_widget/src/components/element.rs#L1531)).

The one-pixel native cursor filter is confirmed at
[`event_handler.rs:320-322`](../aimer_quiver/src/handler/event_handler.rs#L320),
and the early-stop path exists when a child consumes an event
([`element.rs:2418-2427`](../crates/aimer_widget/src/components/element.rs#L2418).
Thus, **uncaptured pointer moves still perform a candidate-tree walk per
event**, but the “every pointer event walks the whole tree” sentence and the
“hundreds of thousands of calls per second” number are not proven as universal
claims.

### 2. Generation invalidation and focus walk

This remains a real cost for ordinary application roots:

- `LayoutElement::is_layout_stable` defaults to `false`
  ([`layout_element.rs:57-67`](../crates/aimer_widget/src/components/layout_element.rs#L57)).
  `StatefulElement`, `StatelessElement`, and `RawFlex` do not override it, so
  the erased `ElementNode` falls back to the global generation at
  [`element.rs:667-673`](../crates/aimer_widget/src/components/element.rs#L667).
- Every generated-tree commit currently advances the global generation at
  [`element.rs:1415-1419`](../crates/aimer_widget/src/components/element.rs#L1415).
  Stateful and stateless rebuilds call that commit path at
  [`stateful.rs:1052-1061`](../crates/aimer_widget/src/widget/stateful.rs#L1052)
  and
  [`stateless.rs:248-259`](../crates/aimer_widget/src/widget/stateless.rs#L248).
- When the observed root generation changes, `synchronize_paths` clears the
  retained index and recursively calls `index_element_links`, which performs
  one link append and one `HashMap` insertion per indexed element
  ([`element.rs:1986-2037`](../crates/aimer_widget/src/components/element.rs#L1986)).
- The changed indexed generation also makes focus synchronization run; it then
  recursively collects focus candidates through `structural_children`
  ([`element.rs:2048-2082`](../crates/aimer_widget/src/components/element.rs#L2048)
  and
  [`element.rs:2248-2255`](../crates/aimer_widget/src/components/element.rs#L2248)).

The mitigation is also proven: `synchronize_paths` compares the root's local
`subtree_generation`, and `dispatcher_reindexes_at_most_once_per_event_frame`
passes while an unrelated global generation bump leaves a stable root's index
unchanged, then re-indexes once after the root generation changes in the next
frame ([`element.rs:2956-2997`](../crates/aimer_widget/src/components/element.rs#L2956)).
The remaining conclusion is therefore **O(N) re-index/focus work per changed
frame for unstable roots, not for every stable root**.

### 3. Captured-path resolution and path allocation

The former “one heap `Box<[usize]>` per element” statement is no longer true.
`ElementPath` stores only `parent: Option<usize>` and `child_index: u32` in a
reusable `Vec`, while the ID map stores the link index
([`element.rs:1471-1474`](../crates/aimer_widget/src/components/element.rs#L1471)
and
[`element.rs:2276-2304`](../crates/aimer_widget/src/components/element.rs#L2276)).
There are still O(N) link appends and ID-map inserts during a re-index, but not
N boxed path slices.

The captured-path complexity claim is confirmed. `resolve_element_path` walks
the saved parent links, then calls `structural_child_at` for every path segment;
`structural_child_at` enumerates the structural children until it finds the
requested index
([`element.rs:1366-1377`](../crates/aimer_widget/src/components/element.rs#L1366)
and
[`element.rs:2307-2331`](../crates/aimer_widget/src/components/element.rs#L2307)).
The cost is O(sum of sibling counts along the path), with a worst case of
O(N), including captured moves and focused-key delivery. The existing test
confirms the saved-path behavior and avoids unrelated siblings, but does not
change that sibling-index complexity:
[`element.rs:3553`](../crates/aimer_widget/src/components/element.rs#L3553).

### 4. Nested dispatch boundaries

The duplicated-boundary mechanism is confirmed. `MouseRegion` deliberately
exposes no `event_children`; its contextual handler forwards the child through
`EventDispatchContext::dispatch_child`
([`mouse_region.rs:339-352`](../crates/aimer_input/src/mouse_region.rs#L339)
and
[`mouse_region.rs:287-291`](../crates/aimer_input/src/mouse_region.rs#L287)).
That context shares the owning `EventDispatcher`, but an uncaptured nested
dispatch invokes a new `dispatch_routed_event_inner` traversal for the child
([`element.rs:1483-1524`](../crates/aimer_widget/src/components/element.rs#L1483)
and
[`element.rs:1649-1698`](../crates/aimer_widget/src/components/element.rs#L1649)).

Therefore shared state removed the per-region dispatcher/path-map allocation,
but it did not flatten nested routed walks. The exact `O(depth × subtree)` cost
depends on how many nested boundaries and candidate descendants are present;
the source proves the repeated traversal mechanism, while a workload trace is
needed to assign a numeric multiplier.

### 5. Debug and paint bookkeeping

The following parts are confirmed:

- `record_draw_traversal` is compiled only for debug or `frame-stats`, and
  increments once per drawn `ElementNode`
  ([`element.rs:202-218`](../crates/aimer_widget/src/components/element.rs#L202)
  and
  [`element.rs:827-831`](../crates/aimer_widget/src/components/element.rs#L827)).
- `record_paint_element` is called from every draw, and inserts into the
  thread-local `HashSet` when retained-paint tracking is active
  ([`element.rs:147-174`](../crates/aimer_widget/src/components/element.rs#L147)
  and
  [`raw_scroll.rs:837-846`](../crates/aimer_scroll/src/scrollable/core/raw_scroll.rs#L837)).
- `REBUILD_PATH` is a `RefCell<Vec<ElementId>>` pushed and popped during the
  rebuild traversal, including the traversal initiated by the outer draw
  ([`element.rs:60`](../crates/aimer_widget/src/components/element.rs#L60),
  [`element.rs:827-838`](../crates/aimer_widget/src/components/element.rs#L827),
  and
  [`element.rs:462-475`](../crates/aimer_widget/src/components/element.rs#L462)).

The former framework inspector snapshot and overlay paths are no longer part of
the implementation. Existing Instruments data in `PERFORMANCE_REPORT.md`
supports `ElementNode::draw` as a scroll hot path, but it does not isolate the
remaining bookkeeping counters as the cause.

### 6. Minor costs

- **Confirmed, low impact:** `dispatch` calls `route` and then
  `settle_focus_requests`, which calls `synchronize_paths` a second time
  ([`element.rs:1628-1640`](../crates/aimer_widget/src/components/element.rs#L1628)
  and
  [`element.rs:1792-1795`](../crates/aimer_widget/src/components/element.rs#L1792)).
  The second call is normally only the frame/root comparison because the
  re-index is already clean.
- **Disproven as written:** `path_indices` uses `hashbrown::HashMap`, not
  `std::collections::HashMap`/SipHash
  ([`element.rs:1-14`](../crates/aimer_widget/src/components/element.rs#L1)).
- **Confirmed worst case:** the default `structural_children` implementation
  scans the accumulated child list with `.any(std::ptr::eq(...))` for every
  visual child, which is O(k²) when the event and visual child views overlap
  heavily ([`event_element.rs:301-315`](../crates/aimer_widget/src/components/event_element.rs#L301)).
  Containers with one canonical child source can override it.
- **Confirmed, rare:** `cancel_captures` collects captured owners into a new
  `HashSet` for each cancellation
  ([`element.rs:2208-2223`](../crates/aimer_widget/src/components/element.rs#L2208)).

### Verification run

All checks below passed:

- `cargo test -p aimer_widget --lib -- --test-threads=1`: **244 passed**.
- `cargo test -p aimer_focus --lib -- --test-threads=1`: **33 passed**.
- `cargo test -p aimer_flex -p aimer_grid -p aimer_space --lib -- --test-threads=1`:
  **88 + 15 + 9 passed**; 4 intentionally ignored profiling tests.
- Focused tests for once-per-frame re-indexing and position-aware routed
  dispatch: **1 passed each**.
- `git diff --check`: passed.

No new Instruments run was performed for this audit, so the numeric rate and
CPU-impact statements above remain hypotheses until measured against the
website release build.

### Re-verification (second pass, same date)

The audit was re-run against the current working tree to prove the claims once
more. Every structural claim above was re-checked at its cited location, and
the cited line anchors remain valid with at most a few lines of drift:

| Anchor | Audit cite | Current |
| --- | --- | --- |
| `EventDispatcher::route` | element.rs:1799 | :1799 ✓ |
| `dispatch` / `settle_focus_requests` double sync | element.rs:1628 / :1792 | :1623 / :1792 ✓ |
| `synchronize_paths` re-index gate | element.rs:1986 | :1986 ✓ |
| `cancel_captures` HashSet | element.rs:2208 | :2208 ✓ |
| `index_element_links` arena walk | element.rs:2276 | :2276 ✓ |
| `resolve_element_path` / `structural_child_at` sibling scan | element.rs:1366 / :2307 | :1367 / :2307 ✓ |
| `dispatch_routed_event_inner` per-event walk | element.rs:2368 | :2368 ✓ |
| `dispatch_nested` boundary re-walk | element.rs:1649 | :1649 ✓ |
| `path_indices` hashbrown (SipHash claim disproven) | element.rs:1-14 | :13 ✓ |
| once-per-frame epoch test | element.rs:2956 | :2956 ✓ |
| flex / stack / grid position-aware hit tests | 1292 / 464 / 714 | 1292 / 464 / 714 ✓ |
| `MouseRegion` `dispatch_child` forwarding | mouse_region.rs:287-291 | :288 ✓ |
| default `structural_children` O(k²) union | event_element.rs:301 | :301 ✓ |

Re-run verification, all green:

- `cargo test -p aimer_widget --lib -- --test-threads=1`: **244 passed** (reproduced).
- `cargo test -p aimer_focus --lib -- --test-threads=1`: **33 passed** (reproduced).
- `cargo test -p aimer_flex -p aimer_grid -p aimer_space --lib -- --test-threads=1`:
  **88 + 15 + 9 passed**, 4 intentionally ignored profiling tests (reproduced).
- Focused once-per-frame re-index test: **1 passed** (reproduced).
- `git diff --check`: clean.

Conclusion of the second pass: findings 2 and 3 stand as written; finding 1
stands for *uncaptured* events with the painted-candidate walk (the "whole
tree" wording remains disproven); finding 4's mechanism is confirmed with the
multiplier left workload-dependent; finding 6 is unchanged. No new measurements
were taken, so the CPU impact estimates remain hypotheses until an Instruments
or frame-stats run on the release build.

### Optimization Path

Ordered by expected impact. Each step names the finding it addresses, the
mechanism, the acceptance test, and the risk. Steps A1–A3 attack the per-event
CPU core (finding 1); steps B1–B2 scope invalidation (finding 2); C1 removes
the sibling scan (finding 3); D1 flattens nested boundaries (finding 4); E1–E2
separate debug cost from release cost (finding 5); F1–F3 fold in the minor
items (finding 6). A measurement gate is proposed first, so every step is
verified with a number rather than a claim.

#### Gate: element-visit instrumentation

Add a frame-stats-gated counter next to `DRAW_TRAVERSAL_COUNT`: one increment
per element reached by `dispatch_routed_event_inner` and one per element
reached by `index_element_links` / `collect_focus_candidates`, reset per frame.
The `frame-stats` feature already exists for exactly this shape of
instrumentation ([`element.rs:202-218`](../crates/aimer_widget/src/components/element.rs#L202)).
Every step below reports its win through this counter on the showcase/website
release build, so "faster" is measured, not asserted.

#### ~~1. Per-pointer hit-chain cache in `EventDispatcher` (finding 1)~~

Cache the routed hit chain of the last delivered pointer event — the chain of
`ElementId`s or link indices returned by the walk — together with the pointer
position. On the next `PointerMove` for the same pointer, revalidate the chain
top-down against cached bounds (`pos_start_end`), which is O(depth) cheap
`contains` checks; fall back to the full walk only when a chain entry no longer
contains the position, when the indexed generation changed (already detected by
`synchronize_paths`), or on capture/exit transitions.

- **Why:** uncaptured moves are the dominant event and rarely consume, so the
  early-stop in
  [`element.rs:2418-2427`](../crates/aimer_widget/src/components/element.rs#L2418)
  almost never fires; a cached chain turns the common case from O(painted
  candidates) into O(depth).
- **Acceptance:** the instrumentation gate shows element visits per move
  ≈ chain depth instead of painted-candidate count; all existing routed-dispatch
  and capture tests in `element.rs` stay green.
- **Risk:** hover side-effects on widgets that overlap the chain but are not on
  it must keep receiving moves if they already consume them today — preserve
  the full walk for any pointer whose last event was consumed, or any tree that
  opts into overlapping-hover semantics (the contract that makes finding 1's
  option (c) safe).

#### ~~A2. Interval-indexed child rejection for very wide containers (finding 1)~~

For `Row`/`Column` (`RawFlex`) above a sibling threshold (suggest 64), keep the
retained per-child bounds sorted on the main axis — the y-index in
[`stack.rs:461-480`](../crates/aimer_space/src/space/stack.rs#L461) is the
proven pattern — and consult it in `visit_painted_children_at` before touching
`child.pos_start_end()`. Painted-range pruning already exists
([`raw_flex.rs:1292-1345`](../crates/aimer_flex/src/flex/raw_flex.rs#L1292));
the index removes the remaining per-child bounds call inside the painted range.

- **Acceptance:** a 10k-row column hit-tests in O(log n + overlap) child visits;
  a regression test mirrors the stack's index tests (order, unknown bounds,
  index rebuild after invalidation).
- **Risk:** index staleness — invalidate on layout/bounds change exactly as
  `stack.rs` does, reusing its cache-generation pattern.

#### A2 Result

Implemented the interval index in [`FlexLayoutCache`](../crates/aimer_flex/src/flex/flex_layout.rs#L826)
and wired it into [`RawFlex::draw`](../crates/aimer_flex/src/flex/raw_flex.rs#L982)
and position-aware hit testing ([`raw_flex.rs`](../crates/aimer_flex/src/flex/raw_flex.rs#L1332)).
For painted ranges larger than 64 children, the cache builds 64 main-axis bins
from retained child bounds after painting. A routed pointer event consults one
bin and still performs the exact bounds check on its candidates. Unknown,
dense, stale, or invalidated bounds use the existing exact fallback.

The focused regression test was run before and after the change:

- Before: **failed as intended**; the 128-child column queried `128 of 128`
  child bounds during hit testing.
- After: **passed**; the same test found the target and queried fewer than all
  128 child bounds.

##### Website scroll capture

The existing Debug website app was rebuilt with the optimized Rust library and
relinked. Each run used the same 45-second Activity Monitor trace and the same
interaction: reset to the top, scroll to the bottom, scroll to the top, scroll
to the bottom. Computer Use screenshots verified the final bottom endpoint.
The website canvas did not expose an accessibility scroll element, so the
scroll gestures were delivered through the native macOS wheel event path after
the Computer Use state/screenshot checks.

| Metric | Before A2 | After A2 | Change |
| --- | ---: | ---: | ---: |
| Activity Monitor CPU time (ledger) | 27.661 s | 20.966 s | -6.695 s (-24.2%) |
| Peak process CPU | 61.283% | 58.812% | -2.471 pp (-4.0%) |
| Busy-sample CPU, duration-weighted (samples >=20%) | 47.161% | 47.098% | -0.063 pp (-0.13%) |
| Peak physical footprint | 226.001 MiB | 225.939 MiB | -0.063 MiB (-0.03%) |
| End-of-trace physical footprint | 184.939 MiB | 120.532 MiB | -64.406 MiB (-34.8%) |

The CPU ledger is lower in this single before/after run, while the CPU level
when the process was busy is effectively unchanged. That means the result is
consistent with less time spent doing work, but it does not by itself prove a
24.2% per-event hit-test speedup; repeated runs and the event-visit counter
are still needed to isolate A2 from run-to-run timing and workload variance.
Peak memory is unchanged, as expected for an input-path optimization. The
end-of-trace memory difference is timing-sensitive and should not be treated
as a memory improvement.

Artifacts:

- Before trace: `/private/tmp/website-a2-before-v3-activity.trace`
- After trace: `/private/tmp/website-a2-after-v2-activity.trace`
- Before process: PID 13849; after process: PID 24069
- Validation: `cargo test -q -p aimer_flex --lib -- --test-threads=1` — **89
  passed, 4 ignored**; `cargo test -q -p aimer_space --lib -- --test-threads=1`
  — **9 passed**; website `xcodebuild` — **BUILD SUCCEEDED**; `git diff
  --check` — **clean**.

#### A3. Move-delivery contract (finding 1, option c) — reverted

Add a `wants_move_events` capability (default off) on `EventElement`. When no
hit candidate asks for move events, `dispatch_routed_event_inner` stops after
the topmost chain — structurally, instead of incidentally on consumption.
Widgets that track hover (mouse regions, gesture detectors) opt in, and their
presence forces the current walk.

- **Acceptance:** moves over a static UI visit only the topmost chain; moves
  over a hover-reactive UI keep today's semantics.
- **Risk:** silently changing delivery for an existing widget that consumes
  moves without opting in — ship the flag with a lint/test that enumerates
  `ElementEvent::PointerMove` consumers.

#### A3 Result — reverted

The move-delivery experiment was implemented and then reverted. Its default-off
capability, per-move candidate pre-scan, and broad widget opt-ins did not produce
a reliable application-level improvement and added overhead on the website's
scrolling path. The current source no longer contains this A3 contract or its
widget opt-ins; A1/A2 remain in place.

The dispatcher regression coverage is in [`dispatch_routed_event_inner`](../crates/aimer_widget/src/components/element.rs#L2805):

- The two A3-only dispatcher tests were removed with the reverted contract.
- The focused pre-change failure and the experimental after-change pass remain
  historical evidence only; they are not part of the current test suite.

##### Website scroll capture

The existing Debug website app was rebuilt and relinked from the updated Rust
library. Each valid run used a 45-second Activity Monitor trace and the same
interaction: reset to the top, scroll to the bottom, scroll to the top, scroll
to the bottom. Computer Use verified the final screenshot at the bottom
endpoint. The canvas exposed no accessibility scroll element, so the wheel
gestures were delivered through native macOS events after the Computer Use
state/screenshot checks.

| Metric | Before A3 | After A3 | Change |
| --- | ---: | ---: | ---: |
| Activity Monitor CPU time (ledger) | 4.917 s | 23.943 s | +19.026 s (+387.0%) |
| Busy-sample CPU, duration-weighted (samples >=20%) | 39.161% | 46.843% | +7.681 pp (+19.6%) |
| Peak process CPU | 47.139% | 57.949% | +10.810 pp (+22.9%) |
| Peak physical footprint | 220.735 MiB | 222.939 MiB | +2.203 MiB (+1.0%) |
| End-of-trace physical footprint | 122.173 MiB | 182.423 MiB | +60.250 MiB (+49.3%) |

This single scroll capture did not show a favorable aggregate resource delta:
the after trace remained busy for 36.156 seconds versus 4.169 seconds before,
while the final screenshot reached the same bottom position. The workload is
dominated by the website's scroll/momentum and rendering behavior, and the
trace does not isolate pointer-move routing; therefore these numbers are
reported as an inconclusive application-level result rather than attributing
the increase solely to the A3 walk. Because the implementation added a
per-move pre-scan while common website containers opted into the full walk, the
experiment was reverted instead of being retained as a performance change.

Artifacts:

- Before trace: `/private/tmp/website-a3-before-v1-activity.trace` (PID 31152)
- Experimental after trace (reverted): `/private/tmp/website-a3-after-v3-activity.trace` (PID 42543)
- Final after screenshot: `/var/folders/xm/cw78mz1d45n94jr8yznpsqfh0000gn/T/com.openai.sky.CUAService/website Screenshot 2026-08-30 at 11.23.49 PM.jpeg`
- Validation: website `cargo check` and `cargo build` — **passed**; website
  `xcodebuild` — **BUILD SUCCEEDED**; `aimer_widget` — **249 passed**;
  affected crate suites — **no failures**; `git diff --check` — **clean** after
  the revert.

#### B1. Path-equivalence skip in reconciliation (finding 2)

`plan_element_reconciliation` already pairs every position and key. When the
plan proves the generated tree is path-equivalent — same child count at every
matched pair, every match `Root`/`Keyed`/`Positional` with compatible
identities — skip `advance_element_tree_generation` for the *structural* index
and advance only a separate state generation. State-only rebuilds (a label's
text, a counter, most animation ticks) then leave `synchronize_paths` cold,
removing the O(N) re-index + focus-candidate walk from animated frames
entirely.

- **Acceptance:** a new test rebuilds a stateful widget with an unchanged
  structure and asserts `EventDispatcher::synchronize_paths` performs no
  `index_element_links` walk; the existing
  `dispatcher_reindexes_at_most_once_per_event_frame`
  ([`element.rs:2956`](../crates/aimer_widget/src/components/element.rs#L2956))
  and all identity-reconciliation tests stay green.
- **Risk:** a structurally identical but semantically new subtree (key reuse,
  focus-node replacement) must still invalidate *focus* — keep focus
  synchronization on the state generation.

#### B2. Per-root generation stamps for every root (finding 2)

Stop falling back to the global counter for non-layout-stable roots: record
`set_subtree_generation` in `STABLE_SUBTREE_GENERATIONS` for every
`ElementNode`, dropping the `is_layout_stable` gate in
[`element.rs:667-673`](../crates/aimer_widget/src/components/element.rs#L667),
and only fall back to the global generation when no stamp exists. A second
window, a modal's dispatcher, or an overlay tree then stops invalidating
unrelated dispatchers' indexes.

- **Acceptance:** two dispatchers over two roots — a rebuild in one root leaves
  the other's index untouched (unit test with the `StableRoot` fixture already
  present in `element.rs` tests).
- **Risk:** stamps must still bubble to the root when a descendant rebuilds
  during draw, which the existing before/after generation compare in
  `ElementNode::draw` already does.

#### C1. O(1) captured-target resolution (finding 3)

`resolve_element_path` currently pays O(sum of sibling indexes) through
`structural_child_at` ([`element.rs:1367`](../crates/aimer_widget/src/components/element.rs#L1367))
on every captured move and focused key. Two options, in order of preference:

1. During `index_element_links`, store the child element pointer per link
   (`*const dyn Element`), validated by `element_id` on use. Safe under the
   existing invariant: the tree is immutable between re-index and dispatch on
   the single UI thread, and any rebuild bumps the generation, forcing a
   re-index before the next resolution.
2. Add `EventElement::nth_structural_child(&self, index) -> Option<&dyn
   Element>` with the current scan as default, overridden O(1) by
   `RawFlex`/`Stack`/`RawGrid`/`Container` retained children.

- **Acceptance:** a drag with the owner at sibling index 5000 resolves in
  constant time (instrumentation or a direct-call-count test fixture); all
  capture tests, including
  `invalid_saved_path_clears_capture_without_falling_back`, stay green.
- **Risk:** option 1 is `unsafe` and must document the single-threaded
  re-index-before-use invariant; option 2 is safe but touches every container.

#### D1. Flatten nested boundary walks (finding 4)

Instead of each `MouseRegion` invoking `dispatch_child` → `dispatch_nested` →
a fresh `dispatch_routed_event_inner` for its subtree
([`element.rs:1649-1698`](../crates/aimer_widget/src/components/element.rs#L1649)),
record the (boundary, child) pairs the outer walk passes through and deliver
each boundary's event from the one outer traversal. The shared
`EventDispatchContext` already carries the dispatcher and path root, so the
change is local to `dispatch_nested` and its callers.

- **Acceptance:** a tree of N nested regions performs one walk per event
  (measured by the gate in the Gate section), with capture/nested-capture
  semantics unchanged — the existing `nested_captures` tests are the
  regression suite.
- **Risk:** hover state currently observed mid-walk by sibling boundaries;
  ordering must be preserved by delivering recorded boundaries in traversal
  order.

#### E1. Release-mode parity check (finding 5)

Run the showcase under `--release` with `frame-stats` and compare against a
`debug` run before optimizing further: `record_paint_element` inserts per drawn
element while scroll paint-tracking is active
([`raw_scroll.rs:837-846`](../crates/aimer_scroll/src/scrollable/core/raw_scroll.rs#L837)),
and the removed framework inspector path is no longer part of the comparison.

- **Acceptance:** the frame-stats report names the top draw-phase costs with
  the debug bookkeeping present and absent.
- **Risk:** none — measurement only; it decides whether E2 is worth doing.

#### E2. Gate paint-tracking inserts behind `frame-stats` or an active tracker

`record_paint_element` already no-ops without a paint-tracking stack
([`element.rs:147-174`](../crates/aimer_widget/src/components/element.rs#L147));
if E1 shows the remaining cost matters in release, move the per-element
`HashSet` insert behind `feature = "frame-stats"` in release builds where
paint tracking is not required for correctness (scroll retained layers are
correctness-relevant — confirm before gating).

- **Acceptance:** unchanged golden rendering for scroll retained layers;
  draw-phase cost drops by the measured insert cost.
- **Risk:** scroll layer reuse depends on the tracked element ids — gate only
  the *recording*, never the reuse decision, and keep it on in debug.

#### F1. Drop the double `synchronize_paths` per dispatch (finding 6)

`dispatch` runs `route` (which synchronizes) and then
`settle_focus_requests`, which synchronizes again
([`element.rs:1623-1640`](../crates/aimer_widget/src/components/element.rs#L1623),
[`element.rs:1792-1795`](../crates/aimer_widget/src/components/element.rs#L1792)).
Have `settle_focus_requests` reuse the route's result or skip the path sync when
the frame/root pair is unchanged.

- **Acceptance:** a counter test proves one generation check per dispatch; all
  focus-request tests stay green.
- **Risk:** none — pure redundancy removal.

#### F2. O(k) default `structural_children` union (finding 6)

The default dedup is O(k²) (`event_element.rs:301-315`). Replace the pointer
scan with a linear two-pass visit that marks seen children in a reusable
thread-local set, or have the few default-dependent elements adopt the
union once and cache it.

- **Acceptance:** re-index of a wide default-traversal container does not grow
  quadratically (direct-call-count test fixture, like the existing
  `StructuralTraversalElement` tests).
- **Risk:** child pointer identity must remain the dedup key to preserve the
  event∪visual contract.

#### F3. Reuse the cancellation set (finding 6, optional)

`cancel_captures` allocates a `HashSet` per cancellation
([`element.rs:2208-2223`](../crates/aimer_widget/src/components/element.rs#L2208)).
Keep one in `EventDispatcher` and `clear()` it between cancellations.

- **Acceptance:** existing cancellation tests green; no allocation per cancel.
- **Risk:** none — rare path, fold in whenever `EventDispatcher` is touched for
  A1/C1 anyway.

#### Sequencing

A1 → B1 → C1 are independently shippable and deliver the three largest wins
(per-event walk, per-frame re-index, per-move sibling scan). D1 and F1/F2 are
cheap follow-ups that compound on A1. E1 is the measurement gate run before and
after each phase; E2 only if E1 names the bookkeeping as a release cost. Each
step lands with its acceptance test first, per the repository's red-green
order, and the `frame-stats` gate keeps every claim numeric.
