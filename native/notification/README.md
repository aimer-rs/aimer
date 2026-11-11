# aimer_notification

System notifications for Aimer: a title and a message the operating system shows
outside the application window, with buttons, delayed delivery, the permission
prompt, click events and push registration.

- **[`Notification`]** describes one message: title, subtitle, body, sound, an
  optional delay and buttons. It is named by a [`NotificationId`] you choose;
  showing the same id again replaces the earlier one.
- **[`Notifier`]** is the connection to the platform. [`Notifier::show`],
  [`Notifier::close`], [`Notifier::permission`], [`Notifier::request_permission`]
  and [`Notifier::register_push`] live here.
- **[`NotificationEvent`]** reports clicks, chosen buttons, dismissals and push
  results.
- **[`push`]** is where launcher code reports a device token or a push message.

```no_run
use aimer_notification::{Notification, NotificationEvent, NotificationId, Notifier};

let notifier = Notifier::new()
    .on_event(|event| {
        if let NotificationEvent::Clicked(_id) = event {
            // Hand over to the event loop here.
        }
    })
    .build()
    .unwrap();

notifier.request_permission(Box::new(|_| {}));
notifier
    .show(&Notification::new(NotificationId(1)).title("Hello").body("From Aimer"))
    .unwrap();
```

In-app messages that never leave the window are a different thing: see the
toasts in `aimer_feedback`.

Availability: macOS 10.14+, iOS 10+, Android 7+, Windows, Linux

## Platforms

| Platform | Mechanism                                                     |
| -------- | ------------------------------------------------------------- |
| macOS    | `UNUserNotificationCenter` (the app must run from a bundle)   |
| iOS      | `UNUserNotificationCenter`                                    |
| Windows  | tray balloon through `Shell_NotifyIconW`                      |
| Linux    | `org.freedesktop.Notifications` over D-Bus                    |
| Android  | `NotificationManager` over JNI                                |
| others   | [`NotificationError::Unsupported`]                            |

Events arrive on a thread the platform chooses; see [`NotificationHandler`].

## Platform setup

Each platform needs something from the application that the library cannot do
for it.

### Android

Declare the runtime permission in `AndroidManifest.xml`. Without it, Android 13
(API 33) and later never show the permission dialog and silently drop every
notification:

```xml
<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>
```

- The dialog needs the running `Activity`. `aimer_quiver` reports it at start-up
  through `set_android_activity`; an application with its own entry point must
  call that function itself. Until then [`Notifier::request_permission`] only
  reports the current state.
- [`Notifier::request_permission`] answers before the user does, because the
  result arrives in `Activity.onRequestPermissionsResult`, a Java callback. Read
  the real answer later with [`Notifier::permission`].
- Channels `aimer_default` and `aimer_silent` are created for you (API 26+).
- A small icon is taken from the application's icon; set `android:icon` on the
  `<application>` element, or the system's default is used.
- Click and button events are not delivered (they need Java glue). A tap opens
  the launcher activity.
- Push: add Firebase Cloud Messaging (`google-services.json`, the Google
  services Gradle plugin and a `FirebaseMessagingService`) and forward the token
  and messages with [`push::report_token`] and [`push::report_message`].
  [`Notifier::register_push`] returns [`NotificationError::Unsupported`] here.

### iOS

- Local notifications need no entitlement. Ask with
  [`Notifier::request_permission`] before the first [`Notifier::show`]; until
  the user answers, nothing is shown.
- Push needs the **Push Notifications** capability (the `aps-environment`
  entitlement) and an App ID with push enabled in the Apple Developer account.
  Forward the `UIApplicationDelegate` callbacks
  `application(_:didRegisterForRemoteNotificationsWithDeviceToken:)`,
  `application(_:didFailToRegisterForRemoteNotificationsWithError:)` and
  `application(_:didReceiveRemoteNotification:fetchCompletionHandler:)` to
  [`push::report_token`] (with [`push::hex_token`]), [`push::report_failure`] and
  [`push::report_message`]. Remote messages in the background also need the
  *Remote notifications* background mode.
- The link flags must include `-framework UserNotifications`; they are written
  by the `aimer` build tool.

### macOS

- The application must run from a signed `.app` bundle with a
  `CFBundleIdentifier`. A bare executable (`cargo run`) gets
  [`NotificationError::Platform`] instead of a notification, because the system
  refuses it.
- The first [`Notifier::request_permission`] shows the system prompt. Afterwards
  the user changes the choice in *System Settings > Notifications*.
- Push needs the *Push Notifications* capability and a provisioning profile, and
  reports its token through [`push`] like iOS.

### Windows

- Nothing to declare. The notification is a balloon of the notification area, so
  the user must not have turned notifications or *Focus assist* off for it.
- Balloons have no buttons; [`NotificationAction`]s are ignored.

### Linux

- A notification daemon that implements `org.freedesktop.Notifications` must be
  running on the session bus (GNOME, KDE, `dunst`, `mako` and the like do). With
  none, building the [`Notifier`] fails with [`NotificationError::Platform`].
- Buttons and click events need a daemon that advertises the `actions`
  capability.
- There is no permission concept: [`Notifier::permission`] always reports
  [`Permission::Granted`].

## Push

Push is client side only. [`Notifier::register_push`] asks the platform for a
token; the launcher code that owns the platform callback (`UIApplicationDelegate`
on iOS, `FirebaseMessagingService` on Android) reports it through [`push`], and
the application's handler receives [`NotificationEvent::PushToken`]. Uploading
the token and sending messages is the application's server's job.
