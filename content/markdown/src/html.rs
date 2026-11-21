//! A forgiving reader of the HTML that Markdown documents carry in them: README files and documentation comments write
//! `<img>`, `<br>`, `<kbd>`, `<details>` and `<p align="center">` among their Markdown.
//!
//! This is not an HTML parser: it cuts text into tags and text and decodes entities, and what it is given is never an error.
//! A `<` that does not start a tag is text, a tag that is not closed by the end of the text is text, and a comment or a
//! declaration is dropped. What the tags mean is for [`crate::html_blocks`].

use std::borrow::Cow;

/// The elements that have no content and no closing tag.
const VOID_ELEMENTS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];

/// What is in a piece of HTML.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    /// Text, with its entities decoded.
    Text(String),
    /// A start tag, with its name in lower case. A void element such as `<br>` is `self_closing`, as is `<x/>`.
    Open { name: String, attributes: Vec<(String, String)>, self_closing: bool },
    /// An end tag, with its name in lower case.
    Close { name: String },
}

impl Token {
    /// The value of the attribute `name` of a start tag, which is in lower case.
    pub(crate) fn attribute<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
        attributes
            .iter()
            .find(|(attribute, _)| attribute == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Cuts `source` into tags and text.
pub(crate) fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    // Where the text that has not been given out begins, and where to look for the next `<`.
    let (mut text_from, mut at) = (0, 0);
    while let Some(found) = source[at..].find('<') {
        let start = at + found;
        match read_markup(source, start) {
            Some((token, end)) => {
                push_text(&mut tokens, &source[text_from..start]);
                tokens.extend(token);
                (text_from, at) = (end, end);
            }
            // A `<` that is not a tag stays in the text that is being read.
            None => at = start + 1,
        }
    }
    push_text(&mut tokens, &source[text_from..]);
    tokens
}

/// Adds `raw`, decoded, to the text that ends `tokens`, or makes it a text of its own.
fn push_text(tokens: &mut Vec<Token>, raw: &str) {
    if raw.is_empty() {
        return;
    }
    let decoded = decode_entities(raw);
    match tokens.last_mut() {
        Some(Token::Text(text)) => text.push_str(&decoded),
        _ => tokens.push(Token::Text(decoded.into_owned())),
    }
}

/// What the `<` at the byte `start` begins, and where it ends: a tag, or a comment or a declaration, which is no token. `None` where it
/// begins nothing, or what it begins is not ended.
fn read_markup(source: &str, start: usize) -> Option<(Option<Token>, usize)> {
    let rest = &source[start..];
    let bytes = rest.as_bytes();
    if rest.starts_with("<!--") {
        // A comment that is not ended takes the rest of the text.
        let end = rest[4..].find("-->").map_or(source.len(), |close| start + 4 + close + 3);
        return Some((None, end));
    }
    if rest.starts_with("<!") || rest.starts_with("<?") {
        return rest.find('>').map(|close| (None, start + close + 1));
    }
    if rest.starts_with("</") {
        if !bytes.get(2)?.is_ascii_alphabetic() {
            return None;
        }
        let name_end = 2 + bytes[2..].iter().take_while(|byte| is_name_byte(**byte)).count();
        let close = rest.find('>')?;
        let name = rest[2..name_end].to_ascii_lowercase();
        return Some((Some(Token::Close { name }), start + close + 1));
    }
    if !bytes.get(1)?.is_ascii_alphabetic() {
        return None;
    }
    let name_end = 1 + bytes[1..].iter().take_while(|byte| is_name_byte(**byte)).count();
    let name = rest[1..name_end].to_ascii_lowercase();
    let mut attributes = Vec::new();
    let mut at = name_end;
    let mut self_closing = VOID_ELEMENTS.contains(&name.as_str());
    loop {
        at += bytes[at.min(bytes.len())..].iter().take_while(|byte| byte.is_ascii_whitespace()).count();
        match *bytes.get(at)? {
            b'>' => {
                at += 1;
                break;
            }
            b'/' => {
                if bytes.get(at + 1) == Some(&b'>') {
                    self_closing = true;
                    at += 2;
                    break;
                }
                at += 1;
            }
            _ => {
                let name_len = bytes[at..]
                    .iter()
                    .take_while(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'=' | b'>' | b'/'))
                    .count();
                // A byte that no name can begin with is passed over.
                if name_len == 0 {
                    at += 1;
                    continue;
                }
                let attribute = rest[at..at + name_len].to_ascii_lowercase();
                at += name_len;
                at += bytes[at..].iter().take_while(|byte| byte.is_ascii_whitespace()).count();
                let mut value = String::new();
                if bytes.get(at) == Some(&b'=') {
                    at += 1;
                    at += bytes[at.min(bytes.len())..].iter().take_while(|byte| byte.is_ascii_whitespace()).count();
                    match *bytes.get(at)? {
                        quote @ (b'"' | b'\'') => {
                            let length = bytes[at + 1..].iter().position(|byte| *byte == quote)?;
                            value = decode_entities(&rest[at + 1..at + 1 + length]).into_owned();
                            at += length + 2;
                        }
                        _ => {
                            let length = bytes[at..]
                                .iter()
                                .take_while(|byte| !byte.is_ascii_whitespace() && **byte != b'>')
                                .count();
                            value = decode_entities(&rest[at..at + length]).into_owned();
                            at += length;
                        }
                    }
                }
                attributes.push((attribute, value));
            }
        }
    }
    Some((Some(Token::Open { name, attributes, self_closing }), start + at))
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_')
}

/// `text` with the character references in it, `&amp;`, `&#38;` and `&#x26;`, made the characters they stand for. A reference that is
/// not known, or not ended, is left as it is written.
pub(crate) fn decode_entities(text: &str) -> Cow<'_, str> {
    let Some(first) = text.find('&') else {
        return Cow::Borrowed(text);
    };
    let mut decoded = String::with_capacity(text.len());
    decoded.push_str(&text[..first]);
    let mut rest = &text[first..];
    while let Some(at) = rest.find('&') {
        decoded.push_str(&rest[..at]);
        rest = &rest[at..];
        // The longest reference that is known is far shorter than this, and a `;` further on is not this one's.
        let reference = rest[1..]
            .find(';')
            .filter(|end| *end <= MOST_REFERENCE_BYTES)
            .and_then(|end| reference_char(&rest[1..1 + end]).map(|character| (character, end + 2)));
        match reference {
            Some((character, length)) => {
                decoded.push(character);
                rest = &rest[length..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    Cow::Owned(decoded)
}

/// The most bytes between the `&` and the `;` of a reference that is read.
const MOST_REFERENCE_BYTES: usize = 10;

/// The character that the reference `name`, which is what is between the `&` and the `;`, stands for.
fn reference_char(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code).filter(|character| *character != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "mdash" => '\u{2014}',
        "ndash" => '\u{2013}',
        "hellip" => '\u{2026}',
        "laquo" => '\u{ab}',
        "raquo" => '\u{bb}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "bull" => '\u{2022}',
        "middot" => '\u{b7}',
        "times" => '\u{d7}',
        "deg" => '\u{b0}',
        "larr" => '\u{2190}',
        "rarr" => '\u{2192}',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str, attributes: &[(&str, &str)], self_closing: bool) -> Token {
        Token::Open {
            name: name.to_owned(),
            attributes: attributes.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect(),
            self_closing,
        }
    }

    fn close(name: &str) -> Token {
        Token::Close { name: name.to_owned() }
    }

    fn text(text: &str) -> Token {
        Token::Text(text.to_owned())
    }

    #[test]
    fn text_and_tags_are_told_apart_and_names_are_lower_case() {
        assert_eq!(
            tokenize("a <B>bold</B> c"),
            [text("a "), open("b", &[], false), text("bold"), close("b"), text(" c")]
        );
        assert_eq!(tokenize(""), []);
        assert_eq!(tokenize("plain"), [text("plain")]);
    }

    #[test]
    fn attributes_are_read_quoted_bare_and_without_a_value() {
        assert_eq!(
            tokenize(r#"<img src="a b.png" alt='x "y"' width=40 hidden>"#),
            [open("img", &[("src", "a b.png"), ("alt", "x \"y\""), ("width", "40"), ("hidden", "")], true)]
        );
        assert_eq!(tokenize(r#"<P ALIGN="Center">"#), [open("p", &[("align", "Center")], false)], "names are lower case, values as written");
        assert_eq!(Token::attribute(&[("a".into(), "1".into())], "a"), Some("1"));
        assert_eq!(Token::attribute(&[], "a"), None);
    }

    #[test]
    fn void_elements_and_empty_element_tags_close_themselves() {
        assert_eq!(tokenize("a<br>b"), [text("a"), open("br", &[], true), text("b")]);
        assert_eq!(tokenize("<br/>"), [open("br", &[], true)]);
        assert_eq!(tokenize("<br />"), [open("br", &[], true)]);
        assert_eq!(tokenize(r#"<img src="x"/>"#), [open("img", &[("src", "x")], true)]);
        assert_eq!(tokenize("<span/>"), [open("span", &[], true)]);
        assert_eq!(tokenize("<hr>"), [open("hr", &[], true)]);
    }

    #[test]
    fn a_less_than_sign_that_starts_no_tag_is_text_and_so_is_a_tag_that_never_ends() {
        assert_eq!(tokenize("1 < 2"), [text("1 < 2")]);
        assert_eq!(tokenize("a <3 b"), [text("a <3 b")]);
        assert_eq!(tokenize("x <b"), [text("x <b")], "the end of the text is in a tag: it was not one");
        assert_eq!(tokenize(r#"<a href="x"#), [text(r#"<a href="x"#)]);
        assert_eq!(tokenize("</ >"), [text("</ >")]);
    }

    #[test]
    fn comments_declarations_and_processing_instructions_are_dropped() {
        assert_eq!(tokenize("a<!-- <b> -->b"), [text("ab")], "the text on both sides of it is one text");
        assert_eq!(tokenize("<!DOCTYPE html>x"), [text("x")]);
        assert_eq!(tokenize("<?xml version=\"1.0\"?>x"), [text("x")]);
        assert_eq!(tokenize("a<!-- never ends"), [text("a")], "a comment that is not ended takes the rest");
    }

    #[test]
    fn a_greater_than_sign_inside_a_quoted_value_does_not_end_the_tag() {
        assert_eq!(tokenize(r#"<a title="a>b" href="x">"#), [open("a", &[("title", "a>b"), ("href", "x")], false)]);
    }

    #[test]
    fn entities_are_decoded_in_text_and_in_values() {
        assert_eq!(tokenize("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#39; &nbsp;"), [text("a & b <c> \"d\" 'e' \u{a0}")]);
        assert_eq!(tokenize(r#"<a title="&lt;x&gt;" href="a?b=1&amp;c=2">"#), [open("a", &[("title", "<x>"), ("href", "a?b=1&c=2")], false)]);
        assert_eq!(decode_entities("&#x1F600;&#128512;"), "\u{1F600}\u{1F600}");
        assert_eq!(decode_entities("&copy; &mdash; &hellip;"), "\u{a9} \u{2014} \u{2026}");
    }

    #[test]
    fn what_is_not_a_known_or_a_whole_entity_is_left_as_written() {
        assert_eq!(decode_entities("&unknown; & &amp"), "&unknown; & &amp");
        assert_eq!(decode_entities("&#xZZ; &#; &#99999999999;"), "&#xZZ; &#; &#99999999999;");
        assert_eq!(decode_entities("&#0;"), "&#0;", "no character for it");
        assert_eq!(decode_entities("no entity"), "no entity");
        assert!(matches!(decode_entities("no entity"), Cow::Borrowed(_)), "and nothing is copied for it");
    }

    #[test]
    fn many_bytes_and_combining_marks_are_kept_whole() {
        assert_eq!(tokenize("\u{e9}\u{301}<b>\u{1F600}</b>"), [text("\u{e9}\u{301}"), open("b", &[], false), text("\u{1F600}"), close("b")]);
    }

    #[test]
    fn a_very_long_text_and_a_very_long_tag_are_read() {
        let long = "x".repeat(100_000);
        assert_eq!(tokenize(&long), [text(&long)]);
        let value = "v".repeat(100_000);
        let tokens = tokenize(&format!(r#"<a href="{value}">"#));
        assert_eq!(tokens, [open("a", &[("href", value.as_str())], false)]);
    }
}
