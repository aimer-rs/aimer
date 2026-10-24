# Aimer — Development Instructions

You are a handful and genius assistant :).These instructions apply to the entire repository. A nested `AGENTS.md` takes precedence for files in its directory.

> Do not mentioned your self in commit

## Critical Considerations

Aimer is a performance-sensitive GUI framework.

- When makes a vague request, be sure to ask them to clarify it.
- Optimize performance to the best possible level by any means necessary.
- Treat per-frame rendering, layout, input, animation, and widget rebuild paths as hot unless evidence shows otherwise.
- Avoid avoidable allocations, clones, repeated tree walks, blocking work, and unnecessary synchronization on hot paths.
- Do not claim a change is faster without evidence. For non-obvious performance changes, add or run a benchmark,
  profiler, allocation check, or equivalent measurement.
- Do not trade correctness or maintainability for speculative micro-optimizations.
- When changing GPU buffers, FFI, or platform code, validate sizes, alignments, lifetimes, and thread-affinity
  assumptions.
- Assume mobile phone performance is slower than PC in range of 3 ~ 10x for performance benchmark

## Priorities

When requirements compete, use this order:

1. Correctness and safety.
2. The user's explicit request and acceptance criteria.
3. Performance on measured or clearly hot GUI paths, Strongly focus on Debug mode optimization, for Release mode it is
   just optimized by compiler, not logic optimization.
4. Consistency with the surrounding module.
5. Minimal complexity and avoid third party dependencies when possible.

Do not stop at a superficial workaround. Find and fix the root cause while keeping the change within the requested
scope.

## Rust source layout

- Keep every project-owned `.rs` file below 1,000 ~ 1600 physical lines, including tests and documentation comments.
- When a file approaches the limit, move cohesive responsibilities into sibling modules and keep the parent module as a small facade.
- Name modules after the responsibility they own. Re-export only the items needed by callers; keep implementation details private.
- Keep tests beside the behavior they exercise, using child test modules when that keeps source files within the line limit.
- Prefer flat layout inside a crate, when need the inner module, it mean crate is too big.

## Before Editing

- Read the relevant implementation, tests, and any nested `AGENTS.md` before changing code.
- Use available Aimer MCP or fallback to Codegraph MCP when Aimer MCP is unavailable, but do not assume an IDE integration exists. CLI
  inspection is valid.
- Use the aimer mcp or cli tool to search the documents from the docs.rs if the aimer is existed.
- Do not invent missing facts. First, inspect the repository, compiler output, tests, or documentation. Ask the user only
  when the ambiguity cannot be resolved locally and different answers would materially change the implementation.
- Preserve unrelated user changes. Do not revert or overwrite them.
- Match existing module patterns unless this file or the user explicitly requires otherwise.

## Sco

- For requests to explain, review, diagnose, or plan: inspect and report; do not modify code unless asked.
- For requests to build, change, or fix: make the in-scope local edits and run relevant non-destructive checks without
  asking first.
- Ask before destructive actions, external writes, adding a substantial dependency, changing public API beyond the
  request, or materially expanding scope.
- Do not create commits, branches, pull requests, or publish artifacts unless the user explicitly asks. Pull requests
  must be created by a human unless the user overrides this rule.

## Test-Driven Development

Use red-green-refactor for behavior changes:

1. Add a focused test that fails for the intended reason.
2. Run it and confirm the failure.
3. Implement the smallest complete production change that makes it pass.
4. Run the focused test again, then the relevant crate tests.
5. Refactor only while the tests remain green.

Additional test rules:

- Bug fixes require a regression test.
- New behavior must cover its happy path plus relevant invalid-input and boundary cases.
- Pure refactors may rely on existing tests when they already cover the affected behavior.
- Prefer inline `#[cfg(test)] mod tests` modules next to the code they cover, matching the prevailing crate pattern.
- Keep tests deterministic. Fix time and random inputs; use tolerances for floating-point assertions where appropriate.
- Never delete, ignore, weaken, or skip a test merely to make the suite pass.
- If a pre-existing failure is unrelated, report it with evidence; do not silently work around it.
- If a useful test is impractical, explain why before implementing and use the strongest feasible verification.

## Rust and API Design

- Do not expose third party crate to public API.
- Use the latest stable Rust toolchain supported by the workspace.
- Prefer clear ownership and borrowing over allocation. Apply zero-copy techniques when they simplify or measurably
  improve a hot path; do not introduce unsafe code or complex lifetimes without demonstrated benefit.
- Keep `unsafe` blocks minimal and document their safety invariants.
- Never hardcode or log credentials, tokens, signing keys, or other secrets.
- Prefer existing workspace dependencies and standard-library solutions. Add a third-party crate only when its
  maintenance, correctness, or complexity benefit justifies it.
- Prefer the crate version that published more than 1 ~ 2 weeks ago to prevent malicious crates from being introduced.
- Prefer using the git url with a specific commit hash or tag for introduce external crate when possible.
- Declare shared dependencies in root `[workspace.dependencies]`, then use `<dependency>.workspace = true` in member
  crates.
- New crate names use the `aimer_*` prefix.
- Route public umbrella-crate re-exports through `src/lib.rs`. Use `pub use aimer_xxxx as xxxx` when exposing a crate
  under a shorter public name.
- Keep source files at or below 2,000 lines (inline-unittest excluded). Split by responsibility before exceeding that
  limit.
  `gesture/drag.rs`; do not introduce `mod.rs` files.
- Add comments for invariants and non-obvious decisions, not for code that is already self-explanatory.
- Do not run `cargo fmt`. Preserve the surrounding formatting style in edited code.
- Add documentation comments to new public APIs. Document purpose, important invariants, panics/errors, safety
  requirements, and a useful example when appropriate. Follow Rust standard-library style, but keep documentation
  proportional to the API.

Example: 
```rust

/// Confines keyboard focus to its subtree while it traps.
///
/// A focus scope changes nothing about layout or painting: it wraps a region and
/// declares that, while it is in the tree, only the focusable targets *inside*
/// it may own the keyboard. `Tab` and `Shift-Tab` therefore cycle within the
/// scope instead of walking out into the application behind it, and no element
/// outside it can be given focus — which is exactly what a dialog rendered
/// inline needs, and what distinguishes a mode from an ordinary panel.
///
/// Entering a scope remembers whatever owned focus outside it, and leaving the
/// scope — dismissed, navigated away from, or simply rebuilt out of the tree —
/// gives focus back to it if it is still there. Nothing has to be restored by
/// hand.
///
/// Scopes nest: the innermost trapping scope is the one that confines focus, so
/// a dialog inside a drawer traps within the dialog, and closing it hands the
/// keyboard back to the drawer.
///
/// A scope that does not [trap](FocusScope::traps) is inert, which is what makes
/// the flag worth having: a widget that is sometimes a mode — a panel that
/// becomes modal on a small screen — keeps one build path and flips a `bool`.
///
/// Overlays presented through `aimer_modal` need no scope. Their content is not
/// part of the tree they cover, so they confine focus with an
/// [`aimer_focus::FocusTrap`] instead; the effect for the application underneath
/// is the same.
///
/// # Examples
///
/// ```
/// use aimer_widget::FocusScope;
///
/// struct Dialog;
/// # impl aimer_widget::PortableWidget for Dialog {}
/// # impl aimer_widget::Widget for Dialog {
/// #     fn to_element(self, _ctx: &aimer_widget::base::BuildContext) -> aimer_widget::AnyElement {
/// #         unreachable!("this example only builds the widget")
/// #     }
/// # }
///
/// // While this is in the tree, Tab cannot leave the dialog.
/// let modal = FocusScope::new().child(Dialog);
///
/// // The same region, not confining anything.
/// let inline = FocusScope::new().traps(false).child(Dialog);
/// ```
pub struct FocusScope<W = RequiredChild> {
    child: W,
    traps: bool,
}

impl FocusScope {
    /// Creates a trapping scope builder.
    ///
    /// Finish the builder with [`FocusScope::child`] or
    /// [`FocusScope::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            traps: true,
        }
    }

    /// Sets whether the scope confines focus to its subtree.
    ///
    /// The default is `true`. A scope built with `false` behaves as if it were
    /// not there.
    #[inline]
    pub fn traps(mut self, traps: bool) -> Self {
        self.traps = traps;
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> FocusScope<W> {
        FocusScope {
            child,
            traps: self.traps,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}
```

## Widget Conventions

For structs that implement `Widget`:

- Provide a zero-argument `new()` that returns the incomplete/default builder state.
- If a child is required, make the child-setting method the final type-state transition that produces a valid widget.
- Put the child type parameter last when practical and consistent with the existing API.
- Mark small builder methods `#[inline]`.
- In `Widget::to_element(self, ctx)`, move owned fields into the retained element. Do not clone fields merely because
  ownership is inconvenient: `self` is consumed.
- A self-rebuilding widget should retain its child through `ChildBuilder` or the repository's equivalent retained-child
  mechanism instead of rebuilding or cloning it unnecessarily.

Example:

```rust
pub struct MyWidget<W = RequiredChild> {
    size: f32,
    child: W,
}

impl MyWidget {
    #[inline]
    pub fn new() -> Self {
        Self {
            size: 0.0,
            child: RequiredChild,
        }
    }

    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    #[inline]
    pub fn child<W: Widget>(self, child: W) -> MyWidget<W> {
        MyWidget {
            size: self.size,
            child,
        }
    }
}

impl<W: Widget + 'static> Widget for MyWidget<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawMyWidget {
            size: self.size,
            child: self.child.to_element(ctx),
        }.notype()
    }
}
```

## Validation Commands

Run commands from the workspace root unless a nested guide says otherwise. Prefer using `cargo-nextest` when available.

```bash
# Focused test
cargo nextest run -p aimer_animation test_curve_linear

# Crate tests
cargo nextest run -p aimer_animation

# Entire workspace (use when the change scope warrants it)
cargo nextest run --workspace
```

Before handing off:

- Ensure production and test code compile.
- Run the narrowest relevant checks first, then broaden according to change risk.
- Report exactly what was run and whether it passed.
- Report any checks not run and the reason.
- Summarize changed behavior and identify remaining risks or follow-up work without claiming unverified success.
