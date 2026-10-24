//! The arguments of a passkey ceremony.
//!
//! These follow WebAuthn's `PublicKeyCredentialCreationOptions` and
//! `PublicKeyCredentialRequestOptions`, trimmed to what every platform's native
//! API can honor. Passkeys are always discoverable credentials stored by the
//! platform authenticator, so there is no option to ask for anything else.

use std::time::Duration;

use crate::error::{InvalidRequest, PasskeyResult};

/// The longest challenge accepted. WebAuthn asks for at least 16 random bytes;
/// the cap only keeps a bug from handing the platform megabytes.
pub const MAX_CHALLENGE_BYTES: usize = 1024;
/// The longest user handle WebAuthn allows.
pub const MAX_USER_ID_BYTES: usize = 64;
/// The longest credential id WebAuthn allows.
pub const MAX_CREDENTIAL_ID_BYTES: usize = 1023;
/// The most credential ids one request may list.
pub const MAX_CREDENTIALS: usize = 64;
const MAX_RP_ID_BYTES: usize = 253;
/// How long a ceremony waits for the user when the request sets no timeout.
/// Matches the five minutes browsers allow.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// The request's timeout in whole milliseconds, for APIs that take a `u32`.
#[cfg_attr(not(any(windows, target_os = "android")), allow(dead_code))]
fn timeout_millis(timeout: Option<Duration>) -> u32 {
    u32::try_from(timeout.unwrap_or(DEFAULT_TIMEOUT).as_millis()).unwrap_or(u32::MAX)
}

/// The service a passkey belongs to, identified by a domain.
///
/// The domain must be one the application is associated with: the relying
/// party publishes the link (`apple-app-site-association`, `assetlinks.json`)
/// and the platform checks it before any ceremony.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelyingParty {
    pub(crate) id: String,
    pub(crate) name: String,
}

impl RelyingParty {
    /// Creates a relying party from its domain (`"example.com"`) and a display
    /// name for it.
    ///
    /// The id is validated when a request is made, not here.
    #[inline]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }

    /// The relying party's domain.
    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    fn validate(&self) -> PasskeyResult<()> {
        validate_rp_id(&self.id)?;
        if self.name.is_empty() {
            return Err(InvalidRequest::RelyingPartyName.into());
        }
        Ok(())
    }
}

/// The account a new passkey is for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct User {
    pub(crate) id: Vec<u8>,
    pub(crate) name: String,
    pub(crate) display_name: String,
}

impl User {
    /// Creates a user from an opaque handle and an account name.
    ///
    /// `id` is the server's stable identifier for the account: random bytes, not
    /// an email address or anything personally identifying, since the platform
    /// stores it beside the passkey and returns it on sign-in. `name` is what the
    /// user recognizes the account by, usually an email address or username.
    /// The display name defaults to `name`.
    #[inline]
    pub fn new(id: impl Into<Vec<u8>>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: id.into(),
            display_name: name.clone(),
            name,
        }
    }

    /// Sets the friendly name shown beside the account, such as `"Alice Smith"`.
    #[inline]
    pub fn display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    fn validate(&self) -> PasskeyResult<()> {
        if self.id.is_empty() || self.id.len() > MAX_USER_ID_BYTES {
            return Err(InvalidRequest::UserId.into());
        }
        if self.name.is_empty() {
            return Err(InvalidRequest::UserName.into());
        }
        Ok(())
    }
}

/// A signature algorithm a new passkey may use.
///
/// Apple platforms only create ES256 passkeys and ignore the list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Algorithm {
    /// ECDSA over P-256 with SHA-256 (COSE `-7`). Supported everywhere.
    Es256,
    /// RSASSA-PKCS1-v1_5 with SHA-256 (COSE `-257`).
    Rs256,
}

impl Algorithm {
    /// The COSE algorithm identifier.
    #[inline]
    pub const fn cose_id(self) -> i32 {
        match self {
            Self::Es256 => -7,
            Self::Rs256 => -257,
        }
    }
}

/// Whether the user must prove who they are (biometrics, PIN) and not only be
/// present.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UserVerification {
    /// Fail if the user cannot be verified.
    Required,
    /// Verify when the device can, and carry on when it cannot.
    #[default]
    Preferred,
    /// Skip verification when possible.
    Discouraged,
}

/// How much proof of the authenticator's make and model to return.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Attestation {
    /// None. The right choice unless the server vets authenticator models.
    #[default]
    None,
    /// A statement the platform may anonymize.
    Indirect,
    /// The authenticator's own statement.
    Direct,
    /// An identifying statement, for managed enterprise devices only.
    Enterprise,
}

/// A credential id, naming one passkey.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CredentialId(Vec<u8>);

impl CredentialId {
    /// Wraps the id exactly as the server stored it from a [`Registration`] or
    /// [`Assertion`].
    ///
    /// [`Registration`]: crate::Registration
    /// [`Assertion`]: crate::Assertion
    #[inline]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// The id's bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for CredentialId {
    #[inline]
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

fn validate_credentials(ids: &[CredentialId]) -> PasskeyResult<()> {
    if ids.len() > MAX_CREDENTIALS {
        return Err(InvalidRequest::TooManyCredentials.into());
    }
    if ids
        .iter()
        .any(|id| id.0.is_empty() || id.0.len() > MAX_CREDENTIAL_ID_BYTES)
    {
        return Err(InvalidRequest::CredentialId.into());
    }
    Ok(())
}

fn validate_challenge(challenge: &[u8]) -> PasskeyResult<()> {
    if challenge.is_empty() || challenge.len() > MAX_CHALLENGE_BYTES {
        return Err(InvalidRequest::Challenge.into());
    }
    Ok(())
}

/// A relying party id is a registrable domain name: dot-separated labels of
/// lowercase ASCII letters, digits, and inner hyphens. That excludes a scheme,
/// port, or path, and it is also what makes the id safe to splice into the
/// origin Windows signs.
fn validate_rp_id(id: &str) -> PasskeyResult<()> {
    let valid = !id.is_empty()
        && id.len() <= MAX_RP_ID_BYTES
        && id.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(InvalidRequest::RelyingPartyId.into())
    }
}

/// A request to create a passkey.
///
/// # Examples
///
/// ```
/// use aimer_passkey::{Attestation, CreateRequest, RelyingParty, User, UserVerification};
///
/// // The challenge comes from the server, fresh for this ceremony.
/// let challenge = vec![0x5a; 32];
/// let request = CreateRequest::new(
///     RelyingParty::new("example.com", "Example"),
///     User::new(vec![0x11; 16], "alice@example.com").display_name("Alice"),
///     challenge,
/// )
/// .user_verification(UserVerification::Required)
/// .attestation(Attestation::None);
/// # let _ = request;
/// ```
#[derive(Clone, Debug)]
pub struct CreateRequest {
    pub(crate) relying_party: RelyingParty,
    pub(crate) user: User,
    pub(crate) challenge: Vec<u8>,
    pub(crate) algorithms: Vec<Algorithm>,
    pub(crate) exclude: Vec<CredentialId>,
    pub(crate) user_verification: UserVerification,
    pub(crate) attestation: Attestation,
    pub(crate) timeout: Option<Duration>,
}

impl CreateRequest {
    /// Starts a request for `user` at `relying_party`, answering `challenge`.
    ///
    /// Signature algorithms default to ES256 then RS256, verification to
    /// preferred, and attestation to none.
    #[inline]
    pub fn new(relying_party: RelyingParty, user: User, challenge: impl Into<Vec<u8>>) -> Self {
        Self {
            relying_party,
            user,
            challenge: challenge.into(),
            algorithms: vec![Algorithm::Es256, Algorithm::Rs256],
            exclude: Vec::new(),
            user_verification: UserVerification::default(),
            attestation: Attestation::default(),
            timeout: None,
        }
    }

    /// Replaces the acceptable signature algorithms, most preferred first.
    #[inline]
    pub fn algorithms(mut self, algorithms: impl IntoIterator<Item = Algorithm>) -> Self {
        self.algorithms = algorithms.into_iter().collect();
        self
    }

    /// Refuses to create a second passkey if one of `credentials` already lives
    /// on this device, so a user is not offered a duplicate. Ignored where the
    /// platform cannot check.
    #[inline]
    pub fn exclude(mut self, credentials: impl IntoIterator<Item = CredentialId>) -> Self {
        self.exclude = credentials.into_iter().collect();
        self
    }

    /// Sets whether the user must be verified.
    #[inline]
    pub fn user_verification(mut self, user_verification: UserVerification) -> Self {
        self.user_verification = user_verification;
        self
    }

    /// Sets the attestation conveyance preference.
    #[inline]
    pub fn attestation(mut self, attestation: Attestation) -> Self {
        self.attestation = attestation;
        self
    }

    /// Sets how long the platform waits for the user. Ignored where the platform
    /// has no timeout.
    #[inline]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The timeout in milliseconds, with the default applied.
    #[cfg_attr(not(any(windows, target_os = "android")), allow(dead_code))]
    pub(crate) fn timeout_millis(&self) -> u32 {
        timeout_millis(self.timeout)
    }

    /// Checks the request without touching the platform.
    pub(crate) fn validate(&self) -> PasskeyResult<()> {
        self.relying_party.validate()?;
        self.user.validate()?;
        validate_challenge(&self.challenge)?;
        if self.algorithms.is_empty() {
            return Err(InvalidRequest::NoAlgorithms.into());
        }
        validate_credentials(&self.exclude)
    }
}

/// A request to sign in with a passkey.
///
/// # Examples
///
/// ```
/// use aimer_passkey::{CredentialId, GetRequest};
///
/// // No allow list: the system offers every passkey for the domain.
/// let any = GetRequest::new("example.com", vec![0x5a; 32]);
///
/// // Or pin the ceremony to the account the user named.
/// let one = GetRequest::new("example.com", vec![0x5a; 32])
///     .allow([CredentialId::new(vec![1, 2, 3])]);
/// # let _ = (any, one);
/// ```
#[derive(Clone, Debug)]
pub struct GetRequest {
    pub(crate) rp_id: String,
    pub(crate) challenge: Vec<u8>,
    pub(crate) allow: Vec<CredentialId>,
    pub(crate) user_verification: UserVerification,
    pub(crate) timeout: Option<Duration>,
}

impl GetRequest {
    /// Starts a request at the relying party with domain `rp_id`, answering
    /// `challenge`.
    #[inline]
    pub fn new(rp_id: impl Into<String>, challenge: impl Into<Vec<u8>>) -> Self {
        Self {
            rp_id: rp_id.into(),
            challenge: challenge.into(),
            allow: Vec::new(),
            user_verification: UserVerification::default(),
            timeout: None,
        }
    }

    /// Restricts the ceremony to these passkeys. With none listed, the user
    /// picks from every passkey the platform holds for the domain.
    #[inline]
    pub fn allow(mut self, credentials: impl IntoIterator<Item = CredentialId>) -> Self {
        self.allow = credentials.into_iter().collect();
        self
    }

    /// Sets whether the user must be verified.
    #[inline]
    pub fn user_verification(mut self, user_verification: UserVerification) -> Self {
        self.user_verification = user_verification;
        self
    }

    /// Sets how long the platform waits for the user. Ignored where the platform
    /// has no timeout.
    #[inline]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The timeout in milliseconds, with the default applied.
    #[cfg_attr(not(any(windows, target_os = "android")), allow(dead_code))]
    pub(crate) fn timeout_millis(&self) -> u32 {
        timeout_millis(self.timeout)
    }

    /// Checks the request without touching the platform.
    pub(crate) fn validate(&self) -> PasskeyResult<()> {
        validate_rp_id(&self.rp_id)?;
        validate_challenge(&self.challenge)?;
        validate_credentials(&self.allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PasskeyError;

    fn create() -> CreateRequest {
        CreateRequest::new(
            RelyingParty::new("example.com", "Example"),
            User::new(vec![1; 16], "alice"),
            vec![7; 32],
        )
    }

    fn invalid(result: PasskeyResult<()>) -> InvalidRequest {
        match result {
            Err(PasskeyError::InvalidRequest(reason)) => reason,
            other => panic!("expected an invalid request, got {other:?}"),
        }
    }

    #[test]
    fn a_complete_request_is_valid() {
        assert_eq!(create().validate(), Ok(()));
        assert_eq!(GetRequest::new("login.example.co.uk", vec![1]).validate(), Ok(()));
    }

    #[test]
    fn relying_party_ids_must_be_bare_lowercase_domains() {
        for bad in [
            "",
            "https://example.com",
            "example.com:8443",
            "example.com/path",
            "Example.com",
            "exa mple.com",
            ".example.com",
            "example.com.",
            "a..b",
            "-a.com",
            "a-.com",
            "bücher.example",
            &"a".repeat(64),
            &format!("{}.com", ["abcdefghi"; 28].join(".")),
        ] {
            assert_eq!(
                invalid(GetRequest::new(bad, vec![1]).validate()),
                InvalidRequest::RelyingPartyId,
                "{bad:?}"
            );
        }
        assert_eq!(GetRequest::new("xn--bcher-kva.example", vec![1]).validate(), Ok(()));
        assert_eq!(GetRequest::new("localhost", vec![1]).validate(), Ok(()));
    }

    #[test]
    fn challenges_are_bounded() {
        for bad in [Vec::new(), vec![0; MAX_CHALLENGE_BYTES + 1]] {
            assert_eq!(
                invalid(GetRequest::new("example.com", bad.clone()).validate()),
                InvalidRequest::Challenge
            );
            assert_eq!(
                invalid(CreateRequest { challenge: bad, ..create() }.validate()),
                InvalidRequest::Challenge
            );
        }
        assert_eq!(
            GetRequest::new("example.com", vec![0; MAX_CHALLENGE_BYTES]).validate(),
            Ok(())
        );
    }

    #[test]
    fn users_need_a_bounded_handle_and_a_name() {
        let with = |user| CreateRequest { user, ..create() };

        assert_eq!(invalid(with(User::new(Vec::new(), "a")).validate()), InvalidRequest::UserId);
        assert_eq!(
            invalid(with(User::new(vec![0; MAX_USER_ID_BYTES + 1], "a")).validate()),
            InvalidRequest::UserId
        );
        assert_eq!(invalid(with(User::new(vec![1], "")).validate()), InvalidRequest::UserName);
        assert_eq!(with(User::new(vec![0; MAX_USER_ID_BYTES], "a")).validate(), Ok(()));
    }

    #[test]
    fn the_display_name_defaults_to_the_account_name() {
        let user = User::new(vec![1], "alice");
        assert_eq!(user.display_name, "alice");
        assert_eq!(user.display_name("Alice Smith").display_name, "Alice Smith");
    }

    #[test]
    fn the_relying_party_needs_a_name() {
        let request = CreateRequest {
            relying_party: RelyingParty::new("example.com", ""),
            ..create()
        };

        assert_eq!(invalid(request.validate()), InvalidRequest::RelyingPartyName);
    }

    #[test]
    fn an_empty_algorithm_list_is_rejected() {
        assert_eq!(
            invalid(create().algorithms([]).validate()),
            InvalidRequest::NoAlgorithms
        );
    }

    #[test]
    fn credential_lists_are_bounded() {
        let ids = |count: usize, len: usize| vec![CredentialId::new(vec![1; len]); count];

        assert_eq!(
            invalid(create().exclude(ids(MAX_CREDENTIALS + 1, 4)).validate()),
            InvalidRequest::TooManyCredentials
        );
        assert_eq!(
            invalid(GetRequest::new("example.com", vec![1]).allow(ids(1, 0)).validate()),
            InvalidRequest::CredentialId
        );
        assert_eq!(
            invalid(
                GetRequest::new("example.com", vec![1])
                    .allow(ids(1, MAX_CREDENTIAL_ID_BYTES + 1))
                    .validate()
            ),
            InvalidRequest::CredentialId
        );
        assert_eq!(
            GetRequest::new("example.com", vec![1])
                .allow(ids(MAX_CREDENTIALS, MAX_CREDENTIAL_ID_BYTES))
                .validate(),
            Ok(())
        );
    }

    #[test]
    fn timeouts_default_to_five_minutes_and_saturate() {
        assert_eq!(create().timeout_millis(), 300_000);
        assert_eq!(
            create().timeout(Duration::from_millis(1500)).timeout_millis(),
            1500
        );
        assert_eq!(
            GetRequest::new("example.com", vec![1])
                .timeout(Duration::from_secs(u64::MAX / 2))
                .timeout_millis(),
            u32::MAX
        );
    }

    #[test]
    fn cose_identifiers_match_the_registry() {
        assert_eq!(Algorithm::Es256.cose_id(), -7);
        assert_eq!(Algorithm::Rs256.cose_id(), -257);
    }
}
