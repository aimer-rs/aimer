//! Android: a Live Update, over JNI.
//!
//! A Live Update is an ongoing notification with a fixed layout that the system
//! may promote to the status bar chip, the lock screen and the always-on
//! display. Nothing about the layout can be customised: it takes a title, a
//! text, a short status, a progress bar made of segments and points, and an
//! icon. That is why [`LiveActivity`] stays this small.
//!
//! | Release          | What is posted                                              |
//! |------------------|-------------------------------------------------------------|
//! | Android 16+      | `Notification.ProgressStyle`, `setRequestPromotedOngoing`, a short critical text from the status |
//! | Android 7 to 15  | an ongoing notification with a plain progress bar, the status as sub text |
//!
//! Whether the system promotes the notification is its decision: the
//! application must declare `android.permission.POST_PROMOTED_NOTIFICATIONS`,
//! the user may turn promotion off, and the notification must be ongoing and
//! not from a minimal-importance channel. Either way the notification itself
//! is shown.
//!
//! Notifications are posted under a tag of their own, so ids never collide with
//! those of `aimer_notification`. Like that crate, no Java class ships with the
//! library: a tap opens the launcher activity and no event is reported. The
//! `POST_NOTIFICATIONS` permission on Android 13 and later is requested through
//! `aimer_notification`; without it the system drops the notification silently.

use jni::objects::{Global, JObject, JValue};
use jni::{jni_sig, jni_str};

use super::Backend;
use crate::activity::Progress;
use crate::{LiveActivity, LiveActivityError, LiveActivityId};

/// First release with notification channels.
const CHANNEL_API: i32 = 26;
/// First release with `PendingIntent.FLAG_IMMUTABLE`.
const IMMUTABLE_PENDING_INTENT_API: i32 = 23;
/// First release with `Notification.ProgressStyle` and promoted ongoing
/// notifications: Android 16.
const PROGRESS_STYLE_API: i32 = 36;

/// Keeps live activity ids apart from other notifications of the application.
const TAG: &str = "aimer.live_activity";
const CHANNEL_ID: &str = "aimer_live_activity";
const CHANNEL_NAME: &str = "Live updates";
/// `NotificationManager.IMPORTANCE_LOW`: shown without sound, and above the
/// minimal importance a promoted notification must not use.
const IMPORTANCE_LOW: i32 = 2;
/// The length of the one segment the bar is drawn with, and so the largest
/// progress and point position.
const PROGRESS_MAX: i32 = 100;
/// `android.R.drawable.ic_dialog_info`, for an app that declares no icon.
const FALLBACK_ICON: i32 = 17301618;
/// `Notification.CATEGORY_PROGRESS`.
const CATEGORY_PROGRESS: &str = "progress";
/// `PendingIntent.FLAG_UPDATE_CURRENT`.
const FLAG_UPDATE_CURRENT: i32 = 0x0800_0000;
/// `PendingIntent.FLAG_IMMUTABLE`.
const FLAG_IMMUTABLE: i32 = 0x0400_0000;

type Failure = String;

fn platform(failure: Failure) -> LiveActivityError {
    LiveActivityError::Platform(failure)
}

/// Android keeps notification ids as `int`s; the low 31 bits of ours are used,
/// so the id is never negative and the same [`LiveActivityId`] always replaces
/// the same notification.
///
/// Two ids that differ only above bit 31 collide and replace each other.
fn stable_id(id: LiveActivityId) -> i32 {
    (id.0 & 0x7fff_ffff) as i32
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

/// Everything a post needs.
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
fn with_env<T>(call: impl FnOnce(&mut jni::Env) -> Result<T, jni::errors::Error>) -> Result<T, Failure> {
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
    .map_err(|error| error.to_string())
}

pub(super) fn create() -> Result<Box<dyn Backend>, LiveActivityError> {
    let android = with_env(Android::connect).map_err(platform)?;
    Ok(Box::new(android))
}

impl Android {
    /// Looks up the context and the notification service, and declares the
    /// channel where the release has them.
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
            .get_static_field(jni_str!("android/os/Build$VERSION"), jni_str!("SDK_INT"), jni_sig!("I"))?
            .i()?;
        let android = Self {
            context: env.new_global_ref(&context)?,
            manager: env.new_global_ref(&manager)?,
            sdk_int,
        };
        if sdk_int >= CHANNEL_API {
            android.create_channel(env)?;
        }
        Ok(android)
    }

    /// Declares the channel; the system ignores a repeat of an existing one.
    fn create_channel(&self, env: &mut jni::Env) -> Result<(), jni::errors::Error> {
        let id = env.new_string(CHANNEL_ID)?;
        let name = env.new_string(CHANNEL_NAME)?;
        let channel = env.new_object(
            jni_str!("android/app/NotificationChannel"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/CharSequence;I)V"),
            &[JValue::Object(&id), JValue::Object(&name), JValue::Int(IMPORTANCE_LOW)],
        )?;
        env.call_method(
            &self.manager,
            jni_str!("createNotificationChannel"),
            jni_sig!("(Landroid/app/NotificationChannel;)V"),
            &[JValue::Object(&channel)],
        )?;
        Ok(())
    }

    /// Builds `activity` and hands it to the system under its stable id.
    fn post(&self, env: &mut jni::Env, activity: &LiveActivity) -> Result<(), jni::errors::Error> {
        let builder = if self.sdk_int >= CHANNEL_API {
            let channel = env.new_string(CHANNEL_ID)?;
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

        if !activity.title.is_empty() {
            let title = env.new_string(&activity.title)?;
            env.call_method(
                &builder,
                jni_str!("setContentTitle"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&title)],
            )?;
        }
        if !activity.body.is_empty() {
            let text = env.new_string(&activity.body)?;
            env.call_method(
                &builder,
                jni_str!("setContentText"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&text)],
            )?;
        }
        let icon = self.small_icon(env, &activity.icon)?;
        env.call_method(
            &builder,
            jni_str!("setSmallIcon"),
            jni_sig!("(I)Landroid/app/Notification$Builder;"),
            &[JValue::Int(icon)],
        )?;
        // A live activity lives until it is ended: it cannot be swiped away and
        // an update must not chime again.
        env.call_method(
            &builder,
            jni_str!("setOngoing"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        env.call_method(
            &builder,
            jni_str!("setOnlyAlertOnce"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        if let Some(countdown) = activity.countdown {
            self.apply_chronometer(env, &builder, countdown.end_ms)?;
        }
        let category = env.new_string(CATEGORY_PROGRESS)?;
        env.call_method(
            &builder,
            jni_str!("setCategory"),
            jni_sig!("(Ljava/lang/String;)Landroid/app/Notification$Builder;"),
            &[JValue::Object(&category)],
        )?;
        if let Some(intent) = self.launch_intent(env)? {
            env.call_method(
                &builder,
                jni_str!("setContentIntent"),
                jni_sig!("(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&intent)],
            )?;
        }

        if self.sdk_int >= PROGRESS_STYLE_API {
            self.apply_live_update(env, &builder, activity)?;
        } else {
            self.apply_ongoing(env, &builder, activity)?;
        }

        let built = env
            .call_method(&builder, jni_str!("build"), jni_sig!("()Landroid/app/Notification;"), &[])?
            .l()?;
        let tag = env.new_string(TAG)?;
        env.call_method(
            &self.manager,
            jni_str!("notify"),
            jni_sig!("(Ljava/lang/String;ILandroid/app/Notification;)V"),
            &[JValue::Object(&tag), JValue::Int(stable_id(activity.id)), JValue::Object(&built)],
        )?;
        Ok(())
    }

    /// A chronometer that counts down to `end_ms`. The system draws it, in the
    /// notification and in the status bar chip of a promoted one, and keeps it
    /// moving while the application is not running.
    fn apply_chronometer(
        &self,
        env: &mut jni::Env,
        builder: &JObject,
        end_ms: u64,
    ) -> Result<(), jni::errors::Error> {
        env.call_method(
            builder,
            jni_str!("setWhen"),
            jni_sig!("(J)Landroid/app/Notification$Builder;"),
            &[JValue::Long(i64::try_from(end_ms).unwrap_or(i64::MAX))],
        )?;
        env.call_method(
            builder,
            jni_str!("setShowWhen"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        env.call_method(
            builder,
            jni_str!("setUsesChronometer"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        env.call_method(
            builder,
            jni_str!("setChronometerCountDown"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        Ok(())
    }

    /// Android 16: ask for promotion, show the status in the chip and draw the
    /// bar with `ProgressStyle`. With a countdown the chip is left to the
    /// chronometer: a status text would replace the moving clock.
    fn apply_live_update(
        &self,
        env: &mut jni::Env,
        builder: &JObject,
        activity: &LiveActivity,
    ) -> Result<(), jni::errors::Error> {
        env.call_method(
            builder,
            jni_str!("setRequestPromotedOngoing"),
            jni_sig!("(Z)Landroid/app/Notification$Builder;"),
            &[JValue::Bool(true)],
        )?;
        if !activity.status.is_empty() && activity.countdown.is_none() {
            let status = env.new_string(&activity.status)?;
            env.call_method(
                builder,
                jni_str!("setShortCriticalText"),
                jni_sig!("(Ljava/lang/String;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&status)],
            )?;
        }
        if activity.progress == Progress::None {
            return Ok(());
        }

        let style = env.new_object(
            jni_str!("android/app/Notification$ProgressStyle"),
            jni_sig!("()V"),
            &[],
        )?;
        match activity.progress.percent() {
            None => {
                env.call_method(
                    &style,
                    jni_str!("setProgressIndeterminate"),
                    jni_sig!("(Z)Landroid/app/Notification$ProgressStyle;"),
                    &[JValue::Bool(true)],
                )?;
            }
            Some(percent) => {
                let segments = env.new_object(jni_str!("java/util/ArrayList"), jni_sig!("()V"), &[])?;
                let segment = env.new_object(
                    jni_str!("android/app/Notification$ProgressStyle$Segment"),
                    jni_sig!("(I)V"),
                    &[JValue::Int(PROGRESS_MAX)],
                )?;
                env.call_method(
                    &segments,
                    jni_str!("add"),
                    jni_sig!("(Ljava/lang/Object;)Z"),
                    &[JValue::Object(&segment)],
                )?;
                env.call_method(
                    &style,
                    jni_str!("setProgressSegments"),
                    jni_sig!("(Ljava/util/List;)Landroid/app/Notification$ProgressStyle;"),
                    &[JValue::Object(&segments)],
                )?;

                let points = env.new_object(jni_str!("java/util/ArrayList"), jni_sig!("()V"), &[])?;
                for milestone in &activity.milestones {
                    let point = env.new_object(
                        jni_str!("android/app/Notification$ProgressStyle$Point"),
                        jni_sig!("(I)V"),
                        &[JValue::Int(i32::from(milestone.position()))],
                    )?;
                    env.call_method(
                        &points,
                        jni_str!("add"),
                        jni_sig!("(Ljava/lang/Object;)Z"),
                        &[JValue::Object(&point)],
                    )?;
                }
                env.call_method(
                    &style,
                    jni_str!("setProgressPoints"),
                    jni_sig!("(Ljava/util/List;)Landroid/app/Notification$ProgressStyle;"),
                    &[JValue::Object(&points)],
                )?;
                env.call_method(
                    &style,
                    jni_str!("setProgress"),
                    jni_sig!("(I)Landroid/app/Notification$ProgressStyle;"),
                    &[JValue::Int(i32::from(percent))],
                )?;
            }
        }
        env.call_method(
            builder,
            jni_str!("setStyle"),
            jni_sig!("(Landroid/app/Notification$Style;)Landroid/app/Notification$Builder;"),
            &[JValue::Object(&style)],
        )?;
        Ok(())
    }

    /// Android 7 to 15: the status as sub text and a plain progress bar. The
    /// bar is only as current as the last update; a countdown's clock is not.
    fn apply_ongoing(
        &self,
        env: &mut jni::Env,
        builder: &JObject,
        activity: &LiveActivity,
    ) -> Result<(), jni::errors::Error> {
        if !activity.status.is_empty() {
            let status = env.new_string(&activity.status)?;
            env.call_method(
                builder,
                jni_str!("setSubText"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&status)],
            )?;
        }
        let (percent, indeterminate) = match activity.progress {
            Progress::None => return Ok(()),
            Progress::Indeterminate => (0, true),
            Progress::Fraction(_) => (i32::from(activity.progress.percent().unwrap_or(0)), false),
        };
        env.call_method(
            builder,
            jni_str!("setProgress"),
            jni_sig!("(IIZ)Landroid/app/Notification$Builder;"),
            &[JValue::Int(PROGRESS_MAX), JValue::Int(percent), JValue::Bool(indeterminate)],
        )?;
        Ok(())
    }

    /// The drawable called `name` in the application's resources, or the
    /// application's own icon, or the system's info icon when it has none.
    fn small_icon(&self, env: &mut jni::Env, name: &str) -> Result<i32, jni::errors::Error> {
        if !name.is_empty() {
            let package = env
                .call_method(
                    &self.context,
                    jni_str!("getPackageName"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            let resources = env
                .call_method(
                    &self.context,
                    jni_str!("getResources"),
                    jni_sig!("()Landroid/content/res/Resources;"),
                    &[],
                )?
                .l()?;
            let name = env.new_string(name)?;
            let kind = env.new_string("drawable")?;
            let found = env
                .call_method(
                    &resources,
                    jni_str!("getIdentifier"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I"),
                    &[JValue::Object(&name), JValue::Object(&kind), JValue::Object(&package)],
                )?
                .i()?;
            if found != 0 {
                return Ok(found);
            }
        }
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
        let icon = env.get_field(&info, jni_str!("icon"), jni_sig!("I"))?.i()?;
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
            .call_method(&self.context, jni_str!("getPackageName"), jni_sig!("()Ljava/lang/String;"), &[])?
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

    fn cancel(&self, env: &mut jni::Env, id: LiveActivityId) -> Result<(), jni::errors::Error> {
        let tag = env.new_string(TAG)?;
        env.call_method(
            &self.manager,
            jni_str!("cancel"),
            jni_sig!("(Ljava/lang/String;I)V"),
            &[JValue::Object(&tag), JValue::Int(stable_id(id))],
        )?;
        Ok(())
    }
}

impl Backend for Android {
    fn show(&self, activity: &LiveActivity) -> Result<(), LiveActivityError> {
        with_env(|env| self.post(env, activity)).map_err(platform)
    }

    fn end(&self, id: LiveActivityId) -> Result<(), LiveActivityError> {
        with_env(|env| self.cancel(env, id)).map_err(platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_never_negative() {
        assert_eq!(stable_id(LiveActivityId(7)), 7);
        assert_eq!(stable_id(LiveActivityId(u64::MAX)), i32::MAX);
        assert!(stable_id(LiveActivityId(0x8000_0000)) >= 0);
    }

    #[test]
    fn pending_intent_is_immutable_where_supported() {
        assert_eq!(pending_intent_flags(22), FLAG_UPDATE_CURRENT);
        assert_eq!(pending_intent_flags(23), FLAG_UPDATE_CURRENT | FLAG_IMMUTABLE);
        assert_eq!(pending_intent_flags(36), FLAG_UPDATE_CURRENT | FLAG_IMMUTABLE);
    }
}
