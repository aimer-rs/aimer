///
/// Represents the various platforms that an application or system can operate
/// on.
///
/// This enum includes variants for commonly used operating systems and
/// environments to help developers manage platform-specific behavior in their
/// code.
///
/// # Variants
///
/// * `Android` - Represents the Android operating system.
/// * `IOS` - Represents the iOS operating system.
/// * `Windows` - Represents the Windows operating system.
/// * `MacOS` - Represents the macOS operating system.
/// * `Linux` - Represents the Linux operating system.
/// * `Web` - Represents a web-based platform or environment.
///
/// # Derives
///
/// * `Debug` - Enables formatting the enum using the `{:?}` formatter.
/// * `Clone` - Allows creating a copy of the value.
/// * `Copy` - Allows bitwise copying of the value, as long as all fields also
///   implement `Copy`.
/// * `PartialEq` - Enables equality comparisons between enum instances (`==`
///   and `!=`).
/// * `Eq` - Indicates that all instances of the enum can be checked for
///   equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Android,
    Ios,
    Windows,
    MacOS,
    Linux,
    Web,
}
