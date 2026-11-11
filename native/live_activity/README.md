# aimer_live_activity

Live activities for Aimer: a short, glanceable status the operating system keeps
outside the application window while something is going on, such as a ride, a
download or a timer.

- **[`LiveActivity`]** describes one activity: title, body, a short status, a
  progress bar, milestones along it, an icon, and optional custom state. It is
  named by a [`LiveActivityId`] you choose; showing the same id again updates it.
- **[`LiveActivities`]** is the connection to the platform. [`LiveActivities::show`]
  starts or updates an activity and [`LiveActivities::end`] ends it.
- **[`Presentation`]** tells where the activities went: the platform's own
  surface, or the tray.

```no_run
use aimer_live_activity::{LiveActivities, LiveActivity, LiveActivityId, Milestone};

let activities = LiveActivities::new().build().unwrap();
let id = LiveActivityId(1);

activities
    .show(
        &LiveActivity::new(id)
            .title("Ride to the airport")
            .body("Driver is 4 minutes away")
            .status("4 min")
            .progress(0.4)
            .milestone(Milestone::new("Picked up", 0.5)),
    )
    .unwrap();
activities.end(id).unwrap();
```

Notifications that are not tied to something in progress are a different thing:
see `aimer_notification`.

## Keeping a clock moving

An activity only changes when the application shows it again, and an
application in the background cannot: iOS suspends it, so text pushed once a
second freezes on the Dynamic Island and the lock screen as soon as the app is
put away. Anything that must keep counting belongs in
[`LiveActivity::countdown`], a deadline the *system* ticks by itself:

```
use std::time::{Duration, SystemTime};
use aimer_live_activity::{LiveActivity, LiveActivityId};

let ends = SystemTime::now() + Duration::from_secs(90);
let timer = LiveActivity::new(LiveActivityId(1))
    .title("Tea")
    .countdown(ends, Duration::from_secs(180));
```

- **iOS:** the template widget draws the clock with `Text(timerInterval:)` and
  the bar with `ProgressView(timerInterval:)`, both drawn by the system. A custom
  widget should do the same, with `AimerLiveActivityAttributes.interval`.
- **Android:** the notification gets a count-down chronometer, which the chip of
  a promoted notification shows too. `ProgressStyle` cannot move on its own, so
  the bar is only as current as the last update.
- **The tray** ignores it.

Show the activity again with a new deadline to move it, and without a
countdown once it is over.

## Views for the Dynamic Island and the lock screen

The SwiftUI of a live activity runs in the widget extension, a separate process
with no Aimer runtime, so an Aimer `Widget` cannot be drawn there.
[`UIKitWidget`] is the trait for what can: it is deliberately *not* a `Widget`
(no `to_element`), so the compiler refuses to place one in an Aimer tree, and it
only describes a small tree of views as data. `LiveActivity::view` puts such a
tree in a [`Region`] of the island or the lock screen, and a generic SwiftUI
interpreter (`AimerNodeView`) in the extension draws it. A region with no view
keeps the template's content.

```
use aimer_live_activity::{
    Bar, Clock, Gap, Label, LiveActivity, LiveActivityId, Region, Row, Symbol, TextStyle, Tint,
};

let timer = LiveActivity::new(LiveActivityId(1))
    .title("Tea")
    .view(Region::CompactLeading, Symbol::new("cup.and.saucer.fill"))
    .view(Region::CompactTrailing, Clock::new().tint(Tint::Orange))
    .view(
        Region::LockScreen,
        Row::new()
            .child(Label::new("Tea").style(TextStyle::Headline))
            .child(Gap::new())
            .child(Clock::new()),
    )
    .view(Region::ExpandedBottom, Bar::new());
```

The built-in widgets are [`Label`], [`Symbol`] (an SF Symbol), [`Bar`], [`Clock`],
[`Row`], [`Column`] and [`Gap`]. `Clock` and `Bar` follow the activity's
[countdown](LiveActivity::countdown), so the system keeps them moving while the
app is suspended. Implement `UIKitWidget` on your own type to compose them under
one name.

- **Limits:** the whole payload of an activity, views included, may not exceed
  about 4 KB, and compact and minimal regions are tiny. Views cannot hold
  buttons: those need an App Intent compiled into the extension.
- **Beyond the built-ins:** edit the SwiftUI in `AimerLiveActivityWidget.swift`
  and read `context.state.state` or `context.state.views` yourself.
- **Elsewhere:** Android's Live Update is a fixed template and the tray is one
  line of text, so both ignore views.

## Platforms

| Platform        | Mechanism                                                        |
| --------------- | ---------------------------------------------------------------- |
| Android 16+     | Live Update: `Notification.ProgressStyle`, promoted ongoing      |
| Android 7 to 15 | an ongoing notification with a progress bar                      |
| iOS 16.2+       | ActivityKit Live Activity, through a Swift bridge                |
| macOS           | none: the macOS SDK marks ActivityKit unavailable; opt in to a tray icon with [`LiveActivitiesBuilder::tray_icon`] |
| Windows, Linux  | none; opt in to a tray icon with [`LiveActivitiesBuilder::tray_icon`] |
| others          | [`LiveActivityError::Unsupported`]                               |

### Who draws the layout

The layout is not Aimer's on any platform.

- **Android** owns it. A Live Update is a fixed system template, so the common
  fields of [`LiveActivity`] are everything it can show.
- **iOS** draws it with SwiftUI inside a *widget extension*, a
  separate process that cannot host an Aimer widget tree. The extension of this
  crate (`swift/AimerLiveActivityWidget.swift`) is a `WidgetBundle` with an
  `ActivityConfiguration` and nothing else, so it cannot turn into a home screen
  widget, and its generic template draws the common fields. An application that
  wants its own layout edits that SwiftUI and reads [`LiveActivity::state`], the
  free-form key/value pairs only that view sees.
- **The tray** shows one tooltip line per activity: title, body, status and
  percent.

## Platform setup

### Android

- Declare the permissions in `AndroidManifest.xml`. The first is needed on
  Android 13 and later, or the system drops every notification; the second lets
  Android 16 promote the notification:

  ```xml
  <uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>
  <uses-permission android:name="android.permission.POST_PROMOTED_NOTIFICATIONS"/>
  ```

- Ask for the notification permission with `aimer_notification` before the
  first [`LiveActivities::show`].
- Promotion to the status bar chip and the lock screen is up to the system and
  to the user. When it does not happen the activity is an ordinary ongoing
  notification.
- [`LiveActivity::icon`] names a drawable resource of the application. A
  missing one falls back to the application icon.
- A tap opens the launcher activity. No event is reported.
- [`LiveActivity::status`] is shown in the chip; about seven characters fit.

### iOS

ActivityKit has no Objective-C surface, so the application ships three Swift
files from this crate's `swift/` directory. They are wrapped in `#if os(iOS)`,
so a macOS target can hold them and compiles them to nothing.

| File                                | Target                        | Purpose                                            |
| ----------------------------------- | ----------------------------- | -------------------------------------------------- |
| `AimerLiveActivityAttributes.swift` | app **and** extension         | the `ActivityAttributes` both processes agree on   |
| `AimerLiveActivityBridge.swift`     | app                           | exports the `@_cdecl` functions Rust looks up      |
| `AimerLiveActivityWidget.swift`     | widget extension              | the live-activity-only `WidgetBundle` and its SwiftUI |

Then:

1. Add a *Widget Extension* target to the Xcode project, with *Include Live
   Activity* checked, and give it the two files above. Delete the home screen
   widget the template generates.
2. Add `NSSupportsLiveActivities = YES` to the **application's** `Info.plist`.
   Add `NSSupportsLiveActivitiesFrequentUpdates = YES` as well if you update
   more than a few times an hour.
3. Link `ActivityKit` in the application and `WidgetKit` and `SwiftUI` in the
   extension, and add the application-side `-framework ActivityKit` to the
   Rust link flags next to `-framework UserNotifications`.
4. Call [`LiveActivities::show`] from the foreground; the system refuses to
   *start* an activity from the background.

The bridge is found with `dlsym`, so an application that does not ship it still
links and runs: [`LiveActivities::new`]`().build()` reports
[`LiveActivityError::Unsupported`], or falls back to the tray when an icon was
given. Users can turn live activities off in *Settings*, which shows up as
[`LiveActivityError::Disabled`].

[`LiveActivity::icon`] names an SF Symbol. Remote updates through push are not
covered here: the push token of an activity belongs to the Swift bridge.

### macOS, Windows and Linux

None has a live activity surface; ActivityKit is unavailable in the macOS SDK.
Offer an icon and the activities show in the menu bar or notification area
instead:

```no_run
use aimer_live_activity::{LiveActivities, Presentation};
use aimer_tray::TrayIcon;

let icon = TrayIcon::from_rgba(2, 2, [40, 120, 255, 255].repeat(4)).unwrap();
let activities = LiveActivities::new().tray_icon(icon).build().unwrap();
assert_eq!(activities.presentation(), Presentation::Tray);
```

The icon appears with the first activity and goes away with the last. On macOS
call `build` on the main thread; Linux needs a `StatusNotifierWatcher`, as
`aimer_tray` does.
