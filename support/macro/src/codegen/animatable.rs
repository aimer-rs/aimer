use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{Attribute, Data, DataEnum, DeriveInput, Fields, Meta, parse_quote};

#[derive(Clone, Copy)]
pub(crate) enum EndpointPolicy {
    Recursive,
    Clone,
}

#[derive(Clone, Copy)]
enum EnumPolicy {
    Discrete,
    Fieldwise,
}

pub(crate) fn animation_path() -> syn::Result<TokenStream> {
    if let Ok(found) = crate_name("aimer") {
        return Ok(match found {
            FoundCrate::Itself => quote!(::aimer::animation),
            FoundCrate::Name(name) => {
                let name = Ident::new(&name, Span::call_site());
                quote!(::#name::animation)
            }
        });
    }

    match crate_name("aimer_animation") {
        Ok(FoundCrate::Itself) => Ok(quote!(::aimer_animation)),
        Ok(FoundCrate::Name(name)) => {
            let name = Ident::new(&name, Span::call_site());
            Ok(quote!(::#name))
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!(
                "Animatable derive requires a dependency on `aimer` or `aimer_animation`: {error}"
            ),
        )),
    }
}

pub(crate) fn generate_animatable_impl(
    input: DeriveInput,
    animation_path: TokenStream,
) -> syn::Result<TokenStream> {
    match &input.data {
        Data::Struct(data) => {
            reject_policy_attributes(&input.attrs, "Animatable policies are only valid on enums")?;
            for field in &data.fields {
                reject_policy_attributes(
                    &field.attrs,
                    "Animatable field attributes are not supported; implement Animatable for the field type",
                )?;
            }
            generate_struct_animatable_impl(&input, animation_path, EndpointPolicy::Recursive)
        }
        Data::Enum(data) => {
            for variant in &data.variants {
                reject_policy_attributes(
                    &variant.attrs,
                    "Animatable policy must be placed on the enum definition",
                )?;
                for field in &variant.fields {
                    reject_policy_attributes(
                        &field.attrs,
                        "Animatable policy must be placed on the enum definition",
                    )?;
                }
            }
            match enum_policy(&input.attrs)? {
                EnumPolicy::Discrete => generate_discrete_enum_impl(&input, data, animation_path),
                EnumPolicy::Fieldwise => generate_fieldwise_enum_impl(&input, data, animation_path),
            }
        }
        Data::Union(_) => Err(syn::Error::new_spanned(
            &input.ident,
            "Animatable cannot be derived for unions",
        )),
    }
}

fn reject_policy_attributes(attributes: &[Attribute], message: &str) -> syn::Result<()> {
    if let Some(attribute) = attributes
        .iter()
        .find(|attribute| attribute.path().is_ident("animatable"))
    {
        return Err(syn::Error::new_spanned(attribute, message));
    }
    Ok(())
}

fn enum_policy(attributes: &[Attribute]) -> syn::Result<EnumPolicy> {
    let mut policy = None;
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("animatable"))
    {
        if !matches!(attribute.meta, Meta::List(_)) {
            return Err(syn::Error::new_spanned(
                attribute,
                "expected `#[animatable(discrete)]` or `#[animatable(fieldwise)]`",
            ));
        }
        attribute.parse_nested_meta(|meta| {
            let candidate = if meta.path.is_ident("discrete") {
                EnumPolicy::Discrete
            } else if meta.path.is_ident("fieldwise") {
                EnumPolicy::Fieldwise
            } else {
                return Err(
                    meta.error("unsupported Animatable policy; expected `discrete` or `fieldwise`")
                );
            };
            if meta.input.peek(syn::Token![=]) || meta.input.peek(syn::token::Paren) {
                return Err(meta.error("Animatable policies do not accept values"));
            }
            if policy.replace(candidate).is_some() {
                return Err(meta.error("Animatable enum policy may only be specified once"));
            }
            Ok(())
        })?;
    }
    policy.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "Animatable enums require `#[animatable(discrete)]` or `#[animatable(fieldwise)]`",
        )
    })
}

fn generate_discrete_enum_impl(
    input: &DeriveInput,
    data: &DataEnum,
    animation_path: TokenStream,
) -> syn::Result<TokenStream> {
    if data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Animatable cannot be derived for an enum with no variants",
        ));
    }

    let name = &input.ident;
    let mut generics = input.generics.clone();
    for variant in &data.variants {
        for field in &variant.fields {
            let ty = &field.ty;
            generics
                .make_where_clause()
                .predicates
                .push(parse_quote!(#ty: ::core::clone::Clone));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let clone_arms = data.variants.iter().map(clone_variant_arm);

    Ok(quote! {
        impl #impl_generics #animation_path::Animatable for #name #ty_generics #where_clause {
            fn lerp(&self, other: &Self, t: f32) -> Self {
                match if t < 0.5 { self } else { other } {
                    #(#clone_arms,)*
                }
            }
        }
    })
}

fn generate_fieldwise_enum_impl(
    input: &DeriveInput,
    data: &DataEnum,
    animation_path: TokenStream,
) -> syn::Result<TokenStream> {
    if data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Animatable cannot be derived for an enum with no variants",
        ));
    }

    let name = &input.ident;
    let mut generics = input.generics.clone();
    for variant in &data.variants {
        for field in &variant.fields {
            let ty = &field.ty;
            generics.make_where_clause().predicates.push(parse_quote!(
                #ty: #animation_path::Animatable + ::core::clone::Clone
            ));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let interpolation_arms = data
        .variants
        .iter()
        .map(|variant| interpolate_variant_arm(variant, &animation_path));
    let different_variant_arm = if data.variants.len() > 1 {
        let clone_arms = data.variants.iter().map(clone_variant_arm);
        quote! {
            (left, right) => match if t < 0.5 { left } else { right } {
                #(#clone_arms,)*
            }
        }
    } else {
        quote!()
    };

    Ok(quote! {
        impl #impl_generics #animation_path::Animatable for #name #ty_generics #where_clause {
            fn lerp(&self, other: &Self, t: f32) -> Self {
                match (self, other) {
                    #(#interpolation_arms,)*
                    #different_variant_arm
                }
            }
        }
    })
}

fn interpolate_variant_arm(variant: &syn::Variant, animation_path: &TokenStream) -> TokenStream {
    let variant_name = &variant.ident;
    match &variant.fields {
        Fields::Named(fields) => {
            let field_names = fields
                .named
                .iter()
                .map(|field| field.ident.as_ref().expect("named fields have identifiers"))
                .collect::<Vec<_>>();
            let left = field_names
                .iter()
                .map(|name| Ident::new(&format!("__left_{name}"), name.span()))
                .collect::<Vec<_>>();
            let right = field_names
                .iter()
                .map(|name| Ident::new(&format!("__right_{name}"), name.span()))
                .collect::<Vec<_>>();
            quote! {
                (
                    Self::#variant_name { #(#field_names: #left,)* },
                    Self::#variant_name { #(#field_names: #right,)* },
                ) => Self::#variant_name {
                    #(
                        #field_names: #animation_path::Animatable::lerp(#left, #right, t),
                    )*
                }
            }
        }
        Fields::Unnamed(fields) => {
            let left = (0..fields.unnamed.len())
                .map(|index| Ident::new(&format!("__left_{index}"), Span::call_site()))
                .collect::<Vec<_>>();
            let right = (0..fields.unnamed.len())
                .map(|index| Ident::new(&format!("__right_{index}"), Span::call_site()))
                .collect::<Vec<_>>();
            quote! {
                (
                    Self::#variant_name(#(#left,)*),
                    Self::#variant_name(#(#right,)*),
                ) => Self::#variant_name(
                    #(#animation_path::Animatable::lerp(#left, #right, t),)*
                )
            }
        }
        Fields::Unit => quote! {
            (Self::#variant_name, Self::#variant_name) => Self::#variant_name
        },
    }
}

fn clone_variant_arm(variant: &syn::Variant) -> TokenStream {
    let variant_name = &variant.ident;
    match &variant.fields {
        Fields::Named(fields) => {
            let names = fields
                .named
                .iter()
                .map(|field| field.ident.as_ref().expect("named fields have identifiers"));
            let cloned_names = names.clone();
            quote! {
                Self::#variant_name { #(#names,)* } => Self::#variant_name {
                    #(#cloned_names: ::core::clone::Clone::clone(#cloned_names),)*
                }
            }
        }
        Fields::Unnamed(fields) => {
            let bindings = (0..fields.unnamed.len())
                .map(|index| Ident::new(&format!("__field_{index}"), Span::call_site()))
                .collect::<Vec<_>>();
            quote! {
                Self::#variant_name(#(#bindings,)*) => Self::#variant_name(
                    #(::core::clone::Clone::clone(#bindings),)*
                )
            }
        }
        Fields::Unit => quote!(Self::#variant_name => Self::#variant_name),
    }
}

pub(crate) fn generate_struct_animatable_impl(
    input: &DeriveInput,
    animation_path: TokenStream,
    endpoint_policy: EndpointPolicy,
) -> syn::Result<TokenStream> {
    let fields = match &input.data {
        Data::Struct(data) => &data.fields,
        Data::Enum(_) | Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "this Animatable generator requires a struct",
            ));
        }
    };

    let name = &input.ident;
    let mut generics = input.generics.clone();
    for field in fields {
        let ty = &field.ty;
        let predicate = match endpoint_policy {
            EndpointPolicy::Recursive => parse_quote!(#ty: #animation_path::Animatable),
            EndpointPolicy::Clone => {
                parse_quote!(#ty: #animation_path::Animatable + ::core::clone::Clone)
            }
        };
        generics.make_where_clause().predicates.push(predicate);
    }

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let interpolated_value = match fields {
        Fields::Named(fields) => {
            let interpolated_fields = fields.named.iter().map(|field| {
                let ident = field.ident.as_ref().expect("named fields have identifiers");
                quote! {
                    #ident: #animation_path::Animatable::lerp(&self.#ident, &other.#ident, t)
                }
            });
            quote! {
                Self {
                    #(#interpolated_fields,)*
                }
            }
        }
        Fields::Unnamed(fields) => {
            let interpolated_fields = fields.unnamed.iter().enumerate().map(|(index, _)| {
                let index = syn::Index::from(index);
                quote! {
                    #animation_path::Animatable::lerp(&self.#index, &other.#index, t)
                }
            });
            quote!(Self(#(#interpolated_fields,)*))
        }
        Fields::Unit => quote!(Self),
    };
    let endpoints = match endpoint_policy {
        EndpointPolicy::Recursive => quote!(),
        EndpointPolicy::Clone => quote! {
            if t <= 0.0 {
                return self.clone();
            }
            if t >= 1.0 {
                return other.clone();
            }
        },
    };

    Ok(quote! {
        impl #impl_generics #animation_path::Animatable for #name #ty_generics #where_clause {
            fn lerp(&self, other: &Self, t: f32) -> Self {
                #endpoints
                #interpolated_value
            }
        }
    })
}
