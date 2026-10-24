//! The process-wide set of syntaxes applications have added.
//!
//! Highlighting happens on whichever thread builds the widgets, and each of
//! those threads keeps its own cache and compiled [`SyntaxSet`]. Registration
//! can come from any thread, so definitions live here behind a lock and every
//! change bumps a revision; a thread notices the new revision on its next
//! highlight and rebuilds, so a syntax added late is picked up everywhere
//! without the threads ever being told.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use syntect::parsing::SyntaxDefinition;

use crate::MarkdownError;

/// File extension of Sublime Text syntax definitions.
const SYNTAX_EXTENSION: &str = "sublime-syntax";

static REVISION: AtomicU64 = AtomicU64::new(0);
static SYNTAXES: Mutex<Vec<Arc<SyntaxDefinition>>> = Mutex::new(Vec::new());

/// Identifies the current set of added syntaxes; it changes whenever one is
/// added and never otherwise.
#[inline]
pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

/// The added syntaxes in registration order, together with the revision they
/// belong to.
///
/// Later syntaxes take precedence over earlier ones and over the bundled set
/// when they claim the same name or file extension.
pub(crate) fn snapshot() -> (u64, Vec<Arc<SyntaxDefinition>>) {
    let syntaxes = lock();
    // Read under the lock so the pair is consistent with `register`.
    (revision(), syntaxes.clone())
}

fn lock() -> MutexGuard<'static, Vec<Arc<SyntaxDefinition>>> {
    // A panic cannot leave the vector half updated, so a poisoned lock is safe
    // to keep using.
    SYNTAXES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Adds every definition in one step: either all of them are visible to
/// highlighting afterwards or none are.
fn register(definitions: Vec<SyntaxDefinition>) {
    if definitions.is_empty() {
        return;
    }
    let mut syntaxes = lock();
    syntaxes.extend(definitions.into_iter().map(Arc::new));
    REVISION.fetch_add(1, Ordering::Release);
}

fn parse(
    source: &str,
    path: Option<&Path>,
    fallback_name: Option<&str>,
) -> Result<SyntaxDefinition, MarkdownError> {
    // The bundled set is loaded "with newlines", so definitions must agree.
    SyntaxDefinition::load_from_str(source, true, fallback_name).map_err(|error| {
        MarkdownError::new(match path {
            Some(path) => format!("Invalid syntax definition {}: {error}", path.display()),
            None => format!("Invalid syntax definition: {error}"),
        })
    })
}

/// Adds a Sublime Text `.sublime-syntax` definition given as YAML text.
///
/// The definition must carry its own `name`. Fenced code blocks select it by
/// that name or by any of its `file_extensions`, compared case-insensitively,
/// so a definition with `file_extensions: [aimer]` highlights
/// ` ```aimer ` blocks. A definition that claims a name or extension the
/// bundled set already uses replaces it, and a later definition replaces an
/// earlier one.
///
/// Embedding another language works as in Sublime Text, through
/// `embed: scope:source.json`, and resolves against everything registered.
///
/// # Errors
///
/// Returns a [`MarkdownError`] describing the problem, and registers nothing,
/// when the text is not a valid definition (bad YAML, a missing `name` or
/// `main` context, an invalid regular expression or scope).
///
/// # Examples
///
/// ```
/// aimer_markdown::add_syntax_from_str(
///     r#"%YAML 1.2
/// ---
/// name: Shout
/// file_extensions: [shout]
/// scope: source.shout
/// contexts:
///   main:
///     - match: '\b[A-Z]+\b'
///       scope: keyword.control.shout
/// "#,
/// )
/// .unwrap();
/// ```
pub fn add_syntax_from_str(source: &str) -> Result<(), MarkdownError> {
    register(vec![parse(source, None, None)?]);
    Ok(())
}

/// Adds the Sublime Text syntax definition stored in `path`.
///
/// This behaves like [`add_syntax_from_str`] with the file's contents, except
/// that a definition without a `name` is named after the file.
///
/// # Errors
///
/// Returns a [`MarkdownError`] naming `path` when it cannot be read or does
/// not hold a valid definition; nothing is registered then.
pub fn add_syntax_from_file(path: impl AsRef<Path>) -> Result<(), MarkdownError> {
    register(vec![load(path.as_ref())?]);
    Ok(())
}

/// Adds every `.sublime-syntax` file below `directory`, descending into
/// subdirectories, and returns how many were added.
///
/// Files are added in path order, so which definition wins when several claim
/// the same name or extension is reproducible. Other files are ignored.
///
/// # Errors
///
/// Returns a [`MarkdownError`] naming the offending path when the directory
/// cannot be read or any file is invalid. The directory is added as a whole or
/// not at all, so a typo in one file cannot leave the rest half applied.
pub fn add_syntax_from_dir(directory: impl AsRef<Path>) -> Result<usize, MarkdownError> {
    let mut files = Vec::new();
    collect_files(directory.as_ref(), &mut files)?;
    files.sort();

    let definitions = files
        .iter()
        .map(|path| load(path))
        .collect::<Result<Vec<_>, _>>()?;
    let count = definitions.len();
    register(definitions);
    Ok(count)
}

fn load(path: &Path) -> Result<SyntaxDefinition, MarkdownError> {
    let source = fs::read_to_string(path).map_err(|error| {
        MarkdownError::new(format!("Cannot read syntax definition {}: {error}", path.display()))
    })?;
    let stem = path.file_stem().and_then(|stem| stem.to_str());
    parse(&source, Some(path), stem)
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), MarkdownError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        MarkdownError::new(format!("Cannot read syntax directory {}: {error}", directory.display()))
    })?;
    for entry in entries {
        let path = entry
            .map_err(|error| {
                MarkdownError::new(format!(
                    "Cannot read syntax directory {}: {error}",
                    directory.display()
                ))
            })?
            .path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(SYNTAX_EXTENSION))
        {
            files.push(path);
        }
    }
    Ok(())
}
