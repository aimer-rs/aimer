use std::error::Error;
use std::fmt;

use crate::{EventId, PropertyId, StableId128, Version, WidgetSchemaId};

/// Resource ceilings shared by portable Widget IR, event, and state documents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelLimits {
    pub(crate) max_document_bytes: u32,
    pub(crate) max_collection_entries: u32,
    pub(crate) max_string_bytes: u32,
    pub(crate) max_blob_bytes: u32,
    pub(crate) max_widget_depth: u32,
}

impl ModelLimits {
    /// Creates explicit document, collection, string, and opaque-byte ceilings.
    #[inline]
    pub const fn new(
        max_document_bytes: u32,
        max_collection_entries: u32,
        max_string_bytes: u32,
        max_blob_bytes: u32,
    ) -> Self {
        Self {
            max_document_bytes,
            max_collection_entries,
            max_string_bytes,
            max_blob_bytes,
            max_widget_depth: max_collection_entries,
        }
    }

    /// Sets the maximum number of nodes on a root-to-leaf Widget IR path.
    ///
    /// The default is the collection-entry limit supplied to [`Self::new`]. A
    /// host can choose a lower ceiling to bound native materialization depth
    /// independently from the total number of nodes in a snapshot.
    #[inline]
    pub const fn max_widget_depth(mut self, max_widget_depth: u32) -> Self {
        self.max_widget_depth = max_widget_depth;
        self
    }
}

/// Failure while encoding or validating a portable application model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelError {
    /// The input does not carry the model's required four-byte magic value.
    InvalidMagic,
    /// The fixed model format version is not supported.
    UnsupportedVersion { version: Version },
    /// A fixed header or section ends before its declared extent.
    Truncated { needed: usize, actual: usize },
    /// The encoded total length disagrees with the provided byte slice.
    LengthMismatch { declared: usize, actual: usize },
    /// Checked section-size arithmetic overflowed.
    LengthOverflow,
    /// The complete immutable snapshot exceeds its configured byte ceiling.
    DocumentTooLarge { length: usize, limit: u32 },
    /// One fixed section exceeds the shared collection-entry ceiling.
    CollectionTooLarge { count: usize, limit: u32 },
    /// One UTF-8 string exceeds its configured byte ceiling.
    StringTooLarge { length: usize, limit: u32 },
    /// One opaque payload exceeds its configured byte ceiling.
    BlobTooLarge { length: usize, limit: u32 },
    /// A string-table entry is not valid UTF-8.
    InvalidUtf8,
    /// The root node index does not name a node in the snapshot.
    RootOutOfBounds { root: u32, node_count: u32 },
    /// A child index does not name a node in the snapshot.
    NodeIndexOutOfBounds { index: u32, node_count: u32 },
    /// Two widget records claim the same retained native identity.
    DuplicateWidgetKey { key: StableId128 },
    /// One parent lists the same child node more than once.
    DuplicateWidgetChild { parent: u32, child: u32 },
    /// One node is owned by two different parents.
    MultipleWidgetParents {
        node: u32,
        first_parent: u32,
        second_parent: u32,
    },
    /// Child relationships form a cycle instead of a rooted tree.
    WidgetCycle { node: u32 },
    /// A node is not reachable from the declared root.
    UnreachableWidgetNode { node: u32 },
    /// A root-to-leaf path exceeds the configured materialization depth.
    WidgetDepthExceeded { depth: u32, limit: u32 },
    /// The permanent host cannot materialize a node's widget schema.
    UnsupportedWidgetSchema {
        node: u32,
        widget_type: WidgetSchemaId,
        schema: Version,
    },
    /// A required property is not declared by the selected widget schema.
    UnsupportedWidgetProperty {
        node: u32,
        widget_type: WidgetSchemaId,
        property_id: PropertyId,
    },
    /// A required property declared by the widget schema is absent.
    MissingWidgetProperty {
        node: u32,
        widget_type: WidgetSchemaId,
        property_id: PropertyId,
    },
    /// A property declared by the widget schema has the wrong wire type.
    InvalidWidgetPropertyType {
        node: u32,
        widget_type: WidgetSchemaId,
        property_id: PropertyId,
    },
    /// A correctly typed property violates the widget schema's value domain.
    InvalidWidgetPropertyValue {
        node: u32,
        widget_type: WidgetSchemaId,
        property_id: PropertyId,
    },
    /// Two property records in one widget node use the same stable identity.
    DuplicateWidgetProperty {
        node: u32,
        widget_type: WidgetSchemaId,
        property_id: PropertyId,
    },
    /// A widget has an invalid number of structural children.
    InvalidWidgetChildCount {
        node: u32,
        widget_type: WidgetSchemaId,
        count: u32,
        minimum: u32,
        maximum: u32,
    },
    /// A callback binding is not declared by the selected widget schema.
    UnsupportedWidgetCallback {
        node: u32,
        widget_type: WidgetSchemaId,
        event_kind: EventId,
    },
    /// A callback advertises an async contract the selected host schema does
    /// not provide or whose version is incompatible.
    UnsupportedAsyncCallback {
        node: u32,
        widget_type: WidgetSchemaId,
        event_kind: EventId,
        version: Version,
    },
    /// A widget has too many bindings for one schema event slot.
    InvalidWidgetCallbackCount {
        node: u32,
        widget_type: WidgetSchemaId,
        count: u32,
        maximum: u32,
    },
    /// Two callback records in one widget node use the same stable callback identity.
    DuplicateWidgetCallback {
        node: u32,
        widget_type: WidgetSchemaId,
        callback_id: StableId128,
    },
    /// A fixed record contains nonzero reserved bits or bytes.
    NonCanonicalReserved,
    /// A fixed record references bytes outside its owning section.
    SectionRangeOutOfBounds,
    /// Canonical ranges overlap, contain gaps, or do not cover their section.
    NonCanonicalSectionLayout,
    /// A typed property references a missing string or blob table entry.
    PropertyReferenceOutOfBounds { index: u32, count: u32 },
    /// A floating-point property is not finite.
    NonFiniteFloat,
    /// A fixed property record uses an unknown kind or non-canonical value.
    InvalidProperty,
    /// Two state entries claim the same persistent identity.
    DuplicateStateId { state_id: StableId128 },
    /// State entries are not strictly ordered by their stable identities.
    NonCanonicalStateOrder,
    /// A fixed state record uses an unknown persistence policy value.
    InvalidStatePolicy { value: u8 },
    /// A manifest's minimum core ABI is newer than its maximum ABI.
    InvalidAbiRange,
    /// Two capability requirements claim the same stable identity.
    DuplicateCapabilityId { capability_id: StableId128 },
    /// Capability requirements are not strictly ordered by stable identity.
    NonCanonicalCapabilityOrder,
    /// A capability record uses an unknown requirement policy value.
    InvalidCapabilityPolicy { value: u8 },
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("portable model magic is invalid"),
            Self::UnsupportedVersion { version } => write!(
                formatter,
                "portable model version {}.{} is unsupported",
                version.major(),
                version.minor()
            ),
            Self::Truncated { needed, actual } => {
                write!(
                    formatter,
                    "portable model needs {needed} bytes but has {actual}"
                )
            }
            Self::LengthMismatch { declared, actual } => write!(
                formatter,
                "portable model declares {declared} bytes but has {actual}"
            ),
            Self::LengthOverflow => {
                formatter.write_str("portable model length arithmetic overflowed")
            }
            Self::DocumentTooLarge { length, limit } => {
                write!(
                    formatter,
                    "portable model length {length} exceeds limit {limit}"
                )
            }
            Self::CollectionTooLarge { count, limit } => {
                write!(
                    formatter,
                    "portable collection count {count} exceeds limit {limit}"
                )
            }
            Self::StringTooLarge { length, limit } => {
                write!(
                    formatter,
                    "portable string length {length} exceeds limit {limit}"
                )
            }
            Self::BlobTooLarge { length, limit } => {
                write!(
                    formatter,
                    "portable blob length {length} exceeds limit {limit}"
                )
            }
            Self::InvalidUtf8 => formatter.write_str("portable string is not valid UTF-8"),
            Self::RootOutOfBounds { root, node_count } => write!(
                formatter,
                "root node {root} is outside node count {node_count}"
            ),
            Self::NodeIndexOutOfBounds { index, node_count } => write!(
                formatter,
                "child node {index} is outside node count {node_count}"
            ),
            Self::DuplicateWidgetKey { key } => {
                write!(formatter, "duplicate widget key {:02x?}", key.as_bytes())
            }
            Self::DuplicateWidgetChild { parent, child } => {
                write!(formatter, "widget node {parent} repeats child {child}")
            }
            Self::MultipleWidgetParents {
                node,
                first_parent,
                second_parent,
            } => write!(
                formatter,
                "widget node {node} is owned by parents {first_parent} and {second_parent}"
            ),
            Self::WidgetCycle { node } => {
                write!(
                    formatter,
                    "widget graph contains a cycle through node {node}"
                )
            }
            Self::UnreachableWidgetNode { node } => {
                write!(formatter, "widget node {node} is unreachable from the root")
            }
            Self::WidgetDepthExceeded { depth, limit } => {
                write!(
                    formatter,
                    "widget graph depth {depth} exceeds limit {limit}"
                )
            }
            Self::UnsupportedWidgetSchema {
                node,
                widget_type,
                schema,
            } => write!(
                formatter,
                "widget node {node} uses unsupported type {widget_type} schema {}.{}",
                schema.major(),
                schema.minor()
            ),
            Self::UnsupportedWidgetProperty {
                node,
                widget_type,
                property_id,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} uses unsupported required property {property_id}"
            ),
            Self::MissingWidgetProperty {
                node,
                widget_type,
                property_id,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} is missing required property {property_id}"
            ),
            Self::InvalidWidgetPropertyType {
                node,
                widget_type,
                property_id,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} property {property_id} has an incompatible wire type"
            ),
            Self::InvalidWidgetPropertyValue {
                node,
                widget_type,
                property_id,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} property {property_id} has an invalid value"
            ),
            Self::DuplicateWidgetProperty {
                node,
                widget_type,
                property_id,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} repeats property {property_id}"
            ),
            Self::InvalidWidgetChildCount {
                node,
                widget_type,
                count,
                minimum,
                maximum,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} has {count} children; expected {minimum}..={maximum}"
            ),
            Self::UnsupportedWidgetCallback {
                node,
                widget_type,
                event_kind,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} uses unsupported callback event {event_kind}"
            ),
            Self::UnsupportedAsyncCallback {
                node,
                widget_type,
                event_kind,
                version,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} uses unsupported async callback event {event_kind} version {}.{}",
                version.major(),
                version.minor(),
            ),
            Self::InvalidWidgetCallbackCount {
                node,
                widget_type,
                count,
                maximum,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} has {count} callback bindings; expected at most {maximum}"
            ),
            Self::DuplicateWidgetCallback {
                node,
                widget_type,
                callback_id,
            } => write!(
                formatter,
                "widget node {node} type {widget_type} repeats callback ID {:02x?}",
                callback_id.as_bytes()
            ),
            Self::NonCanonicalReserved => {
                formatter.write_str("portable model reserved bytes must be zero")
            }
            Self::SectionRangeOutOfBounds => {
                formatter.write_str("portable model section range is out of bounds")
            }
            Self::NonCanonicalSectionLayout => {
                formatter.write_str("portable model section ranges are not canonical")
            }
            Self::PropertyReferenceOutOfBounds { index, count } => write!(
                formatter,
                "property reference {index} is outside table count {count}"
            ),
            Self::NonFiniteFloat => {
                formatter.write_str("portable floating-point properties must be finite")
            }
            Self::InvalidProperty => formatter.write_str("portable property record is invalid"),
            Self::DuplicateStateId { state_id } => {
                write!(formatter, "duplicate state ID {:02x?}", state_id.as_bytes())
            }
            Self::NonCanonicalStateOrder => {
                formatter.write_str("state entries are not in canonical stable-ID order")
            }
            Self::InvalidStatePolicy { value } => {
                write!(formatter, "state policy value {value} is invalid")
            }
            Self::InvalidAbiRange => {
                formatter.write_str("manifest minimum ABI exceeds maximum ABI")
            }
            Self::DuplicateCapabilityId { capability_id } => write!(
                formatter,
                "duplicate capability ID {:02x?}",
                capability_id.as_bytes()
            ),
            Self::NonCanonicalCapabilityOrder => {
                formatter.write_str("capabilities are not in canonical stable-ID order")
            }
            Self::InvalidCapabilityPolicy { value } => {
                write!(formatter, "capability policy value {value} is invalid")
            }
        }
    }
}

impl Error for ModelError {}
