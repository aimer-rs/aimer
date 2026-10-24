//! A small, self-describing-free binary encoding for values an app persists.
//!
//! The format is owned by this crate on purpose: no serializer crate appears in
//! the public API, and the bytes are the same on every platform, so a document
//! written by the desktop build loads in the browser build.
//!
//! Integers are fixed-width little-endian, lengths are `u32`, text is UTF-8, and
//! nothing is padded. The encoding carries no field names or types, so it is
//! compact and fast to decode, and it means a type's layout *is* its format:
//! change it only together with a [document version](crate::Document::version)
//! and a migration.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

/// Why bytes could not be decoded into a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The bytes ended before the value did.
    UnexpectedEnd,
    /// Bytes were left over after the value ended.
    TrailingBytes,
    /// A `bool` that is neither 0 nor 1.
    InvalidBool(u8),
    /// An enum or `Option` tag that names no variant.
    InvalidTag(u8),
    /// Text that is not UTF-8.
    InvalidUtf8,
    /// A length that cannot fit in the bytes that remain, or in memory.
    InvalidLength,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => formatter.write_str("the data ended unexpectedly"),
            Self::TrailingBytes => formatter.write_str("unexpected data after the value"),
            Self::InvalidBool(byte) => write!(formatter, "{byte} is not a boolean"),
            Self::InvalidTag(tag) => write!(formatter, "{tag} is not a known variant"),
            Self::InvalidUtf8 => formatter.write_str("text is not valid UTF-8"),
            Self::InvalidLength => formatter.write_str("a length does not fit the data"),
        }
    }
}

impl Error for DecodeError {}

/// Writes values into a growing buffer.
#[derive(Debug, Default)]
pub struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    /// Creates an empty encoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder with room for `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Appends `value`.
    #[inline]
    pub fn value<T: StorageValue>(&mut self, value: &T) -> &mut Self {
        value.encode(self);
        self
    }

    /// Appends raw bytes with no length prefix.
    #[inline]
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Appends a collection length.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds `u32::MAX`; a value that large cannot be stored
    /// (values are bounded far below that by the storage limits).
    #[inline]
    pub fn len_prefix(&mut self, len: usize) -> &mut Self {
        let len = u32::try_from(len).expect("a persisted collection holds at most u32::MAX items");
        self.raw(&len.to_le_bytes())
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Finishes encoding and returns the bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Reads values back out of a byte slice, never past its end.
#[derive(Clone, Copy, Debug)]
pub struct Decoder<'a> {
    bytes: &'a [u8],
}

impl<'a> Decoder<'a> {
    /// Creates a decoder over `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Reads the next value.
    #[inline]
    pub fn value<T: StorageValue>(&mut self) -> Result<T, DecodeError> {
        T::decode(self)
    }

    /// Takes the next `len` raw bytes.
    #[inline]
    pub fn raw(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let (head, rest) = self
            .bytes
            .split_at_checked(len)
            .ok_or(DecodeError::UnexpectedEnd)?;
        self.bytes = rest;
        Ok(head)
    }

    #[inline]
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut array = [0; N];
        array.copy_from_slice(self.raw(N)?);
        Ok(array)
    }

    /// Reads a collection length, rejecting one that claims more items than
    /// there are bytes left, so a damaged length cannot reserve memory.
    ///
    /// Every encoded item must therefore take at least one byte.
    #[inline]
    pub fn len_prefix(&mut self) -> Result<usize, DecodeError> {
        let len = usize::try_from(u32::from_le_bytes(self.array()?))
            .map_err(|_| DecodeError::InvalidLength)?;
        if len > self.bytes.len() {
            return Err(DecodeError::InvalidLength);
        }
        Ok(len)
    }

    /// The number of bytes not yet read.
    pub fn remaining(&self) -> usize {
        self.bytes.len()
    }

    /// Succeeds only if every byte was consumed.
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

/// A value that can be persisted.
///
/// Implemented for the primitives, `String`, `Vec`, `Option`, `BTreeMap` and
/// tuples; implement it for your own types with
/// [`impl_storage_value!`](crate::impl_storage_value), or by hand with
/// [`Encoder`] and [`Decoder`].
///
/// A value must encode to at least one byte, and `decode` must read back exactly
/// the bytes `encode` wrote, so values can be concatenated.
///
/// # Examples
///
/// ```
/// use aimer_persist::{from_bytes, to_bytes};
///
/// let bytes = to_bytes(&(7u32, "seven".to_owned(), Some(true)));
/// let back: (u32, String, Option<bool>) = from_bytes(&bytes)?;
/// assert_eq!(back, (7, "seven".to_owned(), Some(true)));
/// # Ok::<(), aimer_persist::DecodeError>(())
/// ```
pub trait StorageValue: Sized {
    /// Appends this value's encoding.
    fn encode(&self, out: &mut Encoder);

    /// Reads one value, consuming exactly what [`encode`](Self::encode) wrote.
    ///
    /// # Errors
    ///
    /// A [`DecodeError`] when the bytes are not a valid encoding.
    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError>;

    /// Encodes a whole slice. The default writes each item; `u8` overrides it
    /// with one copy, so byte buffers are not encoded a byte at a time.
    #[doc(hidden)]
    fn encode_slice(items: &[Self], out: &mut Encoder) {
        out.len_prefix(items.len());
        for item in items {
            item.encode(out);
        }
    }

    /// Decodes a whole `Vec`, the counterpart of [`encode_slice`](Self::encode_slice).
    #[doc(hidden)]
    fn decode_vec(input: &mut Decoder<'_>) -> Result<Vec<Self>, DecodeError> {
        let len = input.len_prefix()?;
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(Self::decode(input)?);
        }
        Ok(items)
    }
}

/// Encodes `value` into a new buffer.
pub fn to_bytes<T: StorageValue>(value: &T) -> Vec<u8> {
    let mut encoder = Encoder::new();
    value.encode(&mut encoder);
    encoder.into_bytes()
}

/// Decodes exactly one `T` from `bytes`.
///
/// # Errors
///
/// A [`DecodeError`], including [`DecodeError::TrailingBytes`] when `bytes`
/// holds more than the one value.
pub fn from_bytes<T: StorageValue>(bytes: &[u8]) -> Result<T, DecodeError> {
    let mut decoder = Decoder::new(bytes);
    let value = T::decode(&mut decoder)?;
    decoder.finish()?;
    Ok(value)
}

macro_rules! numbers {
    ($($number:ty),* $(,)?) => {$(
        impl StorageValue for $number {
            #[inline]
            fn encode(&self, out: &mut Encoder) {
                out.raw(&self.to_le_bytes());
            }

            #[inline]
            fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                Ok(Self::from_le_bytes(input.array()?))
            }
        }
    )*};
}

numbers!(u16, u32, u64, i8, i16, i32, i64, f32, f64);

impl StorageValue for u8 {
    #[inline]
    fn encode(&self, out: &mut Encoder) {
        out.raw(&[*self]);
    }

    #[inline]
    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(input.raw(1)?[0])
    }

    fn encode_slice(items: &[Self], out: &mut Encoder) {
        out.len_prefix(items.len()).raw(items);
    }

    fn decode_vec(input: &mut Decoder<'_>) -> Result<Vec<Self>, DecodeError> {
        let len = input.len_prefix()?;
        Ok(input.raw(len)?.to_vec())
    }
}

/// `usize` and `isize` are stored as 64 bits, so a value written by a 64-bit
/// build reads back on a 32-bit one (the browser) if it fits.
impl StorageValue for usize {
    fn encode(&self, out: &mut Encoder) {
        (*self as u64).encode(out);
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        usize::try_from(u64::decode(input)?).map_err(|_| DecodeError::InvalidLength)
    }
}

impl StorageValue for isize {
    fn encode(&self, out: &mut Encoder) {
        (*self as i64).encode(out);
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        isize::try_from(i64::decode(input)?).map_err(|_| DecodeError::InvalidLength)
    }
}

impl StorageValue for bool {
    #[inline]
    fn encode(&self, out: &mut Encoder) {
        out.raw(&[u8::from(*self)]);
    }

    #[inline]
    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        match input.raw(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(DecodeError::InvalidBool(other)),
        }
    }
}

impl StorageValue for String {
    fn encode(&self, out: &mut Encoder) {
        out.len_prefix(self.len()).raw(self.as_bytes());
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let len = input.len_prefix()?;
        std::str::from_utf8(input.raw(len)?)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}

impl<T: StorageValue> StorageValue for Vec<T> {
    fn encode(&self, out: &mut Encoder) {
        T::encode_slice(self, out);
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        T::decode_vec(input)
    }
}

impl<T: StorageValue> StorageValue for Option<T> {
    fn encode(&self, out: &mut Encoder) {
        match self {
            None => {
                out.raw(&[0]);
            }
            Some(value) => {
                out.raw(&[1]).value(value);
            }
        }
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        match input.raw(1)?[0] {
            0 => Ok(None),
            1 => Ok(Some(T::decode(input)?)),
            other => Err(DecodeError::InvalidTag(other)),
        }
    }
}

impl<K: StorageValue + Ord, V: StorageValue> StorageValue for BTreeMap<K, V> {
    fn encode(&self, out: &mut Encoder) {
        out.len_prefix(self.len());
        for (key, value) in self {
            out.value(key).value(value);
        }
    }

    fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let len = input.len_prefix()?;
        let mut map = BTreeMap::new();
        for _ in 0..len {
            let key = K::decode(input)?;
            map.insert(key, V::decode(input)?);
        }
        Ok(map)
    }
}

macro_rules! tuples {
    ($(($($name:ident $index:tt),+)),+ $(,)?) => {$(
        impl<$($name: StorageValue),+> StorageValue for ($($name,)+) {
            fn encode(&self, out: &mut Encoder) {
                $(out.value(&self.$index);)+
            }

            fn decode(input: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                Ok(($($name::decode(input)?,)+))
            }
        }
    )+};
}

tuples!((A 0, B 1), (A 0, B 1, C 2), (A 0, B 1, C 2, D 3), (A 0, B 1, C 2, D 3, E 4));

/// Implements [`StorageValue`] for your own struct or fieldless enum.
///
/// For a struct, list its fields in the order they are written; that order is
/// the format, so append new fields at the end together with a document version
/// bump and a migration. For an enum, give each variant a stable number.
///
/// ```
/// use aimer_persist::{from_bytes, impl_storage_value, to_bytes};
///
/// #[derive(Debug, PartialEq)]
/// enum Theme {
///     Light,
///     Dark,
/// }
/// impl_storage_value!(enum Theme { Light = 0, Dark = 1 });
///
/// #[derive(Debug, PartialEq)]
/// struct Settings {
///     theme: Theme,
///     volume: f32,
///     recent: Vec<String>,
/// }
/// impl_storage_value!(Settings { theme, volume, recent });
///
/// let settings = Settings { theme: Theme::Dark, volume: 0.5, recent: vec!["a".into()] };
/// let back: Settings = from_bytes(&to_bytes(&settings))?;
/// assert_eq!(back, settings);
/// # Ok::<(), aimer_persist::DecodeError>(())
/// ```
#[macro_export]
macro_rules! impl_storage_value {
    (enum $name:ident { $($variant:ident = $tag:literal),+ $(,)? }) => {
        impl $crate::StorageValue for $name {
            fn encode(&self, out: &mut $crate::Encoder) {
                let tag: u8 = match self {
                    $(Self::$variant => $tag,)+
                };
                out.value(&tag);
            }

            fn decode(input: &mut $crate::Decoder<'_>) -> Result<Self, $crate::DecodeError> {
                match input.value::<u8>()? {
                    $($tag => Ok(Self::$variant),)+
                    other => Err($crate::DecodeError::InvalidTag(other)),
                }
            }
        }
    };
    ($name:ident { $($field:ident),+ $(,)? }) => {
        impl $crate::StorageValue for $name {
            fn encode(&self, out: &mut $crate::Encoder) {
                $(out.value(&self.$field);)+
            }

            fn decode(input: &mut $crate::Decoder<'_>) -> Result<Self, $crate::DecodeError> {
                Ok(Self { $($field: input.value()?,)+ })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: StorageValue + PartialEq + fmt::Debug>(value: T) {
        let bytes = to_bytes(&value);
        assert_eq!(from_bytes::<T>(&bytes).as_ref(), Ok(&value), "{bytes:?}");
        // Every strict prefix is an error, never a different valid value.
        for len in 0..bytes.len() {
            assert!(
                from_bytes::<T>(&bytes[..len]).is_err(),
                "prefix of {len} decoded"
            );
        }
    }

    #[test]
    fn primitives_round_trip() {
        round_trip(0u8);
        round_trip(255u8);
        round_trip(u16::MAX);
        round_trip(u32::MAX);
        round_trip(u64::MAX);
        round_trip(i8::MIN);
        round_trip(i16::MIN);
        round_trip(i32::MIN);
        round_trip(i64::MIN);
        round_trip(usize::MAX >> 1);
        round_trip(-5isize);
        round_trip(true);
        round_trip(false);
        round_trip(1.5f32);
        round_trip(-0.0f64);
        round_trip(f64::MAX);
    }

    #[test]
    fn floats_keep_their_exact_bits() {
        let nan = f32::from_bits(0x7fc0_1234);
        let decoded: f32 = from_bytes(&to_bytes(&nan)).unwrap();
        assert_eq!(decoded.to_bits(), nan.to_bits());
    }

    #[test]
    fn text_and_collections_round_trip() {
        round_trip(String::new());
        round_trip("héllo 日本語 😀".to_owned());
        round_trip(Vec::<u32>::new());
        round_trip(vec![1u32, 2, 3]);
        round_trip(vec![vec!["a".to_owned()], vec![]]);
        round_trip(vec![0u8, 1, 255]);
        round_trip(Some(7i32));
        round_trip(None::<i32>);
        round_trip(Some(Some("x".to_owned())));
        round_trip(BTreeMap::from([
            ("a".to_owned(), 1u32),
            ("b".to_owned(), 2),
        ]));
        round_trip((1u8, "two".to_owned(), 3.0f64));
        round_trip((true, 2u16, 3u32, 4u64, -5i8));
    }

    #[test]
    fn the_wire_format_is_stable() {
        // Pinned: persisted data written today must read tomorrow.
        assert_eq!(to_bytes(&0x0102_0304u32), [4, 3, 2, 1]);
        assert_eq!(to_bytes(&true), [1]);
        assert_eq!(to_bytes(&"hi".to_owned()), [2, 0, 0, 0, b'h', b'i']);
        assert_eq!(to_bytes(&vec![9u8, 8]), [2, 0, 0, 0, 9, 8]);
        assert_eq!(to_bytes(&vec![1u16]), [1, 0, 0, 0, 1, 0]);
        assert_eq!(to_bytes(&Some(5u8)), [1, 5]);
        assert_eq!(to_bytes(&None::<u8>), [0]);
        assert_eq!(to_bytes(&7usize), [7, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn invalid_bytes_are_errors() {
        assert_eq!(from_bytes::<bool>(&[2]), Err(DecodeError::InvalidBool(2)));
        assert_eq!(
            from_bytes::<Option<u8>>(&[2, 0]),
            Err(DecodeError::InvalidTag(2))
        );
        assert_eq!(
            from_bytes::<String>(&[1, 0, 0, 0, 0xff]),
            Err(DecodeError::InvalidUtf8)
        );
        assert_eq!(from_bytes::<u8>(&[1, 2]), Err(DecodeError::TrailingBytes));
        assert_eq!(from_bytes::<u32>(&[1]), Err(DecodeError::UnexpectedEnd));
    }

    #[test]
    fn a_huge_claimed_length_is_rejected_without_reserving_memory() {
        let claims_four_billion = [0xff, 0xff, 0xff, 0xff, 1, 2, 3];
        assert_eq!(
            from_bytes::<Vec<u32>>(&claims_four_billion),
            Err(DecodeError::InvalidLength)
        );
        assert_eq!(
            from_bytes::<Vec<u8>>(&claims_four_billion),
            Err(DecodeError::InvalidLength)
        );
        assert_eq!(
            from_bytes::<String>(&claims_four_billion),
            Err(DecodeError::InvalidLength)
        );
        assert_eq!(
            from_bytes::<BTreeMap<u8, u8>>(&claims_four_billion),
            Err(DecodeError::InvalidLength)
        );
    }

    #[derive(Debug, PartialEq)]
    enum Mode {
        Off,
        On,
        Auto,
    }
    impl_storage_value!(
        enum Mode {
            Off = 0,
            On = 1,
            Auto = 7,
        }
    );

    #[derive(Debug, PartialEq)]
    struct Profile {
        name: String,
        mode: Mode,
        scores: Vec<u32>,
        nickname: Option<String>,
    }
    impl_storage_value!(Profile {
        name,
        mode,
        scores,
        nickname
    });

    #[test]
    fn derived_structs_and_enums_round_trip_in_field_order() {
        let profile = Profile {
            name: "Ada".into(),
            mode: Mode::Auto,
            scores: vec![1, 2],
            nickname: None,
        };
        round_trip(profile);
        assert_eq!(to_bytes(&Mode::Auto), [7]);
        assert_eq!(from_bytes::<Mode>(&[1]), Ok(Mode::On));
        assert_eq!(from_bytes::<Mode>(&[2]), Err(DecodeError::InvalidTag(2)));
    }
}
