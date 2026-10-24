use std::collections::{HashMap, HashSet};

use crate::{
    EventId, ModelError, ModelLimits, PropertyId, StableId128, Version, WidgetSchemaId,
    widget_schema::{
        EVENT_BUTTON_DOUBLE_PRESS, EVENT_BUTTON_LONG_PRESS, EVENT_BUTTON_PRESS,
        EVENT_BUTTON_RIGHT_PRESS,
    },
};

const MAGIC: [u8; 4] = *b"AWIR";
/// Current binary Widget IR document format emitted and accepted by Aimer.
pub const WIDGET_IR_FORMAT_VERSION: Version = Version::new(2, 0);
const FORMAT_VERSION: Version = WIDGET_IR_FORMAT_VERSION;
const HEADER_LEN: usize = 64;
const NODE_RECORD_LEN: usize = 64;
const PROPERTY_RECORD_LEN: usize = 32;
const CALLBACK_RECORD_LEN: usize = 40;
const RANGE_RECORD_LEN: usize = 8;
const KEY_PRESENT: u32 = 1;
const PROPERTY_OPTIONAL: u8 = 1;

/// Reports which portable widget schemas the permanent host can materialize.
///
/// Implementations should answer from immutable registration metadata. The
/// decoder calls this only after the complete binary image and graph topology
/// have passed validation, so a rejected schema cannot trigger widget factory
/// side effects.
pub trait WidgetSchemaSupport {
    /// Returns whether `widget_type` at `schema` is supported by this host.
    fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool;

    /// Validates host-specific properties, callbacks, and child constraints.
    ///
    /// This hook runs for every node only after portable decoding and topology
    /// validation have completed, and before any materialization factory is
    /// invoked. Implementations must be side-effect free. Unknown optional
    /// properties may be ignored; an unknown required property should return
    /// [`ModelError::UnsupportedWidgetProperty`].
    #[inline]
    fn validate_node(
        &self,
        _document: &WidgetDocumentView<'_>,
        _node_index: u32,
        _node: WidgetNodeView<'_>,
    ) -> Result<(), ModelError> {
        Ok(())
    }
}

/// A fixed-width property value understood without tagged object parsing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropertyValue {
    /// A canonical zero-or-one Boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    I64(i64),
    /// A finite IEEE-754 64-bit number.
    F64(f64),
    /// A packed red-green-blue-alpha color value.
    Rgba(u32),
    /// An index into the document's UTF-8 string table.
    StringRef(u32),
    /// An index into the document's opaque byte-string table.
    BlobRef(u32),
}

/// One typed property in a widget node's contiguous property range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetProperty {
    property_id: PropertyId,
    value: PropertyValue,
    optional: bool,
}

impl WidgetProperty {
    /// Creates a required property with a stable widget-schema field ID.
    #[inline]
    pub const fn new(property_id: PropertyId, value: PropertyValue) -> Self {
        Self {
            property_id,
            value,
            optional: false,
        }
    }

    /// Marks the property as safe for an older host to skip.
    #[inline]
    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Returns the stable field ID declared by the widget schema.
    #[inline]
    pub const fn property_id(self) -> PropertyId {
        self.property_id
    }

    /// Returns the fixed-width typed value.
    #[inline]
    pub const fn value(self) -> PropertyValue {
        self.value
    }

    /// Returns whether an older host may skip an unknown field safely.
    #[inline]
    pub const fn is_optional(self) -> bool {
        self.optional
    }
}

/// One stable callback binding in a widget node's contiguous callback range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallbackBinding {
    event_kind: EventId,
    event_schema: Version,
    async_schema: Option<Version>,
    callback_id: StableId128,
}

impl CallbackBinding {
    /// Stable event kind for a completed primary Button press.
    pub const EVENT_BUTTON_PRESS: EventId = EVENT_BUTTON_PRESS;

    /// Stable event kind for a recognized Button long press.
    pub const EVENT_BUTTON_LONG_PRESS: EventId = EVENT_BUTTON_LONG_PRESS;

    /// Stable event kind for a completed Button double press.
    pub const EVENT_BUTTON_DOUBLE_PRESS: EventId = EVENT_BUTTON_DOUBLE_PRESS;

    /// Stable event kind for a completed secondary-button press.
    pub const EVENT_BUTTON_RIGHT_PRESS: EventId = EVENT_BUTTON_RIGHT_PRESS;

    /// Creates a binding between a typed event and a stable guest callback.
    #[inline]
    pub const fn new(event_kind: EventId, event_schema: Version, callback_id: StableId128) -> Self {
        Self {
            event_kind,
            event_schema,
            async_schema: None,
            callback_id,
        }
    }

    /// Creates a binding that advertises the reflected async callback contract.
    #[inline]
    pub const fn new_async(
        event_kind: EventId,
        event_schema: Version,
        async_schema: Version,
        callback_id: StableId128,
    ) -> Self {
        Self {
            event_kind,
            event_schema,
            async_schema: Some(async_schema),
            callback_id,
        }
    }

    /// Returns the stable event kind.
    #[inline]
    pub const fn event_kind(self) -> EventId {
        self.event_kind
    }

    /// Returns the expected event payload schema.
    #[inline]
    pub const fn event_schema(self) -> Version {
        self.event_schema
    }

    /// Returns the optional async callback contract version.
    #[inline]
    pub const fn async_schema(self) -> Option<Version> {
        self.async_schema
    }

    /// Returns whether this binding may start a guest-owned async task.
    #[inline]
    pub const fn is_async(self) -> bool {
        self.async_schema.is_some()
    }

    /// Returns the stable callback identity resolved by the active generation.
    #[inline]
    pub const fn callback_id(self) -> StableId128 {
        self.callback_id
    }
}

/// One widget record in an immutable portable Widget IR snapshot.
///
/// Child relationships use document-local node indices. Stable keys are the
/// only identities that survive replacement; no pointer, trait object, closure,
/// or native layout crosses this interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetNode<'a> {
    widget_type: WidgetSchemaId,
    widget_schema: Version,
    key: Option<StableId128>,
    properties: &'a [WidgetProperty],
    callbacks: &'a [CallbackBinding],
    children: &'a [u32],
}

impl<'a> WidgetNode<'a> {
    /// Creates a leaf node without a stable key.
    #[inline]
    pub const fn new(widget_type: WidgetSchemaId, widget_schema: Version) -> Self {
        Self {
            widget_type,
            widget_schema,
            key: None,
            properties: &[],
            callbacks: &[],
            children: &[],
        }
    }

    /// Assigns the stable native-reconciliation key for this node.
    #[inline]
    pub const fn key(mut self, key: StableId128) -> Self {
        self.key = Some(key);
        self
    }

    /// Assigns document-local child indices in declaration order.
    #[inline]
    pub const fn children(mut self, children: &'a [u32]) -> Self {
        self.children = children;
        self
    }

    /// Assigns the node's contiguous typed property records.
    #[inline]
    pub const fn properties(mut self, properties: &'a [WidgetProperty]) -> Self {
        self.properties = properties;
        self
    }

    /// Assigns stable callback bindings for the node's event kinds.
    #[inline]
    pub const fn callbacks(mut self, callbacks: &'a [CallbackBinding]) -> Self {
        self.callbacks = callbacks;
        self
    }
}

/// A complete immutable Widget IR snapshot ready for canonical encoding.
pub struct WidgetDocument<'a> {
    generation_id: u64,
    document_revision: u64,
    root_node: u32,
    nodes: &'a [WidgetNode<'a>],
    strings: &'a [&'a str],
    blobs: &'a [&'a [u8]],
}

impl<'a> WidgetDocument<'a> {
    /// Creates one complete snapshot from indexed node, string, and blob tables.
    #[inline]
    pub const fn new(
        generation_id: u64,
        document_revision: u64,
        root_node: u32,
        nodes: &'a [WidgetNode<'a>],
        strings: &'a [&'a str],
        blobs: &'a [&'a [u8]],
    ) -> Self {
        Self {
            generation_id,
            document_revision,
            root_node,
            nodes,
            strings,
            blobs,
        }
    }

    /// Encodes the snapshot as fixed little-endian sections.
    ///
    /// Node records and child indices remain fixed-width. String and blob table
    /// entries contain offsets into contiguous byte sections, allowing the host
    /// to validate once and borrow the original buffer afterward.
    pub fn encode(&self, limits: ModelLimits) -> Result<Vec<u8>, ModelError> {
        self.encode_with_payloads(self.strings, self.blobs, None, None, limits)
    }

    /// Encodes the snapshot after interning equal strings and opaque payloads.
    ///
    /// Interning is deterministic: the first occurrence of each distinct value
    /// determines its compact table index. Every [`PropertyValue::StringRef`]
    /// and [`PropertyValue::BlobRef`] is rewritten to that index while node,
    /// property, callback, and child order remain unchanged. The resulting
    /// image retains the fixed-width AWIR 2.0 layout used by [`Self::encode`].
    ///
    /// Source references are checked against the original tables before any
    /// reference is rewritten. `limits` are then applied to the compacted
    /// tables and complete encoded image, allowing repeated payloads to fit a
    /// ceiling that the unmodified snapshot exceeds.
    ///
    /// When both source tables already contain only distinct values, this
    /// method produces exactly the same bytes as [`Self::encode`].
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] if the source snapshot is structurally invalid,
    /// a source property reference lies outside its original payload table, or
    /// the compacted snapshot exceeds `limits`.
    pub fn encode_compact(&self, limits: ModelLimits) -> Result<Vec<u8>, ModelError> {
        let source_string_count =
            u32::try_from(self.strings.len()).map_err(|_| ModelError::LengthOverflow)?;
        let source_blob_count =
            u32::try_from(self.blobs.len()).map_err(|_| ModelError::LengthOverflow)?;
        for node in self.nodes {
            for property in node.properties {
                validate_property_value(property.value, source_string_count, source_blob_count)?;
            }
        }

        let mut string_indices = HashMap::with_capacity(self.strings.len());
        let mut strings = Vec::with_capacity(self.strings.len());
        let mut string_remap = Vec::with_capacity(self.strings.len());
        for &value in self.strings {
            let index = if let Some(&index) = string_indices.get(value) {
                index
            } else {
                let index = u32::try_from(strings.len()).map_err(|_| ModelError::LengthOverflow)?;
                string_indices.insert(value, index);
                strings.push(value);
                index
            };
            string_remap.push(index);
        }

        let mut blob_indices = HashMap::with_capacity(self.blobs.len());
        let mut blobs = Vec::with_capacity(self.blobs.len());
        let mut blob_remap = Vec::with_capacity(self.blobs.len());
        for &value in self.blobs {
            let index = if let Some(&index) = blob_indices.get(value) {
                index
            } else {
                let index = u32::try_from(blobs.len()).map_err(|_| ModelError::LengthOverflow)?;
                blob_indices.insert(value, index);
                blobs.push(value);
                index
            };
            blob_remap.push(index);
        }

        self.encode_with_payloads(
            &strings,
            &blobs,
            Some(&string_remap),
            Some(&blob_remap),
            limits,
        )
    }

    fn encode_with_payloads(
        &self,
        strings: &[&str],
        blobs: &[&[u8]],
        string_remap: Option<&[u32]>,
        blob_remap: Option<&[u32]>,
        limits: ModelLimits,
    ) -> Result<Vec<u8>, ModelError> {
        check_count(self.nodes.len(), limits)?;
        check_count(strings.len(), limits)?;
        check_count(blobs.len(), limits)?;
        let node_count = u32::try_from(self.nodes.len()).map_err(|_| ModelError::LengthOverflow)?;
        if self.root_node >= node_count {
            return Err(ModelError::RootOutOfBounds {
                root: self.root_node,
                node_count,
            });
        }

        let mut keys = HashSet::with_capacity(self.nodes.len());
        let mut property_count = 0_usize;
        let mut callback_count = 0_usize;
        let mut child_count = 0_usize;
        for node in self.nodes {
            if let Some(key) = node.key
                && !keys.insert(key)
            {
                return Err(ModelError::DuplicateWidgetKey { key });
            }
            property_count = property_count
                .checked_add(node.properties.len())
                .ok_or(ModelError::LengthOverflow)?;
            callback_count = callback_count
                .checked_add(node.callbacks.len())
                .ok_or(ModelError::LengthOverflow)?;
            child_count = child_count
                .checked_add(node.children.len())
                .ok_or(ModelError::LengthOverflow)?;
            for &child in node.children {
                if child >= node_count {
                    return Err(ModelError::NodeIndexOutOfBounds {
                        index: child,
                        node_count,
                    });
                }
            }
            for property in node.properties {
                validate_property_value(
                    property.value,
                    self.strings.len() as u32,
                    self.blobs.len() as u32,
                )?;
            }
        }
        check_count(property_count, limits)?;
        check_count(callback_count, limits)?;
        check_count(child_count, limits)?;

        let string_bytes_len = checked_variable_bytes(strings, limits.max_string_bytes)?;
        let blob_bytes_len = checked_blob_bytes(blobs, limits.max_blob_bytes)?;
        let total_len = HEADER_LEN
            .checked_add(checked_section_len(self.nodes.len(), NODE_RECORD_LEN)?)
            .and_then(|length| {
                length.checked_add(checked_section_len(property_count, PROPERTY_RECORD_LEN).ok()?)
            })
            .and_then(|length| {
                length.checked_add(checked_section_len(callback_count, CALLBACK_RECORD_LEN).ok()?)
            })
            .and_then(|length| length.checked_add(checked_section_len(child_count, 4).ok()?))
            .and_then(|length| {
                length.checked_add(checked_section_len(strings.len(), RANGE_RECORD_LEN).ok()?)
            })
            .and_then(|length| length.checked_add(string_bytes_len))
            .and_then(|length| {
                length.checked_add(checked_section_len(blobs.len(), RANGE_RECORD_LEN).ok()?)
            })
            .and_then(|length| length.checked_add(blob_bytes_len))
            .ok_or(ModelError::LengthOverflow)?;
        if total_len > limits.max_document_bytes as usize {
            return Err(ModelError::DocumentTooLarge {
                length: total_len,
                limit: limits.max_document_bytes,
            });
        }

        let mut output = Vec::with_capacity(total_len);
        output.extend_from_slice(&MAGIC);
        write_version(&mut output, FORMAT_VERSION);
        output.extend_from_slice(&self.generation_id.to_le_bytes());
        output.extend_from_slice(&self.document_revision.to_le_bytes());
        write_u32(&mut output, self.root_node);
        write_u32(&mut output, node_count);
        write_u32(&mut output, property_count as u32);
        write_u32(&mut output, callback_count as u32);
        write_u32(&mut output, child_count as u32);
        write_u32(&mut output, strings.len() as u32);
        write_u32(&mut output, string_bytes_len as u32);
        write_u32(&mut output, blobs.len() as u32);
        write_u32(&mut output, blob_bytes_len as u32);
        write_u32(&mut output, total_len as u32);

        let mut property_start = 0_u32;
        let mut callback_start = 0_u32;
        let mut child_start = 0_u32;
        for node in self.nodes {
            write_u64(&mut output, node.widget_type.value());
            write_version(&mut output, node.widget_schema);
            write_u32(&mut output, u32::from(node.key.is_some()) * KEY_PRESENT);
            output.extend_from_slice(
                node.key
                    .unwrap_or(StableId128::from_bytes([0; 16]))
                    .as_bytes(),
            );
            write_u32(&mut output, property_start);
            write_u32(&mut output, node.properties.len() as u32);
            write_u32(&mut output, callback_start);
            write_u32(&mut output, node.callbacks.len() as u32);
            write_u32(&mut output, child_start);
            write_u32(&mut output, node.children.len() as u32);
            write_u64(&mut output, 0);
            property_start += node.properties.len() as u32;
            callback_start += node.callbacks.len() as u32;
            child_start += node.children.len() as u32;
        }
        for node in self.nodes {
            for property in node.properties {
                encode_property(
                    &mut output,
                    remap_property(*property, string_remap, blob_remap),
                );
            }
        }
        for node in self.nodes {
            for callback in node.callbacks {
                write_u64(&mut output, callback.event_kind.value());
                write_version(&mut output, callback.event_schema);
                write_version(
                    &mut output,
                    callback.async_schema.unwrap_or(Version::new(0, 0)),
                );
                write_u32(&mut output, 0);
                write_u32(&mut output, 0);
                output.extend_from_slice(callback.callback_id.as_bytes());
            }
        }
        for node in self.nodes {
            for &child in node.children {
                write_u32(&mut output, child);
            }
        }
        write_ranges(&mut output, strings.iter().map(|value| value.len()));
        for value in strings {
            output.extend_from_slice(value.as_bytes());
        }
        write_ranges(&mut output, blobs.iter().map(|value| value.len()));
        for value in blobs {
            output.extend_from_slice(value);
        }
        debug_assert_eq!(output.len(), total_len);
        Ok(output)
    }
}

/// A validated, allocation-free view over one host-owned Widget IR image.
pub struct WidgetDocumentView<'a> {
    bytes: &'a [u8],
    layout: Layout,
    generation_id: u64,
    document_revision: u64,
    root_node: u32,
}

impl<'a> WidgetDocumentView<'a> {
    /// Validates fixed sections, canonical reserved bytes, indices, and strings.
    pub fn decode(bytes: &'a [u8], limits: ModelLimits) -> Result<Self, ModelError> {
        let layout = Layout::decode(bytes, limits)?;
        let root_node = read_u32(bytes, 24);
        if root_node >= layout.node_count {
            return Err(ModelError::RootOutOfBounds {
                root: root_node,
                node_count: layout.node_count,
            });
        }
        let mut keys = HashSet::with_capacity(layout.node_count as usize);
        let mut expected_property_start = 0_u32;
        let mut expected_callback_start = 0_u32;
        let mut expected_child_start = 0_u32;
        for index in 0..layout.node_count {
            let offset = layout.nodes_start + index as usize * NODE_RECORD_LEN;
            let flags = read_u32(bytes, offset + 12);
            if flags & !KEY_PRESENT != 0 || read_u64(bytes, offset + 56) != 0 {
                return Err(ModelError::NonCanonicalReserved);
            }
            if flags == 0 && bytes[offset + 16..offset + 32] != [0; 16] {
                return Err(ModelError::NonCanonicalReserved);
            }
            if flags & KEY_PRESENT != 0 {
                let mut key = [0_u8; 16];
                key.copy_from_slice(&bytes[offset + 16..offset + 32]);
                let key = StableId128::from_bytes(key);
                if !keys.insert(key) {
                    return Err(ModelError::DuplicateWidgetKey { key });
                }
            }
            validate_canonical_range(
                read_u32(bytes, offset + 32),
                read_u32(bytes, offset + 36),
                layout.property_count,
                &mut expected_property_start,
            )?;
            validate_canonical_range(
                read_u32(bytes, offset + 40),
                read_u32(bytes, offset + 44),
                layout.callback_count,
                &mut expected_callback_start,
            )?;
            validate_canonical_range(
                read_u32(bytes, offset + 48),
                read_u32(bytes, offset + 52),
                layout.child_count,
                &mut expected_child_start,
            )?;
        }
        if expected_property_start != layout.property_count
            || expected_callback_start != layout.callback_count
            || expected_child_start != layout.child_count
        {
            return Err(ModelError::NonCanonicalSectionLayout);
        }
        for index in 0..layout.property_count {
            let offset = layout.properties_start + index as usize * PROPERTY_RECORD_LEN;
            decode_property(bytes, offset, layout.string_count, layout.blob_count)?;
        }
        for index in 0..layout.callback_count {
            let offset = layout.callbacks_start + index as usize * CALLBACK_RECORD_LEN;
            if read_u32(bytes, offset + 20) != 0 {
                return Err(ModelError::NonCanonicalReserved);
            }
        }
        for index in 0..layout.child_count {
            let child = read_u32(bytes, layout.children_start + index as usize * 4);
            if child >= layout.node_count {
                return Err(ModelError::NodeIndexOutOfBounds {
                    index: child,
                    node_count: layout.node_count,
                });
            }
        }
        validate_graph(bytes, layout, root_node, limits.max_widget_depth)?;
        validate_strings(bytes, layout, limits)?;
        validate_blob_ranges(bytes, layout, limits)?;
        Ok(Self {
            bytes,
            layout,
            generation_id: read_u64(bytes, 8),
            document_revision: read_u64(bytes, 16),
            root_node,
        })
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub(crate) const fn into_validated(self) -> ValidatedWidgetDocument {
        ValidatedWidgetDocument {
            layout: self.layout,
            generation_id: self.generation_id,
            document_revision: self.document_revision,
            root_node: self.root_node,
        }
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub(crate) const fn from_validated(
        bytes: &'a [u8],
        validated: ValidatedWidgetDocument,
    ) -> Self {
        Self {
            bytes,
            layout: validated.layout,
            generation_id: validated.generation_id,
            document_revision: validated.document_revision,
            root_node: validated.root_node,
        }
    }

    /// Returns the module generation that produced this snapshot.
    #[inline]
    pub const fn generation_id(&self) -> u64 {
        self.generation_id
    }

    /// Returns the monotonic revision within the producing generation.
    #[inline]
    pub const fn document_revision(&self) -> u64 {
        self.document_revision
    }

    /// Returns the document-local root node index.
    #[inline]
    pub const fn root_node(&self) -> u32 {
        self.root_node
    }

    /// Returns the number of fixed-width node records.
    #[inline]
    pub const fn node_count(&self) -> u32 {
        self.layout.node_count
    }

    /// Returns the number of document-local UTF-8 string records.
    #[inline]
    pub const fn string_count(&self) -> u32 {
        self.layout.string_count
    }

    /// Returns the number of document-local binary blob records.
    #[inline]
    pub const fn blob_count(&self) -> u32 {
        self.layout.blob_count
    }

    /// Verifies that every node uses a widget schema supported by the host.
    ///
    /// Structural decoding always completes first. Callers can therefore use a
    /// registry that owns native factories without allowing malformed topology
    /// to invoke those factories or otherwise mutate host state.
    pub fn validate_schemas(&self, support: &impl WidgetSchemaSupport) -> Result<(), ModelError> {
        for node_index in 0..self.layout.node_count {
            let node = self
                .node(node_index)
                .ok_or(ModelError::NodeIndexOutOfBounds {
                    index: node_index,
                    node_count: self.layout.node_count,
                })?;
            let widget_type = node.widget_type();
            let schema = node.widget_schema();
            if !support.supports(widget_type, schema) {
                return Err(ModelError::UnsupportedWidgetSchema {
                    node: node_index,
                    widget_type,
                    schema,
                });
            }
            support.validate_node(self, node_index, node)?;
        }
        Ok(())
    }

    /// Borrows one node record by document-local index.
    #[inline]
    pub fn node(&self, index: u32) -> Option<WidgetNodeView<'a>> {
        (index < self.layout.node_count).then(|| WidgetNodeView {
            bytes: self.bytes,
            record_offset: self.layout.nodes_start + index as usize * NODE_RECORD_LEN,
            properties_start: self.layout.properties_start,
            callbacks_start: self.layout.callbacks_start,
            children_start: self.layout.children_start,
        })
    }

    /// Borrows one validated UTF-8 string by table index.
    pub fn string(&self, index: u32) -> Option<&'a str> {
        if index >= self.layout.string_count {
            return None;
        }
        let record = self.layout.string_ranges_start + index as usize * RANGE_RECORD_LEN;
        let start = read_u32(self.bytes, record) as usize;
        let length = read_u32(self.bytes, record + 4) as usize;
        let start = self.layout.string_bytes_start + start;
        Some(std::str::from_utf8(&self.bytes[start..start + length]).unwrap())
    }

    /// Borrows one validated opaque byte string by table index.
    pub fn blob(&self, index: u32) -> Option<&'a [u8]> {
        if index >= self.layout.blob_count {
            return None;
        }
        let record = self.layout.blob_ranges_start + index as usize * RANGE_RECORD_LEN;
        let start = read_u32(self.bytes, record) as usize;
        let length = read_u32(self.bytes, record + 4) as usize;
        let start = self.layout.blob_bytes_start + start;
        Some(&self.bytes[start..start + length])
    }
}

/// A borrowed view over one validated fixed-width widget record.
pub struct WidgetNodeView<'a> {
    bytes: &'a [u8],
    record_offset: usize,
    properties_start: usize,
    callbacks_start: usize,
    children_start: usize,
}

impl WidgetNodeView<'_> {
    /// Returns the stable Aimer widget type code.
    #[inline]
    pub fn widget_type(&self) -> WidgetSchemaId {
        WidgetSchemaId::new(read_u64(self.bytes, self.record_offset))
    }

    /// Returns the widget schema version for this node.
    #[inline]
    pub fn widget_schema(&self) -> Version {
        read_version(self.bytes, self.record_offset + 8)
    }

    /// Returns the optional stable native-reconciliation key.
    pub fn key(&self) -> Option<StableId128> {
        (read_u32(self.bytes, self.record_offset + 12) & KEY_PRESENT != 0).then(|| {
            let mut key = [0_u8; 16];
            key.copy_from_slice(&self.bytes[self.record_offset + 16..self.record_offset + 32]);
            StableId128::from_bytes(key)
        })
    }

    /// Iterates document-local child indices without allocating.
    pub fn children(&self) -> ChildIndices<'_> {
        let start = read_u32(self.bytes, self.record_offset + 48) as usize;
        let count = read_u32(self.bytes, self.record_offset + 52) as usize;
        let byte_start = self.children_start + start * 4;
        ChildIndices {
            bytes: &self.bytes[byte_start..byte_start + count * 4],
            offset: 0,
        }
    }

    /// Iterates validated typed properties without allocating.
    pub fn properties(&self) -> WidgetProperties<'_> {
        let start = read_u32(self.bytes, self.record_offset + 32) as usize;
        let count = read_u32(self.bytes, self.record_offset + 36) as usize;
        let byte_start = self.properties_start + start * PROPERTY_RECORD_LEN;
        WidgetProperties {
            bytes: &self.bytes[byte_start..byte_start + count * PROPERTY_RECORD_LEN],
            offset: 0,
        }
    }

    /// Iterates stable callback bindings without allocating.
    pub fn callbacks(&self) -> CallbackBindings<'_> {
        let start = read_u32(self.bytes, self.record_offset + 40) as usize;
        let count = read_u32(self.bytes, self.record_offset + 44) as usize;
        let byte_start = self.callbacks_start + start * CALLBACK_RECORD_LEN;
        CallbackBindings {
            bytes: &self.bytes[byte_start..byte_start + count * CALLBACK_RECORD_LEN],
            offset: 0,
        }
    }
}

/// An allocation-free iterator over validated fixed-width property records.
pub struct WidgetProperties<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Iterator for WidgetProperties<'_> {
    type Item = WidgetProperty;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        let value = decode_property(self.bytes, self.offset, u32::MAX, u32::MAX).unwrap();
        self.offset += PROPERTY_RECORD_LEN;
        Some(value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.bytes.len() - self.offset) / PROPERTY_RECORD_LEN;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for WidgetProperties<'_> {}

/// An allocation-free iterator over validated fixed-width callback bindings.
pub struct CallbackBindings<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Iterator for CallbackBindings<'_> {
    type Item = CallbackBinding;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        let record = &self.bytes[self.offset..self.offset + CALLBACK_RECORD_LEN];
        let mut callback_id = [0_u8; 16];
        callback_id.copy_from_slice(&record[24..40]);
        self.offset += CALLBACK_RECORD_LEN;
        let async_version = read_version(record, 12);
        let async_schema = (async_version != Version::new(0, 0)).then_some(async_version);
        Some(match async_schema {
            Some(async_schema) => CallbackBinding::new_async(
                EventId::new(read_u64(record, 0)),
                read_version(record, 8),
                async_schema,
                StableId128::from_bytes(callback_id),
            ),
            None => CallbackBinding::new(
                EventId::new(read_u64(record, 0)),
                read_version(record, 8),
                StableId128::from_bytes(callback_id),
            ),
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.bytes.len() - self.offset) / CALLBACK_RECORD_LEN;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for CallbackBindings<'_> {}

/// An allocation-free iterator over validated document-local child indices.
pub struct ChildIndices<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Iterator for ChildIndices<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        let value = read_u32(self.bytes, self.offset);
        self.offset += 4;
        Some(value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.bytes.len() - self.offset) / 4;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for ChildIndices<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.offset == self.bytes.len() {
            return None;
        }
        let record_offset = self.bytes.len() - size_of::<u32>();
        let value = read_u32(self.bytes, record_offset);
        self.bytes = &self.bytes[..record_offset];
        Some(value)
    }
}

impl ExactSizeIterator for ChildIndices<'_> {}

fn validate_graph(
    bytes: &[u8],
    layout: Layout,
    root_node: u32,
    max_depth: u32,
) -> Result<(), ModelError> {
    let node_count = layout.node_count as usize;
    let mut parents = vec![None; node_count];
    let mut child_stamps = vec![u32::MAX; node_count];
    let mut multiple_parents = None;

    for parent in 0..layout.node_count {
        for child in graph_children(bytes, layout, parent) {
            if child_stamps[child as usize] == parent {
                return Err(ModelError::DuplicateWidgetChild { parent, child });
            }
            child_stamps[child as usize] = parent;
            match parents[child as usize] {
                None => parents[child as usize] = Some(parent),
                Some(first_parent) if first_parent != parent && multiple_parents.is_none() => {
                    multiple_parents = Some(ModelError::MultipleWidgetParents {
                        node: child,
                        first_parent,
                        second_parent: parent,
                    });
                }
                Some(_) => {}
            }
        }
    }

    validate_acyclic_graph(bytes, layout)?;
    if let Some(error) = multiple_parents {
        return Err(error);
    }

    let mut reached = vec![false; node_count];
    let mut stack = Vec::with_capacity(node_count);
    stack.push((root_node, 1_u32));
    while let Some((node, depth)) = stack.pop() {
        if depth > max_depth {
            return Err(ModelError::WidgetDepthExceeded {
                depth,
                limit: max_depth,
            });
        }
        if reached[node as usize] {
            return Err(ModelError::WidgetCycle { node });
        }
        reached[node as usize] = true;
        for child in graph_children(bytes, layout, node).rev() {
            stack.push((child, depth + 1));
        }
    }

    if let Some(node) = (0..layout.node_count)
        .find(|&node| !reached[node as usize] && parents[node as usize].is_none())
    {
        return Err(ModelError::UnreachableWidgetNode { node });
    }
    if let Some(node) = (0..layout.node_count).find(|&node| !reached[node as usize]) {
        return Err(ModelError::WidgetCycle { node });
    }
    Ok(())
}

fn validate_acyclic_graph(bytes: &[u8], layout: Layout) -> Result<(), ModelError> {
    let mut colors = vec![0_u8; layout.node_count as usize];
    let mut stack = Vec::with_capacity(layout.node_count as usize);
    for start in 0..layout.node_count {
        if colors[start as usize] != 0 {
            continue;
        }
        stack.push((start, false));
        while let Some((node, exiting)) = stack.pop() {
            if exiting {
                colors[node as usize] = 2;
                continue;
            }
            if colors[node as usize] == 2 {
                continue;
            }
            colors[node as usize] = 1;
            stack.push((node, true));
            for child in graph_children(bytes, layout, node).rev() {
                match colors[child as usize] {
                    0 => stack.push((child, false)),
                    1 => return Err(ModelError::WidgetCycle { node: child }),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn graph_children(bytes: &[u8], layout: Layout, node: u32) -> ChildIndices<'_> {
    let node_offset = layout.nodes_start + node as usize * NODE_RECORD_LEN;
    let child_start = read_u32(bytes, node_offset + 48) as usize;
    let child_count = read_u32(bytes, node_offset + 52) as usize;
    let byte_start = layout.children_start + child_start * size_of::<u32>();
    ChildIndices {
        bytes: &bytes[byte_start..byte_start + child_count * size_of::<u32>()],
        offset: 0,
    }
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    node_count: u32,
    property_count: u32,
    callback_count: u32,
    child_count: u32,
    string_count: u32,
    string_bytes_len: u32,
    blob_count: u32,
    blob_bytes_len: u32,
    nodes_start: usize,
    properties_start: usize,
    callbacks_start: usize,
    children_start: usize,
    string_ranges_start: usize,
    string_bytes_start: usize,
    blob_ranges_start: usize,
    blob_bytes_start: usize,
}

#[cfg(feature = "wasm-hot-reload")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedWidgetDocument {
    layout: Layout,
    generation_id: u64,
    document_revision: u64,
    root_node: u32,
}

impl Layout {
    fn decode(bytes: &[u8], limits: ModelLimits) -> Result<Self, ModelError> {
        if bytes.len() < HEADER_LEN {
            return Err(ModelError::Truncated {
                needed: HEADER_LEN,
                actual: bytes.len(),
            });
        }
        if bytes[..4] != MAGIC {
            return Err(ModelError::InvalidMagic);
        }
        let version = read_version(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(ModelError::UnsupportedVersion { version });
        }
        let declared_len = read_u32(bytes, 60) as usize;
        if declared_len != bytes.len() {
            return Err(ModelError::LengthMismatch {
                declared: declared_len,
                actual: bytes.len(),
            });
        }
        if bytes.len() > limits.max_document_bytes as usize {
            return Err(ModelError::DocumentTooLarge {
                length: bytes.len(),
                limit: limits.max_document_bytes,
            });
        }
        let node_count = read_u32(bytes, 28);
        let property_count = read_u32(bytes, 32);
        let callback_count = read_u32(bytes, 36);
        let child_count = read_u32(bytes, 40);
        let string_count = read_u32(bytes, 44);
        let string_bytes_len = read_u32(bytes, 48);
        let blob_count = read_u32(bytes, 52);
        let blob_bytes_len = read_u32(bytes, 56);
        for count in [
            node_count,
            property_count,
            callback_count,
            child_count,
            string_count,
            blob_count,
        ] {
            check_count(count as usize, limits)?;
        }
        let nodes_start = HEADER_LEN;
        let properties_start = checked_end(nodes_start, node_count, NODE_RECORD_LEN)?;
        let callbacks_start = checked_end(properties_start, property_count, PROPERTY_RECORD_LEN)?;
        let children_start = checked_end(callbacks_start, callback_count, CALLBACK_RECORD_LEN)?;
        let string_ranges_start = checked_end(children_start, child_count, 4)?;
        let string_bytes_start = checked_end(string_ranges_start, string_count, RANGE_RECORD_LEN)?;
        let blob_ranges_start = string_bytes_start
            .checked_add(string_bytes_len as usize)
            .ok_or(ModelError::LengthOverflow)?;
        let blob_bytes_start = checked_end(blob_ranges_start, blob_count, RANGE_RECORD_LEN)?;
        let expected_end = blob_bytes_start
            .checked_add(blob_bytes_len as usize)
            .ok_or(ModelError::LengthOverflow)?;
        if expected_end != bytes.len() {
            return Err(ModelError::LengthMismatch {
                declared: expected_end,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            node_count,
            property_count,
            callback_count,
            child_count,
            string_count,
            string_bytes_len,
            blob_count,
            blob_bytes_len,
            nodes_start,
            properties_start,
            callbacks_start,
            children_start,
            string_ranges_start,
            string_bytes_start,
            blob_ranges_start,
            blob_bytes_start,
        })
    }
}

fn check_count(count: usize, limits: ModelLimits) -> Result<(), ModelError> {
    if u32::try_from(count).is_err() || count > limits.max_collection_entries as usize {
        return Err(ModelError::CollectionTooLarge {
            count,
            limit: limits.max_collection_entries,
        });
    }
    Ok(())
}

fn checked_variable_bytes(values: &[&str], limit: u32) -> Result<usize, ModelError> {
    let mut total = 0_usize;
    for value in values {
        if value.len() > limit as usize {
            return Err(ModelError::StringTooLarge {
                length: value.len(),
                limit,
            });
        }
        total = total
            .checked_add(value.len())
            .ok_or(ModelError::LengthOverflow)?;
    }
    u32::try_from(total)
        .map_err(|_| ModelError::LengthOverflow)
        .map(|value| value as usize)
}

fn checked_blob_bytes(values: &[&[u8]], limit: u32) -> Result<usize, ModelError> {
    let mut total = 0_usize;
    for value in values {
        if value.len() > limit as usize {
            return Err(ModelError::BlobTooLarge {
                length: value.len(),
                limit,
            });
        }
        total = total
            .checked_add(value.len())
            .ok_or(ModelError::LengthOverflow)?;
    }
    u32::try_from(total)
        .map_err(|_| ModelError::LengthOverflow)
        .map(|value| value as usize)
}

fn checked_section_len(count: usize, width: usize) -> Result<usize, ModelError> {
    count.checked_mul(width).ok_or(ModelError::LengthOverflow)
}

fn checked_end(start: usize, count: u32, width: usize) -> Result<usize, ModelError> {
    start
        .checked_add(checked_section_len(count as usize, width)?)
        .ok_or(ModelError::LengthOverflow)
}

fn validate_range(start: u32, count: u32, section_count: u32) -> Result<(), ModelError> {
    let end = start.checked_add(count).ok_or(ModelError::LengthOverflow)?;
    if end > section_count {
        return Err(ModelError::SectionRangeOutOfBounds);
    }
    Ok(())
}

fn validate_canonical_range(
    start: u32,
    count: u32,
    section_count: u32,
    expected_start: &mut u32,
) -> Result<(), ModelError> {
    if start != *expected_start {
        return Err(ModelError::NonCanonicalSectionLayout);
    }
    validate_range(start, count, section_count)?;
    *expected_start = start.checked_add(count).ok_or(ModelError::LengthOverflow)?;
    Ok(())
}

fn validate_strings(bytes: &[u8], layout: Layout, limits: ModelLimits) -> Result<(), ModelError> {
    let mut expected_start = 0_u32;
    for index in 0..layout.string_count as usize {
        let record = layout.string_ranges_start + index * RANGE_RECORD_LEN;
        let start = read_u32(bytes, record);
        let length = read_u32(bytes, record + 4);
        validate_canonical_range(start, length, layout.string_bytes_len, &mut expected_start)?;
        if length > limits.max_string_bytes {
            return Err(ModelError::StringTooLarge {
                length: length as usize,
                limit: limits.max_string_bytes,
            });
        }
        let start = layout.string_bytes_start + start as usize;
        let end = start + length as usize;
        std::str::from_utf8(&bytes[start..end]).map_err(|_| ModelError::InvalidUtf8)?;
    }
    if expected_start != layout.string_bytes_len {
        return Err(ModelError::NonCanonicalSectionLayout);
    }
    Ok(())
}

fn validate_blob_ranges(
    bytes: &[u8],
    layout: Layout,
    limits: ModelLimits,
) -> Result<(), ModelError> {
    let mut expected_start = 0_u32;
    for index in 0..layout.blob_count as usize {
        let record = layout.blob_ranges_start + index * RANGE_RECORD_LEN;
        let start = read_u32(bytes, record);
        let length = read_u32(bytes, record + 4);
        validate_canonical_range(start, length, layout.blob_bytes_len, &mut expected_start)?;
        if length > limits.max_blob_bytes {
            return Err(ModelError::BlobTooLarge {
                length: length as usize,
                limit: limits.max_blob_bytes,
            });
        }
    }
    if expected_start != layout.blob_bytes_len {
        return Err(ModelError::NonCanonicalSectionLayout);
    }
    Ok(())
}

fn validate_property_value(
    value: PropertyValue,
    string_count: u32,
    blob_count: u32,
) -> Result<(), ModelError> {
    match value {
        PropertyValue::F64(value) if !value.is_finite() => Err(ModelError::NonFiniteFloat),
        PropertyValue::StringRef(index) if index >= string_count => {
            Err(ModelError::PropertyReferenceOutOfBounds {
                index,
                count: string_count,
            })
        }
        PropertyValue::BlobRef(index) if index >= blob_count => {
            Err(ModelError::PropertyReferenceOutOfBounds {
                index,
                count: blob_count,
            })
        }
        _ => Ok(()),
    }
}

#[inline]
fn remap_property(
    property: WidgetProperty,
    string_remap: Option<&[u32]>,
    blob_remap: Option<&[u32]>,
) -> WidgetProperty {
    let value = match (property.value, string_remap, blob_remap) {
        (PropertyValue::StringRef(index), Some(remap), _) => {
            PropertyValue::StringRef(remap[index as usize])
        }
        (PropertyValue::BlobRef(index), _, Some(remap)) => {
            PropertyValue::BlobRef(remap[index as usize])
        }
        (value, _, _) => value,
    };
    WidgetProperty { value, ..property }
}

fn encode_property(output: &mut Vec<u8>, property: WidgetProperty) {
    write_u64(output, property.property_id.value());
    let (kind, value) = match property.value {
        PropertyValue::Bool(value) => (1, u64::from(value)),
        PropertyValue::I64(value) => (2, value as u64),
        PropertyValue::F64(value) => (3, value.to_bits()),
        PropertyValue::Rgba(value) => (4, u64::from(value)),
        PropertyValue::StringRef(value) => (5, u64::from(value)),
        PropertyValue::BlobRef(value) => (6, u64::from(value)),
    };
    output.push(kind);
    output.push(u8::from(property.optional) * PROPERTY_OPTIONAL);
    write_u16(output, 0);
    write_u32(output, 0);
    write_u64(output, value);
    write_u64(output, 0);
}

fn decode_property(
    bytes: &[u8],
    offset: usize,
    string_count: u32,
    blob_count: u32,
) -> Result<WidgetProperty, ModelError> {
    let kind = bytes[offset + 8];
    let flags = bytes[offset + 9];
    let reserved = read_u16(bytes, offset + 10);
    let encoded = read_u64(bytes, offset + 16);
    if flags & !PROPERTY_OPTIONAL != 0
        || reserved != 0
        || read_u32(bytes, offset + 12) != 0
        || read_u64(bytes, offset + 24) != 0
    {
        return Err(ModelError::InvalidProperty);
    }
    let value = match kind {
        1 if encoded <= 1 => PropertyValue::Bool(encoded == 1),
        2 => PropertyValue::I64(encoded as i64),
        3 => PropertyValue::F64(f64::from_bits(encoded)),
        4 => PropertyValue::Rgba(u32::try_from(encoded).map_err(|_| ModelError::InvalidProperty)?),
        5 => PropertyValue::StringRef(
            u32::try_from(encoded).map_err(|_| ModelError::InvalidProperty)?,
        ),
        6 => {
            PropertyValue::BlobRef(u32::try_from(encoded).map_err(|_| ModelError::InvalidProperty)?)
        }
        _ => return Err(ModelError::InvalidProperty),
    };
    validate_property_value(value, string_count, blob_count)?;
    let property = WidgetProperty::new(PropertyId::new(read_u64(bytes, offset)), value);
    Ok(if flags & PROPERTY_OPTIONAL != 0 {
        property.optional()
    } else {
        property
    })
}

fn write_ranges(output: &mut Vec<u8>, lengths: impl Iterator<Item = usize>) {
    let mut start = 0_u32;
    for length in lengths {
        write_u32(output, start);
        write_u32(output, length as u32);
        start += length as u32;
    }
}

#[inline]
fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[inline]
fn write_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[inline]
fn write_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[inline]
fn write_version(output: &mut Vec<u8>, version: Version) {
    output.extend_from_slice(&version.major().to_le_bytes());
    output.extend_from_slice(&version.minor().to_le_bytes());
}

#[inline]
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[inline]
fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

#[inline]
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[inline]
fn read_version(bytes: &[u8], offset: usize) -> Version {
    Version::new(
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()),
        u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()),
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        CallbackBinding, ModelLimits, PROPERTY_TEXT_CONTENT, PropertyValue, StableId128, Version,
        WIDGET_TEXT, WidgetDocument, WidgetDocumentView, WidgetNode, WidgetProperty,
        WidgetSchemaId, WidgetSchemaMetadata, WidgetSchemaMetadataError, stable_schema_hash64,
        validate_widget_schema_metadata,
    };

    #[test]
    fn text_schema_uses_stable_u64_ids_and_awir_two() {
        assert_eq!(stable_schema_hash64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_schema_hash64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            WIDGET_TEXT.value(),
            stable_schema_hash64("aimer.widget:aimer_text::Text")
        );
        assert_eq!(
            PROPERTY_TEXT_CONTENT.value(),
            stable_schema_hash64("aimer.property:aimer_text::Text:text")
        );

        let properties = [WidgetProperty::new(
            PROPERTY_TEXT_CONTENT,
            PropertyValue::StringRef(0),
        )];
        let nodes = [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&properties)];
        let strings = ["Hello"];
        let document = WidgetDocument::new(1, 1, 0, &nodes, &strings, &[]);
        let encoded = document.encode(ModelLimits::new(512, 16, 64, 64)).unwrap();

        assert_eq!(&encoded[..8], &[b'A', b'W', b'I', b'R', 2, 0, 0, 0]);
        let view = WidgetDocumentView::decode(&encoded, ModelLimits::new(512, 16, 64, 64)).unwrap();
        let root = view.node(0).unwrap();
        assert_eq!(root.widget_type(), WIDGET_TEXT);
        assert_eq!(
            root.properties().next().unwrap().property_id(),
            PROPERTY_TEXT_CONTENT
        );
    }

    #[test]
    fn duplicate_text_schema_identity_is_rejected() {
        let metadata = WidgetSchemaMetadata::new(
            WIDGET_TEXT,
            "aimer.widget:aimer_text::Text",
            Version::new(1, 0),
            Version::new(1, 0),
        );

        assert_eq!(
            validate_widget_schema_metadata(&[metadata, metadata]),
            Err(WidgetSchemaMetadataError::OverlappingVersions {
                id: WidgetSchemaId::new(WIDGET_TEXT.value()),
                first: "aimer.widget:aimer_text::Text",
                second: "aimer.widget:aimer_text::Text",
            })
        );
    }

    #[test]
    fn button_event_kinds_are_stable_and_distinct() {
        assert_ne!(
            CallbackBinding::EVENT_BUTTON_PRESS,
            CallbackBinding::EVENT_BUTTON_LONG_PRESS
        );
        assert_ne!(
            CallbackBinding::EVENT_BUTTON_PRESS,
            CallbackBinding::EVENT_BUTTON_DOUBLE_PRESS
        );
        assert_ne!(
            CallbackBinding::EVENT_BUTTON_PRESS,
            CallbackBinding::EVENT_BUTTON_RIGHT_PRESS
        );
    }

    #[test]
    fn minimal_widget_document_matches_the_version_two_golden_image() {
        let nodes = [WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 2))];
        let document = WidgetDocument::new(11, 13, 0, &nodes, &[], &[]);

        let encoded = document.encode(ModelLimits::new(512, 16, 64, 64)).unwrap();

        assert_eq!(
            encoded,
            [
                b'A', b'W', b'I', b'R', 2, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 13, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]
        );

        let view = WidgetDocumentView::decode(&encoded, ModelLimits::new(512, 16, 64, 64)).unwrap();
        assert_eq!(view.generation_id(), 11);
        assert_eq!(view.document_revision(), 13);
        assert_eq!(view.root_node(), 0);
        assert_eq!(view.node_count(), 1);
        let root = view.node(0).unwrap();
        assert_eq!(root.widget_type(), WidgetSchemaId::new(7));
        assert_eq!(root.widget_schema(), Version::new(1, 2));
        assert_eq!(root.key(), None);
        assert_eq!(root.children().count(), 0);
    }

    #[test]
    fn widget_document_sections_round_trip_through_borrowed_views() {
        let widget_key = StableId128::from_bytes([0x11; 16]);
        let callback_id = StableId128::from_bytes([0x22; 16]);
        let child_indices = [1];
        let properties = [
            WidgetProperty::new(crate::PropertyId::new(3), PropertyValue::StringRef(0)),
            WidgetProperty::new(crate::PropertyId::new(5), PropertyValue::BlobRef(0)).optional(),
            WidgetProperty::new(crate::PropertyId::new(8), PropertyValue::Bool(true)),
        ];
        let callbacks = [CallbackBinding::new(
            crate::EventId::new(9),
            Version::new(2, 1),
            callback_id,
        )];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0)).children(&child_indices),
            WidgetNode::new(WidgetSchemaId::new(8), Version::new(1, 1))
                .key(widget_key)
                .properties(&properties)
                .callbacks(&callbacks),
        ];
        let strings = ["Aimer"];
        let blob = [0xA5, 0x5A];
        let blobs: [&[u8]; 1] = [&blob];
        let document = WidgetDocument::new(17, 19, 0, &nodes, &strings, &blobs);

        let encoded = document
            .encode(ModelLimits::new(1_024, 16, 64, 64))
            .unwrap();
        let view =
            WidgetDocumentView::decode(&encoded, ModelLimits::new(1_024, 16, 64, 64)).unwrap();

        assert_eq!(view.string(0), Some("Aimer"));
        assert_eq!(view.blob(0), Some(blob.as_slice()));
        assert_eq!(view.node(0).unwrap().children().collect::<Vec<_>>(), [1]);
        let child = view.node(1).unwrap();
        assert_eq!(child.key(), Some(widget_key));
        assert_eq!(
            child.properties().collect::<Vec<_>>(),
            [
                WidgetProperty::new(crate::PropertyId::new(3), PropertyValue::StringRef(0)),
                WidgetProperty::new(crate::PropertyId::new(5), PropertyValue::BlobRef(0))
                    .optional(),
                WidgetProperty::new(crate::PropertyId::new(8), PropertyValue::Bool(true)),
            ]
        );
        assert_eq!(
            child.callbacks().collect::<Vec<_>>(),
            [CallbackBinding::new(
                crate::EventId::new(9),
                Version::new(2, 1),
                callback_id,
            )]
        );
    }

    #[test]
    fn widget_document_rejects_out_of_range_child_indices() {
        let children = [1];
        let nodes =
            [WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0)).children(&children)];
        let document = WidgetDocument::new(1, 1, 0, &nodes, &[], &[]);

        assert_eq!(
            document.encode(ModelLimits::new(512, 16, 64, 64)),
            Err(crate::ModelError::NodeIndexOutOfBounds {
                index: 1,
                node_count: 1,
            })
        );
    }

    #[test]
    fn widget_document_rejects_duplicate_stable_keys() {
        let key = StableId128::from_bytes([0x44; 16]);
        let children = [1];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0))
                .key(key)
                .children(&children),
            WidgetNode::new(WidgetSchemaId::new(8), Version::new(1, 0)).key(key),
        ];
        let document = WidgetDocument::new(1, 1, 0, &nodes, &[], &[]);

        assert_eq!(
            document.encode(ModelLimits::new(512, 16, 64, 64)),
            Err(crate::ModelError::DuplicateWidgetKey { key })
        );
    }

    #[test]
    fn widget_document_rejects_unsupported_versions_and_resource_limits() {
        let nodes = [WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0))];
        let document = WidgetDocument::new(1, 1, 0, &nodes, &[], &[]);
        let mut encoded = document.encode(ModelLimits::new(512, 16, 64, 64)).unwrap();
        encoded[4] = 3;

        assert!(matches!(
            WidgetDocumentView::decode(&encoded, ModelLimits::new(512, 16, 64, 64)),
            Err(crate::ModelError::UnsupportedVersion {
                version: Version { .. }
            })
        ));
        assert_eq!(
            document.encode(ModelLimits::new(127, 16, 64, 64)),
            Err(crate::ModelError::DocumentTooLarge {
                length: 128,
                limit: 127,
            })
        );
    }

    #[test]
    fn widget_document_decoder_rejects_overlapping_node_ranges() {
        let children = [1];
        let root_properties = [WidgetProperty::new(
            crate::PropertyId::new(1),
            PropertyValue::Bool(true),
        )];
        let child_properties = [WidgetProperty::new(
            crate::PropertyId::new(2),
            PropertyValue::Bool(false),
        )];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0))
                .properties(&root_properties)
                .children(&children),
            WidgetNode::new(WidgetSchemaId::new(8), Version::new(1, 0))
                .properties(&child_properties),
        ];
        let mut encoded = WidgetDocument::new(1, 1, 0, &nodes, &[], &[])
            .encode(ModelLimits::new(512, 16, 64, 64))
            .unwrap();
        encoded[160..164].copy_from_slice(&0_u32.to_le_bytes());

        assert!(matches!(
            WidgetDocumentView::decode(&encoded, ModelLimits::new(512, 16, 64, 64)),
            Err(crate::ModelError::NonCanonicalSectionLayout)
        ));
    }
}
