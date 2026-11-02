//! Android, from the default `Display`'s rotation over JNI.
//!
//! `DisplayManager.getDisplay(0)` is used rather than the deprecated
//! `WindowManager.getDefaultDisplay`, and needs no activity: the application
//! context `ndk-context` holds is enough. The rotation alone is relative to the
//! device's natural orientation, so the display's real size is read with it to
//! tell a phone from a tablet that is naturally landscape.
//!
//! Folds and hinges do not come from here; they arrive from `ScreenBridge.kt`
//! through [`crate::post_display_features`].

use jni::objects::JObject;
use jni::{jni_sig, jni_str};

use crate::DeviceOrientation;

/// `Display.DEFAULT_DISPLAY`.
const DEFAULT_DISPLAY: i32 = 0;

pub(super) fn device_orientation() -> Option<DeviceOrientation> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    let context_ptr = context.context() as jni::sys::jobject;
    if vm_ptr.is_null() || context_ptr.is_null() {
        return None;
    }

    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };
    let read = vm.attach_current_thread(|env| -> Result<Option<DeviceOrientation>, jni::errors::Error> {
        // SAFETY: a global reference owned by the same runtime, not a local one.
        let context = unsafe { JObject::from_raw(&*env, context_ptr) };
        let service = env.new_string("display")?;
        let manager = env
            .call_method(
                &context,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[jni::objects::JValue::Object(&service)],
            )?
            .l()?;
        let display = env
            .call_method(
                &manager,
                jni_str!("getDisplay"),
                jni_sig!("(I)Landroid/view/Display;"),
                &[jni::objects::JValue::Int(DEFAULT_DISPLAY)],
            )?
            .l()?;
        if display.is_null() {
            return Ok(None);
        }
        let rotation = env
            .call_method(&display, jni_str!("getRotation"), jni_sig!("()I"), &[])?
            .i()?;
        let size = env.new_object(jni_str!("android/graphics/Point"), jni_sig!("()V"), &[])?;
        env.call_method(
            &display,
            jni_str!("getRealSize"),
            jni_sig!("(Landroid/graphics/Point;)V"),
            &[jni::objects::JValue::Object(&size)],
        )?;
        let width = env.get_field(&size, jni_str!("x"), jni_sig!("I"))?.i()?;
        let height = env.get_field(&size, jni_str!("y"), jni_sig!("I"))?.i()?;
        Ok(DeviceOrientation::from_android_rotation(
            rotation,
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        ))
    });
    match read {
        Ok(orientation) => orientation,
        Err(error) => {
            // A pending Java exception would poison later JNI calls on this thread.
            let _ = error;
            None
        }
    }
}
