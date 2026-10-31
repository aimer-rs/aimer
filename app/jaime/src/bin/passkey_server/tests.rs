//! The protocol end to end, against a software authenticator.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use aimer_time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::{AndroidApp, Config, Request, Server};
use crate::authenticator::{Asserted, Registered, SoftAuthenticator};
use crate::base64url;

const RP: &str = "example.com";
const ORIGIN: &str = "https://example.com";

fn server() -> Server {
    Server::new(Config {
        rp_id: RP.to_owned(),
        rp_name: "Example".to_owned(),
        apple_app_ids: vec!["TEAM.com.example.jaime".to_owned()],
        android: Some(AndroidApp {
            package: "com.example.jaime".to_owned(),
            fingerprints: vec!["AA:BB".to_owned()],
        }),
    })
}

fn call(server: &Server, method: &str, path: &str, body: Value) -> (u16, Value) {
    call_at(server, method, path, body, Instant::now())
}

fn call_at(server: &Server, method: &str, path: &str, body: Value, now: Instant) -> (u16, Value) {
    let response = server.handle_at(
        &Request {
            method: method.to_owned(),
            path: path.to_owned(),
            body: body.to_string().into_bytes(),
        },
        now,
    );
    let json = serde_json::from_slice(&response.body).expect("every response is JSON");
    (response.status, json)
}

fn challenge_for(server: &Server, username: &str) -> String {
    let (status, options) = call(server, "POST", "/register/options", json!({ "username": username }));
    assert_eq!(status, 200, "{options}");
    options["challenge"].as_str().unwrap().to_owned()
}

fn register_body(challenge: &str, id: &str, done: &Registered) -> Value {
    json!({
        "challenge": challenge,
        "credentialId": id,
        "clientDataJSON": base64url::encode(&done.client_data_json),
        "attestationObject": base64url::encode(&done.attestation_object),
    })
}

fn login_body(challenge: &str, id: &str, done: &Asserted) -> Value {
    json!({
        "challenge": challenge,
        "credentialId": id,
        "clientDataJSON": base64url::encode(&done.client_data_json),
        "authenticatorData": base64url::encode(&done.authenticator_data),
        "signature": base64url::encode(&done.signature),
    })
}

fn login_challenge(server: &Server) -> String {
    let (status, options) = call(server, "POST", "/login/options", json!({}));
    assert_eq!(status, 200);
    options["challenge"].as_str().unwrap().to_owned()
}

/// Registers `alice` with a fresh authenticator and returns it.
fn registered(server: &Server) -> SoftAuthenticator {
    let authenticator = SoftAuthenticator::new(1);
    let challenge = challenge_for(server, "alice");
    let done = authenticator.register(&challenge, RP, ORIGIN);
    let (status, body) = call(
        server,
        "POST",
        "/register/verify",
        register_body(&challenge, &authenticator.id_text(), &done),
    );
    assert_eq!(status, 200, "{body}");
    authenticator
}

fn error_of(body: &Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

#[test]
fn registering_then_signing_in_succeeds() {
    let server = server();
    let mut authenticator = registered(&server);

    let challenge = login_challenge(&server);
    let done = authenticator.assert(&challenge, RP, ORIGIN);
    let (status, body) = call(
        &server,
        "POST",
        "/login/verify",
        login_body(&challenge, &authenticator.id_text(), &done),
    );

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["username"], "alice");
    assert_eq!(body["signCount"], 1);
}

#[test]
fn the_registration_response_reports_the_account_and_credential() {
    let server = server();
    let authenticator = SoftAuthenticator::new(7);
    let challenge = challenge_for(&server, "  alice  ");
    let done = authenticator.register(&challenge, RP, ORIGIN);

    let (_, body) = call(
        &server,
        "POST",
        "/register/verify",
        register_body(&challenge, &authenticator.id_text(), &done),
    );

    assert_eq!(body["username"], "alice", "the name is trimmed");
    assert_eq!(body["credentialId"], authenticator.id_text());
    assert_eq!(body["passkeys"], 1);
    let (_, users) = call(&server, "GET", "/users", json!({}));
    assert_eq!(users["users"][0]["name"], "alice");
}

#[test]
fn registration_options_carry_the_relying_party_and_a_stable_user_handle() {
    let server = server();
    let (_, first) = call(&server, "POST", "/register/options", json!({ "username": "alice" }));
    let (_, second) = call(&server, "POST", "/register/options", json!({ "username": "bob" }));

    assert_eq!(first["rpId"], RP);
    assert_eq!(first["rpName"], "Example");
    assert_eq!(first["userName"], "alice");
    assert_eq!(first["excludeCredentials"], json!([]));
    assert_ne!(first["challenge"], second["challenge"]);
    assert_ne!(first["userId"], second["userId"]);
    assert_eq!(base64url::decode(first["challenge"].as_str().unwrap()).unwrap().len(), 32);
}

#[test]
fn a_known_account_excludes_its_existing_passkeys() {
    let server = server();
    let authenticator = registered(&server);

    let (_, options) = call(&server, "POST", "/register/options", json!({ "username": "alice" }));

    assert_eq!(options["excludeCredentials"], json!([authenticator.id_text()]));
}

#[test]
fn a_second_passkey_joins_the_same_account() {
    let server = server();
    registered(&server);
    let second = SoftAuthenticator::new(2);
    let challenge = challenge_for(&server, "alice");
    let done = second.register(&challenge, RP, ORIGIN);

    let (status, body) = call(
        &server,
        "POST",
        "/register/verify",
        register_body(&challenge, &second.id_text(), &done),
    );

    assert_eq!((status, &body["passkeys"]), (200, &json!(2)));
}

#[test]
fn registering_the_same_credential_twice_conflicts() {
    let server = server();
    let authenticator = registered(&server);
    let challenge = challenge_for(&server, "bob");
    let done = authenticator.register(&challenge, RP, ORIGIN);

    let (status, body) = call(
        &server,
        "POST",
        "/register/verify",
        register_body(&challenge, &authenticator.id_text(), &done),
    );

    assert_eq!(status, 409, "{body}");
}

#[test]
fn bad_usernames_are_refused() {
    let server = server();
    for name in ["", "   ", "line\nbreak", &"x".repeat(65)] {
        let (status, _) = call(&server, "POST", "/register/options", json!({ "username": name }));
        assert_eq!(status, 400, "{name:?}");
    }
    let (status, _) = call(&server, "POST", "/register/options", json!({}));
    assert_eq!(status, 400);
}

#[test]
fn a_challenge_cannot_be_used_twice() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);
    let challenge = challenge_for(&server, "alice");
    let done = authenticator.register(&challenge, RP, ORIGIN);
    let body = register_body(&challenge, &authenticator.id_text(), &done);

    assert_eq!(call(&server, "POST", "/register/verify", body.clone()).0, 200);
    let (status, replay) = call(&server, "POST", "/register/verify", body);

    assert_eq!(status, 400);
    assert!(error_of(&replay).contains("challenge"), "{replay}");
}

#[test]
fn a_challenge_for_one_ceremony_does_not_finish_the_other() {
    let server = server();
    let mut authenticator = registered(&server);
    let registration_challenge = challenge_for(&server, "alice");
    let done = authenticator.assert(&registration_challenge, RP, ORIGIN);

    let (status, _) = call(
        &server,
        "POST",
        "/login/verify",
        login_body(&registration_challenge, &authenticator.id_text(), &done),
    );

    assert_eq!(status, 400);
}

#[test]
fn an_expired_challenge_is_refused() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);
    let now = Instant::now();
    let (_, options) = call_at(&server, "POST", "/register/options", json!({ "username": "alice" }), now);
    let challenge = options["challenge"].as_str().unwrap();
    let done = authenticator.register(challenge, RP, ORIGIN);

    let (status, _) = call_at(
        &server,
        "POST",
        "/register/verify",
        register_body(challenge, &authenticator.id_text(), &done),
        now + Duration::from_secs(301),
    );

    assert_eq!(status, 400);
}

#[test]
fn registration_with_the_wrong_origin_or_challenge_is_refused() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);

    let challenge = challenge_for(&server, "alice");
    let done = authenticator.register(&challenge, RP, "https://evil.example");
    let (status, body) = call(&server, "POST", "/register/verify", register_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!((status, error_of(&body)), (400, "the origin is not this relying party's"));

    let challenge = challenge_for(&server, "alice");
    let other = challenge_for(&server, "alice");
    let done = authenticator.register(&other, RP, ORIGIN);
    let (status, body) = call(&server, "POST", "/register/verify", register_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!((status, error_of(&body)), (400, "the challenge does not match the one issued"));
}

#[test]
fn an_android_app_origin_is_accepted() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);
    let challenge = challenge_for(&server, "alice");
    let done = authenticator.register(&challenge, RP, "android:apk-key-hash:abc");

    let (status, body) = call(&server, "POST", "/register/verify", register_body(&challenge, &authenticator.id_text(), &done));

    assert_eq!(status, 200, "{body}");
}

#[test]
fn registration_for_another_relying_party_or_without_presence_is_refused() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);

    let challenge = challenge_for(&server, "alice");
    let done = authenticator.register(&challenge, "other.example", ORIGIN);
    let (status, body) = call(&server, "POST", "/register/verify", register_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!((status, error_of(&body)), (400, "the rpIdHash is for a different relying party"));

    let challenge = challenge_for(&server, "alice");
    let done = authenticator.register_with_flags(&challenge, RP, ORIGIN, 0x40);
    let (status, body) = call(&server, "POST", "/register/verify", register_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!((status, error_of(&body)), (400, "the user was not present"));
}

#[test]
fn registration_reporting_a_different_credential_id_is_refused() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);
    let challenge = challenge_for(&server, "alice");
    let done = authenticator.register(&challenge, RP, ORIGIN);

    let (status, _) = call(&server, "POST", "/register/verify", register_body(&challenge, &base64url::encode(&[9; 32]), &done));

    assert_eq!(status, 400);
}

#[test]
fn garbage_attestation_is_refused_without_panicking() {
    let server = server();
    let authenticator = SoftAuthenticator::new(1);
    for attestation in [vec![], vec![0xa1], vec![0xff; 40], vec![0xa3; 300]] {
        let challenge = challenge_for(&server, "alice");
        let mut done = authenticator.register(&challenge, RP, ORIGIN);
        done.attestation_object = attestation;
        let (status, _) = call(&server, "POST", "/register/verify", register_body(&challenge, &authenticator.id_text(), &done));
        assert_eq!(status, 400);
    }
}

#[test]
fn a_forged_or_tampered_signature_is_refused() {
    let server = server();
    let mut authenticator = registered(&server);

    let challenge = login_challenge(&server);
    let mut done = authenticator.assert(&challenge, RP, ORIGIN);
    let last = done.signature.len() - 1;
    done.signature[last] ^= 1;
    let (status, body) = call(&server, "POST", "/login/verify", login_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!((status, error_of(&body)), (401, "the signature does not verify"));

    // Signed by a different key under the same credential id.
    let mut impostor = SoftAuthenticator::new(1);
    let challenge = login_challenge(&server);
    let done = impostor.assert(&challenge, RP, ORIGIN);
    let (status, _) = call(&server, "POST", "/login/verify", login_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!(status, 401);
}

#[test]
fn the_signature_covers_the_client_data() {
    let server = server();
    let mut authenticator = registered(&server);
    let challenge = login_challenge(&server);
    let mut done = authenticator.assert(&challenge, RP, ORIGIN);
    // Same challenge and type, different (still accepted) origin spelling.
    done.client_data_json = SoftAuthenticator::client_data(
        "webauthn.get",
        &challenge,
        "android:apk-key-hash:swapped",
    );

    let (status, body) = call(&server, "POST", "/login/verify", login_body(&challenge, &authenticator.id_text(), &done));

    assert_eq!((status, error_of(&body)), (401, "the signature does not verify"));
}

#[test]
fn an_unknown_passkey_and_a_mismatched_user_handle_are_refused() {
    let server = server();
    let mut authenticator = registered(&server);

    let challenge = login_challenge(&server);
    let done = authenticator.assert(&challenge, RP, ORIGIN);
    let (status, _) = call(&server, "POST", "/login/verify", login_body(&challenge, &base64url::encode(&[5; 32]), &done));
    assert_eq!(status, 401);

    let challenge = login_challenge(&server);
    let done = authenticator.assert(&challenge, RP, ORIGIN);
    let mut body = login_body(&challenge, &authenticator.id_text(), &done);
    body["userHandle"] = json!(base64url::encode(&[0; 16]));
    let (status, _) = call(&server, "POST", "/login/verify", body);
    assert_eq!(status, 401);
}

#[test]
fn a_matching_user_handle_is_accepted() {
    let server = server();
    let mut authenticator = registered(&server);
    let (_, options) = call(&server, "POST", "/register/options", json!({ "username": "alice" }));
    let handle = options["userId"].clone();
    let challenge = login_challenge(&server);
    let done = authenticator.assert(&challenge, RP, ORIGIN);
    let mut body = login_body(&challenge, &authenticator.id_text(), &done);
    body["userHandle"] = handle;

    let (status, response) = call(&server, "POST", "/login/verify", body);

    assert_eq!(status, 200, "{response}");
}

#[test]
fn a_counter_that_goes_backwards_is_refused_but_zero_counters_are_fine() {
    let server = server();
    let mut authenticator = registered(&server);
    for _ in 0..2 {
        let challenge = login_challenge(&server);
        let done = authenticator.assert(&challenge, RP, ORIGIN);
        let (status, _) = call(&server, "POST", "/login/verify", login_body(&challenge, &authenticator.id_text(), &done));
        assert_eq!(status, 200);
    }

    // A clone replaying an old counter.
    authenticator.sign_count = 1;
    let challenge = login_challenge(&server);
    let done = authenticator.assert_with_count(&challenge, RP, ORIGIN);
    let (status, body) = call(&server, "POST", "/login/verify", login_body(&challenge, &authenticator.id_text(), &done));
    assert_eq!((status, error_of(&body)), (401, "the signature counter did not increase"));

    // An authenticator that never counts stays at zero and must keep working.
    let zero = SoftAuthenticator::new(3);
    let challenge = challenge_for(&server, "zed");
    let done = zero.register(&challenge, RP, ORIGIN);
    call(&server, "POST", "/register/verify", register_body(&challenge, &zero.id_text(), &done));
    for _ in 0..2 {
        let challenge = login_challenge(&server);
        let done = zero.assert_with_count(&challenge, RP, ORIGIN);
        let (status, _) = call(&server, "POST", "/login/verify", login_body(&challenge, &zero.id_text(), &done));
        assert_eq!(status, 200);
    }
}

#[test]
fn malformed_bodies_are_bad_requests() {
    let server = server();
    for path in ["/register/options", "/register/verify", "/login/verify"] {
        let response = server.handle(&Request {
            method: "POST".to_owned(),
            path: path.to_owned(),
            body: b"not json".to_vec(),
        });
        assert_eq!(response.status, 400, "{path}");
        let (status, _) = call(&server, "POST", path, json!({ "challenge": 5 }));
        assert_eq!(status, 400, "{path}");
    }
}

#[test]
fn routing_distinguishes_unknown_paths_from_wrong_methods() {
    let server = server();
    assert_eq!(call(&server, "GET", "/nope", json!({})).0, 404);
    assert_eq!(call(&server, "GET", "/register/options", json!({})).0, 405);
    assert_eq!(call(&server, "POST", "/health", json!({})).0, 405);
    let (status, health) = call(&server, "GET", "/health", json!({}));
    assert_eq!((status, &health["rpId"]), (200, &json!(RP)));
}

#[test]
fn reset_forgets_accounts() {
    let server = server();
    registered(&server);

    call(&server, "POST", "/reset", json!({}));

    let (_, users) = call(&server, "GET", "/users", json!({}));
    assert_eq!(users["users"], json!([]));
}

#[test]
fn the_domain_association_files_are_served_when_configured() {
    let server = server();
    let (_, apple) = call(&server, "GET", "/.well-known/apple-app-site-association", json!({}));
    assert_eq!(apple["webcredentials"]["apps"], json!(["TEAM.com.example.jaime"]));
    let (_, android) = call(&server, "GET", "/.well-known/assetlinks.json", json!({}));
    assert_eq!(android[0]["target"]["package_name"], "com.example.jaime");
    assert_eq!(android[0]["relation"], json!(["delegate_permission/common.get_login_creds"]));

    let bare = Server::new(Config {
        rp_id: RP.to_owned(),
        rp_name: "Example".to_owned(),
        apple_app_ids: Vec::new(),
        android: None,
    });
    assert_eq!(call(&bare, "GET", "/.well-known/apple-app-site-association", json!({})).0, 404);
    assert_eq!(call(&bare, "GET", "/.well-known/assetlinks.json", json!({})).0, 404);
}

/// Sends `raw` to a freshly served instance and returns the whole reply.
fn over_the_wire(raw: &[u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || crate::http::serve(listener, Arc::new(server())));

    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.write_all(raw).unwrap();
    // Half-close so a request that promises more than it sent ends in EOF.
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    reply
}

#[test]
fn http_serves_a_get_and_a_post_with_a_body() {
    let reply = over_the_wire(b"GET /health?x=1 HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.contains(r#""rpId":"example.com""#), "{reply}");
    assert!(reply.contains("Content-Length: "), "{reply}");

    let body = r#"{"username":"alice"}"#;
    let request = format!(
        "POST /register/options HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let reply = over_the_wire(request.as_bytes());
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.contains(r#""userName":"alice""#), "{reply}");
}

#[test]
fn http_refuses_malformed_oversized_and_chunked_requests() {
    assert!(over_the_wire(b"garbage\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(over_the_wire(b"GET / HTTP/2\r\n\r\n").starts_with("HTTP/1.1 505"));
    assert!(
        over_the_wire(b"POST /reset HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
            .starts_with("HTTP/1.1 501")
    );
    assert!(
        over_the_wire(b"POST /reset HTTP/1.1\r\nContent-Length: 999999\r\n\r\n")
            .starts_with("HTTP/1.1 413")
    );
    assert!(over_the_wire(b"POST /reset HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc").starts_with("HTTP/1.1 400"));
}

#[test]
fn http_answers_a_stalled_client_with_a_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || crate::http::serve(listener, Arc::new(server())));

    // Connect and say nothing: the server must not hold the thread forever.
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();

    assert!(reply.starts_with("HTTP/1.1 408"), "{reply}");
}
