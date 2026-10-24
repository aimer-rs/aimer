//! Passkeys for Aimer applications.
//!
//! `aimer_passkey` signs users in and registers them with the passkey provider
//! the operating system already has: Apple's AuthenticationServices on macOS and
//! iOS, Credential Manager on Android, and Windows Hello on Windows. The
//! platform keeps the private key and guards it with the user's biometrics or
//! device PIN; the application only ever sees the signed statements that a
//! relying party's server verifies.
//!
//! A ceremony is the same two steps everywhere, shaped like WebAuthn:
//!
//! 1. [`Passkeys::create`] makes a passkey for an account and returns a
//!    [`Registration`] for the server to store.
//! 2. [`Passkeys::get`] proves possession later and returns an [`Assertion`] for
//!    the server to verify.
//!
//! The server issues the challenge and checks the result. This crate does not
//! talk to a server and does not parse CBOR or COSE: it carries bytes between
//! your server and the platform.
//!
//! | Adapter | Provider | Where |
//! |---|---|---|
//! | `AuthenticationServicesPasskeys` | iCloud Keychain and other Apple providers | macOS 13+, iOS 16+ |
//! | `CredentialManagerPasskeys` | Android Credential Manager | Android 9+ with Play services |
//! | `WindowsHelloPasskeys` | Windows Hello (`webauthn.dll`) | Windows 10 and 11 with WebAuthn API 4 |
//! | [`UnavailablePasskeys`] | none | Linux, the browser; any target, as a stand-in |
//!
//! [`PlatformPasskeys`] names the one for the current target. On a target with
//! no provider it still opens, and every ceremony reports
//! [`PasskeyError::Unavailable`], so one code path builds everywhere.
//!
//! # Setup the platform requires
//!
//! A passkey is bound to a domain, and the platform only lets an app use a
//! domain that vouches for it. Without this the ceremony fails with
//! [`PasskeyError::NotAssociated`] or [`PasskeyError::MissingEntitlement`].
//!
//! * **Apple:** add the Associated Domains entitlement
//!   (`com.apple.developer.associated-domains`) with `webcredentials:example.com`,
//!   sign the binary, and list the app in the domain's
//!   `/.well-known/apple-app-site-association` under `webcredentials`.
//! * **Android:** list the app's package and signing certificate in the domain's
//!   `/.well-known/assetlinks.json` with the
//!   `delegate_permission/common.get_login_creds` relation. Add the
//!   `androidx.credentials:credentials` and
//!   `androidx.credentials:credentials-play-services-auth` dependencies, copy the
//!   crate's `android/PasskeyBridge.kt` into the app's `com.aimer` package, and
//!   call `PasskeyBridge.attach(this)` from the activity's `onCreate`:
//!   Credential Manager presents over an `Activity`, which Rust cannot supply.
//! * **Windows:** nothing is registered with the domain. The ceremony signs the
//!   origin `https://<relying party id>`, which the server must accept.
//!
//! # Venus-driven
//!
//! Operations return futures meant for a Venus task on the UI thread, and the
//! result is applied in the frame phase the task belongs to. Apple and Android
//! resolve from a system callback; Windows blocks, so it runs on Venus's offload
//! pool. The crate has no threads, channels, or runtime of its own.
//!
//! # Examples
//!
//! ```no_run
//! use aimer_passkey::{
//!     CreateRequest, GetRequest, PasskeyResult, PlatformPasskeys, Passkeys, RelyingParty, User,
//! };
//!
//! async fn sign_up(challenge: Vec<u8>, account: Vec<u8>) -> PasskeyResult<()> {
//!     let passkeys = PlatformPasskeys::new();
//!     if !passkeys.is_available().await? {
//!         return Ok(());
//!     }
//!
//!     let registration = passkeys
//!         .create(CreateRequest::new(
//!             RelyingParty::new("example.com", "Example"),
//!             User::new(account, "alice@example.com"),
//!             challenge,
//!         ))
//!         .await?;
//!     // Upload `registration.attestation_object()` and
//!     // `registration.client_data_json()` for the server to verify.
//!     let _ = registration.credential_id();
//!     Ok(())
//! }
//!
//! async fn sign_in(challenge: Vec<u8>) -> PasskeyResult<()> {
//!     let assertion = PlatformPasskeys::new()
//!         .get(GetRequest::new("example.com", challenge))
//!         .await?;
//!     let _ = (assertion.signature(), assertion.user_handle());
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]

#[cfg(any(target_os = "android", test))]
mod android_json;
mod base64url;
mod client_data;
mod error;
mod passkeys;
mod request;
mod response;
mod status;

#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(target_os = "windows")]
mod windows;
mod unavailable;

pub use crate::error::{InvalidRequest, PasskeyCapability, PasskeyError, PasskeyResult};
pub use crate::passkeys::{PasskeyFuture, Passkeys};
pub use crate::request::{
    Algorithm, Attestation, CreateRequest, CredentialId, GetRequest, MAX_CHALLENGE_BYTES,
    MAX_CREDENTIAL_ID_BYTES, MAX_CREDENTIALS, MAX_USER_ID_BYTES, RelyingParty, User,
    UserVerification,
};
pub use crate::response::{Assertion, Registration};

#[cfg(target_os = "android")]
pub use crate::android::CredentialManagerPasskeys;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use crate::apple::AuthenticationServicesPasskeys;
#[cfg(target_os = "windows")]
pub use crate::windows::WindowsHelloPasskeys;
pub use crate::unavailable::UnavailablePasskeys;

/// The adapter for the current target.
#[cfg(target_os = "android")]
pub type PlatformPasskeys = CredentialManagerPasskeys;

/// The adapter for the current target.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub type PlatformPasskeys = AuthenticationServicesPasskeys;

/// The adapter for the current target.
#[cfg(target_os = "windows")]
pub type PlatformPasskeys = WindowsHelloPasskeys;

/// The adapter for the current target.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "android"
)))]
pub type PlatformPasskeys = UnavailablePasskeys;
