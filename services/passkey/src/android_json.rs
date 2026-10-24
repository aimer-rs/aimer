//! The WebAuthn JSON that Android's Credential Manager speaks.
//!
//! Unlike the other platforms, Credential Manager takes the request as the JSON
//! form of `PublicKeyCredentialCreationOptions` / `PublicKeyCredentialRequestOptions`
//! and returns `RegistrationResponseJSON` / `AuthenticationResponseJSON`, with
//! every byte string base64url encoded. This module converts between that and
//! the crate's types. It is plain data, so it is compiled for tests everywhere.

use serde_json::{Value, json};

use crate::base64url;
use crate::request::{Attestation, CreateRequest, CredentialId, GetRequest, UserVerification};
use crate::response::{Assertion, Registration};

fn verification(preference: UserVerification) -> &'static str {
    match preference {
        UserVerification::Required => "required",
        UserVerification::Preferred => "preferred",
        UserVerification::Discouraged => "discouraged",
    }
}

fn attestation(preference: Attestation) -> &'static str {
    match preference {
        Attestation::None => "none",
        Attestation::Indirect => "indirect",
        Attestation::Direct => "direct",
        Attestation::Enterprise => "enterprise",
    }
}

fn descriptors(ids: &[CredentialId]) -> Vec<Value> {
    ids.iter()
        .map(|id| json!({ "type": "public-key", "id": base64url::encode(id.as_bytes()) }))
        .collect()
}

/// The creation options for `request`, as the JSON Credential Manager takes.
pub(crate) fn creation_options(request: &CreateRequest) -> String {
    json!({
        "rp": { "id": request.relying_party.id, "name": request.relying_party.name },
        "user": {
            "id": base64url::encode(&request.user.id),
            "name": request.user.name,
            "displayName": request.user.display_name,
        },
        "challenge": base64url::encode(&request.challenge),
        "pubKeyCredParams": request
            .algorithms
            .iter()
            .map(|algorithm| json!({ "type": "public-key", "alg": algorithm.cose_id() }))
            .collect::<Vec<_>>(),
        "timeout": request.timeout_millis(),
        "excludeCredentials": descriptors(&request.exclude),
        // A passkey is a discoverable credential.
        "authenticatorSelection": {
            "residentKey": "required",
            "requireResidentKey": true,
            "userVerification": verification(request.user_verification),
        },
        "attestation": attestation(request.attestation),
    })
    .to_string()
}

/// The request options for `request`, as the JSON Credential Manager takes.
pub(crate) fn request_options(request: &GetRequest) -> String {
    json!({
        "challenge": base64url::encode(&request.challenge),
        "rpId": request.rp_id,
        "timeout": request.timeout_millis(),
        "userVerification": verification(request.user_verification),
        "allowCredentials": descriptors(&request.allow),
    })
    .to_string()
}

fn bytes(value: &Value, key: &str) -> Option<Vec<u8>> {
    base64url::decode(value.get(key)?.as_str()?)
}

/// Reads a `RegistrationResponseJSON`. `None` if a required field is missing or
/// is not base64url.
pub(crate) fn parse_registration(json: &str) -> Option<Registration> {
    let root: Value = serde_json::from_str(json).ok()?;
    let response = root.get("response")?;
    Some(Registration {
        credential_id: bytes(&root, "rawId")?,
        client_data_json: bytes(response, "clientDataJSON")?,
        attestation_object: bytes(response, "attestationObject")?,
    })
}

/// Reads an `AuthenticationResponseJSON`. `None` if a required field is missing
/// or is not base64url. A missing or empty `userHandle` is not an error.
pub(crate) fn parse_assertion(json: &str) -> Option<Assertion> {
    let root: Value = serde_json::from_str(json).ok()?;
    let response = root.get("response")?;
    let user_handle = match response.get("userHandle") {
        None | Some(Value::Null) => None,
        Some(Value::String(handle)) => Some(base64url::decode(handle)?).filter(|h| !h.is_empty()),
        Some(_) => return None,
    };
    Some(Assertion {
        credential_id: bytes(&root, "rawId")?,
        client_data_json: bytes(response, "clientDataJSON")?,
        authenticator_data: bytes(response, "authenticatorData")?,
        signature: bytes(response, "signature")?,
        user_handle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Algorithm, RelyingParty, User};

    fn create() -> CreateRequest {
        CreateRequest::new(
            RelyingParty::new("example.com", "Example"),
            User::new(vec![1, 2, 3], "alice").display_name("Alice"),
            vec![0xfb, 0xff],
        )
        .algorithms([Algorithm::Es256])
        .exclude([CredentialId::new(vec![9, 9])])
        .user_verification(UserVerification::Required)
        .timeout(std::time::Duration::from_secs(60))
    }

    #[test]
    fn creation_options_follow_the_webauthn_json_shape() {
        let options: Value = serde_json::from_str(&creation_options(&create())).unwrap();

        assert_eq!(options["rp"], json!({ "id": "example.com", "name": "Example" }));
        assert_eq!(
            options["user"],
            json!({ "id": "AQID", "name": "alice", "displayName": "Alice" })
        );
        assert_eq!(options["challenge"], "-_8");
        assert_eq!(options["pubKeyCredParams"], json!([{ "type": "public-key", "alg": -7 }]));
        assert_eq!(options["timeout"], 60_000);
        assert_eq!(options["excludeCredentials"], json!([{ "type": "public-key", "id": "CQk" }]));
        assert_eq!(options["authenticatorSelection"]["residentKey"], "required");
        assert_eq!(options["authenticatorSelection"]["userVerification"], "required");
        assert_eq!(options["attestation"], "none");
    }

    #[test]
    fn request_options_carry_the_relying_party_and_allow_list() {
        let request = GetRequest::new("example.com", vec![1, 2, 3])
            .allow([CredentialId::new(vec![4, 5, 6])])
            .user_verification(UserVerification::Discouraged);
        let options: Value = serde_json::from_str(&request_options(&request)).unwrap();

        assert_eq!(options["rpId"], "example.com");
        assert_eq!(options["challenge"], "AQID");
        assert_eq!(options["userVerification"], "discouraged");
        assert_eq!(options["allowCredentials"], json!([{ "type": "public-key", "id": "BAUG" }]));
        assert_eq!(options["timeout"], 300_000);
    }

    #[test]
    fn text_in_a_request_is_escaped_by_the_serializer() {
        let request = CreateRequest::new(
            RelyingParty::new("example.com", "Ex\"ample\\"),
            User::new(vec![1], "a\"b\nc"),
            vec![1],
        );
        let options: Value = serde_json::from_str(&creation_options(&request)).unwrap();

        assert_eq!(options["rp"]["name"], "Ex\"ample\\");
        assert_eq!(options["user"]["name"], "a\"b\nc");
    }

    #[test]
    fn a_registration_response_is_read() {
        let json = r#"{"id":"AQID","rawId":"AQID","type":"public-key",
            "response":{"clientDataJSON":"e30","attestationObject":"o2E","transports":["internal"]},
            "clientExtensionResults":{}}"#;

        let registration = parse_registration(json).unwrap();

        assert_eq!(registration.credential_id(), [1, 2, 3]);
        assert_eq!(registration.client_data_json(), b"{}");
        assert_eq!(registration.attestation_object(), [0xa3, 0x61]);
    }

    #[test]
    fn an_assertion_response_is_read_with_and_without_a_user_handle() {
        let with = r#"{"rawId":"AQID","response":{"clientDataJSON":"e30","authenticatorData":"AQ",
            "signature":"Ag","userHandle":"Aw"}}"#;
        let assertion = parse_assertion(with).unwrap();
        assert_eq!(assertion.credential_id(), [1, 2, 3]);
        assert_eq!(assertion.authenticator_data(), [1]);
        assert_eq!(assertion.signature(), [2]);
        assert_eq!(assertion.user_handle(), Some(&[3][..]));

        for without in [
            r#"{"rawId":"AQID","response":{"clientDataJSON":"e30","authenticatorData":"AQ","signature":"Ag"}}"#,
            r#"{"rawId":"AQID","response":{"clientDataJSON":"e30","authenticatorData":"AQ","signature":"Ag","userHandle":null}}"#,
            r#"{"rawId":"AQID","response":{"clientDataJSON":"e30","authenticatorData":"AQ","signature":"Ag","userHandle":""}}"#,
        ] {
            assert_eq!(parse_assertion(without).unwrap().user_handle(), None, "{without}");
        }
    }

    #[test]
    fn malformed_responses_are_rejected() {
        for bad in [
            "",
            "[]",
            "{}",
            r#"{"rawId":"AQID"}"#,
            r#"{"rawId":"AQID","response":{"clientDataJSON":"e30"}}"#,
            r#"{"rawId":"not base64!","response":{"clientDataJSON":"e30","attestationObject":"o2E"}}"#,
            r#"{"rawId":"AQID","response":{"clientDataJSON":"e30","attestationObject":7}}"#,
        ] {
            assert!(parse_registration(bad).is_none(), "{bad}");
        }
        let bad_handle = r#"{"rawId":"AQID","response":{"clientDataJSON":"e30","authenticatorData":"AQ","signature":"Ag","userHandle":5}}"#;
        assert!(parse_assertion(bad_handle).is_none());
        assert!(parse_assertion("{}").is_none());
    }
}
