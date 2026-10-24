//! The Credential Manager adapter for Windows.

use std::iter;
use std::ptr;
use std::sync::Arc;

use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredEnumerateW,
    CredFree, CredReadW, CredWriteW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::access::VaultAccess;
use crate::error::{VaultError, VaultResult};
use crate::key::{check_secret, validate, validate_component};
use crate::lane::Lane;
use crate::secret::SecretBytes;
use crate::vault::{Vault, VaultFuture, ready};

// Values from <winerror.h>.
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_NO_SUCH_LOGON_SESSION: u32 = 1312;

/// Every credential this crate writes starts with this, so they are easy to
/// recognise in Windows' own credential list.
const TARGET_PREFIX: &str = "aimer.vault/";

/// A vault backed by Windows Credential Manager.
///
/// Each secret is a generic credential whose target name is
/// `"aimer.vault/{app_id}/{namespace}/{key}"`. Windows encrypts the blob for the
/// signed-in user (DPAPI) and persists it for this machine only: it does not
/// roam to the user's other PCs. Any process running as the same user can read
/// it, which is the protection Credential Manager offers.
///
/// A credential holds at most [`MAX_SECRET_BYTES`](crate::MAX_SECRET_BYTES).
///
/// Calls run on Venus's offload pool, in call order, and a Venus runtime must be
/// installed on the calling thread: an operation made without one resolves to
/// [`VaultError::Unavailable`] with
/// [`VaultCapability::Runtime`](crate::VaultCapability::Runtime).
///
/// Clones share one ordering queue.
#[derive(Clone)]
pub struct CredentialManagerVault {
    app_id: Arc<str>,
    access: VaultAccess,
    lane: Lane,
}

impl CredentialManagerVault {
    /// Opens the vault an application keeps its secrets in.
    ///
    /// The same `app_id` finds the same secrets on every launch, so use a stable
    /// reverse-DNS name such as `com.example.app`.
    ///
    /// # Errors
    ///
    /// [`VaultError::InvalidKey`] when `app_id` is not a valid name.
    pub fn for_app(app_id: &str) -> VaultResult<Self> {
        validate_component(app_id)?;
        Ok(Self {
            app_id: Arc::from(app_id),
            access: VaultAccess::default(),
            lane: Lane::default(),
        })
    }

    /// Records the access level asked for. See [`VaultAccess`].
    ///
    /// Credential Manager has no accessibility classes, so this changes nothing
    /// here; it exists so code written for every target compiles unchanged.
    #[inline]
    pub fn with_access(mut self, access: VaultAccess) -> Self {
        self.access = access;
        self
    }

    /// Returns the access level that was asked for.
    #[inline]
    pub fn access(&self) -> VaultAccess {
        self.access
    }

    fn target(&self, namespace: &str, key: &str) -> String {
        format!("{TARGET_PREFIX}{}/{namespace}/{key}", self.app_id)
    }

    /// The target-name prefix shared by every secret of `namespace`.
    fn namespace_prefix(&self, namespace: &str) -> String {
        format!("{TARGET_PREFIX}{}/{namespace}/", self.app_id)
    }
}

impl Vault for CredentialManagerVault {
    fn read(&self, namespace: &str, key: &str) -> VaultFuture<Option<SecretBytes>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let target = self.target(namespace, key);
        self.lane.submit(move || read_item(&target))
    }

    fn write(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultFuture<()> {
        if let Err(error) = validate(namespace, key).and_then(|()| check_secret(secret.len())) {
            return ready(Err(error));
        }
        let target = self.target(namespace, key);
        // The worker needs an owned copy; it is wiped when the job finishes.
        let secret = SecretBytes::from_slice(secret);
        self.lane
            .submit(move || write_item(&target, secret.expose()))
    }

    fn remove(&self, namespace: &str, key: &str) -> VaultFuture<bool> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let target = self.target(namespace, key);
        self.lane.submit(move || remove_item(&target))
    }

    fn clear(&self, namespace: &str) -> VaultFuture<()> {
        if let Err(error) = validate_component(namespace) {
            return ready(Err(error));
        }
        let prefix = self.namespace_prefix(namespace);
        self.lane.submit(move || clear_prefix(&prefix))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(iter::once(0)).collect()
}

fn read_item(target: &str) -> VaultResult<Option<SecretBytes>> {
    let target = wide(target);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and outlives the call; on success
    // `credential` receives a buffer that is released with `CredFree` below.
    let read = unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            None,
            &mut credential,
        )
    };
    match read {
        Ok(()) => {}
        Err(error) if win32_code(&error) == ERROR_NOT_FOUND => return Ok(None),
        Err(error) => return Err(map_error(&error)),
    }
    // SAFETY: `CredReadW` succeeded, so `credential` points at a live
    // CREDENTIALW whose blob is `CredentialBlobSize` bytes (or null when empty).
    // The bytes are copied out before the buffer is freed.
    let secret = unsafe {
        let credential_ref = &*credential;
        let bytes = if credential_ref.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(
                credential_ref.CredentialBlob,
                credential_ref.CredentialBlobSize as usize,
            )
            .to_vec()
        };
        CredFree(credential.cast());
        bytes
    };
    Ok(Some(SecretBytes::new(secret)))
}

fn write_item(target: &str, secret: &[u8]) -> VaultResult<()> {
    let mut target = wide(target);
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        // `check_secret` bounded the length far below `u32::MAX`.
        CredentialBlobSize: secret.len() as u32,
        // CredWriteW only reads the blob, so the cast away from `const` is sound.
        CredentialBlob: secret.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: `credential` and the buffers it points at live across the call.
    unsafe { CredWriteW(&credential, 0) }.map_err(|error| map_error(&error))
}

fn remove_item(target: &str) -> VaultResult<bool> {
    let target = wide(target);
    // SAFETY: `target` is NUL-terminated and outlives the call.
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(true),
        Err(error) if win32_code(&error) == ERROR_NOT_FOUND => Ok(false),
        Err(error) => Err(map_error(&error)),
    }
}

/// Deletes every credential whose target name starts with `prefix`.
fn clear_prefix(prefix: &str) -> VaultResult<()> {
    for target in enumerate(prefix)? {
        remove_item(&target)?;
    }
    Ok(())
}

/// The target names that start with `prefix`.
///
/// `CredEnumerateW` takes a filter with a trailing `*` wildcard, but an
/// application id may itself contain `*`, which would widen the match into
/// another application's credentials. Every name is therefore checked against
/// the exact prefix before it is returned.
fn enumerate(prefix: &str) -> VaultResult<Vec<String>> {
    let filter = wide(&format!("{prefix}*"));
    let mut count = 0u32;
    let mut credentials: *mut *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `filter` is NUL-terminated and outlives the call; on success
    // `credentials` receives an array of `count` pointers, freed below.
    let listed =
        unsafe { CredEnumerateW(PCWSTR(filter.as_ptr()), None, &mut count, &mut credentials) };
    match listed {
        Ok(()) => {}
        Err(error) if win32_code(&error) == ERROR_NOT_FOUND => return Ok(Vec::new()),
        Err(error) => return Err(map_error(&error)),
    }
    // SAFETY: `CredEnumerateW` succeeded, so `credentials` holds `count` valid
    // CREDENTIALW pointers. The names are copied out before the array is freed.
    let targets = unsafe {
        let targets = std::slice::from_raw_parts(credentials, count as usize)
            .iter()
            .filter_map(|&credential| (*credential).TargetName.to_string().ok())
            .filter(|target| target.starts_with(prefix))
            .collect();
        CredFree(credentials.cast());
        targets
    };
    Ok(targets)
}

/// The Win32 error code inside an error, whether it came as an `HRESULT` or as
/// `HRESULT_FROM_WIN32`.
fn win32_code(error: &windows::core::Error) -> u32 {
    let code = error.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        code & 0xFFFF
    } else {
        code
    }
}

fn map_error(error: &windows::core::Error) -> VaultError {
    map_code(win32_code(error))
}

/// Translates a Win32 error code into the vault's vocabulary.
fn map_code(code: u32) -> VaultError {
    match code {
        ERROR_ACCESS_DENIED => VaultError::Denied,
        // The user's logon session (and so their credential store) is not there.
        ERROR_NO_SUCH_LOGON_SESSION => VaultError::Locked,
        code => VaultError::Backend { code: code as i32 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_map_to_typed_errors() {
        assert_eq!(map_code(5), VaultError::Denied);
        assert_eq!(map_code(1312), VaultError::Locked);
        assert_eq!(map_code(87), VaultError::Backend { code: 87 });
    }

    #[test]
    fn targets_are_scoped_to_the_application_and_namespace() {
        let vault = CredentialManagerVault::for_app("com.example.app").unwrap();
        assert_eq!(
            vault.target("tokens", "api"),
            "aimer.vault/com.example.app/tokens/api"
        );
        assert!(
            vault
                .target("tokens", "api")
                .starts_with(&vault.namespace_prefix("tokens"))
        );
        // A namespace that merely starts with another's name is not inside it.
        assert!(
            !vault
                .target("tokens-extra", "api")
                .starts_with(&vault.namespace_prefix("tokens"))
        );
    }

    #[test]
    fn wide_strings_are_nul_terminated() {
        assert_eq!(wide("ab"), [b'a' as u16, b'b' as u16, 0]);
    }

    #[test]
    fn an_invalid_application_id_is_rejected() {
        assert!(matches!(
            CredentialManagerVault::for_app("a/b"),
            Err(VaultError::InvalidKey)
        ));
    }
}
