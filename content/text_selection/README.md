# aimer_text_selection

Text selection for Aimer: the session that spans several texts, the
`SelectionArea` region that scopes it, the touch knobs and callout, and the
geometry each selectable text publishes.

It sits below `aimer_text`, which re-exports the public pieces
(`SelectionArea`, `SelectionParticipant`, `TextSelection`, `TextSource`, ...),
and judges the hold a finger must complete before it selects with the
recognizer in `aimer_gesture`, the one behind every `GestureDetector`, so a
long press means the same thing on text as on any other widget. Applications normally reach
this crate through `aimer_text`; the other items are public for the text
widgets that take part in a selection.
