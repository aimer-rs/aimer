//! The error vocabulary shared by every passkey adapter.
//!
//! No variant carries a credential, a challenge, or a relying party, so an error
//! can be logged as it is.

use std::fmt;

/// The result type returned by passkey operations.
pub type PasskeyResult<T> = Result<T, PasskeyError>;

/// What a request got wrong. Reported before anything reaches the platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidRequest {
    /// The relying party id is not a bare lowercase domain name: it is empty, or
    /// carries a scheme, port, path, uppercase letters, or non-ASCII characters
    /// (internationalized names go in as punycode).
    RelyingPartyId,
    /// The relying party's display name is empty.
    RelyingPartyName,
    /// The challenge is empty or longer than
    /// [`MAX_CHALLENGE_BYTES`](crate::MAX_CHALLENGE_BYTES).
    Challenge,
    /// The user handle is empty or longer than
    /// [`MAX_USER_ID_BYTES`](crate::MAX_USER_ID_BYTES).
    UserId,
    /// The account name is empty.
    UserName,
    /// A credential id is empty or longer than
    /// [`MAX_CREDENTIAL_ID_BYTES`](crate::MAX_CREDENTIAL_ID_BYTES).
    CredentialId,
    /// More than [`MAX_CREDENTIALS`](crate::MAX_CREDENTIALS) credential ids were
    /// listed.
    TooManyCredentials,
    /// The list of acceptable signature algorithms is empty.
    NoAlgorithms,
}

/// A platform capability that an adapter needs and the host may not supply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasskeyCapability {
    /// The operating system's passkey provider. Targets without one (Linux, the
    /// browser) and OS versions that predate passkeys report this.
    PlatformAuthenticator,
    /// The Venus runtime installed for the calling thread.
    ///
    /// Ceremonies resolve through Venus, so an operation made before
    /// `Venus::install` has nowhere to run.
    Runtime,
    /// The main thread. Apple's authorization UI can only be driven from it.
    MainThread,
    /// The Kotlin half of the Android adapter, `com.aimer.PasskeyBridge`, or the
    /// `androidx.credentials` library it needs. A build problem: the application
    /// must include both.
    Bridge,
    /// A window to present the system sheet from: no key window on Apple
    /// platforms, no foreground window on Windows, no running activity on
    /// Android.
    Window,
}

/// Errors returned by passkey adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasskeyError {
    /// The request is malformed. See [`InvalidRequest`].
    InvalidRequest(InvalidRequest),
    /// The requested platform capability is unavailable.
    Unavailable {
        /// The unavailable capability.
        capability: PasskeyCapability,
    },
    /// The user dismissed the system sheet or declined to verify.
    Cancelled,
    /// Authentication found no passkey: the user has none for this relying
    /// party, or none among the allowed credentials.
    NoCredential,
    /// Registration found a credential from the exclude list already on this
    /// device.
    AlreadyRegistered,
    /// The platform gave up waiting for the user.
    TimedOut,
    /// The relying party does not vouch for this application.
    ///
    /// A configuration problem, not a runtime one: the domain's
    /// `apple-app-site-association` (Apple), `assetlinks.json` (Android), or
    /// origin (Windows) does not list this app for passkeys.
    NotAssociated,
    /// The application is not entitled to use passkeys. On Apple platforms the
    /// build needs the Associated Domains entitlement with a `webcredentials:`
    /// entry, and a signed binary.
    MissingEntitlement,
    /// The platform cannot honor the request, for example a signature algorithm
    /// no authenticator here supports.
    Unsupported,
    /// The operation was dropped before it finished, or the platform interrupted
    /// it, for example because the app went to the background.
    Interrupted,
    /// The platform reported a failure that has no typed equivalent and no
    /// numeric code.
    Failed,
    /// The platform reported a failure that has no typed equivalent, with its
    /// native code (an `NSError` code on Apple platforms, an `HRESULT` on
    /// Windows).
    Backend {
        /// The platform's own error code.
        code: i32,
    },
}

impl fmt::Display for InvalidRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RelyingPartyId => "the relying party id is not a bare lowercase domain name",
            Self::RelyingPartyName => "the relying party name is empty",
            Self::Challenge => "the challenge is empty or too long",
            Self::UserId => "the user handle is empty or too long",
            Self::UserName => "the account name is empty",
            Self::CredentialId => "a credential id is empty or too long",
            Self::TooManyCredentials => "too many credential ids",
            Self::NoAlgorithms => "no signature algorithm was requested",
        })
    }
}

impl fmt::Display for PasskeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(reason) => write!(f, "invalid passkey request: {reason}"),
            Self::Unavailable { capability } => {
                write!(f, "passkey capability unavailable: {capability:?}")
            }
            Self::Cancelled => f.write_str("the passkey request was cancelled"),
            Self::NoCredential => f.write_str("no passkey is available for this request"),
            Self::AlreadyRegistered => f.write_str("a passkey for this account already exists here"),
            Self::TimedOut => f.write_str("the passkey request timed out"),
            Self::NotAssociated => {
                f.write_str("the relying party does not associate this application with its domain")
            }
            Self::MissingEntitlement => {
                f.write_str("the application is not entitled to use passkeys")
            }
            Self::Unsupported => f.write_str("the platform cannot honor this passkey request"),
            Self::Interrupted => f.write_str("the passkey request was interrupted"),
            Self::Failed => f.write_str("the passkey provider reported a failure"),
            Self::Backend { code } => write!(f, "passkey provider failure (code {code})"),
        }
    }
}

impl std::error::Error for InvalidRequest {}
impl std::error::Error for PasskeyError {}

impl From<InvalidRequest> for PasskeyError {
    fn from(reason: InvalidRequest) -> Self {
        Self::InvalidRequest(reason)
    }
}
