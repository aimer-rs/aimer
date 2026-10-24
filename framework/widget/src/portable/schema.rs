use super::identity::{StableHasher, StableSchemaId, StableTypeId};

use aimer_anteros::{
    PortableWidgetSchemaMetadata, PropertyPresence, PropertyValueKind, ValueSchemaMetadata,
    Version,
};

pub(crate) const DIMENSION_VALUE_NAME: &str = "aimer.value:aimer_attribute::Dimension";
pub(crate) const DIMENSION_VALUE_VERSION: Version = Version::new(1, 0);
pub(crate) const DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 6;

/// The checked conversion between a Rust property type and its AWIR value.
///
/// This metadata is consumed by generated guest lowering and native
/// materialization. It describes semantic conversion rules rather than Rust
/// memory layout, so it remains stable across compiler versions and targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortablePropertyConversion {
    /// A canonical Boolean represented by AWIR `BOOL`.
    Bool,
    /// A signed Rust integer represented by AWIR `I64`.
    SignedInteger { minimum: i64, maximum: i64 },
    /// An unsigned Rust integer whose complete range fits in AWIR `I64`.
    UnsignedInteger { maximum: i64 },
    /// A Rust float widened to a finite AWIR `F64`.
    FiniteFloat { source_bits: u8 },
    /// A stable packed red-green-blue-alpha value.
    PackedRgba,
    /// UTF-8 text interned in the bounded AWIR string table.
    StringRef,
    /// A logical pixel value represented by a finite AWIR `F64`.
    LogicalPixels,
    /// A bounded, versioned custom value stored in the AWIR blob table.
    CustomValue,
}

/// Compile-time reflection for one Rust property type.
///
/// A reflection descriptor is generated once per Rust type. It is not runtime
/// reflection and is not copied into each widget document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortablePropertyReflection {
    value_kind: PropertyValueKind,
    presence: PropertyPresence,
    value_schema: Option<ValueSchemaMetadata<'static>>,
    conversion: PortablePropertyConversion,
}

impl PortablePropertyReflection {
    #[inline]
    /// Creates a reflection descriptor for a fixed AWIR value kind and
    /// conversion policy.
    pub const fn new(
        value_kind: PropertyValueKind,
        conversion: PortablePropertyConversion,
    ) -> Self {
        Self {
            value_kind,
            presence: PropertyPresence::Required,
            value_schema: None,
            conversion,
        }
    }

    /// Creates reflection for a bounded, versioned custom AWIR value.
    #[inline]
    pub const fn custom(value_schema: ValueSchemaMetadata<'static>) -> Self {
        Self {
            value_kind: PropertyValueKind::BlobRef,
            presence: PropertyPresence::Required,
            value_schema: Some(value_schema),
            conversion: PortablePropertyConversion::CustomValue,
        }
    }

    /// Creates reflection for UTF-8 text interned in the AWIR string table.
    #[inline]
    pub const fn string_ref() -> Self {
        Self::new(
            PropertyValueKind::StringRef,
            PortablePropertyConversion::StringRef,
        )
    }

    /// Changes this property to omission-based optional representation.
    #[inline]
    pub const fn optional(mut self) -> Self {
        self.presence = PropertyPresence::Optional;
        self
    }

    /// Returns the fixed AWIR value representation.
    #[inline]
    pub const fn value_kind(self) -> PropertyValueKind { self.value_kind }

    /// Returns whether every node must contain this property.
    #[inline]
    pub const fn presence(self) -> PropertyPresence { self.presence }

    /// Returns bounded custom-value metadata when the representation is a blob.
    #[inline]
    pub const fn value_schema(self) -> Option<ValueSchemaMetadata<'static>> { self.value_schema }

    /// Returns the checked Rust-to-AWIR conversion policy.
    #[inline]
    pub const fn conversion(self) -> PortablePropertyConversion { self.conversion }
}

/// Reflection contract used by generated portable widget schemas.
pub trait PortableProperty {
    /// The complete compile-time mapping from this Rust type to AWIR.
    const REFLECTION: PortablePropertyReflection;
}

/// Complete portable schema generated for one widget type.
pub trait PortableWidgetSchema {
    /// Static widget, property, callback, and child metadata.
    const SCHEMA: PortableWidgetSchemaMetadata<'static>;
}

macro_rules! signed_property {
    ($($type:ty),+ $(,)?) => {
        $(
            impl PortableProperty for $type {
                const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
                    PropertyValueKind::I64,
                    PortablePropertyConversion::SignedInteger {
                        minimum: <$type>::MIN as i64,
                        maximum: <$type>::MAX as i64,
                    },
                );
            }
        )+
    };
}

macro_rules! unsigned_property {
    ($($type:ty),+ $(,)?) => {
        $(
            impl PortableProperty for $type {
                const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
                    PropertyValueKind::I64,
                    PortablePropertyConversion::UnsignedInteger {
                        maximum: <$type>::MAX as i64,
                    },
                );
            }
        )+
    };
}

impl PortableProperty for bool {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::Bool,
        PortablePropertyConversion::Bool,
    );
}

signed_property!(i8, i16, i32, i64);
unsigned_property!(u8, u16, u32);

impl PortableProperty for f32 {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::F64,
        PortablePropertyConversion::FiniteFloat { source_bits: 32 },
    );
}

impl PortableProperty for f64 {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::F64,
        PortablePropertyConversion::FiniteFloat { source_bits: 64 },
    );
}

impl PortableProperty for aimer_attribute::Dimension {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::custom(
        ValueSchemaMetadata::from_canonical_name(
            DIMENSION_VALUE_NAME,
            DIMENSION_VALUE_VERSION,
            DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES,
        ),
    );
}

impl PortableProperty for String {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::StringRef,
        PortablePropertyConversion::StringRef,
    );
}

impl PortableProperty for aimer_color::prelude::Color {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::Rgba,
        PortablePropertyConversion::PackedRgba,
    );
}

impl PortableProperty for &str {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::StringRef,
        PortablePropertyConversion::StringRef,
    );
}

impl<T: PortableProperty> PortableProperty for Option<T> {
    const REFLECTION: PortablePropertyReflection = T::REFLECTION.optional();
}

/// Describes how generated portable code treats a source field.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FieldKind {
    /// The field is encoded and restored across generations.
    Retained = 1,
    /// The field is omitted and reconstructed from fresh configuration.
    Fresh = 2,
    /// The field cannot participate in active portable state.
    Unsupported = 3,
}

/// Static reflection metadata for one source field.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldDescriptor {
    name: &'static str,
    rust_type: &'static str,
    kind: FieldKind,
    stable_type_id: Option<StableTypeId>,
}

impl FieldDescriptor {
    /// Creates a field descriptor without requiring its field type to be portable.
    #[inline]
    pub const fn new(name: &'static str, rust_type: &'static str, kind: FieldKind) -> Self {
        Self {
            name,
            rust_type,
            kind,
            stable_type_id: None,
        }
    }

    /// Attaches the stable identity of a known portable field type.
    #[inline]
    pub const fn stable_type_id(mut self, stable_type_id: StableTypeId) -> Self {
        self.stable_type_id = Some(stable_type_id);
        self
    }

    /// Returns the source field name.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the canonical source spelling of the field type.
    #[inline]
    pub const fn rust_type(&self) -> &'static str {
        self.rust_type
    }

    /// Returns the generated retention classification.
    #[inline]
    pub const fn kind(&self) -> FieldKind {
        self.kind
    }

    /// Returns the stable field type identity when one is available.
    #[inline]
    pub const fn type_id(&self) -> Option<StableTypeId> {
        self.stable_type_id
    }
}

/// Static reflection metadata for a generated source type.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct TypeSchema {
    name: &'static str,
    type_id: StableTypeId,
    fields: &'static [FieldDescriptor],
}

impl TypeSchema {
    /// Creates a schema from canonical generated metadata.
    #[inline]
    pub const fn new(
        name: &'static str,
        type_id: StableTypeId,
        fields: &'static [FieldDescriptor],
    ) -> Self {
        Self {
            name,
            type_id,
            fields,
        }
    }

    /// Returns the canonical source type name.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the generated stable type identity.
    #[inline]
    pub const fn type_id(&self) -> StableTypeId {
        self.type_id
    }

    /// Returns source-order field descriptors.
    #[inline]
    pub const fn fields(&self) -> &'static [FieldDescriptor] {
        self.fields
    }

    /// Computes the deterministic version-one fingerprint for this schema.
    ///
    /// The image includes the type identity, source type name, ordered field
    /// names, canonical field type names, retention kinds, and optional stable
    /// field type identities. Merely naming an unsupported type is valid.
    pub const fn fingerprint(&self) -> StableSchemaId {
        let mut hasher = StableHasher::new();
        hasher.write_str("aimer.schema.v1");
        hasher.write_bytes(&self.type_id.to_bytes());
        hasher.write_str(self.name);
        hasher.write_u64(self.fields.len() as u64);
        let mut index = 0;
        while index < self.fields.len() {
            let field = &self.fields[index];
            hasher.write_str(field.name);
            hasher.write_str(field.rust_type);
            hasher.write_byte(field.kind as u8);
            match field.stable_type_id {
                Some(type_id) => {
                    hasher.write_byte(1);
                    hasher.write_bytes(&type_id.to_bytes());
                }
                None => hasher.write_byte(0),
            }
            index += 1;
        }
        hasher.finish()
    }
}

/// Reflection contract implemented by generated portable source types.
#[doc(hidden)]
pub trait AimerReflectionType {
    /// Stable identity derived from the canonical package/module/type path.
    const TYPE_ID: StableTypeId;

    /// Returns the static schema generated for this source type.
    fn schema() -> &'static TypeSchema;

    /// Returns the current deterministic schema fingerprint.
    #[inline]
    fn schema_id() -> StableSchemaId {
        Self::schema().fingerprint()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FieldDescriptor, FieldKind, PortableProperty, PortablePropertyConversion, TypeSchema,
    };
    use super::super::identity::StableId128;
    use aimer_anteros::{PropertyPresence, PropertyValueKind};

    const FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("count", "u32", FieldKind::Retained)
            .stable_type_id(StableId128::from_path("type", "u32")),
        FieldDescriptor::new("theme", "NativeTheme", FieldKind::Fresh),
        FieldDescriptor::new("socket", "NativeSocket", FieldKind::Unsupported),
    ];

    #[test]
    fn schema_fingerprint_is_deterministic_and_kind_sensitive() {
        let schema = TypeSchema::new("Counter", StableId128::from_path("type", "Counter"), FIELDS);
        const FINGERPRINT: StableId128 = TypeSchema::new(
            "Counter",
            StableId128::from_path("type", "Counter"),
            FIELDS,
        ).fingerprint();
        assert_eq!(schema.fingerprint(), FINGERPRINT);

        const CHANGED: &[FieldDescriptor] = &[
            FieldDescriptor::new("count", "u32", FieldKind::Fresh),
            FieldDescriptor::new("theme", "NativeTheme", FieldKind::Fresh),
            FieldDescriptor::new("socket", "NativeSocket", FieldKind::Unsupported),
        ];
        assert_ne!(
            schema.fingerprint(),
            TypeSchema::new("Counter", schema.type_id(), CHANGED).fingerprint()
        );
    }

    #[test]
    fn merely_describing_unsupported_fields_succeeds() {
        let schema = TypeSchema::new("Counter", StableId128::from_path("type", "Counter"), FIELDS);
        assert_ne!(schema.fingerprint(), StableId128::ZERO);
        assert_eq!(schema.fields()[2].kind(), FieldKind::Unsupported);
    }

    #[test]
    fn built_in_rust_types_publish_complete_awir_reflection() {
        assert_eq!(
            <bool as PortableProperty>::REFLECTION.conversion(),
            PortablePropertyConversion::Bool,
        );
        assert_eq!(
            <i8 as PortableProperty>::REFLECTION.conversion(),
            PortablePropertyConversion::SignedInteger {
                minimum: i8::MIN as i64,
                maximum: i8::MAX as i64,
            },
        );
        assert_eq!(
            <u32 as PortableProperty>::REFLECTION.conversion(),
            PortablePropertyConversion::UnsignedInteger {
                maximum: u32::MAX as i64,
            },
        );
        assert_eq!(
            <f32 as PortableProperty>::REFLECTION.conversion(),
            PortablePropertyConversion::FiniteFloat { source_bits: 32 },
        );
        assert_eq!(
            <String as PortableProperty>::REFLECTION.value_kind(),
            PropertyValueKind::StringRef,
        );
        assert_eq!(
            <aimer_color::prelude::Color as PortableProperty>::REFLECTION.conversion(),
            PortablePropertyConversion::PackedRgba,
        );
        assert_eq!(
            <aimer_attribute::Dimension as PortableProperty>::REFLECTION.conversion(),
            PortablePropertyConversion::CustomValue,
        );
        assert_eq!(
            <aimer_attribute::Dimension as PortableProperty>::REFLECTION.value_kind(),
            PropertyValueKind::BlobRef,
        );
    }

    #[test]
    fn option_preserves_the_inner_codec_and_omits_none() {
        let inner = <u16 as PortableProperty>::REFLECTION;
        let optional = <Option<u16> as PortableProperty>::REFLECTION;

        assert_eq!(optional.value_kind(), inner.value_kind());
        assert_eq!(optional.conversion(), inner.conversion());
        assert_eq!(optional.value_schema(), inner.value_schema());
        assert_eq!(optional.presence(), PropertyPresence::Optional);
    }
}
