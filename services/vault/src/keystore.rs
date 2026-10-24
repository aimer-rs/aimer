//! The Android Keystore adapter.
//!
//! The Keystore stores keys, not arbitrary secrets, so this adapter splits the
//! job: an AES-256-GCM key that never leaves the Keystore (and, on devices with
//! one, never leaves the secure hardware) encrypts each secret, and the
//! encrypted blob is an ordinary file in the application's private storage.
//! Stealing the file gets an attacker nothing without the key.
//!
//! There is no Java shim to call into, so the whole conversation happens in
//! JNI: the activity and the `JavaVM` come from `ndk-context`, which
//! `android-activity` populates on start-up, and the calling thread is attached
//! for the duration of one operation.

mod java;

use std::path::PathBuf;
use std::sync::Arc;

use crate::access::VaultAccess;
use crate::error::VaultResult;
use crate::key::{check_secret, validate, validate_component};
use crate::lane::Lane;
use crate::secret::SecretBytes;
use crate::vault::{Vault, VaultFuture, ready};

use crate::keystore_blob::Blob;
use crate::keystore_files::{self as files, Layout};

/// A vault backed by the Android Keystore.
///
/// Every secret is encrypted with AES-256-GCM under a Keystore key and written
/// to a file in the application's *no-backup* directory, so Android's cloud
/// backup never copies ciphertext that no other device could decrypt. The
/// ciphertext is bound to its `(application, namespace, key)` name, so a file
/// moved or copied onto another name fails to decrypt instead of answering for
/// a different secret.
///
/// [`VaultAccess`] picks the Keystore key: `WhenUnlocked` uses a key that
/// refuses to work while the device is locked (Android 9 and later; earlier
/// releases fall back to a plain Keystore key), `AfterFirstUnlock` a key that
/// does not. A secret is read with the key it was written with, so changing
/// the level affects only later writes.
///
/// A secret whose key has disappeared (the user cleared the lock screen, which
/// invalidates some keys, or the data was restored without the Keystore) can
/// never be decrypted again. Reading it removes the unusable file and reports
/// `None`, which is what an application should treat as "ask the user to sign in
/// again".
///
/// Calls run on Venus's offload pool, in call order, and a Venus runtime must be
/// installed on the calling thread: an operation made without one resolves to
/// [`VaultError::Unavailable`](crate::VaultError::Unavailable) with
/// [`VaultCapability::Runtime`](crate::VaultCapability::Runtime). The
/// application's activity must also be running, because the Keystore and the
/// data directory are reached through its `Context`.
///
/// Clones share one ordering queue.
#[derive(Clone)]
pub struct KeystoreVault {
    app_id: Arc<str>,
    access: VaultAccess,
    lane: Lane,
    /// Where secrets live, when it was chosen explicitly. `None` resolves the
    /// application's no-backup directory on first use.
    root: Option<Arc<PathBuf>>,
}

impl KeystoreVault {
    /// Opens the vault an application keeps its secrets in.
    ///
    /// The same `app_id` finds the same secrets on every launch, so use a stable
    /// reverse-DNS name such as `com.example.app`. Nothing is contacted until
    /// the first operation.
    ///
    /// # Errors
    ///
    /// [`VaultError::InvalidKey`](crate::VaultError::InvalidKey) when `app_id`
    /// is not a valid name.
    pub fn for_app(app_id: &str) -> VaultResult<Self> {
        validate_component(app_id)?;
        Ok(Self {
            app_id: Arc::from(app_id),
            access: VaultAccess::default(),
            lane: Lane::default(),
            root: None,
        })
    }

    /// Sets when newly written secrets may be read. See [`VaultAccess`].
    #[inline]
    pub fn with_access(mut self, access: VaultAccess) -> Self {
        self.access = access;
        self
    }

    /// Returns the access level new secrets are written with.
    #[inline]
    pub fn access(&self) -> VaultAccess {
        self.access
    }

    /// Stores the encrypted files under `root` instead of the application's
    /// no-backup directory.
    ///
    /// For an application that already knows its data directory and for tests;
    /// the directory must be private to the application.
    #[inline]
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(Arc::new(root.into()));
        self
    }

    fn job(&self, namespace: &str, key: Option<&str>) -> Job {
        Job {
            app_id: Arc::clone(&self.app_id),
            namespace: namespace.to_owned(),
            key: key.map(str::to_owned),
            access: self.access,
            root: self.root.clone(),
        }
    }
}

/// Everything a worker needs to find one secret, or one namespace.
struct Job {
    app_id: Arc<str>,
    namespace: String,
    key: Option<String>,
    access: VaultAccess,
    root: Option<Arc<PathBuf>>,
}

impl Job {
    fn layout(&self) -> VaultResult<Layout> {
        let root = match &self.root {
            Some(root) => (**root).clone(),
            None => java::no_backup_dir()?,
        };
        Ok(Layout::new(root, &self.app_id))
    }

    fn key(&self) -> &str {
        self.key.as_deref().expect("a per-secret job names its key")
    }

    /// The name the ciphertext is bound to.
    fn aad(&self) -> String {
        format!("aimer.vault/{}/{}/{}", self.app_id, self.namespace, self.key())
    }
}

impl Vault for KeystoreVault {
    fn read(&self, namespace: &str, key: &str) -> VaultFuture<Option<SecretBytes>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let job = self.job(namespace, Some(key));
        self.lane.submit(move || {
            let layout = job.layout()?;
            let path = layout.secret(&job.namespace, job.key());
            let Some(bytes) = files::read(&path)? else {
                return Ok(None);
            };
            let Some(blob) = Blob::decode(&bytes) else {
                // Not a blob this crate wrote: unusable, so drop it like any other
                // secret that can no longer be decrypted.
                files::remove(&path)?;
                return Ok(None);
            };
            match java::decrypt(&job.app_id, &blob, job.aad().as_bytes())? {
                Some(secret) => Ok(Some(SecretBytes::new(secret))),
                None => {
                    files::remove(&path)?;
                    Ok(None)
                }
            }
        })
    }

    fn write(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultFuture<()> {
        if let Err(error) = validate(namespace, key).and_then(|()| check_secret(secret.len())) {
            return ready(Err(error));
        }
        let job = self.job(namespace, Some(key));
        // The worker needs an owned copy; it is wiped when the job finishes.
        let secret = SecretBytes::from_slice(secret);
        self.lane.submit(move || {
            let layout = job.layout()?;
            let blob = java::encrypt(&job.app_id, job.access, job.aad().as_bytes(), secret.expose())?;
            files::write(&layout.secret(&job.namespace, job.key()), &blob.encode())
        })
    }

    fn remove(&self, namespace: &str, key: &str) -> VaultFuture<bool> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let job = self.job(namespace, Some(key));
        self.lane.submit(move || {
            let layout = job.layout()?;
            files::remove(&layout.secret(&job.namespace, job.key()))
        })
    }

    fn clear(&self, namespace: &str) -> VaultFuture<()> {
        if let Err(error) = validate_component(namespace) {
            return ready(Err(error));
        }
        let job = self.job(namespace, None);
        self.lane.submit(move || {
            let layout = job.layout()?;
            files::remove_namespace(&layout.namespace(&job.namespace))
        })
    }
}
