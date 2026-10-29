# aimer_focusable

Widgets that take part in keyboard focus: `Focusable`, which makes a region a
focus target, and `FocusScope`, which confines focus to its subtree while it
traps.

The focus system itself lives in `aimer_focus`; these widgets connect it to the
widget tree. Built on `aimer_widget_core`; use it through `aimer_widget`.
