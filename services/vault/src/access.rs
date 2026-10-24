//! When a stored secret may be read.

/// When the system allows a stored secret to be read.
///
/// Every level is *this device only*: the secret is not copied to other
/// devices by cloud sync and does not travel in a backup restored elsewhere. A
/// credential that must follow the user between devices should be issued again
/// by the service it belongs to, not carried by the vault.
///
/// Applies to the Apple data-protection keychain (iOS, and macOS with it
/// enabled). The macOS file-based login keychain has one lock state, tied to
/// the user's login, and ignores the distinction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum VaultAccess {
    /// Readable only while the device is unlocked. The safest level and the
    /// default; a background task that runs while the device is locked will see
    /// [`VaultError::Locked`](crate::VaultError::Locked).
    #[default]
    WhenUnlocked,
    /// Readable from the first unlock after a restart until the next restart,
    /// including while the device is locked. Use it for a secret a background
    /// refresh or push handler needs.
    AfterFirstUnlock,
}
