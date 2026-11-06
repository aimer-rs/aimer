//! Content types for the Apple pickers, shared by macOS and iOS.

use objc2::rc::Retained;
use objc2_foundation::NSString;
use objc2_uniform_type_identifiers::UTType;

use crate::filter::FileFilter;

/// The `UTType` a MIME wildcard stands for. `typeWithMIMEType:` cannot express
/// `image/*`, so the families the crate documents are spelled out.
fn wildcard_identifier(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/*" => Some("public.image"),
        "video/*" => Some("public.movie"),
        "audio/*" => Some("public.audio"),
        "text/*" => Some("public.text"),
        _ => None,
    }
}

/// Every content type the filters name that the system recognizes, without
/// duplicates.
///
/// An empty result means "no restriction": the filters were empty or named
/// nothing the system knows. A filter must never silently narrow the picker to
/// nothing, so unknown names are dropped rather than failing the request.
pub(crate) fn content_types(filters: &[FileFilter]) -> Vec<Retained<UTType>> {
    let mut types: Vec<Retained<UTType>> = Vec::new();
    let mut push = |found: Option<Retained<UTType>>| {
        if let Some(found) = found
            && !types.iter().any(|known| known.identifier() == found.identifier())
        {
            types.push(found);
        }
    };
    for filter in filters {
        for mime in filter.mime_types() {
            match wildcard_identifier(mime) {
                Some(identifier) => push(UTType::typeWithIdentifier(&NSString::from_str(identifier))),
                None if mime.ends_with("/*") => {}
                None => push(UTType::typeWithMIMEType(&NSString::from_str(mime))),
            }
        }
        for extension in filter.extension_list() {
            push(UTType::typeWithFilenameExtension(&NSString::from_str(extension)));
        }
    }
    types
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identifiers(filters: &[FileFilter]) -> Vec<String> {
        content_types(filters).iter().map(|ty| ty.identifier().to_string()).collect()
    }

    #[test]
    fn no_filters_means_no_restriction() {
        assert!(content_types(&[]).is_empty());
    }

    #[test]
    fn extensions_and_wildcards_resolve_without_duplicates() {
        let ids = identifiers(&[FileFilter::new("a").extension("png").extension("pdf"), FileFilter::images()]);
        assert!(ids.contains(&"public.png".to_owned()));
        assert!(ids.contains(&"com.adobe.pdf".to_owned()));
        assert!(ids.contains(&"public.image".to_owned()));
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn unknown_wildcards_are_skipped() {
        assert!(identifiers(&[FileFilter::new("x").mime_type("model/*")]).is_empty());
    }
}
