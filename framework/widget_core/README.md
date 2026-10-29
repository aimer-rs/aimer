# aimer_widget_core

The widget layer of the Aimer framework: how a description of a UI becomes an
element tree and stays in sync with it.

- `Widget`, `AnyWidget` and the builder idiom (`RequiredChild`).
- `StatelessWidget` and `StatefulWidget`, with `State` and `StateUpdater`.
- `ChildBuilder`, retained children and `BuildContextRetain::retain`.
- Panic recovery that turns a failing build into an error widget, and the
  `ErrorWidget` / `OverflowIndicator` diagnostics.
- `RepaintBoundary` and `Semantics`.
- The portable widget IR, which lowers widgets for guest code.

It sits on `aimer_element` (elements, contexts, events) and `aimer_portable`
(reflection). `aimer_async_builder` and `aimer_focusable` build on it; most
applications reach everything through `aimer_widget`, which re-exports every
layer.
