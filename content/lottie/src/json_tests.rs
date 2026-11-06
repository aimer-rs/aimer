use super::json::{Json, JsonErrorKind, MAX_DEPTH};
use std::borrow::Cow;

fn parse(text: &str) -> Result<Json<'_>, (usize, JsonErrorKind)> {
    Json::parse(text.as_bytes()).map_err(|e| (e.offset, e.kind))
}

fn kind(text: &str) -> JsonErrorKind {
    parse(text).expect_err("input must be rejected").1
}

#[test]
fn literals() {
    assert_eq!(parse("null"), Ok(Json::Null));
    assert_eq!(parse("true"), Ok(Json::Bool(true)));
    assert_eq!(parse(" false "), Ok(Json::Bool(false)));
    assert_eq!(kind("nul"), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind("nulx"), JsonErrorKind::UnexpectedByte);
}

#[test]
fn numbers() {
    for (text, want) in [
        ("0", 0.0),
        ("-0", -0.0),
        ("12", 12.0),
        ("-3.5", -3.5),
        ("0.5", 0.5),
        ("1e3", 1000.0),
        ("1E-2", 0.01),
        ("2.5e+1", 25.0),
    ] {
        assert_eq!(parse(text), Ok(Json::Number(want)), "{text}");
    }
}

#[test]
fn rejects_malformed_numbers() {
    for text in ["01", "-", "1.", ".5", "+1", "1e", "1e+", "--1", "0x10"] {
        assert!(parse(text).is_err(), "{text} must be rejected");
    }
    assert_eq!(kind("1."), JsonErrorKind::InvalidNumber);
}

#[test]
fn strings_and_escapes() {
    assert_eq!(parse(r#""abc""#), Ok(Json::String("abc".into())));
    assert_eq!(parse(r#""a\"b\\c\/d\n\t""#), Ok(Json::String("a\"b\\c/d\n\t".into())));
    assert_eq!(parse(r#""é""#), Ok(Json::String("é".into())));
    // U+1F600 as a surrogate pair.
    assert_eq!(parse(r#""😀""#), Ok(Json::String("😀".into())));
    assert_eq!(parse("\"héllo\""), Ok(Json::String("héllo".into())));
}

#[test]
fn string_without_escapes_borrows() {
    match parse(r#""plain""#) {
        Ok(Json::String(Cow::Borrowed("plain"))) => {}
        other => panic!("expected a borrowed string, got {other:?}"),
    }
    match parse(r#""a\nb""#) {
        Ok(Json::String(Cow::Owned(_))) => {}
        other => panic!("expected an owned string, got {other:?}"),
    }
}

#[test]
fn rejects_malformed_strings() {
    assert_eq!(kind(r#""abc"#), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind(r#""\q""#), JsonErrorKind::InvalidEscape);
    assert_eq!(kind(r#""\u12""#), JsonErrorKind::InvalidUnicode);
    assert_eq!(kind(r#""\ud83d""#), JsonErrorKind::InvalidUnicode);
    assert_eq!(kind(r#""\ude00""#), JsonErrorKind::InvalidUnicode);
    assert_eq!(kind("\"a\nb\""), JsonErrorKind::ControlInString);
    assert_eq!(Json::parse(b"\"\xff\"").unwrap_err().kind, JsonErrorKind::InvalidUtf8);
}

#[test]
fn arrays() {
    assert_eq!(parse("[]"), Ok(Json::Array(vec![])));
    assert_eq!(
        parse(" [ 1 , [ true ] , null ] "),
        Ok(Json::Array(vec![
            Json::Number(1.0),
            Json::Array(vec![Json::Bool(true)]),
            Json::Null,
        ]))
    );
    assert!(parse("[1,]").is_err());
    assert!(parse("[,1]").is_err());
    assert!(parse("[1 2]").is_err());
    assert_eq!(kind("[1"), JsonErrorKind::UnexpectedEnd);
}

#[test]
fn objects_and_lookup() {
    let value = parse(r#"{"v":"5.7","fr":30,"nested":{"a":[1]}}"#).expect("valid");
    assert_eq!(value.get("v"), Some(&Json::String("5.7".into())));
    assert_eq!(value.get("fr"), Some(&Json::Number(30.0)));
    assert_eq!(value.get("missing"), None);
    assert!(value.get("nested").and_then(|n| n.get("a")).is_some());
    assert_eq!(Json::Null.get("v"), None);

    assert_eq!(parse("{}"), Ok(Json::Object(vec![])));
    assert!(parse(r#"{"a":1,}"#).is_err());
    assert!(parse(r#"{"a" 1}"#).is_err());
    assert!(parse("{1:2}").is_err());
}

#[test]
fn duplicate_keys_last_wins() {
    let value = parse(r#"{"a":1,"a":2}"#).expect("valid");
    assert_eq!(value.get("a"), Some(&Json::Number(2.0)));
}

#[test]
fn empty_trailing_and_bom() {
    assert_eq!(kind(""), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind("   "), JsonErrorKind::UnexpectedEnd);
    assert_eq!(kind("1 2"), JsonErrorKind::TrailingData);
    assert_eq!(kind("{} x"), JsonErrorKind::TrailingData);
    assert_eq!(Json::parse(b"\xEF\xBB\xBF[1]"), Ok(Json::Array(vec![Json::Number(1.0)])));
}

#[test]
fn error_offset_points_at_the_byte() {
    assert_eq!(parse("[1, x]").unwrap_err(), (4, JsonErrorKind::UnexpectedByte));
    assert_eq!(parse("1 2").unwrap_err(), (2, JsonErrorKind::TrailingData));
}

#[test]
fn depth_limit() {
    let nest = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    assert!(parse(&nest(MAX_DEPTH)).is_ok());
    assert_eq!(kind(&nest(MAX_DEPTH + 1)), JsonErrorKind::TooDeep);
    // Far past the limit must fail cleanly instead of overflowing the stack.
    assert_eq!(kind(&"[".repeat(100_000)), JsonErrorKind::TooDeep);
}

#[test]
fn lottie_header_fixture() {
    let text = r#"{"v":"5.7.4","fr":60,"ip":0,"op":120,"w":512,"h":512,"layers":[
        {"ty":4,"nm":"box","ks":{"p":{"a":1,"k":[{"t":0,"s":[0,0]},{"t":60,"s":[100,50]}]}}}]}"#;
    let doc = parse(text).expect("valid");
    assert_eq!(doc.get("op"), Some(&Json::Number(120.0)));
    let Some(Json::Array(layers)) = doc.get("layers") else { panic!("layers must be an array") };
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].get("nm"), Some(&Json::String("box".into())));
}
