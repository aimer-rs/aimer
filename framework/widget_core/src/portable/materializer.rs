use std::error::Error;
use std::fmt;

use aimer_anteros::{
    PortableWidgetSchemaMetadata, PropertyId, PropertyValue, Version, WidgetDocumentView,
    WidgetNodeView, WidgetSchemaId,
};

use crate::AnyWidget;

/// Constructs one native widget from a schema-validated AWIR node.
///
/// Implementations are normally emitted by `#[derive(PortableWidget)]`. The
/// host must run portable schema validation before invoking this trait; value
/// decoding remains checked so a reflected Rust range mismatch cannot begin
/// native widget construction.
///
/// Implementors decode every property and validate child cardinality before
/// invoking a widget constructor. They return [`PortableMaterializeError`]
/// rather than partially constructing a widget when that final checked
/// conversion fails. A host adapter may then convert the returned [`AnyWidget`]
/// into its retained element representation.
///
/// `#[derive(PortableWidget)]` implements this trait for ordinary widgets whose
/// properties have [`PortableMaterializeProperty`] implementations and whose
/// required child uses Aimer's generic child-builder convention. Primitive
/// authors use `#[portable_widget(materializer = path)]` for custom native
/// construction.
pub trait PortableNativeWidget {
    /// Builds the native widget with already materialized children.
    fn materialize_widget(
        document: &WidgetDocumentView<'_>,
        node: WidgetNodeView<'_>,
        children: Vec<AnyWidget>,
    ) -> Result<AnyWidget, PortableMaterializeError>;
}

/// One derived native materializer linked into the permanent host.
///
/// The registration carries the exact schema metadata used for validation and
/// the generated checked constructor. Quiver validates the complete linked
/// registry before resolving any node, so linker order never selects between
/// conflicting schema versions.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct PortableNativeWidgetRegistration {
    schema: PortableWidgetSchemaMetadata<'static>,
    materialize: PortableNativeMaterializer,
}

/// Generated native construction function stored in the host linker registry.
#[doc(hidden)]
pub type PortableNativeMaterializer = fn(
    &WidgetDocumentView<'_>,
    WidgetNodeView<'_>,
    Vec<AnyWidget>,
) -> Result<AnyWidget, PortableMaterializeError>;

impl PortableNativeWidgetRegistration {
    /// Creates one registration from generated schema and construction code.
    #[doc(hidden)]
    #[inline]
    pub const fn new(
        schema: PortableWidgetSchemaMetadata<'static>,
        materialize: PortableNativeMaterializer,
    ) -> Self {
        Self {
            schema,
            materialize,
        }
    }

    /// Returns the registered portable schema.
    #[inline]
    pub const fn schema(self) -> PortableWidgetSchemaMetadata<'static> {
        self.schema
    }

    /// Returns the stable widget identity carried by this registration.
    #[inline]
    pub const fn widget_type(self) -> WidgetSchemaId {
        self.schema.widget().id()
    }

    /// Returns whether this registration can materialize one schema version.
    #[inline]
    pub const fn supports(self, widget_type: WidgetSchemaId, version: Version) -> bool {
        let widget = self.schema.widget();
        widget.id().value() == widget_type.value()
            && version_at_least(version, widget.min_version())
            && version_at_least(widget.max_version(), version)
    }

    /// Returns the checked native construction function.
    #[inline]
    pub const fn materialize(self) -> PortableNativeMaterializer {
        self.materialize
    }
}

#[inline]
const fn version_at_least(version: Version, minimum: Version) -> bool {
    version.major() > minimum.major()
        || (version.major() == minimum.major() && version.minor() >= minimum.minor())
}

/// Schemas contributed by derived native widgets linked into this host.
#[doc(hidden)]
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
#[linkme::distributed_slice]
pub static PORTABLE_NATIVE_WIDGET_SCHEMAS: [PortableWidgetSchemaMetadata<'static>] = [..];

/// Native constructors contributed by derived widgets linked into this host.
#[doc(hidden)]
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
#[linkme::distributed_slice]
pub static PORTABLE_NATIVE_WIDGET_REGISTRATIONS: [PortableNativeWidgetRegistration] = [..];

/// Returns all derived schemas collected on a linker-supported native host.
#[doc(hidden)]
#[inline]
pub fn linked_portable_native_widget_schemas(
) -> &'static [PortableWidgetSchemaMetadata<'static>] {
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "illumos",
    ))]
    {
        &PORTABLE_NATIVE_WIDGET_SCHEMAS
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "illumos",
    )))]
    {
        &[]
    }
}

/// Returns all derived constructors collected on a linker-supported native host.
#[doc(hidden)]
#[inline]
pub fn linked_portable_native_widget_registrations(
) -> &'static [PortableNativeWidgetRegistration] {
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "illumos",
    ))]
    {
        &PORTABLE_NATIVE_WIDGET_REGISTRATIONS
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "illumos",
    )))]
    {
        &[]
    }
}

/// Decodes one reflected Rust property from its validated AWIR representation.
///
/// Primitive implementations reject values outside the Rust type's range.
/// Custom blob-backed values intentionally have no blanket implementation;
/// widgets using them provide a manual materializer.
///
/// Implementations must not construct native resources. This conversion runs
/// before the generated materializer calls `Type::new()`, allowing an error to
/// leave the host tree untouched.
pub trait PortableMaterializeProperty: Sized {
    /// Converts one property value without constructing a native widget.
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError>;
}

/// Failure while decoding or constructing one derived native widget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortableMaterializeError {
    /// A required property was absent after schema validation.
    MissingProperty { property: PropertyId },
    /// A property had the wrong wire representation.
    InvalidPropertyType { property: PropertyId },
    /// A property could not be represented by its reflected Rust type.
    InvalidPropertyValue { property: PropertyId },
    /// A string or blob reference was not present in the validated document.
    InvalidPropertyReference { property: PropertyId, index: u32 },
    /// The generated constructor received the wrong number of native children.
    InvalidChildCount { expected: usize, actual: usize },
}

impl fmt::Display for PortableMaterializeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProperty { property } => {
                write!(formatter, "required portable property {property} is missing")
            }
            Self::InvalidPropertyType { property } => {
                write!(formatter, "portable property {property} has the wrong wire type")
            }
            Self::InvalidPropertyValue { property } => {
                write!(formatter, "portable property {property} is outside its Rust type range")
            }
            Self::InvalidPropertyReference { property, index } => write!(
                formatter,
                "portable property {property} references missing data record {index}",
            ),
            Self::InvalidChildCount { expected, actual } => write!(
                formatter,
                "derived portable materializer expected {expected} children but received {actual}",
            ),
        }
    }
}

impl Error for PortableMaterializeError {}

/// Decodes a required property after portable schema validation.
#[doc(hidden)]
pub fn required_materialized_property<T: PortableMaterializeProperty>(
    document: &WidgetDocumentView<'_>,
    node: &WidgetNodeView<'_>,
    property: PropertyId,
) -> Result<T, PortableMaterializeError> {
    let value = node
        .properties()
        .find(|candidate| candidate.property_id() == property)
        .ok_or(PortableMaterializeError::MissingProperty { property })?
        .value();
    T::from_awir(document, property, value)
}

/// Decodes an omission-based optional property after schema validation.
#[doc(hidden)]
pub fn optional_materialized_property<T: PortableMaterializeProperty>(
    document: &WidgetDocumentView<'_>,
    node: &WidgetNodeView<'_>,
    property: PropertyId,
) -> Result<Option<T>, PortableMaterializeError> {
    node.properties()
        .find(|candidate| candidate.property_id() == property)
        .map(|property_value| T::from_awir(document, property, property_value.value()))
        .transpose()
}

impl PortableMaterializeProperty for bool {
    fn from_awir(
        _document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        match value {
            PropertyValue::Bool(value) => Ok(value),
            _ => Err(PortableMaterializeError::InvalidPropertyType { property }),
        }
    }
}

macro_rules! integer_property {
    ($($type:ty),+ $(,)?) => {$ (
        impl PortableMaterializeProperty for $type {
            fn from_awir(
                _document: &WidgetDocumentView<'_>,
                property: PropertyId,
                value: PropertyValue,
            ) -> Result<Self, PortableMaterializeError> {
                let PropertyValue::I64(value) = value else {
                    return Err(PortableMaterializeError::InvalidPropertyType { property });
                };
                <$type>::try_from(value)
                    .map_err(|_| PortableMaterializeError::InvalidPropertyValue { property })
            }
        }
    )+};
}

integer_property!(i8, i16, i32, i64, u8, u16, u32);

macro_rules! float_property {
    ($($type:ty),+ $(,)?) => {$ (
        impl PortableMaterializeProperty for $type {
            fn from_awir(
                _document: &WidgetDocumentView<'_>,
                property: PropertyId,
                value: PropertyValue,
            ) -> Result<Self, PortableMaterializeError> {
                let PropertyValue::F64(value) = value else {
                    return Err(PortableMaterializeError::InvalidPropertyType { property });
                };
                let converted = value as $type;
                if value.is_finite() && converted.is_finite() {
                    Ok(converted)
                } else {
                    Err(PortableMaterializeError::InvalidPropertyValue { property })
                }
            }
        }
    )+};
}

float_property!(f32, f64);

impl PortableMaterializeProperty for String {
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let PropertyValue::StringRef(index) = value else {
            return Err(PortableMaterializeError::InvalidPropertyType { property });
        };
        document.string(index).map(str::to_owned).ok_or(
            PortableMaterializeError::InvalidPropertyReference { property, index },
        )
    }
}

impl PortableMaterializeProperty for aimer_color::prelude::Color {
    fn from_awir(
        _document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        match value {
            PropertyValue::Rgba(value) => Ok(Self::HexA(value)),
            _ => Err(PortableMaterializeError::InvalidPropertyType { property }),
        }
    }
}

impl PortableMaterializeProperty for aimer_attribute::Dimension {
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let PropertyValue::BlobRef(index) = value else {
            return Err(PortableMaterializeError::InvalidPropertyType { property });
        };
        let blob = document
            .blob(index)
            .ok_or(PortableMaterializeError::InvalidPropertyReference { property, index })?;
        if blob.len() != 6 || blob[0] != 1 {
            Err(PortableMaterializeError::InvalidPropertyValue { property })
        } else {
            let value = f32::from_le_bytes(blob[2..6].try_into().unwrap());
            if !value.is_finite() {
                return Err(PortableMaterializeError::InvalidPropertyValue { property });
            }
            match (blob[1], value) {
                (0, 0.0) => Ok(Self::Auto),
                (1, value) => Ok(Self::Px(value)),
                (2, value) => Ok(Self::Percent(value)),
                _ => Err(PortableMaterializeError::InvalidPropertyValue { property }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use aimer_anteros::{
        ChildCardinality, ModelLimits, PortableWidgetSchemaMetadata, Version, WidgetDocument,
        WidgetNode, WidgetProperty, WidgetSchemaId, WidgetSchemaMetadata,
    };

    use super::*;

    const LIMITS: ModelLimits = ModelLimits::new(1_024, 4, 8, 8).max_widget_depth(2);
    const PROPERTY: PropertyId = PropertyId::new(7);

    fn test_materializer(
        _document: &WidgetDocumentView<'_>,
        _node: WidgetNodeView<'_>,
        children: Vec<crate::AnyWidget>,
    ) -> Result<crate::AnyWidget, PortableMaterializeError> {
        Err(PortableMaterializeError::InvalidChildCount {
            expected: 0,
            actual: children.len(),
        })
    }

    #[test]
    fn native_registration_exposes_stable_identity_and_version_support() {
        let schema = PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::new(
                WidgetSchemaId::new(9),
                "aimer.widget:test::Registered",
                Version::new(1, 0),
                Version::new(1, 1),
            ),
            &[],
            &[],
            ChildCardinality::none(),
        );
        let registration = PortableNativeWidgetRegistration::new(schema, test_materializer);

        assert_eq!(registration.widget_type(), WidgetSchemaId::new(9));
        assert!(registration.supports(WidgetSchemaId::new(9), Version::new(1, 0)));
        assert!(registration.supports(WidgetSchemaId::new(9), Version::new(1, 1)));
        assert!(!registration.supports(WidgetSchemaId::new(9), Version::new(1, 2)));
        assert!(!registration.supports(WidgetSchemaId::new(10), Version::new(1, 0)));
    }

    const COMPILE_TIME_REGISTRATION: PortableNativeWidgetRegistration =
        PortableNativeWidgetRegistration::new(
            PortableWidgetSchemaMetadata::new(
                WidgetSchemaMetadata::new(
                    WidgetSchemaId::new(9),
                    "aimer.widget:test::Registered",
                    Version::new(1, 0),
                    Version::new(1, 1),
                ),
                &[],
                &[],
                ChildCardinality::none(),
            ),
            test_materializer,
        );

    const _: () = assert!(COMPILE_TIME_REGISTRATION.supports(
        WidgetSchemaId::new(9),
        Version::new(1, 1),
    ));

    #[test]
    fn checked_property_decoding_finishes_before_native_construction() {
        let properties = [WidgetProperty::new(PROPERTY, PropertyValue::I64(256))];
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))
            .properties(&properties)];
        let image = WidgetDocument::new(0, 0, 0, &nodes, &[], &[])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let node = document.node(0).unwrap();

        assert_eq!(
            required_materialized_property::<u8>(&document, &node, PROPERTY),
            Err(PortableMaterializeError::InvalidPropertyValue { property: PROPERTY }),
        );
    }

    #[test]
    fn string_and_omitted_optional_properties_decode_without_guessing() {
        let properties = [WidgetProperty::new(PROPERTY, PropertyValue::StringRef(0))];
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))
            .properties(&properties)];
        let image = WidgetDocument::new(0, 0, 0, &nodes, &["Aimer"], &[])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let node = document.node(0).unwrap();

        assert_eq!(
            required_materialized_property::<String>(&document, &node, PROPERTY).unwrap(),
            "Aimer",
        );
        assert_eq!(
            optional_materialized_property::<u32>(&document, &node, PropertyId::new(8)).unwrap(),
            None,
        );
    }

    #[test]
    fn malformed_type_reference_and_missing_property_are_structured_errors() {
        let properties = [WidgetProperty::new(PROPERTY, PropertyValue::Bool(true))];
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))
            .properties(&properties)];
        let image = WidgetDocument::new(0, 0, 0, &nodes, &[], &[])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let node = document.node(0).unwrap();

        assert_eq!(
            required_materialized_property::<u8>(&document, &node, PROPERTY),
            Err(PortableMaterializeError::InvalidPropertyType { property: PROPERTY }),
        );
        assert_eq!(
            required_materialized_property::<u8>(&document, &node, PropertyId::new(8)),
            Err(PortableMaterializeError::MissingProperty {
                property: PropertyId::new(8),
            }),
        );
        assert_eq!(
            String::from_awir(&document, PROPERTY, PropertyValue::StringRef(4)),
            Err(PortableMaterializeError::InvalidPropertyReference {
                property: PROPERTY,
                index: 4,
            }),
        );
    }

    #[test]
    fn reflected_float_color_and_dimension_conversion_remains_checked() {
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
        let dimension_px = [1, 1, 0, 0, 42, 66];
        let dimension_infinity = [1, 1, 0, 0, 128, 127];
        let image = WidgetDocument::new(
            0,
            0,
            0,
            &nodes,
            &[],
            &[&dimension_px, &dimension_infinity],
        )
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();

        assert_eq!(
            f32::from_awir(&document, PROPERTY, PropertyValue::F64(f64::MAX)),
            Err(PortableMaterializeError::InvalidPropertyValue { property: PROPERTY }),
        );
        assert_eq!(
            f64::from_awir(&document, PROPERTY, PropertyValue::F64(f64::NAN)),
            Err(PortableMaterializeError::InvalidPropertyValue { property: PROPERTY }),
        );
        assert_eq!(
            aimer_color::prelude::Color::from_awir(
                &document,
                PROPERTY,
                PropertyValue::I64(1),
            ),
            Err(PortableMaterializeError::InvalidPropertyType { property: PROPERTY }),
        );
        assert_eq!(
            aimer_color::prelude::Color::from_awir(
                &document,
                PROPERTY,
                PropertyValue::Rgba(0x112233FF),
            )
            .unwrap(),
            aimer_color::prelude::Color::HexA(0x112233FF),
        );
        assert_eq!(
            aimer_attribute::Dimension::from_awir(
                &document,
                PROPERTY,
                PropertyValue::BlobRef(0),
            )
            .unwrap(),
            aimer_attribute::Dimension::Px(42.5),
        );
        assert_eq!(
            aimer_attribute::Dimension::from_awir(
                &document,
                PROPERTY,
                PropertyValue::BlobRef(1),
            ),
            Err(PortableMaterializeError::InvalidPropertyValue { property: PROPERTY }),
        );
    }
}
