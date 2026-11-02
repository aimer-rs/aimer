//! The optional, Sublime Text syntax backed highlighting backend.
//!
//! [`syntect`](https://docs.rs/syntect) runs `.sublime-syntax` definitions, the
//! format Sublime Text, `bat` and many editors share. The bundled set covers
//! every language the `arborium` backend does, apart from PowerShell, and
//! applications can add their own with [`add_syntax_from_str`],
//! [`add_syntax_from_file`] and [`add_syntax_from_dir`]. It is only compiled
//! when the `syntect` feature is enabled.
//!
//! Definitions label text with scopes; [`scope`] folds those into the same
//! categories the other backends emit, so no theme file is needed and the
//! renderer's colors apply unchanged.

mod registry;
mod scope;

use std::collections::HashMap;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet, SyntaxSetBuilder};

pub use self::registry::{add_syntax_from_dir, add_syntax_from_file, add_syntax_from_str};
use self::scope::Class;
use super::CaptureSpan;
use super::synoptic::SynopticBackend;

pub(super) use self::registry::revision;

/// Fenced-code tags that are neither a name nor a file extension of the
/// syntax they conventionally mean. Compared ASCII case-insensitively.
const LANGUAGE_ALIASES: &[(&str, &str)] = &[
    ("jsx", "js"),
    ("csharp", "cs"),
    ("golang", "go"),
    ("shell", "sh"),
];

/// The syntect backed highlighting backend.
///
/// The compiled [`SyntaxSet`] and the scope classification table are kept
/// across parses, since building them dominates the cost of highlighting a
/// short snippet. They are rebuilt only when the registry's revision moves.
pub(crate) struct SyntectBackend {
    syntaxes: SyntaxSet,
    /// The registry revision `syntaxes` was built from.
    revision: u64,
    /// [`scope::classify`] results; a document uses a few dozen distinct scopes
    /// however much code it holds, and the lookup avoids rendering each one to
    /// a string on every operation.
    classes: HashMap<Scope, Class>,
    /// Languages the Sublime set lacks keep the highlighting they have without
    /// this feature instead of turning into plain text.
    fallback: SynopticBackend,
}

impl SyntectBackend {
    pub(crate) fn new() -> Self {
        let (revision, added) = registry::snapshot();
        Self {
            syntaxes: build_syntax_set(&added),
            revision,
            classes: HashMap::new(),
            fallback: SynopticBackend::new(),
        }
    }

    /// Highlight `code`, returning the captures of the supported `language`, or
    /// nothing at all when the language is unknown or absent.
    pub(crate) fn highlight(&mut self, code: &str, language: Option<&str>) -> Vec<CaptureSpan> {
        let Some(language) = language else {
            return Vec::new();
        };
        self.refresh();

        let Some(syntax) = find_syntax(&self.syntaxes, language) else {
            return self.fallback.highlight(code, Some(language));
        };
        // `syntax` borrows `self.syntaxes` while the table below is mutated.
        let syntax: &SyntaxReference = syntax;
        let syntaxes = &self.syntaxes;
        let classes = &mut self.classes;
        highlight_with(syntaxes, syntax, classes, code)
    }

    /// Rebuilds the syntax set if syntaxes were added since it was built.
    fn refresh(&mut self) {
        if registry::revision() == self.revision {
            return;
        }
        let (revision, added) = registry::snapshot();
        self.syntaxes = build_syntax_set(&added);
        self.revision = revision;
    }
}

/// The bundled syntaxes plus the ones applications added on top.
///
/// Adding requires taking the whole bundled set apart, which forces every
/// syntax to be decoded, so a build without additions keeps the lazy set.
fn build_syntax_set(added: &[std::sync::Arc<syntect::parsing::SyntaxDefinition>]) -> SyntaxSet {
    let bundled = two_face::syntax::extra_newlines();
    if added.is_empty() {
        return bundled;
    }
    let mut builder: SyntaxSetBuilder = bundled.into_builder();
    for definition in added {
        builder.add((**definition).clone());
    }
    builder.build()
}

fn find_syntax<'a>(syntaxes: &'a SyntaxSet, language: &str) -> Option<&'a SyntaxReference> {
    let token = LANGUAGE_ALIASES
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(language))
        .map_or(language, |(_, token)| *token);
    syntaxes.find_syntax_by_token(token)
}

fn highlight_with(
    syntaxes: &SyntaxSet,
    syntax: &SyntaxReference,
    classes: &mut HashMap<Scope, Class>,
    code: &str,
) -> Vec<CaptureSpan> {
    let mut spans = Vec::new();
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut pending: Option<(&'static str, usize, usize)> = None;
    let mut line_start = 0usize;

    for line in code.split_inclusive('\n') {
        // The parser expects every line to end in a newline, and the last
        // line of a snippet rarely does.
        let terminated;
        let parsed = if line.ends_with('\n') {
            line
        } else {
            terminated = format!("{line}\n");
            &terminated
        };
        let Ok(ops) = state.parse_line(parsed, syntaxes) else {
            // A pathological line (the regex engine gives up) leaves the
            // parser in an unknown state, so keep what is already known.
            break;
        };

        let mut cursor = 0usize;
        for (position, op) in ops {
            if position > cursor {
                emit(
                    &mut spans,
                    &mut pending,
                    category_of(&stack, classes),
                    line_start + cursor,
                    // The appended newline is not part of `code`.
                    line_start + position.min(line.len()),
                );
                cursor = position;
            }
            // An op the stack rejects only means a malformed definition; the
            // text keeps its previous category.
            let _ = stack.apply(&op);
        }
        if cursor < line.len() {
            emit(
                &mut spans,
                &mut pending,
                category_of(&stack, classes),
                line_start + cursor,
                line_start + line.len(),
            );
        }
        line_start += line.len();
    }
    flush(&mut spans, &mut pending);
    spans
}

fn category_of(stack: &ScopeStack, classes: &mut HashMap<Scope, Class>) -> Option<&'static str> {
    scope::category(stack.as_slice().iter().map(|scope| {
        *classes
            .entry(*scope)
            .or_insert_with(|| scope::classify(&scope.build_string()))
    }))
}

/// Extends the span being built, or starts a new one when the category changes
/// or the text is not contiguous.
///
/// Merging is what turns `"` `text` `"` — three differently scoped runs — into
/// the one string span a reader sees, and keeps a block comment one span
/// instead of one per line.
fn emit(
    spans: &mut Vec<CaptureSpan>,
    pending: &mut Option<(&'static str, usize, usize)>,
    category: Option<&'static str>,
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }
    match (category, &mut *pending) {
        (Some(name), Some((current, _, current_end))) if *current == name && *current_end == start => {
            *current_end = end;
        }
        _ => {
            flush(spans, pending);
            *pending = category.map(|name| (name, start, end));
        }
    }
}

fn flush(spans: &mut Vec<CaptureSpan>, pending: &mut Option<(&'static str, usize, usize)>) {
    if let Some((name, start, end)) = pending.take() {
        spans.push(CaptureSpan::from_capture(name, start as u32, end as u32));
    }
}

#[cfg(test)]
mod tests;
