//! On a target with no platform secret store, `PlatformVault` opens but refuses
//! every operation, so a secret is never kept somewhere weaker.

// Only targets with no secret store at all (the browser) get this adapter, so this
// is compiled everywhere but runs only where such a target can run tests.
#![cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]

use std::future::Future;
use std::task::{Context, Poll, Waker};

use aimer_vault::{PlatformVault, Vault, VaultCapability, VaultError};

fn now<T>(future: impl Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("an unavailable vault operation should resolve immediately"),
    }
}

const UNAVAILABLE: VaultError = VaultError::Unavailable {
    capability: VaultCapability::PlatformSecretStore,
};

#[test]
fn every_operation_reports_the_missing_secret_store() {
    let vault = PlatformVault::for_app("com.example.app").unwrap();

    assert_eq!(now(vault.write("accounts", "token", b"s3cr3t")), Err(UNAVAILABLE));
    assert_eq!(now(vault.read("accounts", "token")), Err(UNAVAILABLE));
    assert_eq!(now(vault.remove("accounts", "token")), Err(UNAVAILABLE));
    assert_eq!(now(vault.clear("accounts")), Err(UNAVAILABLE));
}

#[test]
fn an_invalid_application_id_is_still_rejected() {
    assert!(matches!(PlatformVault::for_app("a/b"), Err(VaultError::InvalidKey)));
}
