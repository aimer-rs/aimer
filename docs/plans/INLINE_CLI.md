# Inline CLI

## Decision

Make the inline console the eventual default interactive presentation for
Aimer's long-running commands. During the experimental rollout, keep the
existing full-screen ratatui console as the stable default, expose inline
rendering behind the nightly preview flag, and keep plain streaming output for
CI and scripts.

Planned modes:

```text
aimer run                    full-screen console (default)
aimer run --full-screen     existing full-screen console
aimer run --no-tui           plain machine-friendly output
aimer assemble               inline stages when attached to a terminal
aimer +nightly run -Z inline-render  experimental inline preview
```

The inline console is still interactive terminal UI; it simply does not take
over the alternate screen. It preserves normal scrollback, copy/paste, and
shell behaviour. The `inline-render` selector is an unstable preview and must
require the existing `+nightly` selector until the implementation is ready to
become the default.

## UX contract

The console is a collapsed execution transcript. Every meaningful operation is
represented by a stage with a compact summary and retained detail.

```text
◆ Aimer · macOS

✓ Compile                         2.8s · 42 lines
✓ Assemble application            1.4s · 18 lines
⠋ Launch                         0.2s · running
▸ ⠋ Application output           3 recent lines · running

↑↓ select · Shift+R hot-restart · Shift+Q quit · Enter expand
```

The spinner belongs immediately before the active stage label. The control
navbar advertises `r hot-reload` only for runs where hot reload is enabled.

## 120×30 preview

This is the intended 120-column by 30-row shape for the experimental preview
command. The unused horizontal space is intentionally blank, as it would be
in a real terminal.

```text
$ aimer +nightly run -Z inline-render

◆ Aimer · inline renderer preview
  target: macOS · profile: debug

✓ Resolve project                         0.1s
✓ Compile                                 2.8s · 42 lines
✓ Assemble application                    1.4s · 18 lines
⠋ Launch                                  0.2s · running

▸ Application output                     3 recent lines
  app › Window created
  app › Inspector server listening
  app › Ready

▸ Build details                           4 stages · 2 collapsed

▸ ⠋ Application                           0.4s · running
  ↑/↓ select · Shift+R hot-restart · Enter expand · Shift+Q quit

  Details stay in memory; collapsed output is not discarded.

  The active stage updates in place; completed stages and logs stay in scrollback.

```

The preview should show the inline renderer's core behaviour: stable stage
summaries become scrollback, while only the active stage and navbar lines are
updated in place. The live region is limited to those two lines; it never
clears the terminal or rewrites the transcript.

Stage rules:

- Compile, assemble, packaging, launch, and hot-reload stages are collapsed by
  default after successful completion.
- The active stage shows a spinner immediately to the left of its label,
  together with the current action and progress when one is available.
- Failed stages expand automatically and retain the structured error or panic
  report.
- Application output is visible by default while running and remains retained
  as ordinary scrollback; `Enter` is still available for retained detail that
  has not already been emitted.
- Expanding a stage reveals the original log lines, preserving ANSI styling and
  structured source locations where available.
- `Enter` toggles the selected stage and `↑`/`↓` select stages. Stage expansion
  must not take over an existing application hotkey.
- Existing hotkeys remain available in inline mode. The intentional change is
  that `r` means hot reload and `Shift+R` means hot restart.

The renderer must never discard collapsed detail. Collapse is a presentation
choice, not a logging or retention policy.

## Hotkey contract

Inline mode must preserve the current console controls unless this table calls
out an intentional change. Key handling should be centralized at the input
seam so the inline and full-screen adapters cannot drift apart.

| Shortcut              | Action                  | Contract                                                                                              |
|-----------------------|-------------------------|-------------------------------------------------------------------------------------------------------|
| `r`                   | Hot reload              | Preserve the running app/session when the target supports it. Do not silently fall back to a restart. |
| `Shift+R`             | Hot restart             | Stop the current child, rebuild as needed, and launch a fresh app/session.                            |
| `Shift+Q`             | Quit                    | Also accept the uppercase `Q` event emitted by terminals for this shortcut.                           |
| `1`                   | App logs                | Select the app-log view.                                                                              |
| `2`                   | Build logs              | Select the build-log view.                                                                            |
| `3`                   | Inspector               | Select the inspector view.                                                                            |
| `Tab`                 | Next pane               | Cycle app logs → build logs → inspector.                                                              |
| `F12`                 | Toggle inspector        | Enable/disable the inspector and focus its view.                                                      |
| `t`                   | Toggle full tree        | Available while the inspector is focused.                                                             |
| `e` / `E`             | Toggle source locations | Preserve the existing app-log `(file:line)` visibility control.                                       |
| `s` / `S`             | Selection mode          | Preserve Vim-style mouse selection mode.                                                              |
| `y` / `Y`             | Yank selection          | Copy the active selection when one exists.                                                            |
| `c`                   | Copy pane               | Copy the current pane when no modifier is present.                                                    |
| `Shift+C`             | Clear pane              | Clear the focused app/build log pane.                                                                 |
| `Ctrl+C` / `Cmd+C`    | Copy                    | Copy the selection, or the focused pane when no selection exists.                                     |
| `↑` / `↓`             | Scroll or move          | Scroll logs; move the inspector cursor in the inspector.                                              |
| `PageUp` / `PageDown` | Page scroll             | Preserve the current ten-line movement.                                                               |

`Enter` is reserved for expanding and collapsing the selected stage in inline
mode. It does not replace `e`, because `e` already toggles source locations.

Hot-reload capability is target-dependent. If it is unavailable, `r` should
report that fact and suggest `Shift+R`; it must not unexpectedly perform the
more destructive hot restart. When hot reload is disabled for the run, reload
events and `r` produce no hot-reload stage or notice at all.

## Existing logging remains the source of truth

Do not replace the current runner or logging pipeline. Reuse:

- `RunnerEvent::BuildLog` for build, compile, package, and tool output.
- `RunnerEvent::BuildReport` for structured compiler failures.
- `RunnerEvent::AppLog` for styled application output.
- `RunnerEvent::AppPanic` for recovered widget panic reports.
- `RunnerEvent::StatusChange` for lifecycle and progress state.
- `RunnerEvent::HotReload` for reload notifications.
- `AppState`, `LogHistory`, `StyledLog`, `ErrorReport`, and `PanicReport` for
  retained data and existing formatting behaviour.
- The existing `assemble::Reporter` seam, including `StdioReporter` and
  `run::helpers::ConsoleReporter`, for shared packaging steps.

The existing `tracing` subscriber remains diagnostic output and must stay
separate from the user-facing runner transcript. The inline renderer should
consume the runner event stream rather than merge arbitrary tracing output
into it.

## Rendering seam

Create a small internal presentation interface at the point where runner
events become terminal output. The runner, build readers, packaging code, and
hot-reload pipeline should not know whether output is inline, full-screen, or
plain.

The intended arrangement is:

```text
build/run/hot-reload producers
            ↓
       RunnerEvent
            ↓
 stage tracking + AppState
            ↓
 ┌──────────────────┬────────────────────┬────────────────┐
 │ InlineRenderer   │ FullscreenRenderer │ PlainRenderer  │
 │ default TTY      │ ratatui adapter    │ CI/pipes       │
 └──────────────────┴────────────────────┴────────────────┘
```

Use the existing `RunnerEvent` interface as far as possible. Add only the
small amount of semantic stage metadata needed to identify a stage reliably;
do not infer stage boundaries from human-readable log strings.

The stage model should contain, at minimum:

- a stable stage identifier;
- a stage kind or label (`Compile`, `Assemble`, `Launch`, `HotReload`, and
  future kinds);
- lifecycle state (`Running`, `Succeeded`, `Failed`, `Cancelled`);
- start/end timing and optional progress;
- retained detail entries;
- expanded/collapsed view state.

Prefer explicit `StageStarted` and `StageFinished` events at orchestration
points. Existing build and app log events can remain detail events associated
with the currently active stage. The `assemble::Reporter` implementation is a
natural place to emit lifecycle metadata around each `Step`.

Keep the shared interface independent of `ratatui::Frame` and avoid making
inline rendering depend on ratatui-specific `Line` or pane hit-test state.
Existing ratatui data can remain inside the full-screen adapter while the
shared stage/detail representation is made neutral as the implementation
requires.

## Inline terminal rules

- Do not enter the alternate screen.
- Use raw mode only while interactive input is active.
- Restore raw mode, cursor visibility, keyboard modes, and mouse modes with an
  RAII guard on every exit path, including errors and panics.
- Route all user-facing writes through the renderer so child output cannot
  corrupt the cursor position.
- Keep only the active-stage line and control navbar in a managed live region;
  replace those two lines in place rather than redrawing the terminal.
- Cursor movement is permitted only within that two-line managed region; never
  move the cursor into or rewrite existing scrollback.
- Completed stage summaries and expanded detail blocks must be ordinary,
  append-only scrollback text. Never clear or redraw the transcript.
- Handle terminal width changes before wrapping detail and error blocks.
- Detect non-TTY output and fall back to plain output without emitting ANSI
  cursor-control sequences.

Inline terminals cannot reliably edit arbitrary content that has already
entered scrollback. The first implementation should therefore update only the
current managed region. For an old completed stage, expansion should either
append a clearly labelled detail block at the current cursor position or open
the full-screen detail view. Never attempt to rewrite unknown scrollback.

## Implementation plan

### Phase 0: lock down current behaviour

- [ ] Record the current `aimer run`, `aimer assemble`, and `--no-tui` output
  contracts.
- [ ] Confirm which runner paths already emit build and packaging output as
  `RunnerEvent`s and which paths still inherit stdio.
- [ ] Keep the current full-screen console working throughout the migration.
- [ ] Add focused tests before changing stage behaviour, following the
  repository's red-green-refactor rule.

### Phase 1: introduce stage tracking

- [ ] Define the smallest neutral stage/detail model needed by all renderers.
- [ ] Add explicit stage lifecycle events at compile, assemble, package,
  launch, and hot-reload orchestration points.
- [ ] Associate existing `BuildLog`, `BuildReport`, `AppLog`, and `AppPanic`
  events with the active stage without changing their formatting semantics.
- [ ] Make stage transitions deterministic when a build is cancelled, fails to
  spawn, or exits unexpectedly.
- [ ] Unit-test successful, failed, cancelled, nested, and repeated stages.

### Phase 2: build the inline renderer

- [x] Add an `InlineRenderer` that writes to an injected `Write` target so its
  output can be tested without a real terminal.
- [x] Render stage summaries, spinners, durations, progress, line counts, and
  the application log tail.
- [x] Keep the spinner on the live stage row and keep capability-specific
  controls out of the navbar when unavailable.
- [x] Render existing styled logs and structured reports through the renderer
  without introducing a second logging implementation.
- [ ] Add deterministic width-aware wrapping for reports and long stage names.
- [ ] Add tests for empty stages, multiline details, ANSI content, narrow
  terminals, resize, and output/error completion.

### Phase 3: add collapse and expansion

- [x] Track selected stage and expanded/collapsed state separately from log
  retention state.
- [x] Implement keyboard navigation and expansion without mouse capture by
  default.
- [x] Preserve every existing hotkey and add the intentional `r`/`Shift+R`
  split: hot reload versus hot restart.
- [x] Keep `Enter` as the inline expand/collapse action without stealing `e`,
  which already toggles source locations.
- [x] Auto-expand failures and keep the failed stage selected.
- [x] Show application log output by default while retaining the full output.
- [ ] Define the old-stage expansion behaviour for content already in
  scrollback; prefer append-only detail output or the full-screen viewer.
- [ ] Test repeated toggles, stage completion while expanded, hot reload while
  another stage is selected, and cancellation during expansion.

### Phase 4: integrate the command modes

- [ ] Route interactive `aimer run` through the inline renderer by default.
- [x] Add the unstable `inline-render` feature to the existing `-Z` selector
  and route `aimer +nightly run -Z inline-render` through the preview renderer.
- [x] Route `r` through the target's hot-reload capability and `Shift+R`
  through the existing full-restart path.
- [ ] Preserve the existing ratatui implementation behind `--full-screen`.
- [ ] Keep `--no-tui` plain and free of cursor-control sequences.
- [ ] Adapt `aimer assemble` to use the same stage presentation when attached
  to a TTY while retaining inherited/plain output for pipes and CI.
- [ ] Ensure all shared packaging steps continue to work with both the inline
  reporter and the existing plain reporter.
- [ ] Verify terminal cleanup on success, failure, Ctrl-C, child cancellation,
  and panic paths.

### Phase 5: polish and advanced details

- [ ] Add optional expand-all/detail commands only after the core interaction
  is stable.
- [ ] Decide whether the inspector remains full-screen-only or gets a compact
  inline summary with an explicit full-screen detail view.
- [ ] Add a verbose option or environment switch for users who want successful
  build details expanded by default.
- [ ] Measure rendering and allocation behaviour with high-volume app logs;
  do not rebuild the entire retained history every frame.
- [ ] Remove only duplicated presentation code after both adapters have parity.

## Verification and acceptance criteria

The implementation is complete when:

- Interactive `aimer run` preserves normal terminal scrollback and exits with a
  clean prompt.
- `aimer +nightly run -Z inline-render` enables the preview without changing
  the stable default before the implementation is ready.
- Compile, assemble, launch, and reload stages are collapsed by default after
  success and expandable without losing detail.
- All current hotkeys continue to work, with `r` reserved for hot reload and
  `Shift+R` explicitly performing hot restart.
- Failures expand automatically with the existing structured reports intact.
- Existing app/build log styling and copy behaviour remain correct.
- The same runner events can drive inline, full-screen, and plain adapters.
- `--no-tui` remains safe for CI and pipes, with no ANSI cursor control.
- The terminal is restored after normal exit, errors, Ctrl-C, and child
  cancellation.
- Focused unit tests, `cargo test -p aimer_cli`, and the relevant workspace
  checks pass.

## Out of scope for the first implementation

- Replacing `tracing` or changing application log producers.
- Removing ratatui or deleting the existing full-screen console.
- Rewriting arbitrary terminal scrollback.
- Introducing a chat-style command language or prompt unless the current
  single-key controls prove insufficient.
- Adding a new third-party terminal UI dependency before the existing
  crossterm and ratatui dependencies have been evaluated.
