//! Checking what an authenticator returned.
//!
//! This follows the WebAuthn "registering" and "verifying an assertion"
//! procedures for the one configuration the example asks for: ES256 passkeys
//! with `none` attestation. The attestation statement is therefore not
//! evaluated, which is what a relying party that does not vet authenticator
//! models does anyway.

use ring::digest::{SHA256, digest};
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};

use crate::cbor::{self, Value};

/// Authenticator data flag: the user was present.
const FLAG_USER_PRESENT: u8 = 0x01;
/// Authenticator data flag: attested credential data follows.
const FLAG_ATTESTED: u8 = 0x40;
/// Prefix of the origin an Android app signs, in place of an `https` one.
const ANDROID_ORIGIN_PREFIX: &str = "android:apk-key-hash:";

/// Why a ceremony result was refused.
#[derive(Debug, Eq, PartialEq)]
pub enum VerifyError {
    /// `clientDataJSON` is not the JSON object WebAuthn defines.
    ClientData,
    /// `clientDataJSON` is for the other ceremony.
    WrongCeremony,
    /// The challenge is not the one this server issued.
    Challenge,
    /// The origin is not this relying party's.
    Origin,
    /// Authenticator data or the attestation object is malformed.
    Malformed(&'static str),
    /// The authenticator data is for a different relying party id.
    RpIdHash,
    /// The user was not present.
    UserPresence,
    /// The credential's key is not an ES256 key.
    Algorithm,
    /// The signature does not verify under the stored public key.
    Signature,
    /// The signature counter went backwards: a cloned authenticator.
    Counter,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientData => f.write_str("clientDataJSON is malformed"),
            Self::WrongCeremony => f.write_str("clientDataJSON is for the other ceremony"),
            Self::Challenge => f.write_str("the challenge does not match the one issued"),
            Self::Origin => f.write_str("the origin is not this relying party's"),
            Self::Malformed(what) => write!(f, "malformed {what}"),
            Self::RpIdHash => f.write_str("the rpIdHash is for a different relying party"),
            Self::UserPresence => f.write_str("the user was not present"),
            Self::Algorithm => f.write_str("the credential key is not ES256"),
            Self::Signature => f.write_str("the signature does not verify"),
            Self::Counter => f.write_str("the signature counter did not increase"),
        }
    }
}

/// Which ceremony a `clientDataJSON` must belong to.
#[derive(Clone, Copy)]
pub enum Ceremony {
    Create,
    Get,
}

/// Checks `clientDataJSON` against the ceremony, the issued challenge, and the
/// relying party's origin.
///
/// `challenge` is the base64url text the server issued, compared as text: the
/// client echoes it back verbatim.
pub fn check_client_data(
    client_data_json: &[u8],
    ceremony: Ceremony,
    challenge: &str,
    rp_id: &str,
) -> Result<(), VerifyError> {
    let json: serde_json::Value =
        serde_json::from_slice(client_data_json).map_err(|_| VerifyError::ClientData)?;
    let field = |name| json.get(name).and_then(serde_json::Value::as_str);

    let expected = match ceremony {
        Ceremony::Create => "webauthn.create",
        Ceremony::Get => "webauthn.get",
    };
    if field("type").ok_or(VerifyError::ClientData)? != expected {
        return Err(VerifyError::WrongCeremony);
    }
    if field("challenge").ok_or(VerifyError::ClientData)? != challenge {
        return Err(VerifyError::Challenge);
    }
    let origin = field("origin").ok_or(VerifyError::ClientData)?;
    // Android signs the app's certificate hash, not a web origin; trusting the
    // prefix is acceptable here because the platform has already matched the
    // app to this domain through `assetlinks.json`.
    if origin != format!("https://{rp_id}") && !origin.starts_with(ANDROID_ORIGIN_PREFIX) {
        return Err(VerifyError::Origin);
    }
    Ok(())
}

/// The fixed head of authenticator data.
struct AuthData<'a> {
    rp_id_hash: &'a [u8],
    flags: u8,
    sign_count: u32,
    /// Attested credential data and extensions, when `flags` says it is there.
    tail: &'a [u8],
}

fn parse_auth_data(data: &[u8]) -> Result<AuthData<'_>, VerifyError> {
    if data.len() < 37 {
        return Err(VerifyError::Malformed("authenticator data"));
    }
    Ok(AuthData {
        rp_id_hash: &data[..32],
        flags: data[32],
        sign_count: u32::from_be_bytes(data[33..37].try_into().unwrap()),
        tail: &data[37..],
    })
}

fn check_rp_and_presence(auth: &AuthData<'_>, rp_id: &str) -> Result<(), VerifyError> {
    if auth.rp_id_hash != digest(&SHA256, rp_id.as_bytes()).as_ref() {
        return Err(VerifyError::RpIdHash);
    }
    if auth.flags & FLAG_USER_PRESENT == 0 {
        return Err(VerifyError::UserPresence);
    }
    Ok(())
}

/// A credential accepted at registration.
#[derive(Debug, Eq, PartialEq)]
pub struct NewCredential {
    /// The uncompressed SEC1 point (`0x04 || x || y`) `ring` verifies against.
    pub public_key: Vec<u8>,
    pub sign_count: u32,
}

/// Verifies a registration and extracts the credential to store.
///
/// `credential_id` is the id the client reported; it must match the one the
/// authenticator put in the signed attested data.
pub fn verify_registration(
    attestation_object: &[u8],
    credential_id: &[u8],
    rp_id: &str,
) -> Result<NewCredential, VerifyError> {
    let (object, _) = cbor::decode(attestation_object)
        .map_err(|_| VerifyError::Malformed("attestation object"))?;
    let auth_data = object
        .get_text("authData")
        .and_then(Value::as_bytes)
        .ok_or(VerifyError::Malformed("attestation object"))?;

    let auth = parse_auth_data(auth_data)?;
    check_rp_and_presence(&auth, rp_id)?;
    if auth.flags & FLAG_ATTESTED == 0 {
        return Err(VerifyError::Malformed("authenticator data"));
    }

    // aaguid (16) | credential id length (2) | credential id | COSE key.
    let attested = auth.tail;
    let id_len = attested
        .get(16..18)
        .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
        .ok_or(VerifyError::Malformed("attested credential data"))?;
    let id = attested
        .get(18..18 + id_len)
        .ok_or(VerifyError::Malformed("attested credential data"))?;
    if id != credential_id {
        return Err(VerifyError::Malformed("credential id"));
    }
    let (key, _) = cbor::decode(&attested[18 + id_len..])
        .map_err(|_| VerifyError::Malformed("credential public key"))?;

    // COSE EC2 (kty 2), ES256 (alg -7), P-256 (crv 1).
    let int = |label| key.get_int(label).and_then(Value::as_int);
    if int(1) != Some(2) || int(3) != Some(-7) || int(-1) != Some(1) {
        return Err(VerifyError::Algorithm);
    }
    let coordinate = |label| {
        key.get_int(label)
            .and_then(Value::as_bytes)
            .filter(|c| c.len() == 32)
            .ok_or(VerifyError::Malformed("credential public key"))
    };
    let mut public_key = Vec::with_capacity(65);
    public_key.push(0x04);
    public_key.extend_from_slice(coordinate(-2)?);
    public_key.extend_from_slice(coordinate(-3)?);

    Ok(NewCredential {
        public_key,
        sign_count: auth.sign_count,
    })
}

/// Verifies a sign-in assertion against a stored credential and returns the new
/// signature counter.
pub fn verify_assertion(
    authenticator_data: &[u8],
    client_data_json: &[u8],
    signature: &[u8],
    public_key: &[u8],
    stored_sign_count: u32,
    rp_id: &str,
) -> Result<u32, VerifyError> {
    let auth = parse_auth_data(authenticator_data)?;
    check_rp_and_presence(&auth, rp_id)?;

    // The authenticator signs authenticatorData || SHA-256(clientDataJSON).
    let mut signed = authenticator_data.to_vec();
    signed.extend_from_slice(digest(&SHA256, client_data_json).as_ref());
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_key)
        .verify(&signed, signature)
        .map_err(|_| VerifyError::Signature)?;

    // Platform authenticators that keep no counter report zero forever; only a
    // counter that is in use can go backwards.
    if (auth.sign_count != 0 || stored_sign_count != 0) && auth.sign_count <= stored_sign_count {
        return Err(VerifyError::Counter);
    }
    Ok(auth.sign_count)
}
