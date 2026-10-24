# WASM Hot Reload Security Review

**Review date:** 2026-08-19
**Scope:** the experimental native-development WASM reload path in `aimer_anteros`,
`aimer_reload_protocol`, `aimer_reload_server`, `aimer_quiver`, and `aimer_cli`.

This review is a maintained engineering threat model, not an assertion that the
Phase 10 milestone is complete. The subsystem remains feature-gated and must not
be enabled by default until the open blocking gates below have passing evidence.

## Assets and trust boundaries

- The native host process, its widget tree, native state, and platform
  capabilities are trusted and remain authoritative.
- Guest modules, guest memory, guest-produced Widget IR/state/callback data, and
  every unauthenticated network byte are untrusted.
- The CLI and app authenticate one development session with ephemeral random
  credentials. Discovery and readiness announcements are public hints, not
  authorization.
- A candidate generation is untrusted and dormant until state transfer,
  validation, materialization, reconciliation preparation, and the application
  thread's safe point all succeed.
- Release AOT artifacts are a separate trust domain in which no interpreter,
  listener, remote-transfer code, session secret, mutable module, or development
  network permission is allowed.

## Threats, controls, and maintained evidence

| Boundary | Threat | Required control | Evidence |
|---|---|---|---|
| Module transfer | unauthorized upload, transcript change, replay, sequence gap, oversized staging | mutual nonce transcript authentication, per-connection directional keys, authenticated encryption, strict sequence/length/digest checks, explicit limits | protocol vectors and mutation tests; three Phase 7 fuzz campaigns; `aimer_reload_protocol/fuzz` |
| Token injection and storage | secret in process arguments, logs, diagnostics, launch announcements, or persistent files | CSPRNG credentials, redacted types, private child environment or standard input, zeroization, owned Android file cleanup, bounded expiry | adapter and redaction tests in `aimer_cli`; `launch.rs`, `readiness.rs`, and route cleanup guards |
| Listener discovery | forged Bonjour/readiness data, public bind, route confusion | discovery carries only a session digest; stream authentication remains mandatory; every target except physical iOS requires loopback; advertised and bound ports must agree | route/readiness tests and Phase 0 transport proofs |
| Guest module | WASI, undeclared imports, shared memory, threads, unsupported proposals, start-function effects | strict import/export/signature validation, proposal allowlist, no WASI, no shared memory, isolated `wasmi` store | `aimer_anteros` runtime and guest ABI tests |
| Guest memory | negative, overflowing, stale, or overlapping ranges; output-size substitution | checked integer conversion and exact ranges, copy-before-call, fresh memory lookup after growth, disjoint migration buffers, bounded negotiation | guest-memory and persistent ABI tests; safe Rust host implementation |
| Portable documents | malformed lengths, invalid UTF-8/bool/float, noncanonical order, deep/cyclic graphs, oversized payloads | checked bounded codecs and model validation before factory or capability side effects | golden/truncation/adversarial tests; `aimer_anteros/fuzz` envelope and portable-document targets |
| Capabilities | undeclared authority, ABI drift, quota bypass, cross-generation handle use | deny-by-default manifest negotiation, contract fingerprint, generation ownership, per-kind limits, staged-effect classes, explicit retirement | capability registry/staging/generation tests and native/WASM parity fixtures |
| Candidate staging | failed candidate emits haptics/network/subscriptions or mutates the active tree | dormant reversible resources, irreversible-effect rejection, disconnected materialization, side-effect-free reconciliation planning | Phase 4–6 staging, failure-injection, and headless safe-point tests |
| Resource exhaustion | guest loop, memory/table/stack growth, event flood, module upload, registry growth | per-call fuel, explicit runtime/model/protocol/state/resource limits, bounded FIFO, one-build/one-commit invariants | limit and overflow tests plus repeated commit/rejection soak coverage |
| Rollback and retirement | old tasks or callbacks mutate a new generation; candidate leaks after rejection | coherent generation snapshot, generation-tagged handles/events, exactly-once idempotent retirement, late-completion rejection | Anteros generation/reload tests and Quiver conformance/soak tests |
| Release feature leakage | interpreter/listener/crypto/session code reaches native AOT release | optional Cargo features, release compile guard, native-AOT CLI policy, dependency/symbol/string/package/entitlement inspection | `scripts/hot_reload_release_audit.sh`; target matrix checks |

## Cryptography and dependency provenance

The protocol composes reviewed library primitives rather than local
cryptographic implementations:

| Dependency | Locked version | Use and provenance decision |
|---|---:|---|
| `ring` | 0.17.14 | system randomness, HMAC-SHA256, and ChaCha20-Poly1305; established Rust cryptography implementation |
| `sha2` | 0.10.9 / 0.11.0 | incremental module digest and canonical fingerprints; RustCrypto implementation |
| `subtle` | workspace-locked | constant-time comparisons where a library API does not already verify a tag |
| `zeroize` | 1.9.0 | credential and private launch-data erasure on drop |
| `wasmi` | 1.1.0 | native-only debug interpreter selected by the recorded Phase 2 proposal/limit proof |
| `std::net` | toolchain | bounded TCP listener and loopback routes; no custom networking dependency |

`cargo audit` on 2026-08-19 initially found `RUSTSEC-2026-0258` in
`h2 0.4.15`. The lockfile was updated to `h2 0.4.16`, after which the audit
returned success with no vulnerability. Eleven warnings remain: the
unmaintained GTK3 `atk`/`gdk`/`gtk` family, `paste`, `proc-macro-error`, and
`ttf-parser`, plus `RUSTSEC-2024-0429` for `glib`. They are transitive native UI
dependencies rather than reload protocol/runtime dependencies; they remain a
maintained migration watchlist and must be reassessed for the supported Linux
toolchain floor.

## Unsafe and FFI review scope

The protocol codecs, authenticated server, host guest-memory range handling,
CLI workflow, and Quiver reload bridge contain no `unsafe` blocks. The one
reload-related boundary is the WASM32 guest proxy's `aimer_capability_call`
import in `aimer_anteros/src/capability.rs`. Its call site documents that input
and output slices stay live for the synchronous call, the host validates every
range, and no pointer is retained after return. Platform UI/renderer FFI outside
the reload subsystem requires its own existing platform review and is not made
reachable by enabling a guest capability.

## Operational failure diagnostics

- Authentication, version, application, runtime, digest, sequence, staging,
  timeout, route, and transaction-stage failures are distinct and stable.
- Diagnostics may contain session identifiers and one-way service digests, but
  never tokens, derived keys, private launch input, module contents, or guest
  state payloads.
- Compile, validation, migration, and pre-commit failures leave the active app
  running and permit a later edit. Native provider/contract changes explicitly
  require a native restart rather than silently degrading the guest.
- Queue, module, document, state, fuel, memory, table, call-depth, and resource
  failures reject only the candidate or connection that crossed the limit.

## Open blocking gates

Phase 10 and the first usable milestone remain blocked until all of the
following are recorded:

1. Complete benchmark distributions on the small, stateful, and near-limit
   applications across a low-resource physical mobile device and representative
   desktops, followed by approved numeric soft and hard limits.
2. Full-state successful reload, rollback, reconnect, hostile unauthenticated
   connection, and listener-bind proof runs on every supported native target.
3. Release package and live-port inspection on every target family, including
   entitlements/permissions and mutable-module absence.
4. The production generated guest package and module-to-candidate pipeline,
   which are prerequisites for complete real-target scenarios.
5. Focused human security review of protocol composition and the guest
   capability FFI invariant; this document records engineering evidence but
   does not substitute for that independent review.
