use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_quote};

use super::animatable::{EndpointPolicy, generate_struct_animatable_impl};

pub(crate) fn style_path() -> syn::Result<TokenStream> {
    if let Ok(found) = crate_name("aimer") {
        return Ok(match found {
            FoundCrate::Itself => quote!(::aimer::style),
            FoundCrate::Name(name) => {
                let name = Ident::new(&name, Span::call_site());
                quote!(::#name::style)
            }
        });
    }

    match crate_name("aimer_style") {
        Ok(FoundCrate::Itself) => Ok(quote!(::aimer_style)),
        Ok(FoundCrate::Name(name)) => {
            let name = Ident::new(&name, Span::call_site());
            Ok(quote!(::#name))
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!("Theme derive requires a dependency on `aimer` or `aimer_style`: {error}"),
        )),
    }
}

pub(crate) fn generate_theme_impl(
    input: DeriveInput,
    style_path: TokenStream,
) -> syn::Result<TokenStream> {
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            Fields::Unnamed(_) | Fields::Unit => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "Theme can only be derived for structs with named fields",
                ));
            }
        },
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Theme cannot be derived for enums",
            ));
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Theme cannot be derived for unions",
            ));
        }
    };

    let animation_impl = generate_struct_animatable_impl(
        &input,
        quote!(#style_path::__private),
        EndpointPolicy::Clone,
    )?;
    let name = &input.ident;
    let mut theme_generics = input.generics.clone();
    for field in fields {
        let ty = &field.ty;
        theme_generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(
                #ty: #style_path::__private::Animatable + ::core::clone::Clone
            ));
    }

    let (_, input_ty_generics, _) = input.generics.split_for_impl();
    theme_generics
        .make_where_clause()
        .predicates
        .push(parse_quote!(
            #name #input_ty_generics: ::core::clone::Clone + ::core::cmp::PartialEq + 'static
        ));

    let (theme_impl_generics, theme_ty_generics, theme_where_clause) =
        theme_generics.split_for_impl();

    Ok(quote! {
        #animation_impl

        impl #theme_impl_generics #style_path::Theme
            for #name #theme_ty_generics #theme_where_clause
        {}
    })
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::parse_quote;

    use super::*;

    #[test]
    fn named_struct_generates_fieldwise_interpolation_and_theme_impl() {
        let input = parse_quote! {
            struct AppTheme {
                accent: f32,
                radius: i32,
            }
        };

        let output = generate_theme_impl(input, quote!(::aimer::style))
            .expect("named structs should derive Theme")
            .to_string();

        assert!(output.contains("Animatable :: lerp (& self . accent , & other . accent , t)"));
        assert!(output.contains("Animatable :: lerp (& self . radius , & other . radius , t)"));
        assert!(output.contains("impl :: aimer :: style :: Theme for AppTheme"));
        assert!(output.contains("if t <= 0.0"));
        assert!(output.contains("if t >= 1.0"));
        assert!(!output.contains("is_finite"));
        assert!(!output.contains("is_nan"));
    }

    #[test]
    fn generic_struct_preserves_generics_where_clause_and_adds_field_bounds() {
        let input = parse_quote! {
            struct GenericTheme<T>
            where
                T: Send,
            {
                value: T,
            }
        };

        let output = generate_theme_impl(input, quote!(::aimer::style))
            .expect("generic named structs should derive Theme")
            .to_string();

        assert!(output.contains("impl < T >"));
        assert!(output.contains("T : Send"));
        assert!(output.contains("T : :: aimer :: style :: __private :: Animatable"));
        assert!(output.contains(":: core :: clone :: Clone"));
    }

    #[test]
    fn tuple_struct_has_a_targeted_diagnostic() {
        let error = generate_theme_impl(
            parse_quote!(
                struct TupleTheme(f32);
            ),
            quote!(::aimer::style),
        )
        .expect_err("tuple structs must be rejected");

        assert_eq!(
            error.to_string(),
            "Theme can only be derived for structs with named fields"
        );
    }

    #[test]
    fn unit_struct_has_a_targeted_diagnostic() {
        let error = generate_theme_impl(
            parse_quote!(
                struct UnitTheme;
            ),
            quote!(::aimer::style),
        )
        .expect_err("unit structs must be rejected");

        assert_eq!(
            error.to_string(),
            "Theme can only be derived for structs with named fields"
        );
    }

    #[test]
    fn enum_has_a_targeted_diagnostic() {
        let error = generate_theme_impl(
            parse_quote!(
                enum ThemeChoice {
                    Light,
                    Dark,
                }
            ),
            quote!(::aimer::style),
        )
        .expect_err("enums must be rejected");

        assert_eq!(error.to_string(), "Theme cannot be derived for enums");
    }
}
