//! The first line a client sends: `hello TOKEN [NAME]`.
//!
//! The name says who is attached, and the app shows it, so it is limited to a
//! short run of plain characters: it comes from another process and must never
//! be able to bend the app's display or its logs.

/// The longest name a client may give itself.
pub(crate) const MAX_NAME: usize = 32;

/// What a client called itself when it said hello.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Hello<'a> {
    pub(crate) token: &'a str,
    pub(crate) name: Option<&'a str>,
}

/// Whether `name` is something a client may call itself: one to [`MAX_NAME`]
/// characters from letters, digits and `. _ : / @ + -`, so no whitespace,
/// control characters or quoting.
pub(crate) fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/@+-".contains(&b))
}

/// The hello line for `token`, naming the client when `name` is given.
pub(crate) fn line(token: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("hello {token} {name}"),
        None => format!("hello {token}"),
    }
}

/// Reads a hello line. `None` when it is not one, or says more than a token and
/// a name; whether the name is acceptable is [`is_valid_name`]'s to say.
pub(crate) fn parse(text: &str) -> Option<Hello<'_>> {
    let mut words = text.strip_prefix("hello ")?.split_ascii_whitespace();
    let token = words.next()?;
    let name = words.next();
    words.next().is_none().then_some(Hello { token, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hello_with_and_without_a_name_is_read() {
        assert_eq!(parse("hello abc"), Some(Hello { token: "abc", name: None }));
        assert_eq!(parse("hello abc aimer-cli"), Some(Hello { token: "abc", name: Some("aimer-cli") }));
    }

    #[test]
    fn anything_else_is_not_a_hello() {
        for text in ["", "hello", "hello ", "hi abc", "hello abc name extra", "move 1 2"] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn lines_round_trip() {
        assert_eq!(parse(&line("t", None)), Some(Hello { token: "t", name: None }));
        assert_eq!(parse(&line("t", Some("mcp:claude"))), Some(Hello { token: "t", name: Some("mcp:claude") }));
    }

    #[test]
    fn names_are_short_plain_and_nonempty() {
        for good in ["aimer-cli", "mcp:claude", "a", "a.b_c/d@e+f", &"x".repeat(MAX_NAME)] {
            assert!(is_valid_name(good), "{good}");
        }
        for bad in ["", "two words", "tab\t", "new\nline", "quo\"te", "back`tick", "é", &"x".repeat(MAX_NAME + 1)] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
    }
}
