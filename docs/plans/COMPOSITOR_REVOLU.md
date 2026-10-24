# Aimer Combination Compositor

## Summary

Aimer uses one in-process, platform-independent compositor built on the
existing wgpu renderer. The design combines:

- Core Animation's retained property tree for stable visual content.
- Wayland's immutable frame transaction and explicit damage semantics.
- DirectComposition's selective GPU-surface promotion and memory budgeting.
- X11's conservative multi-region damage model.

The lowering flow is:

```text
Widget Tree -> Element Tree -> immutable Scene transaction
           -> RetainedSceneTree commit -> retained paint buffers/surfaces
           -> damage-aware raster -> direct composition -> window surface
```

The scene transaction is lowered beside the existing draw payload. Stable,
bounded elements can be retained without a wrapper: small streams are replayed
as commands, while larger safe streams become renderer-owned surfaces.
`RepaintBoundary` is the explicit priority hint for a subtree that should be
isolated even when it is small or dynamic. Dynamic, unbounded, oversized, or
unsupported content remains on the live command path.

## Implementation

- Add `aimer_cupid::compositor` with logical scene nodes, clip chains, ordered
  paint operations, surface properties, damage footprints, scene diffs, and
  `RetainedSceneTree`.
- Keep the public `Frame` shape and existing `FramePacket::new`/`full` behavior
  unchanged; attach an optional scene to packets through `FramePacket::with_scene`.
- Lower the Element Tree into an immutable scene transaction during paint.
  Record node identity, bounds, clip state, transform, opacity, ordering, and
  command/surface ranges while preserving the DrawList as the safe payload
  fallback. Native and web frame builders attach the transaction to packets.
- Commit each packet into `RetainedSceneTree`; unchanged node records stay in
  place, while changed geometry, clips, order, opacity, content, additions,
  and removals produce conservative old/new damage.
- Add public `aimer_widget::RepaintBoundary<W>` with a type-state
  `new().child(child)` builder. It forwards layout, reconciliation, events,
  focus, hit testing, and geometry while supplying a priority retained-paint
  owner. Share its cache with automatic stable-element retention.
- Extend the renderer to rasterize a retained surface only when its content
  changes, skip surfaces outside each normalized damage region, reuse the
  persistent target for empty damage, and expose compositor work counters.
- Preserve safe fallbacks for materials/backdrop effects, multisampling,
  non-finite geometry, texture/resource changes, and portable guest builds.
- Keep this compositor in-process; do not add native CALayer,
  DirectComposition, Wayland, or X11 surface adapters in this slice.

## Acceptance tests

- Scene packets preserve target size, surface identity, paint order, transform,
  opacity, and damage metadata.
- Stable boundary paint is recorded once and replayed; dynamic boundary paint
  stays on the live path.
- A stable bounded element is retained without a `RepaintBoundary`; a small
  command stream remains command-retained and a larger safe stream produces a
  surface.
- A warm `RetainedSceneTree` commit updates zero nodes when scene properties
  are unchanged and updates only the changed node when one property changes.
- Boundary layout, event, focus, hit-test, and reconciliation behavior remains
  transparent.
- Old and new bounds are damaged when a promoted region moves, resizes, is
  clipped, or is removed; unknown/non-finite paint promotes to full damage.
- Multiple disjoint damage regions repaint correctly without forcing a full
  target repaint; empty damage reuses the persistent target.
- Nested retained content is not rendered twice and normal draw order remains
  pixel-identical for rectangles, text, images, opacity, transforms, shadows,
  and unsupported material effects.
- Legacy `Frame` and packet constructors remain source-compatible.
- Validate native, wasm, and portable-guest compilation plus focused, crate,
  and workspace tests. Existing unrelated failures must be reported rather
  than hidden.

## Defaults and constraints

- A boundary is an optimization hint, never a mandatory texture allocation.
- No widget receives its own GPU surface by default.
- Existing retained-layer dimensions, tile size, idle lifetime, and 64 MiB
  cache budget remain the initial limits.
- Automatic promotion is adaptive: a small standalone stream is cheaper to
  replay than to allocate and composite as a texture; explicit boundaries may
  override that choice.
- No new third-party dependency is required.
- Debug-mode hot paths remain the performance priority; claims of savings are
  supported by compositor counters and pixel regression tests.
