//! `PlatformVault` against the machine's real secret store (Keychain, Credential
//! Manager, or Secret Service), driven the way an app drives it: from Venus
//! tasks, on a Venus offload pool. The same assertions hold for every backend.
//!
//! Each test uses its own application id and removes what it wrote, so a failing
//! test leaves at most a few throwaway items under `aimer-vault-test.*`.

#![cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use aimer_vault::{
    MAX_SECRET_BYTES, PlatformVault, Vault, VaultAccess, VaultCapability, VaultError,
};
use aimer_venus::Venus;

fn block_on<F: Future>(future: F) -> F::Output {
    struct Parker(std::thread::Thread);

    impl Wake for Parker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Parker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match Pin::new(&mut future).poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// Installs a Venus runtime for the calling thread for the test's duration.
struct InstalledVenus(#[allow(dead_code)] Rc<Venus>);

impl InstalledVenus {
    fn new() -> Self {
        let venus = Venus::new();
        venus.install();
        Self(venus)
    }
}

impl Drop for InstalledVenus {
    fn drop(&mut self) {
        Venus::uninstall();
    }
}

/// A vault under a unique application id whose namespaces are cleared on drop.
struct TestVault {
    vault: PlatformVault,
    namespaces: &'static [&'static str],
}

impl TestVault {
    fn new(namespaces: &'static [&'static str]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = format!(
            "aimer-vault-test.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            vault: PlatformVault::for_app(&id).expect("the test id is a valid name"),
            namespaces,
        }
    }
}

impl std::ops::Deref for TestVault {
    type Target = PlatformVault;

    fn deref(&self) -> &PlatformVault {
        &self.vault
    }
}

impl Drop for TestVault {
    fn drop(&mut self) {
        for namespace in self.namespaces {
            let _ = block_on(self.vault.clear(namespace));
        }
    }
}

#[test]
fn a_secret_round_trips_through_the_keychain() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["accounts"]);

    block_on(vault.write("accounts", "api-token", b"s3cr3t")).unwrap();
    let secret = block_on(vault.read("accounts", "api-token")).unwrap().unwrap();
    assert_eq!(secret.expose(), b"s3cr3t");
}

#[test]
fn a_missing_secret_reads_as_none() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["accounts"]);
    assert_eq!(block_on(vault.read("accounts", "absent")).unwrap(), None);
}

#[test]
fn binary_secrets_survive_unchanged() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["keys"]);
    let bytes: Vec<u8> = (0..=255).collect();

    block_on(vault.write("keys", "blob", &bytes)).unwrap();
    let secret = block_on(vault.read("keys", "blob")).unwrap().unwrap();
    assert_eq!(secret.expose(), bytes.as_slice());
}

#[test]
fn writing_replaces_the_previous_secret() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["accounts"]);

    block_on(vault.write("accounts", "api-token", b"old")).unwrap();
    block_on(vault.write("accounts", "api-token", b"new")).unwrap();

    let secret = block_on(vault.read("accounts", "api-token")).unwrap().unwrap();
    assert_eq!(secret.expose(), b"new");
}

#[test]
fn remove_reports_whether_the_secret_existed() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["accounts"]);

    block_on(vault.write("accounts", "api-token", b"s3cr3t")).unwrap();
    assert!(block_on(vault.remove("accounts", "api-token")).unwrap());
    assert!(!block_on(vault.remove("accounts", "api-token")).unwrap());
    assert_eq!(block_on(vault.read("accounts", "api-token")).unwrap(), None);
}

#[test]
fn clear_removes_every_secret_of_one_namespace_only() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["a", "b"]);

    for key in ["one", "two", "three"] {
        block_on(vault.write("a", key, b"x")).unwrap();
    }
    block_on(vault.write("b", "one", b"y")).unwrap();

    block_on(vault.clear("a")).unwrap();

    for key in ["one", "two", "three"] {
        assert_eq!(block_on(vault.read("a", key)).unwrap(), None, "{key}");
    }
    assert!(block_on(vault.read("b", "one")).unwrap().is_some());
    // Clearing what is already empty is not an error.
    block_on(vault.clear("a")).unwrap();
}

#[test]
fn applications_do_not_see_each_others_secrets() {
    let _venus = InstalledVenus::new();
    let first = TestVault::new(&["accounts"]);
    let second = TestVault::new(&["accounts"]);

    block_on(first.write("accounts", "api-token", b"first")).unwrap();

    assert_eq!(block_on(second.read("accounts", "api-token")).unwrap(), None);
    block_on(second.clear("accounts")).unwrap();
    assert!(block_on(first.read("accounts", "api-token")).unwrap().is_some());
}

#[test]
fn operations_take_effect_in_call_order() {
    let _venus = InstalledVenus::new();
    let vault = TestVault::new(&["accounts"]);

    // Issue everything first and await afterwards: the read must still see the
    // write, even though the pool has several workers.
    let write = vault.write("accounts", "api-token", b"first");
    let rewrite = vault.write("accounts", "api-token", b"second");
    let read = vault.read("accounts", "api-token");

    block_on(write).unwrap();
    block_on(rewrite).unwrap();
    assert_eq!(block_on(read).unwrap().unwrap().expose(), b"second");
}

#[test]
fn both_access_levels_store_and_read_back() {
    let _venus = InstalledVenus::new();
    for access in [VaultAccess::WhenUnlocked, VaultAccess::AfterFirstUnlock] {
        let vault = TestVault::new(&["accounts"]);
        let vault_access = vault.vault.clone().with_access(access);
        assert_eq!(vault_access.access(), access);

        block_on(vault_access.write("accounts", "api-token", b"s3cr3t")).unwrap();
        let secret = block_on(vault_access.read("accounts", "api-token")).unwrap().unwrap();
        assert_eq!(secret.expose(), b"s3cr3t", "{access:?}");
    }
}

#[test]
fn invalid_input_is_rejected_before_the_keychain_is_touched() {
    // No Venus runtime is installed: a rejection that came from the keychain path
    // would report `Unavailable`, not the typed input error.
    let vault = TestVault::new(&[]);

    assert_eq!(block_on(vault.read("a/b", "k")), Err(VaultError::InvalidKey));
    assert_eq!(block_on(vault.write("n", "", b"x")), Err(VaultError::InvalidKey));
    assert_eq!(block_on(vault.write("n", "k", b"")), Err(VaultError::EmptySecret));
    assert_eq!(
        block_on(vault.write("n", "k", &vec![1; MAX_SECRET_BYTES + 1])),
        Err(VaultError::SecretTooLarge {
            len: MAX_SECRET_BYTES + 1,
            limit: MAX_SECRET_BYTES,
        })
    );
    assert_eq!(block_on(vault.remove("n", "..")), Err(VaultError::InvalidKey));
    assert_eq!(block_on(vault.clear("a:b")), Err(VaultError::InvalidKey));
}

#[test]
fn without_a_venus_runtime_operations_report_it() {
    let vault = TestVault::new(&[]);
    assert_eq!(
        block_on(vault.read("accounts", "api-token")),
        Err(VaultError::Unavailable {
            capability: VaultCapability::Runtime,
        })
    );
}
