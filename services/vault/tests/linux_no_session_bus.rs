//! With no session bus to reach, `SecretServiceVault` reports that there is no
//! secret store rather than failing in some unrelated way.
//!
//! This is its own test binary because it points the process at a bus that does
//! not exist, which must not leak into other tests.

#![cfg(target_os = "linux")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use aimer_vault::{PlatformVault, Vault, VaultCapability, VaultError};
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

#[test]
fn a_missing_session_bus_means_no_secret_store() {
    // SAFETY: this is the only test in the binary and nothing else has started
    // threads that read the environment yet.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/aimer-vault-bus");
    }
    let venus = Venus::new();
    venus.install();

    let vault = PlatformVault::for_app("aimer-vault-test.no-bus").unwrap();
    let unavailable = VaultError::Unavailable {
        capability: VaultCapability::PlatformSecretStore,
    };
    assert_eq!(block_on(vault.read("accounts", "token")).unwrap_err(), unavailable);
    assert_eq!(
        block_on(vault.write("accounts", "token", b"s3cr3t")).unwrap_err(),
        unavailable
    );
    // A failed connection is not cached: asking again asks again.
    assert_eq!(block_on(vault.remove("accounts", "token")).unwrap_err(), unavailable);

    Venus::uninstall();
}
