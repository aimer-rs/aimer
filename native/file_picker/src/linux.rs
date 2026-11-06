//! Linux: the `org.freedesktop.portal.FileChooser` desktop portal.
//!
//! The portal is the one picker that works the same under GNOME, KDE, Flatpak
//! and Snap, and the only one an unprivileged sandboxed app may use. The call is
//! a request/response pair over the session bus: `OpenFile` returns a request
//! object immediately, and the answer arrives later as that object's `Response`
//! signal. Both are blocking from this crate's point of view, so they run on
//! Venus's offload pool.
//!
//! The request object's path is predictable from the connection's name and a
//! token the caller picks, so the signal is subscribed to *before* `OpenFile` is
//! sent; otherwise a fast answer could be missed.
//!
//! No parent window is passed: Rust holds no X11 or Wayland window handle for
//! the portal's `parent_window` string. A shown dialog cannot be closed from
//! here, so dropping the future leaves it up until the user dismisses it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Array, OwnedObjectPath, OwnedValue, Signature, StructureBuilder, Value};

use crate::error::{PickError, PickResult};
use crate::file_uri::path_from_uri;
use crate::filter::FileFilter;
use crate::future::{PickFuture, ready};
use crate::picked::PickedFile;
use crate::request::FilePicker;

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(crate) fn pick(request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    let Some(offloaded) = aimer_venus::spawn_blocking(move || open(&request)) else {
        return ready(Err(PickError::Platform(
            "no Venus runtime is installed on this thread to run the portal call on".into(),
        )));
    };
    Box::pin(async move {
        let paths = offloaded.await?;
        Ok(paths.into_iter().map(PickedFile::from_path).collect())
    })
}

fn bus(error: zbus::Error) -> PickError {
    match &error {
        // No portal on the bus: a minimal window manager session, or a container
        // without xdg-desktop-portal.
        zbus::Error::MethodError(name, ..) if is_missing_service(name.as_str()) => PickError::Unsupported,
        zbus::Error::FDO(fdo) if matches!(**fdo, zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)) => {
            PickError::Unsupported
        }
        _ => PickError::Platform(error.to_string()),
    }
}

fn is_missing_service(name: &str) -> bool {
    name.ends_with("ServiceUnknown") || name.ends_with("NameHasNoOwner")
}

/// Asks the portal and waits for the answer; empty if the user dismissed it.
fn open(request: &FilePicker) -> PickResult<Vec<PathBuf>> {
    let connection = Connection::session().map_err(bus)?;
    let unique = connection
        .unique_name()
        .ok_or_else(|| PickError::Platform("the session bus gave no unique name".into()))?;
    let sender = unique.as_str().trim_start_matches(':').replace('.', "_");
    let token = format!("aimer{}", NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    let response = Proxy::new(&connection, DESTINATION, request_path.as_str(), "org.freedesktop.portal.Request")
        .map_err(bus)?;
    let mut responses = response.receive_signal("Response").map_err(bus)?;

    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("multiple", Value::from(request.multiple));
    options.insert("modal", Value::from(true));
    let filters = filters(&request.filters);
    if !filters.is_empty() {
        options.insert("filters", Value::Array(filters));
    }

    let chooser = Proxy::new(&connection, DESTINATION, PORTAL_PATH, "org.freedesktop.portal.FileChooser")
        .map_err(bus)?;
    let title = request.title.as_deref().unwrap_or("");
    let _handle: OwnedObjectPath = chooser.call("OpenFile", &("", title, options)).map_err(bus)?;

    let message = responses.next().ok_or(PickError::Interrupted)?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|error| PickError::Platform(error.to_string()))?;
    // 0 is success, 1 is "the user cancelled", 2 is "ended in some other way".
    match code {
        0 => {}
        1 => return Ok(Vec::new()),
        _ => return Err(PickError::Platform("the portal ended the dialog without a choice".into())),
    }
    let uris = results
        .get("uris")
        .and_then(|uris| <Vec<String>>::try_from(uris.try_clone().ok()?).ok())
        .unwrap_or_default();
    Ok(uris.iter().filter_map(|uri| path_from_uri(uri)).collect())
}

/// The portal's `a(sa(us))`: a name and a list of `(kind, pattern)` per filter,
/// where kind 0 is a glob and 1 a MIME type.
///
/// The nesting is built from `Value`s because a structure's fields are only
/// known at run time; the element signatures are spelled out so that an empty
/// list still has a type.
fn filters(filters: &[FileFilter]) -> Array<'static> {
    let mut list = Array::new(&signature("(sa(us))"));
    for filter in filters {
        let patterns = patterns(filter);
        if patterns.is_empty() {
            continue;
        }
        let mut globs = Array::new(&signature("(us)"));
        for (kind, pattern) in patterns {
            let Ok(pair) = StructureBuilder::new().add_field(kind).add_field(pattern).build() else {
                continue;
            };
            if globs.append(Value::Structure(pair)).is_err() {
                return Array::new(&signature("(sa(us))"));
            }
        }
        let Ok(entry) = StructureBuilder::new()
            .add_field(filter.name().to_owned())
            .add_field(Value::Array(globs))
            .build()
        else {
            continue;
        };
        let _ = list.append(Value::Structure(entry));
    }
    list
}

/// A signature from a literal that is valid by inspection (and by the test
/// below); `v` is only the unreachable fallback.
fn signature(text: &str) -> Signature {
    Signature::from_bytes(text.as_bytes()).unwrap_or(Signature::Variant)
}

/// The patterns of one filter. Globs are case-sensitive, so each extension is
/// offered in lower and upper case (cameras write `.JPG`); exact MIME types are
/// added as they are, and wildcards are left to the extensions.
fn patterns(filter: &FileFilter) -> Vec<(u32, String)> {
    let mut patterns = Vec::new();
    for extension in filter.resolved_extensions() {
        patterns.push((0, format!("*.{extension}")));
        patterns.push((0, format!("*.{}", extension.to_ascii_uppercase())));
    }
    for mime in filter.mime_types() {
        if !mime.ends_with("/*") {
            patterns.push((1, mime.clone()));
        }
    }
    patterns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_become_globs_in_both_cases() {
        let patterns = patterns(&FileFilter::new("x").extension("png"));
        assert_eq!(patterns, [(0, "*.png".to_owned()), (0, "*.PNG".to_owned())]);
    }

    #[test]
    fn exact_mime_types_are_kept_and_wildcards_are_not() {
        let filter = FileFilter::new("x").mime_type("application/pdf").mime_type("image/*");
        let patterns = patterns(&filter);
        assert!(patterns.contains(&(1, "application/pdf".to_owned())));
        assert!(!patterns.iter().any(|(_, pattern)| pattern.contains('*') && pattern.starts_with("image")));
    }

    #[test]
    fn the_signature_literals_parse() {
        assert_eq!(signature("(sa(us))").to_string(), "(sa(us))");
        assert_eq!(signature("(us)").to_string(), "(us)");
    }

    #[test]
    fn a_filter_with_nothing_translatable_is_dropped() {
        assert!(filters(&[FileFilter::new("x").mime_type("image/*")]).is_empty());
    }
}
