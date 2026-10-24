use aimer_anteros::{CallbackSchemaMetadata, PropertyValue};
use aimer_utils::callback::{Callback, CallbackExecutor, RawInnerCallback, VoidCallback};

use super::widget_ir::{
    PortableBuildContext, PortableBuildError, PortableCallback, SourceFingerprint,
};
use crate::key::Key;

/// Converts one callback field into the bounded callback registration owned by
/// a portable Widget IR node.
///
/// The derive uses the callback's reflected schema metadata for both the event
/// identity and event version. Implementors must either return one synchronous
/// callback registration or `None` for an unset optional callback. A
/// non-optional [`VoidCallback`] keeps its empty event slot by registering a
/// no-op callback, which preserves fixed callback routing contracts. Async
/// callbacks require an explicit reflected async schema and retain their future
/// in the generation-owned Venus scheduler.
pub trait PortableCallbackBinding {
    /// Binds this callback to one reflected event slot.
    fn bind_portable_callback(
        self,
        context: &PortableBuildContext,
        key: Option<&Key>,
        source: SourceFingerprint,
        metadata: CallbackSchemaMetadata<'static>,
        widget: &'static str,
    ) -> Result<Option<PortableCallback>, PortableBuildError>;
}

impl<F> PortableCallbackBinding for F
where
    F: Fn() + 'static,
{
    #[inline]
    fn bind_portable_callback(
        self,
        context: &PortableBuildContext,
        key: Option<&Key>,
        source: SourceFingerprint,
        metadata: CallbackSchemaMetadata<'static>,
        _widget: &'static str,
    ) -> Result<Option<PortableCallback>, PortableBuildError> {
        let callback_id = context.callback_id_for(key, source, metadata.id());
        Ok(Some(PortableCallback::new(
            metadata.id(),
            metadata.event_schema(),
            callback_id,
            move || {
                self();
                Ok(())
            },
        )))
    }
}

impl<F> PortableCallbackBinding for Option<F>
where
    F: PortableCallbackBinding,
{
    #[inline]
    fn bind_portable_callback(
        self,
        context: &PortableBuildContext,
        key: Option<&Key>,
        source: SourceFingerprint,
        metadata: CallbackSchemaMetadata<'static>,
        widget: &'static str,
    ) -> Result<Option<PortableCallback>, PortableBuildError> {
        self.map(|callback| {
            callback.bind_portable_callback(context, key, source, metadata, widget)
        })
        .transpose()
        .map(Option::flatten)
    }
}

impl<R> PortableCallbackBinding for Callback<(), R>
where
    R: 'static,
{
    #[inline]
    fn bind_portable_callback(
        self,
        context: &PortableBuildContext,
        key: Option<&Key>,
        source: SourceFingerprint,
        metadata: CallbackSchemaMetadata<'static>,
        widget: &'static str,
    ) -> Result<Option<PortableCallback>, PortableBuildError> {
        bind_raw_callback(
            self.raw(),
            context,
            key,
            source,
            metadata,
            widget,
            false,
        )
    }
}

impl PortableCallbackBinding for VoidCallback {
    #[inline]
    fn bind_portable_callback(
        self,
        context: &PortableBuildContext,
        key: Option<&Key>,
        source: SourceFingerprint,
        metadata: CallbackSchemaMetadata<'static>,
        widget: &'static str,
    ) -> Result<Option<PortableCallback>, PortableBuildError> {
        bind_raw_callback(
            self.raw(),
            context,
            key,
            source,
            metadata,
            widget,
            true,
        )
    }
}

fn bind_raw_callback<R>(
    raw: Option<&RawInnerCallback<(), R>>,
    context: &PortableBuildContext,
    key: Option<&Key>,
    source: SourceFingerprint,
    metadata: CallbackSchemaMetadata<'static>,
    widget: &'static str,
    emit_empty: bool,
) -> Result<Option<PortableCallback>, PortableBuildError>
where
    R: 'static,
{
    let body = match raw {
        Some(RawInnerCallback::Sync(body)) => Some(std::rc::Rc::clone(body)),
        Some(RawInnerCallback::Async(body)) => {
            let Some(async_schema) = metadata.async_schema() else {
                return Err(PortableBuildError::UnsupportedCallback {
                    widget,
                    event_kind: metadata.id(),
                    reason: "requires an explicit async callback schema",
                    source,
                });
            };
            let callback_id = context.callback_id_for(key, source, metadata.id());
            return Ok(Some(PortableCallback::new_async(
                metadata.id(),
                metadata.event_schema(),
                async_schema,
                callback_id,
                {
                    let body = std::rc::Rc::clone(body);
                    move || body(())
                },
            )));
        }
        None if emit_empty => None,
        None => return Ok(None),
    };
    let callback_id = context.callback_id_for(key, source, metadata.id());
    Ok(Some(PortableCallback::new(
        metadata.id(),
        metadata.event_schema(),
        callback_id,
        move || {
            if let Some(body) = body.as_ref() {
                let _ = body(());
            }
            Ok(())
        },
    )))
}

/// Encodes one semantic Rust property value into its reflected AWIR value.
///
/// Implementations must use the value representation declared by
/// [`PortableProperty::REFLECTION`](super::schema::PortableProperty::REFLECTION).
/// The trait consumes the value so owned strings and custom byte payloads can
/// move directly into the bounded build context. Optional lowering is a
/// caller concern: it should invoke this trait only for `Some` values.
///
/// Custom property implementations must serialize an explicit versioned wire
/// format and insert the resulting bytes with
/// [`PortableBuildContext::push_owned_blob`] or [`PortableBuildContext::push_blob`].
pub trait PortableEncodeProperty {
    /// Converts this value to one canonical AWIR property value.
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError>;
}

impl PortableEncodeProperty for bool {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        Ok(PropertyValue::Bool(self))
    }
}

macro_rules! signed_property_encoder {
    ($($type:ty),+ $(,)?) => {$ (
        impl PortableEncodeProperty for $type {
            #[inline]
            fn encode_property(
                self,
                _context: &mut PortableBuildContext,
            ) -> Result<PropertyValue, PortableBuildError> {
                Ok(PropertyValue::I64(self as i64))
            }
        }
    )+};
}

macro_rules! unsigned_property_encoder {
    ($($type:ty),+ $(,)?) => {$ (
        impl PortableEncodeProperty for $type {
            #[inline]
            fn encode_property(
                self,
                _context: &mut PortableBuildContext,
            ) -> Result<PropertyValue, PortableBuildError> {
                Ok(PropertyValue::I64(self as i64))
            }
        }
    )+};
}

signed_property_encoder!(i8, i16, i32, i64);
unsigned_property_encoder!(u8, u16, u32);

impl PortableEncodeProperty for f32 {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        if self.is_finite() {
            Ok(PropertyValue::F64(self as f64))
        } else {
            Err(PortableBuildError::NonFiniteFloat)
        }
    }
}

impl PortableEncodeProperty for f64 {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        if self.is_finite() {
            Ok(PropertyValue::F64(self))
        } else {
            Err(PortableBuildError::NonFiniteFloat)
        }
    }
}

impl PortableEncodeProperty for aimer_color::prelude::Color {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        // Color stores ARGB for the renderer; AWIR deliberately publishes
        // RGBA so the wire value is independent of that native storage order.
        Ok(PropertyValue::Rgba(self.as_u32().rotate_left(8)))
    }
}

impl PortableEncodeProperty for String {
    #[inline]
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        context.push_owned_string(self)
    }
}

impl PortableEncodeProperty for &str {
    #[inline]
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        context.push_string(self)
    }
}

impl PortableEncodeProperty for aimer_attribute::Dimension {
    #[inline]
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        const AUTO_TAG: u8 = 0;
        const PX_TAG: u8 = 1;
        const PERCENT_TAG: u8 = 2;

        let (tag, value) = match self {
            Self::Auto => (AUTO_TAG, 0.0),
            Self::Px(value) => (PX_TAG, value),
            Self::Percent(value) => (PERCENT_TAG, value),
        };
        if !value.is_finite() {
            return Err(PortableBuildError::NonFiniteFloat);
        }
        let mut bytes = Vec::with_capacity(6);
        bytes.push(1);
        bytes.push(tag);
        bytes.extend_from_slice(&value.to_le_bytes());
        context.push_owned_blob(bytes)
    }
}

#[cfg(test)]
mod tests {
    use aimer_anteros::{
        AsyncCallbackSchemaMetadata, ModelLimits, PropertyId, PropertyValue, Version, WidgetDocument, WidgetDocumentView,
        WidgetNode, WidgetProperty, WidgetSchemaId,
    };
    use aimer_attribute::Dimension;
    use aimer_color::prelude::Color;

    use super::*;
    use crate::portable::{
        PortableBuildError, PortableLimits, PortableMaterializeProperty, PortableProperty,
        PortableCallbackStart, PortableWidgetLimits, SourceFingerprint, StableId128,
    };

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            1,
            0,
            PortableWidgetLimits::new(4, 16, 4, 4, 64, 4_096)
                .with_max_blob_bytes(128),
            PortableLimits::new(4, 16, 64, 128, 4_096),
        )
        .unwrap()
    }

    #[test]
    fn primitive_literals_use_the_independently_specified_awir_values() {
        let mut context = context();

        assert_eq!(true.encode_property(&mut context).unwrap(), PropertyValue::Bool(true));
        assert_eq!((-7_i8).encode_property(&mut context).unwrap(), PropertyValue::I64(-7));
        assert_eq!((42_u32).encode_property(&mut context).unwrap(), PropertyValue::I64(42));
        assert_eq!(1.25_f32.encode_property(&mut context).unwrap(), PropertyValue::F64(1.25));
        assert_eq!(2.5_f64.encode_property(&mut context).unwrap(), PropertyValue::F64(2.5));
        assert_eq!(
            Color::Rgba(0x11, 0x22, 0x33, 0x44)
                .encode_property(&mut context)
                .unwrap(),
            PropertyValue::Rgba(0x11223344),
        );
        assert_eq!(
            Dimension::Px(12.5).encode_property(&mut context).unwrap(),
            PropertyValue::BlobRef(0),
        );
    }

    #[test]
    fn dimension_codec_preserves_each_explicit_unit() {
        let mut context = context();
        let dimensions = [
            Dimension::Auto,
            Dimension::Px(12.5),
            Dimension::Percent(37.5),
        ];
        let properties = dimensions
            .iter()
            .enumerate()
            .map(|(index, dimension)| {
                WidgetProperty::new(
                    PropertyId::new(index as u64 + 1),
                    (*dimension).encode_property(&mut context).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let node = context
            .push_node(
                WidgetSchemaId::new(1),
                Version::new(1, 0),
                None,
                SourceFingerprint::new(StableId128::from_u128(12)),
                &properties,
                &[],
            )
            .unwrap();
        let document = context.finish_document(node).unwrap();

        let image = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&image, document.model_limits()).unwrap();
        assert_eq!(view.blob(0), Some(&[1, 0, 0, 0, 0, 0][..]));
        assert_eq!(view.blob(1), Some(&[1, 1, 0, 0, 72, 65][..]));
        assert_eq!(view.blob(2), Some(&[1, 2, 0, 0, 22, 66][..]));
        for (index, expected) in dimensions.into_iter().enumerate() {
            assert_eq!(
                Dimension::from_awir(
                    &view,
                    PropertyId::new(index as u64 + 1),
                    PropertyValue::BlobRef(index as u32),
                )
                .unwrap(),
                expected,
            );
        }
    }

    #[test]
    fn strings_are_interned_through_the_context_for_borrowed_and_owned_values() {
        let mut context = context();

        let borrowed = "same".encode_property(&mut context).unwrap();
        let owned = String::from("same").encode_property(&mut context).unwrap();

        assert_eq!(borrowed, PropertyValue::StringRef(0));
        assert_eq!(owned, borrowed);
    }

    #[test]
    fn custom_values_can_publish_explicit_versioned_blobs_through_the_checked_context() {
        struct VersionedValue(&'static [u8]);

        impl PortableEncodeProperty for VersionedValue {
            fn encode_property(
                self,
                context: &mut PortableBuildContext,
            ) -> Result<PropertyValue, PortableBuildError> {
                let mut bytes = Vec::with_capacity(self.0.len() + 1);
                bytes.push(1);
                bytes.extend_from_slice(self.0);
                context.push_owned_blob(bytes)
            }
        }

        let mut context = context();
        let value = VersionedValue(&[1, 2, 3])
            .encode_property(&mut context)
            .unwrap();
        assert_eq!(value, PropertyValue::BlobRef(0));

        let node = context
            .push_node(
                WidgetSchemaId::new(1),
                Version::new(1, 0),
                None,
                SourceFingerprint::new(StableId128::from_u128(12)),
                &[WidgetProperty::new(PropertyId::new(10), value)],
                &[],
            )
            .unwrap();
        let graph = context.finish_graph(node).unwrap();
        assert_eq!(graph.blob(0), Some(&[1, 1, 2, 3][..]));
    }

    #[test]
    fn async_callback_lowers_to_a_guest_task_identity_and_runs_on_the_portable_scheduler() {
        use std::cell::Cell;
        use std::rc::Rc;

        let completed = Rc::new(Cell::new(false));
        let callback = VoidCallback::from_async({
            let completed = completed.clone();
            move || {
                let completed = completed.clone();
                async move { completed.set(true) }
            }
        });
        let metadata = CallbackSchemaMetadata::from_canonical_name(
            "aimer.event:test::async",
            Version::new(1, 0),
            1,
        )
        .with_async_schema(AsyncCallbackSchemaMetadata::new(
            Version::new(1, 0),
            2,
            64,
        ));
        let mut context = context();
        let source = SourceFingerprint::new(StableId128::from_u128(77));
        let binding = callback
            .bind_portable_callback(&context, None, source, metadata, "AsyncProbe")
            .unwrap()
            .expect("the async callback is present");
        let callback_id = context.callback_id_for(None, source, metadata.id());
        let node = context
            .push_node_with_callbacks(
                WidgetSchemaId::new(77),
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![binding],
                &[],
            )
            .unwrap();
        context.finish_document(node).unwrap();

        let start = context
            .callback_registry()
            .dispatch_start(callback_id, &mut context)
            .unwrap();
        assert!(matches!(start, PortableCallbackStart::Started { .. }));
        assert!(!completed.get());

        context.run_async_microtasks();
        assert!(completed.get());
    }

    #[test]
    fn invalid_numeric_and_dimension_values_fail_before_a_property_is_committed() {
        let mut context = context();

        assert!(f32::NAN.encode_property(&mut context).is_err());
        assert!(f64::INFINITY.encode_property(&mut context).is_err());
        assert!(Dimension::Px(f32::NAN).encode_property(&mut context).is_err());
        assert!(Dimension::Percent(f32::INFINITY).encode_property(&mut context).is_err());

        let annotated = context
            .encode_property(
                PropertyId::new(9),
                SourceFingerprint::new(StableId128::from_u128(11)),
                f32::NAN,
            )
            .unwrap_err();
        assert!(matches!(
            annotated,
            PortableBuildError::PropertyEncoding {
                property,
                source,
                cause,
                ..
            } if property == PropertyId::new(9)
                && source == SourceFingerprint::new(StableId128::from_u128(11))
                && matches!(*cause, PortableBuildError::NonFiniteFloat)
        ));

        let annotated = context
            .encode_property_named(
                PropertyId::new(9),
                "aimer.property:test::Container:decoration",
                SourceFingerprint::new(StableId128::from_u128(11)),
                f32::NAN,
            )
            .unwrap_err();
        let diagnostic = annotated.into_guest_diagnostic();
        assert_eq!(diagnostic.category(), aimer_anteros::GuestDiagnosticCategory::PropertyEncoding);
        assert_eq!(
            diagnostic.property(),
            Some("aimer.property:test::Container:decoration"),
        );
        assert!(diagnostic.message().contains("property codec error"));

        let node_properties = [WidgetProperty::new(
            aimer_anteros::PropertyId::new(1),
            PropertyValue::Bool(true),
        )];
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))
            .properties(&node_properties)];
        let image = WidgetDocument::new(1, 0, 0, &nodes, &[], &[])
            .encode(aimer_anteros::ModelLimits::new(4_096, 16, 64, 128))
            .unwrap();
        assert!(!image.is_empty());
    }

    #[test]
    fn every_reflected_owned_value_has_matching_encode_and_decode_contracts() {
        fn assert_bidirectional<T: PortableProperty
            + PortableEncodeProperty
            + PortableMaterializeProperty>()
        {
        }

        assert_bidirectional::<bool>();
        assert_bidirectional::<i8>();
        assert_bidirectional::<i16>();
        assert_bidirectional::<i32>();
        assert_bidirectional::<i64>();
        assert_bidirectional::<u8>();
        assert_bidirectional::<u16>();
        assert_bidirectional::<u32>();
        assert_bidirectional::<f32>();
        assert_bidirectional::<f64>();
        assert_bidirectional::<String>();
        assert_bidirectional::<Color>();
        assert_bidirectional::<Dimension>();

        let mut context = context();
        let dimension_value = Dimension::Px(12.5)
            .encode_property(&mut context)
            .unwrap();
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
        let dimension_blob = [1_u8, 1, 0, 0, 72, 65];
        let image = WidgetDocument::new(1, 0, 0, &nodes, &["portable"], &[&dimension_blob])
            .encode(ModelLimits::new(4_096, 16, 64, 128))
            .unwrap();
        let document = WidgetDocumentView::decode(
            &image,
            ModelLimits::new(4_096, 16, 64, 128),
        )
        .unwrap();
        let property = PropertyId::new(7);

        assert_eq!(
            bool::from_awir(&document, property, true.encode_property(&mut context).unwrap())
                .unwrap(),
            true,
        );
        assert_eq!(
            i8::from_awir(&document, property, (-7_i8).encode_property(&mut context).unwrap())
                .unwrap(),
            -7,
        );
        assert_eq!(
            i16::from_awir(
                &document,
                property,
                (-300_i16).encode_property(&mut context).unwrap(),
            )
            .unwrap(),
            -300,
        );
        assert_eq!(
            i32::from_awir(
                &document,
                property,
                (-70_000_i32).encode_property(&mut context).unwrap(),
            )
            .unwrap(),
            -70_000,
        );
        assert_eq!(
            i64::from_awir(
                &document,
                property,
                (-7_000_000_000_i64)
                    .encode_property(&mut context)
                    .unwrap(),
            )
            .unwrap(),
            -7_000_000_000,
        );
        assert_eq!(
            u8::from_awir(&document, property, 8_u8.encode_property(&mut context).unwrap())
                .unwrap(),
            8,
        );
        assert_eq!(
            u16::from_awir(
                &document,
                property,
                800_u16.encode_property(&mut context).unwrap(),
            )
            .unwrap(),
            800,
        );
        assert_eq!(
            u32::from_awir(
                &document,
                property,
                800_000_u32.encode_property(&mut context).unwrap(),
            )
            .unwrap(),
            800_000,
        );
        assert_eq!(
            f32::from_awir(
                &document,
                property,
                1.25_f32.encode_property(&mut context).unwrap(),
            )
            .unwrap(),
            1.25,
        );
        assert_eq!(
            f64::from_awir(
                &document,
                property,
                2.5_f64.encode_property(&mut context).unwrap(),
            )
            .unwrap(),
            2.5,
        );
        assert_eq!(
            String::from_awir(
                &document,
                property,
                PropertyValue::StringRef(0),
            )
            .unwrap(),
            "portable",
        );
        assert_eq!(
            Color::from_awir(
                &document,
                property,
                Color::Rgba(1, 2, 3, 4)
                    .encode_property(&mut context)
                    .unwrap(),
            )
            .unwrap(),
            Color::Rgba(1, 2, 3, 4),
        );
        assert_eq!(
            Dimension::from_awir(
                &document,
                property,
                dimension_value,
            )
            .unwrap(),
            Dimension::Px(12.5),
        );
    }
}
