//! The `clientDataJSON` a client hands the authenticator.
//!
//! Apple and Android build it themselves. Windows' WebAuthn API does not: the
//! caller supplies the bytes, and the authenticator signs a hash of them, so the
//! relying party can trust only what is in here.

use crate::base64url;

/// Which ceremony the client data belongs to.
#[derive(Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum Ceremony {
    Create,
    Get,
}

/// Builds the client data for `ceremony`, with the origin the relying party
/// should expect: `https://` followed by the relying party id.
///
/// `rp_id` must already be validated: its alphabet (`a-z`, `0-9`, `-`, `.`)
/// needs no JSON escaping, which is why this does none.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn build(ceremony: Ceremony, challenge: &[u8], rp_id: &str) -> Vec<u8> {
    let kind = match ceremony {
        Ceremony::Create => "webauthn.create",
        Ceremony::Get => "webauthn.get",
    };
    format!(
        r#"{{"type":"{kind}","challenge":"{}","origin":"https://{rp_id}","crossOrigin":false}}"#,
        base64url::encode(challenge)
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_data_names_the_ceremony_challenge_and_origin() {
        let json = build(Ceremony::Create, b"\x01\x02\x03", "example.com");

        assert_eq!(
            std::str::from_utf8(&json).unwrap(),
            r#"{"type":"webauthn.create","challenge":"AQID","origin":"https://example.com","crossOrigin":false}"#
        );
    }

    #[test]
    fn get_data_uses_the_get_type() {
        let json = build(Ceremony::Get, b"c", "login.example.com");

        assert!(
            std::str::from_utf8(&json)
                .unwrap()
                .starts_with(r#"{"type":"webauthn.get","challenge":"Yw","#)
        );
    }
}
