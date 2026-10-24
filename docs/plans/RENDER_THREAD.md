# Native render thread

Native applications now default to `RenderMode::Threaded`. Widgets, layout,
input, and command recording stay on the UI thread. Renderer construction,
encoding, and destruction happen on `aimer-raster`; custom pipelines need not
implement `Send`. Browser and headless drivers remain inline.

Select the synchronous fallback before attaching the child:

```rust,ignore
AimerApp::new()
    .render_mode(RenderMode::Inline)
    .child(my_widget)
    .run();
```

Jaime accepts `--render-mode threaded` or `--render-mode inline`. The existing
`raster-thread` Cargo feature remains a compatibility alias. To enable timing:

```sh
cargo run --locked -p jaime --bin jaime --features aimer/frame-stats -- --render-mode threaded
```

## Admission and ownership

There are at most two outstanding frames, including completions awaiting UI
presentation. Admission is nonblocking and occurs before consuming frame
requests, animation work, or paint damage. When full, input still dispatches;
completion resumes the deferred frame with its original animation/scroll reason.
No frame or image operation is silently overwritten. Manual packet producers
can use `try_present_packet` to recover the original packet when admission fails,
and must poll completions to observe initialization and release capacity.

Native Metal attaches/configures its layer on the main thread. The worker
acquires and encodes the drawable, waits for the actual producer command buffer
to be scheduled, and returns a retained presentation token. The UI checks the
surface generation and viewport before presenting in a Core Animation
transaction. iOS also checks current view bounds and scale. Apple worker and
frame scopes have autorelease pools.

DX12 initialization and normal close are asynchronous so the UI keeps pumping
window messages. Emergency Windows shutdown pumps while waiting for GPU-owner
destruction. Vulkan and OpenGL construct their owners on the worker; OpenGL
keeps its context there throughout its lifetime. WGPU creates its surface on
the UI thread and its renderer on the worker; Apple WGPU worker presentation
does not enable transactional presentation.

A failed acquisition applies the packet's image uploads/removals in order and
invalidates retained targets before another queued partial-damage frame can
render. Completion dispatch runs first-frame callbacks and reveals hidden
Windows windows on the UI thread. Suspension joins the worker before native
surfaces become invalid, including Android's synchronous suspension boundary.

## Validation — 2026-10-11/12

Focused regressions were run red, then green, for worker ownership/admission,
resource recovery, runtime mode selection, native admission, close completion,
WGPU scene fallback, and preservation of deferred frame reasons. Coverage also
includes stop cancellation, initialization errors, zero-size targets, input
delivery with both slots occupied, and concurrent timing-counter drains.

| Check | Result |
|---|---|
| `cargo nextest run --locked -p aimer_quiver --features frame-stats` | 328 passed |
| Native Cupid suite with GPU access | 634 passed, 5 skipped |
| `cargo nextest run --locked -p aimer_quiver --no-default-features --features wgpu,raster-thread,frame-stats --no-fail-fast` with GPU access | 333 passed, 3 existing failures |
| `cargo test --locked -p aimer_quiver --features frame-stats --doc` | 7 passed |
| `cargo build --locked -p jaime --bin jaime --features aimer/frame-stats` | Passed, Debug |
| `cargo check --locked -p aimer_quiver --target x86_64-pc-windows-gnu` | Passed |
| `cargo check --locked -p aimer_quiver --target x86_64-unknown-linux-gnu` | Passed |
| `cargo check --locked -p aimer_quiver --target aarch64-apple-ios` | Passed |

The final combined native run, after the Metal teardown adjustment, used
`cargo nextest run --locked -p aimer_quiver -p aimer_cupid --features aimer_quiver/frame-stats,aimer_cupid/frame-stats --no-fail-fast`:
962 passed, 5 skipped. Nextest marked one passing existing Metal binding test
(`the_widest_supported_layout_still_leaves_the_tables_consistent`) as leaky.
This classification is retained in the log; no GPU memory-leak measurement was
performed. The final Debug build, Apple/Windows/Linux checks, and doctests passed.

All three WGPU failures reproduce in an unchanged `HEAD` snapshot:

- `actual_headless_virtualized_scroll_packet_renders_and_resizes_on_metal`
- `retained_context_menu_hover_updates_only_its_rows_layer`
- `retained_pill_menu_records_separator_and_hover_pixels_at_one_and_two_x`

The web check (`--no-default-features --features web --target wasm32-unknown-unknown`)
is blocked by an existing incorrect path to `animation_only_stub.rs` in
`../crates/aimer_widget/src/components/element.rs`. Android's native check
(`--no-default-features --features native --target aarch64-linux-android`) is
blocked by existing missing shader include paths in `aimer_cupid/src/backend.rs`.
These were left unchanged. No full workspace test suite or Windows/Linux/iOS/
Android runtime validation was performed; those native runtimes are unavailable
on this macOS host. GPU checks required execution outside the filesystem sandbox.

Native Jaime smoke checks cover first presentation, sidebar/button input, eight
Window → Zoom resize transitions with frames in flight, and normal close. The
animation continued across resizing. An idle stack sample showed the UI in its
native run loop and `aimer-raster` parked on its receiver. The final inline app
also exited normally after its Metal layer teardown adjustment.

## Timing semantics and capture limits

`ui` measures recording and submission; in inline mode it also includes
acquisition, encoding, and presentation. It excludes event dispatch and the
preparation before recording. Threaded Metal UI presentation is reported
separately as `present`. `queue` measures submission to worker start, `acquire`
measures drawable acquisition, and `encode` excludes acquisition. Metal's
threaded `cpu-frame` includes queue delay through producer scheduling, before
UI presentation. It is a pipeline interval, not UI occupancy or input-to-display
latency. Worker and GPU samples arrive asynchronously in independent windows.

The paired loading-animation capture uses native Metal, Debug, 2300×1600 physical
pixels, scale 2, and a monitor reporting 120 Hz. Each mode contributes 150 warm
30-frame reports (4500 recorded frames), with one node and three commands per
frame. Means derive from reports rounded to 0.01 ms. No stack sampling or test
suite runs overlap the selected windows. UI p95 values are per-window histogram
upper bounds, not a pooled percentile.

| Mean per recorded frame | Inline | Threaded |
|---|---:|---:|
| UI recording/submission timer | 4.49 ms | 0.81 ms |
| Separate threaded UI presentation | Included above | 0.09 ms |
| Command recording | 1.69 ms | 0.79 ms |
| Encoding | 1.16 ms | 0.74 ms |
| Queue | 0.00 ms | 3.17 ms |
| Acquisition | 1.50 ms | 3.14 ms |
| CPU pipeline interval | 4.38 ms | 7.92 ms |

The UI recording/submission timer decreased about 82% in this capture, with
threaded presentation still consuming a separate 0.09 ms on the UI thread.
Per-report UI p95 upper bounds ranged from 7–9 ms inline and 1.5–2 ms threaded.
Queueing increased the CPU pipeline interval. These measurements support lower
UI occupancy for this workload; they do not establish higher throughput or
lower input-to-display latency. Build/encode differences across these sequential
runs are observations, not an isolated optimization claim.

Eight threaded Window → Zoom transitions exercised 2300×1600 through 3600×2150
pixels, including intermediate native animation sizes. The worst 30-frame UI
mean was 5.51 ms and the largest UI p95 upper bound was 7.0 ms. This segment
also includes steady animation frames, so its mean would dilute resize work.
The final inline-only source adjustment detaches the Metal layer at teardown;
it does not change the steady rendering path measured in the threaded capture.

The Markdown automation did not yield a reliable paired capture; preliminary
Markdown timings are excluded from the performance conclusion. Earlier captures
that promoted deferred animation frames to full repaints were also excluded.
Intermittent lag, continuous edge dragging, exact input-to-display latency, and
other native GPU backends remain unmeasured.

Raw captures and validation logs are local artifacts under `/tmp`:
`aimer-render-thread-markers.json`, `aimer-jaime-*-loading-fixed.jsonl`,
`aimer-native-final962.log`, `aimer-quiver-final328.log`, `aimer-cupid-final.log`, `aimer-wgpu-final336.log`,
`aimer-wgpu-baseline.log`, and `aimer-wgpu-modal-baseline.log`.
