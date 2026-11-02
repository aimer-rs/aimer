use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse2};

/// Emits the item together with the `Widget` impl that drives it through
/// [`StatelessWidget::build`], for the `#[widget(Stateless)]` attribute form.
///
/// The attribute *replaces* the item it is written on, so the item has to be
/// emitted again here. A malformed input is returned unchanged so that the
/// compiler reports the syntax error on the user's own tokens rather than on
/// tokens this macro invented.
///
/// [`StatelessWidget::build`]: https://docs.rs/aimer::widget
pub fn generate_stateless_widget_impl(input: TokenStream) -> TokenStream {
    let item = match parse2::<DeriveInput>(input.clone()) {
        Ok(item) => item,
        Err(_) => return input, // Should handle error properly but returning input is safe fallback
    };
    let widget_impl = stateless_widget_impl(&item);

    quote! {
        #item
        #widget_impl
    }
}

/// Emits only the `Widget` impl, for the `#[derive(StatelessWidget)]` form.
///
/// A derive is expanded *beside* the item it is written on, which the compiler
/// keeps: emitting the item again would define it twice.
pub fn derive_stateless_widget(input: TokenStream) -> TokenStream {
    match parse2::<DeriveInput>(input) {
        Ok(item) => stateless_widget_impl(&item),
        Err(err) => err.to_compile_error(),
    }
}

/// Both forms share the generated `Widget` impl.
fn stateless_widget_impl(input: &DeriveInput) -> TokenStream {
    let item_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let item_name_str = item_name.to_string();

    // Only named structs can expose the conventional `key` field without
    // imposing a positional or variant-specific convention on other items.
    let has_key = matches!(
        &input.data,
        Data::Struct(data) if matches!(&data.fields, Fields::Named(fields) if fields.named.iter().any(|field| field.ident.as_ref().is_some_and(|ident| ident == "key")))
    );

    let key_pass = if has_key {
        quote! { self.key.clone() }
    } else {
        quote! { None }
    };

    let key_method = if has_key {
        quote! {
            fn key(&self) -> Option<aimer::widget::key::Key> {
                self.key.clone()
            }
        }
    } else {
        quote! {}
    };

    quote! {
        impl #impl_generics aimer::widget::Widget for #item_name #ty_generics #where_clause {
            #key_method

            fn to_element(self, ctx: &aimer::widget::base::BuildContext) -> aimer::widget::AnyElement {
                // The element keeps *this* widget — not a copy of it — so it can
                // re-run `build()` (re-reading `MediaQuery`) when marked dirty on
                // resize. Each rebuild describes a fresh child widget, which the
                // consuming conversion then eats, so nothing here needs `Clone`.
                let __key = #key_pass;
                let __rebuild = move |ctx: &aimer::widget::base::BuildContext| -> aimer::widget::AnyElement {
                    aimer::widget::Widget::to_element(self.build(ctx), ctx)
                };
                aimer::widget::Element::boxed(aimer::widget::StatelessElement::from_builder(
                    ctx,
                    __rebuild,
                    __key,
                    #item_name_str,
                ))
            }

            fn debug_name(&self) -> &'static str {
                #item_name_str
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn generated_widget_uses_any_element_for_rebuild_and_erasure() {
        let output = generate_stateless_widget_impl(quote! {
            #[derive(Clone)]
            struct GeneratedWidget;
        })
        .to_string();

        assert!(output.contains("aimer :: widget :: AnyElement"));
        assert!(output.contains("aimer :: widget :: Element :: boxed"));
        assert!(!output.contains("Box < dyn aimer::widget :: Element >"));
        assert!(!output.contains("Box :: new"));
    }

    #[test]
    fn a_generated_widget_is_never_cloned() {
        // The element owns the widget it was built from, so a rebuild re-runs
        // the original. A widget therefore does not have to be `Clone` — which
        // it had to be while the conversion only borrowed it.
        let output = derive_stateless_widget(quote! {
            struct NotCloneWidget {
                buffer: Vec<u8>,
            }
        })
        .to_string();

        assert!(
            !output.contains("clone"),
            "a rebuild source must not be copied out of the widget"
        );
        assert!(output.contains("self . build (ctx)"));
    }

    #[test]
    fn the_derive_leaves_the_struct_to_the_compiler() {
        // A derive is expanded *beside* the item it is written on. Emitting the
        // struct again — as the attribute form must — would define it twice.
        let output = derive_stateless_widget(quote! {
            #[derive(Clone, StatelessWidget)]
            struct GeneratedWidget {
                label: String,
            }
        })
        .to_string();

        assert!(!output.contains("struct GeneratedWidget"));
        assert!(output.contains("impl aimer :: widget :: Widget for GeneratedWidget"));
    }

    #[test]
    fn the_derive_and_the_attribute_wire_the_widget_up_the_same_way() {
        let item = quote! {
            #[derive(Clone)]
            struct GeneratedWidget {
                key: Option<Key>,
            }
        };

        let attribute = generate_stateless_widget_impl(item.clone()).to_string();
        let derived = derive_stateless_widget(item).to_string();

        let (_, attribute_impl) = attribute
            .split_once("impl aimer :: widget :: Widget")
            .expect("the attribute form implements Widget");
        let (_, derived_impl) = derived
            .split_once("impl aimer :: widget :: Widget")
            .expect("the derive implements Widget");
        assert_eq!(attribute_impl, derived_impl);
    }

    #[test]
    fn the_derive_supports_an_enum() {
        let output = derive_stateless_widget(quote! {
            enum NotAWidget {
                Nope,
            }
        })
        .to_string();

        assert!(output.contains("impl aimer :: widget :: Widget for NotAWidget"));
    }
}
