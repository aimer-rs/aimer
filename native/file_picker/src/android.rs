//! Android: the Storage Access Framework and the system photo picker.
//!
//! `startActivityForResult` answers an `Activity`, which Rust cannot be, so the
//! Android half is Kotlin: `com.aimer.FilePickerBridge` and the transparent
//! `com.aimer.PickerActivity` (see the crate's `android/` directory), which the
//! application ships. Rust sends the bridge a request with a token, and the
//! bridge calls back into [`native_complete`] on Android's UI thread with the
//! outcome. That completes a Venus [`Completion`], which the returned future
//! awaits, so the result is applied in the frame phase of the awaiting task.
//!
//! The Android context the runtime holds (`ndk-context`) is the `Application`,
//! not an `Activity`; the bridge tracks the foreground activity itself.
//!
//! A picked file is a `content://` URI with no path. [`read`] asks the bridge to
//! read it through `ContentResolver` on a worker thread.
//!
//! Dropping a pending pick tells the bridge to dismiss the picker.

use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::task::{Context, Poll};

use aimer_venus::{Completer, Completion, completion};
use jni::errors::Error;
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::sys::jlong;
use jni::{Env, EnvUnowned, JavaVM, NativeMethod, jni_sig, jni_str};

use crate::android_wire::{Query, parse_entries, query};
use crate::error::{PickError, PickResult};
use crate::future::{PickFuture, ready};
use crate::picked::{PickedFile, Source};
use crate::request::FilePicker;

/// The bridge class, by the name the class loader knows it.
const BRIDGE_CLASS: &str = "com.aimer.FilePickerBridge";
/// The status the bridge reports on success.
const OK: &str = "ok";

/// What the bridge reports: [`OK`] and the file records, or an error kind and the
/// exception's message.
struct Reply {
    status: String,
    payload: String,
}

/// Requests the bridge has not answered yet, by token.
static PENDING: LazyLock<Mutex<HashMap<i64, Completer<Reply>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_TOKEN: AtomicI64 = AtomicI64::new(1);

fn pending() -> MutexGuard<'static, HashMap<i64, Completer<Reply>>> {
    // Nothing runs under the lock but map operations, so a poisoned map is still
    // consistent.
    PENDING.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn pick(request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    let query = query(&request.filters);
    match begin(&query, request.multiple, request.max_selection()) {
        Ok(pick) => Box::pin(pick),
        Err(error) => ready(Err(error)),
    }
}

/// A pick in flight. Dropping it before it finishes dismisses the picker.
struct Pick {
    token: i64,
    completion: Completion<Reply>,
    finished: bool,
}

impl Future for Pick {
    type Output = PickResult<Vec<PickedFile>>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let pick = &mut *self;
        match Pin::new(&mut pick.completion).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(reply) => {
                pick.finished = true;
                Poll::Ready(match reply {
                    Ok(Reply { status, payload }) if status == OK => Ok(parse_entries(&payload)
                        .into_iter()
                        .map(|entry| {
                            PickedFile::from_parts(entry.name, entry.mime_type, entry.size, Source::Uri(entry.uri))
                        })
                        .collect()),
                    Ok(Reply { status, payload }) => Err(PickError::Platform(if payload.is_empty() {
                        status
                    } else {
                        format!("{status}: {payload}")
                    })),
                    Err(_) => Err(PickError::Interrupted),
                })
            }
        }
    }
}

impl Drop for Pick {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // The bridge may answer any moment; whichever of the two loses the race
        // finds nothing to do.
        pending().remove(&self.token);
        let token = self.token;
        let _ = run(|env| {
            let class = bridge(env)?;
            env.call_static_method(class, jni_str!("cancel"), jni_sig!("(J)V"), &[JValue::Long(token)])
                .map(|_| ())
        });
    }
}

/// Hands a request to the bridge.
fn begin(query: &Query, multiple: bool, max: Option<usize>) -> PickResult<Pick> {
    let (completer, completion) = completion();
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    pending().insert(token, completer);

    // 0 tells the bridge there is no cap; a cap too large for an `int` is none.
    let limit = max.and_then(|max| i32::try_from(max).ok()).unwrap_or(0);
    let started = run(|env| {
        let class = bridge(env)?;
        let mime_types = env.new_string(query.mime_types.join(","))?;
        env.call_static_method(
            class,
            jni_str!("pick"),
            jni_sig!("(JLjava/lang/String;ZZI)V"),
            &[
                JValue::Long(token),
                JValue::Object(&mime_types),
                JValue::Bool(multiple),
                JValue::Bool(query.media),
                JValue::Int(limit),
            ],
        )
        .map(|_| ())
    });
    if let Err(error) = started {
        pending().remove(&token);
        return Err(error.into_error());
    }
    Ok(Pick {
        token,
        completion,
        finished: false,
    })
}

/// Reads a picked `content://` URI on a worker thread.
pub(crate) fn read(uri: String) -> PickFuture<Vec<u8>> {
    match aimer_venus::spawn_blocking({
        let uri = uri.clone();
        move || read_blocking(&uri)
    }) {
        Some(offloaded) => Box::pin(offloaded),
        None => ready(read_blocking(&uri)),
    }
}

fn read_blocking(uri: &str) -> PickResult<Vec<u8>> {
    run(|env| {
        let class = bridge(env)?;
        let uri = env.new_string(uri)?;
        let bytes = env
            .call_static_method(
                class,
                jni_str!("read"),
                jni_sig!("(Ljava/lang/String;)[B"),
                &[JValue::Object(&uri)],
            )?
            .l()?;
        let bytes = env.cast_local::<JByteArray>(bytes)?;
        env.convert_byte_array(&bytes)
    })
    .map_err(Run::into_read_error)
}

/// The bridge class, loaded through the application's class loader (a native
/// thread's own loader cannot see application classes) and with its native
/// method registered, once per process.
fn bridge<'a>(env: &mut Env<'_>) -> Result<&'a JClass<'static>, Error> {
    static BRIDGE: OnceLock<Global<JClass<'static>>> = OnceLock::new();
    if let Some(class) = BRIDGE.get() {
        return Ok(class);
    }

    let context = ndk_context::android_context();
    // SAFETY: `ndk-context` hands out the global application context owned by the
    // android-activity runtime, valid for the life of the process.
    let application = unsafe { JObject::from_raw(&*env, context.context() as jni::sys::jobject) };
    let loader = env
        .call_method(
            &application,
            jni_str!("getClassLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?;
    let name = env.new_string(BRIDGE_CLASS)?;
    let class = env
        .call_method(
            &loader,
            jni_str!("loadClass"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
            &[JValue::Object(&name)],
        )?
        .l()?;
    let class = env.cast_local::<JClass>(class)?;

    // SAFETY: `native_complete` has the signature the descriptor declares for a
    // static method: the environment, the class, a `long` and two strings.
    unsafe {
        env.register_native_methods(
            &class,
            &[NativeMethod::from_raw_parts(
                jni_str!("nativeComplete"),
                jni_str!("(JLjava/lang/String;Ljava/lang/String;)V"),
                native_complete as *mut c_void,
            )],
        )?;
    }
    // The context the Rust runtime holds is the `Application`; handing it over lets
    // the bridge track the foreground activity and read content URIs.
    env.call_static_method(
        &class,
        jni_str!("init"),
        jni_sig!("(Landroid/content/Context;)V"),
        &[JValue::Object(&application)],
    )?;
    let global = env.new_global_ref(class)?;
    Ok(BRIDGE.get_or_init(|| global))
}

/// Called by `FilePickerBridge` on Android's UI thread when a request ends.
extern "system" fn native_complete<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    token: jlong,
    status: JString<'local>,
    payload: JString<'local>,
) {
    env.with_env(|env| -> Result<(), Error> {
        let reply = Reply {
            status: String::from(status.mutf8_chars(env)?),
            payload: String::from(payload.mutf8_chars(env)?),
        };
        // A token with no entry belongs to a pick that was dropped.
        if let Some(completer) = pending().remove(&token) {
            completer.complete(reply);
        }
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// Why a JNI call failed, after exceptions have been caught and cleared.
enum Run {
    /// No JVM or no application context to talk to.
    NoContext,
    Java(Error),
}

impl Run {
    fn is_missing_bridge(&self) -> bool {
        matches!(self, Self::Java(Error::CaughtJavaException { name, .. })
            if name.ends_with("ClassNotFoundException") || name.ends_with("NoClassDefFoundError"))
    }

    fn into_error(self) -> PickError {
        match self {
            _ if self.is_missing_bridge() => PickError::BridgeMissing,
            Self::NoContext => PickError::NoWindow,
            Self::Java(error) => PickError::Platform(error.to_string()),
        }
    }

    fn into_read_error(self) -> PickError {
        match self {
            _ if self.is_missing_bridge() => PickError::BridgeMissing,
            Self::NoContext => PickError::Unsupported,
            Self::Java(Error::CaughtJavaException { name, .. }) if name.ends_with("FileNotFoundException") => {
                PickError::Io(io::ErrorKind::NotFound)
            }
            Self::Java(Error::CaughtJavaException { name, .. }) if name.ends_with("SecurityException") => {
                PickError::Io(io::ErrorKind::PermissionDenied)
            }
            Self::Java(Error::CaughtJavaException { .. }) => PickError::Io(io::ErrorKind::Other),
            Self::Java(error) => PickError::Platform(error.to_string()),
        }
    }
}

impl From<Error> for Run {
    fn from(error: Error) -> Self {
        Self::Java(error)
    }
}

/// Attaches the current thread to the JVM and runs `call` with a live `Env`.
///
/// A pending Java exception is turned into a caught one, so the thread is left
/// clean and the failure can be classified.
fn run<T>(call: impl FnOnce(&mut Env) -> Result<T, Error>) -> Result<T, Run> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() || context.context().is_null() {
        return Err(Run::NoContext);
    }

    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    let vm = unsafe { JavaVM::from_raw(vm_ptr) };
    vm.attach_current_thread(|env| {
        call(env).map_err(|error| match error {
            Error::JavaException => env.exception_catch().err().unwrap_or(Error::JavaException),
            other => other,
        })
    })
    .map_err(Run::Java)
}
