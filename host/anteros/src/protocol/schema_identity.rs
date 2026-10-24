//! Stable identities and metadata validation for portable widget schemas.

use crate::Version;
use core::fmt;
use std::error::Error;

const FNV_OFFSET_BASIS_64: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME_64: u64 = 0x0000_0100_0000_01b3;

/// Computes the stable 64-bit identity of a canonical schema name.
///
/// The hash uses FNV-1a with the standard 64-bit offset basis and prime. Its
/// result is independent of the target and Rust release, making it suitable
/// for identities persisted in AWIR documents. Canonical names are hashed as
/// their UTF-8 bytes.
#[inline]
pub const fn stable_schema_hash64(canonical_name: &str) -> u64 {
    let bytes = canonical_name.as_bytes();
    let mut hash = FNV_OFFSET_BASIS_64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(FNV_PRIME_64);
        index += 1;
    }
    hash
}

macro_rules! schema_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Creates an identity from its stable 64-bit representation.
            #[inline]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Derives an identity from its canonical schema name.
            #[inline]
            pub const fn from_canonical_name(canonical_name: &str) -> Self {
                Self(stable_schema_hash64(canonical_name))
            }

            /// Returns the stable 64-bit representation of this identity.
            #[inline]
            pub const fn value(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            #[inline]
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:016x}", self.0)
            }
        }
    };
}

schema_id!(
    WidgetSchemaId,
    "A stable identity for a portable widget schema."
);
schema_id!(
    PropertyId,
    "A stable identity for a property within a widget schema."
);
schema_id!(
    EventId,
    "A stable identity for an event exposed by a widget schema."
);
schema_id!(
    ValueTypeId,
    "A stable identity for a value type used by a schema."
);

/// Describes the canonical identity and supported version interval of a widget schema.
///
/// Version intervals are inclusive. Multiple entries may describe disjoint
/// intervals for the same schema identity and canonical name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WidgetSchemaMetadata<'a> {
    id: WidgetSchemaId,
    canonical_name: &'a str,
    min_version: Version,
    max_version: Version,
}

impl<'a> WidgetSchemaMetadata<'a> {
    /// Creates metadata whose identity is derived from its canonical name.
    #[inline]
    pub const fn from_canonical_name(
        canonical_name: &'a str,
        min_version: Version,
        max_version: Version,
    ) -> Self {
        Self::new(
            WidgetSchemaId::from_canonical_name(canonical_name),
            canonical_name,
            min_version,
            max_version,
        )
    }

    /// Creates metadata for an inclusive widget-schema version interval.
    ///
    /// Call [`validate_widget_schema_metadata`] before accepting metadata from
    /// independently assembled schema registries.
    #[inline]
    pub const fn new(
        id: WidgetSchemaId,
        canonical_name: &'a str,
        min_version: Version,
        max_version: Version,
    ) -> Self {
        Self {
            id,
            canonical_name,
            min_version,
            max_version,
        }
    }

    /// Returns the stable widget-schema identity.
    #[inline]
    pub const fn id(self) -> WidgetSchemaId {
        self.id
    }

    /// Returns the canonical, namespace-qualified schema name.
    #[inline]
    pub const fn canonical_name(self) -> &'a str {
        self.canonical_name
    }

    /// Returns the first supported schema version.
    #[inline]
    pub const fn min_version(self) -> Version {
        self.min_version
    }

    /// Returns the last supported schema version.
    #[inline]
    pub const fn max_version(self) -> Version {
        self.max_version
    }
}

/// An inconsistency in a collection of widget-schema metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetSchemaMetadataError<'a> {
    /// The stable identity does not equal the canonical name's required hash.
    IdentityNameMismatch {
        /// The identity declared by the schema metadata.
        declared: WidgetSchemaId,
        /// The identity derived from the canonical name.
        derived: WidgetSchemaId,
        /// The canonical name whose identity did not match.
        canonical_name: &'a str,
    },
    /// A version interval ends before it begins.
    InvalidVersionRange {
        /// The schema whose version interval is invalid.
        id: WidgetSchemaId,
        /// The canonical name supplied for the schema.
        canonical_name: &'a str,
        /// The first claimed supported version.
        min_version: Version,
        /// The last claimed supported version.
        max_version: Version,
    },
    /// One stable identity was assigned to different canonical names.
    HashCollision {
        /// The colliding stable identity.
        id: WidgetSchemaId,
        /// The first canonical name using the identity.
        first: &'a str,
        /// The second canonical name using the identity.
        second: &'a str,
    },
    /// One canonical name was assigned different stable identities.
    CanonicalNameConflict {
        /// The canonical name assigned more than once.
        canonical_name: &'a str,
        /// The first identity assigned to the canonical name.
        first: WidgetSchemaId,
        /// The second identity assigned to the canonical name.
        second: WidgetSchemaId,
    },
    /// Two entries for one schema claim at least one common version.
    OverlappingVersions {
        /// The stable identity of the overlapping schema entries.
        id: WidgetSchemaId,
        /// The canonical name of the first entry.
        first: &'a str,
        /// The canonical name of the second entry.
        second: &'a str,
    },
}

impl fmt::Display for WidgetSchemaMetadataError<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdentityNameMismatch {
                declared,
                derived,
                canonical_name,
            } => write!(
                formatter,
                "widget schema {canonical_name} declares identity {declared} but its canonical identity is {derived}",
            ),
            Self::InvalidVersionRange {
                canonical_name,
                min_version,
                max_version,
                ..
            } => write!(
                formatter,
                "widget schema {canonical_name} has invalid version range {}.{} through {}.{}",
                min_version.major(),
                min_version.minor(),
                max_version.major(),
                max_version.minor(),
            ),
            Self::HashCollision { id, first, second } => write!(
                formatter,
                "widget schema identity {:#018x} is shared by {first} and {second}",
                id.value(),
            ),
            Self::CanonicalNameConflict {
                canonical_name,
                first,
                second,
            } => write!(
                formatter,
                "widget schema {canonical_name} has conflicting identities {:#018x} and {:#018x}",
                first.value(),
                second.value(),
            ),
            Self::OverlappingVersions { id, first, second } => write!(
                formatter,
                "widget schema entries {first} and {second} overlap for identity {:#018x}",
                id.value(),
            ),
        }
    }
}

impl Error for WidgetSchemaMetadataError<'_> {}

/// Validates identities, canonical names, and version intervals in a schema registry.
///
/// Validation performs no allocation. Entries with the same identity and
/// canonical name are accepted only when their inclusive version intervals are
/// disjoint.
pub fn validate_widget_schema_metadata<'a>(
    metadata: &[WidgetSchemaMetadata<'a>],
) -> Result<(), WidgetSchemaMetadataError<'a>> {
    for entry in metadata {
        if version_is_before(entry.max_version, entry.min_version) {
            return Err(WidgetSchemaMetadataError::InvalidVersionRange {
                id: entry.id,
                canonical_name: entry.canonical_name,
                min_version: entry.min_version,
                max_version: entry.max_version,
            });
        }
    }

    let mut first_index = 0;
    while first_index < metadata.len() {
        let first = metadata[first_index];
        let mut second_index = first_index + 1;
        while second_index < metadata.len() {
            let second = metadata[second_index];
            if first.id == second.id && first.canonical_name != second.canonical_name {
                return Err(WidgetSchemaMetadataError::HashCollision {
                    id: first.id,
                    first: first.canonical_name,
                    second: second.canonical_name,
                });
            }
            if first.canonical_name == second.canonical_name && first.id != second.id {
                return Err(WidgetSchemaMetadataError::CanonicalNameConflict {
                    canonical_name: first.canonical_name,
                    first: first.id,
                    second: second.id,
                });
            }
            if first.id == second.id
                && first.canonical_name == second.canonical_name
                && versions_overlap(first, second)
            {
                return Err(WidgetSchemaMetadataError::OverlappingVersions {
                    id: first.id,
                    first: first.canonical_name,
                    second: second.canonical_name,
                });
            }
            second_index += 1;
        }
        first_index += 1;
    }
    for entry in metadata {
        let derived = WidgetSchemaId::from_canonical_name(entry.canonical_name);
        if entry.id != derived {
            return Err(WidgetSchemaMetadataError::IdentityNameMismatch {
                declared: entry.id,
                derived,
                canonical_name: entry.canonical_name,
            });
        }
    }
    Ok(())
}

#[inline]
const fn version_is_before(first: Version, second: Version) -> bool {
    first.major() < second.major()
        || (first.major() == second.major() && first.minor() < second.minor())
}

#[inline]
const fn versions_overlap(
    first: WidgetSchemaMetadata<'_>,
    second: WidgetSchemaMetadata<'_>,
) -> bool {
    !version_is_before(first.max_version, second.min_version)
        && !version_is_before(second.max_version, first.min_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA_NAME: &str = "aimer.widget:test::Widget";
    const SCHEMA_ID: WidgetSchemaId = WidgetSchemaId::new(stable_schema_hash64(SCHEMA_NAME));

    #[test]
    fn fnv_one_a_matches_fixed_vectors() {
        assert_eq!(stable_schema_hash64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_schema_hash64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(stable_schema_hash64("foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn identity_must_match_the_canonical_name_hash() {
        let metadata = WidgetSchemaMetadata::new(
            WidgetSchemaId::new(7),
            SCHEMA_NAME,
            Version::new(1, 0),
            Version::new(1, 0),
        );

        assert!(matches!(
            validate_widget_schema_metadata(&[metadata]),
            Err(WidgetSchemaMetadataError::IdentityNameMismatch { .. })
        ));
    }

    #[test]
    fn invalid_version_range_is_rejected() {
        let metadata = WidgetSchemaMetadata::new(
            SCHEMA_ID,
            SCHEMA_NAME,
            Version::new(2, 0),
            Version::new(1, 9),
        );

        assert!(matches!(
            validate_widget_schema_metadata(&[metadata]),
            Err(WidgetSchemaMetadataError::InvalidVersionRange { .. })
        ));
    }

    #[test]
    fn same_id_with_different_names_is_rejected_as_a_hash_collision() {
        let first = metadata(SCHEMA_ID, "aimer.widget:test::First", 1, 0, 1, 0);
        let second = metadata(SCHEMA_ID, "aimer.widget:test::Second", 2, 0, 2, 0);

        assert!(matches!(
            validate_widget_schema_metadata(&[first, second]),
            Err(WidgetSchemaMetadataError::HashCollision { .. })
        ));
    }

    #[test]
    fn same_canonical_name_with_different_ids_is_rejected() {
        let first = metadata(SCHEMA_ID, SCHEMA_NAME, 1, 0, 1, 0);
        let second = metadata(WidgetSchemaId::new(8), SCHEMA_NAME, 2, 0, 2, 0);

        assert!(matches!(
            validate_widget_schema_metadata(&[first, second]),
            Err(WidgetSchemaMetadataError::CanonicalNameConflict { .. })
        ));
    }

    #[test]
    fn touching_inclusive_version_ranges_overlap() {
        let first = metadata(SCHEMA_ID, SCHEMA_NAME, 1, 0, 1, 5);
        let second = metadata(SCHEMA_ID, SCHEMA_NAME, 1, 5, 2, 0);

        assert_eq!(
            validate_widget_schema_metadata(&[first, second]),
            Err(WidgetSchemaMetadataError::OverlappingVersions {
                id: SCHEMA_ID,
                first: SCHEMA_NAME,
                second: SCHEMA_NAME,
            })
        );
    }

    #[test]
    fn disjoint_version_ranges_are_allowed() {
        let first = metadata(SCHEMA_ID, SCHEMA_NAME, 1, 0, 1, 5);
        let second = metadata(SCHEMA_ID, SCHEMA_NAME, 1, 6, 2, 0);

        assert_eq!(validate_widget_schema_metadata(&[first, second]), Ok(()));
    }

    fn metadata(
        id: WidgetSchemaId,
        canonical_name: &'static str,
        min_major: u16,
        min_minor: u16,
        max_major: u16,
        max_minor: u16,
    ) -> WidgetSchemaMetadata<'static> {
        WidgetSchemaMetadata::new(
            id,
            canonical_name,
            Version::new(min_major, min_minor),
            Version::new(max_major, max_minor),
        )
    }
}
