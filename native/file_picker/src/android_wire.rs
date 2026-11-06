//! What crosses the JNI boundary on Android, as plain Rust.
//!
//! Kept free of JNI so it is built, and tested, on the host.

use crate::filter::FileFilter;
use crate::mime::mime_for_extension;

/// Separates the fields of one picked file in the bridge's reply.
const FIELD: char = '\u{1f}';
/// Separates picked files in the bridge's reply.
const RECORD: char = '\u{1e}';

/// What the bridge is asked to show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Query {
    /// Use the system photo picker (`ACTION_PICK_IMAGES`) rather than the
    /// document browser. Only when the filters are nothing but whole image and
    /// video families, because the photo picker cannot filter any finer.
    pub media: bool,
    /// MIME types and wildcards; empty means any file.
    pub mime_types: Vec<String>,
}

/// Translates filters into what Android's intents understand.
pub(crate) fn query(filters: &[FileFilter]) -> Query {
    let any = Query {
        media: false,
        mime_types: Vec::new(),
    };

    let mut mime_types: Vec<String> = Vec::new();
    for filter in filters {
        // Android has no extension filter, only MIME types. An extension the
        // table cannot translate would silently hide files the user asked for, so
        // such a request offers everything instead.
        if filter
            .extension_list()
            .iter()
            .any(|extension| mime_for_extension(extension).is_none())
        {
            return any;
        }
        for mime in filter.resolved_mime_types() {
            if !mime_types.contains(&mime) {
                mime_types.push(mime);
            }
        }
    }

    // A concrete type that a wildcard in the set already covers adds nothing.
    let families: Vec<String> = mime_types
        .iter()
        .filter_map(|mime| mime.strip_suffix('*').map(str::to_owned))
        .collect();
    mime_types.retain(|mime| mime.ends_with('*') || !families.iter().any(|family| mime.starts_with(family)));

    if mime_types.is_empty() {
        return any;
    }
    let media = mime_types.iter().all(|mime| mime == "image/*" || mime == "video/*");
    Query { media, mime_types }
}

/// One file as the bridge reported it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub size: Option<u64>,
}

/// Parses the bridge's reply. A malformed record is skipped; a missing size is
/// reported as `-1`.
pub(crate) fn parse_entries(payload: &str) -> Vec<Entry> {
    payload
        .split(RECORD)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let mut fields = record.split(FIELD);
            let uri = fields.next().filter(|uri| !uri.is_empty())?;
            let name = fields.next()?;
            let mime_type = fields.next()?;
            let size = fields.next()?;
            Some(Entry {
                uri: uri.to_owned(),
                name: name.to_owned(),
                mime_type: (!mime_type.is_empty()).then(|| mime_type.to_owned()),
                size: size.parse::<i64>().ok().and_then(|size| u64::try_from(size).ok()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filters_offer_everything() {
        assert_eq!(query(&[]), Query { media: false, mime_types: vec![] });
    }

    #[test]
    fn images_alone_use_the_photo_picker() {
        let query = query(&[FileFilter::images()]);
        assert!(query.media);
        assert_eq!(query.mime_types, ["image/*"]);
    }

    #[test]
    fn images_and_videos_use_the_photo_picker_for_both() {
        let query = query(&[FileFilter::images(), FileFilter::videos()]);
        assert!(query.media);
        assert_eq!(query.mime_types, ["image/*", "video/*"]);
    }

    #[test]
    fn specific_image_types_need_the_document_browser() {
        let query = query(&[FileFilter::new("x").extension("png").extension("jpg")]);
        assert!(!query.media);
        assert_eq!(query.mime_types, ["image/png", "image/jpeg"]);
    }

    #[test]
    fn documents_alongside_images_are_not_media() {
        let query = query(&[FileFilter::images(), FileFilter::new("pdf").extension("pdf")]);
        assert!(!query.media);
        assert_eq!(query.mime_types, ["image/*", "application/pdf"]);
    }

    #[test]
    fn an_untranslatable_extension_offers_everything() {
        let query = query(&[FileFilter::new("x").extension("png").extension("fbx")]);
        assert_eq!(query, Query { media: false, mime_types: vec![] });
    }

    #[test]
    fn replies_are_parsed_into_entries() {
        let payload = "content://a/1\u{1f}one.png\u{1f}image/png\u{1f}12\u{1e}content://a/2\u{1f}two\u{1f}\u{1f}-1";
        assert_eq!(
            parse_entries(payload),
            [
                Entry {
                    uri: "content://a/1".into(),
                    name: "one.png".into(),
                    mime_type: Some("image/png".into()),
                    size: Some(12),
                },
                Entry {
                    uri: "content://a/2".into(),
                    name: "two".into(),
                    mime_type: None,
                    size: None,
                },
            ]
        );
    }

    #[test]
    fn an_empty_or_malformed_reply_yields_nothing() {
        assert!(parse_entries("").is_empty());
        assert!(parse_entries("only-a-uri").is_empty());
        assert!(parse_entries("\u{1e}\u{1e}").is_empty());
    }
}
