use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Path};

pub fn auto_impl(trait_name: &str, input: TokenStream) -> TokenStream {
    let input = match syn::parse2::<DeriveInput>(input.into()) {
        Ok(data) => data,
        Err(err) => {
            return err.to_compile_error().into();
        }
    };

    let trait_path: Path = match syn::parse_str(trait_name) {
        Ok(path) => path,

        Err(err) => {
            return err.to_compile_error().into();
        }
    };
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #impl_generics #trait_path for #name #ty_generics  #where_clause {}
    }
    .into()
}

/// `#[derive(Rebuildable)]`: an implementation with the default behavior, or one
/// that declares [`forwards_marks_only`] when the type carries
/// `#[rebuildable(forwards_marks_only)]`.
///
/// The attribute is a promise that the element does no rebuild work of its own:
/// it neither overrides `mark_needs_rebuild` nor `rebuild_if_dirty` to do
/// anything but pass the call on to its children.
///
/// [`forwards_marks_only`]: https://docs.rs/aimer_widget/*/aimer_widget/trait.Rebuildable.html#method.forwards_marks_only
pub fn rebuildable_impl(input: TokenStream) -> TokenStream {
    let input = match syn::parse2::<DeriveInput>(input.into()) {
        Ok(data) => data,
        Err(err) => return err.to_compile_error().into(),
    };
    let mut forwards = false;
    for attribute in input.attrs.iter().filter(|attribute| attribute.path().is_ident("rebuildable")) {
        let parsed = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("forwards_marks_only") {
                forwards = true;
                Ok(())
            } else {
                Err(meta.error("unknown `rebuildable` option; expected `forwards_marks_only`"))
            }
        });
        if let Err(err) = parsed {
            return err.to_compile_error().into();
        }
    }
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let body = if forwards {
        quote! {
            #[inline]
            fn forwards_marks_only(&self) -> bool {
                true
            }
        }
    } else {
        quote! {}
    };
    quote! {
        impl #impl_generics aimer_widget::Rebuildable for #name #ty_generics #where_clause {
            #body
        }
    }
    .into()
}
