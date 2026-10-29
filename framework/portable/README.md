# aimer_portable

The reflection layer behind portable widgets: stable identities, type and widget
schemas, the binary codec and the widget registry.

Nothing here knows what an element or a widget is, so the crate sits below the
widget layer. The guest-side Widget IR, the encoder and the materializer, which
turn this vocabulary into widgets, live in `aimer_widget_core`.

Use these types through `aimer_widget::portable`, which re-exports this crate
next to the parts that need widgets.
