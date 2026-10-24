//! The adapter contract.

use std::future::Future;
use std::pin::Pin;

use crate::error::PasskeyResult;
use crate::request::{CreateRequest, GetRequest};
use crate::response::{Assertion, Registration};

/// A boxed asynchronous passkey operation.
///
/// Deliberately not `Send`: it is awaited by a Venus task on the UI thread,
/// which keeps its `Rc` captures, and Apple's authorization controller may only
/// be driven from the main thread in any case.
pub type PasskeyFuture<T> = Pin<Box<dyn Future<Output = PasskeyResult<T>> + 'static>>;

/// A future that is already resolved.
pub(crate) fn ready<T: 'static>(result: PasskeyResult<T>) -> PasskeyFuture<T> {
    Box::pin(std::future::ready(result))
}

/// The operating system's passkey provider.
///
/// A passkey is a key pair the platform keeps and guards behind the user's
/// biometrics or device PIN. The private key never reaches the application: it
/// only receives the signed statements a relying party's server verifies. Both
/// operations show the system's own sheet, so each needs a user who is looking
/// at the app and is cancelled by dismissing the sheet.
///
/// Dropping a pending operation's future cancels it where the platform allows.
pub trait Passkeys {
    /// Reports whether this device can create and use passkeys.
    ///
    /// `false` is a fact about the device or OS version (no platform
    /// authenticator, nothing enrolled, an OS that predates passkeys), not about
    /// the application's configuration, which only a real ceremony can test.
    fn is_available(&self) -> PasskeyFuture<bool>;

    /// Creates a passkey for a user at a relying party and returns what the
    /// server needs to register it.
    ///
    /// # Errors
    ///
    /// * [`PasskeyError::InvalidRequest`](crate::PasskeyError::InvalidRequest) for a malformed request.
    /// * [`PasskeyError::Cancelled`](crate::PasskeyError::Cancelled) if the user dismisses the sheet.
    /// * [`PasskeyError::AlreadyRegistered`](crate::PasskeyError::AlreadyRegistered) if an excluded credential exists here.
    /// * [`PasskeyError::NotAssociated`](crate::PasskeyError::NotAssociated) or
    ///   [`PasskeyError::MissingEntitlement`](crate::PasskeyError::MissingEntitlement)
    ///   when the app is not set up for the domain.
    /// * Any platform error from the adapter.
    fn create(&self, request: CreateRequest) -> PasskeyFuture<Registration>;

    /// Signs in with an existing passkey and returns the signed assertion.
    ///
    /// # Errors
    ///
    /// * [`PasskeyError::InvalidRequest`](crate::PasskeyError::InvalidRequest) for a malformed request.
    /// * [`PasskeyError::Cancelled`](crate::PasskeyError::Cancelled) if the user dismisses the sheet.
    /// * [`PasskeyError::NoCredential`](crate::PasskeyError::NoCredential) if no passkey matches.
    /// * [`PasskeyError::NotAssociated`](crate::PasskeyError::NotAssociated) or
    ///   [`PasskeyError::MissingEntitlement`](crate::PasskeyError::MissingEntitlement)
    ///   when the app is not set up for the domain.
    /// * Any platform error from the adapter.
    fn get(&self, request: GetRequest) -> PasskeyFuture<Assertion>;
}
