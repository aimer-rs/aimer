use proc_macro2::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::{Data, DeriveInput, Fields, parse2};

/// Emits the item together with the `Widget` impl that hands it to a
/// `StatefulElement`, for the `#[widget(Stateful)]` attribute form.
///
/// The attribute *replaces* the item it is written on, so the item has to be
/// emitted again here. A malformed input is returned unchanged so that the
/// compiler reports the syntax error on the user's own tokens rather than on
/// tokens this macro invented.
pub fn generate_stateful_widget_impl(input: TokenStream) -> TokenStream {
    let mut item = match parse2::<DeriveInput>(input.clone()) {
        Ok(item) => item,
        Err(_) => return input,
    };
    let widget_impl = stateful_widget_impl(&item);
    // An attribute macro has no helper attributes: `#[state(..)]` would reach the
    // compiler as an unknown attribute, so it is consumed here.
    item.attrs.retain(|attr| !attr.path().is_ident("state"));

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

/// The `State` implementation the element creates for this widget: the type
/// named by `#[state(MyState)]`, or, without the attribute, the widget's name
/// followed by `State` (see [`default_state_type`]).
///
/// A `#[state]` that does not parse as a type is an error rather than a silent
/// fall back to the default, so a typo is reported where it was written.
fn state_type(input: &DeriveInput) -> Result<syn::Type, syn::Error> {
    match input.attrs.iter().find(|attr| attr.path().is_ident("state")) {
        Some(attr) => attr.parse_args::<syn::Type>(),
        None => Ok(default_state_type(input)),
    }
}

/// `Counter` -> `CounterState`, and `Provider<T, W>` -> `ProviderState<T, W>`.
///
/// The widget's own generic arguments (type, lifetime and const) are applied to
/// the state, so a generic widget pairs with a state that is generic the same
/// way. The name carries the widget's span: when no such state exists the
/// compiler's "cannot find type" points at the widget, which is where the
/// missing state or the `#[state(..)]` override has to be written.
fn default_state_type(input: &DeriveInput) -> syn::Type {
    // `r#Type` names the type `Type`; the prefix is not part of the state's name.
    let name = syn::Ident::new(
        &format!("{}State", input.ident.unraw()),
        input.ident.span(),
    );
    let (_, ty_generics, _) = input.generics.split_for_impl();
    syn::parse_quote!(#name #ty_generics)
}

/// Both forms share the generated `Widget` impl.
fn stateful_widget_impl(input: &DeriveInput) -> TokenStream {
    let state_ty = match state_type(input) {
        Ok(state_ty) => state_ty,
        Err(err) => return err.to_compile_error(),
    };
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
                // The state is named here, in a body, and never in a signature, so
                // it can stay private to the module that defines it.
                aimer::widget::StatefulElement::from_widget::<Self, #state_ty>(
                    self,
                    ctx,
                    stringify!(#item_name),
                    __key,
                )
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
            #[state(PanicBoundaryState)]
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
            #[state(CounterState)]
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
            #[state(CounterState)]
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
            #[state(NotAWidgetState)]
            enum NotAWidget {
                Nope,
            }
        })
        .to_string();

        assert!(output.contains("impl aimer :: widget :: Widget for NotAWidget"));
    }

    #[test]
    fn the_state_is_named_in_the_body_never_in_a_signature() {
        let output = derive_stateful_widget(quote! {
            #[state(PrivateState)]
            struct Counter;
        })
        .to_string();

        assert!(output.contains("from_widget :: < Self , PrivateState >"));
        // Only `to_element`'s body mentions it, so a private state does not leak.
        assert_eq!(output.matches("PrivateState").count(), 1);
    }

    #[test]
    fn a_generic_widget_names_a_generic_state() {
        let output = derive_stateful_widget(quote! {
            #[state(ProviderState<T, W>)]
            struct Provider<T, W> {
                value: T,
                child: W,
            }
        })
        .to_string();

        assert!(output.contains("from_widget :: < Self , ProviderState < T , W > >"));
    }

    #[test]
    fn the_attribute_form_consumes_the_helper_attribute() {
        let output = generate_stateful_widget_impl(quote! {
            #[state(CounterState)]
            struct Counter;
        })
        .to_string();

        // Re-emitted item: `#[state(..)]` would be an unknown attribute on it.
        let (item, _) = output.split_once("impl aimer").expect("emits the Widget impl");
        assert!(!item.contains("# [state"), "{item}");
    }

    #[test]
    fn without_a_state_attribute_the_state_is_the_struct_name_plus_state() {
        let output = derive_stateful_widget(quote! {
            struct Counter;
        })
        .to_string();

        assert!(output.contains("from_widget :: < Self , CounterState >"), "{output}");
        assert!(!output.contains("compile_error"), "{output}");
    }

    #[test]
    fn the_attribute_form_defaults_the_state_the_same_way() {
        let output = generate_stateful_widget_impl(quote! {
            struct Counter;
        })
        .to_string();

        assert!(output.contains("from_widget :: < Self , CounterState >"), "{output}");
    }

    #[test]
    fn an_explicit_state_attribute_overrides_the_default() {
        let output = derive_stateful_widget(quote! {
            #[state(Elsewhere)]
            struct Counter;
        })
        .to_string();

        assert!(output.contains("from_widget :: < Self , Elsewhere >"), "{output}");
        assert!(!output.contains("CounterState"), "{output}");
    }

    #[test]
    fn the_default_state_of_a_generic_widget_takes_the_same_arguments() {
        let output = derive_stateful_widget(quote! {
            struct Provider<T, W> {
                value: T,
                child: W,
            }
        })
        .to_string();

        assert!(
            output.contains("from_widget :: < Self , ProviderState < T , W > >"),
            "{output}"
        );
    }

    #[test]
    fn the_default_state_carries_lifetimes_and_const_arguments() {
        let output = derive_stateful_widget(quote! {
            struct Slice<'a, const N: usize> {
                items: &'a [u8; N],
            }
        })
        .to_string();

        assert!(
            output.contains("from_widget :: < Self , SliceState < 'a , N > >"),
            "{output}"
        );
    }

    #[test]
    fn the_default_state_of_an_enum_and_a_tuple_struct_follows_the_item_name() {
        let enum_output = derive_stateful_widget(quote! {
            enum Choice {
                Yes,
                No,
            }
        })
        .to_string();
        let tuple_output = derive_stateful_widget(quote! {
            struct Pair(i32, i32);
        })
        .to_string();

        assert!(enum_output.contains("from_widget :: < Self , ChoiceState >"), "{enum_output}");
        assert!(tuple_output.contains("from_widget :: < Self , PairState >"), "{tuple_output}");
    }

    #[test]
    fn a_raw_identifier_does_not_leak_into_the_default_name() {
        let output = derive_stateful_widget(quote! {
            struct r#Match;
        })
        .to_string();

        assert!(output.contains("from_widget :: < Self , MatchState >"), "{output}");
    }

    #[test]
    fn a_state_attribute_without_a_type_is_still_a_compile_error() {
        let output = derive_stateful_widget(quote! {
            #[state]
            struct Counter;
        })
        .to_string();

        assert!(output.contains("compile_error"), "{output}");
        assert!(!output.contains("impl aimer"), "{output}");
    }
}
