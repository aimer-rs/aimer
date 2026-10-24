//! Declarative metadata shared by portable widget lowering and materialization.

use core::fmt;
use std::error::Error;

use crate::{
    EventId, ModelError, PropertyId, PropertyValue, ValueTypeId, Version, WidgetDocumentView,
    WidgetNodeView, WidgetSchemaId, WidgetSchemaMetadata, WidgetSchemaMetadataError,
    WidgetSchemaSupport, validate_widget_schema_metadata,
};

/// The fixed AWIR representation selected for a portable property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PropertyValueKind {
    /// A canonical zero-or-one Boolean.
    Bool = 1,
    /// A signed 64-bit integer.
    I64 = 2,
    /// A finite IEEE-754 64-bit number.
    F64 = 3,
    /// A packed red-green-blue-alpha value.
    Rgba = 4,
    /// An index into the document string table.
    StringRef = 5,
    /// An index into the document blob table.
    BlobRef = 6,
}

/// Whether a portable widget property must appear in every node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyPresence {
    /// The property must be present.
    Required,
    /// The property may be omitted so the versioned widget default applies.
    Optional,
}

/// Versioned and bounded metadata for one custom blob-backed value type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueSchemaMetadata<'a> {
    id: ValueTypeId,
    canonical_name: &'a str,
    version: Version,
    maximum_encoded_bytes: u32,
}

impl<'a> ValueSchemaMetadata<'a> {
    /// Creates custom value metadata with an identity derived from its canonical name.
    #[inline]
    pub const fn from_canonical_name(
        canonical_name: &'a str,
        version: Version,
        maximum_encoded_bytes: u32,
    ) -> Self {
        Self::new(
            ValueTypeId::from_canonical_name(canonical_name),
            canonical_name,
            version,
            maximum_encoded_bytes,
        )
    }

    /// Creates custom value metadata.
    #[inline]
    pub const fn new(
        id: ValueTypeId,
        canonical_name: &'a str,
        version: Version,
        maximum_encoded_bytes: u32,
    ) -> Self {
        Self {
            id,
            canonical_name,
            version,
            maximum_encoded_bytes,
        }
    }

    /// Returns the stable value-type identity.
    #[inline]
    pub const fn id(self) -> ValueTypeId {
        self.id
    }

    /// Returns the domain-separated canonical value name.
    #[inline]
    pub const fn canonical_name(self) -> &'a str {
        self.canonical_name
    }

    /// Returns the value codec version.
    #[inline]
    pub const fn version(self) -> Version {
        self.version
    }

    /// Returns the maximum encoded payload length.
    #[inline]
    pub const fn maximum_encoded_bytes(self) -> u32 {
        self.maximum_encoded_bytes
    }
}

/// Static wire metadata for one property in a portable widget schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PropertySchemaMetadata<'a> {
    id: PropertyId,
    canonical_name: &'a str,
    value_kind: PropertyValueKind,
    presence: PropertyPresence,
    value_schema: Option<ValueSchemaMetadata<'a>>,
}

impl<'a> PropertySchemaMetadata<'a> {
    /// Creates required property metadata with an identity derived from its canonical name.
    #[inline]
    pub const fn from_canonical_name(
        canonical_name: &'a str,
        value_kind: PropertyValueKind,
    ) -> Self {
        Self::new(
            PropertyId::from_canonical_name(canonical_name),
            canonical_name,
            value_kind,
        )
    }

    /// Creates required property metadata.
    #[inline]
    pub const fn new(
        id: PropertyId,
        canonical_name: &'a str,
        value_kind: PropertyValueKind,
    ) -> Self {
        Self {
            id,
            canonical_name,
            value_kind,
            presence: PropertyPresence::Required,
            value_schema: None,
        }
    }

    /// Marks the property as omittable.
    #[inline]
    pub const fn optional(mut self) -> Self {
        self.presence = PropertyPresence::Optional;
        self
    }

    /// Sets whether this property is required or omittable.
    #[inline]
    pub const fn with_presence(mut self, presence: PropertyPresence) -> Self {
        self.presence = presence;
        self
    }

    /// Attaches the custom value contract used by a blob-backed property.
    #[inline]
    pub const fn with_value_schema(mut self, value_schema: ValueSchemaMetadata<'a>) -> Self {
        self.value_schema = Some(value_schema);
        self
    }

    /// Attaches custom value metadata when the reflected Rust type provides it.
    #[inline]
    pub const fn with_optional_value_schema(
        mut self,
        value_schema: Option<ValueSchemaMetadata<'a>>,
    ) -> Self {
        self.value_schema = value_schema;
        self
    }

    /// Returns the stable property identity.
    #[inline]
    pub const fn id(self) -> PropertyId {
        self.id
    }

    /// Returns the domain-separated canonical property name.
    #[inline]
    pub const fn canonical_name(self) -> &'a str {
        self.canonical_name
    }

    /// Returns the property's fixed AWIR representation.
    #[inline]
    pub const fn value_kind(self) -> PropertyValueKind {
        self.value_kind
    }

    /// Returns whether the property is required or optional.
    #[inline]
    pub const fn presence(self) -> PropertyPresence {
        self.presence
    }

    /// Returns custom value metadata for a blob-backed property.
    #[inline]
    pub const fn value_schema(self) -> Option<ValueSchemaMetadata<'a>> {
        self.value_schema
    }
}

/// Versioned and bounded metadata for an async callback capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncCallbackSchemaMetadata {
    contract_version: Version,
    maximum_in_flight_tasks: u32,
    maximum_completion_bytes: u32,
    maximum_callback_fuel: u32,
    maximum_retained_resources: u32,
}

impl AsyncCallbackSchemaMetadata {
    /// Creates a bounded async callback contract.
    #[inline]
    pub const fn new(
        contract_version: Version,
        maximum_in_flight_tasks: u32,
        maximum_completion_bytes: u32,
    ) -> Self {
        Self {
            contract_version,
            maximum_in_flight_tasks,
            maximum_completion_bytes,
            maximum_callback_fuel: u32::MAX,
            maximum_retained_resources: u32::MAX,
        }
    }

    /// Sets the maximum callback fuel budget declared by the contract.
    #[inline]
    pub const fn with_maximum_callback_fuel(mut self, maximum: u32) -> Self {
        self.maximum_callback_fuel = maximum;
        self
    }

    /// Sets the maximum number of retained host resources for one task.
    #[inline]
    pub const fn with_maximum_retained_resources(mut self, maximum: u32) -> Self {
        self.maximum_retained_resources = maximum;
        self
    }

    /// Returns the async protocol contract version.
    #[inline]
    pub const fn contract_version(self) -> Version {
        self.contract_version
    }

    /// Returns the in-flight task ceiling.
    #[inline]
    pub const fn maximum_in_flight_tasks(self) -> u32 {
        self.maximum_in_flight_tasks
    }

    /// Returns the completion payload byte ceiling.
    #[inline]
    pub const fn maximum_completion_bytes(self) -> u32 {
        self.maximum_completion_bytes
    }

    /// Returns the callback fuel ceiling.
    #[inline]
    pub const fn maximum_callback_fuel(self) -> u32 {
        self.maximum_callback_fuel
    }

    /// Returns the retained-resource ceiling.
    #[inline]
    pub const fn maximum_retained_resources(self) -> u32 {
        self.maximum_retained_resources
    }
}

/// Static wire metadata for one callback slot in a portable widget schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallbackSchemaMetadata<'a> {
    id: EventId,
    canonical_name: &'a str,
    event_schema: Version,
    maximum_bindings: u32,
    async_schema: Option<AsyncCallbackSchemaMetadata>,
}

impl<'a> CallbackSchemaMetadata<'a> {
    /// Creates callback metadata with an identity derived from its canonical name.
    #[inline]
    pub const fn from_canonical_name(
        canonical_name: &'a str,
        event_schema: Version,
        maximum_bindings: u32,
    ) -> Self {
        Self::new(
            EventId::from_canonical_name(canonical_name),
            canonical_name,
            event_schema,
            maximum_bindings,
        )
    }

    /// Creates callback metadata with a bounded number of bindings per node.
    #[inline]
    pub const fn new(
        id: EventId,
        canonical_name: &'a str,
        event_schema: Version,
        maximum_bindings: u32,
    ) -> Self {
        Self {
            id,
            canonical_name,
            event_schema,
            maximum_bindings,
            async_schema: None,
        }
    }

    /// Declares the versioned async callback capability for this event slot.
    #[inline]
    pub const fn with_async_schema(mut self, async_schema: AsyncCallbackSchemaMetadata) -> Self {
        self.async_schema = Some(async_schema);
        self
    }

    /// Returns the stable event identity.
    #[inline]
    pub const fn id(self) -> EventId {
        self.id
    }

    /// Returns the domain-separated canonical event name.
    #[inline]
    pub const fn canonical_name(self) -> &'a str {
        self.canonical_name
    }

    /// Returns the callback payload schema version.
    #[inline]
    pub const fn event_schema(self) -> Version {
        self.event_schema
    }

    /// Returns the maximum bindings accepted for one node.
    #[inline]
    pub const fn maximum_bindings(self) -> u32 {
        self.maximum_bindings
    }

    /// Returns the optional async callback contract.
    #[inline]
    pub const fn async_schema(self) -> Option<AsyncCallbackSchemaMetadata> {
        self.async_schema
    }
}

/// Inclusive child-count bounds for a portable widget schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChildCardinality {
    minimum: u32,
    maximum: u32,
}

impl ChildCardinality {
    /// Creates inclusive child-count bounds.
    #[inline]
    pub const fn new(minimum: u32, maximum: u32) -> Self {
        Self { minimum, maximum }
    }

    /// Creates metadata for a leaf widget.
    #[inline]
    pub const fn none() -> Self {
        Self::new(0, 0)
    }

    /// Creates metadata requiring exactly `count` children.
    #[inline]
    pub const fn exactly(count: u32) -> Self {
        Self::new(count, count)
    }

    /// Returns the minimum accepted child count.
    #[inline]
    pub const fn minimum(self) -> u32 {
        self.minimum
    }

    /// Returns the maximum accepted child count.
    #[inline]
    pub const fn maximum(self) -> u32 {
        self.maximum
    }
}

/// Complete static metadata for one portable widget schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableWidgetSchemaMetadata<'a> {
    widget: WidgetSchemaMetadata<'a>,
    properties: &'a [PropertySchemaMetadata<'a>],
    callbacks: &'a [CallbackSchemaMetadata<'a>],
    children: ChildCardinality,
}

impl<'a> PortableWidgetSchemaMetadata<'a> {
    /// Creates complete portable widget metadata.
    #[inline]
    pub const fn new(
        widget: WidgetSchemaMetadata<'a>,
        properties: &'a [PropertySchemaMetadata<'a>],
        callbacks: &'a [CallbackSchemaMetadata<'a>],
        children: ChildCardinality,
    ) -> Self {
        Self {
            widget,
            properties,
            callbacks,
            children,
        }
    }

    /// Returns the widget identity and version metadata.
    #[inline]
    pub const fn widget(self) -> WidgetSchemaMetadata<'a> {
        self.widget
    }

    /// Returns property metadata in declaration order.
    #[inline]
    pub const fn properties(self) -> &'a [PropertySchemaMetadata<'a>] {
        self.properties
    }

    /// Returns callback metadata in declaration order.
    #[inline]
    pub const fn callbacks(self) -> &'a [CallbackSchemaMetadata<'a>] {
        self.callbacks
    }

    /// Returns the accepted child-count bounds.
    #[inline]
    pub const fn children(self) -> ChildCardinality {
        self.children
    }
}

/// A borrowed, metadata-driven validator for portable widget documents.
///
/// The validator performs no registration or per-node allocation. It borrows a
/// complete metadata registry, verifies that registry during construction, and
/// then implements [`WidgetSchemaSupport`] for use with
/// [`WidgetDocumentView::validate_schemas`] or a delegating widget factory.
/// Validation covers only contracts represented by portable metadata. Numeric
/// or other host-specific value domains remain the responsibility of a later
/// validation layer.
#[derive(Clone, Copy, Debug)]
pub struct PortableWidgetSchemaValidator<'a> {
    schemas: &'a [PortableWidgetSchemaMetadata<'a>],
    additional_schemas: &'a [PortableWidgetSchemaMetadata<'a>],
}

impl<'a> PortableWidgetSchemaValidator<'a> {
    /// Creates a validator after checking the complete metadata registry.
    ///
    /// # Errors
    ///
    /// Returns [`PortableWidgetSchemaMetadataError`] when widget ranges,
    /// stable identities, property or callback registrations, child bounds, or
    /// custom blob contracts are inconsistent.
    pub fn new(
        schemas: &'a [PortableWidgetSchemaMetadata<'a>],
    ) -> Result<Self, PortableWidgetSchemaMetadataError<'a>> {
        validate_portable_widget_schema_metadata(schemas)?;
        Ok(Self {
            schemas,
            additional_schemas: &[],
        })
    }

    /// Creates a validator over two static registries without joining or
    /// allocating a temporary metadata table.
    ///
    /// The complete union is checked, including identities and version ranges
    /// that conflict across the two input slices. Lookup preserves the first
    /// slice's order but never uses order to resolve overlapping schemas.
    pub fn new_with_additional(
        schemas: &'a [PortableWidgetSchemaMetadata<'a>],
        additional_schemas: &'a [PortableWidgetSchemaMetadata<'a>],
    ) -> Result<Self, PortableWidgetSchemaMetadataError<'a>> {
        validate_portable_widget_schema_metadata(schemas)?;
        validate_portable_widget_schema_metadata(additional_schemas)?;
        for schema in schemas.iter().copied() {
            for additional in additional_schemas.iter().copied() {
                if schema == additional {
                    continue;
                }
                validate_widget_pair(schema.widget, additional.widget)?;
                validate_schema_pair(schema, additional)?;
            }
        }
        Ok(Self {
            schemas,
            additional_schemas,
        })
    }

    #[inline]
    fn schema(
        &self,
        widget_type: WidgetSchemaId,
        version: Version,
    ) -> Option<PortableWidgetSchemaMetadata<'a>> {
        self.schemas
            .iter()
            .chain(self.additional_schemas)
            .copied()
            .find(|schema| {
                let widget = schema.widget();
                widget.id() == widget_type
                    && version_at_least(version, widget.min_version())
                    && version_at_least(widget.max_version(), version)
            })
    }
}

impl WidgetSchemaSupport for PortableWidgetSchemaValidator<'_> {
    #[inline]
    fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
        self.schema(widget_type, schema).is_some()
    }

    fn validate_node(
        &self,
        document: &WidgetDocumentView<'_>,
        node_index: u32,
        node: WidgetNodeView<'_>,
    ) -> Result<(), ModelError> {
        let widget_type = node.widget_type();
        let Some(schema) = self.schema(widget_type, node.widget_schema()) else {
            return Err(ModelError::UnsupportedWidgetSchema {
                node: node_index,
                widget_type,
                schema: node.widget_schema(),
            });
        };

        validate_node_children(schema, node_index, &node)?;
        validate_node_properties(document, schema, node_index, &node)?;
        validate_node_callbacks(schema, node_index, &node)
    }
}

fn validate_node_children(
    schema: PortableWidgetSchemaMetadata<'_>,
    node_index: u32,
    node: &WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    let children = schema.children();
    let count = node.children().len() as u32;
    if count < children.minimum() || count > children.maximum() {
        return Err(ModelError::InvalidWidgetChildCount {
            node: node_index,
            widget_type: node.widget_type(),
            count,
            minimum: children.minimum(),
            maximum: children.maximum(),
        });
    }
    Ok(())
}

fn validate_node_properties(
    document: &WidgetDocumentView<'_>,
    schema: PortableWidgetSchemaMetadata<'_>,
    node_index: u32,
    node: &WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    for (index, property) in node.properties().enumerate() {
        if node
            .properties()
            .skip(index + 1)
            .any(|other| other.property_id() == property.property_id())
        {
            return Err(ModelError::DuplicateWidgetProperty {
                node: node_index,
                widget_type: node.widget_type(),
                property_id: property.property_id(),
            });
        }
    }

    for property in node.properties() {
        let Some(metadata) = schema
            .properties()
            .iter()
            .find(|metadata| metadata.id() == property.property_id())
        else {
            if property.is_optional() {
                continue;
            }
            return Err(ModelError::UnsupportedWidgetProperty {
                node: node_index,
                widget_type: node.widget_type(),
                property_id: property.property_id(),
            });
        };
        if !property_kind_matches(property.value(), metadata.value_kind()) {
            return Err(ModelError::InvalidWidgetPropertyType {
                node: node_index,
                widget_type: node.widget_type(),
                property_id: property.property_id(),
            });
        }
        if let (PropertyValue::BlobRef(index), Some(value_schema)) =
            (property.value(), metadata.value_schema())
        {
            let length = document.blob(index).map_or(0, <[u8]>::len);
            if length > value_schema.maximum_encoded_bytes() as usize {
                return Err(ModelError::InvalidWidgetPropertyValue {
                    node: node_index,
                    widget_type: node.widget_type(),
                    property_id: property.property_id(),
                });
            }
        }
    }

    for required in schema
        .properties()
        .iter()
        .filter(|property| property.presence() == PropertyPresence::Required)
    {
        if !node
            .properties()
            .any(|property| property.property_id() == required.id())
        {
            return Err(ModelError::MissingWidgetProperty {
                node: node_index,
                widget_type: node.widget_type(),
                property_id: required.id(),
            });
        }
    }
    Ok(())
}

fn validate_node_callbacks(
    schema: PortableWidgetSchemaMetadata<'_>,
    node_index: u32,
    node: &WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    for (index, callback) in node.callbacks().enumerate() {
        if node
            .callbacks()
            .skip(index + 1)
            .any(|other| other.callback_id() == callback.callback_id())
        {
            return Err(ModelError::DuplicateWidgetCallback {
                node: node_index,
                widget_type: node.widget_type(),
                callback_id: callback.callback_id(),
            });
        }
    }

    for callback in node.callbacks() {
        let Some(metadata) = schema.callbacks().iter().find(|metadata| {
            metadata.id() == callback.event_kind()
                && metadata.event_schema() == callback.event_schema()
        }) else {
            return Err(ModelError::UnsupportedWidgetCallback {
                node: node_index,
                widget_type: node.widget_type(),
                event_kind: callback.event_kind(),
            });
        };
        if let Some(version) = callback.async_schema() {
            let Some(async_schema) = metadata.async_schema() else {
                return Err(ModelError::UnsupportedAsyncCallback {
                    node: node_index,
                    widget_type: node.widget_type(),
                    event_kind: callback.event_kind(),
                    version,
                });
            };
            if async_schema.contract_version() != version {
                return Err(ModelError::UnsupportedAsyncCallback {
                    node: node_index,
                    widget_type: node.widget_type(),
                    event_kind: callback.event_kind(),
                    version,
                });
            }
        }
        let count = node
            .callbacks()
            .filter(|other| other.event_kind() == callback.event_kind())
            .count() as u32;
        if count > metadata.maximum_bindings() {
            return Err(ModelError::InvalidWidgetCallbackCount {
                node: node_index,
                widget_type: node.widget_type(),
                count,
                maximum: metadata.maximum_bindings(),
            });
        }
    }
    Ok(())
}

#[inline]
fn version_at_least(version: Version, minimum: Version) -> bool {
    (version.major(), version.minor()) >= (minimum.major(), minimum.minor())
}

#[inline]
fn property_kind_matches(value: PropertyValue, kind: PropertyValueKind) -> bool {
    matches!(
        (value, kind),
        (PropertyValue::Bool(_), PropertyValueKind::Bool)
            | (PropertyValue::I64(_), PropertyValueKind::I64)
            | (PropertyValue::F64(_), PropertyValueKind::F64)
            | (PropertyValue::Rgba(_), PropertyValueKind::Rgba)
            | (PropertyValue::StringRef(_), PropertyValueKind::StringRef)
            | (PropertyValue::BlobRef(_), PropertyValueKind::BlobRef)
    )
}

/// An inconsistency in complete portable widget-schema metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableWidgetSchemaMetadataError<'a> {
    /// Widget identity or version metadata is inconsistent.
    Widget(WidgetSchemaMetadataError<'a>),
    /// A property's identity does not equal the hash of its canonical name.
    PropertyIdentityNameMismatch {
        /// The identity declared by the property.
        declared: PropertyId,
        /// The identity derived from the canonical name.
        derived: PropertyId,
        /// The canonical property name.
        canonical_name: &'a str,
    },
    /// Two properties use the same identity in one widget schema.
    DuplicateProperty {
        /// The containing widget schema.
        widget: WidgetSchemaId,
        /// The duplicate property identity.
        property: PropertyId,
    },
    /// One property identity is registered by more than one widget schema.
    PropertyRegistrationConflict {
        /// The duplicate property identity.
        property: PropertyId,
        /// The first containing widget schema.
        first_widget: WidgetSchemaId,
        /// The second containing widget schema.
        second_widget: WidgetSchemaId,
    },
    /// A callback identity does not equal the hash of its canonical name.
    CallbackIdentityNameMismatch {
        /// The identity declared by the callback.
        declared: EventId,
        /// The identity derived from the canonical name.
        derived: EventId,
        /// The canonical callback name.
        canonical_name: &'a str,
    },
    /// Two callbacks use the same identity in one widget schema.
    DuplicateCallback {
        /// The containing widget schema.
        widget: WidgetSchemaId,
        /// The duplicate callback identity.
        callback: EventId,
    },
    /// One callback identity is registered by more than one widget schema.
    CallbackRegistrationConflict {
        /// The duplicate callback identity.
        callback: EventId,
        /// The first containing widget schema.
        first_widget: WidgetSchemaId,
        /// The second containing widget schema.
        second_widget: WidgetSchemaId,
    },
    /// A callback permits no bindings.
    EmptyCallbackSlot {
        /// The containing widget schema.
        widget: WidgetSchemaId,
        /// The unusable callback identity.
        callback: EventId,
    },
    /// Child-count bounds end before they begin.
    InvalidChildCardinality {
        /// The containing widget schema.
        widget: WidgetSchemaId,
        /// The minimum child count.
        minimum: u32,
        /// The maximum child count.
        maximum: u32,
    },
    /// A blob property has no value-type contract.
    MissingValueSchema {
        /// The containing widget schema.
        widget: WidgetSchemaId,
        /// The blob-backed property.
        property: PropertyId,
    },
    /// A scalar property unexpectedly declares blob value metadata.
    UnexpectedValueSchema {
        /// The containing widget schema.
        widget: WidgetSchemaId,
        /// The scalar property.
        property: PropertyId,
    },
    /// A value identity does not equal the hash of its canonical name.
    ValueIdentityNameMismatch {
        /// The identity declared by the value.
        declared: ValueTypeId,
        /// The identity derived from the canonical name.
        derived: ValueTypeId,
        /// The canonical value name.
        canonical_name: &'a str,
    },
    /// A custom value permits no encoded payload bytes.
    EmptyValuePayload {
        /// The custom value identity.
        value: ValueTypeId,
    },
}

impl fmt::Display for PortableWidgetSchemaMetadataError<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid portable widget schema metadata: {self:?}"
        )
    }
}

impl Error for PortableWidgetSchemaMetadataError<'_> {}

/// Validates complete widget, property, callback, and child metadata.
pub fn validate_portable_widget_schema_metadata<'a>(
    schemas: &[PortableWidgetSchemaMetadata<'a>],
) -> Result<(), PortableWidgetSchemaMetadataError<'a>> {
    for (schema_index, schema) in schemas.iter().copied().enumerate() {
        validate_widget_schema_metadata(core::slice::from_ref(&schema.widget))
            .map_err(PortableWidgetSchemaMetadataError::Widget)?;
        if schema.children.maximum < schema.children.minimum {
            return Err(PortableWidgetSchemaMetadataError::InvalidChildCardinality {
                widget: schema.widget.id(),
                minimum: schema.children.minimum,
                maximum: schema.children.maximum,
            });
        }
        validate_properties(schema)?;
        validate_callbacks(schema)?;

        for other in schemas.iter().copied().skip(schema_index + 1) {
            if schema == other {
                continue;
            }
            validate_widget_pair(schema.widget, other.widget)?;
            validate_schema_pair(schema, other)?;
        }
    }
    Ok(())
}

fn validate_properties(
    schema: PortableWidgetSchemaMetadata,
) -> Result<(), PortableWidgetSchemaMetadataError> {
    for (index, property) in schema.properties.iter().copied().enumerate() {
        let derived = PropertyId::from_canonical_name(property.canonical_name);
        if property.id != derived {
            return Err(
                PortableWidgetSchemaMetadataError::PropertyIdentityNameMismatch {
                    declared: property.id,
                    derived,
                    canonical_name: property.canonical_name,
                },
            );
        }
        match (property.value_kind, property.value_schema) {
            (PropertyValueKind::BlobRef, None) => {
                return Err(PortableWidgetSchemaMetadataError::MissingValueSchema {
                    widget: schema.widget.id(),
                    property: property.id,
                });
            }
            (PropertyValueKind::BlobRef, Some(value)) => validate_value_schema(value)?,
            (_, Some(_)) => {
                return Err(PortableWidgetSchemaMetadataError::UnexpectedValueSchema {
                    widget: schema.widget.id(),
                    property: property.id,
                });
            }
            (_, None) => {}
        }
        if schema.properties[index + 1..]
            .iter()
            .any(|other| other.id == property.id)
        {
            return Err(PortableWidgetSchemaMetadataError::DuplicateProperty {
                widget: schema.widget.id(),
                property: property.id,
            });
        }
    }
    Ok(())
}

fn validate_value_schema<'a>(
    value: ValueSchemaMetadata<'a>,
) -> Result<(), PortableWidgetSchemaMetadataError<'a>> {
    let derived = ValueTypeId::from_canonical_name(value.canonical_name);
    if value.id != derived {
        return Err(
            PortableWidgetSchemaMetadataError::ValueIdentityNameMismatch {
                declared: value.id,
                derived,
                canonical_name: value.canonical_name,
            },
        );
    }
    if value.maximum_encoded_bytes == 0 {
        return Err(PortableWidgetSchemaMetadataError::EmptyValuePayload { value: value.id });
    }
    Ok(())
}

fn validate_callbacks(
    schema: PortableWidgetSchemaMetadata,
) -> Result<(), PortableWidgetSchemaMetadataError> {
    for (index, callback) in schema.callbacks.iter().copied().enumerate() {
        let derived = EventId::from_canonical_name(callback.canonical_name);
        if callback.id != derived {
            return Err(
                PortableWidgetSchemaMetadataError::CallbackIdentityNameMismatch {
                    declared: callback.id,
                    derived,
                    canonical_name: callback.canonical_name,
                },
            );
        }
        if callback.maximum_bindings == 0 {
            return Err(PortableWidgetSchemaMetadataError::EmptyCallbackSlot {
                widget: schema.widget.id(),
                callback: callback.id,
            });
        }
        if schema.callbacks[index + 1..]
            .iter()
            .any(|other| other.id == callback.id)
        {
            return Err(PortableWidgetSchemaMetadataError::DuplicateCallback {
                widget: schema.widget.id(),
                callback: callback.id,
            });
        }
    }
    Ok(())
}

fn validate_widget_pair<'a>(
    first: WidgetSchemaMetadata<'a>,
    second: WidgetSchemaMetadata<'a>,
) -> Result<(), PortableWidgetSchemaMetadataError<'a>> {
    validate_widget_schema_metadata(&[first, second])
        .map_err(PortableWidgetSchemaMetadataError::Widget)
}

fn validate_schema_pair<'a>(
    first: PortableWidgetSchemaMetadata<'a>,
    second: PortableWidgetSchemaMetadata<'a>,
) -> Result<(), PortableWidgetSchemaMetadataError<'a>> {
    if first.widget.id() == second.widget.id() {
        return Ok(());
    }
    for property in first.properties {
        if second
            .properties
            .iter()
            .any(|other| other.id == property.id)
        {
            return Err(
                PortableWidgetSchemaMetadataError::PropertyRegistrationConflict {
                    property: property.id,
                    first_widget: first.widget.id(),
                    second_widget: second.widget.id(),
                },
            );
        }
    }
    for callback in first.callbacks {
        if second.callbacks.iter().any(|other| other.id == callback.id) {
            return Err(
                PortableWidgetSchemaMetadataError::CallbackRegistrationConflict {
                    callback: callback.id,
                    first_widget: first.widget.id(),
                    second_widget: second.widget.id(),
                },
            );
        }
    }
    Ok(())
}
