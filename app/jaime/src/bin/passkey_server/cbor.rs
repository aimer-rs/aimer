//! The slice of CBOR (RFC 8949) a WebAuthn attestation uses.
//!
//! `attestationObject` is a CBOR map and the credential public key inside its
//! authenticator data is a COSE key, another CBOR map. That needs unsigned and
//! negative integers, byte and text strings, arrays and maps. Anything else
//! (floats, tags, indefinite lengths) is not something an authenticator emits
//! there, so it is rejected rather than half-understood.

/// Deepest nesting accepted. Real attestations are three levels deep.
const MAX_DEPTH: usize = 8;
/// Most items a single array or map may declare. The declared count is checked
/// against the bytes left, so this only bounds the allocation.
const MAX_ITEMS: usize = 256;

/// A decoded CBOR data item.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Major types 0 and 1, as one signed number.
    Int(i64),
    /// Major type 2.
    Bytes(Vec<u8>),
    /// Major type 3.
    Text(String),
    /// Major type 4.
    Array(Vec<Value>),
    /// Major type 5, in encounter order.
    Map(Vec<(Value, Value)>),
    /// Booleans and null, which attestation statements may carry and we ignore.
    Simple,
}

/// Why a buffer did not decode.
#[derive(Debug, Eq, PartialEq)]
pub enum CborError {
    /// The buffer ended inside an item.
    Truncated,
    /// A construct this reader deliberately does not support.
    Unsupported,
    /// Text that is not UTF-8.
    BadText,
}

impl Value {
    /// The value under the text key `key`, if this is a map holding one.
    pub fn get_text(&self, key: &str) -> Option<&Value> {
        let Value::Map(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find_map(|(k, v)| matches!(k, Value::Text(t) if t == key).then_some(v))
    }

    /// The value under the integer key `key`, if this is a map holding one.
    pub fn get_int(&self, key: i64) -> Option<&Value> {
        let Value::Map(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find_map(|(k, v)| matches!(k, Value::Int(i) if *i == key).then_some(v))
    }

    /// The bytes of a byte string.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The number, if this is an integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
}

/// Decodes the first item of `input` and returns it with the bytes it used.
///
/// The length matters: a credential's COSE key sits in the middle of the
/// authenticator data, followed by extensions that are not part of it.
pub fn decode(input: &[u8]) -> Result<(Value, usize), CborError> {
    let mut reader = Reader { input, pos: 0 };
    let value = reader.item(0)?;
    Ok((value, reader.pos))
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], CborError> {
        let end = self.pos.checked_add(len).ok_or(CborError::Truncated)?;
        let slice = self.input.get(self.pos..end).ok_or(CborError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    /// The argument that follows an initial byte. The indefinite-length marker
    /// (additional info 31) is not supported.
    fn argument(&mut self, info: u8) -> Result<u64, CborError> {
        Ok(match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.take(1)?[0]),
            25 => u64::from(u16::from_be_bytes(self.take(2)?.try_into().unwrap())),
            26 => u64::from(u32::from_be_bytes(self.take(4)?.try_into().unwrap())),
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap()),
            _ => return Err(CborError::Unsupported),
        })
    }

    fn length(&mut self, info: u8) -> Result<usize, CborError> {
        usize::try_from(self.argument(info)?).map_err(|_| CborError::Unsupported)
    }

    fn item(&mut self, depth: usize) -> Result<Value, CborError> {
        if depth > MAX_DEPTH {
            return Err(CborError::Unsupported);
        }
        let initial = self.take(1)?[0];
        let (major, info) = (initial >> 5, initial & 0x1f);
        match major {
            0 => Ok(Value::Int(
                i64::try_from(self.argument(info)?).map_err(|_| CborError::Unsupported)?,
            )),
            1 => {
                let n = i64::try_from(self.argument(info)?).map_err(|_| CborError::Unsupported)?;
                Ok(Value::Int(-1 - n))
            }
            2 => {
                let len = self.length(info)?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = self.length(info)?;
                let text = std::str::from_utf8(self.take(len)?).map_err(|_| CborError::BadText)?;
                Ok(Value::Text(text.to_owned()))
            }
            4 => {
                let count = self.count(info)?;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.item(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let count = self.count(info)?;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let key = self.item(depth + 1)?;
                    entries.push((key, self.item(depth + 1)?));
                }
                Ok(Value::Map(entries))
            }
            // 7 with info 20..=22 is false, true, null; everything else in
            // major 6 (tags) and 7 (floats, break) is not used here.
            7 if (20..=22).contains(&info) => Ok(Value::Simple),
            _ => Err(CborError::Unsupported),
        }
    }

    /// An element count, checked against what is left: each element takes at
    /// least one byte, so a larger claim is a lie and must not size an
    /// allocation.
    fn count(&mut self, info: u8) -> Result<usize, CborError> {
        let count = self.length(info)?;
        if count > MAX_ITEMS || count > self.input.len() - self.pos {
            return Err(CborError::Truncated);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_rfc_8949_appendix_a_examples() {
        assert_eq!(decode(&[0x18, 0x64]).unwrap().0, Value::Int(100));
        assert_eq!(decode(&[0x20]).unwrap().0, Value::Int(-1));
        assert_eq!(decode(&[0x38, 0x63]).unwrap().0, Value::Int(-100));
        assert_eq!(
            decode(&[0x44, 1, 2, 3, 4]).unwrap().0,
            Value::Bytes(vec![1, 2, 3, 4])
        );
        assert_eq!(decode(b"\x64IETF").unwrap().0, Value::Text("IETF".into()));
        assert_eq!(
            decode(&[0x83, 1, 2, 3]).unwrap().0,
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
        assert_eq!(decode(&[0xf5]).unwrap().0, Value::Simple);
    }

    #[test]
    fn reports_how_many_bytes_the_first_item_used() {
        // {1: 2} followed by trailing extension bytes.
        let (value, used) = decode(&[0xa1, 0x01, 0x02, 0xff, 0xff]).unwrap();
        assert_eq!(used, 3);
        assert_eq!(value.get_int(1).and_then(Value::as_int), Some(2));
    }

    #[test]
    fn looks_values_up_by_text_and_integer_key() {
        let (map, _) = decode(b"\xa2\x63fmt\x64none\x20\x41\x07").unwrap();
        assert_eq!(map.get_text("fmt"), Some(&Value::Text("none".into())));
        assert_eq!(map.get_int(-1).and_then(Value::as_bytes), Some(&[7u8][..]));
        assert_eq!(map.get_text("absent"), None);
    }

    #[test]
    fn rejects_truncation_and_unsupported_constructs() {
        assert_eq!(decode(&[]), Err(CborError::Truncated));
        assert_eq!(decode(&[0x44, 1, 2]), Err(CborError::Truncated));
        // Indefinite-length array, a tag, and a float.
        assert_eq!(decode(&[0x9f, 0xff]), Err(CborError::Unsupported));
        assert_eq!(decode(&[0xc1, 0x00]), Err(CborError::Unsupported));
        assert_eq!(decode(&[0xf9, 0x3c, 0x00]), Err(CborError::Unsupported));
        assert_eq!(decode(&[0x62, 0xff, 0xfe]), Err(CborError::BadText));
    }

    #[test]
    fn a_huge_declared_count_does_not_allocate_or_loop() {
        // An array claiming 2^32 elements with nothing behind it.
        assert_eq!(
            decode(&[0x9a, 0xff, 0xff, 0xff, 0xff]),
            Err(CborError::Truncated)
        );
    }

    #[test]
    fn deeply_nested_input_is_refused_not_recursed_into() {
        let input = vec![0x81; 64];
        assert_eq!(decode(&input), Err(CborError::Unsupported));
    }
}
