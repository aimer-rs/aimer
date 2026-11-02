//! Android, through `android.content.ClipboardManager`.
//!
//! There is no Java shim to call into, so the whole conversation happens in
//! JNI: the activity and the `JavaVM` come from `ndk-context`, which
//! `android-activity` populates on start-up, and the calling thread is attached
//! for the duration of one operation.
//!
//! Both operations are Binder round-trips to the system clipboard service, not
//! view work, so they do not have to be marshalled onto the Java main thread.

use jni::objects::{JObject, JString, JValue};
use jni::{jni_sig, jni_str};

use super::ClipboardError;

/// `android.content.Context.CLIPBOARD_SERVICE`, whose value is the literal
/// `"clipboard"`; naming the constant would cost a static field lookup.
const CLIPBOARD_SERVICE: &str = "clipboard";

/// The label Android shows for the clip in system UI, e.g. the clipboard
/// preview on Android 13 and later.
const CLIP_LABEL: &str = "aimer";

pub(super) fn set_text(text: &str) -> Result<(), ClipboardError> {
    with_activity(|env, activity| {
        let manager = clipboard_manager(env, activity)?;
        let label = env.new_string(CLIP_LABEL)?;
        let value = env.new_string(text)?;
        let clip = env
            .call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newPlainText"),
                jni_sig!(
                    "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"
                ),
                &[JValue::Object(&label), JValue::Object(&value)],
            )?
            .l()?;
        env.call_method(
            &manager,
            jni_str!("setPrimaryClip"),
            jni_sig!("(Landroid/content/ClipData;)V"),
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    })
}

pub(super) fn get_text() -> Result<String, ClipboardError> {
    let text = with_activity(|env, activity| {
        let manager = clipboard_manager(env, activity)?;
        let clip = env
            .call_method(
                &manager,
                jni_str!("getPrimaryClip"),
                jni_sig!("()Landroid/content/ClipData;"),
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env
            .call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?
            .i()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = env
            .call_method(
                &clip,
                jni_str!("getItemAt"),
                jni_sig!("(I)Landroid/content/ClipData$Item;"),
                &[JValue::Int(0)],
            )?
            .l()?;
        // `coerceToText` is what the platform's own paste does: it renders an
        // HTML or URI clip as the text a user would expect, instead of failing.
        let coerced = env
            .call_method(
                &item,
                jni_str!("coerceToText"),
                jni_sig!("(Landroid/content/Context;)Ljava/lang/CharSequence;"),
                &[JValue::Object(activity)],
            )?
            .l()?;
        if coerced.is_null() {
            return Ok(None);
        }
        let string = env
            .call_method(
                &coerced,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let string = env.cast_local::<JString>(string)?;
        Ok(Some(string.try_to_string(env)?))
    })?;
    text.ok_or(ClipboardError::Unsupported)
}

/// The activity's `ClipboardManager`.
fn clipboard_manager<'local>(
    env: &mut jni::Env<'local>,
    activity: &JObject<'_>,
) -> Result<JObject<'local>, jni::errors::Error> {
    let name = env.new_string(CLIPBOARD_SERVICE)?;
    env.call_method(
        activity,
        jni_str!("getSystemService"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
        &[JValue::Object(&name)],
    )?
    .l()
}

/// Attaches the current thread to the JVM and runs `call` with a live `Env` and
/// the running activity, translating every JNI failure into a
/// [`ClipboardError::Unavailable`].
fn with_activity<T>(
    call: impl FnOnce(&mut jni::Env, &JObject<'_>) -> Result<T, jni::errors::Error>,
) -> Result<T, ClipboardError> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    let activity_ptr = context.context() as jni::sys::jobject;
    if vm_ptr.is_null() || activity_ptr.is_null() {
        return Err(ClipboardError::Unavailable(
            "the Android activity is not running".into(),
        ));
    }

    // SAFETY: `ndk-context` hands out the `JavaVM` and the global activity
    // reference owned by the android-activity runtime, both valid for the life
    // of the process.
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };
    vm.attach_current_thread(|env| {
        // SAFETY: as above — a global reference, not a local one, so it needs
        // no frame of its own.
        let activity = unsafe { JObject::from_raw(&*env, activity_ptr) };
        call(env, &activity)
    })
    .map_err(|error: jni::errors::Error| ClipboardError::Unavailable(error.to_string()))
}
