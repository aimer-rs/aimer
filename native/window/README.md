# aimer_window

Native windows for Aimer: open a window, steer it once it's open, and decide
whether closing windows ends the application.

An application starts with one window, described by a [`WindowAttr`] given to
`AimerApp::window`. Everything after that goes through this crate.

- **[`Window`]** — a widget that declares a window as part of a tree. The window
  is on screen while the widget is mounted and `open` is `true`, follows the
  widget's title and size, and closes when the widget goes away.
- **[`NewWindow`]** — a builder that opens a window from a callback and returns
  its [`WindowId`].
- **[`WindowId`]** — a `Copy` handle that focuses, retitles, resizes, minimizes,
  maximizes or closes a window.
- **[`WindowAttr`]** — the properties of a window: title, size and limits,
  decorations, transparency, visibility. It does not expose `winit`, so
  applications are not tied to the windowing backend.
- **[`ExitPolicy`]** — whether closing windows ends the application. [`quit`]
  ends it whatever the policy says.

Availability: macOS, Windows, Linux

## Opening a window

Use [`Window`] when a window's existence is part of your state, and
[`NewWindow`] when a window is the answer to an event, such as a menu item.
Both produce the same window.

```no_run
use aimer_container::SizedBox;
use aimer_window::NewWindow;

let inspector = NewWindow::new()
    .title("Inspector")
    .size(360, 480)
    .builder(|_ctx| SizedBox::new())
    .open();

inspector.set_title("Inspector - 3 items");
```

To tie a window to state, put a [`Window`] in a widget's `build` and feed its
`open` from that state. The user can close a window on their own, so clear the
state in [`on_close`](Window::on_close). A window the user closed stays closed
while `open` stays `true`, and opens again after `open` has been `false`.

```no_run
use aimer_container::SizedBox;
use aimer_widget::{AnyElement, State, StateView, StatefulElement, Widget, base::BuildContext};
use aimer_window::Window;

struct Page;

impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // The state is named here, in a body, so it can stay private.
        StatefulElement::from_widget::<Self, PageState>(self, ctx, "Page", None)
    }
}

struct PageState {
    settings_open: bool,
}

impl State<Page> for PageState {
    fn create(_widget: Page) -> Self {
        PageState { settings_open: true }
    }

    fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        let updater = this.updater();
        Window::new()
            .title("Settings")
            .size(480, 320)
            .open(this.settings_open)
            .on_close(move || updater.set_state(|page| page.settings_open = false))
            .builder(|_ctx| SizedBox::new())
    }
}
```

## Behavior

- **Requests, not actions.** Opening, resizing and closing are queued on the UI
  thread and carried out by the event loop the next time it runs. A call returns
  before the window has changed, so [`is_open`](WindowId::is_open) is `false`
  right after [`open`](NewWindow::open). Make these calls on the UI thread;
  only [`quit`] may be called from any thread.
- **A tree of its own.** Each window builds its own widget tree on the
  application's runtime and tasks. It sees the providers in scope where it was
  opened: always for a [`Window`], and for a [`NewWindow`] when
  [`inherit`](NewWindow::inherit) is given a `BuildContext`. Providers are
  shared, not copied, so windows read and update the same state.
  [`isolated`](Window::isolated) starts a [`Window`] with none.
- **Stale ids are harmless.** An id never names a second window. Anything done
  through an id whose window is gone is ignored.

## Frosted windows

[`WindowBackdrop`] turns the window it is in into frosted glass: the system blurs
the desktop and the windows behind it and shows that through everything the
content draws with some transparency. Opaque content stays opaque, so put it
under a translucent background, or none. The blur is drawn by the system, not by
Aimer, and the effect is removed again when the widget leaves the tree.

```no_run
use aimer_container::SizedBox;
use aimer_window::{WindowBackdrop, WindowMaterial};

let frosted = WindowBackdrop::new()
    .material(WindowMaterial::Sidebar)
    .child(SizedBox::new());
```

A [`WindowMaterial`] names the look in macOS's vocabulary. Windows has fewer, so
the sidebar, titlebar and content materials are Mica, menu, popover and HUD are
Acrylic, and `Under` is Mica Alt.

`Glass` and `ClearGlass` are the refractive glass of macOS 26 and later (Liquid
Glass), drawn by an `NSGlassEffectView`. That view lenses and lights what is
behind it like a floating surface, so it fits a small panel rather than a
window-wide backdrop; for a sidebar like the Finder's use `Sidebar`. A system
without it falls back to the sidebar blur, and Windows draws Acrylic.

```no_run
use aimer_container::SizedBox;
use aimer_window::{WindowBackdrop, WindowMaterial};

let glass = WindowBackdrop::new()
    .material(WindowMaterial::Glass)
    .child(SizedBox::new());
```

## Exit policy

| [`ExitPolicy`]                   | The application ends when                        |
|----------------------------------|--------------------------------------------------|
| `LastWindowClosed` (the default) | its last window has closed                       |
| `MainWindowClosed`               | the first window closes; the rest close with it  |
| `Never`                          | [`quit`] is called or the process is told to stop |

## Platforms

| Platform              | Windows                                           |
|-----------------------|---------------------------------------------------|
| macOS, Windows, Linux | supported                                         |
| iOS, Android, web     | one surface only; requests for more are never answered |

| Platform | [`WindowBackdrop`]                                              |
|----------|-----------------------------------------------------------------|
| macOS    | `NSVisualEffectView`, or `NSGlassEffectView` for glass (26+), behind the content; run and checked |
| Windows  | DWM system backdrop (Windows 11 22H2+); compile-checked only     |
| others   | inert; the content is shown as it is                             |

## For the event loop

The `host` module, hidden from the generated documentation, is what
`aimer_quiver`'s desktop event loop uses to drain requests and to report
windows opening and closing. Applications do not use it.
