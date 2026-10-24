# Hot-reload target proof runs

Repeatable acceptance procedure for the all-native hot-reload targets of phase 9 in `HOT_RELOAD_IMPL.md`.

Every adapter is unit-tested without devices in `aimer_cli/src/commands/run/hot_reload/route/`. This document owns the
part that unit tests cannot own: the real-target evidence. A target is supported only when the scenario below has a
recorded passing run against a named toolchain and device.

## 1. Scope and rules

- One run covers exactly one target family and one device.
- Record the exact toolchain, operating-system, device model, and commit for every run. A run without recorded metadata
  is not evidence.
- Never weaken a step to make a run pass. Increasing a limit, disabling authentication, binding a public interface, or
  accepting state loss invalidates the run.
- The session token must never appear in a terminal transcript, a device log, a process listing, or a project file. A
  run that leaks it fails, even if the reload works.

## 2. Prerequisites

| Target        | Required tooling                                                            |
|---------------|-----------------------------------------------------------------------------|
| macOS         | Xcode command line tools, `aarch64-apple-darwin`                            |
| Windows       | MSVC toolchain, `x86_64-pc-windows-msvc`                                    |
| Linux         | GCC/Clang toolchain, `x86_64-unknown-linux-gnu`                             |
| iOS Simulator | Xcode with a booted Simulator, `aarch64-apple-ios-sim`                      |
| Android       | Android SDK platform tools with `adb`, `ANDROID_NDK_HOME`, `aarch64-linux-android` |
| iOS device    | Xcode `devicectl`, a paired unlocked device, `dns-sd`, `aarch64-apple-ios`   |

Build configuration coverage is checked separately and needs no device:

```sh
AIMER_REQUIRE_ALL_TARGETS=1 scripts/hot_reload_target_checks.sh
```

The development host compiles the reload protocol, whose cryptography crate builds C and assembly sources, so that
configuration needs a C compiler for the selected target. Xcode covers the Apple targets, `ANDROID_NDK_HOME` covers
Android, and the Windows development host must be checked on a Windows host with its MSVC toolchain. The script
discovers the Android NDK itself and names exactly what is missing instead of failing opaquely.

Last local run on macOS (`aarch64-apple-darwin` host, 2026-08-19): 16 of the 20 allowed configurations compiled,
including the development host for macOS, both iOS families, and Android. The remaining four were skipped with reasons:
three Linux configurations because that rustup target was not installed, and the Windows development host because it
needs a Windows host.

## 3. Route behavior each target must show

The adapters construct exactly these routes. A proof run confirms the real tooling accepts them.

| Target                | Launch                                                                             | Secret channel                        | Route                                  |
|-----------------------|------------------------------------------------------------------------------------|---------------------------------------|----------------------------------------|
| macOS/Windows/Linux   | the built host binary                                                              | private child environment             | direct loopback to the announced port  |
| iOS Simulator         | `xcrun simctl launch --console-pty --terminate-running-process <udid> <bundle>`     | `SIMCTL_CHILD_*`                      | host loopback to the announced port    |
| Android               | `adb -s <id> shell run-as <package> sh -c 'mkdir -p files && cat > files/...'` then `adb -s <id> shell am start -n <package>/com.aimer.AimerActivity` | standard input of `run-as`            | owned `adb forward --no-rebind`        |
| iOS device            | `xcrun devicectl device process launch --device <id> --terminate-existing --console <bundle>` | `DEVICECTL_CHILD_*`                   | encrypted Bonjour `_aimer-reload._tcp` |

Checks that apply to every run:

1. the launch console prints one `AIMER_RELOAD_LISTENER_READY` line with the session, port, process, and protocol;
2. the CLI connects only to the announced loopback port, or to the resolved advertisement on a physical device;
3. the token appears in no transcript, `logcat`, `os_log`, or process listing;
4. shutdown removes exactly this session's route and provisioned files and nothing else.

## 4. The full-state acceptance scenario

Run the steps in order without restarting the app. Every step has an observable result.

| Step | Action                                                                      | Required result                                                                     |
|------|-----------------------------------------------------------------------------|-------------------------------------------------------------------------------------|
| 1    | `aimer +nightly run -Z hot-reload` on the selected device                    | app installs, listener announces readiness, initial module commits generation 1      |
| 2    | interact until the app holds guest state and native runtime state            | state is visible in the running UI                                                   |
| 3    | edit a widget body and save                                                  | one guest build, one commit, all state preserved                                      |
| 4    | change a state schema and add its migration                                  | required state migrates, reset-safe losses are named in the terminal result           |
| 5    | rebind a callback and save                                                   | the new callback runs; no event reaches the retired generation                        |
| 6    | introduce a compile error and save                                           | the app keeps running the last committed generation and accepts the next edit         |
| 7    | make the guest trap during its initial build                                 | the reload is rejected with its stage, and the previous generation stays active       |
| 8    | interrupt the transport (unplug the cable, drop Wi-Fi, or kill the client)    | reconnect recovers the outstanding terminal result without restarting the app         |
| 9    | change a native provider or capability contract                              | the CLI reports the named native-restart reason instead of pushing a module           |
| 10   | stop the CLI                                                                 | the route, provisioned session file, and listener are gone; the app exits cleanly     |

The equivalent host-side scenarios run without a device in `aimer_quiver/tests/reload_conformance.rs`, so a real run
proves the transport and tooling rather than the reload logic.

## 5. Recorded evidence

| Target                       | Transport proof | Full acceptance scenario | Recorded                       |
|------------------------------|-----------------|--------------------------|--------------------------------|
| iOS device (iPhone15,2, 27.0) | pass            | not recorded             | 2026-08-18 transport proof only |
| Android emulator (arm64)      | pass            | not recorded             | 2026-08-18 transport proof only |
| iOS Simulator                 | not recorded    | not recorded             | —                              |
| macOS 27.0 (arm64)            | pass            | pass                     | 2026-08-20; see section 8       |
| Windows                       | not recorded    | not recorded             | —                              |
| Linux                         | not recorded    | not recorded             | —                              |

Adapter behavior itself is covered without devices by `aimer_cli/src/commands/run/hot_reload/route/` unit tests, and the
host-side reload scenarios by `aimer_quiver/tests/reload_conformance.rs`.

The 2026-08-18 transport proofs are described in `HOT_RELOAD_IMPL.md` section 18 and live in
`aimer_anteros/device_tests/ios_transport`. Full-state runs use the standalone application in
`aimer_anteros/device_tests/full_state_app`. The phase 9 platform gate stays open until every row has a passing full
acceptance scenario.

## 6. macOS partial run — 2026-08-19

- Host: macOS 27.0 arm64; Xcode 27.0 (`27A5228h`); `rustc 1.99.0-nightly (8ab9fdff5 2026-07-30)`.
- Source: base revision `83a6a542a5f2d29a1071a408feecb831a0cc9f6f` plus the uncommitted milestone-IV/V working tree. This is development
  evidence, not release evidence for that base revision.
- Command: `aimer +nightly run -Z hot-reload` from `aimer_anteros/device_tests/full_state_app`.
- Initial launch emitted one canonical readiness line, authenticated over loopback, uploaded `5,603,780` bytes, and
  committed generation 1 without exposing the token in CLI status.
- `widget-body` and `schema-migration` each produced one build and one commit without restarting process `82243`; the
  required counter state remained at one, observed through the unchanged `0x70` proof-surface shade.
- `callback-rebind` committed with the same callback identifier, but macOS `System Events` synthetic clicks did not
  reach Aimer's custom event tree. The changed callback action is therefore **not proven** by this run.
- `compile-failure` failed with the fixture's intentional Rust diagnostic and reported `active app retained`.
  `initial-build-trap` then compiled, was rejected at the build stage for WebAssembly `unreachable`, and retained the
  active generation. `recovery` subsequently committed without an app restart and retained the counter shade.
- A `native/contract.marker` replacement produced `native app restart required: native host source changed` without a
  guest push. Ctrl-C terminated the CLI and its owned application process.
- The server intentionally closes each completed request connection; every later accepted update rediscovered and
  authenticated a new connection. An independently induced interruption with outstanding-result recovery was not
  performed, so step 8 remains **not proven**.

This run establishes real macOS launch, readiness, authenticated initial push, source watching, state migration,
compile/runtime rollback, recovery, reconnect between completed requests, native-change classification, and cleanup.
It does not close the macOS row because steps 5 and 8 remain incomplete. Windows, Linux, iOS Simulator, iOS device,
and Android require the external native targets listed in section 2 and remain handoff work.

## 7. macOS callback and interrupted-result supplement — 2026-08-20

- Host: macOS 27.0 arm64 (`26A5416b`); Xcode 27.0 (`27A5228h`); `rustc 1.99.0-nightly (8ab9fdff5 2026-07-30)`.
- Source: base revision `83a6a542a5f2d29a1071a408feecb831a0cc9f6f` plus the uncommitted milestone-IV/V working tree. The temporary
  fault injection described below was removed immediately after the run.
- Command from `aimer_anteros/device_tests/full_state_app`:
  ```sh
  AIMER_PROOF_INTERRUPT_AFTER_ACK_REQUEST=5 cargo +nightly run --manifest-path ../../../Cargo.toml -p aimer_cli -- +nightly run -Z hot-reload --target macos
  ```
  The run used a debug-only one-shot branch at the post-upload-acknowledgement boundary of `send_reload_command`.
- The user clicked the initial proof surface once and observed `counter: 1`. `widget-body`, `schema-migration`, and
  `callback-rebind` then committed as generations 2, 3, and 4 without restarting native process `3127`.
- One real pointer click on generation 4 changed the counter from `1` to `11`. The callback identifier was unchanged,
  so this proves the event reached the rebound `+10` callback rather than the retired `+1` generation.
- Request 5 uploaded the `recovery` module once and received a valid upload acknowledgement. The proof branch then
  returned a connection-reset error before reading the terminal frame, dropping that authenticated TCP stream.
- The production client opened a fresh authenticated connection, queried request 5, reported `committed generation 5`,
  and did not re-upload the module. The same native window changed to `FULL STATE / RECOVERED` with its state retained.
- Ctrl-C stopped the watcher and native app. The fixture was restored to `initial`, and the one-shot protocol branch was
  removed; no fault-injection behavior remains in production source.

This supplement proves the two missing behaviors from section 6 on a real macOS app: pointer-driven callback rebinding
and outstanding-result recovery after an independently forced post-ack transport interruption. At that point, the macOS
table row remained `partial` because this supplement skipped the compile-failure, trap-rollback, and native-marker
actions rather than repeating all ten steps in one uninterrupted run. The cross-target phase gate also remained open.

## 8. macOS full acceptance run — 2026-08-20

- Host: macOS 27.0 arm64 (`26A5416b`); Xcode 27.0 (`27A5228h`); `rustc 1.99.0-nightly (8ab9fdff5 2026-07-30)`.
- Source: base revision `83a6a542a5f2d29a1071a408feecb831a0cc9f6f` plus the uncommitted milestone-IV/V working tree. The temporary
  fault injection described below was removed immediately after the run.
- Command from `aimer_anteros/device_tests/full_state_app`:
  ```sh
  AIMER_PROOF_INTERRUPT_AFTER_ACK_REQUEST=6 cargo +nightly run --manifest-path ../../../Cargo.toml -p aimer_cli -- +nightly run -Z hot-reload --target macos
  ```
  The run used a debug-only one-shot branch immediately after request 6's validated upload acknowledgement and before
  its terminal frame. This branch was present only for the proof.
- The initial module committed as generation 1 in native process `7481`. The user clicked the real proof surface three
  times and observed `counter: 3`.
- `widget-body` committed generation 2 with `FULL STATE / BODY CHANGED` and counter `3`; one click advanced it to `4`.
  `schema-migration` then committed generation 3 with `FULL STATE / SCHEMA V2 MIGRATED` and retained counter `4`.
- `callback-rebind` committed generation 4 without changing the callback identifier. One real pointer click changed
  the counter from `4` to `14`, proving the new `+10` callback ran and no event reached the retired `+1` callback.
- `compile-failure` emitted the intentional Rust diagnostic and reported `active app retained`; the unchanged generation
  4 UI remained interactive and advanced from `14` to `24` through its rebound callback.
- `initial-build-trap` compiled and uploaded as request 5, then was rejected at candidate build on WebAssembly
  `unreachable`. Generation 4 remained active and another real click advanced its counter from `24` to `34`.
- `recovery` compiled once and uploaded as request 6. After the host acknowledged the complete upload, the proof branch
  reset that authenticated stream before the terminal frame. The production client opened a fresh authenticated
  connection, queried the outstanding request, and reported `committed generation 5` without re-uploading the module.
  The same window displayed `FULL STATE / RECOVERED`, retained counter `34`, and advanced it to `44` on one click.
- Replacing only `native/contract.marker` reported `native app restart required: native host source changed`; there was
  no additional guest build, request, or upload. The native process remained `7481` for every preceding action.
- Ctrl-C terminated the CLI, watcher, and owned native app. No session process or independent proof process remained.
  The fixture was restored to `initial` and `native-contract-v1`, and the one-shot protocol branch was removed.
- Post-run verification passed: all 15 `aimer_reload_protocol` tests, both full-state fixture contract tests, and the
  fixture's six compiling WASM variant checks. No proof selector remains in Rust source.

This single CLI/app session completes all ten ordered macOS acceptance actions, including the intentional in-session
transport interruption. The macOS full-acceptance row is therefore `pass`; the cross-target phase gate remains open
until Windows, Linux, iOS Simulator/device, and Android also record complete runs.
