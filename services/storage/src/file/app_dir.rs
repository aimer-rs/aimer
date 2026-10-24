//! Where an application keeps its own data on each platform.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{StorageCapability, StorageError, StorageResult};
use crate::key::is_valid_component;
use crate::location::StorageLocation;

/// The family of directory conventions a host follows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HostOs {
    /// macOS and iOS: `~/Library/Application Support`. On iOS `HOME` is the
    /// app's sandbox container, so the same path lands inside the sandbox.
    Apple,
    /// Windows: the roaming `%APPDATA%`.
    Windows,
    /// Android: a directory the host hands over, see [`set_android_data_dir`].
    Android,
    /// Linux and other Unix: the XDG data directory.
    Unix,
}

impl HostOs {
    const CURRENT: Self = if cfg!(target_os = "android") {
        Self::Android
    } else if cfg!(any(target_os = "macos", target_os = "ios")) {
        Self::Apple
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Unix
    };
}

#[cfg(target_os = "android")]
static ANDROID_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Tells the storage layer where an Android app may write.
///
/// Android has no environment variable for this; the path comes from the
/// activity (`AndroidApp::internal_data_path`), which is the app's `files`
/// directory. The cache locations use its sibling `cache` directory, the layout
/// Android gives every app (`…/<package>/files` beside `…/<package>/cache`).
/// Call it once at startup, before the first [`app_dir`] or
/// `FileStorage::for_app`. Returns `false` if a directory was already set, in
/// which case the first one stays.
#[cfg(target_os = "android")]
pub fn set_android_data_dir(path: impl Into<PathBuf>) -> bool {
    ANDROID_DATA_DIR.set(path.into()).is_ok()
}

fn android_data_dir() -> Option<&'static Path> {
    #[cfg(target_os = "android")]
    {
        ANDROID_DATA_DIR.get().map(PathBuf::as_path)
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// The directory where the application named `app_id` keeps its own data.
///
/// This is [`app_dir`] for [`StorageLocation::Data`].
///
/// # Errors
///
/// See [`app_dir`].
///
/// # Examples
///
/// ```no_run
/// let dir = aimer_storage::app_data_dir("com.example.notes")?;
/// println!("{}", dir.display());
/// # Ok::<(), aimer_storage::StorageError>(())
/// ```
pub fn app_data_dir(app_id: &str) -> StorageResult<PathBuf> {
    app_dir(app_id, StorageLocation::Data)
}

/// The directory where the application named `app_id` keeps data of the given
/// [`StorageLocation`], following each platform's conventions.
///
/// | Platform | `Data` | `Roaming` | `Cache` | `Temp` |
/// |---|---|---|---|---|
/// | macOS, iOS | `~/Library/Application Support/<id>` | same as `Data` | `~/Library/Caches/<id>` | `$TMPDIR/<id>` |
/// | Windows | `%LOCALAPPDATA%\<id>` | `%APPDATA%\<id>` | `%LOCALAPPDATA%\<id>\Cache` | `%TEMP%\<id>` |
/// | Linux, other Unix | `$XDG_DATA_HOME/<id>` or `~/.local/share/<id>` | same as `Data` | `$XDG_CACHE_HOME/<id>` or `~/.cache/<id>` | `$TMPDIR/<id>` or `/tmp/<id>` |
/// | Android | `<files>/<id>` | same as `Data` | `<cache>/<id>` | `<cache>/<id>/tmp` |
///
/// The directory is only *named*: it is created by the first write into it.
/// `app_id` should be stable and unique, such as `com.example.notes`, and must
/// satisfy the same rules as a storage key.
///
/// # Errors
///
/// * [`StorageError::InvalidKey`] when `app_id` is empty, too long, or contains
///   a path separator or control byte.
/// * [`StorageError::Unavailable`] when the platform's base directory cannot be
///   determined: an unset or relative environment variable, or an Android host
///   that never called `set_android_data_dir`.
pub fn app_dir(app_id: &str, location: StorageLocation) -> StorageResult<PathBuf> {
    resolve(
        app_id,
        location,
        HostOs::CURRENT,
        &|name| std::env::var_os(name),
        android_data_dir(),
    )
}

/// [`app_dir`] with the host and its environment supplied, so every platform's
/// rule can be tested from any machine.
pub(super) fn resolve(
    app_id: &str,
    location: StorageLocation,
    os: HostOs,
    env: &dyn Fn(&str) -> Option<OsString>,
    android_dir: Option<&Path>,
) -> StorageResult<PathBuf> {
    use StorageLocation::{Cache, Data, Roaming, Temp};

    if !is_valid_component(app_id) {
        return Err(StorageError::InvalidKey);
    }
    // A relative base would resolve against whatever the working directory
    // happens to be, which is how data ends up scattered. XDG says to ignore
    // it, and so does everything else here.
    let absolute = |name: &str| {
        env(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let home = |parts: &[&str]| {
        absolute("HOME").map(|home| parts.iter().fold(home, |path, part| path.join(part)))
    };
    let base = match (os, location) {
        (HostOs::Apple, Data | Roaming) => home(&["Library", "Application Support"]),
        (HostOs::Apple, Cache) => home(&["Library", "Caches"]),
        (HostOs::Apple, Temp) => absolute("TMPDIR"),

        (HostOs::Windows, Data) => absolute("LOCALAPPDATA"),
        (HostOs::Windows, Roaming) => absolute("APPDATA"),
        (HostOs::Windows, Cache) => absolute("LOCALAPPDATA"),
        (HostOs::Windows, Temp) => absolute("TEMP").or_else(|| absolute("TMP")),

        (HostOs::Android, Data | Roaming) => android_dir.map(Path::to_path_buf),
        (HostOs::Android, Cache | Temp) => android_dir
            .and_then(Path::parent)
            .map(|package| package.join("cache")),

        (HostOs::Unix, Data | Roaming) => {
            absolute("XDG_DATA_HOME").or_else(|| home(&[".local", "share"]))
        }
        (HostOs::Unix, Cache) => absolute("XDG_CACHE_HOME").or_else(|| home(&[".cache"])),
        (HostOs::Unix, Temp) => absolute("TMPDIR").or_else(|| Some(PathBuf::from("/tmp"))),
    };
    let unavailable = || StorageError::Unavailable {
        capability: StorageCapability::NativeFile,
    };
    let base = base.ok_or_else(unavailable)?;

    // Windows keeps the cache beside the data in a folder of its own, and Android
    // keeps scratch files apart from the cache proper.
    Ok(match (os, location) {
        (HostOs::Windows, Cache) => base.join(app_id).join("Cache"),
        (HostOs::Android, Temp) => base.join(app_id).join("tmp"),
        _ => base.join(app_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use StorageLocation::{Cache, Data, Roaming, Temp};

    /// A path that is absolute on whichever host runs the tests.
    fn base(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn env_of(pairs: Vec<(&'static str, PathBuf)>) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone().into_os_string())
        }
    }

    const ID: &str = "com.example.notes";

    fn unavailable() -> StorageResult<PathBuf> {
        Err(StorageError::Unavailable {
            capability: StorageCapability::NativeFile,
        })
    }

    #[test]
    fn apple_hosts_follow_the_library_conventions() {
        let env = env_of(vec![("HOME", base("home")), ("TMPDIR", base("tmp"))]);
        let library = base("home").join("Library");
        let at = |location| resolve(ID, location, HostOs::Apple, &env, None);

        assert_eq!(at(Data), Ok(library.join("Application Support").join(ID)));
        assert_eq!(at(Roaming), at(Data), "there is no roaming store to use");
        assert_eq!(at(Cache), Ok(library.join("Caches").join(ID)));
        assert_eq!(at(Temp), Ok(base("tmp").join(ID)));
    }

    #[test]
    fn windows_separates_local_roaming_cache_and_temp() {
        let env = env_of(vec![
            ("LOCALAPPDATA", base("local")),
            ("APPDATA", base("roaming")),
            ("TEMP", base("temp")),
            ("HOME", base("home")),
        ]);
        let at = |location| resolve(ID, location, HostOs::Windows, &env, None);

        assert_eq!(at(Data), Ok(base("local").join(ID)));
        assert_eq!(at(Roaming), Ok(base("roaming").join(ID)));
        assert_eq!(at(Cache), Ok(base("local").join(ID).join("Cache")));
        assert_eq!(at(Temp), Ok(base("temp").join(ID)));
    }

    #[test]
    fn windows_temp_falls_back_to_tmp() {
        let env = env_of(vec![("TMP", base("tmp"))]);
        assert_eq!(
            resolve(ID, Temp, HostOs::Windows, &env, None),
            Ok(base("tmp").join(ID))
        );
    }

    #[test]
    fn unix_prefers_the_xdg_directories_then_falls_back_to_home() {
        let xdg = env_of(vec![
            ("XDG_DATA_HOME", base("xdg-data")),
            ("XDG_CACHE_HOME", base("xdg-cache")),
            ("HOME", base("home")),
            ("TMPDIR", base("tmp")),
        ]);
        let at = |location| resolve(ID, location, HostOs::Unix, &xdg, None);
        assert_eq!(at(Data), Ok(base("xdg-data").join(ID)));
        assert_eq!(at(Roaming), at(Data));
        assert_eq!(at(Cache), Ok(base("xdg-cache").join(ID)));
        assert_eq!(at(Temp), Ok(base("tmp").join(ID)));

        let home_only = env_of(vec![("HOME", base("home"))]);
        let at = |location| resolve(ID, location, HostOs::Unix, &home_only, None);
        assert_eq!(
            at(Data),
            Ok(base("home").join(".local").join("share").join(ID))
        );
        assert_eq!(at(Cache), Ok(base("home").join(".cache").join(ID)));
        assert_eq!(at(Temp), Ok(PathBuf::from("/tmp").join(ID)));
    }

    #[test]
    fn android_uses_the_files_directory_and_its_sibling_cache() {
        let env = env_of(vec![]);
        let files = base("data").join("com.example.notes").join("files");
        let package = base("data").join("com.example.notes");
        let at = |location| resolve(ID, location, HostOs::Android, &env, Some(&files));

        assert_eq!(at(Data), Ok(files.join(ID)));
        assert_eq!(at(Roaming), at(Data));
        assert_eq!(at(Cache), Ok(package.join("cache").join(ID)));
        assert_eq!(at(Temp), Ok(package.join("cache").join(ID).join("tmp")));

        for location in [Data, Roaming, Cache, Temp] {
            assert_eq!(
                resolve(ID, location, HostOs::Android, &env, None),
                unavailable()
            );
        }
    }

    #[test]
    fn unset_empty_and_relative_bases_are_unavailable() {
        for location in [Data, Cache] {
            assert_eq!(
                resolve(ID, location, HostOs::Unix, &env_of(vec![]), None),
                unavailable()
            );
        }

        let relative = |name: &str| (name == "HOME").then(|| OsString::from("relative/home"));
        assert_eq!(
            resolve(ID, Data, HostOs::Unix, &relative, None),
            unavailable()
        );

        let empty = |name: &str| (name == "HOME").then(OsString::new);
        assert_eq!(
            resolve(ID, Cache, HostOs::Apple, &empty, None),
            unavailable()
        );
    }

    #[test]
    fn an_app_id_that_could_escape_the_directory_is_rejected() {
        let env = env_of(vec![("HOME", base("home"))]);
        for location in [Data, Roaming, Cache, Temp] {
            for bad in ["", ".", "..", "a/b", "..\\x", "a\0b"] {
                assert_eq!(
                    resolve(bad, location, HostOs::Unix, &env, None),
                    Err(StorageError::InvalidKey),
                    "{bad:?}"
                );
            }
        }
    }
}
