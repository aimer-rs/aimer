# Aimer MCP Survey: Answers and Recommendation

**Survey date:** 2026-10-03

## Answers

### 1. What can the current MCP do?

It exposes Rust Analyzer workspace operations: start/select a Rust workspace, search symbols and text, inspect
definitions, references, implementations, call hierarchy, types, documentation, diagnostics, completions, code actions,
and formatting. Rename, code-action, and format changes use a preview followed by an explicit apply. The wider Aimer
controls also expose project status, start/restart/stop, logs, and resource monitoring/recordings. The survey notes that
the live `aimer_rust_start_lsp` registration has an older schema than the repository source, so the deployed interface
and source documentation should be kept in sync.

### 2. Can it work with an ordinary Rust project?

The LSP start tool accepts an absolute or project-relative workspace path and starts or reuses rust-analyzer for that
project. This should work for an ordinary Cargo project that rust-analyzer can load. This survey exercised the Aimer
workspace only, so an unrelated plain Rust project has not been independently verified here. App controls and runtime
telemetry apply to the attached Aimer project, not to arbitrary Rust projects.

### 3. Does the MCP wait for rust-analyzer indexing?

Yes. The tool description says LSP calls wait for rust-analyzer to finish loading and indexing and report readiness
progress or an explicit timeout/health error. In this survey, starting the workspace returned `status: ready` and
`workspace_ready: true` before the search calls completed.

### 4. Are semantic results reliable across runs?

Not consistently enough to assume so. The prior Bamboo observations recorded in `SURVEY.md` report different semantic
results across two runs. In the current session, two identical `Widget` symbol searches returned the same five results
and both reported truncation; this is a useful sanity check, not evidence of cross-run reliability. Recheck important
results against source or another tool, especially after workspace changes.

### 5. Is `aimer_rust_check` a dependable build check?

No, not as the only build gate. The prior survey evidence says Cargo flycheck was blocked or timed out. Treat it as a
convenient diagnostic when it completes, and run the project’s normal Cargo check/build in a suitable environment when a
dependable build result is needed. A flycheck timeout is inconclusive, not a successful build.

### 6. Are write operations safe for Agent use?

The Rust LSP edits have a good review boundary: rename, code action, and formatting return previews, and applying
requires a preview ID while the source remains unchanged. This lowers accidental-edit risk and makes the patch
reviewable. It does not replace reviewing the preview for scope and correctness. This MCP editing path is for Rust
source; it does not write this Markdown answer.

### 7. Is the output Agent-friendly?

Mostly. Results are bounded and structured, include file/line locations for navigation, and use readable fields. For
example, symbol search returned names, kinds, locations, match quality, and a truncation flag; diagnostics returned an
empty diagnostics array. The current wrapper presents that JSON as text rather than native structured MCP content, which
makes parsing less direct.

### 8. What makes errors harder to interpret?

The JSON-as-text wrapper adds a presentation layer around the actual result. Indexing progress, health/readiness
failures, timeouts, and Cargo flycheck failures also describe different layers of failure, but can be easy to conflate
if the caller only sees a short tool error. Responses should clearly identify the operation and phase, distinguish
timeout from a negative diagnostic/build result, and preserve the underlying rust-analyzer or Cargo message.

### 9. What do we know about token usage?

There is no token counter or measured token-cost comparison in this survey, so a numeric saving cannot be claimed.
Output is bounded, which helps, but JSON carried as text and verbose LSP fields add overhead. The current symbol query
also returned five fuzzy matches and `truncated: true`, so callers may need follow-up queries. Prefer small limits,
exact names or path filters, and compact structured fields when possible.

### 10. Does the MCP stop rust-analyzer when the Agent exits?

This survey did not observe an Agent exit and cannot establish the cleanup behavior. The LSP process was ready during
the session, and the MCP offers explicit project stop controls, but that does not prove whether rust-analyzer is stopped
automatically when an Agent disconnects. Document and test the lifecycle, including restart and abandoned-session
cleanup.

### 11. Is the MCP ready for unattended coding?

Not yet. It is useful for supervised navigation and bounded edits when rust-analyzer is healthy, but cross-run semantic
inconsistency and blocked or timed-out flycheck make unattended decisions unsafe. Require a successful build/check and
review of edits; treat missing, truncated, or timed-out results as inconclusive and fall back to source inspection or
Cargo commands.

## Recommendation

Keep using Aimer MCP as a supervised Rust navigation and editing assistant. Before relying on it for unattended coding,
address three gaps:

1. Make semantic queries repeatable across fresh Bamboo runs and add a regression check for the observed inconsistency.
2. Make flycheck failures actionable and clearly distinguish a Cargo failure from a timeout or unavailable runner;
   retain a dependable Cargo CLI validation path.
3. Return native structured results with concise error categories, operation/phase context, and stable truncation
   metadata. Measure representative tool responses before making token-efficiency claims.

Also align the live MCP schema with the repository source, and document and verify rust-analyzer process ownership and
cleanup on Agent disconnect. Until these are covered, require human review of edits and a successful independent
build/check for code changes.
