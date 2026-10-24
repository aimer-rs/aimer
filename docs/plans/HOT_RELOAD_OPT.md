# Hot Reload Optimization Plan

## Problem statement

Hot-reload callbacks currently execute application Rust code inside the Wasmi guest and exchange Widget IR with the
native host. This is correct for code isolation and reloadability, but the current design treats many operations as full
guest rebuilds.

Reported behavior in `website` (to be independently measured in Phase 0):

- visual updates in the `same_looking.rs` section are reported at roughly 0.5–2 seconds per frame instead of approaching
  60 FPS; the current source does not establish that its portable `AnimatedSwitcher` is the cause.
- `get_started_button.rs` takes roughly 0.8 seconds to respond even though its callback does not change UI state or
  contain an animation.

A 60 FPS target leaves approximately 16.7 ms for the complete frame. The current hot-reload animation path can exceed
that target when a full Widget IR build, WASM memory transfer, native materialization, reconciliation, and layout each
occur on every animation frame.

The timings above are performance hypotheses, not repository-verified facts. The control-flow findings below are
code-confirmed; Phase 0 must establish the baseline, identify the dominant phase, and record the build profile and
hardware used for comparison.

## Current execution paths

### Ordinary callback

```text
native widget event
  -> native closure captures StableId128
  -> bounded callback queue
  -> event-loop wake
  -> application safe point
  -> CallbackEvent encoding and validation
  -> Wasmi aimer_dispatch_event
  -> guest callback body
  -> optional Widget IR output
  -> native materialization/reconciliation
```

The native callback adapter is installed in
`aimer_quiver/src/hot_reload.rs` (`bind_button_callbacks`). The safe-point dispatch is implemented in
`aimer_quiver/src/hot_reload/live.rs`.

### Portable animation

`ImplicitAnimatedBuilder` lowers its current child into the portable guest and stores animation state in
`PortableBuildContext`. While active, it calls
`PortableBuildContext::request_frame()` on every guest build.

The native host sees that frame request through `has_async_work()`. At the next safe point it calls `poll_async()`,
receives a new complete Widget IR image, and calls `install_widget_image()`:

```text
guest animation tick
  -> request_frame
  -> native poll_async
  -> complete guest Widget IR rebuild
  -> WASM/native buffer copy
  -> materialize_aimer_widget_tree
  -> plan_element_reconciliation
  -> layout and draw
```

This is fundamentally different from native mode, where
`ImplicitAnimatedElement::draw()` ticks an `AnimationController` locally, rebuilds only its animated child, and requests
another native redraw.

## Code-confirmed issues and constraints

### 1. Every completed synchronous callback requests a rebuild

`crates/aimer_widget/src/portable/widget_ir.rs` currently calls
`context.queue_rebuild()` whenever a synchronous portable callback completes. That means a callback that only performs a
side effect still returns a full Widget IR image.

`HoverableGetStartedButton` is an example: its callback prints a message and calls `webbrowser::open()`, but it does not
mutate application state. In hot-reload mode, the dependency is rewritten to
`aimer_cli/portable_webbrowser`, whose `open()` implementation returns an unsupported error because a guest has no
direct native browser handle. The browser shim is not expected to account for the reported 0.8 seconds by itself. The
unnecessary full rebuild is a confirmed source of work, but its share of the observed latency must be measured rather
than assumed.

### 2. Portable implicit animations rebuild the complete guest tree per frame

`website/src/components/animation_button.rs` creates one
`ImplicitAnimatedBuilder` for each platform button. A selection change can change the targets of the old and new
selections, so normally at most two button subtrees have active transitions at once. The complete page is nevertheless
rebuilt and transferred for each requested frame.

`same_looking.rs` contains an `AnimatedSwitcher`, but its current portable lowering is transparent to the child. It does
not start a portable animation or account for portable per-frame work. It is a native-mode animation today and should
not be used as evidence for the current portable hot-reload cost.

Every active portable animation requests another guest build. The host then materializes and reconciles the complete
page, not just the changing visual properties. This explains why the animation cost grows with page complexity.

### 3. Safe-point work is serialized with rendering

`LiveReloadHost::process_safe_point()` runs guest polling, callback dispatch, and possible Widget IR installation before
the frame is drawn. This preserves tree coherence, but any expensive guest rebuild directly becomes frame time.

### 4. Native redraws still enter the Wasmi polling path

`FrameDrawer::draw()` calls `process_safe_point()` for every frame. The current safe point unconditionally calls
`poll_async()`, and when no command is pending it calls `request_async_frame()`, which invokes the guest's
`has_async_work()`. Therefore, adding a native animation controller alone does not satisfy the no-Wasmi-per-frame goal:
every native redraw would still execute guest polling and work detection.

The host needs an explicit guest-poll gate. A native-only redraw must be able to skip both `poll_async()` and
`has_async_work()`. Guest async work must arm the gate through an explicit host/guest request or response, rather than
using `has_async_work()` as an unconditional per-frame probe. The gate must remain enabled for guest callbacks, guest
async tasks, and portable-animation fallback frames.

### 5. Native and portable animation semantics diverge

The native `AnimatedBuilder` path already owns a native animation controller, ticks during draw, and requests a native
redraw. Portable `AnimatedBuilder`
also has an animation representation and native materialization path that should be reused where possible. The missing
equivalent is for portable
`ImplicitAnimatedBuilder`: its builder closure is evaluated in the guest, so the native host cannot interpolate the
guest-produced child without a richer representation of its target values and child/property payload.

The existing portable `AnimatedBuilder` materializer is not itself proof that this seam is solved: its current builder
closure retains a static child and does not consume the animation value in the native materialization path. Reuse it only
after its schema, value propagation, and retained-child behavior are corrected for the intended payload. Do not make
`ImplicitAnimatedBuilder` depend on an arbitrary guest closure executing on the native side.

### 6. AWIR preparation and commit share the UI safe point

`LiveReloadHost::process_safe_point()` currently performs module instantiation, state transfer, Widget IR preparation,
materialization, reconciliation, and generation commit on the application thread. The listener thread only transfers the
module and queues a pending command. Consequently, the old UI cannot render while the new candidate is being prepared.

The reload path should split preparation from publication. A background worker can prepare an isolated, owned candidate
while the active generation continues to render. The current preparer cannot be moved to a worker unchanged: it retains
UI-thread-bound values such as `Rc<LocalScheduler>`, `BuildContext`, and native `AnyElement` data. The worker must never
mutate the active widget tree or native UI objects. The final native materialization/reconciliation and generation swap
must still happen on the application thread at a safe point.

### 7. A prepared candidate can become stale while the active generation runs

Capturing one serialized state bundle is not sufficient: the active generation continues to receive input and async
events while preparation runs. A callback can mutate live state after the worker snapshot, so a candidate built from the
older state must not be committed merely because its source revision is current.

The current callback queue is separate from `ReloadCoordinator`'s event-routing barrier. The implementation must give
one UI-owned reload coordinator responsibility for callback events, async completions, candidate readiness, and commit
validation. It must record at least an active generation ID, a monotonic active-state revision, and the last event
sequence represented by the candidate. If any state-changing event, source edit, generation replacement, or queue-drop
occurs after capture, the candidate is stale. The initial implementation should discard it and reprepare from a fresh
UI-thread snapshot; it must not silently replay an incomplete event history or commit an ambiguous candidate.

Do not hold all callbacks until the user presses `R`: the active generation must remain interactive. At the final safe
point, commit only when the candidate's source revision, base generation, state revision, event sequence, and capability
context still match. Otherwise leave the active tree unchanged and report a stale-candidate diagnostic.

### 8. One source edit can trigger 4–5 WASM compilations

One editor save can produce several filesystem notifications: content writes, metadata updates, temporary-file
replacement, or rename events. The current watch path already drains events that arrive within a 75 ms receive window
and passes one path batch to the production loop. However, it does not inspect
`EventKind` or track a semantic source revision, and the production loop still starts one rebuild for each returned
batch. Changes that arrive while a build or upload is in progress remain queued and can trigger another build after the
first one finishes.

This can display four or five `Compiling` build numbers for one logical edit. The build counter is incremented before
compilation, so these numbers are compilation attempts, not necessarily committed native generations. The existing
`ChangeCoalescer` and `WatchStateMachine` abstractions are not yet connected to the production watcher. `WatchSet`
already filters ignored paths and removes overlapping automatic watch roots; the production integration should preserve
those invariants rather than duplicate them.

The watcher should treat one quiet batch of relevant source changes as one reload request:

- filter `notify::EventKind` to source create/modify/remove/rename events and ignore access, metadata-only, and known
  editor temporary-file events;
- normalize and deduplicate paths before classification;
- use a trailing quiet window, such as 100–200 ms, after the last relevant event;
- track `Idle`, `Building`, `Uploading`, and terminal states with a dirty flag; changes during an active request must
  not start a concurrent build;
- after the active request finishes, perform at most one follow-up build for the newest dirty batch;
- preserve non-overlapping watch roots and continue ignoring Cargo target outputs.

Expose separate diagnostics for raw filesystem events, coalesced batches, guest build attempts, uploads, and committed
generations. This makes it clear whether a repeated status is caused by the editor, the watcher, Cargo, or the host
commit path.

## Proposed module seam and invariants

The clean seam is between an owned background preparation module and a UI-owned commit module:

```text
ReloadPreparationInput (owned + Send)
  -> background preparation implementation
  -> PreparedCandidate (owned + Send)
  -> UI-owned ReloadCommitCoordinator
  -> native materialization/reconciliation at a safe point
```

The preparation module should hide Wasmi instantiation, state migration from the captured bundle, Widget IR generation,
limits validation, and candidate diagnostics behind a small interface. The commit coordinator should hide candidate
replacement, stale checks, event routing, native materialization, generation swap, and exactly-once frame requests.
This gives callers leverage without exposing `Rc`, `RefCell`, `BuildContext`, native elements, window handles, or live
callbacks across the seam, and keeps correctness fixes local to the coordinator.

Every `PreparedCandidate` must carry:

- a unique candidate ID and source revision;
- the base active generation ID;
- the base active-state revision and last represented event sequence;
- the migrated state report and owned guest/AWIR data required by the UI adapter;
- resource and capability validation results;
- enough generation metadata to reject duplicate, superseded, or cross-session commits.

The candidate interface must be `Send` and must not borrow the active snapshot. If the Wasmi guest instance is not
`Send`, the worker must retain it behind a worker-owned implementation and return a thread-safe prepared representation;
moving `ReloadCandidatePreparer` unchanged is not an acceptable shortcut. The UI adapter alone may create or mutate
native elements and may consume `BuildContext`.

Make this ownership decision a Phase 4 entry check with a compile-time proof. There are only two valid implementations:
either the complete prepared guest generation is `Send` and can cross to the UI adapter, or a worker-owned guest module
exposes all later guest calls through an explicit message interface and crosses only a stable handle plus owned results.
If neither is viable, the worker may return validated module/state/AWIR bytes, but the plan must measure and explicitly
accept the remaining UI-thread guest-instantiation cost; it must not claim that full Wasmi preparation is in the
background. Never hide a non-`Send` guest behind an unsafe wrapper.

## Optimization goals

1. Side-effect-only callbacks should not rebuild Widget IR.
2. A normal hot-reload callback should complete within one display frame when it does not change a large UI tree.
3. Active hot-reload animations should target 60 FPS on a representative page:
   p95 frame processing below 16.7 ms, excluding unavoidable platform GPU presentation variance.
4. Animation state must remain generation-local and safe across reloads.
5. Native input and rendering must remain single-thread coherent; no callback may mutate the live tree from the listener
   thread.
6. One logical source edit should produce one guest compilation, except for a single intentional follow-up when a newer
   edit arrives during compilation.

## Prioritized optimization path

### [ ] Phase 0: Select a valid benchmark profile and measure before changing behavior

Add feature- or runtime-gated timing counters or trace spans, disabled for normal runs, around:

- callback enqueue;
- callback safe-point start;
- `GuestInstance::dispatch_event()`;
- `GuestInstance::poll_async()`;
- `GuestInstance::has_async_work()`;
- Widget IR byte length and node count;
- `materialize_aimer_widget_tree()`;
- reconciliation commit;
- layout and draw completion.

Record separate distributions for:

- `Get Started` side-effect-only callback;
- a state-changing platform selection callback;
- one animation frame with no callback;
- one native-local animation frame with the guest-poll gate closed;
- initial module installation.

This distinguishes event-loop delay from Wasmi execution, serialization, materialization, reconciliation, and rendering
cost. Debug logging and
`widget_ir_diagnostics` must be disabled for the performance baseline.

The current `ExecutionPolicy` allows only `debug/wasmi/hot-reload`, `run_hot_reload()` hardcodes that policy, and
`aimer_quiver` rejects Wasmi hot reload when debug assertions are disabled. Therefore, “optimized build” cannot mean a
release/Wasmi build under the current implementation. Add and select a dedicated hot-reload-optimized development
profile (or an equivalent debug-assertions-preserving profile) with optimized code generation, update the CLI artifact
routing and policy validation, and record that profile in every baseline. Do not weaken the non-debug safety guard merely
to obtain a benchmark.

### [ ] Phase 1: Stop unnecessary callback rebuilds

Change portable callback completion semantics so a rebuild is requested only when one of these conditions is true:

- a retained/live state mutation was actually submitted;
- the callback explicitly requests a rebuild;
- an async callback completes with a state or output change.

Do not infer “rebuild required” solely from synchronous completion. Preserve the existing callback error, generation,
and cancellation behavior.

Add regression tests for:

- a synchronous side-effect-only callback returning no Widget IR;
- a synchronous callback that calls `StateUpdater::set_state()` returning one rebuilt image;
- callback failure still returning a diagnostic without publishing a tree;
- a callback that submits a live mutation and then fails, proving the failed callback cannot publish or leak a later
  rebuild;
- multiple state mutations coalescing into one rebuild.

This should make `HoverableGetStartedButton` cheap, although opening a browser still needs a native capability/event if
it is expected to work in hot-reload mode.

### [ ] Phase 2: Coalesce watcher events into one guest build

Connect the production watcher to the existing `ChangeCoalescer` and
`WatchStateMachine`, or provide an equivalent state machine at the same boundary. The watcher must emit one logical
change batch rather than exposing raw filesystem notifications directly to `rebuild_and_push()`.

Add regression tests for:

- five notifications for one file save producing one guest build;
- notifications within the quiet window being grouped, while notifications after the quiet window start a distinct
  batch;
- changes arriving during compilation producing one follow-up build without overlapping compiler processes;
- temporary, metadata-only, and ignored target events producing no build;
- a failed build still clearing the current batch and coalescing later edits.

Record both the raw event count and the resulting build count in diagnostics. The expected invariant is one build per
semantic source revision, not one build per operating-system notification.

### [ ] Phase 3: Native-local animation node

Extend the existing portable animation representation and native materializer for `ImplicitAnimatedBuilder`; do not
create a second generic animation protocol. The representation must carry the animation parameters and animatable values
needed by the native host, rather than only a guest-produced child image. The minimum representation should contain:

- a generation-local stable animation slot/key that is independent of transient child indexes;
- current value and target value;
- duration;
- curve;
- animation status;
- typed property tracks or endpoint values for the supported visual properties;
- the child/property payload required by the native materializer, including any explicit endpoint structure changes.

For the website button path, the guest must emit a representable payload for `f32` progress, width, border radius,
colors, checkmark spacing, and the selected/unselected child structure. The host must interpolate those typed values; it
must not attempt to execute the guest builder closure or infer structure from an arbitrary child image. Unsupported
payloads must remain on the complete portable rebuild fallback path.

The native materializer should create or update a native animation element, reusing the existing `AnimatedBuilder` path
where its schema and retained-child semantics apply. The element should:

- tick an `AnimationController` on the UI thread;
- interpolate supported visual properties locally;
- request only a native redraw while active;
- avoid calling Wasmi for intermediate frames.

Implement the guest-poll gate described above as part of this phase. A redraw caused only by this native animation node
must not call `poll_async()` or `has_async_work()` from `process_safe_point()`. Add an instrumentation test that counts
guest calls across several animation frames and proves they remain zero after the initial target update, while a real
guest async task still arms and services the poll path.

The first implementation should target the concrete hot path used by the website: `f32` progress driving width, border
radius, colors, and checkmark spacing. General arbitrary guest closures should remain on the existing portable fallback
path until they have a representable native form.

### [ ] Phase 4: Prepare AWIR in the background and gate commit with `R`

Split the reload pipeline into a background preparation phase and a short application-thread commit phase:

```text
file change
  -> guest compile and module upload with a reload/source revision
  -> background Wasmi/AWIR candidate preparation
  -> prepared-candidate notification
  -> user presses R in the `aimer-cli` terminal
  -> UI-thread validation, reconciliation, generation swap, and frame request
```

This flow requires a protocol split. The current `send_reload_command()` waits for a terminal result, and the listener's
`execute_once()` runs the reload sink synchronously. That design cannot report `Reload Ready` while leaving the candidate
uncommitted, and a long preparation can prevent the listener from accepting a later commit command. Replace it with an
asynchronous, authenticated command flow that acknowledges upload independently of application commit:

```text
upload(module, source_revision)
  -> UploadAccepted
  -> PrepareStarted
  -> CandidateReady(candidate_id, source_revision, base_generation, base_state_revision, event_sequence)
  -> CommitPrepared(candidate_id)
  -> CommitResult(candidate_id, generation | rejection)
```

The request/source revision and candidate ID must be distinct, monotonic identities. The commit command must identify the
exact candidate it intends to apply. The listener must continue accepting authenticated commands while preparation is
running, and the result ledger must make retries and duplicate commit commands idempotent without executing the sink
under the ledger lock.

The UI thread should first capture the minimum immutable input needed for the candidate, including the active generation
ID, a serialized state bundle, the active-state revision, and the last event sequence represented by that bundle. The
background phase should then perform only work that is independent of the live widget tree, including module validation,
Wasmi instantiation, state migration from the captured bundle, and owned Widget IR generation. A reconciliation plan may
be prepared in the background only after its inputs are proven owned and thread-safe; the current native
element/context-based preparer does not satisfy that seam. The UI phase should perform the final native
materialization/reconciliation: verify every candidate identity and revision, install the new root and callback table,
invalidate layout, and request a frame.

The active generation continues to dispatch callbacks and async completions during preparation. Each state-changing
operation must advance the active-state revision in the UI-owned coordinator. At commit time, a mismatch in source
revision, base generation, active-state revision, event sequence, capability context, or queue-drop status rejects the
candidate as stale and leaves the active tree unchanged. The initial implementation should then capture a fresh state
bundle and reprepare; it must not commit a candidate built from an untracked event history.

Manual commit mode is a terminal command owned by `aimer-cli`; it is not an app window key binding and does not enter
the guest callback path. In that mode, the proposed `R` command commits a prepared candidate; it does not start
compilation or preparation. It must be implemented as a distinct
`CommitPrepared` console action: the current console already maps `Shift+R` to hot restart, so the new command must be
routed explicitly or replaced with a named command rather than silently reusing `HotRestart`.

The current `run_hot_reload()` watch loop has no terminal input reader and calls build/upload synchronously. Add a
dedicated input reader and coordinator channel, and move compilation, preparation, and commit waiting behind the
background state machine so a typed command cannot block the watcher or the application event loop. Do not assume that
the existing `ConsoleAction` mapping applies to this path: in the existing application console, lowercase `r` remains
`HotReload` and `Shift+R` remains `HotRestart`; `CommitPrepared` must be a separate hot-reload CLI action. In a
non-interactive run, manual `R` input is unavailable and the status output must remain plain, without cursor-control
sequences.

It should work as follows:

- Keep rendering the current generation while preparation is in progress.
- Store at most one pending candidate and use latest-candidate-wins behavior.
- If a newer edit arrives, cancel preparation or discard its stale result.
- If `R` is pressed in the `aimer-cli` terminal before preparation completes, record a commit request and commit as soon
  as the matching candidate is ready.
- If no candidate is ready, keep the current UI unchanged and show a bounded “preparing reload” status rather than
  blocking the event loop.
- If a source edit or active state-changing event supersedes the requested candidate, clear the pending commit for that
  candidate and wait for the newer candidate to become valid.
- Keep generation identity, callback ownership, state-transfer checks, and resource cleanup on the UI-thread commit
  path.

When hot reload is disabled, preserve the existing application-console key mappings unchanged. `CommitPrepared` must
not be added as an alias for either `HotReload` or `HotRestart` in that mode.

### Terminal status and notification UX

The `aimer-cli` terminal should show one in-place spinner/status line while the background reload pipeline is active. It
should update for meaningful state transitions instead of printing one line per filesystem notification:

The existing `ReloadStatus::WaitingForCommit` is emitted while the current synchronous upload waits for a terminal host
result; it is not a ready-to-commit state. Split the status vocabulary so upload acknowledgement, background
preparation, candidate readiness, a latched commit request, and UI-thread commit are distinguishable. The production
watch loop must feed these transitions from the coordinator rather than calling a blocking `rebuild_and_push()` for each
watch batch.

```text
Watching              ·
Change detected       ·
Coalescing changes    ·
Locking reload        ·
Compiling WASM        ·
Uploading module      ·
Preparing AWIR        ·
Waiting for R         ·
Reload Ready          ✓  press R to apply
Reloading             ·
Reloaded generation 7 ✓
```

The ready bulletin should use green terminal styling when color is available:
`Reload Ready ✓  press R to apply`. Failure, rejection, and cancellation should use the existing diagnostic path with an
appropriate non-green status. The spinner and bulletin must be driven by the background state transitions; they must not
block compilation, AWIR preparation, or the UI event loop.

For interactive terminals, provide optional notification integration:

- emit one terminal bell only when a candidate becomes ready, never for every source event or spinner update;
- use a supported terminal notification escape sequence when capability detection or an explicit opt-in confirms it is
  safe;
- allow notifications to be disabled with the existing CLI convention, such as `NO_COLOR`/non-interactive detection or a
  dedicated reload-notify option;
- fall back to the visible `Reload Ready` bulletin when the terminal does not support notifications;
- never emit spinner control sequences or notification escapes in CI, piped output, or captured logs.

Terminal input and status output must share one synchronized CLI console so a typed `R` is not lost while the spinner
redraws. Pressing `R` should change the status to `Reloading`, then replace it with `Reloaded generation N` or a bounded
rejection diagnostic.

The candidate passed between threads must be an owned, `Send` value. It must not contain `Rc`, `RefCell`,
`BuildContext`, live native elements, window handles, or callbacks that capture the active tree. If the current Wasmi or
AWIR types cannot satisfy this boundary, split them into a thread-safe prepared representation and a UI-owned
materialization step.

Add tests for:

- the active generation continuing to render while preparation runs;
- terminal `R` committing exactly the matching source revision;
- terminal `R` before preparation completion committing once preparation finishes;
- an active state-changing callback during preparation invalidating the old candidate and causing a fresh preparation;
- a dropped callback or async event invalidating the candidate with an explicit diagnostic;
- upload acknowledgement returning before candidate preparation and commit;
- duplicate and superseded commit commands being idempotent and never replacing a newer candidate;
- the terminal spinner updating in place for background compile, upload, preparation, and commit states;
- a green `Reload Ready` bulletin appearing exactly once per ready candidate;
- one optional terminal notification occurring only when reload becomes ready;
- non-interactive output containing no cursor-control or notification escapes;
- a stale candidate never replacing a newer generation;
- a rejected candidate leaving the active tree unchanged;
- the UI-thread commit requesting at most one frame.

Measure the commit duration separately from background preparation. The commit path should stay within the frame budget;
if reconciliation itself exceeds 16.7 ms, add incremental or frame-budgeted reconciliation rather than moving native
tree mutation to the worker.

### [ ] Phase 5: Make `ImplicitAnimatedBuilder` portable lowering native-aware

Update `ImplicitAnimatedBuilder` so hot-reload lowering emits the native-aware animation node instead of recursively
rebuilding its child in the guest for each tick. The initial target update may still cross the WASM boundary once.

The native host must retain the old visual value when a target changes so retargeting remains continuous. A generation
replacement must discard the native animation element and seed the new generation from its initial value.

Add tests for:

- target changes starting one native animation;
- equal target values not restarting an animation;
- retargeting from the current interpolated value;
- animation completion stopping redraw requests;
- generation replacement not retaining stale callback or animation state.

### [ ] Phase 6: Reduce rebuild scope for non-animation state changes

For callbacks that genuinely change UI state, add a bounded Widget IR patch or subtree update path. A callback should
not need to serialize and materialize the complete page when a stable subtree is the only affected region.

The patch protocol must retain generation and callback validation, document limits, stable keys, and atomic publication.
If a patch cannot be validated or materialized, the active tree must remain unchanged.

### [ ] Phase 7: Runtime and allocation tuning

Only after the architectural work is measured, and only in the debug-assertions-preserving hot-reload-optimized profile:

- compare the hot-reload-optimized Wasmi profile against the current debug pipeline;
- reuse guest input/output buffers where safe;
- avoid duplicate Widget IR validation when the same buffer has already been validated at the ABI boundary;
- reduce temporary allocations in native materialization;
- evaluate an optimizing WASM runtime only if guest execution remains a measured bottleneck.

These changes cannot compensate for a full guest rebuild per 16.7 ms frame, so they are later-stage work.

## Acceptance criteria

- `Get Started` side-effect-only callback does not produce a Widget IR image.
- `Get Started` hot-reload callback latency is measured separately from the external browser-launch operation.
- `animation_button.rs` `ImplicitAnimatedBuilder` animation frames do not call Wasmi, `poll_async()`, or
  `has_async_work()` after the initial target update when the native animation node is active. `same_looking.rs`'s
  current portable `AnimatedSwitcher` lowering is transparent and is not counted as a portable animation node.
- Background AWIR preparation does not block active rendering, and manual `R` commits only a candidate whose source
  revision, base generation, active-state revision, event sequence, and capability context still match. A stale or
  dropped-event candidate is rejected without changing the active tree.
- Upload acknowledgement is returned before candidate preparation completes; candidate-ready and commit results are
  distinct, idempotent protocol states.
- One logical source edit produces one guest build, with at most one queued follow-up build when a newer edit arrives
  during the active build.
- The UI-thread candidate commit, including native reconciliation, has a measured p95 below 16.7 ms on the
  representative page.
- Representative animation frame processing has p95 below 16.7 ms in the debug-assertions-preserving hot-reload-
  optimized profile, with the test page, profile, and diagnostics configuration recorded.
- No new callback queue overflow, stale-generation acceptance, dropped-event ambiguity, or live-tree mutation from a
  non-application thread is introduced. If an existing bounded queue reaches capacity, the candidate is rejected with
  an explicit overflow diagnostic rather than being committed with unknown event history.
- Native AOT behavior remains unchanged.

## Non-goals

- Guaranteeing 60 FPS when the platform GPU, browser launch, asset decoding, or host machine is independently
  overloaded.
- Allowing arbitrary guest closures to execute directly against native widget objects.
- Removing generation validation, event ordering, resource limits, or the application safe point.
