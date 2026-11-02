use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse2};

/// Emits the item together with the `Widget` impl that hands it to a
/// `StatefulElement`, for the `#[widget(Stateful)]` attribute form.
///
/// The attribute *replaces* the item it is written on, so the item has to be
/// emitted again here. A malformed input is returned unchanged so that the
/// compiler reports the syntax error on the user's own tokens rather than on
/// tokens this macro invented.
pub fn generate_stateful_widget_impl(input: TokenStream) -> TokenStream {
    let item = match parse2::<DeriveInput>(input.clone()) {
        Ok(item) => item,
        Err(_) => return input,
    };
    let widget_impl = stateful_widget_impl(&item);

    quote! {
        #item
        #widget_impl
    }
}

/// Emits only the `Widget` impl, for the `#[derive(StatefulWidget)]` form.
///
/// A derive is expanded *beside* the item it is written on, which the compiler
/// keeps: emitting the item again would define it twice.
pub fn derive_stateful_widget(input: TokenStream) -> TokenStream {
    match parse2::<DeriveInput>(input) {
        Ok(item) => stateful_widget_impl(&item),
        Err(err) => err.to_compile_error(),
    }
}

/// Both forms share the generated `Widget` impl.
fn stateful_widget_impl(input: &DeriveInput) -> TokenStream {
    let item_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

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
                // The key is read before the widget is handed over, because the
                // element takes it by value and moves its props into the state.
                let __key = #key_pass;
                aimer::widget::StatefulElement::from_widget(self, ctx, stringify!(#item_name), __key)
            }

            fn debug_name(&self) -> &'static str {
                stringify!(#item_name)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn generated_widget_uses_recoverable_stateful_conversion() {
        let output = generate_stateful_widget_impl(quote! {
            struct PanicBoundaryWidget;
        })
        .to_string();

        assert!(output.contains("StatefulElement :: from_widget"));
        assert!(output.contains("aimer :: widget :: AnyElement"));
        assert!(!output.contains("StatefulElement :: new_with_name"));
        assert!(!output.contains("Box < dyn widget :: Element >"));
    }

    #[test]
    fn the_derive_leaves_the_struct_to_the_compiler() {
        let output = derive_stateful_widget(quote! {
            #[derive(StatefulWidget)]
            struct CounterWidget {
                initial_count: i32,
            }
        })
        .to_string();

        assert!(!output.contains("struct CounterWidget"));
        assert!(output.contains("impl aimer :: widget :: Widget for CounterWidget"));
    }

    #[test]
    fn the_derive_and_the_attribute_wire_the_widget_up_the_same_way() {
        let item = quote! {
            struct CounterWidget {
                key: Option<Key>,
            }
        };

        let attribute = generate_stateful_widget_impl(item.clone()).to_string();
        let derived = derive_stateful_widget(item).to_string();

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
        let output = derive_stateful_widget(quote! {
            enum NotAWidget {
                Nope,
            }
        })
        .to_string();

        assert!(output.contains("impl aimer :: widget :: Widget for NotAWidget"));
    }
}
