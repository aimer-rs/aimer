//! Unpadded base64url (RFC 4648 §5), the encoding WebAuthn uses for every byte
//! string that crosses a JSON boundary.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes `bytes` without padding.
#[cfg_attr(not(any(windows, target_os = "android")), allow(dead_code))]
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = match *chunk {
            [a, b, c] => u32::from(a) << 16 | u32::from(b) << 8 | u32::from(c),
            [a, b] => u32::from(a) << 16 | u32::from(b) << 8,
            [a] => u32::from(a) << 16,
            _ => unreachable!("chunks(3) yields one to three bytes"),
        };
        // A chunk of `k` bytes carries `k + 1` sextets; the rest would be padding.
        for shift in [18, 12, 6, 0].into_iter().take(chunk.len() + 1) {
            out.push(char::from(ALPHABET[(n >> shift & 0x3f) as usize]));
        }
    }
    out
}

/// Decodes unpadded base64url. Padding, whitespace, the standard alphabet's
/// `+` and `/`, and non-canonical trailing bits are all rejected, so one byte
/// string has exactly one encoding.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 2);
    for chunk in text.chunks(4) {
        let mut n = 0_u32;
        for &byte in chunk {
            n = n << 6 | sextet(byte)?;
        }
        // Left-align a short final group so its bytes sit at the top.
        n <<= 6 * (4 - chunk.len() as u32);
        let bytes = n.to_be_bytes();
        let count = chunk.len() * 6 / 8;
        out.extend_from_slice(&bytes[1..1 + count]);
        // Bits below the last whole byte must be zero.
        if chunk.len() < 4 && bytes[1 + count..].iter().any(|&b| b != 0) {
            return None;
        }
    }
    Some(out)
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn sextet(byte: u8) -> Option<u32> {
    Some(u32::from(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'-' => 62,
        b'_' => 63,
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 4648 §10, with the padding removed.
    const VECTORS: [(&[u8], &str); 7] = [
        (b"", ""),
        (b"f", "Zg"),
        (b"fo", "Zm8"),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg"),
        (b"fooba", "Zm9vYmE"),
        (b"foobar", "Zm9vYmFy"),
    ];

    #[test]
    fn matches_the_rfc_vectors_both_ways() {
        for (bytes, text) in VECTORS {
            assert_eq!(encode(bytes), text);
            assert_eq!(decode(text).as_deref(), Some(bytes));
        }
    }

    #[test]
    fn uses_the_url_alphabet() {
        assert_eq!(encode(&[0xfb, 0xff, 0xfe]), "-__-");
        assert_eq!(decode("-__-"), Some(vec![0xfb, 0xff, 0xfe]));
    }

    #[test]
    fn every_byte_value_round_trips() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&bytes)), Some(bytes));
    }

    #[test]
    fn rejects_what_is_not_canonical() {
        for bad in ["Zg==", "Zm9v\n", "Zm 9v", "Zm+/", "Z", "Zh", "Zm9vY", "Zm9vYmF=", "é"] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }
}
