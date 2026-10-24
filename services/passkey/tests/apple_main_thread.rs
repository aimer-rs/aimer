//! Apple's authorization UI can only be driven from the main thread. A test runs
//! on a worker thread, which makes the refusal observable without a window.

#![cfg(any(target_os = "macos", target_os = "ios"))]

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use aimer_passkey::{
    CreateRequest, GetRequest, PasskeyCapability, PasskeyError, Passkeys, PlatformPasskeys,
    RelyingParty, User,
};

fn now<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut future = Box::pin(future);
    match Pin::new(&mut future).poll(&mut Context::from_waker(waker)) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a refused ceremony must resolve immediately"),
    }
}

const NOT_MAIN_THREAD: PasskeyError = PasskeyError::Unavailable {
    capability: PasskeyCapability::MainThread,
};

#[test]
fn a_ceremony_off_the_main_thread_is_refused_not_started() {
    let passkeys = PlatformPasskeys::new();

    let created = now(passkeys.create(CreateRequest::new(
        RelyingParty::new("example.com", "Example"),
        User::new(vec![1; 16], "alice"),
        vec![7; 32],
    )));
    let signed_in = now(passkeys.get(GetRequest::new("example.com", vec![7; 32])));

    assert_eq!(created.unwrap_err(), NOT_MAIN_THREAD);
    assert_eq!(signed_in.unwrap_err(), NOT_MAIN_THREAD);
}
