# aimer_shortcut

Keyboard shortcuts for Aimer: local (inside a window), menu, and global
(system-wide).

- [`KeyChord`] is what the user presses: `"CmdOrCtrl+Shift+K"`, `"F5"`.
  `CmdOrCtrl` is Command on macOS and Control everywhere else.
- [`ShortcutScope`] is a set of chords bound to actions. The `Shortcuts` widget
  in `aimer_widget` puts one in the tree: it is active while focus is inside its
  subtree, the innermost scope wins, and a [window](ShortcutScope::window) scope
  is active wherever focus is.
- [`menu`] describes the application menu. macOS shows it as a native menu bar
  with the shortcut beside each item; elsewhere the same chords run as
  window-wide shortcuts.
- [`global`] claims a chord system-wide so it fires while the application is in
  the background.

The chord model, [`Keymap`] and [`resolve`] know nothing about windows, focus or
the operating system; the widget tree and the platform layer are built on them.

```
use aimer_shortcut::{Keymap, resolve};

let mut window = Keymap::new();
window.bind("CmdOrCtrl+K".parse().unwrap(), "command palette");

let chord = "CmdOrCtrl+K".parse().unwrap();
assert_eq!(resolve([&window], &chord), Some(&"command palette"));
```

Availability: macOS, iOS, Android 7+, Windows, Linux, Web

## Dispatch order

A key press goes to the focused element first, so a text field keeps its own
`Ctrl+C`. What it declines is offered to the scopes around the focused element,
innermost first, then to window scopes, then to the application menu. The first
to bind the chord runs and the press is spent. Held keys do not repeat an action
unless the scope opts in with [`repeat`](ShortcutScope::repeat).

## Platform status

| | Local | Menu | Global |
|---|---|---|---|
| macOS | run | native menu bar, run | Carbon, run |
| Windows | compile-checked | as window shortcuts | `RegisterHotKey`, compile-checked |
| Linux | compile-checked | as window shortcuts | X11 grabs, compile-checked |
| iOS, Android, web | compile-checked | as window shortcuts | `Unsupported` |
