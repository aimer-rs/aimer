# Portable widget checklist

This checklist covers the public, built-in widgets shipped by Aimer. Private
implementation frames, test widgets, and `Element` types are intentionally
excluded.

- `[x]` means the widget uses the `#[derive(PortableWidget)]` macro, with generated schema support and either
  generated or explicitly audited manual lowering.
- `[ ]` means the widget currently uses a handwritten `PortableWidget` implementation.

Every final `Widget` already has an explicit `PortableWidget` capability because
`Widget` extends `PortableWidget`; this checklist tracks derive-based migration.



## Core widgets — [aimer_widget](crates/aimer_widget/src/lib.rs)

- [ ] `AnyWidget`
- [x] `AsyncBuilder`
- [x] `ChildBuilder`
- [x] `NamedWidget`
- [x] `ErrorWidget`
- [x] `OverflowIndicator`
- [x] `FocusScope`
- [x] `Focusable`

## Containers — [aimer_container](crates/aimer_container/src/lib.rs)

- [x] `AspectRatio`
- [x] `Container`
- [x] `Opacity`
- [x] `Resizable`
- [x] `Scalable`
- [x] `SizedBox`
- [x] `ZeroSizedBox`

## Flex and layout — [aimer_flex](crates/aimer_flex/src/lib.rs), [aimer_grid](crates/aimer_grid/src/lib.rs), [aimer_scroll](crates/aimer_scroll/src/lib.rs), [aimer_space](crates/aimer_space/src/lib.rs)

- [x] `Expanded`
- [x] `Flex`
- [x] `Column`
- [x] `Row`
- [x] `ListFlex`
- [x] `Grid`
- [x] `Scrollable`
- [x] `ScrollBar`
- [x] `Align`
- [x] `Positioned`
- [x] `Stack`

## Input — [aimer_input](crates/aimer_input/src/lib.rs)

- [x] `Button`
- [x] `GestureDetector`
- [x] `TextField`
- [x] `TextArea`
- [x] `MouseRegion`

## Text — [aimer_text](crates/aimer_text/src/lib.rs)

- [x] `Text`
- [x] `RichText`
- [x] `TextButton`
- [x] `SelectionArea`

## Animation and theme — [aimer_animation](crates/aimer_animation/src/lib.rs), [aimer_style](crates/aimer_style/src/lib.rs)

- [x] `Animated`
- [x] `AnimatedBuilder`
- [x] `AnimatedSwitcher`
- [x] `ImplicitAnimatedBuilder`
- [x] `MorphTransition`
- [x] `FadeTransition`
- [x] `SlideTransition`
- [x] `ScaleTransition`
- [x] `RotationTransition`
- [x] `AnimatedTheme`

## Modal, drag-and-drop, and menus — [aimer_modal](crates/aimer_modal/src/lib.rs), [aimer_dnd](crates/aimer_dnd/src/lib.rs), [aimer_ctxmenu](crates/aimer_ctxmenu/src/lib.rs)

- [x] `Anchor`
- [x] `Floating`
- [x] `Modal`
- [x] `ModalHost`
- [x] `Draggable`
- [x] `DropZone` (`DropZone<HasChild>`)
- [x] `DragTarget` (`DragTarget<T, HasChild>`)
- [x] `ContextMenu`
- [x] `ContextMenuRows`

## Assets and content — [aimer_assets](crates/aimer_assets/src/lib.rs), [aimer_svg](crates/aimer_svg/src/lib.rs), [aimer_markdown](crates/aimer_markdown/src/lib.rs)

- [x] `Image`
- [x] `AssetImage`
- [x] `NetworkImage`
- [x] `Svg`
- [x] `SvgAsset`
- [x] `MarkdownViewer`

## Routing and providers — [aimer_router](crates/aimer_router/src/lib.rs), [aimer_provider](crates/aimer_provider/src/lib.rs)

- [x] `Navigator`
- [x] `Outlet`
- [x] `Shell`
- [x] `StatefulShell`
- [x] `Provider` (`NotifierProvider` is an alias)
- [x] `StoreProvider`

`MarkdownViewer`, provider widgets, and most SVG exports are feature-gated when
accessed through the umbrella `aimer` crate. `AnyWidget` remains handwritten
because it is the type-erased widget alias and cannot carry a derive. Builder
wrappers and stateful/platform-backed widgets use `schema_only` plus audited
manual lowering where runtime state, native handles, or collection child
adaptation cannot be represented by ordinary field lowering. Native-only fields
and callback signatures outside the guest ABI are explicitly skipped rather than
serialized accidentally.

The standard value codec matrix now covers `Option<T>`, `Result<T, E>`,
`Box<T>`, arrays, tuples, `Vec<T>`, `VecDeque<T>`, `LinkedList<T>`, ordered
maps/sets, `BinaryHeap<T>`, and the `CanonicalHashMap`/
`CanonicalHashSet` adapters. Sequence order, ordered collection order, heap
sorting, encoded-key collisions, malformed tags, limits, and trailing input
are covered by focused codec and derive fixtures.

## Full-contract audit status — 2026-08-24

The migration checkmarks above describe derive-based schema adoption. The
Phase 28 completeness gate additionally requires bounded guest lowering,
permanent host materialization, and focused round-trip coverage. That stricter
audit now passes for all 25 public built-in registry contracts, including
`TextField`, `TextArea`, `Resizable`, `Scalable`, `RichText`,
`ContextMenuRows`, `ContextMenu`, `AnimatedBuilder`, `NamedWidget`, and
`ChildBuilder`. Their callback, closure, controller, platform, and collection
boundaries are represented by explicit descriptors or audited manual
materializers. `AnyWidget` remains intentionally handwritten because it is the
type-erased widget owner rather than a wire-level built-in schema.

The stateful/stateless derive paths are covered by generated guest lowering;
the external `CounterWidget` sample reaches a committed native macOS
generation with `MY_PROJECT_DIR` routing. Normal Button decoration also uses
the bounded `BoxDecoration` value codec; native-only state decorations remain
rejected explicitly.
