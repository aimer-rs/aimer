# aimer_prendre

Sends synthetic input to a running Aimer application from another process.

A driver is a test script, an automation tool, a developer at a terminal,
connects to the app, sends pointer, scroll, key and text events, and the app
handles them exactly like real input: they enter the event loop as the same
`AimerNativePlatformEvent::NativeInput` and `InsertText` the platform bridges
send, so nothing downstream can tell the difference.

```text
driver process                          app process
Client ── hello TOKEN ──► Server ──► EVENT_PROXY ──► event loop
       ◄───── ok ───────        (TCP)
```

## Features

The crate has two sides, each behind a feature, and the commands and wire format
they share are always built.

| Feature  | Adds                                         | For                                |
|----------|----------------------------------------------|------------------------------------|
| `host`   | `Server`, `Config`, `Command::deliver`       | the app that accepts input         |
| `client` | `Client`                                     | a tool or test that sends input    |

Both are on by default for this crate alone. The workspace entry turns the
defaults off, so a dependent says which side it is:

```toml
# the app — a debug or test build only
aimer_prendre = { workspace = true, features = ["host"] }
# the driver
aimer_prendre = { workspace = true, features = ["client"] }
```

`client` has no dependencies at all — not the framework, not even `winit`. The
commands are built from small value types of this crate (`Modifiers`,
`NamedKey`, `PointerButton`, `TouchPhase` and friends) that mirror the
framework's, and only `host` converts them into the framework's, with matches
that list every variant. `host` is the one that reaches into the event loop,
which is why nothing enables it for you.

## In the app

Needs the `host` feature. Start the server deliberately, once the event loop is
running:

```rust,no_run
# #[cfg(feature = "host")]
# {
let server = aimer_prendre::Server::start(Default::default())?;
server.endpoint().save("aimer-prendre.endpoint")?;
# }
# Ok::<(), std::io::Error>(())
```

`Server::start` listens on `127.0.0.1` and picks a free port. Keep the returned
`Server` alive for as long as input should be accepted; dropping it stops
listening and closes its connections. A command that arrives before the event
loop exists is refused with `app unavailable`, so the driver can retry.

## From the driver

Needs the `client` feature.

```rust,no_run
# #[cfg(feature = "client")]
# {
use aimer_prendre::{Client, Command, Endpoint};

let mut app = Client::connect(&Endpoint::load("aimer-prendre.endpoint")?)?;
for line in ["move 120 48", "down 120 48", "up 120 48"] {
    app.send(&line.parse::<Command>().expect("valid command"))?;
}
app.send(&Command::Text("hello".into()))?;
# }
# Ok::<(), std::io::Error>(())
```

`send` returns once the app's event loop has been handed the event, and returns
an error when the server refused it.

## Whole gestures

A click is three events and a drag a dozen. `aimer_prendre::actions` builds the
sequence from the words a person or an agent uses, so a driver says what it
means and gets what a real mouse, keyboard or finger would have produced:

```rust
use aimer_prendre::actions::{self, Direction};
use aimer_prendre::PointerButton;

let double_click = actions::click(120.0, 48.0, PointerButton::Primary, 2)?;
let scroll_down = actions::scroll(Some((200.0, 300.0)), Direction::Down, 3.0)?;
let select_all = actions::key("ctrl+a")?; // or "Return", "cmd+shift+z", "F5"
let swipe = actions::swipe((100.0, 400.0), (100.0, 100.0))?;
# Ok::<(), actions::ActionError>(())
```

`mouse_move`, `click` and `drag` glide to their points (`click` without a point does not glide: it presses where the mouse already is), so what lies on the way
sees the pointer pass; the raw `move` jumps. `scroll` takes an optional point and
otherwise scrolls where the mouse is. `click`, `drag`, `scroll`, `mouse_move`,
`key`, `type_text`, `tap`, `swipe` and `pinch` return
the [`Command`]s to send in order. Scrolling down moves the content up, as a
wheel does. A character with no `ctrl`, `meta` or `alt` held is typed, and with
one held it is a shortcut. Bad numbers are an `ActionError`, never a sequence
the app would mistranslate.

## Wire format

Any language can drive an app: the protocol is one text line per message over
TCP, each answered with one line, `ok` or `err REASON`. The first line must be
`hello TOKEN`.

| Line                                   | Meaning                                                   |
|----------------------------------------|-----------------------------------------------------------|
| `move X Y`                             | The mouse jumped there.                                   |
| `glide X Y [MS]`                       | The mouse glides there, in about `MS` milliseconds (1 to 3000; default follows the distance). Replies at once; see [Gliding](#gliding). |
| `settle`                               | Replies when the mouse has finished gliding.              |
| `trackpad DX DY [MS]`                  | Scrolls smoothly, as a trackpad swipe, where the mouse is; replies when it is over. See [Scrolling](#scrolling). |
| `screenshot [width=N] [jpeg=Q]`        | A picture of the window. See [Screenshots](#screenshots). |
| `down [X Y] [BUTTON]`, `up [X Y] [BUTTON]` | A button went down / came up, at `X Y` or, without a point, where the injected mouse is (after any glide arrives; an error if nothing has placed it yet). Default `primary`. |
| `leave`                                | The mouse left the window.                                |
| `scroll DX DY`                         | A pixel-precise scroll; `kind=line` counts wheel steps.   |
| `touch start\|move\|end ID X Y`        | Finger `ID` touched down, moved or lifted.                |
| `touch cancel`                         | The system took the touch away; presses are cancelled.    |
| `pinch X Y DELTA PHASE`                | A pinch at `X Y`; `DELTA` is the fractional scale change. |
| `rotate X Y DELTA PHASE`               | A rotation in radians, clockwise positive.                |
| `pan X Y DX DY PHASE`                  | A multi-finger pan.                                       |
| `smartzoom X Y`                        | A trackpad smart zoom (two-finger double tap).            |
| `key press\|release\|repeat MODS KEY`   | A named key changed state.                                |
| `text TEXT`                            | Text committed as one edit (like an IME commit).          |
| `preedit RANGE [TEXT]`                 | Text an input method is composing; no text ends it.       |
| `typing MS\|- TEXT`                    | Types the text a keystroke at a time, at a human pace; replies when done. See [Typing](#typing). |
| `soft-backspace`                       | The soft keyboard's backspace.                            |
| `back start\|progress P\|invoke\|cancel` | A step of the system's back (predictive back on Android). |
| `link URL`                             | The app was opened with a deep link.                      |
| `menu undo\|redo\|cut\|copy\|paste\|select-all` | An editing shortcut from the menu bar.        |
| `file-hover PATH`, `file-hover-cancel`, `file-drop PATH` | A file drag entered, left, or dropped.  |
| `quit`                                 | Ask the app to quit in an orderly way, as `SIGTERM` does. |

`BUTTON` is `primary`, `secondary`, `middle` or `other:N`. `MODS` is `-` or
`+`-joined `ctrl`, `shift`, `alt`, `meta`. `KEY` is `backspace`, `delete`, `up`,
`down`, `left`, `right`, `pageup`, `pagedown`, `home`, `end`, `enter`, `escape`,
`tab` or `other:NAME`. `PHASE` is `start`, `move`, `end` or `cancel`. `RANGE` is
`-` or the byte range `START,END` of the active segment inside the preedit text,
which must lie on character boundaries. `P` is a progress from `0` to `1`.

`move`, `glide`, `down`, `up` and `scroll` also take `name=value` options after their
other arguments, in any order and each once: `mods=MODS` on all four, and on
`scroll` also `kind=pixel|line` (default `pixel`) and `phase=PHASE` (default
`move`), for example `scroll 0 -3 kind=line mods=ctrl`.

Positions and deltas are logical pixels in window coordinates and must be finite.
`TEXT`, `NAME`, `URL` and `PATH` run to the end of the line; write `\`, newline and
carriage return as `\\`, `\n` and `\r`. Lines are limited to 64 KiB.

```text
$ { echo "hello $(cut -d' ' -f2 aimer-prendre.endpoint)"; echo "key press ctrl a"; } | nc 127.0.0.1 PORT
```

## Starting it from a launcher

An app can call `Server::start_from_env()` unconditionally in debug builds. It
listens only when the launcher asked for it through the [`env`] variables, so
`aimer run` can drive an app that was not changed for the occasion:

| Variable               | Meaning                                                  |
|------------------------|----------------------------------------------------------|
| `AIMER_PRENDRE`        | The switch: anything but empty or `0` starts the server. |
| `AIMER_PRENDRE_BIND`   | The address to listen on. Default `127.0.0.1`.           |
| `AIMER_PRENDRE_PORT`   | The port. Default `0`, which the OS chooses.             |
| `AIMER_PRENDRE_TOKEN`  | The token. Default: a random one.                        |

A setting that cannot be read is an error, never a silent default. When the
server starts it logs its endpoint on one line; a launcher that owns the app's
console finds it with `Endpoint::from_announcement`, and swaps in the address
that actually leads to the app with `Endpoint::with_ip`.

## Gliding

A real mouse crosses the pixels between two points, and whatever lies on the way
sees it pass: a hover starts and ends, a tooltip timer is cut short. `glide`
does the same, and it does not hold the connection while it does.

- **`glide X Y [MS]` replies at once.** The server steers the injected mouse
  toward the target on a thread of its own, as ordinary moves about once a
  frame. It eases out of the start and brakes into the end, and without `MS` the
  time follows the distance (about 120 ms for a hop, up to 800 ms for a
  crossing). `MS` is capped at 3 seconds.
- **A new target mid-flight bends the path.** The mouse keeps the velocity it
  has and turns toward the new target at a limited rate (7 rad/s), slowing for a
  sharp turn, so a change of mind is an arc and not a jump or a stop. Sending
  `glide` again is how a driver steers.
- **Where it starts.** From where the injected mouse last was. With nothing
  injected yet, or after `leave`, there is nowhere to glide from and it is a
  single move.
- **Order is kept for what needs it.** `move`, `down` and `up` carry a position,
  and first wait for a glide in progress to arrive, so `glide A`, `down A`,
  `up A` is a click on `A` and not a press that cuts the glide short. Everything
  else — keys, text, a `glide` — goes straight through. A command that acts *at*
  the mouse, like `scroll`, sends `settle` first, which replies when the mouse
  has arrived (or with an error after four seconds).

`aimer_prendre::actions` builds these: `click` and `drag` glide to their
points, `mouse_move` glides, and `scroll` takes an optional point: with one it
glides there and settles, without one it scrolls wherever the mouse already is.

## Scrolling

`scroll DX DY` is one wheel pulse, which the app turns into a quick burst.
`trackpad DX DY [MS]` is a swipe: the server sends a steady stream of small pixel
deltas, about every 12 ms, eased so it starts and ends gently, with a start
before the first and an end after the last that stand for the finger landing and
lifting. The app recognises that as a trackpad — a stream faster than one event
every 20 ms whose deltas change, where a run of equal deltas would be a wheel —
and scrolls by exactly the distance travelled. Without `MS` the time follows the
distance, about 1250 pixels a second, so 120 pixels take 0.4 s and 700 take
0.9 s; `MS` is capped at 3 seconds. The command replies when the swipe is over,
and scrolls wherever the mouse is: send `glide` and `settle` first to aim it,
which is what `actions::scroll` does when it is given a point. Its `amount` is in
steps of 40 pixels.

## Showing that the app is driven

A driven app says so, and it can tell who is driving it. Every client says hello
with a name and, optionally, what its connection is for:
`hello TOKEN [NAME] [role=launcher|agent]`.

- An **agent** (the default, so clients that predate roles keep working) is
  remote-controlling the app: `Client::connect_as(&endpoint, "mcp:codex")`.
- The **launcher** is the tool that started the app and holds a connection for the
  whole run, as `aimer run` does: `Client::connect_with(&endpoint, "aimer-cli",
  Role::Launcher)`. Its presence says the app *can* be driven, not that anyone is.

A client with no name is called `driver`; a name is one to 32 characters from
letters, digits and `. _ : / @ + -`. The server reports every connection with its
role (`Server::start_with_presence` gets `&[Attached]`), and the app keeps two
lists in `aimer_platform::attachment`: `attached()` for everyone connected and
`agents()` for the ones controlling it.

A debug build shows a badge while an agent is connected, and only then: an indigo
pill with a white `square-mouse-pointer` icon in the window's top-right corner,
which fades in when the first agent connects and out when the last one goes. It
does not follow activity — an agent that is idle between commands is still
remote — and it does not show for the launcher alone.

How the CLI knows an agent is there: its MCP server holds a *lease* on the run
session from its first tool that acts on the app until it exits, and the session
gives each lease a connection of its own to the app in the agent role, so a killed
agent is noticed when its lease connection closes. A one-shot `aimer input` or
`aimer screenshot`, which leaves no process behind, counts as an agent for four
seconds after it.

An injected event moves no real cursor and touches no real screen, so each
pointer or touch command also tells the app where it landed. The overlay draws
the mouse as the cursor icon in the repository's `.assets` folder
(`cursor-alt-svgrepo-com.svg`, in its own colours; its size and opacity are
constants in `aimer_quiver`'s `attach_overlay.rs`) with its tip on the injected position, a finger as a
ring, and a press as a ripple. A mouse that has rested for 0.7 s idles: it sways
a few degrees either side, pivoting on its tip, at about 30 frames a second
while it lasts. The badge and the pointers fade in over about 0.3 s when a client attaches and fade out when the last one disconnects (a driver that returns mid-fade fades back from where it was); a pointer that leaves goes at once. Both live in `aimer_quiver`'s debug builds only; a
release build has no overlay and starts no server.

## Typing

`text` is one edit: the whole string lands at once, as an input method's commit
does. `typing MS|- TEXT` is a hand at the keys: each character is its own commit,
about 70 ms apart with an uneven rhythm, a breath after a word and a longer one
after a sentence, so a recording of a driven app looks typed. `MS` is about how
long all of it takes (1 to 3000), or `-` to let the length decide; a text too long
to type at that pace in three seconds is typed faster, every character still sent.
The rhythm is a fixed function of the position, not random, so the same text types
the same way every time. A character that takes several code points — a letter
with an accent, a joined emoji, a flag — is one keystroke. The command holds its
connection until the last character is in. `actions::typing` builds it, and is what
`aimer input type` and the MCP `type` tool send; `actions::type_text` is the
instant one.

## Screenshots

`screenshot` answers with a picture instead of `ok`: the line
`ok image MIME LENGTH WIDTH HEIGHT SCALE`, then exactly `LENGTH` bytes. `SCALE` is
how many picture pixels make a logical pixel, so a point in the picture is found
in window coordinates by dividing by it. By default the picture is the window's
*logical* size, so what an agent sees is what it can click; `width=0` keeps the
real pixels and `width=N` caps the width. `jpeg=Q` (1 to 100) gives a much smaller
JPEG instead of a lossless PNG. `Client::screenshot` returns the bytes.

The picture is the window's last drawn frame, copied on the renderer's own queue
behind the drawing and encoded off the UI thread, so asking for one does not delay
showing one. An app that is idle is woken to draw it, and that frame is built from
the whole tree, not the slice that changed. The overlay (badge and pointer) is in
the picture. Only the Metal renderer can copy a frame so far: the others answer
`err` at once, naming the renderer, instead of waiting.

## Security

Injected input is full control of the app, so the server is **opt-in** and
guarded twice:

- it binds the loopback interface unless `Config::bind` says otherwise;
- a client must present the endpoint's token before anything else is read. The
  random token is 128 bits taken from the standard library's OS-seeded hash
  keys; supply your own with `Config::token` where you have stronger material.

Other local processes can still reach a loopback port — on a shared machine and
on Android — which is why the token exists. Keep the endpoint file readable by
the current user only, and do not ship a build that starts the server.

**Binding beyond loopback** (`0.0.0.0`, `::`, or a network address) is for a
phone, whose loopback the machine driving it cannot reach. It makes the app
reachable by every device on that network, and the token is then the only thing
between them and full control of it. The server logs a warning when it does.
Bind the narrowest address that works, prefer a USB or tunnel interface over a
shared Wi-Fi, and never enable it in a release build.

## Platforms

Windows, macOS, Linux, Android and iOS use the TCP transport above. On
`wasm32` a browser page has no listening socket another process can reach, so
only `Command` and its wire format are available there; no transport is
provided yet.

## Scope

Every event that comes from outside the process has a command: mouse, touch,
scroll (pixel and line, with phases), pinch, rotate, pan, smart zoom, keys, text
and composition, the system's back, links, menu shortcuts, file drags and
quitting. Pointer and scroll commands carry modifiers, so `ctrl`-click and
`ctrl`-wheel zoom work.

What is not a command is not input:

- focus changes, `CharInput`/`TextInput` and the in-app `DragOver`/`DragLeave`/
  `DragDrop` events are produced by the framework in response to input. `text`
  reaches the focused field as `CharInput` for one character and `TextInput` for
  more, and a `move` while `file-hover` is in flight reports the drag to the
  widgets under it;
- `TextEditingDelta` mirrors a revisioned native editor and cannot be written
  without that editor's state;
- `ScreenChanged`, `Accessibility` and the frame, render and window wake-ups are
  the platform's own bookkeeping.

A `touch` reaches the same code as a finger on the screen, so it works on the
desktop too, where it marks the pointer as touch until the next mouse event.
