//! The Keychain adapter for macOS and iOS.

use std::ptr;
use std::sync::Arc;

use core_foundation::base::{CFType, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
#[cfg(target_os = "ios")]
use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::passwords::{PasswordOptions, delete_generic_password, generic_password};
use security_framework_sys::base::{
    errSecAuthFailed as ERR_SEC_AUTH_FAILED, errSecDuplicateItem as ERR_SEC_DUPLICATE_ITEM,
    errSecItemNotFound as ERR_SEC_ITEM_NOT_FOUND,
};
use security_framework_sys::item::{
    kSecAttrAccessControl, kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword,
    kSecValueData,
};
use security_framework_sys::keychain_item::{SecItemAdd, SecItemDelete, SecItemUpdate};

use crate::access::VaultAccess;
use crate::error::{VaultError, VaultResult};
use crate::key::{check_secret, validate, validate_component};
use crate::lane::Lane;
use crate::secret::SecretBytes;
use crate::vault::{Vault, VaultFuture, ready};

// Not exported by security-framework-sys. Values from <Security/SecBase.h> and
// <CoreServices/CoreServices.h>.
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;
const USER_CANCELED_ERR: i32 = -128;

/// A vault backed by the Keychain.
///
/// Each secret is a generic-password item. Its *service* is
/// `"{app_id}/{namespace}"` and its *account* is the key, so two applications
/// never see each other's secrets and clearing a namespace touches only this
/// application's.
///
/// Keychain calls can block on a system daemon (and on an unlock prompt), so
/// they run on Venus's offload pool, in call order, and a Venus runtime must be
/// installed on the calling thread: an operation made without one resolves to
/// [`VaultError::Unavailable`] with
/// [`VaultCapability::Runtime`](crate::VaultCapability::Runtime).
///
/// On iOS the item is created with the strictest accessibility the chosen
/// [`VaultAccess`] allows, always this-device-only. On macOS it lives in the
/// user's login keychain.
///
/// Clones share one ordering queue.
#[derive(Clone)]
pub struct KeychainVault {
    app_id: Arc<str>,
    access: VaultAccess,
    lane: Lane,
}

impl KeychainVault {
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

    /// Sets when newly written secrets may be read. See [`VaultAccess`].
    ///
    /// Applies to secrets written afterwards; an existing secret keeps the level
    /// it was created with.
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

    /// `"{app_id}/{namespace}"`; `/` cannot appear in either part, so the split
    /// is unambiguous.
    fn service(&self, namespace: &str) -> String {
        format!("{}/{namespace}", self.app_id)
    }
}

impl Vault for KeychainVault {
    fn read(&self, namespace: &str, key: &str) -> VaultFuture<Option<SecretBytes>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let (service, account) = (self.service(namespace), key.to_owned());
        self.lane.submit(move || read_item(&service, &account))
    }

    fn write(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultFuture<()> {
        if let Err(error) = validate(namespace, key).and_then(|()| check_secret(secret.len())) {
            return ready(Err(error));
        }
        let (service, account, access) = (self.service(namespace), key.to_owned(), self.access);
        // The worker needs an owned copy; it is wiped when the job finishes.
        let secret = SecretBytes::from_slice(secret);
        self.lane
            .submit(move || write_item(&service, &account, secret.expose(), access))
    }

    fn remove(&self, namespace: &str, key: &str) -> VaultFuture<bool> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let (service, account) = (self.service(namespace), key.to_owned());
        self.lane.submit(move || remove_item(&service, &account))
    }

    fn clear(&self, namespace: &str) -> VaultFuture<()> {
        if let Err(error) = validate_component(namespace) {
            return ready(Err(error));
        }
        let service = self.service(namespace);
        self.lane.submit(move || clear_service(&service))
    }
}

fn read_item(service: &str, account: &str) -> VaultResult<Option<SecretBytes>> {
    match generic_password(PasswordOptions::new_generic_password(service, account)) {
        Ok(bytes) => Ok(Some(SecretBytes::new(bytes))),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(map_status(error.code())),
    }
}

fn remove_item(service: &str, account: &str) -> VaultResult<bool> {
    match delete_generic_password(service, account) {
        Ok(()) => Ok(true),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(false),
        Err(error) => Err(map_status(error.code())),
    }
}

/// Deletes every item of `service`.
///
/// One `SecItemDelete` removes a single match on macOS, so it is repeated until
/// nothing is left; each pass removes at least one item, which bounds the loop.
fn clear_service(service: &str) -> VaultResult<()> {
    let query = dictionary(&[
        (key(unsafe { kSecClass }), generic_password_class()),
        (key(unsafe { kSecAttrService }), CFString::new(service).as_CFType()),
    ]);
    loop {
        // SAFETY: `query` is a live CFDictionary for the duration of the call.
        match unsafe { SecItemDelete(query.as_concrete_TypeRef()) } {
            0 => {}
            ERR_SEC_ITEM_NOT_FOUND => return Ok(()),
            status => return Err(map_status(status)),
        }
    }
}

/// Creates or replaces one secret.
///
/// Updating first keeps an existing item's own accessibility and avoids ever
/// deleting a secret before its replacement is stored; only an absent item is
/// added, with the requested access level.
fn write_item(service: &str, account: &str, secret: &[u8], access: VaultAccess) -> VaultResult<()> {
    let identity = [
        (key(unsafe { kSecClass }), generic_password_class()),
        (key(unsafe { kSecAttrService }), CFString::new(service).as_CFType()),
        (key(unsafe { kSecAttrAccount }), CFString::new(account).as_CFType()),
    ];
    let value = CFData::from_buffer(secret).as_CFType();

    let update_status = || {
        let query = dictionary(&identity);
        let update = dictionary(&[(key(unsafe { kSecValueData }), value.clone())]);
        // SAFETY: both dictionaries are live for the duration of the call.
        unsafe { SecItemUpdate(query.as_concrete_TypeRef(), update.as_concrete_TypeRef()) }
    };

    match update_status() {
        0 => return Ok(()),
        ERR_SEC_ITEM_NOT_FOUND => {}
        status => return Err(map_status(status)),
    }

    let mut attributes = identity.to_vec();
    attributes.push((key(unsafe { kSecValueData }), value.clone()));
    if let Some(control) = access_control(access)? {
        attributes.push((key(unsafe { kSecAttrAccessControl }), control));
    }
    let add = dictionary(&attributes);
    // SAFETY: `add` is live for the call, and a null result pointer asks for no
    // result.
    match unsafe { SecItemAdd(add.as_concrete_TypeRef(), ptr::null_mut()) } {
        0 => Ok(()),
        // Another writer added it between our update and our add: it exists now,
        // so the update succeeds.
        ERR_SEC_DUPLICATE_ITEM => match update_status() {
            0 => Ok(()),
            status => Err(map_status(status)),
        },
        status => Err(map_status(status)),
    }
}

/// The accessibility to create an item with, on platforms that honour one.
///
/// iOS always uses the data-protection keychain, which does. The macOS
/// file-based login keychain rejects `kSecAttrAccessControl` outright
/// (`errSecMissingEntitlement`), because accessibility classes belong to the
/// data-protection keychain, so there the item is created with the keychain's
/// own login-bound lock state.
#[cfg(target_os = "ios")]
fn access_control(access: VaultAccess) -> VaultResult<Option<CFType>> {
    let protection = match access {
        VaultAccess::WhenUnlocked => ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly,
        VaultAccess::AfterFirstUnlock => ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly,
    };
    SecAccessControl::create_with_protection(Some(protection), 0)
        .map(|control| Some(control.as_CFType()))
        .map_err(|error| map_status(error.code()))
}

#[cfg(not(target_os = "ios"))]
fn access_control(_access: VaultAccess) -> VaultResult<Option<CFType>> {
    Ok(None)
}

fn key(constant: CFStringRef) -> CFString {
    // SAFETY: the `kSec*` constants are immortal `CFString`s owned by Security.
    unsafe { CFString::wrap_under_get_rule(constant) }
}

fn generic_password_class() -> CFType {
    key(unsafe { kSecClassGenericPassword }).as_CFType()
}

fn dictionary(pairs: &[(CFString, CFType)]) -> CFDictionary<CFString, CFType> {
    CFDictionary::from_CFType_pairs(pairs)
}

/// Translates an `OSStatus` into the vault's vocabulary.
fn map_status(status: i32) -> VaultError {
    match status {
        ERR_SEC_INTERACTION_NOT_ALLOWED => VaultError::Locked,
        ERR_SEC_AUTH_FAILED | USER_CANCELED_ERR => VaultError::Denied,
        ERR_SEC_MISSING_ENTITLEMENT => VaultError::MissingEntitlement,
        code => VaultError::Backend { code },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_typed_errors() {
        assert_eq!(map_status(-25308), VaultError::Locked);
        assert_eq!(map_status(-25293), VaultError::Denied);
        assert_eq!(map_status(-128), VaultError::Denied);
        assert_eq!(map_status(-34018), VaultError::MissingEntitlement);
        assert_eq!(map_status(-50), VaultError::Backend { code: -50 });
    }

    #[test]
    fn the_service_is_scoped_to_the_application() {
        let vault = KeychainVault::for_app("com.example.app").unwrap();
        assert_eq!(vault.service("tokens"), "com.example.app/tokens");
    }

    #[test]
    fn an_invalid_application_id_is_rejected() {
        assert!(matches!(
            KeychainVault::for_app("a/b"),
            Err(VaultError::InvalidKey)
        ));
        assert!(matches!(KeychainVault::for_app(""), Err(VaultError::InvalidKey)));
    }
}
