//! The JSON the example exchanges with the local passkey server.
//!
//! Binary fields travel as unpadded base64url, the encoding WebAuthn uses on
//! the web, so the same server could front a browser client too.

use serde_json::{Value, json};

use super::base64url;

/// What the server wants a new passkey to look like.
#[derive(Debug, Eq, PartialEq)]
pub struct RegisterOptions {
    /// The challenge exactly as the server wrote it; it is echoed back verbatim.
    pub challenge_text: String,
    pub challenge: Vec<u8>,
    pub rp_id: String,
    pub rp_name: String,
    pub user_id: Vec<u8>,
    pub user_name: String,
    /// Passkeys the account already has, so the platform skips this authenticator.
    pub exclude: Vec<Vec<u8>>,
}

/// What the server wants a sign-in to prove.
#[derive(Debug, Eq, PartialEq)]
pub struct LoginOptions {
    pub challenge_text: String,
    pub challenge: Vec<u8>,
    pub rp_id: String,
}

fn object(body: &str) -> Result<Value, String> {
    match serde_json::from_str::<Value>(body) {
        Ok(json) if json.is_object() => Ok(json),
        _ => Err("the server did not answer with a JSON object".to_owned()),
    }
}

fn text<'a>(json: &'a Value, key: &str) -> Result<&'a str, String> {
    json.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the server's answer has no \"{key}\""))
}

fn bytes(json: &Value, key: &str) -> Result<Vec<u8>, String> {
    base64url::decode(text(json, key)?).ok_or_else(|| format!("\"{key}\" is not base64url"))
}

/// The server's own `{"error": ...}` message, if the body carries one.
pub fn server_error(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_owned)
}

/// The body that asks for registration options for `username`.
pub fn register_options_request(username: &str) -> String {
    json!({ "username": username }).to_string()
}

/// Reads the server's registration options.
pub fn parse_register_options(body: &str) -> Result<RegisterOptions, String> {
    let json = object(body)?;
    let exclude = json
        .get("excludeCredentials")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .map(|id| {
                    id.as_str()
                        .and_then(base64url::decode)
                        .ok_or_else(|| "an excluded credential id is not base64url".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(RegisterOptions {
        challenge_text: text(&json, "challenge")?.to_owned(),
        challenge: bytes(&json, "challenge")?,
        rp_id: text(&json, "rpId")?.to_owned(),
        rp_name: text(&json, "rpName")?.to_owned(),
        user_id: bytes(&json, "userId")?,
        user_name: text(&json, "userName")?.to_owned(),
        exclude,
    })
}

/// Reads the server's sign-in options.
pub fn parse_login_options(body: &str) -> Result<LoginOptions, String> {
    let json = object(body)?;
    Ok(LoginOptions {
        challenge_text: text(&json, "challenge")?.to_owned(),
        challenge: bytes(&json, "challenge")?,
        rp_id: text(&json, "rpId")?.to_owned(),
    })
}

/// The bytes of a created passkey the server verifies.
pub struct RegistrationParts<'a> {
    pub credential_id: &'a [u8],
    pub client_data_json: &'a [u8],
    pub attestation_object: &'a [u8],
}

/// The bytes of a sign-in proof the server verifies.
pub struct AssertionParts<'a> {
    pub credential_id: &'a [u8],
    pub client_data_json: &'a [u8],
    pub authenticator_data: &'a [u8],
    pub signature: &'a [u8],
    pub user_handle: Option<&'a [u8]>,
}

/// The body that hands a new passkey to the server.
pub fn register_verify_body(challenge: &str, parts: &RegistrationParts<'_>) -> String {
    json!({
        "challenge": challenge,
        "credentialId": base64url::encode(parts.credential_id),
        "clientDataJSON": base64url::encode(parts.client_data_json),
        "attestationObject": base64url::encode(parts.attestation_object),
    })
    .to_string()
}

/// The body that hands a sign-in proof to the server.
pub fn login_verify_body(challenge: &str, parts: &AssertionParts<'_>) -> String {
    let mut body = json!({
        "challenge": challenge,
        "credentialId": base64url::encode(parts.credential_id),
        "clientDataJSON": base64url::encode(parts.client_data_json),
        "authenticatorData": base64url::encode(parts.authenticator_data),
        "signature": base64url::encode(parts.signature),
    });
    if let Some(handle) = parts.user_handle {
        body["userHandle"] = json!(base64url::encode(handle));
    }
    body.to_string()
}

/// One line for the log from a successful registration answer.
pub fn describe_registration(body: &str) -> Result<String, String> {
    let json = object(body)?;
    let passkeys = json
        .get("passkeys")
        .and_then(Value::as_u64)
        .ok_or("the server's answer has no \"passkeys\"")?;
    Ok(format!(
        "Registered a passkey for {} ({passkeys} on the account)",
        text(&json, "username")?
    ))
}

/// One line for the log from a successful sign-in answer.
pub fn describe_login(body: &str) -> Result<String, String> {
    let json = object(body)?;
    Ok(format!("Signed in as {}", text(&json, "username")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTER_OPTIONS: &str = r#"{
        "challenge": "AQIDBA", "rpId": "example.com", "rpName": "Example",
        "userId": "BQYH", "userName": "alice", "excludeCredentials": ["CAkK"]
    }"#;

    #[test]
    fn registration_options_are_decoded_and_the_challenge_text_is_kept() {
        let options = parse_register_options(REGISTER_OPTIONS).unwrap();

        assert_eq!(options.challenge_text, "AQIDBA");
        assert_eq!(options.challenge, [1, 2, 3, 4]);
        assert_eq!(options.rp_id, "example.com");
        assert_eq!(options.rp_name, "Example");
        assert_eq!(options.user_id, [5, 6, 7]);
        assert_eq!(options.user_name, "alice");
        assert_eq!(options.exclude, vec![vec![8, 9, 10]]);
    }

    #[test]
    fn a_missing_exclude_list_means_no_exclusions() {
        let body = r#"{"challenge":"AQ","rpId":"a.b","rpName":"n","userId":"AQ","userName":"u"}"#;

        assert!(parse_register_options(body).unwrap().exclude.is_empty());
    }

    #[test]
    fn login_options_are_decoded() {
        let options = parse_login_options(r#"{"challenge":"AQIDBA","rpId":"localhost"}"#).unwrap();

        assert_eq!(
            options,
            LoginOptions {
                challenge_text: "AQIDBA".to_owned(),
                challenge: vec![1, 2, 3, 4],
                rp_id: "localhost".to_owned(),
            }
        );
    }

    #[test]
    fn malformed_answers_name_what_is_wrong() {
        assert!(parse_register_options("<html>").unwrap_err().contains("JSON object"));
        assert!(parse_register_options("[]").unwrap_err().contains("JSON object"));
        assert!(parse_login_options("{}").unwrap_err().contains("\"challenge\""));
        assert!(
            parse_login_options(r#"{"challenge":"+++","rpId":"x"}"#)
                .unwrap_err()
                .contains("base64url")
        );
        let bad_exclude = REGISTER_OPTIONS.replace("CAkK", "!!");
        assert!(parse_register_options(&bad_exclude).unwrap_err().contains("excluded"));
    }

    #[test]
    fn the_servers_error_message_is_extracted_when_present() {
        assert_eq!(server_error(r#"{"error":"nope"}"#).as_deref(), Some("nope"));
        assert_eq!(server_error(r#"{"ok":true}"#), None);
        assert_eq!(server_error("not json"), None);
    }

    #[test]
    fn the_options_request_escapes_the_username() {
        let body = register_options_request("a\"b\\c");

        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["username"], "a\"b\\c");
    }

    #[test]
    fn verify_bodies_use_the_field_names_the_server_reads() {
        let registration = register_verify_body(
            "AQ",
            &RegistrationParts {
                credential_id: &[1],
                client_data_json: &[2],
                attestation_object: &[3],
            },
        );
        let parsed: Value = serde_json::from_str(&registration).unwrap();
        assert_eq!(
            parsed,
            json!({"challenge":"AQ","credentialId":"AQ","clientDataJSON":"Ag","attestationObject":"Aw"})
        );

        let parts = AssertionParts {
            credential_id: &[1],
            client_data_json: &[2],
            authenticator_data: &[3],
            signature: &[4],
            user_handle: None,
        };
        let anonymous: Value = serde_json::from_str(&login_verify_body("AQ", &parts)).unwrap();
        assert_eq!(anonymous["signature"], "BA");
        assert!(anonymous.get("userHandle").is_none(), "absent, not null");

        let with_handle = AssertionParts {
            user_handle: Some(&[5]),
            ..parts
        };
        let named: Value = serde_json::from_str(&login_verify_body("AQ", &with_handle)).unwrap();
        assert_eq!(named["userHandle"], "BQ");
    }

    #[test]
    fn success_answers_become_log_lines() {
        assert_eq!(
            describe_registration(r#"{"ok":true,"username":"alice","passkeys":2}"#).unwrap(),
            "Registered a passkey for alice (2 on the account)"
        );
        assert_eq!(
            describe_login(r#"{"ok":true,"username":"alice","signCount":1}"#).unwrap(),
            "Signed in as alice"
        );
        assert!(describe_login("{}").is_err());
    }
}
