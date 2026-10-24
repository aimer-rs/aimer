//! Secure storage for Aimer applications.
//!
//! `aimer_vault` keeps credentials, tokens, and keys. It is the counterpart of
//! `aimer_storage`, with the same `(namespace, key)` shape and the same Venus
//! driven futures, but where storage writes files, a vault hands each secret to
//! the platform's secret store. Never put a password, a private key, or an
//! authentication token in ordinary storage: it sits in plaintext on disk.
//!
//! The core is a small [`Vault`] trait over `(namespace, key) -> secret`, with
//! typed [`VaultError`]s. Secrets are [`SecretBytes`], which wipe themselves on
//! drop and never print, and are at most [`MAX_SECRET_BYTES`] long. Adapters:
//!
//! | Adapter | Keeps secrets in | Where |
//! |---|---|---|
//! | `KeychainVault` | the Keychain | macOS and iOS |
//! | `CredentialManagerVault` | Windows Credential Manager | Windows |
//! | `SecretServiceVault` | the Secret Service (GNOME Keyring, KWallet) | Linux |
//! | `KeystoreVault` | files encrypted by an Android Keystore key | Android |
//! | [`MemoryVault`] | process memory, **not** secure | everywhere; tests |
//!
//! [`PlatformVault`] names the secure one for the current target, and
//! `PlatformVault::for_app("com.example.app")` opens an application's own
//! vault. On a target with no secret store (the browser) it still opens, and
//! every operation reports [`VaultError::Unavailable`]: a secret is never
//! silently kept somewhere weaker. So does a Linux system with no Secret
//! Service running.
//!
//! # Venus-driven
//!
//! Keychain calls block on a system daemon, so operations leave the UI thread
//! through Venus's offload pool and come back as a wake in the awaiting task's
//! frame phase, in the order they were issued. The crate has no threads,
//! channels, or runtime of its own.
//!
//! # Examples
//!
//! ```no_run
//! use aimer_vault::{PlatformVault, Vault};
//!
//! async fn remember(token: &[u8]) -> aimer_vault::VaultResult<()> {
//!     let vault = PlatformVault::for_app("com.example.app")?;
//!     vault.write("accounts", "api-token", token).await?;
//!
//!     if let Some(token) = vault.read("accounts", "api-token").await? {
//!         // Hand the bytes to the code that needs them, and let `token` drop:
//!         // dropping wipes them.
//!         let _ = token.expose();
//!     }
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]

mod access;
mod error;
mod key;
mod keystore_blob;
mod keystore_files;
mod memory;
mod secret;
mod vault;

// Backends that make blocking calls share one ordered offload queue.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
))]
mod lane;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod keychain;
#[cfg(target_os = "windows")]
mod credential_manager;
#[cfg(target_os = "linux")]
mod secret_service;
#[cfg(target_os = "android")]
mod keystore;
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod unavailable;

pub use crate::access::VaultAccess;
pub use crate::error::{VaultCapability, VaultError, VaultResult};
pub use crate::key::MAX_SECRET_BYTES;
pub use crate::memory::MemoryVault;
pub use crate::secret::SecretBytes;
pub use crate::vault::{Vault, VaultFuture};

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use crate::keychain::KeychainVault;
#[cfg(target_os = "windows")]
pub use crate::credential_manager::CredentialManagerVault;
#[cfg(target_os = "linux")]
pub use crate::secret_service::SecretServiceVault;
#[cfg(target_os = "android")]
pub use crate::keystore::KeystoreVault;
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
pub use crate::unavailable::UnavailableVault;

/// The secure adapter for the current target: `KeychainVault` on Apple
/// platforms, `CredentialManagerVault` on Windows, `SecretServiceVault` on
/// Linux, `KeystoreVault` on Android, and `UnavailableVault` (which reports every
/// operation as [`VaultError::Unavailable`]) where there is no secret store.
///
/// All of them open an application's vault with `for_app(app_id)` and take a
/// [`VaultAccess`] with `with_access`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub type PlatformVault = KeychainVault;

/// The secure adapter for the current target: `KeychainVault` on Apple
/// platforms, `CredentialManagerVault` on Windows, `SecretServiceVault` on
/// Linux, `KeystoreVault` on Android, and `UnavailableVault` (which reports every
/// operation as [`VaultError::Unavailable`]) where there is no secret store.
///
/// All of them open an application's vault with `for_app(app_id)` and take a
/// [`VaultAccess`] with `with_access`.
#[cfg(target_os = "windows")]
pub type PlatformVault = CredentialManagerVault;

/// The secure adapter for the current target: `KeychainVault` on Apple
/// platforms, `CredentialManagerVault` on Windows, `SecretServiceVault` on
/// Linux, `KeystoreVault` on Android, and `UnavailableVault` (which reports every
/// operation as [`VaultError::Unavailable`]) where there is no secret store.
///
/// All of them open an application's vault with `for_app(app_id)` and take a
/// [`VaultAccess`] with `with_access`.
#[cfg(target_os = "linux")]
pub type PlatformVault = SecretServiceVault;

/// The secure adapter for the current target: `KeychainVault` on Apple
/// platforms, `CredentialManagerVault` on Windows, `SecretServiceVault` on
/// Linux, `KeystoreVault` on Android, and `UnavailableVault` (which reports every
/// operation as [`VaultError::Unavailable`]) where there is no secret store.
///
/// All of them open an application's vault with `for_app(app_id)` and take a
/// [`VaultAccess`] with `with_access`.
#[cfg(target_os = "android")]
pub type PlatformVault = KeystoreVault;

/// The secure adapter for the current target: `KeychainVault` on Apple
/// platforms, `CredentialManagerVault` on Windows, `SecretServiceVault` on
/// Linux, `KeystoreVault` on Android, and `UnavailableVault` (which reports every
/// operation as [`VaultError::Unavailable`]) where there is no secret store.
///
/// All of them open an application's vault with `for_app(app_id)` and take a
/// [`VaultAccess`] with `with_access`.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
pub type PlatformVault = UnavailableVault;
