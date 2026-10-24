//! What a ceremony hands back for the relying party's server to verify.

/// The result of creating a passkey.
///
/// Send all three fields to the server: it parses the attestation object for the
/// public key, checks the client data against the challenge it issued, and
/// stores the credential id and public key against the account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) client_data_json: Vec<u8>,
    pub(crate) attestation_object: Vec<u8>,
}

impl Registration {
    /// The new credential's id. Name it in a later request's allow list to
    /// target this passkey.
    #[inline]
    pub fn credential_id(&self) -> &[u8] {
        &self.credential_id
    }

    /// The `clientDataJSON` the authenticator signed over: the ceremony type, the
    /// challenge, and the origin.
    #[inline]
    pub fn client_data_json(&self) -> &[u8] {
        &self.client_data_json
    }

    /// The CBOR attestation object, carrying the authenticator data and the new
    /// credential's public key.
    #[inline]
    pub fn attestation_object(&self) -> &[u8] {
        &self.attestation_object
    }
}

/// The result of authenticating with a passkey.
///
/// Send it to the server, which looks the credential up by id, verifies
/// `signature` over `authenticator_data` followed by the hash of
/// `client_data_json`, and checks the challenge and origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assertion {
    pub(crate) credential_id: Vec<u8>,
    pub(crate) client_data_json: Vec<u8>,
    pub(crate) authenticator_data: Vec<u8>,
    pub(crate) signature: Vec<u8>,
    pub(crate) user_handle: Option<Vec<u8>>,
}

impl Assertion {
    /// The id of the passkey the user chose.
    #[inline]
    pub fn credential_id(&self) -> &[u8] {
        &self.credential_id
    }

    /// The `clientDataJSON` the authenticator signed over.
    #[inline]
    pub fn client_data_json(&self) -> &[u8] {
        &self.client_data_json
    }

    /// The authenticator data: relying party hash, flags, and signature counter.
    #[inline]
    pub fn authenticator_data(&self) -> &[u8] {
        &self.authenticator_data
    }

    /// The signature over the authenticator data and client data hash.
    #[inline]
    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    /// The user handle given at registration, which identifies the account when
    /// the request named no allowed credentials. `None` if the platform did not
    /// return one.
    #[inline]
    pub fn user_handle(&self) -> Option<&[u8]> {
        self.user_handle.as_deref()
    }
}
