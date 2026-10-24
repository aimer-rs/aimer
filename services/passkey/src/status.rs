//! Maps each platform's native failure codes to [`PasskeyError`].
//!
//! The mappings are pure functions, compiled on every target so they can be
//! tested without the platform that produces the codes.

use crate::error::{PasskeyCapability, PasskeyError};

/// `HRESULT`s that `webauthn.dll` returns, as signed values.
mod hresult {
    pub(super) const NTE_EXISTS: i32 = 0x8009_000F_u32 as i32;
    pub(super) const NTE_NOT_FOUND: i32 = 0x8009_0011_u32 as i32;
    pub(super) const NTE_NOT_SUPPORTED: i32 = 0x8009_0029_u32 as i32;
    pub(super) const NTE_USER_CANCELLED: i32 = 0x8009_0036_u32 as i32;
    /// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`.
    pub(super) const CANCELLED: i32 = 0x8007_04C7_u32 as i32;
    /// `HRESULT_FROM_WIN32(ERROR_TIMEOUT)`.
    pub(super) const TIMEOUT: i32 = 0x8007_05B4_u32 as i32;
}

/// The typed error for a failed WebAuthn call on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn from_hresult(code: i32) -> PasskeyError {
    match code {
        hresult::NTE_USER_CANCELLED | hresult::CANCELLED => PasskeyError::Cancelled,
        hresult::NTE_EXISTS => PasskeyError::AlreadyRegistered,
        hresult::NTE_NOT_FOUND => PasskeyError::NoCredential,
        hresult::TIMEOUT => PasskeyError::TimedOut,
        hresult::NTE_NOT_SUPPORTED => PasskeyError::Unsupported,
        code => PasskeyError::Backend { code },
    }
}

/// The typed error for a failed Credential Manager request.
///
/// `kind` is what the Kotlin bridge reports: the simple class name of the
/// `androidx.credentials` exception, or `Dom.` followed by the simple class name
/// of the DOM error a `*PublicKeyCredentialDomException` wraps.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub(crate) fn from_android(kind: &str) -> PasskeyError {
    match kind {
        "CreateCredentialCancellationException" | "GetCredentialCancellationException"
        // The DOM spec's catch-all for a refused or abandoned ceremony.
        | "Dom.NotAllowedError" => PasskeyError::Cancelled,
        "NoCredentialException" => PasskeyError::NoCredential,
        // The bridge has no foreground activity to present over.
        "NoActivity" => PasskeyError::Unavailable {
            capability: PasskeyCapability::Window,
        },
        "Dom.InvalidStateError" => PasskeyError::AlreadyRegistered,
        "Dom.TimeoutError" => PasskeyError::TimedOut,
        // The relying party id is not valid for this app.
        "Dom.SecurityError" | "Dom.URLMismatchError" => PasskeyError::NotAssociated,
        "CreateCredentialUnsupportedException"
        | "GetCredentialUnsupportedException"
        | "Dom.NotSupportedError"
        | "Dom.ConstraintError" => PasskeyError::Unsupported,
        "CreateCredentialInterruptedException" | "GetCredentialInterruptedException" => {
            PasskeyError::Interrupted
        }
        // No provider can serve the request: none installed, enabled, or able.
        "CreateCredentialNoCreateOptionException"
        | "CreateCredentialProviderConfigurationException"
        | "GetCredentialProviderConfigurationException" => PasskeyError::Unavailable {
            capability: PasskeyCapability::PlatformAuthenticator,
        },
        _ => PasskeyError::Failed,
    }
}

/// The `NSError` domain AuthenticationServices reports ceremony failures in.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub(crate) const APPLE_DOMAIN: &str = "com.apple.AuthenticationServices.AuthorizationError";

/// The typed error for a failed AuthenticationServices request.
///
/// `ASAuthorizationErrorFailed` (1004) is what the system reports for a
/// misconfigured app as well as for an ordinary failure, and only its message
/// tells them apart. "does not have an application identifier" (an unsigned
/// binary) was observed on macOS; "not associated with domain" (a signed app the
/// domain does not list) is the system's documented wording and has not been
/// seen here. Matching on messages is a best effort: an unrecognized one stays
/// `Failed`.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub(crate) fn from_apple(domain: &str, code: i64, message: &str) -> PasskeyError {
    if domain != APPLE_DOMAIN {
        return PasskeyError::Backend {
            code: i32::try_from(code).unwrap_or(i32::MAX),
        };
    }
    match code {
        1001 => PasskeyError::Cancelled,
        1004 => {
            let message = message.to_ascii_lowercase();
            if message.contains("not associated with domain") {
                PasskeyError::NotAssociated
            } else if message.contains("entitlement") || message.contains("application identifier") {
                PasskeyError::MissingEntitlement
            } else {
                PasskeyError::Failed
            }
        }
        1005 => PasskeyError::Interrupted,
        1006 => PasskeyError::AlreadyRegistered,
        // The device has no passcode or iCloud Keychain to keep a passkey in.
        1010 => PasskeyError::Unavailable {
            capability: PasskeyCapability::PlatformAuthenticator,
        },
        1002 | 1003 => PasskeyError::Failed,
        code => PasskeyError::Backend {
            code: i32::try_from(code).unwrap_or(i32::MAX),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_codes_map_to_typed_errors() {
        for (code, expected) in [
            (0x8009_0036_u32, PasskeyError::Cancelled),
            (0x8007_04C7, PasskeyError::Cancelled),
            (0x8009_000F, PasskeyError::AlreadyRegistered),
            (0x8009_0011, PasskeyError::NoCredential),
            (0x8007_05B4, PasskeyError::TimedOut),
            (0x8009_0029, PasskeyError::Unsupported),
        ] {
            assert_eq!(from_hresult(code as i32), expected, "{code:#x}");
        }
    }

    #[test]
    fn an_unknown_hresult_keeps_its_code() {
        assert_eq!(
            from_hresult(0x8007_0005_u32 as i32),
            PasskeyError::Backend { code: 0x8007_0005_u32 as i32 }
        );
    }

    #[test]
    fn apple_codes_map_to_typed_errors() {
        for (code, message, expected) in [
            (1001, "", PasskeyError::Cancelled),
            (1004, "Application with identifier X is not associated with domain Y", PasskeyError::NotAssociated),
            (1004, "missing the com.apple.developer.associated-domains Entitlement", PasskeyError::MissingEntitlement),
            // Verbatim from an unsigned binary on macOS.
            (
                1004,
                "The operation couldn’t be completed. The calling process does not have an application identifier. Make sure it is properly configured.",
                PasskeyError::MissingEntitlement,
            ),
            (1004, "something else", PasskeyError::Failed),
            (1005, "", PasskeyError::Interrupted),
            (1006, "", PasskeyError::AlreadyRegistered),
            (
                1010,
                "",
                PasskeyError::Unavailable { capability: PasskeyCapability::PlatformAuthenticator },
            ),
            (1002, "", PasskeyError::Failed),
            (1003, "", PasskeyError::Failed),
            (1000, "", PasskeyError::Backend { code: 1000 }),
        ] {
            assert_eq!(from_apple(APPLE_DOMAIN, code, message), expected, "{code} {message}");
        }
    }

    #[test]
    fn a_foreign_apple_domain_is_never_mistaken_for_a_ceremony_error() {
        assert_eq!(
            from_apple("com.apple.LocalAuthentication", 1001, ""),
            PasskeyError::Backend { code: 1001 }
        );
    }

    #[test]
    fn android_kinds_map_to_typed_errors() {
        for (kind, expected) in [
            ("CreateCredentialCancellationException", PasskeyError::Cancelled),
            ("GetCredentialCancellationException", PasskeyError::Cancelled),
            ("Dom.NotAllowedError", PasskeyError::Cancelled),
            ("NoCredentialException", PasskeyError::NoCredential),
            (
                "NoActivity",
                PasskeyError::Unavailable { capability: PasskeyCapability::Window },
            ),
            ("Dom.InvalidStateError", PasskeyError::AlreadyRegistered),
            ("Dom.TimeoutError", PasskeyError::TimedOut),
            ("Dom.SecurityError", PasskeyError::NotAssociated),
            ("GetCredentialUnsupportedException", PasskeyError::Unsupported),
            ("Dom.NotSupportedError", PasskeyError::Unsupported),
            ("GetCredentialInterruptedException", PasskeyError::Interrupted),
            (
                "CreateCredentialNoCreateOptionException",
                PasskeyError::Unavailable { capability: PasskeyCapability::PlatformAuthenticator },
            ),
            ("CreateCredentialUnknownException", PasskeyError::Failed),
            ("Dom.UnknownError", PasskeyError::Failed),
            ("", PasskeyError::Failed),
        ] {
            assert_eq!(from_android(kind), expected, "{kind}");
        }
    }
}
