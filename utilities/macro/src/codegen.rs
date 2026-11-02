pub(crate) mod animatable;
mod raw_widget;
pub mod router;
mod stateful_widget;
mod stateless_widget;
pub(crate) mod theme;

use proc_macro2::TokenStream;

pub struct StatelessWidgetCodegen;

impl StatelessWidgetCodegen {
    /// The `#[widget(Stateless)]` form: the struct plus its `Widget` impl.
    pub fn generate(input: TokenStream) -> TokenStream {
        stateless_widget::generate_stateless_widget_impl(input)
    }

    /// The `#[derive(StatelessWidget)]` form: the `Widget` impl alone, since
    /// the compiler keeps the struct a derive is written on.
    pub fn derive(input: TokenStream) -> TokenStream {
        stateless_widget::derive_stateless_widget(input)
    }
}

pub struct StatefulWidgetCodegen;

impl StatefulWidgetCodegen {
    /// The `#[widget(Stateful)]` form: the struct plus its `Widget` impl.
    pub fn generate(input: TokenStream) -> TokenStream {
        stateful_widget::generate_stateful_widget_impl(input)
    }

    /// The `#[derive(StatefulWidget)]` form: the `Widget` impl alone, since
    /// the compiler keeps the struct a derive is written on.
    pub fn derive(input: TokenStream) -> TokenStream {
        stateful_widget::derive_stateful_widget(input)
    }
}

pub struct RawWidgetCodegen;

impl RawWidgetCodegen {
    pub fn generate(input: TokenStream) -> TokenStream {
        raw_widget::generate_raw_widget_impl(input)
    }
}
