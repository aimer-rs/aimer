//! `file://` URIs, as the desktop portal reports chosen files.

use std::path::PathBuf;

/// The path a `file://` URI names, with its percent-escapes decoded.
///
/// Returns `None` for any other scheme, for a host other than empty or
/// `localhost`, and for escapes that do not decode to UTF-8.
pub(crate) fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = match rest.split_once('/') {
        Some(("" | "localhost", path)) => path,
        _ => return None,
    };

    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len() + 1);
    decoded.push(b'/');
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = bytes.get(index + 1..index + 3)?;
            let digits = std::str::from_utf8(pair).ok()?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_are_taken_as_they_are() {
        assert_eq!(path_from_uri("file:///home/me/a.txt"), Some(PathBuf::from("/home/me/a.txt")));
        assert_eq!(path_from_uri("file://localhost/tmp/a"), Some(PathBuf::from("/tmp/a")));
    }

    #[test]
    fn escapes_are_decoded() {
        assert_eq!(
            path_from_uri("file:///home/me/My%20Pictures/caf%C3%A9.png"),
            Some(PathBuf::from("/home/me/My Pictures/café.png"))
        );
    }

    #[test]
    fn other_schemes_hosts_and_bad_escapes_are_refused() {
        assert_eq!(path_from_uri("https://example.com/a"), None);
        assert_eq!(path_from_uri("file://other-host/a"), None);
        assert_eq!(path_from_uri("file:///a%2"), None);
        assert_eq!(path_from_uri("file:///a%zz"), None);
        assert_eq!(path_from_uri("file:///a%FF"), None);
    }
}
