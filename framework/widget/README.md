# aimer_widget

The widget framework's public face. This crate holds no code of its own: it
re-exports the layers it is made of, so `aimer_widget::Widget`,
`aimer_widget::Element` and the rest keep their paths.

| Crate | What it owns |
| --- | --- |
| `aimer_element` | elements, `BuildContext`, events, reconciliation, frame bookkeeping |
| `aimer_portable` | portable reflection: identities, schemas, codec, registry |
| `aimer_widget_core` | `Widget`, stateless / stateful widgets, `ChildBuilder`, diagnostics |
| `aimer_async_builder` | `AsyncBuilder`, `AsyncSnapshot`, `waitme`, `watch` |
| `aimer_focusable` | `Focusable`, `FocusScope` |

Depend on a layer directly when you only need that layer; depend on this crate
for everything.
