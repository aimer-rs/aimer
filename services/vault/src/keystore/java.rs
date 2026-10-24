//! The JNI half of the Keystore adapter: key management and AES-GCM.

use std::path::PathBuf;

use jni::errors::Error;
use jni::objects::{JByteArray, JObject, JString, JValue};
use jni::{Env, JavaVM, jni_sig, jni_str};

use crate::access::VaultAccess;
use crate::error::{VaultCapability, VaultError, VaultResult};
use crate::keystore_blob::Blob;

/// `Cipher.ENCRYPT_MODE` and `Cipher.DECRYPT_MODE`.
const ENCRYPT_MODE: i32 = 1;
const DECRYPT_MODE: i32 = 2;
/// `KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT`.
const PURPOSES: i32 = 1 | 2;
/// Bits in the GCM authentication tag.
const TAG_BITS: i32 = 128;
const KEY_BITS: i32 = 256;
/// `Build.VERSION_CODES.P`, where `setUnlockedDeviceRequired` appeared.
const SDK_P: i32 = 28;

/// Why a Java call failed, after exceptions have been caught and cleared.
enum Fault {
    /// The device is locked and the key refuses to work.
    Locked,
    /// The key or the ciphertext can no longer be used: the key is gone or was
    /// invalidated, or the data does not authenticate under it.
    Unusable,
    Other,
}

enum Run {
    /// No JVM or no running activity to talk to.
    NoContext,
    Java(Error),
}

impl Run {
    fn fault(&self) -> Fault {
        let Run::Java(Error::CaughtJavaException { name, msg, .. }) = self else {
            return Fault::Other;
        };
        let class = name.rsplit(['.', '$']).next().unwrap_or(name);
        match class {
            "UserNotAuthenticatedException" => Fault::Locked,
            "KeyStoreException" if msg.contains("locked") || msg.contains("authenticated") => {
                Fault::Locked
            }
            "AEADBadTagException"
            | "BadPaddingException"
            | "IllegalBlockSizeException"
            | "InvalidKeyException"
            | "KeyPermanentlyInvalidatedException"
            | "UnrecoverableKeyException" => Fault::Unusable,
            _ => Fault::Other,
        }
    }

    fn into_vault_error(self) -> VaultError {
        match self.fault() {
            _ if matches!(self, Run::NoContext) => VaultError::Unavailable {
                capability: VaultCapability::PlatformSecretStore,
            },
            Fault::Locked => VaultError::Locked,
            Fault::Unusable | Fault::Other => VaultError::Failed,
        }
    }
}

impl From<Error> for Run {
    fn from(error: Error) -> Self {
        Run::Java(error)
    }
}

/// Attaches the current thread to the JVM and runs `call` with a live `Env` and
/// the running activity (the application `Context`).
///
/// A pending Java exception is turned into a caught one, so the thread is left
/// clean and the failure can be classified.
fn run<T>(call: impl FnOnce(&mut Env, &JObject<'_>) -> Result<T, Error>) -> Result<T, Run> {
    let context = ndk_context::android_context();
    let vm_ptr = context.vm() as *mut jni::sys::JavaVM;
    let activity_ptr = context.context() as jni::sys::jobject;
    if vm_ptr.is_null() || activity_ptr.is_null() {
        return Err(Run::NoContext);
    }

    // SAFETY: `ndk-context` hands out the `JavaVM` and the global activity
    // reference owned by the android-activity runtime, both valid for the life
    // of the process.
    let vm = unsafe { JavaVM::from_raw(vm_ptr) };
    vm.attach_current_thread(|env| {
        // SAFETY: as above, a global reference, so it needs no frame of its own.
        let activity = unsafe { JObject::from_raw(&*env, activity_ptr) };
        call(env, &activity).map_err(|error| match error {
            Error::JavaException => env.exception_catch().err().unwrap_or(Error::JavaException),
            other => other,
        })
    })
    .map_err(Run::Java)
}

/// The application's no-backup directory: private to the app, and excluded from
/// Android's cloud backup and device-to-device transfer.
pub(super) fn no_backup_dir() -> VaultResult<PathBuf> {
    run(|env, context| {
        let dir = env
            .call_method(
                context,
                jni_str!("getNoBackupFilesDir"),
                jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let path = env
            .call_method(
                &dir,
                jni_str!("getAbsolutePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let path = env.cast_local::<JString>(path)?;
        Ok(PathBuf::from(path.try_to_string(env)?))
    })
    .map_err(Run::into_vault_error)
}

/// The Keystore alias of the key used for `access`.
fn alias(app_id: &str, access: VaultAccess) -> String {
    let level = match access {
        VaultAccess::WhenUnlocked => "unlocked",
        VaultAccess::AfterFirstUnlock => "first-unlock",
    };
    format!("aimer.vault.{app_id}.{level}")
}

/// Encrypts `plaintext` under the key for `access`, creating the key if it does
/// not exist yet. `aad` is authenticated but not stored.
pub(super) fn encrypt(
    app_id: &str,
    access: VaultAccess,
    aad: &[u8],
    plaintext: &[u8],
) -> VaultResult<Blob> {
    let alias = alias(app_id, access);
    let sealed = match seal(&alias, access, aad, plaintext) {
        // A key that was invalidated since it was created (a changed lock screen,
        // for instance) is useless for new secrets too: replace it once.
        Err(error) if matches!(error.fault(), Fault::Unusable) => {
            run(|env, _| delete_key(env, &alias)).map_err(Run::into_vault_error)?;
            seal(&alias, access, aad, plaintext)
        }
        other => other,
    };
    let (iv, ciphertext) = sealed.map_err(Run::into_vault_error)?;
    Ok(Blob {
        access,
        iv,
        ciphertext,
    })
}

fn seal(
    alias: &str,
    access: VaultAccess,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), Run> {
    run(|env, _| {
        let key = match secret_key(env, alias)? {
            Some(key) => key,
            None => generate_key(env, alias, access)?,
        };
        let cipher = new_cipher(env)?;
        env.call_method(
            &cipher,
            jni_str!("init"),
            jni_sig!("(ILjava/security/Key;)V"),
            &[JValue::Int(ENCRYPT_MODE), JValue::Object(&key)],
        )?;
        update_aad(env, &cipher, aad)?;

        let input = env.byte_array_from_slice(plaintext)?;
        let output = env
            .call_method(
                &cipher,
                jni_str!("doFinal"),
                jni_sig!("([B)[B"),
                &[JValue::Object(&input)],
            )?
            .l()?;
        wipe_java(env, &input);
        let iv = env
            .call_method(&cipher, jni_str!("getIV"), jni_sig!("()[B"), &[])?
            .l()?;
        let output = env.cast_local::<JByteArray>(output)?;
        let iv = env.cast_local::<JByteArray>(iv)?;
        Ok((env.convert_byte_array(&iv)?, env.convert_byte_array(&output)?))
    })
}

/// Decrypts `blob`, or `None` when it can never be decrypted: its key is gone
/// or invalidated, or the bytes do not authenticate under `aad`.
pub(super) fn decrypt(app_id: &str, blob: &Blob, aad: &[u8]) -> VaultResult<Option<Vec<u8>>> {
    let alias = alias(app_id, blob.access);
    let opened = run(|env, _| {
        let Some(key) = secret_key(env, &alias)? else {
            return Ok(None);
        };
        let cipher = new_cipher(env)?;
        let iv = env.byte_array_from_slice(&blob.iv)?;
        let spec = env.new_object(
            jni_str!("javax/crypto/spec/GCMParameterSpec"),
            jni_sig!("(I[B)V"),
            &[JValue::Int(TAG_BITS), JValue::Object(&iv)],
        )?;
        env.call_method(
            &cipher,
            jni_str!("init"),
            jni_sig!("(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V"),
            &[
                JValue::Int(DECRYPT_MODE),
                JValue::Object(&key),
                JValue::Object(&spec),
            ],
        )?;
        update_aad(env, &cipher, aad)?;

        let input = env.byte_array_from_slice(&blob.ciphertext)?;
        let output = env
            .call_method(
                &cipher,
                jni_str!("doFinal"),
                jni_sig!("([B)[B"),
                &[JValue::Object(&input)],
            )?
            .l()?;
        let output = env.cast_local::<JByteArray>(output)?;
        let plaintext = env.convert_byte_array(&output)?;
        wipe_java(env, &output);
        Ok(Some(plaintext))
    });
    match opened {
        Ok(plaintext) => Ok(plaintext),
        Err(error) if matches!(error.fault(), Fault::Unusable) => Ok(None),
        Err(error) => Err(error.into_vault_error()),
    }
}

/// The Keystore's `AndroidKeyStore` instance, loaded.
fn key_store<'local>(env: &mut Env<'local>) -> Result<JObject<'local>, Error> {
    let provider = env.new_string("AndroidKeyStore")?;
    let store = env
        .call_static_method(
            jni_str!("java/security/KeyStore"),
            jni_str!("getInstance"),
            jni_sig!("(Ljava/lang/String;)Ljava/security/KeyStore;"),
            &[JValue::Object(&provider)],
        )?
        .l()?;
    env.call_method(
        &store,
        jni_str!("load"),
        jni_sig!("(Ljava/security/KeyStore$LoadStoreParameter;)V"),
        &[JValue::Object(&JObject::null())],
    )?;
    Ok(store)
}

/// The key under `alias`, or `None` if there is none.
fn secret_key<'local>(env: &mut Env<'local>, alias: &str) -> Result<Option<JObject<'local>>, Error> {
    let store = key_store(env)?;
    let alias = env.new_string(alias)?;
    let key = env
        .call_method(
            &store,
            jni_str!("getKey"),
            jni_sig!("(Ljava/lang/String;[C)Ljava/security/Key;"),
            &[JValue::Object(&alias), JValue::Object(&JObject::null())],
        )?
        .l()?;
    Ok((!key.is_null()).then_some(key))
}

fn delete_key(env: &mut Env<'_>, alias: &str) -> Result<(), Error> {
    let store = key_store(env)?;
    let alias = env.new_string(alias)?;
    env.call_method(
        &store,
        jni_str!("deleteEntry"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(&alias)],
    )?;
    Ok(())
}

/// Creates an AES-256-GCM key in the Keystore.
fn generate_key<'local>(
    env: &mut Env<'local>,
    alias: &str,
    access: VaultAccess,
) -> Result<JObject<'local>, Error> {
    let alias = env.new_string(alias)?;
    let builder = env.new_object(
        jni_str!("android/security/keystore/KeyGenParameterSpec$Builder"),
        jni_sig!("(Ljava/lang/String;I)V"),
        &[JValue::Object(&alias), JValue::Int(PURPOSES)],
    )?;
    let modes = string_array(env, "GCM")?;
    let paddings = string_array(env, "NoPadding")?;
    let builder_sig = jni_sig!("([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;");
    env.call_method(&builder, jni_str!("setBlockModes"), &builder_sig, &[JValue::Object(&modes)])?;
    env.call_method(
        &builder,
        jni_str!("setEncryptionPaddings"),
        &builder_sig,
        &[JValue::Object(&paddings)],
    )?;
    env.call_method(
        &builder,
        jni_str!("setKeySize"),
        jni_sig!("(I)Landroid/security/keystore/KeyGenParameterSpec$Builder;"),
        &[JValue::Int(KEY_BITS)],
    )?;
    if access == VaultAccess::WhenUnlocked && sdk_int(env)? >= SDK_P {
        env.call_method(
            &builder,
            jni_str!("setUnlockedDeviceRequired"),
            jni_sig!("(Z)Landroid/security/keystore/KeyGenParameterSpec$Builder;"),
            &[JValue::Bool(true)],
        )?;
    }
    let spec = env
        .call_method(
            &builder,
            jni_str!("build"),
            jni_sig!("()Landroid/security/keystore/KeyGenParameterSpec;"),
            &[],
        )?
        .l()?;

    let algorithm = env.new_string("AES")?;
    let provider = env.new_string("AndroidKeyStore")?;
    let generator = env
        .call_static_method(
            jni_str!("javax/crypto/KeyGenerator"),
            jni_str!("getInstance"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;"),
            &[JValue::Object(&algorithm), JValue::Object(&provider)],
        )?
        .l()?;
    env.call_method(
        &generator,
        jni_str!("init"),
        jni_sig!("(Ljava/security/spec/AlgorithmParameterSpec;)V"),
        &[JValue::Object(&spec)],
    )?;
    env.call_method(
        &generator,
        jni_str!("generateKey"),
        jni_sig!("()Ljavax/crypto/SecretKey;"),
        &[],
    )?
    .l()
}

fn sdk_int(env: &mut Env<'_>) -> Result<i32, Error> {
    env.get_static_field(
        jni_str!("android/os/Build$VERSION"),
        jni_str!("SDK_INT"),
        jni_sig!("I"),
    )?
    .i()
}

fn string_array<'local>(env: &mut Env<'local>, value: &str) -> Result<JObject<'local>, Error> {
    let value = env.new_string(value)?;
    let array = env.new_object_array(1, jni_str!("java/lang/String"), JObject::null())?;
    array.set_element(env, 0, &value)?;
    Ok(array.into())
}

fn new_cipher<'local>(env: &mut Env<'local>) -> Result<JObject<'local>, Error> {
    let transformation = env.new_string("AES/GCM/NoPadding")?;
    env.call_static_method(
        jni_str!("javax/crypto/Cipher"),
        jni_str!("getInstance"),
        jni_sig!("(Ljava/lang/String;)Ljavax/crypto/Cipher;"),
        &[JValue::Object(&transformation)],
    )?
    .l()
}

fn update_aad(env: &mut Env<'_>, cipher: &JObject<'_>, aad: &[u8]) -> Result<(), Error> {
    let aad = env.byte_array_from_slice(aad)?;
    env.call_method(
        cipher,
        jni_str!("updateAAD"),
        jni_sig!("([B)V"),
        &[JValue::Object(&aad)],
    )?;
    Ok(())
}

/// Overwrites a Java byte array that held a secret. The garbage collector may
/// have copied it already, so this narrows the exposure, it does not end it.
fn wipe_java(env: &mut Env<'_>, array: &JByteArray<'_>) {
    if let Ok(len) = array.len(env) {
        let _ = array.set_region(env, 0, &vec![0i8; len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_separate_applications_and_access_levels() {
        assert_ne!(
            alias("a", VaultAccess::WhenUnlocked),
            alias("a", VaultAccess::AfterFirstUnlock)
        );
        assert_ne!(
            alias("a", VaultAccess::WhenUnlocked),
            alias("b", VaultAccess::WhenUnlocked)
        );
    }
}
