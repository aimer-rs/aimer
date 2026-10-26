//! Android, from `android.os.Build` over JNI.
//!
//! The build properties are static fields, so they need only the `JavaVM`. The
//! form factor and the user's name for the device need the application context
//! as well; when `ndk-context` has none yet they are left as `Unknown` and
//! `None` rather than failing the whole query.

use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str};

use crate::virtualization::{AndroidBuild, is_android_emulator};
use crate::{
    AndroidDetails, DeviceInfo, DeviceInfoError, DeviceKind, OsVersion, Platform, PlatformDetails,
    Queried,
};

/// The smallest-width threshold, in dp, at which Android's own resource
/// qualifiers (`sw600dp`) start treating a screen as a tablet.
const TABLET_MIN_SMALLEST_WIDTH_DP: i32 = 600;

pub(super) fn query() -> Result<Queried, DeviceInfoError> {
    let info = with_vm(|env, context| {
        let build = jni_str!("android/os/Build");
        let version = jni_str!("android/os/Build$VERSION");
        let text = |class: &JNIStr, field: &JNIStr, env: &mut jni::Env| {
            static_string(env, class, field).map(Option::unwrap_or_default)
        };

        let manufacturer = text(build, jni_str!("MANUFACTURER"), env)?;
        let model = text(build, jni_str!("MODEL"), env)?;
        let android = AndroidDetails {
            sdk_int: u32::try_from(
                env.get_static_field(version, jni_str!("SDK_INT"), jni_sig!("I"))?
                    .i()?,
            )
            .unwrap_or(0),
            brand: text(build, jni_str!("BRAND"), env)?,
            device: text(build, jni_str!("DEVICE"), env)?,
            product: text(build, jni_str!("PRODUCT"), env)?,
            board: text(build, jni_str!("BOARD"), env)?,
            hardware: text(build, jni_str!("HARDWARE"), env)?,
            fingerprint: text(build, jni_str!("FINGERPRINT"), env)?,
            security_patch: static_string(env, version, jni_str!("SECURITY_PATCH"))?,
            supported_abis: supported_abis(env, build)?,
        };
        let release = text(version, jni_str!("RELEASE"), env)?;
        let incremental = static_string(env, version, jni_str!("INCREMENTAL"))?;

        let is_physical = !is_android_emulator(&AndroidBuild {
            fingerprint: &android.fingerprint,
            model: &model,
            manufacturer: &manufacturer,
            brand: &android.brand,
            device: &android.device,
            product: &android.product,
            hardware: &android.hardware,
        });
        // The context-dependent fields are best effort: a failed JNI call is
        // cleared and reported as "not known".
        let kind = context
            .and_then(|context| optional(env, |env| form_factor(env, context)))
            .unwrap_or(DeviceKind::Unknown);
        let name = context.and_then(|context| optional(env, |env| device_name(env, context)).flatten());

        Ok(DeviceInfo {
            platform: Platform::Android,
            kind,
            os_name: "Android".into(),
            os_version: OsVersion::parse(&release),
            os_build: incremental,
            model: non_empty(model),
            manufacturer: non_empty(manufacturer),
            name,
            arch: std::env::consts::ARCH,
            is_physical,
            details: PlatformDetails::Android(android),
        })
    })?;
    Ok(Queried {
        info,
        complete: true,
    })
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// Runs `call`, turning a failure into `None` and clearing the Java exception
/// it may have left pending, which would poison every later JNI call.
fn optional<T>(
    env: &mut jni::Env,
    call: impl FnOnce(&mut jni::Env) -> Result<T, jni::errors::Error>,
) -> Option<T> {
    match call(env) {
        Ok(value) => Some(value),
        Err(_) => {
            env.exception_clear();
            None
        }
    }
}

/// A `static String` field, or `None` when it is `null`.
fn static_string(
    env: &mut jni::Env,
    class: &JNIStr,
    field: &JNIStr,
) -> Result<Option<String>, jni::errors::Error> {
    let value = env
        .get_static_field(class, field, jni_sig!("Ljava/lang/String;"))?
        .l()?;
    if value.is_null() {
        return Ok(None);
    }
    let value = env.cast_local::<JString>(value)?;
    Ok(Some(value.try_to_string(env)?))
}

/// `Build.SUPPORTED_ABIS`, most preferred first.
fn supported_abis(env: &mut jni::Env, build: &JNIStr) -> Result<Vec<String>, jni::errors::Error> {
    let array = env
        .get_static_field(build, jni_str!("SUPPORTED_ABIS"), jni_sig!("[Ljava/lang/String;"))?
        .l()?;
    if array.is_null() {
        return Ok(Vec::new());
    }
    let array = env.cast_local::<JObjectArray<JString>>(array)?;
    let len = array.len(env)?;
    let mut abis = Vec::with_capacity(len);
    for index in 0..len {
        let abi = array.get_element(env, index)?;
        abis.push(abi.try_to_string(env)?);
    }
    Ok(abis)
}

/// Phone or tablet, by the smallest screen width in dp, which does not change
/// with rotation — the same measure Android's `sw600dp` qualifier uses.
fn form_factor(env: &mut jni::Env, context: &JObject<'_>) -> Result<DeviceKind, jni::errors::Error> {
    let resources = env
        .call_method(
            context,
            jni_str!("getResources"),
            jni_sig!("()Landroid/content/res/Resources;"),
            &[],
        )?
        .l()?;
    let configuration = env
        .call_method(
            &resources,
            jni_str!("getConfiguration"),
            jni_sig!("()Landroid/content/res/Configuration;"),
            &[],
        )?
        .l()?;
    let smallest_width = env
        .get_field(&configuration, jni_str!("smallestScreenWidthDp"), jni_sig!("I"))?
        .i()?;
    Ok(match smallest_width {
        // `SMALLEST_SCREEN_WIDTH_DP_UNDEFINED`.
        i32::MIN..=0 => DeviceKind::Unknown,
        TABLET_MIN_SMALLEST_WIDTH_DP.. => DeviceKind::Tablet,
        _ => DeviceKind::Phone,
    })
}

/// `Settings.Global.device_name`, which the user sets in system settings and
/// which needs no permission to read (API 25 and later; `null` before that).
fn device_name(
    env: &mut jni::Env,
    context: &JObject<'_>,
) -> Result<Option<String>, jni::errors::Error> {
    let resolver = env
        .call_method(
            context,
            jni_str!("getContentResolver"),
            jni_sig!("()Landroid/content/ContentResolver;"),
            &[],
        )?
        .l()?;
    let key = env.new_string("device_name")?;
    let value = env
        .call_static_method(
            jni_str!("android/provider/Settings$Global"),
            jni_str!("getString"),
            jni_sig!("(Landroid/content/ContentResolver;Ljava/lang/String;)Ljava/lang/String;"),
            &[JValue::Object(&resolver), JValue::Object(&key)],
        )?
        .l()?;
    if value.is_null() {
        return Ok(None);
    }
    let value = env.cast_local::<JString>(value)?;
    Ok(non_empty(value.try_to_string(env)?))
}

/// Attaches the current thread to the JVM and runs `call` with a live `Env` and
/// the application context, if `ndk-context` has one yet.
fn with_vm<T>(
    call: impl FnOnce(&mut jni::Env, Option<&JObject<'_>>) -> Result<T, jni::errors::Error>,
) -> Result<T, DeviceInfoError> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    let context_ptr = context.context() as jni::sys::jobject;
    if vm_ptr.is_null() {
        return Err(DeviceInfoError::Unavailable(
            "the Android runtime is not running".into(),
        ));
    }

    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };
    vm.attach_current_thread(|env| {
        // SAFETY: the context is a global reference owned by the same runtime,
        // not a local one, so it needs no frame of its own.
        let context =
            (!context_ptr.is_null()).then(|| unsafe { JObject::from_raw(&*env, context_ptr) });
        call(env, context.as_ref())
    })
    .map_err(|error: jni::errors::Error| DeviceInfoError::Unavailable(error.to_string()))
}
