//! `UnavailablePasskeys`: what an application sees on a target with no provider.
//! Every future is already resolved, so no runtime is needed.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use aimer_passkey::{
    CreateRequest, GetRequest, InvalidRequest, PasskeyCapability, PasskeyError, Passkeys,
    RelyingParty, UnavailablePasskeys, User,
};

fn now<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut future = Box::pin(future);
    match Pin::new(&mut future).poll(&mut Context::from_waker(waker)) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("an unavailable adapter must resolve immediately"),
    }
}

fn create(rp_id: &str) -> CreateRequest {
    CreateRequest::new(
        RelyingParty::new(rp_id, "Example"),
        User::new(vec![1; 16], "alice"),
        vec![7; 32],
    )
}

const UNAVAILABLE: PasskeyError = PasskeyError::Unavailable {
    capability: PasskeyCapability::PlatformAuthenticator,
};

#[test]
fn it_reports_that_passkeys_are_not_available() {
    assert_eq!(now(UnavailablePasskeys::new().is_available()), Ok(false));
}

#[test]
fn every_ceremony_is_unavailable() {
    let passkeys = UnavailablePasskeys::new();

    assert_eq!(now(passkeys.create(create("example.com"))), Err(UNAVAILABLE));
    assert_eq!(
        now(passkeys.get(GetRequest::new("example.com", vec![7; 32]))),
        Err(UNAVAILABLE)
    );
}

#[test]
fn a_malformed_request_is_rejected_before_availability_is_considered() {
    let passkeys = UnavailablePasskeys::new();

    assert_eq!(
        now(passkeys.create(create("https://example.com"))),
        Err(PasskeyError::InvalidRequest(InvalidRequest::RelyingPartyId))
    );
    assert_eq!(
        now(passkeys.get(GetRequest::new("example.com", Vec::new()))),
        Err(PasskeyError::InvalidRequest(InvalidRequest::Challenge))
    );
}
