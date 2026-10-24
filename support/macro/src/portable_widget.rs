use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, GenericParam, LitInt, LitStr, Path, PathArguments,
    Type, TypePath, parse_quote,
};

#[derive(Clone, Copy)]
enum ChildKind {
    Required,
    Optional,
    Collection,
}

struct WidgetOptions {
    identity: Option<String>,
    major: u16,
    minor: u16,
    explicit_version: bool,
    schema_only: bool,
    manual_lowering: bool,
    materializer: Option<Path>,
    validator: Option<Path>,
}

struct MaterializedProperty {
    name: syn::Ident,
    value_type: Type,
    optional: bool,
    canonical: TokenStream,
}

struct LoweredProperty {
    name: syn::Ident,
    value_type: Type,
    canonical: TokenStream,
    option_value: bool,
    default_optional: bool,
    schema_index: usize,
    source_discriminator: TokenStream,
}

struct ChildField {
    name: syn::Ident,
    field_type: Type,
    kind: ChildKind,
    source_discriminator: TokenStream,
}

struct ChildAttribute {
    optional: bool,
    discriminator: Option<u64>,
}

struct CallbackField {
    name: syn::Ident,
    field_type: Type,
    schema_index: usize,
}

pub(crate) fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            name,
            "PortableWidget requires a struct",
        ));
    };
    let fields = match &data.fields {
        Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "PortableWidget requires named fields or a unit struct",
            ));
        }
    };

    let options = widget_options(&input)?;
    let widget_identity = &options.identity;
    let major = options.major;
    let minor = options.minor;
    let widget_version = if options.explicit_version {
        quote!(::aimer_widget::portable::__anteros::Version::new(#major, #minor))
    } else {
        quote!(::aimer_widget::portable::__anteros::BUILTIN_WIDGET_SCHEMA_VERSION)
    };
    let widget_canonical = widget_identity.as_ref().map_or_else(
        || {
            quote!(concat!(
                "aimer.widget:",
                module_path!(),
                "::",
                stringify!(#name)
            ))
        },
        |identity| quote!(#identity),
    );
    let mut properties = Vec::new();
    let mut callbacks = Vec::new();
    let mut property_types: Vec<Type> = Vec::new();
    let mut materialized_properties = Vec::new();
    let mut lowered_properties = Vec::new();
    let mut callback_fields = Vec::new();
    let mut child_field = None;

    for field in fields {
        let field_name = field.ident.as_ref().expect("named field");
        let source_discriminator =
            field_source_discriminator(widget_identity.as_deref(), name, field_name);
        if has_attribute(field, "portable_skip") {
            continue;
        }
        if let Some(child) = child_attribute(field)? {
            let kind = if child.optional {
                ChildKind::Optional
            } else {
                ChildKind::Required
            };
            let source_discriminator = child.discriminator.map_or_else(
                || source_discriminator,
                |discriminator| quote!(#discriminator),
            );
            if child_field
                .replace(ChildField {
                    name: field_name.clone(),
                    field_type: field.ty.clone(),
                    kind,
                    source_discriminator,
                })
                .is_some()
            {
                return Err(syn::Error::new_spanned(
                    field,
                    "PortableWidget supports one structural child field",
                ));
            }
            continue;
        }
        if has_attribute(field, "portable_children") {
            if child_field
                .replace(ChildField {
                    name: field_name.clone(),
                    field_type: field.ty.clone(),
                    kind: ChildKind::Collection,
                    source_discriminator,
                })
                .is_some()
            {
                return Err(syn::Error::new_spanned(
                    field,
                    "PortableWidget supports one structural child field",
                ));
            }
            continue;
        }
        if let Some((
            callback_major,
            callback_minor,
            maximum_bindings,
            async_capable,
            async_major,
            async_minor,
            max_async_tasks,
            max_completion_bytes,
            max_callback_fuel,
            max_retained_resources,
        )) = callback_attribute(field)?
        {
            let callback_version = if (callback_major, callback_minor) == (1, 0) {
                quote!(::aimer_widget::portable::__anteros::BUILTIN_WIDGET_SCHEMA_VERSION)
            } else {
                quote!(::aimer_widget::portable::__anteros::Version::new(
                    #callback_major,
                    #callback_minor,
                ))
            };
            let canonical =
                field_canonical("aimer.event:", widget_identity.as_deref(), name, field_name);
            let callback_metadata = if async_capable {
                quote! {
                    ::aimer_widget::portable::__anteros::CallbackSchemaMetadata::from_canonical_name(
                        #canonical,
                        #callback_version,
                        #maximum_bindings,
                    ).with_async_schema(
                        ::aimer_widget::portable::__anteros::AsyncCallbackSchemaMetadata::new(
                            ::aimer_widget::portable::__anteros::Version::new(
                                #async_major,
                                #async_minor,
                            ),
                            #max_async_tasks,
                            #max_completion_bytes,
                        )
                        .with_maximum_callback_fuel(#max_callback_fuel)
                        .with_maximum_retained_resources(#max_retained_resources)
                    )
                }
            } else {
                quote! {
                    ::aimer_widget::portable::__anteros::CallbackSchemaMetadata::from_canonical_name(
                        #canonical,
                        #callback_version,
                        #maximum_bindings,
                    )
                }
            };
            callbacks.push(callback_metadata);
            callback_fields.push(CallbackField {
                name: field_name.clone(),
                field_type: field.ty.clone(),
                schema_index: callback_fields.len(),
            });
            continue;
        }

        let field_type = &field.ty;
        let default_optional = has_attribute(field, "portable_optional");
        if default_optional && optional_value_type(field_type).is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "`portable_optional` cannot be combined with an Option property",
            ));
        }
        property_types.push(field_type.clone());
        let canonical = field_canonical(
            "aimer.property:",
            widget_identity.as_deref(),
            name,
            field_name,
        );
        let reflection = if default_optional {
            quote!(<#field_type as ::aimer_widget::portable::PortableProperty>::REFLECTION.optional())
        } else {
            quote!(<#field_type as ::aimer_widget::portable::PortableProperty>::REFLECTION)
        };
        properties.push(quote! {
            ::aimer_widget::portable::__anteros::PropertySchemaMetadata::from_canonical_name(
                #canonical,
                #reflection.value_kind(),
            ).with_presence(
                #reflection.presence()
            ).with_optional_value_schema(
                #reflection.value_schema()
            )
        });
        let (option_value, value_type) = optional_value_type(field_type).map_or_else(
            || (false, field_type.clone()),
            |value| (true, value.clone()),
        );
        let optional = option_value || default_optional;
        lowered_properties.push(LoweredProperty {
            name: field_name.clone(),
            value_type: value_type.clone(),
            canonical: canonical.clone(),
            option_value,
            default_optional,
            schema_index: properties.len() - 1,
            source_discriminator,
        });
        materialized_properties.push(MaterializedProperty {
            name: field_name.clone(),
            value_type,
            optional,
            canonical,
        });
    }

    let child_kind = child_field.as_ref().map(|field| field.kind);
    let children = match child_kind {
        None => quote!(::aimer_widget::portable::__anteros::ChildCardinality::none()),
        Some(ChildKind::Required) => {
            quote!(::aimer_widget::portable::__anteros::ChildCardinality::exactly(1))
        }
        Some(ChildKind::Optional) => {
            quote!(::aimer_widget::portable::__anteros::ChildCardinality::new(
                0, 1
            ))
        }
        Some(ChildKind::Collection) => {
            quote!(::aimer_widget::portable::__anteros::ChildCardinality::new(
                0,
                u32::MAX
            ))
        }
    };
    let mut generics = input.generics.clone();
    for field_type in property_types {
        generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(#field_type: ::aimer_widget::portable::PortableProperty));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let native_materializer = if options.schema_only {
        schema_registration(
            &input.ident,
            quote!(#name #type_generics),
            input.generics.params.is_empty(),
        )
    } else {
        native_materializer(
            &input,
            &materialized_properties,
            child_kind,
            !callbacks.is_empty(),
            options.materializer.as_ref(),
        )?
    };
    let portable_lowering = if options.manual_lowering {
        TokenStream::new()
    } else {
        portable_lowering(
            &input,
            &lowered_properties,
            child_field.as_ref(),
            &callback_fields,
            options.validator.as_ref(),
        )
    };

    Ok(quote! {
        impl #impl_generics ::aimer_widget::portable::PortableWidgetSchema
            for #name #type_generics #where_clause
        {
            const SCHEMA: ::aimer_widget::portable::__anteros::PortableWidgetSchemaMetadata<'static> =
                ::aimer_widget::portable::__anteros::PortableWidgetSchemaMetadata::new(
                    ::aimer_widget::portable::__anteros::WidgetSchemaMetadata::from_canonical_name(
                        #widget_canonical,
                        #widget_version,
                        #widget_version,
                    ),
                    &[#(#properties),*],
                    &[#(#callbacks),*],
                    #children,
                );
        }

        #portable_lowering

        #native_materializer
    })
}

fn portable_lowering(
    input: &DeriveInput,
    properties: &[LoweredProperty],
    child: Option<&ChildField>,
    callbacks: &[CallbackField],
    validator: Option<&Path>,
) -> TokenStream {
    let name = &input.ident;
    let (base_impl_generics, type_generics, base_where_clause) = input.generics.split_for_impl();
    let mut feature_generics = input.generics.clone();
    feature_generics
        .make_where_clause()
        .predicates
        .push(parse_quote!(#name #type_generics: ::aimer_widget::Widget));
    for property in properties {
        let value_type = &property.value_type;
        feature_generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(
                #value_type: ::aimer_widget::portable::PortableProperty
            ));
        feature_generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(
                #value_type: ::aimer_widget::portable::PortableEncodeProperty
            ));
        if property.default_optional {
            feature_generics
                .make_where_clause()
                .predicates
                .push(parse_quote!(
                    #value_type: ::core::default::Default + ::core::cmp::PartialEq
                ));
        }
    }
    if let Some(child) = child {
        if let Some(child_type) = child_value_type(child) {
            feature_generics
                .make_where_clause()
                .predicates
                .push(parse_quote!(#child_type: ::aimer_widget::Widget));
        }
    }
    for callback in callbacks {
        let field_type = &callback.field_type;
        feature_generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(
                #field_type: ::aimer_widget::portable::PortableCallbackBinding
            ));
    }
    let (feature_impl_generics, _, feature_where_clause) = feature_generics.split_for_impl();
    let property_lowering = properties.iter().map(|property| {
        let field = &property.name;
        let schema_index = property.schema_index;
        let discriminator = &property.source_discriminator;
        let canonical = &property.canonical;
        let property_id = quote!(__aimer_portable_schema.properties()[#schema_index].id());
        let value = if property.option_value {
            quote! {
                if let ::std::option::Option::Some(__aimer_value) = self.#field {
                    let __aimer_value = ctx.encode_property_named(
                        #property_id,
                        #canonical,
                        source.child(#discriminator),
                        __aimer_value,
                    )?;
                    __aimer_portable_properties.push(
                        ::aimer_widget::portable::__anteros::WidgetProperty::new(
                            #property_id,
                            __aimer_value,
                        )
                        .optional(),
                    );
                }
            }
        } else if property.default_optional {
            quote! {
                if self.#field != ::core::default::Default::default() {
                    let __aimer_value = ctx.encode_property_named(
                        #property_id,
                        #canonical,
                        source.child(#discriminator),
                        self.#field,
                    )?;
                    __aimer_portable_properties.push(
                        ::aimer_widget::portable::__anteros::WidgetProperty::new(
                            #property_id,
                            __aimer_value,
                        )
                        .optional(),
                    );
                }
            }
        } else {
            quote! {
                let __aimer_value = ctx.encode_property_named(
                    #property_id,
                    #canonical,
                    source.child(#discriminator),
                    self.#field,
                )?;
                __aimer_portable_properties.push(
                    ::aimer_widget::portable::__anteros::WidgetProperty::new(
                        #property_id,
                        __aimer_value,
                    ),
                );
            }
        };
        value
    });

    let child_lowering = child.map_or_else(TokenStream::new, |child| {
        let field = &child.name;
        let discriminator = &child.source_discriminator;
        match child.kind {
            ChildKind::Required => {
                let child = if optional_value_type(&child.field_type).is_some() {
                    quote! {{
                        let __aimer_child = self.#field
                            .expect("a required portable child must be present");
                        ::aimer_widget::PortableWidget::to_portable_node(
                            __aimer_child,
                            ctx,
                            source.child(#discriminator),
                        )?
                    }}
                } else {
                    quote! {
                        ::aimer_widget::PortableWidget::to_portable_node(
                            self.#field,
                            ctx,
                            source.child(#discriminator),
                        )?
                    }
                };
                quote! {
                    __aimer_portable_children.push(#child);
                }
            }
            ChildKind::Optional => quote! {
                if let ::std::option::Option::Some(__aimer_child) = self.#field {
                    __aimer_portable_children.push(
                        ::aimer_widget::PortableWidget::to_portable_node(
                            __aimer_child,
                            ctx,
                            source.child(#discriminator),
                        )?,
                    );
                }
            },
            ChildKind::Collection => quote! {
                for (__aimer_index, __aimer_child) in self.#field.into_iter().enumerate() {
                    __aimer_portable_children.push(
                        ::aimer_widget::PortableWidget::to_portable_node(
                            __aimer_child,
                            ctx,
                            source.child(#discriminator).child(__aimer_index as u64),
                        )?,
                    );
                }
            },
        }
    });

    let callback_lowering = callbacks.iter().map(|callback| {
        let field = &callback.name;
        let schema_index = callback.schema_index;
        quote! {
            if let ::std::option::Option::Some(__aimer_callback) =
                ::aimer_widget::portable::PortableCallbackBinding::bind_portable_callback(
                    self.#field,
                    ctx,
                    __aimer_portable_key.as_ref(),
                    source,
                    __aimer_portable_schema.callbacks()[#schema_index],
                    stringify!(#name),
                )?
            {
                __aimer_portable_callbacks.push(__aimer_callback);
            }
        }
    });

    let push = if callbacks.is_empty() {
        quote! {
            ctx.push_node(
                __aimer_portable_schema.widget().id(),
                __aimer_portable_schema.widget().min_version(),
                __aimer_portable_key.as_ref(),
                source,
                &__aimer_portable_properties,
                &__aimer_portable_children,
            )
        }
    } else {
        quote! {
            ctx.push_node_with_callbacks(
                __aimer_portable_schema.widget().id(),
                __aimer_portable_schema.widget().min_version(),
                __aimer_portable_key.as_ref(),
                source,
                &__aimer_portable_properties,
                __aimer_portable_callbacks,
                &__aimer_portable_children,
            )
        }
    };
    let callback_storage = if callbacks.is_empty() {
        TokenStream::new()
    } else {
        quote! {
            let mut __aimer_portable_callbacks = ::std::vec::Vec::new();
        }
    };
    let validation = validator.map_or_else(
        TokenStream::new,
        |validator| quote!(#validator(&self, ctx, source)?;),
    );

    quote! {
        #[cfg(not(feature = "portable-guest"))]
        impl #base_impl_generics ::aimer_widget::PortableWidget
            for #name #type_generics #base_where_clause
        {}

        #[cfg(feature = "portable-guest")]
        impl #feature_impl_generics ::aimer_widget::PortableWidget
            for #name #type_generics #feature_where_clause
        {
            fn to_portable_node(
                self,
                ctx: &mut ::aimer_widget::portable::PortableBuildContext,
                source: ::aimer_widget::portable::SourceFingerprint,
            ) -> ::std::result::Result<
                ::aimer_widget::portable::PortableNodeId,
                ::aimer_widget::portable::PortableBuildError,
            > {
                #validation
                let __aimer_portable_key = ::aimer_widget::Widget::key(&self);
                let __aimer_portable_schema =
                    <Self as ::aimer_widget::portable::PortableWidgetSchema>::SCHEMA;
                let mut __aimer_portable_properties: ::std::vec::Vec<
                    ::aimer_widget::portable::__anteros::WidgetProperty,
                > = ::std::vec::Vec::new();
                let mut __aimer_portable_children: ::std::vec::Vec<
                    ::aimer_widget::portable::PortableNodeId,
                > = ::std::vec::Vec::new();
                #(#property_lowering)*
                __aimer_portable_properties
                    .sort_unstable_by_key(|property| property.property_id());
                #child_lowering
                #callback_storage
                #(#callback_lowering)*
                #push
            }
        }
    }
}

fn child_value_type(child: &ChildField) -> Option<Type> {
    match child.kind {
        ChildKind::Required => optional_value_type(&child.field_type)
            .cloned()
            .or_else(|| Some(child.field_type.clone())),
        ChildKind::Optional => optional_value_type(&child.field_type).cloned(),
        ChildKind::Collection => collection_value_type(&child.field_type),
    }
}

fn collection_value_type(field_type: &Type) -> Option<Type> {
    let Type::Path(TypePath { qself: None, path }) = field_type else {
        return None;
    };
    let segment = path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(value_type) => Some(value_type.clone()),
        _ => None,
    }
}

fn native_materializer(
    input: &DeriveInput,
    properties: &[MaterializedProperty],
    child: Option<ChildKind>,
    has_callbacks: bool,
    materializer: Option<&Path>,
) -> syn::Result<TokenStream> {
    let name = &input.ident;
    if let Some(materializer) = materializer {
        let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
        let registration = native_registration(
            name,
            quote!(#name #type_generics),
            input.generics.params.is_empty(),
        );
        return Ok(quote! {
            impl #impl_generics ::aimer_widget::portable::PortableNativeWidget
                for #name #type_generics #where_clause
            {
                fn materialize_widget(
                    document: &::aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
                    node: ::aimer_widget::portable::__anteros::WidgetNodeView<'_>,
                    children: ::std::vec::Vec<::aimer_widget::AnyWidget>,
                ) -> ::std::result::Result<
                    ::aimer_widget::AnyWidget,
                    ::aimer_widget::portable::PortableMaterializeError,
                > {
                    #materializer(document, node, children)
                }
            }

            #registration
        });
    }

    if has_callbacks {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "automatic portable materialization does not support callbacks; use `#[portable_widget(materializer = path)]`",
        ));
    }
    if matches!(child, Some(ChildKind::Optional)) {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "automatic portable materialization does not support an optional child; use `#[portable_widget(materializer = path)]`",
        ));
    }
    if matches!(child, Some(ChildKind::Collection)) {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "automatic portable materialization does not support `#[portable_children]`; use `#[portable_widget(materializer = path)]`",
        ));
    }

    let (mut impl_generics, target_type) = materializer_target(input, child)?;
    let register = impl_generics.params.is_empty();
    for property in properties {
        let value_type = &property.value_type;
        impl_generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(
                #value_type: ::aimer_widget::portable::PortableMaterializeProperty
            ));
    }
    let (impl_generics, _, where_clause) = impl_generics.split_for_impl();
    let registration = native_registration(name, target_type.clone(), register);

    let decoders = properties
        .iter()
        .enumerate()
        .map(|(property_index, property)| {
            let field = &property.name;
            let local = format_ident!(
                "__aimer_materialized_{}",
                field.unraw(),
                span = field.span(),
            );
            let property_id = format_ident!(
                "__AIMER_MATERIALIZED_PROPERTY_ID_{}",
                property_index,
                span = field.span(),
            );
            let value_type = &property.value_type;
            let canonical = &property.canonical;
            if property.optional {
                quote! {
                    const #property_id: ::aimer_widget::portable::__anteros::PropertyId =
                        ::aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
                            #canonical,
                        );
                    let #local: ::std::option::Option<#value_type> =
                        ::aimer_widget::portable::optional_materialized_property(
                            document,
                            &node,
                            #property_id,
                        )?;
                }
            } else {
                quote! {
                    const #property_id: ::aimer_widget::portable::__anteros::PropertyId =
                        ::aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
                            #canonical,
                        );
                    let #local: #value_type =
                        ::aimer_widget::portable::required_materialized_property(
                            document,
                            &node,
                            #property_id,
                        )?;
                }
            }
        });
    let builders = properties.iter().map(|property| {
        let field = &property.name;
        let local = format_ident!(
            "__aimer_materialized_{}",
            field.unraw(),
            span = field.span(),
        );
        if property.optional {
            quote! {
                let widget = if let ::std::option::Option::Some(#local) = #local {
                    widget.#field(#local)
                } else {
                    widget
                };
            }
        } else {
            quote!(let widget = widget.#field(#local);)
        }
    });
    let child_validation = if matches!(child, Some(ChildKind::Required)) {
        quote! {
            if children.len() != 1 {
                return ::std::result::Result::Err(
                    ::aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                        expected: 1,
                        actual: children.len(),
                    },
                );
            }
            let child = children.into_iter().next().expect("validated one portable child");
        }
    } else {
        quote! {
            if !children.is_empty() {
                return ::std::result::Result::Err(
                    ::aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                        expected: 0,
                        actual: children.len(),
                    },
                );
            }
        }
    };
    let apply_child = if matches!(child, Some(ChildKind::Required)) {
        quote!(let widget = widget.child(child);)
    } else {
        TokenStream::new()
    };

    Ok(quote! {
        impl #impl_generics ::aimer_widget::portable::PortableNativeWidget
            for #target_type #where_clause
        {
            fn materialize_widget(
                document: &::aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
                node: ::aimer_widget::portable::__anteros::WidgetNodeView<'_>,
                children: ::std::vec::Vec<::aimer_widget::AnyWidget>,
            ) -> ::std::result::Result<
                ::aimer_widget::AnyWidget,
                ::aimer_widget::portable::PortableMaterializeError,
            > {
                #child_validation
                #(#decoders)*
                let widget = <#target_type>::new();
                #(#builders)*
                #apply_child
                ::std::result::Result::Ok(::aimer_widget::Widget::boxed(widget))
            }
        }

        #registration
    })
}

fn native_registration(name: &syn::Ident, target_type: TokenStream, register: bool) -> TokenStream {
    if !register {
        return TokenStream::new();
    }
    let schema_registration = schema_registration(name, target_type.clone(), true);
    let materializer_registration = format_ident!(
        "__AIMER_PORTABLE_NATIVE_MATERIALIZER_FOR_{}",
        name.unraw(),
        span = name.span(),
    );
    quote! {
        #schema_registration

        #[cfg(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "illumos",
        ))]
        #[aimer_widget::portable::__linkme::distributed_slice(
            aimer_widget::portable::materializer::PORTABLE_NATIVE_WIDGET_REGISTRATIONS
        )]
        #[linkme(crate = aimer_widget::portable::__linkme)]
        #[allow(non_upper_case_globals)]
        static #materializer_registration:
            ::aimer_widget::portable::PortableNativeWidgetRegistration =
            ::aimer_widget::portable::PortableNativeWidgetRegistration::new(
                <#target_type as ::aimer_widget::portable::PortableWidgetSchema>::SCHEMA,
                <#target_type as ::aimer_widget::portable::PortableNativeWidget>::materialize_widget,
            );
    }
}

fn schema_registration(name: &syn::Ident, target_type: TokenStream, register: bool) -> TokenStream {
    if !register {
        return TokenStream::new();
    }
    let schema_registration = format_ident!(
        "__AIMER_PORTABLE_NATIVE_SCHEMA_FOR_{}",
        name.unraw(),
        span = name.span(),
    );
    quote! {
        #[cfg(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "illumos",
        ))]
        #[aimer_widget::portable::__linkme::distributed_slice(
            aimer_widget::portable::materializer::PORTABLE_NATIVE_WIDGET_SCHEMAS
        )]
        #[linkme(crate = aimer_widget::portable::__linkme)]
        #[allow(non_upper_case_globals)]
        static #schema_registration:
            ::aimer_widget::portable::__anteros::PortableWidgetSchemaMetadata<'static> =
            <#target_type as ::aimer_widget::portable::PortableWidgetSchema>::SCHEMA;
    }
}

fn materializer_target(
    input: &DeriveInput,
    child: Option<ChildKind>,
) -> syn::Result<(syn::Generics, TokenStream)> {
    let name = &input.ident;
    let Some(ChildKind::Required) = child else {
        let generics = input.generics.clone();
        let (_, type_generics, _) = input.generics.split_for_impl();
        return Ok((generics, quote!(#name #type_generics)));
    };
    let Data::Struct(data) = &input.data else {
        unreachable!("the derive already requires a struct");
    };
    let child_field = data
        .fields
        .iter()
        .find(|field| child_attribute(field).ok().flatten().is_some())
        .expect("required child field was recorded");
    let Type::Path(TypePath { qself: None, path }) = &child_field.ty else {
        return Err(syn::Error::new_spanned(
            &child_field.ty,
            "automatic portable materialization requires a required child type to be a type-generic identifier; use `materializer = path`",
        ));
    };
    let Some(child_ident) = path.get_ident() else {
        return Err(syn::Error::new_spanned(
            &child_field.ty,
            "automatic portable materialization requires a required child type to be a type-generic identifier; use `materializer = path`",
        ));
    };
    let matching_parameters = input
        .generics
        .type_params()
        .filter(|parameter| parameter.ident == *child_ident)
        .count();
    if matching_parameters != 1 {
        return Err(syn::Error::new_spanned(
            &child_field.ty,
            "automatic portable materialization requires an unambiguous child type generic; use `materializer = path`",
        ));
    }

    let replacement: Type = input
        .generics
        .type_params()
        .find(|parameter| parameter.ident == *child_ident)
        .and_then(|parameter| parameter.default.clone())
        .unwrap_or_else(|| parse_quote!(::aimer_widget::RequiredChild));
    let mut impl_generics = input.generics.clone();
    TypeReplacement {
        ident: child_ident,
        replacement: &replacement,
    }
    .visit_generics_mut(&mut impl_generics);
    impl_generics.params = impl_generics.params.into_iter().filter(|parameter| {
        !matches!(parameter, GenericParam::Type(parameter) if parameter.ident == *child_ident)
    }).collect();
    let arguments = input
        .generics
        .params
        .iter()
        .map(|parameter| match parameter {
            GenericParam::Lifetime(parameter) => {
                let lifetime = &parameter.lifetime;
                quote!(#lifetime)
            }
            GenericParam::Type(parameter) if parameter.ident == *child_ident => {
                quote!(#replacement)
            }
            GenericParam::Type(parameter) => {
                let ident = &parameter.ident;
                quote!(#ident)
            }
            GenericParam::Const(parameter) => {
                let ident = &parameter.ident;
                quote!(#ident)
            }
        });
    Ok((impl_generics, quote!(#name <#(#arguments),*>)))
}

struct TypeReplacement<'a> {
    ident: &'a syn::Ident,
    replacement: &'a Type,
}

impl VisitMut for TypeReplacement<'_> {
    fn visit_type_mut(&mut self, node: &mut Type) {
        if let Type::Path(TypePath { qself: None, path }) = node
            && path.is_ident(self.ident)
        {
            *node = self.replacement.clone();
            return;
        }
        visit_mut::visit_type_mut(self, node);
    }
}

fn optional_value_type(field_type: &Type) -> Option<&Type> {
    let Type::Path(TypePath { qself: None, path }) = field_type else {
        return None;
    };
    let segment = path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(value_type) => Some(value_type),
        _ => None,
    }
}

fn field_canonical(
    domain: &str,
    widget_identity: Option<&str>,
    widget: &syn::Ident,
    field: &syn::Ident,
) -> TokenStream {
    if let Some(widget_identity) = widget_identity {
        let widget_identity = widget_identity
            .strip_prefix("aimer.widget:")
            .unwrap_or(widget_identity);
        let canonical = format!("{domain}{widget_identity}:{field}");
        quote!(#canonical)
    } else {
        quote!(concat!(
            #domain,
            module_path!(),
            "::",
            stringify!(#widget),
            ":",
            stringify!(#field)
        ))
    }
}

fn field_source_discriminator(
    widget_identity: Option<&str>,
    widget: &syn::Ident,
    field: &syn::Ident,
) -> TokenStream {
    let canonical = field_canonical("aimer.source:", widget_identity, widget, field);
    quote!(::aimer_widget::portable::__anteros::stable_schema_hash64(#canonical))
}

fn widget_options(input: &DeriveInput) -> syn::Result<WidgetOptions> {
    let mut identity = None;
    let mut version = (1, 0);
    let mut explicit_version = false;
    let mut schema_only = false;
    let mut manual_lowering = false;
    let mut materializer = None;
    let mut validator = None;
    for attribute in &input.attrs {
        if !attribute.path().is_ident("portable_widget") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                let value: LitStr = meta.value()?.parse()?;
                let value = value.value();
                identity = Some(if value.starts_with("aimer.widget:") {
                    value
                } else {
                    format!("aimer.widget:{value}")
                });
                return Ok(());
            }
            if meta.path.is_ident("version") {
                let value: LitStr = meta.value()?.parse()?;
                version = parse_version(&value)?;
                explicit_version = true;
                return Ok(());
            }
            if meta.path.is_ident("schema_only") {
                schema_only = true;
                return Ok(());
            }
            if meta.path.is_ident("manual_lowering") {
                manual_lowering = true;
                return Ok(());
            }
            if meta.path.is_ident("materializer") {
                materializer = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("validate") {
                validator = Some(meta.value()?.parse()?);
                return Ok(());
            }
            Err(meta.error(
                "expected `id`, `version`, `schema_only`, `manual_lowering`, `materializer`, or `validate`",
            ))
        })?;
    }
    if schema_only && materializer.is_some() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`schema_only` cannot be combined with `materializer`",
        ));
    }
    Ok(WidgetOptions {
        identity,
        major: version.0,
        minor: version.1,
        explicit_version,
        schema_only,
        manual_lowering,
        materializer,
        validator,
    })
}

fn parse_version(value: &LitStr) -> syn::Result<(u16, u16)> {
    let version = value.value();
    let Some((major, minor)) = version.split_once('.') else {
        return Err(syn::Error::new_spanned(
            value,
            "version must be `major.minor`",
        ));
    };
    let major = major
        .parse()
        .map_err(|_| syn::Error::new_spanned(value, "invalid major version"))?;
    let minor = minor
        .parse()
        .map_err(|_| syn::Error::new_spanned(value, "invalid minor version"))?;
    Ok((major, minor))
}

fn has_attribute(field: &syn::Field, name: &str) -> bool {
    field
        .attrs
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn child_attribute(field: &syn::Field) -> syn::Result<Option<ChildAttribute>> {
    let Some(attribute) = field
        .attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("portable_child"))
    else {
        return Ok(None);
    };
    let mut optional = false;
    let mut discriminator = None;
    if matches!(attribute.meta, syn::Meta::List(_)) {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("optional") {
                optional = true;
                Ok(())
            } else if meta.path.is_ident("discriminator") {
                let value: LitInt = meta.value()?.parse()?;
                discriminator = Some(value.base10_parse()?);
                Ok(())
            } else {
                Err(meta.error("expected `optional` or `discriminator`"))
            }
        })?;
    }
    Ok(Some(ChildAttribute {
        optional,
        discriminator,
    }))
}

fn callback_attribute(
    field: &syn::Field,
) -> syn::Result<Option<(u16, u16, u32, bool, u16, u16, u32, u32, u32, u32)>> {
    let Some(attribute) = field
        .attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("portable_callback"))
    else {
        return Ok(None);
    };
    let mut version = (1, 0);
    let mut maximum_bindings = 1;
    let mut async_capable = false;
    let mut async_version = (1, 0);
    let mut max_async_tasks = 64;
    let mut max_completion_bytes = 4_096;
    let mut max_callback_fuel = u32::MAX;
    let mut max_retained_resources = u32::MAX;
    if matches!(attribute.meta, syn::Meta::List(_)) {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("async") {
                async_capable = true;
                return Ok(());
            }
            if meta.path.is_ident("version") {
                let value: LitStr = meta.value()?.parse()?;
                version = parse_version(&value)?;
                return Ok(());
            }
            if meta.path.is_ident("async_version") {
                let value: LitStr = meta.value()?.parse()?;
                async_version = parse_version(&value)?;
                async_capable = true;
                return Ok(());
            }
            if meta.path.is_ident("max_bindings") {
                let value: LitInt = meta.value()?.parse()?;
                maximum_bindings = value.base10_parse()?;
                return Ok(());
            }
            if meta.path.is_ident("max_async_tasks") {
                let value: LitInt = meta.value()?.parse()?;
                max_async_tasks = value.base10_parse()?;
                async_capable = true;
                return Ok(());
            }
            if meta.path.is_ident("max_completion_bytes") {
                let value: LitInt = meta.value()?.parse()?;
                max_completion_bytes = value.base10_parse()?;
                async_capable = true;
                return Ok(());
            }
            if meta.path.is_ident("max_callback_fuel") {
                let value: LitInt = meta.value()?.parse()?;
                max_callback_fuel = value.base10_parse()?;
                async_capable = true;
                return Ok(());
            }
            if meta.path.is_ident("max_retained_resources") {
                let value: LitInt = meta.value()?.parse()?;
                max_retained_resources = value.base10_parse()?;
                async_capable = true;
                return Ok(());
            }
            Err(meta.error(
                "expected `version`, `max_bindings`, `async`, `async_version`, `max_async_tasks`, `max_completion_bytes`, `max_callback_fuel`, or `max_retained_resources`",
            ))
        })?;
    }
    Ok(Some((
        version.0,
        version.1,
        maximum_bindings,
        async_capable,
        async_version.0,
        async_version.1,
        max_async_tasks,
        max_completion_bytes,
        max_callback_fuel,
        max_retained_resources,
    )))
}
