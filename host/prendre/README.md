# aimer_prendre

Sends synthetic input to a running Aimer application from another process.

A driver — a test script, an automation tool, a developer at a terminal —
connects to the app, sends pointer, scroll, key and text events, and the app
handles them exactly like real input: they enter the event loop as the same
`AimerNativePlatformEvent::NativeInput` and `InsertText` the platform bridges
send, so nothing downstream can tell the difference.

```text
driver process                          app process
Client ── hello TOKEN ──► Server ──► EVENT_PROXY ──► event loop
       ◄───── ok ───────        (loopback TCP)
```

## In the app

Start the server deliberately, once the event loop is running:

```rust,no_run
let server = aimer_prendre::Server::start(Default::default())?;
server.endpoint().save("aimer-prendre.endpoint")?;
# Ok::<(), std::io::Error>(())
```

`Server::start` listens on `127.0.0.1` and picks a free port. Keep the returned
`Server` alive for as long as input should be accepted; dropping it stops
listening and closes its connections. A command that arrives before the event
loop exists is refused with `app unavailable`, so the driver can retry.

## From the driver

```rust,no_run
use aimer_prendre::{Client, Command, Endpoint};

let mut app = Client::connect(&Endpoint::load("aimer-prendre.endpoint")?)?;
for line in ["move 120 48", "down 120 48", "up 120 48"] {
    app.send(&line.parse::<Command>().expect("valid command"))?;
}
app.send(&Command::Text("hello".into()))?;
# Ok::<(), std::io::Error>(())
```

`send` returns once the app's event loop has been handed the event, and returns
an error when the server refused it.

## Wire format

Any language can drive an app: the protocol is one text line per message over
TCP, each answered with one line, `ok` or `err REASON`. The first line must be
`hello TOKEN`.

| Line                               | Meaning                                              |
|------------------------------------|------------------------------------------------------|
| `move X Y`                         | The mouse moved.                                     |
| `down X Y [BUTTON]`, `up X Y [BUTTON]` | A button went down / came up. Default `primary`. |
| `leave`                            | The mouse left the window.                           |
| `scroll DX DY`                     | A pixel-precise scroll.                              |
| `key press\|release\|repeat MODS KEY` | A named key changed state.                        |
| `text TEXT`                        | Text committed as one edit (like an IME commit).     |

Positions and deltas are logical pixels in window coordinates and must be finite.
`BUTTON` is `primary`, `secondary`, `middle` or `other:N`. `MODS` is `-` or
`+`-joined `ctrl`, `shift`, `alt`, `meta`. `KEY` is `backspace`, `delete`, `up`,
`down`, `left`, `right`, `pageup`, `pagedown`, `home`, `end`, `enter`, `escape`,
`tab` or `other:NAME`. `TEXT` and `NAME` run to the end of the line; write `\`,
newline and carriage return as `\\`, `\n` and `\r`. Lines are limited to 64 KiB.

```text
$ { echo "hello $(cut -d' ' -f2 aimer-prendre.endpoint)"; echo "key press ctrl a"; } | nc 127.0.0.1 PORT
```

## Security

Injected input is full control of the app, so the server is **opt-in** and
guarded twice:

- it binds the loopback interface only, never a network address;
- a client must present the endpoint's token before anything else is read. The
  random token is 128 bits taken from the standard library's OS-seeded hash
  keys; supply your own with `Config::token` where you have stronger material.

Other local processes can still reach a loopback port — on a shared machine and
on Android — which is why the token exists. Keep the endpoint file readable by
the current user only, and do not ship a build that starts the server.

## Platforms

Windows, macOS, Linux, Android and iOS use the TCP transport above. On
`wasm32` a browser page has no listening socket another process can reach, so
only `Command` and its wire format are available there; no transport is
provided yet.

## Scope

Only mouse pointer, scroll, named-key and text input are supported: that is what
the application loop accepts through `NativeInput` on desktop. Touch contacts
are not injected, because the loop ignores non-mouse pointers on that path.
