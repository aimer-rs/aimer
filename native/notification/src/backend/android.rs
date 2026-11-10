//! Android: `android.app.NotificationManager` over JNI.
//!
//! No Java or Kotlin class of our own ships with the library, which decides
//! what this backend can and cannot do:
//!
//! | Feature                    | Support                                            |
//! |----------------------------|----------------------------------------------------|
//! | Title and body             | `Notification.Builder` (subtitle is joined to body) |
//! | Replace by id              | `notify(int, Notification)` with a stable `i32`     |
//! | Close                      | `NotificationManager.cancel(int)`                   |
//! | Sound off                  | `setSound(null)` below API 26, a silent channel above |
//! | Delay                      | in-process [`Scheduler`], lost if the process dies  |
//! | Tapping the notification   | launches the app; **no** [`NotificationEvent`] is sent |
//! | Action buttons             | **not shown and never delivered**                   |
//! | Permission query           | `areNotificationsEnabled` + `POST_NOTIFICATIONS`    |
//! | Permission prompt          | `Activity.requestPermissions`, answer not observable |
//! | Push token                 | [`NotificationError::Unsupported`]                  |
//!
//! Buttons and click events need a `PendingIntent` that targets a
//! `BroadcastReceiver` (or an activity that forwards its intent), and both are
//! Java classes that must be declared in the manifest; Rust cannot define them.
//! A tap therefore only brings the application to the front through the launch
//! intent of the package. Applications that need clicks or buttons should
//! handle them in their own glue.
//!
//! Push is the same story: the FCM token is delivered to a
//! `FirebaseMessagingService`, which the application's glue forwards with
//! [`crate::push::report_token`].
//!
//! The app must declare `<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>`
//! to be allowed to notify on Android 13 and later.
//!
//! [`NotificationEvent`]: crate::NotificationEvent

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

use jni::objects::{Global, JObject, JObjectArray, JString, JValue};
use jni::{jni_sig, jni_str};

use super::Backend;
use crate::scheduler::Scheduler;
use crate::{
    Notification, NotificationError, NotificationHandler, NotificationId, Permission,
    PermissionCallback,
};

/// First release with notification channels.
const CHANNEL_API: i32 = 26;
/// First release with `PendingIntent.FLAG_IMMUTABLE`.
const IMMUTABLE_PENDING_INTENT_API: i32 = 23;
/// First release with `NotificationManager.areNotificationsEnabled`.
const ARE_ENABLED_API: i32 = 24;
/// First release where posting needs the `POST_NOTIFICATIONS` runtime permission.
const RUNTIME_PERMISSION_API: i32 = 33;

const CHANNEL_ID: &str = "aimer_default";
const CHANNEL_NAME: &str = "Notifications";
/// Same as [`CHANNEL_ID`] but without sound, for `Notification::sound(false)`
/// on releases where sound belongs to the channel, not the notification.
const SILENT_CHANNEL_ID: &str = "aimer_silent";
const SILENT_CHANNEL_NAME: &str = "Silent notifications";
/// `NotificationManager.IMPORTANCE_LOW`: shown, but makes no sound.
const IMPORTANCE_LOW: i32 = 2;
/// `NotificationManager.IMPORTANCE_DEFAULT`.
const IMPORTANCE_DEFAULT: i32 = 3;

/// `android.R.drawable.ic_dialog_info`, for an app that declares no icon.
const FALLBACK_ICON: i32 = 17301618;
const POST_NOTIFICATIONS: &str = "android.permission.POST_NOTIFICATIONS";
/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;
/// `PendingIntent.FLAG_UPDATE_CURRENT`.
const FLAG_UPDATE_CURRENT: i32 = 0x0800_0000;
/// `PendingIntent.FLAG_IMMUTABLE`.
const FLAG_IMMUTABLE: i32 = 0x0400_0000;
/// Request code of the `requestPermissions` call; nothing receives the answer.
const PERMISSION_REQUEST_CODE: i32 = 0x4149;

type Failure = String;

fn describe(error: jni::errors::Error) -> Failure {
    error.to_string()
}

fn platform(what: &str, failure: &str) -> NotificationError {
    NotificationError::Platform(format!("{what}: {failure}"))
}

/// Android keeps notification ids as `int`s; the low 31 bits of ours are used,
/// so the id is never negative and the same [`NotificationId`] always replaces
/// the same notification.
///
/// Two ids that differ only above bit 31 collide and replace each other.
fn stable_id(id: NotificationId) -> i32 {
    (id.0 & 0x7fff_ffff) as i32
}

/// Whether the running release has notification channels, which every
/// notification must then name.
fn uses_channels(sdk_int: i32) -> bool {
    sdk_int >= CHANNEL_API
}

/// The channel a notification is posted to, from its `sound` flag.
fn channel_for(sound: bool) -> &'static str {
    if sound { CHANNEL_ID } else { SILENT_CHANNEL_ID }
}

/// Whether sound is silenced on the notification itself, which is only
/// possible before channels took that over.
fn silences_on_notification(sdk_int: i32, sound: bool) -> bool {
    !sound && !uses_channels(sdk_int)
}

/// The `PendingIntent` flags: updating the intent always, immutable where the
/// release knows the flag (and requires it from API 31).
fn pending_intent_flags(sdk_int: i32) -> i32 {
    if sdk_int >= IMMUTABLE_PENDING_INTENT_API {
        FLAG_UPDATE_CURRENT | FLAG_IMMUTABLE
    } else {
        FLAG_UPDATE_CURRENT
    }
}

/// Whether the release gates notifications behind a runtime permission.
fn needs_runtime_permission(sdk_int: i32) -> bool {
    sdk_int >= RUNTIME_PERMISSION_API
}

/// Combines what Android reports into a [`Permission`].
///
/// `runtime_granted` is the `POST_NOTIFICATIONS` state (always `true` before
/// API 33) and `enabled` is `areNotificationsEnabled()`. Android cannot tell
/// "never asked" from "refused", so anything not granted is
/// [`Permission::Denied`].
fn resolve_permission(runtime_granted: bool, enabled: bool) -> Permission {
    if runtime_granted && enabled { Permission::Granted } else { Permission::Denied }
}

/// The application's `Activity`, as a JNI global reference, or null before the
/// host reports it.
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Tells the crate which `Activity` is running.
///
/// `ndk-context` holds the `Application`, which cannot show the system
/// permission dialog (`requestPermissions` is an `Activity` method), so the host
/// reports the activity itself. With winit this is
/// `AndroidApp::activity_as_ptr()`. Until it is called,
/// [`crate::Notifier::request_permission`] cannot prompt and only reports the
/// current state.
///
/// # Safety
///
/// `activity` must be null or a JNI global reference to the running `Activity`
/// that stays valid for the life of the process, as the reference
/// android-activity owns is.
pub unsafe fn set_android_activity(activity: *mut c_void) {
    ACTIVITY.store(activity, Ordering::Release);
}

/// Everything a post needs, shared with the scheduler thread.
///
/// The references are globals, so they are valid on any thread attached to the
/// JVM and for as long as this lives.
struct Android {
    context: Global<JObject<'static>>,
    manager: Global<JObject<'static>>,
    sdk_int: i32,
}

/// Attaches the thread to the JVM and runs `call`.
///
/// A Java exception left pending by a failed call would poison every later JNI
/// call on the thread, so it is printed (to logcat) and cleared before the error
/// returns.
fn with_env<T>(
    call: impl FnOnce(&mut jni::Env) -> Result<T, jni::errors::Error>,
) -> Result<T, Failure> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return Err("the Android runtime is not running".into());
    }
    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };
    vm.attach_current_thread(|env| {
        let outcome = call(env);
        if outcome.is_err() && env.exception_check() {
            env.exception_describe();
            env.exception_clear();
        }
        outcome
    })
    .map_err(describe)
}

impl Android {
    /// Looks up the context and the notification service, and declares the
    /// channels where the release has them.
    fn connect(env: &mut jni::Env) -> Result<Self, jni::errors::Error> {
        let context_ptr = ndk_context::android_context().context() as jni::sys::jobject;
        if context_ptr.is_null() {
            return Err(jni::errors::Error::NullPtr("application context"));
        }
        // SAFETY: the context is a global reference owned by the android-activity
        // runtime, not a local one, so it needs no frame of its own.
        let context = unsafe { JObject::from_raw(&*env, context_ptr) };

        let name = env.new_string("notification")?;
        let manager = env
            .call_method(
                &context,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&name)],
            )?
            .l()?;
        if manager.is_null() {
            return Err(jni::errors::Error::NullPtr("notification service"));
        }
        let sdk_int = env
            .get_static_field(
                jni_str!("android/os/Build$VERSION"),
                jni_str!("SDK_INT"),
                jni_sig!("I"),
            )?
            .i()?;
        let android = Self {
            context: env.new_global_ref(&context)?,
            manager: env.new_global_ref(&manager)?,
            sdk_int,
        };
        if uses_channels(sdk_int) {
            android.create_channel(env, CHANNEL_ID, CHANNEL_NAME, IMPORTANCE_DEFAULT)?;
            android.create_channel(env, SILENT_CHANNEL_ID, SILENT_CHANNEL_NAME, IMPORTANCE_LOW)?;
        }
        Ok(android)
    }

    /// Declares a channel; the system ignores a repeat of an existing one.
    fn create_channel(
        &self,
        env: &mut jni::Env,
        id: &str,
        name: &str,
        importance: i32,
    ) -> Result<(), jni::errors::Error> {
        let id = env.new_string(id)?;
        let name = env.new_string(name)?;
        let channel = env.new_object(
            jni_str!("android/app/NotificationChannel"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/CharSequence;I)V"),
            &[JValue::Object(&id), JValue::Object(&name), JValue::Int(importance)],
        )?;
        env.call_method(
            &self.manager,
            jni_str!("createNotificationChannel"),
            jni_sig!("(Landroid/app/NotificationChannel;)V"),
            &[JValue::Object(&channel)],
        )?;
        Ok(())
    }

    /// Builds `notification` and hands it to the system under its stable id.
    fn post(&self, env: &mut jni::Env, notification: &Notification) -> Result<(), jni::errors::Error> {
        let builder = if uses_channels(self.sdk_int) {
            let channel = env.new_string(channel_for(notification.sound))?;
            env.new_object(
                jni_str!("android/app/Notification$Builder"),
                jni_sig!("(Landroid/content/Context;Ljava/lang/String;)V"),
                &[JValue::Object(&self.context), JValue::Object(&channel)],
            )?
        } else {
            env.new_object(
                jni_str!("android/app/Notification$Builder"),
                jni_sig!("(Landroid/content/Context;)V"),
                &[JValue::Object(&self.context)],
            )?
        };

        let title = env.new_string(&notification.title)?;
        env.call_method(
            &builder,
            jni_str!("setContentTitle"),
            jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
            &[JValue::Object(&title)],
        )?;
        let text = env.new_string(notification.flat_body())?;
        env.call_method(
            &builder,
            jni_str!("setContentText"),
            jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
            &[JValue::Object(&text)],
        )?;
        let icon = self.small_icon(env)?;
        env.call_method(
            &builder,
            jni_str!("setSmallIcon"),
            jni_sig!("(I)Landroid/app/Notification$Builder;"),
            &[JValue::Int(icon)],
        )?;
        env.call_method(
            &builder,
            jni_str!("setAutoCancel"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        env.call_method(
            &builder,
            jni_str!("setOnlyAlertOnce"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        if silences_on_notification(self.sdk_int, notification.sound) {
            env.call_method(
                &builder,
                jni_str!("setSound"),
                jni_sig!("(Landroid/net/Uri;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&JObject::null())],
            )?;
        }
        if let Some(intent) = self.launch_intent(env)? {
            env.call_method(
                &builder,
                jni_str!("setContentIntent"),
                jni_sig!("(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&intent)],
            )?;
        }

        let built = env
            .call_method(
                &builder,
                jni_str!("build"),
                jni_sig!("()Landroid/app/Notification;"),
                &[],
            )?
            .l()?;
        env.call_method(
            &self.manager,
            jni_str!("notify"),
            jni_sig!("(ILandroid/app/Notification;)V"),
            &[JValue::Int(stable_id(notification.id)), JValue::Object(&built)],
        )?;
        Ok(())
    }

    /// The application's own icon, or the system's info icon when it has none.
    fn small_icon(&self, env: &mut jni::Env) -> Result<i32, jni::errors::Error> {
        let info = env
            .call_method(
                &self.context,
                jni_str!("getApplicationInfo"),
                jni_sig!("()Landroid/content/pm/ApplicationInfo;"),
                &[],
            )?
            .l()?;
        if info.is_null() {
            return Ok(FALLBACK_ICON);
        }
        let icon = env
            .get_field(&info, jni_str!("icon"), jni_sig!("I"))?
            .i()?;
        Ok(if icon == 0 { FALLBACK_ICON } else { icon })
    }

    /// A `PendingIntent` that opens the launcher activity of this package, or
    /// `None` when the package has none.
    fn launch_intent<'local>(
        &self,
        env: &mut jni::Env<'local>,
    ) -> Result<Option<JObject<'local>>, jni::errors::Error> {
        let package_manager = env
            .call_method(
                &self.context,
                jni_str!("getPackageManager"),
                jni_sig!("()Landroid/content/pm/PackageManager;"),
                &[],
            )?
            .l()?;
        let package = env
            .call_method(
                &self.context,
                jni_str!("getPackageName"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let intent = env
            .call_method(
                &package_manager,
                jni_str!("getLaunchIntentForPackage"),
                jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::Object(&package)],
            )?
            .l()?;
        if intent.is_null() {
            return Ok(None);
        }
        let pending = env
            .call_static_method(
                jni_str!("android/app/PendingIntent"),
                jni_str!("getActivity"),
                jni_sig!("(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;"),
                &[
                    JValue::Object(&self.context),
                    JValue::Int(0),
                    JValue::Object(&intent),
                    JValue::Int(pending_intent_flags(self.sdk_int)),
                ],
            )?
            .l()?;
        Ok(Some(pending))
    }

    fn cancel(&self, env: &mut jni::Env, id: NotificationId) -> Result<(), jni::errors::Error> {
        env.call_method(
            &self.manager,
            jni_str!("cancel"),
            jni_sig!("(I)V"),
            &[JValue::Int(stable_id(id))],
        )?;
        Ok(())
    }

    /// Reads the current state without prompting.
    fn read_permission(&self, env: &mut jni::Env) -> Result<Permission, jni::errors::Error> {
        let enabled = if self.sdk_int >= ARE_ENABLED_API {
            env.call_method(
                &self.manager,
                jni_str!("areNotificationsEnabled"),
                jni_sig!("()Z"),
                &[],
            )?
            .z()?
        } else {
            true
        };
        let runtime_granted = if needs_runtime_permission(self.sdk_int) {
            let name = env.new_string(POST_NOTIFICATIONS)?;
            let state = env
                .call_method(
                    &self.context,
                    jni_str!("checkSelfPermission"),
                    jni_sig!("(Ljava/lang/String;)I"),
                    &[JValue::Object(&name)],
                )?
                .i()?;
            state == PERMISSION_GRANTED
        } else {
            true
        };
        Ok(resolve_permission(runtime_granted, enabled))
    }

    /// Shows the system prompt through the `Activity` the host reported, or the
    /// context when that already is one. Returns whether a prompt was requested.
    fn prompt(&self, env: &mut jni::Env) -> Result<bool, jni::errors::Error> {
        let reported = ACTIVITY.load(Ordering::Acquire) as jni::sys::jobject;
        let activity = if !reported.is_null() {
            // SAFETY: `set_android_activity` requires a global reference that
            // outlives the process, so it needs no frame of its own.
            unsafe { JObject::from_raw(&*env, reported) }
        } else if env.is_instance_of(&self.context, jni_str!("android/app/Activity"))? {
            env.new_local_ref(&self.context)?
        } else {
            return Ok(false);
        };
        let name = env.new_string(POST_NOTIFICATIONS)?;
        let permissions = JObjectArray::<JString>::new(env, 1, JString::null())?;
        permissions.set_element(env, 0, &name)?;
        env.call_method(
            &activity,
            jni_str!("requestPermissions"),
            jni_sig!("([Ljava/lang/String;I)V"),
            &[JValue::Object(&permissions), JValue::Int(PERMISSION_REQUEST_CODE)],
        )?;
        Ok(true)
    }
}

struct AndroidBackend {
    android: Arc<Android>,
    scheduler: Scheduler,
}

impl Backend for AndroidBackend {
    fn show(&self, notification: &Notification) -> Result<(), NotificationError> {
        if let Some(delay) = notification.delay {
            // The scheduler hands the notification back with its delay cleared.
            self.scheduler.schedule(notification.clone(), delay);
            return Ok(());
        }
        with_env(|env| self.android.post(env, notification))
            .map_err(|failure| platform("Notification post", &failure))
    }

    fn close(&self, id: NotificationId) -> Result<(), NotificationError> {
        self.scheduler.cancel(id);
        with_env(|env| self.android.cancel(env, id))
            .map_err(|failure| platform("NotificationManager.cancel", &failure))
    }

    fn permission(&self, done: PermissionCallback) {
        done(self.current_permission());
    }

    /// Asks for `POST_NOTIFICATIONS` where the release needs it, then reports
    /// the state **at that moment**.
    ///
    /// The user's answer arrives in `Activity.onRequestPermissionsResult`, a
    /// Java callback this library cannot implement, so `done` is called before
    /// the user has responded: it carries the old state. Read the real answer
    /// with [`Backend::permission`] afterwards.
    fn request_permission(&self, done: PermissionCallback) {
        if needs_runtime_permission(self.android.sdk_int)
            && self.current_permission() != Permission::Granted
        {
            match with_env(|env| self.android.prompt(env)) {
                Ok(true) => {}
                Ok(false) => eprintln!(
                    "aimer_notification: cannot ask for POST_NOTIFICATIONS: no Activity \
                     was reported; call `set_android_activity` at start-up"
                ),
                Err(failure) => {
                    eprintln!("aimer_notification: Activity.requestPermissions failed: {failure}")
                }
            }
        }
        done(self.current_permission());
    }

    /// FCM delivers its token to a `FirebaseMessagingService`; the
    /// application's glue forwards it with [`crate::push::report_token`].
    fn register_push(&self) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }
}

impl AndroidBackend {
    fn current_permission(&self) -> Permission {
        with_env(|env| self.android.read_permission(env)).unwrap_or_else(|failure| {
            eprintln!("aimer_notification: reading the permission failed: {failure}");
            Permission::Denied
        })
    }
}

/// Connects to the system notification service and starts the delay scheduler.
///
/// Click and action events are never sent to `_handler`; see the module docs.
pub(super) fn create(_handler: NotificationHandler) -> Result<Box<dyn Backend>, NotificationError> {
    let android = Arc::new(
        with_env(Android::connect).map_err(|failure| platform("NotificationManager", &failure))?,
    );
    let fire = Arc::clone(&android);
    let scheduler = Scheduler::new(move |notification| {
        if let Err(failure) = with_env(|env| fire.post(env, &notification)) {
            eprintln!("aimer_notification: delayed notification failed: {failure}");
        }
    })
    .map_err(|error| platform("scheduler thread", &error.to_string()))?;
    Ok(Box::new(AndroidBackend { android, scheduler }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_keeps_the_low_31_bits() {
        assert_eq!(stable_id(NotificationId(0)), 0);
        assert_eq!(stable_id(NotificationId(42)), 42);
        assert_eq!(stable_id(NotificationId(0x7fff_ffff)), i32::MAX);
        assert_eq!(stable_id(NotificationId(0x8000_0000)), 0);
        assert_eq!(stable_id(NotificationId(u64::MAX)), i32::MAX);
    }

    #[test]
    fn id_is_never_negative_and_is_stable() {
        for raw in [1, 0xffff_ffff, 0x1_0000_0001, u64::MAX - 1, 1 << 63] {
            let id = stable_id(NotificationId(raw));
            assert!(id >= 0);
            assert_eq!(id, stable_id(NotificationId(raw)));
        }
        assert_eq!(stable_id(NotificationId(5)), stable_id(NotificationId(5 | (1 << 40))));
    }

    #[test]
    fn channels_start_at_api_26() {
        assert!(!uses_channels(25));
        assert!(uses_channels(26));
        assert!(uses_channels(34));
    }

    #[test]
    fn silent_notifications_pick_the_silent_channel() {
        assert_eq!(channel_for(true), CHANNEL_ID);
        assert_eq!(channel_for(false), SILENT_CHANNEL_ID);
    }

    #[test]
    fn sound_is_silenced_on_the_notification_only_before_channels() {
        assert!(silences_on_notification(25, false));
        assert!(!silences_on_notification(25, true));
        assert!(!silences_on_notification(26, false));
        assert!(!silences_on_notification(34, false));
    }

    #[test]
    fn pending_intents_are_immutable_from_api_23() {
        assert_eq!(pending_intent_flags(22), FLAG_UPDATE_CURRENT);
        assert_eq!(pending_intent_flags(23), FLAG_UPDATE_CURRENT | FLAG_IMMUTABLE);
        assert_eq!(pending_intent_flags(34), FLAG_UPDATE_CURRENT | FLAG_IMMUTABLE);
    }

    #[test]
    fn runtime_permission_starts_at_api_33() {
        assert!(!needs_runtime_permission(32));
        assert!(needs_runtime_permission(33));
    }

    #[test]
    fn permission_needs_both_the_grant_and_the_setting() {
        assert_eq!(resolve_permission(true, true), Permission::Granted);
        assert_eq!(resolve_permission(true, false), Permission::Denied);
        assert_eq!(resolve_permission(false, true), Permission::Denied);
        assert_eq!(resolve_permission(false, false), Permission::Denied);
    }
}
