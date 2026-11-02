use std::panic::Location;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

#[cfg(not(target_arch = "wasm32"))]
use colored::Colorize;

/// Command line flag that switches the logger from human readable, colored
/// output to newline-delimited JSON.
///
/// `aimer run` passes this flag to the application it launches so that
/// `aimer_cli` can parse every log event instead of guessing its severity from
/// the text, and render it inside the console with its own styling.
///
/// # Examples
///
/// ```no_run
/// // The application is launched by the CLI as:
/// // my_app --json-output
/// assert_eq!(aimer_utils::log::JSON_OUTPUT_FLAG, "--json-output");
/// ```
pub const JSON_OUTPUT_FLAG: &str = "--json-output";

/// Environment variable that enables JSON output on platforms where the process
/// cannot be given command line arguments (Android, iOS).
///
/// Any value other than `0` enables it.
pub const JSON_OUTPUT_ENV: &str = "AIMER_JSON_OUTPUT";

#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
mod console {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console)]
        pub fn log(s: &str);
        #[wasm_bindgen(js_namespace = console)]
        pub fn warn(s: &str);
        #[wasm_bindgen(js_namespace = console)]
        pub fn error(s: &str);
        #[wasm_bindgen]
        pub fn eval(s: &str);
    }
}

/// Whether this process must emit newline-delimited JSON instead of colored
/// text.
///
/// The answer is computed once, on the first log call, and cached: the command
/// line and the environment cannot change afterwards, so per-call logging stays
/// as cheap as a single atomic load.
///
/// JSON output is requested either by passing [`JSON_OUTPUT_FLAG`] on the
/// command line or by setting [`JSON_OUTPUT_ENV`] to anything but `0`.
#[cfg(not(target_arch = "wasm32"))]
pub fn json_output_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::args().any(|arg| arg == JSON_OUTPUT_FLAG)
            || std::env::var_os(JSON_OUTPUT_ENV).is_some_and(|value| value != "0")
    })
}

/// Escape `s` so it can be embedded in a JSON string literal, as described by
/// [RFC 8259 §7](https://www.rfc-editor.org/rfc/rfc8259#section-7).
///
/// Quotes, backslashes and the control characters below `U+0020` are escaped;
/// everything else — including non-ASCII text — is copied verbatim, since JSON
/// strings are UTF-8.
#[cfg(not(target_arch = "wasm32"))]
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Severity of a log event.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Info,
    Warn,
    Error,
    Debug,
}

#[cfg(not(target_arch = "wasm32"))]
impl Level {
    /// Lowercase name written to the JSON `level` field.
    #[inline]
    const fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
            Level::Debug => "debug",
        }
    }

    /// Fixed-width label used by the human readable output, so the messages of
    /// consecutive lines stay aligned.
    #[inline]
    const fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
            Level::Debug => "DEBUG",
        }
    }
}

/// Build one compact JSON log record, without a trailing newline.
///
/// The `__aimer` marker lets a consumer tell a log record apart from arbitrary
/// JSON an application may print itself.
#[cfg(not(target_arch = "wasm32"))]
fn json_record(level: Level, message: &str, location: &Location) -> String {
    format!(
        r#"{{"__aimer":1,"level":"{}","message":"{}","file":"{}","line":{}}}"#,
        level.as_str(),
        escape_json(message),
        escape_json(location.file()),
        location.line()
    )
}

/// Print one log event: a JSON record when [`json_output_enabled`], colored
/// text otherwise.
#[cfg(not(target_arch = "wasm32"))]
fn emit(level: Level, message: &str, location: &Location) {
    if json_output_enabled() {
        println!("{}", json_record(level, message, location));
        return;
    }
    let label = level.label();
    match level {
        Level::Info => println!("[{}] {}", label.bold().bright_cyan(), message.bright_cyan()),
        Level::Warn => println!("[{}] {}", label.bold().yellow(), message.yellow()),
        Level::Error => println!("[{}] {}", label.bold().red(), message.red()),
        Level::Debug => println!("[{}] {}", label.bold().green(), message.bright_green()),
    }
}

#[allow(dead_code)]
fn extract_location(locat: &Location, log: &str, namespace: &str) -> String {
    let file_line = format!("{}:{}", locat.file(), locat.line());
    format!(
        r#"
//# sourceURL={file_line}
console.{namespace}(`{log}`);
"#,
    )
}

#[track_caller]
pub fn log(msg: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        emit(Level::Info, msg, Location::caller());
    }
    #[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
    {
        #[cfg(debug_assertions)]
        {
            let fmt = format!("[INFO]  {}", msg);
            let location = extract_location(Location::caller(), &fmt, "log");
            console::eval(&location);
        }
        #[cfg(not(debug_assertions))]
        {
            let fmt = format!("[INFO]  {}", msg);
            console::log(&fmt);
        }
    }
    #[cfg(all(target_arch = "wasm32", aimer_portable_guest))]
    let _ = msg;
}

#[track_caller]
pub fn warn(msg: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        emit(Level::Warn, msg, Location::caller());
    }
    #[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
    {
        #[cfg(debug_assertions)]
        {
            let fmt = format!("[WARN]  {}", msg);
            let location = extract_location(Location::caller(), &fmt, "warn");
            console::eval(&location);
        }
        #[cfg(not(debug_assertions))]
        {
            let fmt = format!("[WARN]  {}", msg);
            console::warn(&fmt);
        }
    }
    #[cfg(all(target_arch = "wasm32", aimer_portable_guest))]
    let _ = msg;
}

#[track_caller]
pub fn error(msg: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        emit(Level::Error, msg, Location::caller());
    }
    #[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
    {
        #[cfg(debug_assertions)]
        {
            let fmt = format!("[ERROR] {}", msg);
            let location = extract_location(Location::caller(), &fmt, "error");
            console::eval(&location);
        }
        #[cfg(not(debug_assertions))]
        {
            let fmt = format!("[ERROR] {}", msg);
            console::error(&fmt);
        }
    }
    #[cfg(all(target_arch = "wasm32", aimer_portable_guest))]
    let _ = msg;
}

#[track_caller]
pub fn debug(msg: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        emit(Level::Debug, msg, Location::caller());
    }
    #[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
    {
        #[cfg(debug_assertions)]
        {
            let fmt = format!("[DEBUG] {}", msg);
            let location = extract_location(Location::caller(), &fmt, "log");
            console::eval(&location);
        }
        #[cfg(not(debug_assertions))]
        {
            let fmt = format!("[DEBUG] {}", msg);
            console::log(&fmt);
        }
    }
    #[cfg(all(target_arch = "wasm32", aimer_portable_guest))]
    let _ = msg;
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        $crate::log::log(&format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        $crate::log::warn(&format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        $crate::log::error(&format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{
        // #[cfg(debug_assertions)]
        $crate::log::debug(&format!($($arg)*));
    }};
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[track_caller]
    fn record(level: Level, message: &str) -> String {
        json_record(level, message, Location::caller())
    }

    #[test]
    fn escape_json_leaves_plain_text_untouched() {
        assert_eq!(escape_json("plain message"), "plain message");
    }

    #[test]
    fn escape_json_escapes_quotes_and_backslashes() {
        assert_eq!(escape_json(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape_json(r"C:\path"), r"C:\\path");
    }

    #[test]
    fn escape_json_escapes_whitespace_controls() {
        assert_eq!(escape_json("a\nb\tc\rd"), "a\\nb\\tc\\rd");
    }

    #[test]
    fn escape_json_escapes_other_controls_as_unicode() {
        assert_eq!(escape_json("\u{1}"), "\\u0001");
        assert_eq!(escape_json("\u{8}"), "\\b");
        assert_eq!(escape_json("\u{c}"), "\\f");
    }

    #[test]
    fn escape_json_keeps_non_ascii_verbatim() {
        assert_eq!(escape_json("héllo ⠋"), "héllo ⠋");
    }

    #[test]
    fn json_record_is_one_compact_line() {
        let line = record(Level::Info, "started");
        assert!(!line.contains('\n'));
        assert!(line.starts_with('{') && line.ends_with('}'));
    }

    #[test]
    fn json_record_carries_marker_level_and_message() {
        let line = record(Level::Error, "boom");
        assert!(line.contains(r#""__aimer":1"#));
        assert!(line.contains(r#""level":"error""#));
        assert!(line.contains(r#""message":"boom""#));
    }

    #[test]
    fn json_record_carries_caller_location() {
        let line = record(Level::Warn, "careful");
        // `file!()` is the path as the compiler saw it: backslashes on Windows.
        assert!(line.contains(&format!(r#""file":"{}""#, escape_json(file!()))));
        assert!(line.contains(r#""line":"#));
    }

    #[test]
    fn json_record_uses_lowercase_level_names() {
        assert!(record(Level::Info, "m").contains(r#""level":"info""#));
        assert!(record(Level::Warn, "m").contains(r#""level":"warn""#));
        assert!(record(Level::Error, "m").contains(r#""level":"error""#));
        assert!(record(Level::Debug, "m").contains(r#""level":"debug""#));
    }

    #[test]
    fn json_record_escapes_the_message() {
        let line = record(Level::Info, "quoted: \"value\"\nnext");
        assert!(line.contains(r#""message":"quoted: \"value\"\nnext""#));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn json_record_of_empty_message_is_still_valid() {
        assert!(record(Level::Debug, "").contains(r#""message":"""#));
    }

    #[test]
    fn level_labels_are_width_aligned() {
        let widths =
            [Level::Info, Level::Warn, Level::Error, Level::Debug].map(|level| level.label().len());
        assert!(widths.iter().all(|w| *w == widths[0]));
    }

    #[test]
    fn json_output_flag_matches_the_documented_spelling() {
        assert_eq!(JSON_OUTPUT_FLAG, "--json-output");
        assert_eq!(JSON_OUTPUT_ENV, "AIMER_JSON_OUTPUT");
    }
}
