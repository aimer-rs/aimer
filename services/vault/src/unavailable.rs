//! The platform adapter on targets that have no secret store behind it.

use crate::access::VaultAccess;
use crate::error::{VaultCapability, VaultError, VaultResult};
use crate::key::validate_component;
use crate::secret::SecretBytes;
use crate::vault::{Vault, VaultFuture, ready};

/// The [`PlatformVault`](crate::PlatformVault) of a target with no platform
/// secret store.
///
/// It opens like the real one, so application code is the same on every
/// target, and then every operation reports
/// [`VaultError::Unavailable`] with [`VaultCapability::PlatformSecretStore`].
/// That is the point: a secret asked to be kept safely is never kept somewhere
/// weaker, and the application learns it can not store the credential and can
/// ask the user to sign in again.
#[derive(Clone)]
pub struct UnavailableVault {
    _private: (),
}

impl UnavailableVault {
    /// Opens the vault an application would keep its secrets in.
    ///
    /// # Errors
    ///
    /// [`VaultError::InvalidKey`] when `app_id` is not a valid name.
    pub fn for_app(app_id: &str) -> VaultResult<Self> {
        validate_component(app_id)?;
        Ok(Self { _private: () })
    }

    /// Accepts an access level so code written for every target compiles
    /// unchanged. There is no secret store to apply it to.
    #[inline]
    pub fn with_access(self, _access: VaultAccess) -> Self {
        self
    }
}

fn unavailable<T: 'static>() -> VaultFuture<T> {
    ready(Err(VaultError::Unavailable {
        capability: VaultCapability::PlatformSecretStore,
    }))
}

impl Vault for UnavailableVault {
    fn read(&self, _namespace: &str, _key: &str) -> VaultFuture<Option<SecretBytes>> {
        unavailable()
    }

    fn write(&self, _namespace: &str, _key: &str, _secret: &[u8]) -> VaultFuture<()> {
        unavailable()
    }

    fn remove(&self, _namespace: &str, _key: &str) -> VaultFuture<bool> {
        unavailable()
    }

    fn clear(&self, _namespace: &str) -> VaultFuture<()> {
        unavailable()
    }
}
