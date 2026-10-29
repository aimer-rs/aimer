# aimer_element

The element layer of the Aimer widget framework: everything a tree of elements
needs to be built, laid out, painted, hit-tested and kept in sync, without
knowing what a widget is.

- `Element`, `AnyElement` and the traits it is built from (`VisitorElement`,
  `EventElement`, `LayoutElement`, `Rebuildable`, `Drawable`).
- `BuildContext`, the inherited-state scope every element is built under.
- Event dispatch, focus hand-off, pointer claims and the element id / path
  indexes.
- Reconciliation plans and the state carry that moves live state from a replaced
  subtree into its replacement.
- Per-frame bookkeeping: paint damage, invalidation batches, frame statistics,
  window metrics, safe-area insets and platform brightness.

Widgets (`Widget`, `StatelessWidget`, `StatefulWidget`, `ChildBuilder`) live in
`aimer_widget_core`; most applications use these types through `aimer_widget`,
which re-exports every layer.
