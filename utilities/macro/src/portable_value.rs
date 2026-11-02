use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DataEnum, DataStruct, DeriveInput, Fields, GenericArgument, Ident, Index, LitInt, LitStr,
    Member, PathArguments, Type, parse_quote,
};

#[derive(Clone)]
struct FieldSpec {
    member: Member,
    binding: Ident,
    ty: Type,
    name: String,
    order: u32,
    index: usize,
}

struct VariantSpec {
    ident: Ident,
    name: String,
    tag: u32,
    fields: Vec<FieldSpec>,
}

struct ValueOptions {
    canonical_name: Option<String>,
    major: u16,
    minor: u16,
    maximum_encoded_bytes: u32,
    max_depth: usize,
    max_entries: usize,
    max_string_bytes: usize,
    max_key_bytes: usize,
    max_value_bytes: usize,
    max_reconstruction_work: usize,
}

pub(crate) fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let options = value_options(&input)?;
    let major = options.major;
    let minor = options.minor;
    let maximum_encoded_bytes = options.maximum_encoded_bytes;
    let max_depth = options.max_depth;
    let max_entries = options.max_entries;
    let max_string_bytes = options.max_string_bytes;
    let max_key_bytes = options.max_key_bytes;
    let max_value_bytes = options.max_value_bytes;
    let max_reconstruction_work = options.max_reconstruction_work;
    let canonical_name = options.canonical_name.as_ref().map_or_else(
        || {
            quote!(concat!(
                "aimer.value:",
                module_path!(),
                "::",
                stringify!(#name)
            ))
        },
        |value| quote!(#value),
    );
    let schema = quote! {
        ::aimer_widget::portable::__anteros::ValueSchemaMetadata::from_canonical_name(
            #canonical_name,
            ::aimer_widget::portable::__anteros::Version::new(#major, #minor),
            #maximum_encoded_bytes,
        )
    };

    let (fields, variants, encode, decode) = match &input.data {
        Data::Struct(data) => {
            let fields = struct_fields(data, name)?;
            let encode = encode_struct(name, &fields);
            let decode = decode_struct(name, &fields);
            (fields, Vec::new(), encode, decode)
        }
        Data::Enum(data) => {
            let variants = enum_variants(data, name)?;
            let encode = encode_enum(name, &variants);
            let decode = decode_enum(name, &variants);
            (Vec::new(), variants, encode, decode)
        }
        Data::Union(_data) => {
            return Err(syn::Error::new_spanned(
                name,
                "PortableValue does not support unions; use a named struct or explicitly tagged enum",
            ));
        }
    };

    let field_types = fields
        .iter()
        .map(|field| field.ty.clone())
        .chain(
            variants
                .iter()
                .flat_map(|variant| variant.fields.iter().map(|field| field.ty.clone())),
        )
        .collect::<Vec<_>>();
    reject_raw_unordered_collections(&field_types)?;

    let target = type_target(&input);
    let encode_impl = codec_impl(
        &input,
        &field_types,
        quote!(::aimer_widget::portable::PortableEncode),
        encode,
    );
    let decode_impl = codec_impl(
        &input,
        &field_types,
        quote!(::aimer_widget::portable::PortableDecode),
        decode,
    );
    let both_bounds = field_types.iter().flat_map(|ty| {
        [
            parse_quote!(#ty: ::aimer_widget::portable::PortableEncode),
            parse_quote!(#ty: ::aimer_widget::portable::PortableDecode),
        ]
    });
    let (impl_generics, _type_generics, where_clause) = generics_with_bounds(&input, both_bounds);
    let field_metadata = fields.iter().map(|field| {
        let name = &field.name;
        let order = field.order;
        quote!(::aimer_widget::portable::PortableValueField::new(#name, #order))
    });
    let enum_field_metadata = variants.iter().flat_map(|variant| {
        let variant_name = &variant.name;
        variant.fields.iter().map(move |field| {
            let name = format!("{variant_name}::{}", field.name);
            let order = field.order;
            quote!(::aimer_widget::portable::PortableValueField::new(#name, #order))
        })
    });
    let variant_metadata = variants.iter().map(|variant| {
        let name = &variant.name;
        let tag = variant.tag;
        quote!(::aimer_widget::portable::PortableValueVariant::new(#name, #tag))
    });
    let value_impl = quote! {
        impl #impl_generics ::aimer_widget::portable::PortableValue for #target #where_clause {
            const SCHEMA: ::aimer_widget::portable::__anteros::ValueSchemaMetadata<'static> = #schema;
            const FIELDS: &'static [::aimer_widget::portable::PortableValueField] = &[#(#field_metadata,)* #(#enum_field_metadata,)*];
            const VARIANTS: &'static [::aimer_widget::portable::PortableValueVariant] = &[#(#variant_metadata,)*];
            const MAX_DEPTH: usize = #max_depth;
            const MAX_ENTRIES: usize = #max_entries;
            const MAX_STRING_BYTES: usize = #max_string_bytes;
            const MAX_KEY_BYTES: usize = #max_key_bytes;
            const MAX_VALUE_BYTES: usize = #max_value_bytes;
            const MAX_RECONSTRUCTION_WORK: usize = #max_reconstruction_work;
        }
    };

    let property_impl = quote! {
        impl #impl_generics ::aimer_widget::portable::PortableProperty for #target #where_clause {
            const REFLECTION: ::aimer_widget::portable::PortablePropertyReflection =
                ::aimer_widget::portable::PortablePropertyReflection::custom(
                    <Self as ::aimer_widget::portable::PortableValue>::SCHEMA,
                );
        }

        impl #impl_generics ::aimer_widget::portable::PortableMaterializeProperty for #target #where_clause {
            fn from_awir(
                document: &::aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
                property: ::aimer_widget::portable::__anteros::PropertyId,
                value: ::aimer_widget::portable::__anteros::PropertyValue,
            ) -> ::std::result::Result<Self, ::aimer_widget::portable::PortableMaterializeError> {
                let ::aimer_widget::portable::__anteros::PropertyValue::BlobRef(index) = value else {
                    return ::std::result::Result::Err(
                        ::aimer_widget::portable::PortableMaterializeError::InvalidPropertyType { property },
                    );
                };
                let bytes = document.blob(index).ok_or(
                    ::aimer_widget::portable::PortableMaterializeError::InvalidPropertyReference {
                        property,
                        index,
                    },
                )?;
                <Self as ::aimer_widget::portable::PortableValue>::decode_value(
                    bytes,
                    <Self as ::aimer_widget::portable::PortableValue>::SCHEMA.version(),
                )
                .map_err(|_| ::aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                    property,
                })
            }
        }

        #[cfg(feature = "portable-guest")]
        impl #impl_generics ::aimer_widget::portable::PortableEncodeProperty for #target #where_clause {
            fn encode_property(
                self,
                context: &mut ::aimer_widget::portable::PortableBuildContext,
            ) -> ::std::result::Result<
                ::aimer_widget::portable::__anteros::PropertyValue,
                ::aimer_widget::portable::PortableBuildError,
            > {
                let bytes = <Self as ::aimer_widget::portable::PortableValue>::encode_value(&self)
                    .map_err(|error| ::aimer_widget::portable::PortableBuildError::ValueCodec {
                        rust_type: ::core::any::type_name::<Self>(),
                        message: error.to_string(),
                    })?;
                context.push_owned_blob(bytes)
            }
        }
    };

    Ok(quote! {
        #encode_impl
        #decode_impl
        #value_impl
        #property_impl
    })
}

fn type_target(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;
    let (_, type_generics, _) = input.generics.split_for_impl();
    quote!(#name #type_generics)
}

fn codec_impl(
    input: &DeriveInput,
    field_types: &[Type],
    trait_path: TokenStream,
    body: TokenStream,
) -> TokenStream {
    let (impl_generics, type_generics, where_clause) = generics_with_bounds(
        input,
        field_types.iter().map(|ty| parse_quote!(#ty: #trait_path)),
    );
    let name = &input.ident;
    quote! {
        impl #impl_generics #trait_path for #name #type_generics #where_clause {
            #body
        }
    }
}

fn generics_with_bounds(
    input: &DeriveInput,
    bounds: impl IntoIterator<Item = syn::WherePredicate>,
) -> (TokenStream, TokenStream, TokenStream) {
    let mut generics = input.generics.clone();
    generics.make_where_clause().predicates.extend(bounds);
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    (
        quote!(#impl_generics),
        quote!(#type_generics),
        quote!(#where_clause),
    )
}

fn encode_struct(_name: &Ident, fields: &[FieldSpec]) -> TokenStream {
    let count = fields.len();
    let calls = fields.iter().map(|field| {
        let member = &field.member;
        quote!(::aimer_widget::portable::PortableEncode::encode(&self.#member, encoder)?;)
    });
    quote! {
        fn encode(
            &self,
            encoder: &mut ::aimer_widget::portable::Encoder<'_>,
        ) -> ::std::result::Result<(), ::aimer_widget::portable::EncodeError> {
            encoder.nested(|encoder| {
                encoder.claim_entries(#count)?;
                #(#calls)*
                Ok(())
            })
        }
    }
}

fn decode_struct(name: &Ident, fields: &[FieldSpec]) -> TokenStream {
    let locals = fields.iter().enumerate().map(|(index, field)| {
        let local = format_ident!("__aimer_value_field_{index}");
        let ty = &field.ty;
        quote!(let #local = <#ty as ::aimer_widget::portable::PortableDecode>::decode(decoder)?;)
    });
    let local_names = fields
        .iter()
        .enumerate()
        .map(|(index, _)| format_ident!("__aimer_value_field_{index}"))
        .collect::<Vec<_>>();
    let source_locals = fields
        .iter()
        .enumerate()
        .map(|(source_index, _)| {
            let wire_index = fields
                .iter()
                .position(|field| field.index == source_index)
                .expect("every field has one source index");
            local_names[wire_index].clone()
        })
        .collect::<Vec<_>>();
    let construct = match fields.first().map(|field| &field.member) {
        None => quote!(Self),
        Some(Member::Named(_)) => {
            let assignments = fields.iter().map(|field| {
                let member = &field.member;
                let local = &source_locals[field.index];
                quote!(#member: #local)
            });
            quote!(#name { #(#assignments,)* })
        }
        Some(Member::Unnamed(_)) => quote!(#name(#(#source_locals),*)),
    };
    let count = fields.len();
    quote! {
        fn decode(
            decoder: &mut ::aimer_widget::portable::Decoder<'_>,
        ) -> ::std::result::Result<Self, ::aimer_widget::portable::DecodeError> {
            decoder.nested(|decoder| {
                decoder.claim_entries(#count)?;
                #(#locals)*
                Ok(#construct)
            })
        }
    }
}

fn encode_enum(_name: &Ident, variants: &[VariantSpec]) -> TokenStream {
    let arms = variants.iter().map(|variant| {
        let tag = variant.tag;
        let pattern = variant_pattern(variant);
        let calls = variant.fields.iter().map(|field| {
            let binding = &field.binding;
            quote!(::aimer_widget::portable::PortableEncode::encode(#binding, encoder)?;)
        });
        let count = variant.fields.len() + 1;
        quote! {
            #pattern => encoder.nested(|encoder| {
                encoder.claim_entries(#count)?;
                ::aimer_widget::portable::PortableEncode::encode(&(#tag as u32), encoder)?;
                #(#calls)*
                Ok(())
            }),
        }
    });
    quote! {
        fn encode(
            &self,
            encoder: &mut ::aimer_widget::portable::Encoder<'_>,
        ) -> ::std::result::Result<(), ::aimer_widget::portable::EncodeError> {
            match self {
                #(#arms)*
            }
        }
    }
}

fn decode_enum(_name: &Ident, variants: &[VariantSpec]) -> TokenStream {
    let arms = variants.iter().map(|variant| {
        let tag = variant.tag;
        let locals = variant.fields.iter().enumerate().map(|(index, field)| {
            let local = &field.binding;
            let ty = &field.ty;
            let _ = index;
            quote!(let #local = <#ty as ::aimer_widget::portable::PortableDecode>::decode(decoder)?;)
        });
        let construct = variant_construct(variant);
        let field_count = variant.fields.len();
        quote!(#tag => {
            decoder.claim_entries(#field_count)?;
            #(#locals)*
            Ok(#construct)
        },)
    });
    quote! {
        fn decode(
            decoder: &mut ::aimer_widget::portable::Decoder<'_>,
        ) -> ::std::result::Result<Self, ::aimer_widget::portable::DecodeError> {
            decoder.nested(|decoder| {
                decoder.claim_entries(1)?;
                let tag = <u32 as ::aimer_widget::portable::PortableDecode>::decode(decoder)?;
                match tag {
                    #(#arms)*
                    _ => Err(::aimer_widget::portable::DecodeError::InvalidEnumTag(tag)),
                }
            })
        }
    }
}

fn variant_pattern(variant: &VariantSpec) -> TokenStream {
    let ident = &variant.ident;
    match variant.fields.first().map(|field| &field.member) {
        None => quote!(Self::#ident),
        Some(Member::Named(_)) => {
            let fields = variant.fields.iter().map(|field| {
                let Member::Named(member) = &field.member else {
                    unreachable!()
                };
                let binding = &field.binding;
                quote!(#member: #binding)
            });
            quote!(Self::#ident { #(#fields,)* })
        }
        Some(Member::Unnamed(_)) => {
            let bindings = source_order_bindings(&variant.fields);
            quote!(Self::#ident(#(#bindings),*))
        }
    }
}

fn variant_construct(variant: &VariantSpec) -> TokenStream {
    let ident = &variant.ident;
    match variant.fields.first().map(|field| &field.member) {
        None => quote!(Self::#ident),
        Some(Member::Named(_)) => {
            let fields = variant.fields.iter().map(|field| {
                let Member::Named(member) = &field.member else {
                    unreachable!()
                };
                let binding = &field.binding;
                quote!(#member: #binding)
            });
            quote!(Self::#ident { #(#fields,)* })
        }
        Some(Member::Unnamed(_)) => {
            let bindings = source_order_bindings(&variant.fields);
            quote!(Self::#ident(#(#bindings),*))
        }
    }
}

fn struct_fields(data: &DataStruct, type_name: &Ident) -> syn::Result<Vec<FieldSpec>> {
    let fields = match &data.fields {
        Fields::Named(fields) => fields
            .named
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let member = Member::Named(field.ident.clone().expect("named field"));
                let default_name = field.ident.as_ref().expect("named field").to_string();
                field_spec(field, member, default_name, index as u32, index)
            })
            .collect::<syn::Result<Vec<_>>>(),
        Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(index, field)| {
                field_spec(
                    field,
                    Member::Unnamed(Index::from(index)),
                    index.to_string(),
                    index as u32,
                    index,
                )
            })
            .collect::<syn::Result<Vec<_>>>(),
        Fields::Unit => Ok(Vec::new()),
    }?;
    validate_field_order(fields, type_name)
}

fn enum_variants(data: &DataEnum, _type_name: &Ident) -> syn::Result<Vec<VariantSpec>> {
    let mut variants = Vec::new();
    for variant in &data.variants {
        let (name, tag) = variant_metadata(variant)?;
        let fields = match &variant.fields {
            Fields::Named(fields) => fields
                .named
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let ident = field.ident.clone().expect("named field");
                    field_spec(
                        field,
                        Member::Named(ident.clone()),
                        ident.to_string(),
                        index as u32,
                        index,
                    )
                })
                .collect::<syn::Result<Vec<_>>>(),
            Fields::Unnamed(fields) => fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    field_spec(
                        field,
                        Member::Unnamed(Index::from(index)),
                        index.to_string(),
                        index as u32,
                        index,
                    )
                })
                .collect::<syn::Result<Vec<_>>>(),
            Fields::Unit => Ok(Vec::new()),
        }?;
        let fields = validate_field_order(fields, &variant.ident)?;
        variants.push(VariantSpec {
            ident: variant.ident.clone(),
            name,
            tag: tag.ok_or_else(|| {
                syn::Error::new_spanned(
                    &variant.ident,
                    "every PortableValue enum variant requires an explicit `tag = ...`",
                )
            })?,
            fields,
        });
    }
    for (index, variant) in variants.iter().enumerate() {
        if variants[..index]
            .iter()
            .any(|other| other.tag == variant.tag)
        {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "PortableValue enum discriminant tags must be unique",
            ));
        }
    }
    variants.sort_unstable_by_key(|variant| variant.tag);
    Ok(variants)
}

fn field_spec(
    field: &syn::Field,
    member: Member,
    default_name: String,
    default_order: u32,
    index: usize,
) -> syn::Result<FieldSpec> {
    let (name, order) = field_metadata(&field.attrs, default_name, default_order)?;
    Ok(FieldSpec {
        member,
        binding: format_ident!("__aimer_value_field_{index}"),
        ty: field.ty.clone(),
        name,
        order,
        index,
    })
}

fn source_order_bindings(fields: &[FieldSpec]) -> Vec<&Ident> {
    let mut bindings = fields
        .iter()
        .map(|field| (field.index, &field.binding))
        .collect::<Vec<_>>();
    bindings.sort_unstable_by_key(|(index, _)| *index);
    bindings.into_iter().map(|(_, binding)| binding).collect()
}

fn validate_field_order(
    mut fields: Vec<FieldSpec>,
    type_name: &Ident,
) -> syn::Result<Vec<FieldSpec>> {
    for (index, field) in fields.iter().enumerate() {
        if fields[..index]
            .iter()
            .any(|other| other.order == field.order)
        {
            return Err(syn::Error::new_spanned(
                type_name,
                "PortableValue field orders must be unique",
            ));
        }
        if fields[..index].iter().any(|other| other.name == field.name) {
            return Err(syn::Error::new_spanned(
                type_name,
                "PortableValue field names must be unique",
            ));
        }
    }
    fields.sort_unstable_by_key(|field| field.order);
    Ok(fields)
}

fn field_metadata(
    attrs: &[syn::Attribute],
    mut name: String,
    mut order: u32,
) -> syn::Result<(String, u32)> {
    for attribute in attrs {
        if !attribute.path().is_ident("portable_value")
            && !attribute.path().is_ident("portable_field")
        {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                name = meta.value()?.parse::<LitStr>()?.value();
                return Ok(());
            }
            if meta.path.is_ident("order") {
                order = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            Err(meta.error("expected `name` or `order` on a PortableValue field"))
        })?;
    }
    if name.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "PortableValue field name must not be empty",
        ));
    }
    Ok((name, order))
}

fn variant_metadata(variant: &syn::Variant) -> syn::Result<(String, Option<u32>)> {
    let mut name = variant.ident.to_string();
    let mut tag = None;
    for attribute in &variant.attrs {
        if !attribute.path().is_ident("portable_value")
            && !attribute.path().is_ident("portable_variant")
        {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                name = meta.value()?.parse::<LitStr>()?.value();
                return Ok(());
            }
            if meta.path.is_ident("tag") {
                tag = Some(meta.value()?.parse::<LitInt>()?.base10_parse()?);
                return Ok(());
            }
            Err(meta.error("expected `name` or `tag` on a PortableValue variant"))
        })?;
    }
    Ok((name, tag))
}

fn value_options(input: &DeriveInput) -> syn::Result<ValueOptions> {
    let mut canonical_name = None;
    let mut version = (1, 0);
    let mut maximum_encoded_bytes = None;
    let mut max_depth = 32;
    let mut max_entries = 4_096;
    let mut max_string_bytes = 4_096;
    let mut max_key_bytes = 4_096;
    let mut max_value_bytes = 4_096;
    let mut max_reconstruction_work = 16_384;
    for attribute in &input.attrs {
        if !attribute.path().is_ident("portable_value") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("id")
                || meta.path.is_ident("name")
                || meta.path.is_ident("canonical_name")
            {
                let value = meta.value()?.parse::<LitStr>()?.value();
                if canonical_name.replace(value).is_some() {
                    return Err(meta.error("PortableValue identity may be declared only once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("version") {
                version = parse_version(&meta.value()?.parse::<LitStr>()?)?;
                return Ok(());
            }
            if meta.path.is_ident("max_encoded_bytes")
                || meta.path.is_ident("maximum_encoded_bytes")
                || meta.path.is_ident("max_size")
            {
                let value = meta.value()?.parse::<LitInt>()?.base10_parse::<u32>()?;
                if maximum_encoded_bytes.replace(value).is_some() {
                    return Err(meta.error("PortableValue maximum encoded size may be declared only once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("max_depth") {
                max_depth = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            if meta.path.is_ident("max_entries") || meta.path.is_ident("max_elements") {
                max_entries = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            if meta.path.is_ident("max_string_bytes") {
                max_string_bytes = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            if meta.path.is_ident("max_key_bytes") {
                max_key_bytes = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            if meta.path.is_ident("max_value_bytes") {
                max_value_bytes = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            if meta.path.is_ident("max_reconstruction_work") {
                max_reconstruction_work = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                return Ok(());
            }
            Err(meta.error(
                "expected `id`, `version`, `max_encoded_bytes`, `max_depth`, `max_entries`, `max_string_bytes`, `max_key_bytes`, `max_value_bytes`, or `max_reconstruction_work`",
            ))
        })?;
    }
    let maximum_encoded_bytes = maximum_encoded_bytes.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "PortableValue requires `max_encoded_bytes = ...`",
        )
    })?;
    if maximum_encoded_bytes < 4 {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "PortableValue `max_encoded_bytes` must leave room for its version header",
        ));
    }
    if max_depth == 0 {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "PortableValue `max_depth` must be greater than zero",
        ));
    }
    Ok(ValueOptions {
        canonical_name,
        major: version.0,
        minor: version.1,
        maximum_encoded_bytes,
        max_depth,
        max_entries,
        max_string_bytes,
        max_key_bytes,
        max_value_bytes,
        max_reconstruction_work,
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

fn reject_raw_unordered_collections(types: &[Type]) -> syn::Result<()> {
    for ty in types {
        reject_raw_unordered_type(ty)?;
    }
    Ok(())
}

fn reject_raw_unordered_type(ty: &Type) -> syn::Result<()> {
    match ty {
        Type::Array(value) => reject_raw_unordered_type(&value.elem),
        Type::Group(value) => reject_raw_unordered_type(&value.elem),
        Type::Paren(value) => reject_raw_unordered_type(&value.elem),
        Type::Ptr(value) => reject_raw_unordered_type(&value.elem),
        Type::Reference(value) => reject_raw_unordered_type(&value.elem),
        Type::Slice(value) => reject_raw_unordered_type(&value.elem),
        Type::Tuple(value) => {
            for element in &value.elems {
                reject_raw_unordered_type(element)?;
            }
            Ok(())
        }
        Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return Ok(());
            };
            if segment.ident == "HashMap" || segment.ident == "HashSet" {
                return Err(syn::Error::new_spanned(
                    ty,
                    "raw HashMap/HashSet values are not canonical; wrap them in `CanonicalHashMap` or `CanonicalHashSet`",
                ));
            }
            if let PathArguments::AngleBracketed(arguments) = &segment.arguments {
                for argument in &arguments.args {
                    if let GenericArgument::Type(value) = argument {
                        reject_raw_unordered_type(value)?;
                    }
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
