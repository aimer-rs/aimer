//! Passkeys through Android's Credential Manager.
//!
//! Credential Manager reports through a Java callback interface, so the Android
//! half is Kotlin: `com.aimer.PasskeyBridge` (see the crate's `android/`
//! directory), which the application ships. Rust sends it a ceremony with a
//! token and a WebAuthn JSON request, and the bridge calls back into
//! [`native_complete`] on Android's UI thread with the outcome. That completes a
//! Venus [`Completion`], which the returned future awaits, so the result is
//! applied in the frame phase of the awaiting task.
//!
//! The Android context the runtime holds (`ndk-context`) is the `Application`,
//! not an `Activity`, so the bridge tracks the foreground activity itself; see
//! `PasskeyBridge.attach`.
//!
//! Dropping a pending future tells the bridge to cancel the system sheet.

use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::task::{Context, Poll};

use aimer_venus::{Completer, Completion, completion};
use jni::errors::Error;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::sys::jlong;
use jni::{Env, EnvUnowned, JavaVM, NativeMethod, jni_sig, jni_str};

use crate::android_json;
use crate::error::{PasskeyCapability, PasskeyError, PasskeyResult};
use crate::passkeys::{PasskeyFuture, Passkeys, ready};
use crate::request::{CreateRequest, GetRequest};
use crate::response::{Assertion, Registration};
use crate::status::from_android;

/// The bridge class, by the name the class loader knows it.
const BRIDGE_CLASS: &str = "com.aimer.PasskeyBridge";
/// The status the bridge reports on success.
const OK: &str = "ok";

/// Passkeys through Credential Manager: Google Password Manager and any other
/// passkey provider the user enabled.
///
/// The application must include `com.aimer.PasskeyBridge` and the
/// `androidx.credentials` libraries, call `PasskeyBridge.attach(activity)` from
/// its activity, and be listed in the relying party's
/// `/.well-known/assetlinks.json`. Without the bridge a ceremony fails with
/// [`PasskeyCapability::Bridge`] unavailable, without an attached or foreground
/// activity with [`PasskeyCapability::Window`] unavailable, and without the link
/// with [`PasskeyError::NotAssociated`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CredentialManagerPasskeys;

impl CredentialManagerPasskeys {
    /// Creates the adapter.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Passkeys for CredentialManagerPasskeys {
    fn is_available(&self) -> PasskeyFuture<bool> {
        ready(
            run(|env| {
                let class = bridge(env)?;
                env.call_static_method(class, jni_str!("isAvailable"), jni_sig!("()Z"), &[])?
                    .z()
            })
            .map_err(Run::into_error),
        )
    }

    fn create(&self, request: CreateRequest) -> PasskeyFuture<Registration> {
        if let Err(error) = request.validate() {
            return ready(Err(error));
        }
        match begin(
            jni_str!("create"),
            &android_json::creation_options(&request),
            android_json::parse_registration,
        ) {
            Ok(ceremony) => Box::pin(ceremony),
            Err(error) => ready(Err(error)),
        }
    }

    fn get(&self, request: GetRequest) -> PasskeyFuture<Assertion> {
        if let Err(error) = request.validate() {
            return ready(Err(error));
        }
        match begin(
            jni_str!("get"),
            &android_json::request_options(&request),
            android_json::parse_assertion,
        ) {
            Ok(ceremony) => Box::pin(ceremony),
            Err(error) => ready(Err(error)),
        }
    }
}

/// What the bridge reports: [`OK`] and the response JSON, or an error kind and
/// the exception's message.
struct Reply {
    status: String,
    payload: String,
}

/// Ceremonies the bridge has not answered yet, by token.
static PENDING: LazyLock<Mutex<HashMap<i64, Completer<Reply>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_TOKEN: AtomicI64 = AtomicI64::new(1);

fn pending() -> MutexGuard<'static, HashMap<i64, Completer<Reply>>> {
    // Nothing runs under the lock but map operations, so a poisoned map is still
    // consistent.
    PENDING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A ceremony in flight. Dropping it before it finishes cancels the sheet.
struct Ceremony<T> {
    token: i64,
    completion: Completion<Reply>,
    parse: fn(&str) -> Option<T>,
    finished: bool,
}

impl<T> Future for Ceremony<T> {
    type Output = PasskeyResult<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let ceremony = &mut *self;
        match Pin::new(&mut ceremony.completion).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(reply) => {
                ceremony.finished = true;
                Poll::Ready(match reply {
                    Ok(Reply { status, payload }) if status == OK => {
                        (ceremony.parse)(&payload).ok_or(PasskeyError::Failed)
                    }
                    Ok(Reply { status, .. }) => Err(from_android(&status)),
                    Err(_) => Err(PasskeyError::Interrupted),
                })
            }
        }
    }
}

impl<T> Drop for Ceremony<T> {
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
            env.call_static_method(
                class,
                jni_str!("cancel"),
                jni_sig!("(J)V"),
                &[JValue::Long(token)],
            )
            .map(|_| ())
        });
    }
}

/// Hands a ceremony to the bridge: `method` is `create` or `get`.
fn begin<T>(
    method: &'static jni::strings::JNIStr,
    request_json: &str,
    parse: fn(&str) -> Option<T>,
) -> PasskeyResult<Ceremony<T>> {
    let (completer, completion) = completion();
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    pending().insert(token, completer);

    let started = run(|env| {
        let class = bridge(env)?;
        let json = env.new_string(request_json)?;
        env.call_static_method(
            class,
            method,
            jni_sig!("(JLjava/lang/String;)V"),
            &[JValue::Long(token), JValue::Object(&json)],
        )
        .map(|_| ())
    });
    if let Err(error) = started {
        pending().remove(&token);
        return Err(error.into_error());
    }
    Ok(Ceremony {
        token,
        completion,
        parse,
        finished: false,
    })
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
    // the bridge start tracking the foreground activity.
    env.call_static_method(
        &class,
        jni_str!("init"),
        jni_sig!("(Landroid/content/Context;)V"),
        &[JValue::Object(&application)],
    )?;
    let global = env.new_global_ref(class)?;
    Ok(BRIDGE.get_or_init(|| global))
}

/// Called by `PasskeyBridge` on Android's UI thread when a ceremony ends.
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
        // A token with no entry belongs to a ceremony that was dropped.
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
    fn into_error(self) -> PasskeyError {
        match self {
            Self::NoContext => PasskeyError::Unavailable {
                capability: PasskeyCapability::Runtime,
            },
            Self::Java(Error::CaughtJavaException { name, .. })
                if name.ends_with("ClassNotFoundException")
                    || name.ends_with("NoClassDefFoundError") =>
            {
                PasskeyError::Unavailable {
                    capability: PasskeyCapability::Bridge,
                }
            }
            Self::Java(_) => PasskeyError::Failed,
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
