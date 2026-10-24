//! The error vocabulary shared by every vault adapter.
//!
//! No variant carries a secret, a key, or a service name, so an error can be
//! logged as it is.

use std::fmt;

/// The result type returned by vault operations.
pub type VaultResult<T> = Result<T, VaultError>;

/// A platform capability that an adapter needs and the host may not supply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultCapability {
    /// The operating system's secret store: Keychain on Apple platforms.
    ///
    /// Targets without a backend report this rather than quietly keeping
    /// secrets somewhere weaker.
    PlatformSecretStore,
    /// The Venus runtime installed for the calling thread.
    ///
    /// Keychain calls are driven by Venus, so an operation made before
    /// `Venus::install` has nowhere to run.
    Runtime,
}

/// Errors returned by vault adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VaultError {
    /// A namespace or key is empty, too long, or contains an unsafe byte.
    InvalidKey,
    /// A secret has no bytes. An empty secret is indistinguishable from a
    /// missing one on some backends, so none accepts it.
    EmptySecret,
    /// A secret exceeds [`MAX_SECRET_BYTES`](crate::MAX_SECRET_BYTES).
    SecretTooLarge {
        /// The rejected secret's byte length.
        len: usize,
        /// The maximum accepted byte length.
        limit: usize,
    },
    /// The requested platform capability is unavailable.
    Unavailable {
        /// The unavailable capability.
        capability: VaultCapability,
    },
    /// The secret store is locked: the device or the login keychain has not
    /// been unlocked, so the secret cannot be read or written right now.
    /// Retrying after the user unlocks it can succeed.
    Locked,
    /// The user or the system refused access to the secret.
    Denied,
    /// The application is not entitled to use the secret store.
    ///
    /// On Apple platforms this is a build problem, not a runtime one: a binary
    /// without a keychain-access-groups entitlement (or an unsigned one, for the
    /// data-protection keychain) cannot reach it.
    MissingEntitlement,
    /// The operation was dropped before it finished, for example because the
    /// worker running it panicked.
    Interrupted,
    /// The platform reported a failure that has no typed equivalent and no
    /// numeric code: a D-Bus error from the Secret Service, or a Java exception
    /// from the Android Keystore.
    Failed,
    /// The platform reported a failure that has no typed equivalent, with its
    /// native status code (an `OSStatus` on Apple platforms, a Win32 error on
    /// Windows).
    Backend {
        /// The platform's own error code.
        code: i32,
    },
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey => f.write_str("invalid vault namespace or key"),
            Self::EmptySecret => f.write_str("a vault secret cannot be empty"),
            Self::SecretTooLarge { len, limit } => {
                write!(f, "vault secret of {len} bytes exceeds the {limit} byte limit")
            }
            Self::Unavailable { capability } => {
                write!(f, "vault capability unavailable: {capability:?}")
            }
            Self::Locked => f.write_str("the secret store is locked"),
            Self::Denied => f.write_str("access to the secret store was denied"),
            Self::MissingEntitlement => {
                f.write_str("the application is not entitled to use the secret store")
            }
            Self::Interrupted => f.write_str("the vault operation was interrupted"),
            Self::Failed => f.write_str("the secret store reported a failure"),
            Self::Backend { code } => write!(f, "secret store failure (code {code})"),
        }
    }
}

impl std::error::Error for VaultError {}
