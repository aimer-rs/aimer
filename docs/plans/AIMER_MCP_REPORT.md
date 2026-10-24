# Aimer MCP Field Report: Session Experience and Improvement Points

**Date:** 2026-10-06
**Scope:** One Claude Code session in the `aimer-performance-opt` worktree (macOS). The work was an event-handling
diagnosis followed by a Render v2 integration audit.
**Builds on:** `SURVEY.md` / `SURVEY_ANSWER.md` (2026-10-03). This report does not repeat those answers; it adds
evidence from a second, independent session and lists what is still untested.

## Executive finding

Once the server is running, the navigation tools are useful: `start_lsp`, `inspect_file` and `search_text` were
accurate, bounded and fast to use. Three problems cost the most time:

1. **The server could not start in this repository, and the client only saw `CONNECTION_CLOSED`.** The real cause was
   printed to stderr and never reached the client or the model. No semantic tool was available for the first part of
   the session, so exploration fell back to `rg`/`sed`. (F1)
2. **`find_implementations` returned a large, unsorted result with no metadata, and it was silently incomplete.** It
   could not be used as a census; `rg` had to be the ground truth. (F2)
3. **Filters and size controls do not match what the tool descriptions promise.** `search_symbols(kind="trait")`
   returned nothing, and `get_document_symbols` is not compact. (F3, F4)

Recommendation, consistent with the earlier survey: keep using Aimer MCP as a **supervised navigation aid**, and
cross-check any "list all X" answer against a text search until result completeness is reported by the tool.

## Timeline

| Step | What happened | Outcome |
| --- | --- | --- |
| Session start | `aimer` MCP failed, `codegraph` MCP failed (`ENOENT`, binary not on `PATH`) | No semantic tool; `rg`/`sed` used |
| "start aimer lsp server" | No `start_lsp` tool existed to call; ran `aimer mcp` by hand | `no Aimer.toml found …` (F1) |
| `/mcp` reconnect | `Failed to reconnect to aimer: CONNECTION_CLOSED` | Still down |
| Later turn | `claude mcp get aimer` → Connected, config unchanged; tools appeared | Working (cause not proven, F1) |
| `start_lsp` | `health: ok`, `status: ready`, `workspace_ready: true` | Good |
| Render v2 audit | `search_symbols`, `find_implementations`, `inspect_file`, `search_text`, `get_document_symbols` | Mixed, see F2–F6 |

## Findings

### F1. Startup failure is opaque and tied to project discovery

**Evidence**

- `claude mcp get aimer`: `Failed to connect`, `CONNECTION_CLOSED`. The server is a **user-scope** stdio entry:
  `/Users/cottons/.cargo/bin/aimer mcp`, with no `--project`.
- Running the same command by hand from the repository root, with an MCP `initialize` request on stdin, printed:
  `Error: no Aimer.toml found from '<cwd>'; run this command from an Aimer project or pass --project`
  and exited. The client never sees this text.
- `Aimer.toml` exists only in `jaime/` and `website/`. The repository root, where the agent was started, has none.
- `aimer mcp --help` describes the command as *"Expose a running Aimer session as an MCP server"*. Its options are
  `--install <AGENT>…`, `--global`, `--project <PROJECT>` ("Project path used for automatic session discovery") and
  `--attach <ATTACH>`.
- Later, the identical command from the identical directory answered `initialize` correctly
  (`serverInfo: {name: "rmcp", version: "3.5.1"}`) and `claude mcp get aimer` showed *Connected*. The only difference I
  observed was that `aimer sessions` now listed a running session under `jaime/`
  (`running macos 55880 …/jaime`).

**Interpretation (not proven).** Startup appears to depend on finding an Aimer project or a live session. I did not
stop the running session to confirm causality, and I did not see what changed between the failure and the success.

**Why it matters.** The one tool that selects a workspace (`start_lsp(project_path)`) cannot be reached when the server
exits before the handshake. That is circular, and `/mcp` reconnect cannot fix it. It also defeats the fallback chain in
`AGENTS.md` ("Aimer MCP, else Codegraph"): both were down, so the chain ended at the shell.

**Improvements**

1. Always complete the MCP handshake. When no project or session is found, serve `tools/list`, answer `status` with a
   structured `no_project` error, and let `start_lsp(project_path=…)` pick a Cargo workspace.
2. If exiting is unavoidable, send the reason as a JSON-RPC error to `initialize` instead of writing to stderr and
   closing, so the client UI and the model can read it.
3. Separate the two roles: *Rust LSP tools* (need only a Cargo workspace) and *app/session controls* (need a running
   Aimer session). The LSP half should not depend on the second.
4. Make the error message mention sessions (`aimer sessions`) as well as `Aimer.toml`, and show the exact fix.
5. Document install guidance for repositories whose root has no `Aimer.toml` (this one). *Untested workaround:*
   register the server with an explicit `--project <path>`; I have not run this.
6. Set `serverInfo.name` to `aimer` and the version to the CLI version. It currently reports the SDK defaults
   (`rmcp` / `3.5.1`), which makes `/mcp` output and bug reports ambiguous.

### F2. `find_implementations` is large, unsorted, unlabeled and incomplete

**Evidence** (call on the `Drawable` trait, `drawable.rs:143`)

- Roughly 250 results in one response. There is no `limit`, `offset`, `truncated` or `complete` field. By entry size
  (~220 bytes each) I estimate on the order of 50 KB; this is an estimate, not a measurement.
- Results are **not sorted**. The first entries alternate between files, for example `aimer_range/src/visuals.rs:266`,
  then `widgets.rs:788`, then `visuals.rs:462`, then `visuals.rs:62`. Diffing two runs would be hard.
- Production code, `#[cfg(test)]` modules (indented impls in `aimer_quiver/src/aimer_app.rs`, `handler.rs`,
  `event/tests/*`), examples and `aimer_laboratory` are mixed with no marker.
- Each entry has only a file and a range. It does not say which type implements the trait, so each hit needs a follow-up
  read.
- Two hits are reported at `aimer_input/src/input_field/raw_fields.rs:60`, the
  `include!("raw_fields/caret_host.rs")` line, instead of a position in the included file. Impls from another
  `include!`d file (`raw_fields/layout.rs`) were reported at their real location, so the mapping is inconsistent.
- **Silently incomplete.** I found no entries for `aimer_container`, `aimer_flex`, `aimer_scroll`, `aimer_grid`,
  `aimer_picker`, `aimer_svg`, `aimer_assets`, `aimer_ctxmenu` or `aimer_selection`. These crates do implement
  `Drawable`: for example `RawContainer<T>` at `container.rs:602`, found through `get_document_symbols`. I did not
  determine whether the cause is a result cap or rust-analyzer's view of those crates.

**Improvements**

- Add `limit` / `offset`, a `file_glob`, and `include_tests: false` as the default.
- Sort deterministically (path, then line).
- Return `name`, `container` / `header` (for example `impl Drawable for RawContainer<T>`), `crate`, and
  `in_test_cfg`.
- Return `complete: bool` with a `reason` when the engine's answer is partial or capped. A caller must be able to tell
  "no more results" from "stopped early".
- Report positions in the real source file for `include!`d code.
- Accept a symbol path (`aimer_widget::Drawable`) as an alternative to `file` + `line` + `column`. Today it takes
  two calls: `search_symbols`, then `find_implementations`.

### F3. `search_symbols` kind filter returns an empty result with no error

**Evidence**

- `search_symbols(query="Drawable", kind="trait")` returned `{"results":[]}`. The tool description lists `trait` as a
  valid kind.
- Without the filter, the trait is reported as `kind: "interface"`. The `#[proc_macro_derive]` function is reported as
  `"function"`, and a `pub use` re-export (`aimer_widget/src/lib.rs:106`) is another `"interface"` hit, with nothing
  to tell it apart from the definition.
- `get_document_symbols` uses the same LSP-style kinds: `impl` blocks appear as `"struct"` (named
  `impl Widget for Container<W>`) and local bindings as `"variable"`.

**Improvements**

- Accept Rust names (`trait`, `impl`, `macro`) and map them to the engine's kinds, or document the real values.
- Reject an unknown `kind` with an error that lists the valid values, and add a hint when a filter yields zero results.
- Add `crate`, `module_path` and `is_reexport` to each result.

### F4. `get_document_symbols` is not compact

**Evidence.** On `container.rs` (~1,600 lines) it returned the full nested tree including **every local binding**
(`scale_bits`, `p_w`, `m_left`, …). The output is tens of KB, and the useful part is buried: the
`impl Drawable for RawContainer<T>` node whose children show `can_paint_local_v2` at line 607. The description says
"compact".

**Improvements**

- Default to a small `max_depth` and exclude `variable` and `field` unless requested.
- Add `kinds` and `name_filter` parameters. A query like "which `Drawable` methods does this type override" should be
  one small call.

### F5. Position errors do not say what is wrong

`find_implementations` at line 141, column 12 failed with `column is outside the source line`. The tool was right: I
had miscounted the line by two, and the trait is at line 143. The message does not include the line length or text, so
the caller cannot self-correct.

**Improvements:** include `line_length` (and optionally the line text) in the error, and offer the symbol-path input
from F2.

### F6. What worked and should be kept

- **`start_lsp`**: structured result with `health`, `status`, `workspace_ready`, `proc_macro_enabled`, the proc-macro
  server path and `project_root`. Readiness is explicit, which matches the survey's answer to Q3.
- **`inspect_file`**: several line ranges and regex searches in one call replaced many `sed`/`rg` invocations.
  Validation is precise (`search context must be between 0 and 5 lines` when I passed 45).
- **`search_text`**: bounded, ignore-aware, returns line text and column. Good for checking whether a macro derive is
  used anywhere.
- **`get_document_symbols`**: the structure is accurate and showed exactly which trait hooks a type overrides.

Small fixes: the `inspect_file` schema documents `context_before` / `context_after` as 0–5 in prose but declares no
`maximum`, so clients cannot pre-validate. `search_text` has no regex mode although `inspect_file` does.

### F7. Naming and documentation drift

- The `aimer-rust` skill tells the agent to select a workspace with `aimer_rust_start_lsp`. The live tool is
  `start_lsp` (`mcp__aimer__start_lsp`). The earlier survey already noted an older `aimer_rust_start_lsp` schema, so
  the tool has been renamed without the skill following.
- The skill also says "Use Codex editing tools" and "Codex session", wording that is wrong for other agents.

**Improvements:** generate the skill's tool list from the server's `tools/list`, and add a check that the skill only
references tool names the server exposes. Keep the agent-neutral wording.

### F8. Output is JSON carried as text

Confirms survey Q7/Q8. `inspect_file` returns source as a JSON string with `\n` escapes, so line numbers must be
reconstructed from `start_line`. Return native structured content, and offer a plain-text variant with numbered lines
for source ranges.

## How this session compares with the earlier survey

| Earlier answer | This session |
| --- | --- |
| Q3: waits for indexing | Confirmed (`ready` / `workspace_ready: true` from `start_lsp`) |
| Q4: results repeatable across runs | **Not tested** (one run). `find_implementations` order is unsorted, which makes run-to-run comparison harder |
| Q7: bounded, structured output | True for `search_*`; **not** for `find_implementations` or `get_document_symbols` |
| Q9: token use unmeasured | Rough size estimates only (F2, F4); still not measured |
| Q2: ordinary Rust project | Not tested; only the Aimer framework workspace was used |
| Q5, Q6, Q10: flycheck, edit safety, process cleanup | **Not exercised** |

## Prioritized improvements

| Priority | Item | Finding |
| --- | --- | --- |
| P0 | Complete the handshake without a project; surface the reason to the client | F1 |
| P0 | Report completeness (`complete` / `truncated`) on every list-returning tool | F2 |
| P1 | Validate `kind`, alias Rust names, and add `crate` / `is_reexport` | F3 |
| P1 | Add `limit`, sorting, `name` / `header` and `in_test_cfg` to `find_implementations` | F2 |
| P1 | Add `max_depth`, `kinds`, `name_filter` to `get_document_symbols` | F4 |
| P1 | Align the skill's tool names with the server | F7 |
| P2 | Real `serverInfo` name and version | F1 |
| P2 | `maximum: 5` in the `inspect_file` schema; regex mode for `search_text` | F6 |
| P2 | Line length in position errors; symbol-path input | F2, F5 |
| P2 | Native structured content | F8 |

## Acceptance checks (to make the fixes testable)

1. With no `Aimer.toml` and no live session, `aimer mcp` completes `initialize` and `tools/list`, `status` returns a
   structured `no_project` error, and `start_lsp(project_path=<Cargo workspace>)` succeeds.
2. `search_symbols(query="Drawable", kind="trait")` returns the trait definition.
3. `find_implementations` on a trait with many impls returns `complete` / `truncated` metadata and a stable order. The
   result includes `RawContainer<T>` (`aimer_container`), or `complete: false` with a reason.
4. `get_document_symbols` on `container.rs` returns no local bindings by default and stays under an agreed size.
5. A unit test fails if the skill references a tool name that `tools/list` does not contain.

## Guidance for agents until these are fixed

- Treat `find_implementations` and `search_symbols` as **cross-checks, not censuses**. Establish "all implementors of X"
  with a repository text search, then confirm individual cases semantically.
- Prefer `search_symbols` with a small `limit` to get a position, then call the position-based tools.
- Request `get_document_symbols` only for small files, or use `inspect_file` with a regex for the one method needed.
- If the server is down at session start, check `aimer sessions` and the `Aimer.toml` location before retrying.

## Not covered and caveats

- One session, one machine, one workspace. Nothing was timed.
- Sizes in F2 and F4 are estimates from entry counts, not measured token costs.
- I did not exercise `cargo_check`, `check`, `cargo_test`, `preview_edit`, `apply_edit`, the rename / code-action /
  format previews, the resource-monitoring tools or the crate-docs tools.
- The cause of the F1 recovery is unconfirmed. The omissions in F2 were not traced to a cap or to rust-analyzer.
