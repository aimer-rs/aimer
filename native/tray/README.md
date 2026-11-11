# aimer_tray

A system tray icon for Aimer: a picture and tooltip in the menu bar (macOS), the
notification area (Windows) or the status notifier host (Linux), with a menu and
click events.

- **[`Tray`]** describes the icon and [`Tray::build`] shows it.
- **[`TrayHandle`]** is the live icon; change its picture, tooltip or menu, and
  drop it to remove the icon.
- **[`TrayIcon`]** is an RGBA picture.
- **[`TrayMenu`]** lists items, check items, separators and submenus; each item
  is named by a [`TrayItemId`] you choose.
- **[`TrayEvent`]** reports clicks and chosen items.

```no_run
use aimer_tray::{Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};

const QUIT: TrayItemId = TrayItemId(1);

let icon = TrayIcon::from_rgba(2, 2, [40, 120, 255, 255].repeat(4)).unwrap();
let _tray = Tray::new()
    .icon(icon)
    .tooltip("My app")
    .menu(TrayMenu::new().item(QUIT, "Quit"))
    .on_event(|event| {
        if event == TrayEvent::MenuItem(QUIT) {
            aimer_tray_example_quit();
        }
    })
    .build();
# fn aimer_tray_example_quit() {}
```

Availability: macOS, Windows, Linux

## Platforms

| Platform | Mechanism                                              |
| -------- | ------------------------------------------------------ |
| macOS    | `NSStatusItem` in the system status bar                |
| Windows  | `Shell_NotifyIconW` with a hidden message window       |
| Linux    | `StatusNotifierItem` and `com.canonical.dbusmenu`      |
| others   | [`TrayError::Unsupported`]                             |

Events arrive on the main thread on macOS and on a thread of the tray's own on
Windows and Linux; see [`TrayHandler`].
