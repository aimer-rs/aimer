//! A software authenticator for tests: produces exactly the bytes a platform
//! passkey provider would, signed with a throwaway ES256 key.

use ring::digest::{SHA256, digest};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};

use crate::base64url;

pub struct SoftAuthenticator {
    key: EcdsaKeyPair,
    rng: SystemRandom,
    pub credential_id: Vec<u8>,
    pub sign_count: u32,
}

/// What a registration ceremony hands back.
pub struct Registered {
    pub client_data_json: Vec<u8>,
    pub attestation_object: Vec<u8>,
}

/// What a sign-in ceremony hands back.
pub struct Asserted {
    pub client_data_json: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
}

impl SoftAuthenticator {
    pub fn new(credential_id: u8) -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();
        Self {
            key,
            rng,
            credential_id: vec![credential_id; 32],
            sign_count: 0,
        }
    }

    pub fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
        format!(
            r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{origin}","crossOrigin":false}}"#
        )
        .into_bytes()
    }

    fn auth_data(&self, rp_id: &str, flags: u8, attested: bool) -> Vec<u8> {
        let mut data = digest(&SHA256, rp_id.as_bytes()).as_ref().to_vec();
        data.push(flags);
        data.extend_from_slice(&self.sign_count.to_be_bytes());
        if attested {
            data.extend_from_slice(&[0u8; 16]);
            data.extend_from_slice(&(self.credential_id.len() as u16).to_be_bytes());
            data.extend_from_slice(&self.credential_id);
            data.extend_from_slice(&self.cose_key());
        }
        data
    }

    fn cose_key(&self) -> Vec<u8> {
        let point = self.key.public_key().as_ref();
        let mut key = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01];
        key.extend_from_slice(&[0x21, 0x58, 0x20]);
        key.extend_from_slice(&point[1..33]);
        key.extend_from_slice(&[0x22, 0x58, 0x20]);
        key.extend_from_slice(&point[33..65]);
        key
    }

    pub fn register(&self, challenge: &str, rp_id: &str, origin: &str) -> Registered {
        self.register_with_flags(challenge, rp_id, origin, 0x41)
    }

    pub fn register_with_flags(
        &self,
        challenge: &str,
        rp_id: &str,
        origin: &str,
        flags: u8,
    ) -> Registered {
        let auth_data = self.auth_data(rp_id, flags, true);
        // {"fmt": "none", "attStmt": {}, "authData": h'..'}
        let mut object = vec![0xa3, 0x63];
        object.extend_from_slice(b"fmt");
        object.extend_from_slice(&[0x64]);
        object.extend_from_slice(b"none");
        object.push(0x67);
        object.extend_from_slice(b"attStmt");
        object.push(0xa0);
        object.push(0x68);
        object.extend_from_slice(b"authData");
        // Byte string with a two-byte length: authData is longer than 255.
        object.extend_from_slice(&[0x59]);
        object.extend_from_slice(&(auth_data.len() as u16).to_be_bytes());
        object.extend_from_slice(&auth_data);
        Registered {
            client_data_json: Self::client_data("webauthn.create", challenge, origin),
            attestation_object: object,
        }
    }

    /// Signs in, advancing the counter first as a counting authenticator does.
    pub fn assert(&mut self, challenge: &str, rp_id: &str, origin: &str) -> Asserted {
        self.sign_count += 1;
        self.assert_with_count(challenge, rp_id, origin)
    }

    pub fn assert_with_count(&self, challenge: &str, rp_id: &str, origin: &str) -> Asserted {
        let client_data_json = Self::client_data("webauthn.get", challenge, origin);
        let authenticator_data = self.auth_data(rp_id, 0x01, false);
        let mut signed = authenticator_data.clone();
        signed.extend_from_slice(digest(&SHA256, &client_data_json).as_ref());
        let signature = self.key.sign(&self.rng, &signed).unwrap().as_ref().to_vec();
        Asserted {
            client_data_json,
            authenticator_data,
            signature,
        }
    }

    pub fn id_text(&self) -> String {
        base64url::encode(&self.credential_id)
    }
}
