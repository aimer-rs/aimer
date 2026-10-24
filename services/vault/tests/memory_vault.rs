//! `MemoryVault` honours the same contract as every other adapter.

use std::future::Future;
use std::task::{Context, Poll, Waker};

use aimer_vault::{MAX_SECRET_BYTES, MemoryVault, SecretBytes, Vault, VaultError};

/// Memory operations resolve before they are returned, so one poll is enough.
fn now<T>(future: impl Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a memory vault operation should resolve immediately"),
    }
}

#[test]
fn a_written_secret_reads_back() {
    let vault = MemoryVault::new();
    now(vault.write("accounts", "token", b"s3cr3t")).unwrap();

    let secret = now(vault.read("accounts", "token")).unwrap().unwrap();
    assert_eq!(secret, SecretBytes::from_slice(b"s3cr3t"));
}

#[test]
fn a_missing_secret_reads_as_none() {
    let vault = MemoryVault::new();
    assert_eq!(now(vault.read("accounts", "token")).unwrap(), None);
}

#[test]
fn writing_replaces_the_previous_secret() {
    let vault = MemoryVault::new();
    now(vault.write("accounts", "token", b"old")).unwrap();
    now(vault.write("accounts", "token", b"new")).unwrap();

    let secret = now(vault.read("accounts", "token")).unwrap().unwrap();
    assert_eq!(secret.expose(), b"new");
}

#[test]
fn namespaces_are_isolated() {
    let vault = MemoryVault::new();
    now(vault.write("a", "token", b"one")).unwrap();
    now(vault.write("b", "token", b"two")).unwrap();

    assert_eq!(now(vault.read("a", "token")).unwrap().unwrap().expose(), b"one");
    assert_eq!(now(vault.read("b", "token")).unwrap().unwrap().expose(), b"two");
}

#[test]
fn remove_reports_whether_the_secret_existed() {
    let vault = MemoryVault::new();
    now(vault.write("accounts", "token", b"s3cr3t")).unwrap();

    assert!(now(vault.remove("accounts", "token")).unwrap());
    assert!(!now(vault.remove("accounts", "token")).unwrap());
    assert_eq!(now(vault.read("accounts", "token")).unwrap(), None);
}

#[test]
fn clear_empties_one_namespace_only() {
    let vault = MemoryVault::new();
    now(vault.write("a", "one", b"1")).unwrap();
    now(vault.write("a", "two", b"2")).unwrap();
    now(vault.write("b", "one", b"3")).unwrap();

    now(vault.clear("a")).unwrap();

    assert_eq!(now(vault.read("a", "one")).unwrap(), None);
    assert_eq!(now(vault.read("a", "two")).unwrap(), None);
    assert!(now(vault.read("b", "one")).unwrap().is_some());
    // Clearing what is already empty is not an error.
    now(vault.clear("a")).unwrap();
}

#[test]
fn clones_share_one_store() {
    let vault = MemoryVault::new();
    let clone = vault.clone();
    now(vault.write("accounts", "token", b"s3cr3t")).unwrap();
    assert!(now(clone.read("accounts", "token")).unwrap().is_some());
}

#[test]
fn invalid_names_are_rejected_by_every_operation() {
    let vault = MemoryVault::new();
    for bad in ["", ".", "..", "a/b", "a\\b", "a:b", "a\0b"] {
        assert_eq!(now(vault.read(bad, "k")), Err(VaultError::InvalidKey), "{bad:?}");
        assert_eq!(now(vault.read("n", bad)), Err(VaultError::InvalidKey), "{bad:?}");
        assert_eq!(now(vault.write(bad, "k", b"x")), Err(VaultError::InvalidKey));
        assert_eq!(now(vault.write("n", bad, b"x")), Err(VaultError::InvalidKey));
        assert_eq!(now(vault.remove(bad, "k")), Err(VaultError::InvalidKey));
        assert_eq!(now(vault.clear(bad)), Err(VaultError::InvalidKey));
    }
}

#[test]
fn secrets_must_be_non_empty_and_bounded() {
    let vault = MemoryVault::new();
    assert_eq!(now(vault.write("n", "k", b"")), Err(VaultError::EmptySecret));

    now(vault.write("n", "k", &vec![1; MAX_SECRET_BYTES])).unwrap();
    assert_eq!(
        now(vault.write("n", "k", &vec![1; MAX_SECRET_BYTES + 1])),
        Err(VaultError::SecretTooLarge {
            len: MAX_SECRET_BYTES + 1,
            limit: MAX_SECRET_BYTES,
        })
    );
}

#[test]
fn a_rejected_write_leaves_the_old_secret_alone() {
    let vault = MemoryVault::new();
    now(vault.write("n", "k", b"keep")).unwrap();
    let _ = now(vault.write("n", "k", b""));
    assert_eq!(now(vault.read("n", "k")).unwrap().unwrap().expose(), b"keep");
}
