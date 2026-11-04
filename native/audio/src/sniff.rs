//! Recognising an encoded audio format from its first bytes, for players that
//! need a content type with in-memory data (Windows' `MediaSource`).

/// The MIME type of encoded audio, from its magic bytes. `None` when unknown;
/// the player then sniffs for itself.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn audio_mime(bytes: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        Some("audio/wav")
    } else if starts(b"ID3") || (bytes.len() > 1 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0) {
        // ADTS AAC shares the frame-sync bits with MPEG audio; its layer bits
        // are zero, which MPEG audio never uses.
        if bytes.len() > 1 && bytes[0] == 0xFF && bytes[1] & 0x06 == 0 {
            Some("audio/aac")
        } else {
            Some("audio/mpeg")
        }
    } else if starts(b"OggS") {
        Some("audio/ogg")
    } else if starts(b"fLaC") {
        Some("audio/flac")
    } else if bytes.get(4..8) == Some(b"ftyp") {
        Some("audio/mp4")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::audio_mime;

    #[test]
    fn common_formats_are_recognised_by_their_magic_bytes() {
        assert_eq!(audio_mime(b"RIFF\0\0\0\0WAVEfmt "), Some("audio/wav"));
        assert_eq!(audio_mime(b"ID3\x04\0"), Some("audio/mpeg"));
        assert_eq!(audio_mime(&[0xFF, 0xFB, 0x90]), Some("audio/mpeg"));
        assert_eq!(audio_mime(&[0xFF, 0xF1, 0x50]), Some("audio/aac"));
        assert_eq!(audio_mime(b"OggS\0\x02"), Some("audio/ogg"));
        assert_eq!(audio_mime(b"fLaC\0\0\0\x22"), Some("audio/flac"));
        assert_eq!(audio_mime(b"\0\0\0\x20ftypM4A "), Some("audio/mp4"));
    }

    #[test]
    fn unknown_short_or_empty_data_has_no_type() {
        assert_eq!(audio_mime(b""), None);
        assert_eq!(audio_mime(&[0xFF]), None);
        assert_eq!(audio_mime(b"RIFF\0\0\0\0AVI "), None);
        assert_eq!(audio_mime(b"<html>"), None);
    }
}
