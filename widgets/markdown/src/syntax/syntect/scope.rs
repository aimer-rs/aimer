//! Maps Sublime Text scope names onto the crate's highlight categories.
//!
//! A `.sublime-syntax` file labels text with TextMate style scopes such as
//! `keyword.control.rust` or `string.quoted.double`. Those are far finer
//! grained than [`CaptureSpan`](super::CaptureSpan)'s categories, and they are
//! not tied to any theme, so the mapping is done here once per distinct scope
//! and the existing [`MarkdownTheme`](crate::MarkdownTheme) colors do the rest.

/// What a single scope contributes to the category of the text it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    /// The scope says nothing about highlighting (`source.rust`, `meta.block`).
    Skip,
    /// Names a category. Scopes deeper in the stack override it.
    Strong(&'static str),
    /// Names a category only for text no enclosing scope has categorized, so
    /// the quotes of a string stay string colored rather than punctuation.
    Weak(&'static str),
    /// Names a category that nothing nested inside it may override, so a doc
    /// comment full of markup stays one comment.
    Sticky(&'static str),
}

/// Whether `scope` is `prefix` or a dotted descendant of it.
#[inline]
fn is(scope: &str, prefix: &str) -> bool {
    scope
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// Whether the scope has `atom` as one of its dot separated segments.
#[inline]
fn has(scope: &str, atom: &str) -> bool {
    scope.split('.').any(|segment| segment == atom)
}

/// Classifies one scope, written as `Scope::build_string` renders it.
///
/// The names are the capture names [`CaptureSpan::from_capture`] already
/// understands, so this is the only table the `syntect` backend adds.
pub(super) fn classify(scope: &str) -> Class {
    use Class::*;

    if is(scope, "comment") {
        return Sticky("comment");
    }
    if is(scope, "string") {
        return Strong("string");
    }
    if is(scope, "constant.numeric") {
        return Strong("number");
    }
    if is(scope, "constant") {
        return Strong("constant");
    }
    if is(scope, "keyword.operator") {
        return Strong("operator");
    }
    if is(scope, "keyword") {
        return Strong("keyword");
    }
    if is(scope, "storage.type") {
        // `fn`, `struct` and friends are declarations, which a reader sees as
        // keywords; the plain `storage.type` of `int` or `u32` is a type.
        const DECLARATORS: &[&str] = &[
            "function", "struct", "enum", "trait", "impl", "class", "interface", "namespace",
            "module", "type", "union", "def", "let", "var", "const", "fn",
        ];
        // Only the segments after `storage.type`: the prefix itself contains
        // `type`, and the trailing one is the language (`.rust`, `.c`).
        let rest = scope.strip_prefix("storage.type").unwrap_or_default();
        return if DECLARATORS.iter().any(|declarator| has(rest, declarator)) {
            Strong("keyword")
        } else {
            Strong("type")
        };
    }
    if is(scope, "storage") {
        return Strong("keyword");
    }
    if is(scope, "entity.name.tag") {
        return Strong("tag");
    }
    if is(scope, "entity.other.attribute-name") {
        return Strong("attribute");
    }
    if is(scope, "entity.name.function.constructor") {
        return Strong("constructor");
    }
    if is(scope, "entity.name.function") || is(scope, "support.function") {
        return Strong(if has(scope, "macro") { "macro" } else { "function" });
    }
    if is(scope, "support.macro") {
        return Strong("macro");
    }
    if is(scope, "entity.name.namespace") || is(scope, "entity.name.module") {
        return Strong("namespace");
    }
    if is(scope, "entity.name.constant") || is(scope, "support.constant") {
        return Strong("constant");
    }
    if is(scope, "entity.name.label") {
        return Strong("label");
    }
    if is(scope, "entity.name.section") {
        return Strong("heading");
    }
    if is(scope, "entity.name")
        || is(scope, "entity.other.inherited-class")
        || is(scope, "support.type")
        || is(scope, "support.class")
    {
        return Strong("type");
    }
    if is(scope, "variable.function") {
        return Strong("function");
    }
    if is(scope, "variable.parameter") {
        return Strong("variable");
    }
    if is(scope, "variable.language") {
        return Strong("keyword");
    }
    if is(scope, "variable.annotation") {
        return Strong("attribute");
    }
    if is(scope, "variable.other.constant") {
        return Strong("constant");
    }
    if is(scope, "variable.other") && (has(scope, "member") || has(scope, "property")) {
        return Strong("property");
    }
    if is(scope, "punctuation") {
        return Weak("punctuation");
    }
    if is(scope, "invalid") {
        return Strong("error");
    }
    if is(scope, "markup.heading") {
        return Strong("heading");
    }
    if is(scope, "markup.bold") {
        return Strong("strong");
    }
    if is(scope, "markup.italic") {
        return Strong("emphasis");
    }
    if is(scope, "markup.underline.link") {
        return Strong("link");
    }
    if is(scope, "markup.raw") {
        return Strong("literal");
    }
    if is(scope, "markup.strikethrough") {
        return Strong("strikethrough");
    }
    if is(scope, "markup.inserted") {
        return Strong("insertion");
    }
    if is(scope, "markup.deleted") {
        return Strong("deletion");
    }
    Skip
}

/// The category of text covered by `stack`, outermost scope first.
///
/// Deeper scopes are more specific and win, with the exceptions [`Class`]
/// describes.
pub(super) fn category(stack: impl IntoIterator<Item = Class>) -> Option<&'static str> {
    let mut category = None;
    for class in stack {
        match class {
            Class::Skip => {}
            Class::Strong(name) => category = Some(name),
            Class::Weak(name) => {
                if category.is_none() {
                    category = Some(name);
                }
            }
            Class::Sticky(name) => return Some(name),
        }
    }
    category
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::CaptureSpan;

    #[test]
    fn scopes_match_whole_dotted_segments_only() {
        assert!(is("keyword.control.rust", "keyword"));
        assert!(is("keyword", "keyword"));
        assert!(!is("keywordish", "keyword"));
        assert_eq!(classify("keywordish.thing"), Class::Skip);
    }

    #[test]
    fn common_scopes_land_in_their_category() {
        for (scope, expected) in [
            ("keyword.control.rust", "keyword"),
            ("keyword.operator.arithmetic.rust", "operator"),
            ("storage.type.function.rust", "keyword"),
            ("storage.modifier.rust", "keyword"),
            ("storage.type.c", "type"),
            ("string.quoted.double.rust", "string"),
            ("constant.numeric.integer.decimal.rust", "number"),
            ("constant.language.boolean.true.rust", "constant"),
            ("entity.name.function.rust", "function"),
            ("entity.name.function.macro.rust", "macro"),
            ("entity.name.struct.rust", "type"),
            ("entity.name.tag.html", "tag"),
            ("entity.other.attribute-name.html", "attribute"),
            ("support.function.builtin.python", "function"),
            ("variable.parameter.rust", "variable"),
            ("variable.language.self.rust", "keyword"),
            ("variable.other.member.rust", "property"),
            ("markup.heading.markdown", "heading"),
            ("markup.inserted.diff", "insertion"),
            ("invalid.illegal.rust", "error"),
        ] {
            assert_eq!(category([classify(scope)]), Some(expected), "{scope}");
        }
    }

    #[test]
    fn scopes_that_do_not_style_text_are_skipped() {
        for scope in [
            "source.rust",
            "text.html.basic",
            "meta.function.rust",
            "variable.other.rust",
        ] {
            assert_eq!(classify(scope), Class::Skip, "{scope}");
        }
    }

    #[test]
    fn every_category_name_is_understood_by_capture_span() {
        // A name `from_capture` does not know would silently become
        // `CaptureSpan::Other`, which the theme paints a neutral grey.
        for scope in [
            "comment.line",
            "string.quoted",
            "constant.numeric",
            "constant.language",
            "keyword.operator",
            "keyword.control",
            "storage.type.function",
            "storage.type",
            "entity.name.tag",
            "entity.other.attribute-name",
            "entity.name.function.constructor",
            "entity.name.function",
            "entity.name.function.macro",
            "entity.name.namespace",
            "entity.name.label",
            "entity.name.section",
            "entity.name.class",
            "variable.parameter",
            "variable.other.member",
            "punctuation.separator",
            "invalid.illegal",
            "markup.bold",
            "markup.italic",
            "markup.underline.link",
            "markup.raw",
            "markup.strikethrough",
            "markup.inserted",
            "markup.deleted",
        ] {
            let name = category([classify(scope)]).unwrap_or_else(|| panic!("{scope}"));
            assert!(
                !matches!(
                    CaptureSpan::from_capture(name, 0, 1),
                    CaptureSpan::Other { .. }
                ),
                "{scope} -> {name}"
            );
        }
    }

    #[test]
    fn deeper_scopes_override_shallower_ones() {
        let stack = [
            classify("string.quoted.double"),
            classify("constant.character.escape"),
        ];
        assert_eq!(category(stack), Some("constant"));
    }

    #[test]
    fn punctuation_never_overrides_an_enclosing_category() {
        let quote = [
            classify("string.quoted.double"),
            classify("punctuation.definition.string.begin"),
        ];
        assert_eq!(category(quote), Some("string"));
        assert_eq!(
            category([classify("punctuation.section.block.begin")]),
            Some("punctuation")
        );
    }

    #[test]
    fn nothing_overrides_a_comment() {
        let stack = [
            classify("comment.block.documentation"),
            classify("markup.heading"),
            classify("keyword.other"),
        ];
        assert_eq!(category(stack), Some("comment"));
    }

    #[test]
    fn an_unstyled_stack_has_no_category() {
        assert_eq!(category([classify("source.rust"), classify("meta.block")]), None);
    }
}
