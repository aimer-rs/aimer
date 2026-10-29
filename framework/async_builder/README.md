# aimer_async_builder

Widgets that build from the outcome of a future: `AsyncBuilder`, the
`AsyncSnapshot` it hands to its builder, the `FutureSnapshotExt::waitme` handle
and the keyed `watch` slots that keep a running request alive across rebuilds.

Built on `aimer_widget_core`; use it through `aimer_widget`, which re-exports it.
