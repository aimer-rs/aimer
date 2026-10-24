## About This Crate

This is a crate for implementing the Router system for the Aimer FrameWork.

## Golden Rules

- **Use CodeGraph to understand code.** It is fast and always safe for reading/navigating the
  codebase. Prefer it before opening files blindly.
- **Use the IDE (IDEA/CLion) integration to edit code** when connected — it is the safest, fastest
  path for refactors and renames.
- **Never write "Lazy Senior Dev" code.** Do not merely patch the symptom with spaghetti that other
  developers will curse. Solve the actual problem cleanly.
- **Follow Test Driven Development.** Write the failing test first, then the code that makes it pass.

### Test Driven Development

TDD relies on a very short cycle: turn a requirement into a specific, failing test, then write only
the code needed to make it pass, then refactor. Do not add behavior that isn't proven by a test.

**_Before you write code, write the test cases first!_**

# Implementation Road Map


-[x] step 1
-[x] step 2
-[x] step 3
-[x] step 4
-[x] step 5

### Step 1: Matching foundation: named routes & query parameters
The router resolves URLs including query strings into a typed match with named-route lookup, keeping existing
routes working.

- Introduce RouteMatch { path_params, query_params } and extend the Route trait in
 crates/aimer_router/src/router.rs with a default name() method.
- Update Route::parse/format handling to split and re-emit ?query=... in addition to path segments.
- Extend the macro in crates/aimer_macro/src/codegen/router.rs to accept name = "..." and query placeholders,
 generating a name→variant builder table and query parse/format arms.
- Add push_named(name, params) to NavigatorController in navigator.rs.
- Write failing inline #[cfg(test)] tests first for round-trip parse/format of named and query routes, then
 implement.

### Step 2: Redirects & guards
 Navigation transparently re-routes through guard/redirect hooks on initial load, push, and browser back/forward.

- Add a default Route::redirect(&self, ctx) -> Option<Self> to the trait and a #[redirect(...)] attribute in the
 macro codegen.
- Evaluate redirects in NavigatorState::build and inside push/popstate handling in navigator.rs, with a bounded
 max-hop loop guard.
- Ensure the WASM address bar reflects the final post-redirect route.
- Add regression tests covering Some(route) re-route, None pass-through, and loop-guard termination before
 implementing.

### Step 3: Nested routes with Shell & Outlet
 A persistent shell frame stays mounted while an inner Outlet swaps between child routes.

- Create crates/aimer_router/src/outlet.rs Outlet widget reading the active child from injected shell state and
 crates/aimer_router/src/shell.rs Shell persistent frame.
- Extend the macro to support a #[shell] variant embedding a child route enum and generate build that renders
 the shell + passes the child to the Outlet.
- Export the new modules from crates/aimer_router/src/lib.rs.
- Add tests asserting the shell instance persists while outlet content changes; implement after tests fail.

### Step 4: StatefulShellRoute: per-branch history stacks
 A tabbed shell keeps an independent navigation stack per branch, preserving each branch's state across switches.

- Add StatefulShell a StatefulWidget owning branches: Vec<Vec<R>> and an active index and a
 StatefulShellController go_branch, push_in_branch, active_branch in shell.rs, injected via ctx.insert_state.
- Route only the active branch's top route into the Outlet; ensure branch stacks are stored in State and not
 rebuilt on switch.
- Integrate branch-aware URL formatting with the WASM History API path.
- Add tests for push-in-branch, branch switch, and state restoration before implementing.

### Step 5: Docs, example app, and milestone updates
 A working shell example and accurate documentation ship alongside the new features.

- Build a sample stateful-shell app in jaime/ new screens + updated jaime/src/routing.rs demonstrating named
 routes, a redirect/guard, and a tabbed shell.
- Rewrite aimer_book/src/guide/route.md to document named routes, query params, redirects, shells, and
 per-branch stacks, and fix the existing :id vs {} placeholder drift.
- Tick the corresponding items Named routes, nested/shell in the README.md navigation milestone.
- Verify the full workspace builds and the example runs; run fmt/clippy/tests.