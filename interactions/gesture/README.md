# aimer_gesture

Gesture recognition for Aimer: pointer events in, gesture events out.

The recognizer is a pure state machine (`recognize`) that can be driven with a
fake clock, and `GestureDetector` is the thin widget over it that every
interactive widget of the framework shares. It lives in its own crate, below
`aimer_text`, so that text selection can judge a finger's hold with the same
recognizer buttons and scroll bars use.

`aimer_input` re-exports this crate as `aimer_input::gesture`.
