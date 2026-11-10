//! Starting the daemon when the user logs in.
//!
//! | Platform | Mechanism                                                     |
//! | -------- | ------------------------------------------------------------- |
//! | macOS    | a launchd agent, `~/Library/LaunchAgents/<id>.plist`          |
//! | Linux    | an XDG autostart entry, `$XDG_CONFIG_HOME/autostart/<id>.desktop` |
//! | Windows  | a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` |
//! | others   | not supported: the OS owns when a mobile app launches         |
//!
//! Everything is per-user and needs no elevated rights. The text each mechanism
//! consumes is produced by small pure functions so it can be tested without
//! touching the real login items.

use std::io;
use std::path::PathBuf;

/// Registers a program to start at login.
///
/// The `id` names the entry on disk and in the registry, so it is restricted to
/// ASCII letters, digits, `.`, `_` and `-`, and may not start with a dot; reverse
/// DNS names such as `com.example.daemon` fit.
///
/// # Examples
///
/// ```no_run
/// use aimer_daemon::Autostart;
///
/// // Starts the current executable with `--background` at every login.
/// Autostart::new("com.example.daemon").arg("--background").enable()?;
/// assert!(Autostart::new("com.example.daemon").is_enabled()?);
/// Autostart::new("com.example.daemon").disable()?;
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Autostart {
    id: String,
    program: Option<PathBuf>,
    args: Vec<String>,
}

impl Autostart {
    /// Describes an entry named `id`, launching the current executable.
    #[inline]
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), program: None, args: Vec::new() }
    }

    /// The program to launch instead of the current executable.
    #[inline]
    pub fn program(mut self, program: impl Into<PathBuf>) -> Self {
        self.program = Some(program.into());
        self
    }

    /// Appends one command-line argument.
    #[inline]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends several command-line arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Makes the program start at every login, replacing an earlier entry with
    /// the same id.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] for a bad id or a program path that is
    /// not valid Unicode, [`io::ErrorKind::Unsupported`] on platforms without
    /// login items, and the underlying error if the entry cannot be written.
    pub fn enable(&self) -> io::Result<()> {
        validate_id(&self.id)?;
        let argv = self.argv()?;
        imp::enable(&self.id, &argv)
    }

    /// Removes the entry. Succeeds if there was none.
    ///
    /// # Errors
    ///
    /// As for [`Autostart::enable`], without the program checks.
    pub fn disable(&self) -> io::Result<()> {
        validate_id(&self.id)?;
        imp::disable(&self.id)
    }

    /// Whether an entry with this id exists.
    ///
    /// # Errors
    ///
    /// As for [`Autostart::disable`].
    pub fn is_enabled(&self) -> io::Result<bool> {
        validate_id(&self.id)?;
        imp::is_enabled(&self.id)
    }

    /// The program followed by its arguments.
    fn argv(&self) -> io::Result<Vec<String>> {
        let program = match &self.program {
            Some(program) => program.clone(),
            None => std::env::current_exe()?,
        };
        let program = program.into_os_string().into_string().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "the program path is not valid Unicode")
        })?;
        let mut argv = Vec::with_capacity(1 + self.args.len());
        argv.push(program);
        argv.extend(self.args.iter().cloned());
        Ok(argv)
    }
}

/// Keeps `id` from escaping the directory it names a file in, or breaking the
/// text format it is written into.
fn validate_id(id: &str) -> io::Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an autostart id is 1 to 128 ASCII letters, digits, '.', '_' or '-', and does not start with '.'",
        ))
    }
}

/// The launchd property list for `argv`.
#[cfg(any(target_os = "macos", test))]
fn launch_agent_plist(id: &str, argv: &[String]) -> String {
    let mut plist = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n",
    );
    plist.push_str(&format!("\t<string>{}</string>\n", xml_escape(id)));
    plist.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
    for arg in argv {
        plist.push_str(&format!("\t\t<string>{}</string>\n", xml_escape(arg)));
    }
    plist.push_str("\t</array>\n\t<key>RunAtLoad</key>\n\t<true/>\n</dict>\n</plist>\n");
    plist
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The XDG desktop entry for `argv`.
#[cfg(any(target_os = "linux", test))]
fn desktop_entry(id: &str, argv: &[String]) -> String {
    let exec = argv.iter().map(|arg| desktop_quote(arg)).collect::<Vec<_>>().join(" ");
    format!(
        "[Desktop Entry]\nType=Application\nName={id}\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// Quotes one `Exec` argument. The desktop entry spec has two layers: the quoted
/// argument, in which `"`, `` ` ``, `$` and `\` take a backslash, and the string
/// value of the file, in which a backslash is itself written twice.
#[cfg(any(target_os = "linux", test))]
fn desktop_quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        match c {
            '\\' => out.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                out.push('\\');
                out.push(c);
            }
            // `%` starts a field code; `%%` is a literal percent sign.
            '%' => out.push_str("%%"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Windows command line for `argv`, quoted the way `CommandLineToArgvW` and
/// the C runtime read it back.
#[cfg(any(target_os = "windows", test))]
fn windows_command_line(argv: &[String]) -> String {
    argv.iter().map(|arg| windows_quote(arg)).collect::<Vec<_>>().join(" ")
}

#[cfg(any(target_os = "windows", test))]
fn windows_quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, then the quote escaped.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    // Trailing backslashes would escape the closing quote, so double them.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Writes `contents` to `path` through a sibling file, so a crash never leaves a
/// half-written entry for the login machinery to trip over.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_atomically(path: &std::path::Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "the entry path has no parent directory")
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remove_entry(path: &std::path::Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{launch_agent_plist, remove_entry, write_atomically};

    pub(super) fn entry_path(home: &Path, id: &str) -> PathBuf {
        home.join("Library/LaunchAgents").join(format!("{id}.plist"))
    }

    fn path(id: &str) -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        Ok(entry_path(&home, id))
    }

    pub(super) fn enable(id: &str, argv: &[String]) -> io::Result<()> {
        write_atomically(&path(id)?, &launch_agent_plist(id, argv))
    }

    pub(super) fn disable(id: &str) -> io::Result<()> {
        remove_entry(&path(id)?)
    }

    pub(super) fn is_enabled(id: &str) -> io::Result<bool> {
        path(id)?.try_exists()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{desktop_entry, remove_entry, write_atomically};

    /// `xdg_config_home` counts only if it is an absolute path, as the XDG base
    /// directory spec requires.
    pub(super) fn entry_path(xdg_config_home: Option<&Path>, home: Option<&Path>, id: &str) -> Option<PathBuf> {
        let config = match xdg_config_home.filter(|path| path.is_absolute()) {
            Some(config) => config.to_path_buf(),
            None => home?.join(".config"),
        };
        Some(config.join("autostart").join(format!("{id}.desktop")))
    }

    fn path(id: &str) -> io::Result<PathBuf> {
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        entry_path(xdg.as_deref(), home.as_deref(), id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "neither XDG_CONFIG_HOME nor HOME is set")
        })
    }

    pub(super) fn enable(id: &str, argv: &[String]) -> io::Result<()> {
        write_atomically(&path(id)?, &desktop_entry(id, argv))
    }

    pub(super) fn disable(id: &str) -> io::Result<()> {
        remove_entry(&path(id)?)
    }

    pub(super) fn is_enabled(id: &str) -> io::Result<bool> {
        path(id)?.try_exists()
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey,
        RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    use super::windows_command_line;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(code: u32) -> io::Result<()> {
        if code == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(code as i32)) }
    }

    /// The `Run` key, closed on drop.
    struct RunKey(HKEY);

    impl RunKey {
        fn open() -> io::Result<Self> {
            let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            let mut key: HKEY = std::ptr::null_mut();
            // SAFETY: `subkey` is NUL-terminated, and `key` is a valid out-pointer.
            let code = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    0,
                    KEY_SET_VALUE | KEY_QUERY_VALUE,
                    &mut key,
                )
            };
            check(code)?;
            Ok(Self(key))
        }
    }

    impl Drop for RunKey {
        fn drop(&mut self) {
            // SAFETY: `self.0` was opened by `RegOpenKeyExW` and is closed once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    pub(super) fn enable(id: &str, argv: &[String]) -> io::Result<()> {
        let key = RunKey::open()?;
        let name = wide(id);
        let command = wide(&windows_command_line(argv));
        let bytes = u32::try_from(command.len() * 2)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the command line is too long"))?;
        // SAFETY: `name` is NUL-terminated, and `command` holds `bytes` bytes of
        // UTF-16 including its terminator, as `REG_SZ` requires.
        let code = unsafe {
            RegSetValueExW(key.0, name.as_ptr(), 0, REG_SZ, command.as_ptr().cast(), bytes)
        };
        check(code)
    }

    pub(super) fn disable(id: &str) -> io::Result<()> {
        let key = RunKey::open()?;
        let name = wide(id);
        // SAFETY: `name` is NUL-terminated.
        let code = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(code) }
    }

    pub(super) fn is_enabled(id: &str) -> io::Result<bool> {
        let key = RunKey::open()?;
        let name = wide(id);
        // SAFETY: `name` is NUL-terminated; null type and data pointers only ask
        // whether the value exists.
        let code = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        match code {
            ERROR_SUCCESS => Ok(true),
            ERROR_FILE_NOT_FOUND => Ok(false),
            code => Err(io::Error::from_raw_os_error(code as i32)),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod imp {
    use std::io;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this platform has no login items; the OS decides when the app launches",
        ))
    }

    pub(super) fn enable(_id: &str, _argv: &[String]) -> io::Result<()> {
        unsupported()
    }

    pub(super) fn disable(_id: &str) -> io::Result<()> {
        unsupported()
    }

    pub(super) fn is_enabled(_id: &str) -> io::Result<bool> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn ids_must_be_safe_file_names() {
        for good in ["a", "com.example.daemon", "my-app_2"] {
            assert!(validate_id(good).is_ok(), "{good}");
        }
        for bad in ["", ".hidden", "a/b", "..", "a b", "a\\b", "é", &"x".repeat(129)] {
            assert_eq!(
                validate_id(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_bad_id_is_rejected_before_anything_is_written() {
        let entry = Autostart::new("../escape").program("/bin/true");
        assert_eq!(entry.enable().unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(entry.disable().unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(entry.is_enabled().unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn argv_puts_the_program_first() {
        let entry = Autostart::new("x").program("/opt/app").arg("--a").args(["--b", "c"]);
        assert_eq!(entry.argv().unwrap(), argv(&["/opt/app", "--a", "--b", "c"]));
    }

    #[test]
    fn argv_defaults_to_the_current_executable() {
        let argv = Autostart::new("x").argv().unwrap();
        assert_eq!(argv.len(), 1);
        assert_eq!(PathBuf::from(&argv[0]), std::env::current_exe().unwrap());
    }

    #[test]
    fn plist_lists_every_argument_and_runs_at_load() {
        let plist = launch_agent_plist("com.example.d", &argv(&["/opt/a b/app", "--flag"]));
        assert!(plist.contains("<key>Label</key>\n\t<string>com.example.d</string>"));
        assert!(plist.contains("<string>/opt/a b/app</string>\n\t\t<string>--flag</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn plist_escapes_markup_in_arguments() {
        let plist = launch_agent_plist("x", &argv(&["/a&b", "<\"'>"]));
        assert!(plist.contains("<string>/a&amp;b</string>"));
        assert!(plist.contains("<string>&lt;&quot;&apos;&gt;</string>"));
    }

    #[test]
    fn desktop_entry_quotes_the_command() {
        let entry = desktop_entry("com.example.d", &argv(&["/opt/my app/run", "--name=a b"]));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(entry.contains("Name=com.example.d\n"));
        assert!(entry.contains("Exec=\"/opt/my app/run\" \"--name=a b\"\n"));
    }

    #[test]
    fn desktop_quote_escapes_reserved_characters() {
        assert_eq!(desktop_quote("a$b`c\"d"), "\"a\\$b\\`c\\\"d\"");
        assert_eq!(desktop_quote("a\\b"), "\"a\\\\\\\\b\"");
        assert_eq!(desktop_quote("100%"), "\"100%%\"");
        assert_eq!(desktop_quote("a\nb"), "\"a b\"");
    }

    #[test]
    fn windows_quote_follows_the_argv_rules() {
        assert_eq!(windows_quote("plain"), "plain");
        assert_eq!(windows_quote(""), "\"\"");
        assert_eq!(windows_quote("a b"), "\"a b\"");
        assert_eq!(windows_quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(windows_quote("C:\\dir with space\\"), "\"C:\\dir with space\\\\\"");
        assert_eq!(windows_quote("a\\\"b"), "\"a\\\\\\\"b\"");
        assert_eq!(windows_quote("C:\\plain\\path"), "C:\\plain\\path");
        assert_eq!(
            windows_command_line(&argv(&["C:\\Program Files\\app.exe", "--x"])),
            "\"C:\\Program Files\\app.exe\" --x"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_entry_lives_in_launch_agents() {
        let path = imp::entry_path(std::path::Path::new("/Users/me"), "com.example.d");
        assert_eq!(path, PathBuf::from("/Users/me/Library/LaunchAgents/com.example.d.plist"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_entry_honours_xdg_config_home_only_when_absolute() {
        use std::path::Path;
        let id = "com.example.d";
        let home = Some(Path::new("/home/me"));
        assert_eq!(
            imp::entry_path(Some(Path::new("/cfg")), home, id),
            Some(PathBuf::from("/cfg/autostart/com.example.d.desktop"))
        );
        assert_eq!(
            imp::entry_path(Some(Path::new("relative")), home, id),
            Some(PathBuf::from("/home/me/.config/autostart/com.example.d.desktop"))
        );
        assert_eq!(imp::entry_path(None, None, id), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn write_then_remove_round_trips_on_disk() {
        let dir = std::env::temp_dir().join(format!("aimer_daemon_autostart_{}", std::process::id()));
        let path = dir.join("nested").join("entry.txt");
        write_atomically(&path, "first").unwrap();
        write_atomically(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(!path.with_extension("tmp").exists());
        remove_entry(&path).unwrap();
        remove_entry(&path).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
