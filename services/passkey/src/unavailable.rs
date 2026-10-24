//! The adapter for targets with no passkey provider.

use crate::error::{PasskeyCapability, PasskeyError};
use crate::passkeys::{PasskeyFuture, Passkeys, ready};
use crate::request::{CreateRequest, GetRequest};
use crate::response::{Assertion, Registration};

/// An adapter whose every ceremony fails with
/// [`PasskeyError::Unavailable`].
///
/// Linux and the browser have no operating-system passkey provider. This opens
/// anyway, so an application builds and runs everywhere and can hide its passkey
/// button after [`is_available`](Passkeys::is_available) says `false`. Requests
/// are validated first, so a malformed one reports the same error everywhere.
///
/// It exists on every target, so an application can also pick it deliberately,
/// for a build or a test that must not reach a real authenticator.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailablePasskeys;

impl UnavailablePasskeys {
    /// Creates the adapter.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

const UNAVAILABLE: PasskeyError = PasskeyError::Unavailable {
    capability: PasskeyCapability::PlatformAuthenticator,
};

impl Passkeys for UnavailablePasskeys {
    fn is_available(&self) -> PasskeyFuture<bool> {
        ready(Ok(false))
    }

    fn create(&self, request: CreateRequest) -> PasskeyFuture<Registration> {
        ready(request.validate().and(Err(UNAVAILABLE)))
    }

    fn get(&self, request: GetRequest) -> PasskeyFuture<Assertion> {
        ready(request.validate().and(Err(UNAVAILABLE)))
    }
}
