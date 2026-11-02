//! Android: `android.os.Vibrator` over JNI.
//!
//! What to play is decided by [`crate::waveform`]; this module only finds the
//! system vibrator and hands the effect to it, using the newest API the running
//! release has:
//!
//! | API    | Call                                                       |
//! |--------|------------------------------------------------------------|
//! | 29+    | `VibrationEffect.createPredefined` for ticks and clicks     |
//! | 26+    | `createOneShot` / `createWaveform` (with amplitude)         |
//! | 24, 25 | `vibrate(long[], int)` (on/off only, no amplitude)          |
//!
//! The app must declare `<uses-permission android:name="android.permission.VIBRATE"/>`;
//! it is a normal permission, granted at install time. Without it the call is
//! refused with a `SecurityException`, which is logged and otherwise ignored.

use std::sync::OnceLock;

use jni::objects::{Global, JIntArray, JLongArray, JObject, JValue};
use jni::{jni_sig, jni_str};

use crate::waveform::{Effect, Segment, legacy_timings};
use crate::{HapticPattern, ImpactStyle, NotificationStyle};

/// First release with `VibrationEffect` and amplitude control.
const VIBRATION_EFFECT_API: i32 = 26;
/// First release with `VibrationEffect.createPredefined`.
const PREDEFINED_EFFECT_API: i32 = 29;
/// `Vibrator.vibrate(long[], int)`'s "play once" repeat index.
const NO_REPEAT: i32 = -1;

/// The process-wide system vibrator.
///
/// The service never changes for the life of the process, so it is looked up
/// once and kept as a global reference; haptics fire on input events and should
/// not pay for a `getSystemService` round trip each time.
struct Vibrator {
    object: Global<JObject<'static>>,
    sdk_int: i32,
}

/// `None` once the device is known to have no vibrator (most tablets), so the
/// lookup is not repeated, and not reported, on every haptic.
static VIBRATOR: OnceLock<Option<Vibrator>> = OnceLock::new();

type Failure = String;

fn describe(error: jni::errors::Error) -> Failure {
    error.to_string()
}

fn report(what: &str, failure: &str) {
    eprintln!("aimer_haptics: {what} failed: {failure}");
}

/// Attaches the thread to the JVM and runs `call` with the system vibrator.
/// Returns `Ok(None)` without calling it on a device that has none.
///
/// A Java exception left pending by a failed call would poison every later JNI
/// call on the thread, so it is printed (to logcat, which is where a missing
/// `VIBRATE` permission becomes visible) and cleared before the error returns.
fn with_vibrator<T>(
    call: impl FnOnce(&mut jni::Env, &Vibrator) -> Result<T, jni::errors::Error>,
) -> Result<Option<T>, Failure> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return Err("the Android runtime is not running".into());
    }
    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr) };
    vm.attach_current_thread(|env| {
        let outcome = (|| {
            let known = match VIBRATOR.get() {
                Some(known) => known,
                None => {
                    let found = find_vibrator(env, context.context() as jni::sys::jobject)?;
                    // Another thread may have raced us here; either result is
                    // an equivalent vibrator, so keep whichever landed first.
                    VIBRATOR.get_or_init(|| found)
                }
            };
            match known {
                Some(vibrator) => call(env, vibrator).map(Some),
                None => Ok(None),
            }
        })();
        if outcome.is_err() && env.exception_check() {
            env.exception_describe();
            env.exception_clear();
        }
        outcome
    })
    .map_err(describe)
}

fn find_vibrator(
    env: &mut jni::Env,
    context_ptr: jni::sys::jobject,
) -> Result<Option<Vibrator>, jni::errors::Error> {
    if context_ptr.is_null() {
        return Err(jni::errors::Error::NullPtr("application context"));
    }
    // SAFETY: the context is a global reference owned by the android-activity
    // runtime, not a local one, so it needs no frame of its own.
    let context = unsafe { JObject::from_raw(&*env, context_ptr) };

    let name = env.new_string("vibrator")?;
    let service = env
        .call_method(
            &context,
            jni_str!("getSystemService"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::Object(&name)],
        )?
        .l()?;
    if service.is_null() {
        return Err(jni::errors::Error::NullPtr("vibrator service"));
    }
    let has_vibrator = env
        .call_method(&service, jni_str!("hasVibrator"), jni_sig!("()Z"), &[])?
        .z()?;
    if !has_vibrator {
        return Ok(None);
    }
    let sdk_int = env
        .get_static_field(
            jni_str!("android/os/Build$VERSION"),
            jni_str!("SDK_INT"),
            jni_sig!("I"),
        )?
        .i()?;
    Ok(Some(Vibrator {
        object: env.new_global_ref(&service)?,
        sdk_int,
    }))
}

/// Plays `effect`, replacing whatever is vibrating.
fn play(effect: &Effect) {
    let result = with_vibrator(|env, vibrator| match effect {
        Effect::Predefined { id, .. } if vibrator.sdk_int >= PREDEFINED_EFFECT_API => {
            let effect = env
                .call_static_method(
                    jni_str!("android/os/VibrationEffect"),
                    jni_str!("createPredefined"),
                    jni_sig!("(I)Landroid/os/VibrationEffect;"),
                    &[JValue::Int(*id)],
                )?
                .l()?;
            vibrate_effect(env, vibrator, &effect)
        }
        Effect::Predefined { fallback, .. } | Effect::Timeline(fallback) => {
            play_segments(env, vibrator, fallback)
        }
    });
    if let Err(failure) = result {
        report("Vibrator.vibrate", &failure);
    }
}

fn vibrate_effect(
    env: &mut jni::Env,
    vibrator: &Vibrator,
    effect: &JObject<'_>,
) -> Result<(), jni::errors::Error> {
    env.call_method(
        &vibrator.object,
        jni_str!("vibrate"),
        jni_sig!("(Landroid/os/VibrationEffect;)V"),
        &[JValue::Object(effect)],
    )?;
    Ok(())
}

fn play_segments(
    env: &mut jni::Env,
    vibrator: &Vibrator,
    segments: &[Segment],
) -> Result<(), jni::errors::Error> {
    if vibrator.sdk_int < VIBRATION_EFFECT_API {
        return play_legacy(env, vibrator, segments);
    }

    let effect = match segments {
        [only] if only.amplitude > 0 => env
            .call_static_method(
                jni_str!("android/os/VibrationEffect"),
                jni_str!("createOneShot"),
                jni_sig!("(JI)Landroid/os/VibrationEffect;"),
                &[JValue::Long(only.millis), JValue::Int(only.amplitude)],
            )?
            .l()?,
        _ => {
            let timings: Vec<i64> = segments.iter().map(|s| s.millis).collect();
            let amplitudes: Vec<i32> = segments.iter().map(|s| s.amplitude).collect();
            let timings_array = long_array(env, &timings)?;
            let amplitudes_array = int_array(env, &amplitudes)?;
            env.call_static_method(
                jni_str!("android/os/VibrationEffect"),
                jni_str!("createWaveform"),
                jni_sig!("([J[II)Landroid/os/VibrationEffect;"),
                &[
                    JValue::Object(&timings_array),
                    JValue::Object(&amplitudes_array),
                    JValue::Int(NO_REPEAT),
                ],
            )?
            .l()?
        }
    };
    vibrate_effect(env, vibrator, &effect)
}

/// API 24 and 25: wait/vibrate durations only, no amplitude.
fn play_legacy(
    env: &mut jni::Env,
    vibrator: &Vibrator,
    segments: &[Segment],
) -> Result<(), jni::errors::Error> {
    let timings = long_array(env, &legacy_timings(segments))?;
    env.call_method(
        &vibrator.object,
        jni_str!("vibrate"),
        jni_sig!("([JI)V"),
        &[JValue::Object(&timings), JValue::Int(NO_REPEAT)],
    )?;
    Ok(())
}

fn long_array<'local>(
    env: &mut jni::Env<'local>,
    values: &[i64],
) -> Result<JLongArray<'local>, jni::errors::Error> {
    let array = JLongArray::new(env, values.len())?;
    array.set_region(env, 0, values)?;
    Ok(array)
}

fn int_array<'local>(
    env: &mut jni::Env<'local>,
    values: &[i32],
) -> Result<JIntArray<'local>, jni::errors::Error> {
    let array = JIntArray::new(env, values.len())?;
    array.set_region(env, 0, values)?;
    Ok(array)
}

/// Whether this backend can produce feedback at all.
pub(crate) const SUPPORTED: bool = true;

/// The vibration motor has no warm-up worth hiding.
pub(crate) fn prepare_impact(_style: ImpactStyle) {}

pub(crate) fn impact(style: ImpactStyle) {
    play(&Effect::impact(style));
}

pub(crate) fn selection() {
    play(&Effect::selection());
}

pub(crate) fn notification(style: NotificationStyle) {
    play(&Effect::notification(style));
}

pub(crate) fn play_pattern(pattern: &HapticPattern) {
    if let Some(effect) = Effect::pattern(pattern) {
        play(&effect);
    }
}
