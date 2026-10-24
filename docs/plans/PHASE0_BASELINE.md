# Phase 0 baseline — Aimer framework and Jaime

Date: 2026-09-01. This is the first measurement record for
[`DAMAGE_REGION_IMPL.md`](DAMAGE_REGION_IMPL.md). Jaime is treated as the
reproduction fixture; the counters are implemented in the framework.

## Fixture and launch

- Repository: `/Users/cottons/AimerFramework/aimer-widget-features`
- Target: arm64 macOS, Xcode 27.0 SDK, Metal backend.
- Window capture: approximately 1104 × 768 pixels at the default window scale.
- Debug executable:
  `jaime/builds/macos/build/Debug/jaime.app/Contents/MacOS/jaime`
- Release executable:
  `jaime/builds/macos/build/Release/jaime.app/Contents/MacOS/jaime`
- LaunchServices has multiple Jaime bundles with the same bundle identifier,
  so profiling launches the exact executable path rather than using `open`.
- The current Jaime showcase was verified visually. The sidebar can be paged
  from the initial Accessibility semantics entry through the lower catalogue,
  and the content pane changes with selection. This is the current showcase,
  not the stale Radio application from another checkout.

## Instrumentation added

The framework now samples, in debug builds or when `aimer_quiver/frame-stats`
is enabled:

- retained `ElementNode` rebuild-walk visits and dirty-path prunes;
- stateful/stateless rebuild checks;
- stateful/stateless `build` callbacks that actually run; and
- existing retained draw traversal, command, text-cache, image, and
  build/encode/present phase counters;
- framework-owned paint-isolation candidates, records, cache replays,
  invalidations, direct fallbacks, and retained-tile records/replays; and
- full-frame damage clears/pixels plus zero-valued placeholders for partial
  damage regions, coalescing, target reuse, and full-frame promotion.
- per-frame retained layout, routed hit-test, paint, and root-draw calls;
- accepted scroll events, delivered scroll steps, active smoothing steps,
  state-update requests, committed scroll-offset changes, and direct redraw
  requests; and
- frame-wake requests that were accepted or coalesced plus delivered display
  ticks.

The Debug handler emits a report every 30 completed frames; optimized builds
can opt in with the `frame-stats` feature. Counters are thread-local at the
widget boundary and reset/taken around each frame, so a report can distinguish
a full draw traversal from actual widget rebuilding.
The retained-element layout/paint and routed-event counters are boundary
counts, not exclusive timing measurements; they identify how much work entered
each phase but do not yet attribute CPU time to individual phases.

The Metal frame report now defines a CPU frame as the interval immediately
before the `MetalApi` retained build/rebuild walk through renderer command-buffer
submission. It includes layout, paint, and CPU command encoding; it excludes
drawable presentation and vsync waits. The GPU frame interval uses the earliest
valid `MTLCommandBuffer::GPUStartTime` and latest
`GPUEndTime` across that renderer call's submitted command buffers, excluding
the separate drawable-present command buffer. A completion handler records
one sample when all submissions finish. CPU/GPU p95 values are upper bounds
from 0.5 ms histogram bins; the report exposes sample counts and saturation.

## What the debug report shows

Representative steady-state 30-frame windows after the first frame:

| Counter | Observed range per frame |
| --- | ---: |
| Build phase | 0.30–0.39 ms |
| Encode phase | 2.00–4.80 ms |
| Present phase | 0.07–0.09 ms |
| Drawn retained nodes | 341–352 |
| Recorded commands | 1,653–1,707 |
| Rebuild-walk visits | 2–3 |
| Rebuild-walk prunes | 2–3 |
| Stateful rebuild checks | 32–33 |
| Stateful builds | 0 |
| Stateless builds | 0 |
| Retained-layer commands | 0 |

The new paint/damage fields are emitted in the same report. The damage
baseline reports one full-frame clear per rendered frame and zero partial
regions/target reuse because the persistent-target `DamageRenderer` is not
integrated yet. The framework now has the first shared `PaintIsolated` seam
and backend-independent `DamageSet` model, but this baseline predates their
performance acceptance.

The freshly assembled debug executable produced the expected initial report:
`paint-candidates/frame=1`, `paint-records/frame=0`,
`paint-replays/frame=0`, `paint-fallbacks/frame=1`,
`damage-full/frame=1`, `damage-full-clears/frame=1`,
`damage-full-pixels/frame=3,680,000`, and zero partial regions, merges,
promotions, target reuse, or partial clears. This is an initial idle/content
report, not the sidebar-scroll acceptance run.

During real sidebar paging, representative 30-frame windows included:

- `build=0.74 ms`, `encode=37.28 ms`, `present=0.59 ms`, with
  `rebuild-visits/frame=187.8`, `stateful-builds/frame=0.8`;
- `build=0.45 ms`, `encode=37.39 ms`, `present=0.24 ms`, with only
  `rebuild-visits/frame=2.0`, `stateful-builds/frame=0.0`; and
- several settling windows returned to roughly `build=0.3–0.4 ms`,
  `encode=2–4 ms`, `rebuild-visits/frame=2`, and zero build callbacks.

The precise number of rebuild visits changes while the sidebar offset is
being propagated, but the ordinary draw traversal remains approximately the
whole retained surface. The reports therefore do not support the claim that
the whole widget tree is rebuilt every frame. They do support the narrower
diagnosis that the current frame still records and encodes a large visual
command stream, with no retained-layer replay in this fixture.

## CPU samples

These are preliminary process samples, not acceptance results. The process
was sampled with macOS `top` once per second for 18 seconds; `%CPU` is the
process value reported by `top`, and both peak and settling behavior are
recorded because a short gesture burst is not a sustained average.

### Debug

- PID 29469, sampled at 01:54:23–01:54:40.
- Real sidebar paging used `Page_Down`/`Page_Up` after focusing the sidebar.
- Idle samples were 0.0%.
- Interaction samples reached 2.0%, 5.0%, 77.7%, 22.8%, and 1.3%; the
  observed peak was 77.7%.
- The process returned to 0.0% after the paging burst settled.

### Optimized release

- PID 32890, sampled at 02:02:44–02:03:00.
- Same real sidebar paging sequence, using the Release bundle.
- The observed interaction samples included 0.2%, 11.7%, 27.3%, 8.1%, and
  0.4%; the observed peak was 27.3%.
- The process returned to 0.0% after the paging burst settled.
- The release sample did not include frame counters because Jaime does not yet
  enable the `aimer_quiver/frame-stats` feature in release mode.

The release result is close enough to the requested `<20%` goal to justify
the framework work, but it is not a pass: this run contains a 27.3% peak and
does not yet have a controlled sustained-average or percentile measurement.

## Phase 0 conclusion

The baseline separates the likely costs:

```text
sidebar offset input
  -> a small/variable retained rebuild walk
  -> root draw still visits the visual tree
  -> roughly 1.6k–1.7k commands are re-recorded
  -> encode/raster work can dominate the burst
```

This makes `PaintIsolated` and `DamageRegion` the appropriate next framework
seams. The first framework counter layer is now present: it can show whether
the existing scroll retention seam records, replays, invalidates, or falls
back, while the full-frame damage baseline makes the absence of persistent
target reuse explicit. It also reports the input-to-frame work chain needed to
separate scroll delivery, retained rebuild, layout, paint, and redraw wakeups.
The next measurement pass should repeat the fixture with a Release build that
has `frame-stats` enabled for measurement only and collect reliable
glyph/resource and allocation measurements. The macOS Metal path now records
asynchronous GPU frame intervals. No optimization is declared successful from
this baseline.

## Still outstanding after the first baseline

The following list records the open items at the time of the initial
measurement. The later Aimer MCP sections below update several of these items.

- 60 Hz versus 120 Hz frame-budget runs and dropped-frame counts.
- Fixed display-scale and refresh-rate capture in the measurement record.
- Live Jaime values for the new scroll, layout, hit-test, paint, root-draw,
  state, redraw, and frame-wake counters during the controlled workload.
- Reliable text-preparation/glyph-miss, image/resource-miss, and allocation
  counters at their respective framework/renderer seams.
- Live Jaime values for the new paint-isolation counters.
- Persistent-target `DamageRenderer` integration; the current damage fields
  are still an instrumented full-frame baseline, and partial-clear,
  target-reuse, and live promotion values remain zero by design.
- Completion of shared `PaintIsolated` adoption for retained tiles and
  dynamic islands, plus live Jaime paint-isolation counter values.
- Controlled idle, offset-only, momentum, sibling-animation, selection,
  hover/focus, resize, scale, theme, image, Markdown, async-resource, and
  effects baselines.
- Sustained release CPU average plus peak and percentile reporting.

## Earlier Phase 0 restart — Aimer MCP session c61, run 2 (2026-09-30)

### Instrumentation

- The existing Debug frame counters and 30-frame accumulator were present, but
  the `AimerFrameHandler::end_frame` report call was commented out. The report
  is enabled again; it emits build, encode, present, rebuild, layout, paint,
  input, and redraw counters every 30 rendered frames.
- `cargo check -p aimer_quiver` passed after re-enabling the report. Existing
  unrelated warnings remain. Tests were not run in this instrumentation pass.

### Aimer MCP resource recording

- Run ID: `2`; session: `c61e9e8f-86a2-4795-b7b7-2c3d2ed78ef0`; recording:
  `c61e9e8f-86a2-4795-b7b7-2c3d2ed78ef0_2`.
- Target: macOS Debug Jaime, PID `74272`; resource monitor interval: 1000 ms;
  recording duration: 292.3 s across 281 samples.
- The workload relaunched Jaime on the Accessibility semantics page, paged the
  sidebar down to the lower catalogue, attempted an additional page at the
  bottom, and paged back to the initial entry. The target window dimensions,
  scale, and refresh rate were not fixed for this run, so treat it as a
  preliminary baseline rather than an acceptance run.
- Across the full recording, MCP-reported process CPU was mean 1.437%, p50
  0.010%, p95 1.225%, max 78.497%. MCP GPU utilization was mean 0.691%, p50 and
  p95 0%, max 54.821%; three samples reported GPU data unavailable. These are
  process resource samples, not per-frame CPU/GPU durations.
- After the first 10 seconds, process memory ranged from 356.3 to 362.3 MiB and
  ended at 359.3 MiB. The initial process-refresh samples are excluded from
  this steady-state range.

### Frame report samples

| Window | Build | Encode | Present | Drawn nodes/frame | Commands/frame | Rebuild visits/pruned | Scroll steps/frame |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Startup, first 30 frames | 4.98 ms | 20.12 ms | 0.76 ms | 64.7 | 767.1 | 310.6 / 0.5 | 0.0 |
| Paging activity | 0.69 ms | 0.72 ms | 0.15 ms | 53.6 | 725.4 | 3.1 / 2.9 | 2.1 |
| Returned-to-top settled frames | 0.29 ms | 0.21 ms | 0.04 ms | 36.0 | 687.0 | 3.0 / 3.0 | 0.0 |

The paging window reported about 1.9 smoothing steps and no stateful builds;
the settled window reported no scroll events or scroll steps and no widget
build callbacks. The frame report contains 30-frame averages, not frame-time
percentiles. The resource recording's CPU/GPU utilization p95 values must not
be substituted for the future per-frame timing gate.

### Remaining work after the earlier c61 run

- [ ] Capture a controlled idle window and repeat sidebar paging with fixed
  target dimensions, scale, and refresh rate; record explicit workload start
  and end samples.
- [ ] Pair each Aimer MCP resource window with its matching frame-report
  sequence. The stress-scroll recording ID and input markers are saved below,
  but the live `aimer_get_logs` endpoint currently returns connection refused.
- [ ] Verify valid per-frame Metal GPU timestamps appear in the live frame
  reports; MCP GPU utilization is not a GPU duration measurement.
- [ ] Capture and retain Debug and optimized Release p95 CPU/GPU frame-time
  windows using the new frame timing summaries.
- [ ] Complete the remaining controlled idle, momentum, sibling-animation,
  selection, hover/focus, resize, scale, theme, image, Markdown, async-resource,
  and effects baselines.

## Updated Aimer MCP attachment — run c036 (2026-09-30)

- The updated Aimer MCP server exposed session
  `c036b99d-e0e9-4f68-a3ad-558070b6ae59`. `aimer_status` confirmed project
  `jaime`, run ID `1`, PID `90302`, macOS target, and an active resource
  sampler. The recording ID is
  `c036b99d-e0e9-4f68-a3ad-558070b6ae59_1`.
- A bounded MCP resource capture ran for 30.002 s at 1 s intervals and returned
  28 samples. Process CPU utilization was mean **1.562%**, p50 **1.579%**, p95
  **2.308%**, max **2.339%**. GPU utilization was mean **1.404%**, p50
  **1.424%**, p95 **1.779%**, max **1.931%**. Memory stayed at
  **309,575,680 bytes** (~295.2 MiB) and thread count stayed at **28**.
- These are process resource utilization samples, not per-frame CPU/GPU time.
  The app window size, device scale, and refresh rate were not captured for
  this interval, so it is a live MCP monitor check rather than a controlled
  acceptance baseline.
- App logs from this run contain 30-frame windows with about **0.6–0.9 scroll
  events/frame** and about **2 scroll steps/frame**, followed by scroll-free
  windows. Those log windows are not timestamp-aligned to this bounded resource
  capture and should not be used as a paired workload result.

### Page_Down/Page_Up capture from the updated MCP recording

- Recording: `c036b99d-e0e9-4f68-a3ad-558070b6ae59_1`. The retained recording
  returned **29 samples** from **2026-09-30 15:25:44.533–15:26:13.759 UTC**
  (about 1.04 s between samples). The window overlapped paging the sidebar down
  to the lower catalogue and back to the top. The content pane stayed on
  `Custom TextField caret`; the window dimensions, device scale, and refresh
  rate were not fixed. The input events themselves were not given separate
  timestamp markers.
- Process CPU utilization was mean **2.41%**, p50 **1.63%**, p95 **9.26%**, and
  max **13.05%**. GPU utilization was mean **1.03%**, p50 **0.57%**, p95
  **4.82%**, and max **6.20%**. Memory ranged from **299.5 to 300.5 MiB**;
  thread count remained **28**. These are sampled process utilization values,
  not per-frame CPU/GPU durations.
- Frame-stat log windows from the same run showed active paging at about
  **0.6–0.9 scroll events/frame**, **2 scroll steps/frame**, and **2 smoothing
  steps/frame**. Root draw remained **1/frame**; reports showed about
  **46 drawn nodes** and **755–999 commands/frame**, with build **0.95–1.31 ms**,
  encode **2.0–2.9 ms**, and present **0.05–0.06 ms**. Settled windows returned
  to zero scroll events/steps and zero stateful builds, with about **46 nodes**,
  **733 commands/frame**, build **1.16–1.83 ms**, encode **0.8–1.14 ms**, and
  present **0.17–0.27 ms**. These 30-frame reports are separate windows and
  should not be treated as one-second resource-sample timing.
- On a follow-up MCP check, `aimer sessions` reported no live managed run;
  `aimer_status` and `aimer_get_logs` failed with connection/control-endpoint
  errors, and `aimer_get_resource_usage` returned connection refused. The
  retained resource-recording query still worked and its sample count advanced;
  the Jaime process was visible as PID `90302`. This records the available
  measurements without treating the recording as a fully controllable live
  attachment.
- This remains a preliminary workload sample: the target geometry and display
  settings were not fixed, action start/end times were not marked separately,
  and the Aimer MCP utilization samples do not provide frame-time percentiles.

### Timestamped stress-scroll measurement — run c036

- Workload: **30 cycles** alternating `Page_Down` and `Page_Up`, with a 350 ms
  pause after each key event. Jaime returned to the top of the sidebar after
  the workload. The measured input interval was **2026-09-30
  15:35:18.502–15:35:40.163 UTC** (**21.661 s**).
- Resource recording: `c036b99d-e0e9-4f68-a3ad-558070b6ae59_1`, run ID `1`,
  PID `90302`. The interval contained **21 samples** at a mean spacing of
  **1,048 ms**; the first and last in-range samples were at 15:35:19.126 and
  15:35:40.092 UTC.
- Process CPU utilization was mean **3.86%**, p50 **3.75%**, p95 **4.98%**, and
  max **5.37%**. GPU utilization was mean **1.70%**, p50 **1.66%**, p95
  **2.33%**, and max **2.33%**. Memory ranged from **312,606,720 to
  312,770,560 bytes** (about **298.1–298.3 MiB**), with **26 threads**.
- These are process utilization samples, not per-frame CPU/GPU durations. The
  live log request still returned connection refused, so no frame-stat window
  could be paired with this stress run. Target dimensions, device scale, and
  refresh rate also remain uncontrolled; treat it as a timestamped preliminary
  stress sample.

### Synchronized user-scroll capture — run c036

- The user scrolled Jaime during a bounded Aimer MCP recording configured with
  `duration=10 s` and `interval=0.5 s`. Capture ran from **2026-09-30
  15:43:18.641–15:43:28.643 UTC** and returned **19 samples** with a mean
  spacing of **526 ms**. The screenshot after capture showed the sidebar back
  at its top position.
- Recording: `c036b99d-e0e9-4f68-a3ad-558070b6ae59_1`, run ID `1`, PID `90302`.
  Process CPU utilization was mean **69.38%**, p50 **69.93%**, p95 **78.14%**,
  max **78.14%**. GPU utilization was mean **39.52%**, p50 **38.99%**, p95
  **43.42%**, max **43.42%**. Memory ranged from **305,217,536 to
  305,332,224 bytes** (about **291.1–291.2 MiB**), with **25 threads**.
- The recording tool restored the prior sampling interval successfully. These
  are process utilization measurements, not per-frame CPU/GPU times; the live
  log endpoint did not provide a matching frame-stat window. Display size,
  device scale, and refresh rate were not fixed for this capture.

### Host and target snapshot — 2026-09-30

- Host: arm64 MacBook Pro with Apple M4 and 24 GB memory; macOS **27.2**.
- `system_profiler SPDisplaysDataType` reported Apple M4 and Metal support but
  did not report a connected display mode or separate Metal driver version.
- Aimer MCP frame reports from the rebuilt app confirm a **2300 × 1600 px**
  physical target, **2.000** scale, and **120000 mHz** (120 Hz) monitor refresh.
  This confirms the values against Winit's frame target rather than only a
  screenshot.
- The reference process is the macOS Debug Jaime executable on Metal. Run 2
  remained on this target throughout the idle and user-scroll captures below.

### Discarded extra launch a31a — 2026-09-30

- This extra session (`a31a7b24-c2ae-44ac-9e47-571d20727ebd`, PID `15976`)
  came from a CLI launch made while trying to rebuild Jaime. The user clarified
  that the existing app must be rebuilt through Aimer MCP. The extra process
  was stopped, and its resource measurements are excluded from the baseline.

### Rebuilt original Aimer MCP session c036 — run 2

- Used `aimer_restart_project` on the existing session
  `c036b99d-e0e9-4f68-a3ad-558070b6ae59`; this rebuilt that managed app instead
  of starting another Jaime instance. Aimer MCP status confirmed run ID `2`,
  PID `67417`, project root
  `/Users/cottons/AimerFramework/aimer-compositor/jaime`, and `running` status.
- The run's resource recording is
  `c036b99d-e0e9-4f68-a3ad-558070b6ae59_2`. Bounded captures temporarily used
  **0.5 s** sampling and restored the run's **1000 ms** continuous monitor
  afterward. The logs report no unavailable CPU/GPU samples for these windows.
- The app's live frame report confirmed the **2300 × 1600 px** physical target,
  **2.000** scale, and **120 Hz** refresh. The host was the Apple M4 MacBook Pro
  described above; no separate Metal driver version was available.

#### Controlled idle window

- Capture bounds: **2026-09-30 16:46:57.139–16:47:12.141 UTC** (**15.002 s**);
  **28 samples**. First and last resource samples were at 16:46:57.176 and
  16:47:12.061 UTC.
- Process CPU utilization was mean **0.025%**, p50 **0.0093%**, p95/max
  **0.215%**. GPU utilization stayed at **0%**. Memory stayed at
  **379,060,224 bytes** (**361.5 MiB**) and thread count at **25**.
- This is process utilization, not per-frame CPU/GPU time. The small CPU peak
  occurred in two adjacent samples; the remainder stayed near 0.01%.

#### User-driven sidebar scroll window

- The user continuously scrolled the visible Jaime sidebar during the full
  capture. Bounds: **2026-09-30 16:45:26.700–16:45:41.702 UTC** (**15.002 s**);
  **28 samples** at a configured **0.5 s** interval. The first and last samples
  were at 16:45:27.220 and 16:45:41.452 UTC; all 28 reported active CPU or GPU
  use. The initial attempted capture is excluded because only its final six
  samples showed scroll activity.
- Process CPU utilization was mean **59.51%**, p50 **66.37%**, p95
  **71.33%**, max **72.17%**. GPU utilization was mean **42.53%**, p50
  **46.89%**, p95 **49.69%**, max **49.89%**. Memory ranged from
  **378,847,232 to 379,076,608 bytes** (**361.3–361.5 MiB**); thread count
  stayed at **26**.
- These are process utilization samples, not per-frame CPU/GPU times. The
  capture tool reported `interval_restored=true` after both windows.

#### Automated CUA sidebar scroll and frame-report correlation — run 2

- To remove manual timing variability, a native CUA loop sent **76** alternating
  one-page up/down scroll actions to the visible sidebar. Input markers were
  **2026-09-30 17:02:20.675–17:02:35.811 UTC** (**15.136 s**). The content page
  remained `Accessibility semantics` while the sidebar moved through its
  catalogue.
- Resource recording: `c036b99d-e0e9-4f68-a3ad-558070b6ae59_2`, run 2, PID
  `67417`. The monitor was set to **500 ms** before the workload and restored to
  **1000 ms** after it. The input window contained **26** timestamped samples,
  from **17:02:22.153–17:02:35.325 UTC**.
- Process CPU utilization was mean **50.39%**, p50 **49.92%**, p95 **55.95%**,
  max **57.83%**. GPU utilization was mean **14.89%**, p50 **14.89%**, p95
  **17.43%**, max **17.55%**. Memory ranged from **382,173,184 to
  382,451,712 bytes** (**364.5–364.7 MiB**); thread count stayed at **25**.
- The app-log cursor was **32197** before the workload and **33821** after it.
  Pagination succeeded when `aimer_get_logs` omitted the `run_id` filter; the
  cursor range contains **53** 30-frame reports (sequences **32199–33811**).
  Representative reports were:

  | Log sequence | CPU frame mean / p95 bound | GPU frame mean / p95 bound | Scroll events / steps per frame |
  | ---: | ---: | ---: | ---: |
  | 32416 | 4.69 / ≤11.50 ms | 1.80 / ≤4.50 ms | 0.1 / 2.1 |
  | 32664 | 4.71 / ≤9.50 ms | 1.99 / ≤4.00 ms | 0.1 / 2.1 |
  | 33811 | 5.00 / ≤7.50 ms | 2.39 / ≤4.50 ms | 0.1 / 2.1 |

- The frame reports confirm that the current scroll path still enters root draw
  once per frame. Reports include sequence numbers but no individual timestamps;
  the exact CUA input bounds, resource-sample bounds, and surrounding log cursor
  range are kept together here. CPU/GPU utilization remains distinct from
  per-frame CPU/GPU duration.

#### Frame timing and log pairing

- Earlier unpaired examples from run 2's first retrievable app-log page were:

  | Log sequence | Build / encode / present | CPU frame mean / p95 bound | GPU frame mean / p95 bound | Scroll events / steps per frame |
  | ---: | ---: | ---: | ---: | ---: |
  | 2594 | 4.01 / 4.48 / 0.64 ms | 9.32 / ≤8.00 ms | 4.42 / ≤7.50 ms (29 samples) | 0.5 / 1.5 |
  | 3028 | 0.45 / 0.59 / 0.06 ms | 3.27 / ≤8.50 ms | 3.09 / ≤8.00 ms (30 samples) | 0.6 / 2.0 |
  | 3059 | 1.27 / 2.81 / 0.07 ms | 4.84 / ≤10.00 ms | 4.12 / ≤7.00 ms (30 samples) | 0.6 / 1.9 |

- These earlier examples are not paired to the 16:45 or 16:46 captures. For the
  later automated scroll window above, omitting `run_id` made cursor pagination
  work and yielded the sequence bracket and 53 reports.
- The CPU/GPU p95 figures are upper bounds from 0.5 ms histogram buckets, not
  exact percentiles. A p95 bound below the mean can occur when a small slow-frame
  tail raises the mean; it is not by itself evidence of an instrumentation
  error.

#### Final idle capture after MCP restart — run 3

- `aimer_restart_project` restarted the same MCP session c036 as run 3 (PID
  `66814`); no separate Jaime session was launched. The resource sampler was
  running at its default **1000 ms** after the restart.
- Idle capture bounds: **2026-09-30 17:15:34.519–17:15:49.522 UTC** (**15.003 s**);
  recording `c036b99d-e0e9-4f68-a3ad-558070b6ae59_3`; **28 samples** at **0.5 s**.
  First and last samples were at **17:15:34.550** and **17:15:49.459 UTC**. The
  recorder restored the 1000 ms monitor interval.
- Process CPU was mean **0.0098%**, p50 **0.0105%**, p95/max **0.0113%**; GPU
  stayed at **0%**. Memory stayed at **366,067,712 bytes** (**349.1 MiB**), with
  **23 threads**. These are process utilization samples, not per-frame times.
- The app-log sequence remained at **34500** across the capture and added no
  frame reports, consistent with no rendered frames during idle.

#### Phase 0 closure

- [x] Confirm physical target size, scale, and 120 Hz monitor refresh in the
  live frame reports; preserve Debug/macOS/Metal and host details.
- [x] Capture idle and automated sidebar-scroll resource
  windows through the same Aimer MCP session; retain timestamps and sample IDs.
- [x] Confirm live CPU and GPU frame-time instrumentation emits samples and
  p95 histogram bounds in 30-frame reports.
- [x] Record scroll input start/end markers and the corresponding 30-frame
  sequence bracket. For MCP log pagination, omit `run_id` and filter the
  returned entries locally.

Phase 0 baseline and instrumentation are closed for the Debug/macOS/Metal
reference fixture. The measurements establish the initial idle and scroll
baseline; they do not claim that dirty-region redraw is implemented or that the
performance acceptance gate has passed.

#### Follow-up measurements outside Phase 0 closure

- [ ] Capture optimized Release CPU/GPU frame-time windows with
  `frame-stats` enabled and compare against Debug. The attached MCP restart
  tool exposes no build-profile selector; the managed session remained Debug.
- [ ] Capture a 60 Hz budget/dropped-frame run and compare it with this 120 Hz
  fixture.
- [ ] Extend coverage to momentum, sibling animation, selection, hover/focus,
  resize, scale, theme, images, Markdown, async resources, effects,
  text-preparation/resource misses, allocations, and dropped frames.
- CUA temporarily returned `cgWindowNotFound` after the run 3/4 MCP restarts.
  It attached successfully to run 5 on retry, and the automated scroll capture
  with Phase 1 invalidation counters is recorded in
  [`REAL_DIRTY_REGION.md`](REAL_DIRTY_REGION.md). No separate app process was
  launched.
