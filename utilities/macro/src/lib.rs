mod assets;
mod auto_trait_impl;
mod codegen;
mod unique_key;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, ItemFn, parse_macro_input};

use crate::auto_trait_impl::auto_impl;
use crate::codegen::animatable::{animation_path, generate_animatable_impl};
use crate::codegen::router::RouterCodegen;
use crate::codegen::theme::{generate_theme_impl, style_path};
use crate::codegen::{RawWidgetCodegen, StatefulWidgetCodegen, StatelessWidgetCodegen};
use crate::unique_key::UniqueKeyInput;

/// Fills a unit struct with typed consts for the assets and fonts declared in
/// the crate's `Aimer.toml`.
///
/// Each entry of `[assets]` becomes an `::aimer::BundledAsset` and each entry of
/// `[fonts]` an `::aimer::BundledFont`, named after its key in `UPPER_SNAKE_CASE`
/// (`my-image` becomes `MY_IMAGE`). The manifest is read from
/// `CARGO_MANIFEST_DIR` at compile time, so a path that no longer exists, or
/// two keys that map to the same const, are compile errors rather than runtime
/// failures. Editing `Aimer.toml` re-runs the macro.
///
/// The struct keeps its own attributes and visibility, and the consts share
/// that visibility. It must be a unit struct without generics. To have the
/// macro declare the struct as well, use [`declare_assets!`].
///
/// ```rust.ignore
/// // Aimer.toml
/// // [assets]
/// // my_image = "assets/my_image.png"
/// // [fonts]
/// // inter = "assets/fonts/Inter-Regular.ttf"
///
/// #[aimer::assets]
/// pub struct Assets;
///
/// let image = AssetImage::new(Assets::MY_IMAGE);
/// let inter = Assets::INTER.register()?;
/// ```
#[proc_macro_attribute]
pub fn assets(attr: TokenStream, item: TokenStream) -> TokenStream {
    assets::expand_attribute_in(&manifest_dir(), attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares a struct of typed asset and font consts from the crate's
/// `Aimer.toml`. The function-like form of [`macro@assets`].
///
/// The input is an optional visibility, an optional `struct` keyword and an
/// optional name; the name defaults to `Assets` and the visibility to private.
///
/// ```rust.ignore
/// aimer::declare_assets!(pub Assets);
/// ```
#[proc_macro]
pub fn declare_assets(input: TokenStream) -> TokenStream {
    assets::expand_in(&manifest_dir(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}

/// Attribute macro that marks the Aimer application entry point.
///
/// Wraps the annotated function so it is callable from all supported targets:
/// native (via a `#[no_mangle] extern "C"` symbol), Android (via
/// `android_main`), and WebAssembly (via `#[wasm_bindgen]`).
///
/// # Usage
/// ```rust.ignore
///
/// #[aimer::main]
/// fn main() {
///     // application setup
/// }
/// ```
///
/// # What is generated
/// - The original function is kept as-is (marked `#[inline]`).
/// - **Native** (`not(target_arch = "wasm32")`): a `#[no_mangle] pub extern "C"
///   fn __generated_entrance_point()` that calls your function.
/// - **Android** (`target_os = "android"`): an `android_main(app: AndroidApp)`
///   that registers the `AndroidApp` handle (in `ANDROID_APP`, and as the
///   location `aimer_storage` keeps app data in) and then calls your function.
/// - **WASM** (`target_arch = "wasm32"`): a `#[wasm_bindgen] pub fn
///   __generated_entrance_point()` that calls your function.
///
/// # Notes
/// - The macro does not accept any arguments; the `_attr` parameter is ignored.
/// - Your function must be a plain `fn` item (no async, no generics).
#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);
    TokenStream::from(expand_main(input_fn))
}

fn expand_main(input_fn: ItemFn) -> proc_macro2::TokenStream {
    let fn_name = &input_fn.sig.ident;
    quote! {
        #[allow(unexpected_cfgs)]
        #[cfg(target_arch = "wasm32")]
        use aimer::wasm_bindgen;
        #[allow(unexpected_cfgs)]
        #[cfg(target_arch = "wasm32")]
        use aimer::wasm_bindgen::prelude::wasm_bindgen;

        #[inline]
        #input_fn

        #[cfg(not(target_arch = "wasm32"))]
        #[unsafe(no_mangle)]
        pub extern "C" fn __generated_entrance_point(){
            #fn_name()
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "C" fn android_main(app: aimer::quiver::winit::platform::android::activity::AndroidApp) {
            aimer::quiver::register_android_app(app);
            #fn_name()
        }

        #[allow(unexpected_cfgs)]
        #[cfg(target_arch = "wasm32")]
        #[wasm_bindgen(start)]
        pub fn __generated_entrance_point(){
            #fn_name()
        }
    }
}

#[allow(dead_code)]
enum AttributeKind {
    Stateless,
    Stateful,
    Router,
    RawWidget,
}

impl TryFrom<&str> for AttributeKind {
    type Error = syn::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "stateless" => Ok(AttributeKind::Stateless),
            "stateful" => Ok(AttributeKind::Stateful),
            "router" => Ok(AttributeKind::Router),
            "rawwidget" => Ok(AttributeKind::RawWidget),
            _ => Err(syn::Error::new_spanned(
                value,
                "Only accepts `Stateless`, `Stateful`, `Router` or `RawWidget`",
            )),
        }
    }
}

/// Attribute macro that wires up an item as a Widget.
///
/// # Deprecated
///
/// Use the derives instead. They generate identical code, but are expanded
/// *beside* the item instead of replacing it, so the definition the compiler
/// (and rust-analyzer) sees is the one you wrote:
///
/// | Instead of | Write |
/// |------------|-------|
/// | `#[widget(Stateless)]` | [`#[derive(StatelessWidget)]`](macro@StatelessWidget) |
/// | `#[widget(Stateful)]` | [`#[derive(StatefulWidget)]`](macro@StatefulWidget) |
/// | `#[widget(Router)]` | [`#[derive(Router)]`](macro@Router) |
/// | `#[widget(RawWidget)]` | implement `Widget` by hand: this kind only emitted a stub whose body is `unimplemented!` |
///
/// ```rust,ignore
/// // Before
/// #[widget(Stateless)]
/// pub struct MyWidget {
///     pub label: String,
/// }
///
/// // After
/// #[derive(StatelessWidget)]
/// pub struct MyWidget {
///     pub label: String,
/// }
/// ```
///
/// The `impl StatelessWidget`, `impl State<..>` or `impl Router` block you
/// already wrote stays as it is.
///
/// # What it did
///
/// Accepts one of four kinds as its argument:
///
/// | Kind | Target | What is generated |
/// |------|--------|-------------------|
/// | `Stateless` | struct, tuple struct, enum, or union | `impl Widget` via `StatelessWidget::build` |
/// | `Stateful` | struct, tuple struct, enum, or union | `impl Widget` via `StatefulElement`; the state is `<Name>State`, or the type in `#[state(..)]` |
/// | `Router` | enum | full router `impl Widget` dispatch |
/// | `RawWidget` | struct | bare `impl Widget` stub (body uses `unimplemented!`) |
///
/// # Panics
/// Panics at compile time if no argument is provided.
#[deprecated(
    since = "0.1.0",
    note = "use `#[derive(StatelessWidget)]`, `#[derive(StatefulWidget)]` or `#[derive(Router)]` instead; they generate the same code beside your item rather than replacing it"
)]
#[proc_macro_attribute]
pub fn widget(args: TokenStream, input: TokenStream) -> TokenStream {
    if args.is_empty() {
        panic!("Missing the widget kind : Stateless, Stateful, Router or RawWidget");
    }

    let args_str = args.to_string();
    let is_stateful = args_str.to_lowercase().contains("stateful");
    let is_router = args_str.to_lowercase().contains("router");
    let is_raw_widget = args_str.to_lowercase().contains("rawwidget");

    // Parse the input item
    let item = parse_macro_input!(input as Item);

    if is_router {
        return if let Item::Enum(item_enum) = item {
            let input_ts = quote! { #item_enum };
            let router_code = RouterCodegen::generate(input_ts);
            TokenStream::from(router_code)
        } else {
            syn::Error::new_spanned(item, "Router widget can only be applied to enums")
                .to_compile_error()
                .into()
        };
    }

    if is_raw_widget && !matches!(&item, Item::Struct(_)) {
        return syn::Error::new_spanned(item, "RawWidget can only be applied to structs")
            .to_compile_error()
            .into();
    }

    if !matches!(&item, Item::Struct(_) | Item::Enum(_) | Item::Union(_)) {
        return syn::Error::new_spanned(
            item,
            "Stateless and Stateful widgets can only be applied to structs, enums, or unions",
        )
        .to_compile_error()
        .into();
    }

    // Convert the original item back to TokenStream for codegen. Stateless and
    // Stateful intentionally accept every item shape that can implement their
    // respective user traits.
    let input_ts = quote! { #item };

    let widget_code = if is_raw_widget {
        RawWidgetCodegen::generate(input_ts)
    } else if is_stateful {
        StatefulWidgetCodegen::generate(input_ts)
    } else {
        StatelessWidgetCodegen::generate(input_ts)
    };

    TokenStream::from(widget_code)
}

/// Derives `Widget` for a stateless widget item.
///
/// Replaces the deprecated `#[widget(Stateless)]`: it generates the exact same
/// `Widget` impl, but is written *beside* your item instead of replacing it, so
/// the definition the compiler sees is the one you wrote.
///
/// The generated `to_element` keeps the widget in a [`StatelessElement`] so
/// that the element can re-run `build()` when it is marked dirty — on a resize,
/// for instance. Your item must implement `StatelessWidget`.
///
/// For named structs, a field named `key` (of type `Option<Key>`) is picked up
/// automatically and forwarded to the element, giving the widget a stable
/// identity across rebuilds.
///
/// # Examples
///
/// ```rust,ignore
/// use aimer::*;
///
/// #[derive(StatelessWidget)]
/// pub struct Greeting {
///     pub name: String,
/// }
///
/// impl StatelessWidget for Greeting {
///     fn build(&self, _: &BuildContext) -> AnyWidget {
///         Text::new(format!("Hello, {}", self.name)).boxed()
///     }
/// }
/// ```
///
/// [`StatelessElement`]: https://docs.rs/aimer_widget
#[proc_macro_derive(StatelessWidget)]
pub fn stateless_widget_derive(input: TokenStream) -> TokenStream {
    TokenStream::from(StatelessWidgetCodegen::derive(input.into()))
}

/// Derives `Widget` for a stateful widget item.
///
/// Replaces the deprecated `#[widget(Stateful)]`: it generates the exact same
/// `Widget` impl, but is written *beside* your item instead of replacing it, so
/// the definition the compiler sees is the one you wrote.
///
/// The generated `to_element` hands the widget to a `StatefulElement`, which
/// creates the widget's state and keeps it alive across rebuilds. That type must
/// implement `State<Self>`. It is named only inside the body of `to_element`, so
/// it can stay private to its module.
///
/// # Naming the state
///
/// By default the state is the widget's name followed by `State`: `Counter`
/// uses `CounterState`. A generic widget uses a state with the same arguments,
/// so `Provider<T, W>` uses `ProviderState<T, W>`. Write `#[state(Other)]` to
/// use a different type; it replaces the default entirely. If the default type
/// does not exist, the compiler reports "cannot find type" at the widget's
/// name.
///
/// For named structs, a field named `key` (of type `Option<Key>`) is picked up
/// automatically and forwarded to the element, giving the state a stable
/// identity across rebuilds.
///
/// # Examples
///
/// ```rust,ignore
/// use aimer::*;
///
/// // No attribute: the state is `CounterState`.
/// #[derive(StatefulWidget)]
/// pub struct Counter {
///     pub initial_count: i32,
/// }
///
/// struct CounterState {
///     count: i32,
/// }
///
/// impl State<Counter> for CounterState {
///     fn create(widget: Counter) -> Self {
///         Self { count: widget.initial_count }
///     }
///
///     fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
///         Text::new(format!("{}", this.count))
///     }
/// }
///
/// // A state with another name is named explicitly.
/// #[derive(StatefulWidget)]
/// #[state(Tally)]
/// pub struct Score {
///     pub points: i32,
/// }
/// ```
#[proc_macro_derive(StatefulWidget, attributes(state))]
pub fn stateful_widget_derive(input: TokenStream) -> TokenStream {
    TokenStream::from(StatefulWidgetCodegen::derive(input.into()))
}

/// Derives `Route` and `Widget` for a route enum.
///
/// Replaces the deprecated `#[widget(Router)]`: it generates the exact same
/// `Route` parsing/formatting table and `Widget` dispatch, but is
/// written *beside* your enum instead of replacing it. You supply the
/// `Router::build` impl that turns a route into a widget.
///
/// # Helper attributes
///
/// | Attribute | On | Meaning |
/// |-----------|----|---------|
/// | `#[route("/path/{field}", name = "...")]` | variant | One path template. `{field}` binds a named field, `{}` a tuple field, and anything after `?` is read from the query string. An optional `name` makes the route addressable by [`Route::resolve_named`]. |
/// | `#[routes("/a", "/b")]` | variant | Several templates parsing to the same variant. The first one is what `format()` produces. |
/// | `#[shell("/prefix", name = "...")]` | variant | A nested route: the variant's single unnamed field is a child route enum, and the remainder of the path is parsed by it. |
/// | `#[redirect(guard = "path::to::fn")]` or `#[redirect(to = "/other")]` | variant | Overrides [`Route::redirect`] for this variant. |
///
/// A variant with no `#[route]`/`#[routes]` falls back to `/` plus its
/// lowercased name.
///
/// # Examples
///
/// ```rust,ignore
/// use aimer::*;
/// use aimer::router::Router;
///
/// #[derive(Clone, Debug, PartialEq, Router)]
/// pub enum AppRoute {
///     #[route("/")]
///     Home,
///     #[route("/profile/{name}", name = "profile")]
///     Profile { name: String },
/// }
///
/// impl Router for AppRoute {
///     fn build(&self, _: &BuildContext) -> AnyWidget {
///         match self {
///             AppRoute::Home => HomeScreen {}.boxed(),
///             AppRoute::Profile { name } => ProfileScreen::new(name).boxed(),
///         }
///     }
/// }
/// ```
///
/// [`Route::resolve_named`]: https://docs.rs/aimer_router
/// [`Route::redirect`]: https://docs.rs/aimer_router
#[proc_macro_derive(Router, attributes(route, routes, shell, redirect, transition, view_level))]
pub fn router_derive(input: TokenStream) -> TokenStream {
    TokenStream::from(RouterCodegen::derive(input.into()))
}

#[proc_macro_derive(VisitorElement)]
pub fn visitor_element_derive(input: TokenStream) -> TokenStream {
    auto_impl("aimer_widget::VisitorElement", input)
}
#[proc_macro_derive(EventElement)]
pub fn event_element_derive(input: TokenStream) -> TokenStream {
    auto_impl("aimer_widget::EventElement", input)
}

#[proc_macro_derive(LayoutElement)]
pub fn layout_element_derive(input: TokenStream) -> TokenStream {
    auto_impl("aimer_widget::LayoutElement", input)
}

/// Implements `aimer_widget::Rebuildable` with its defaults.
///
/// Add `#[rebuildable(forwards_marks_only)]` to declare that the element is a
/// plain wrapper that only passes marks and rebuilds on to its children; see
/// `Rebuildable::forwards_marks_only` for what that promises.
#[proc_macro_derive(Rebuildable, attributes(rebuildable))]
pub fn rebuildable_element_derive(input: TokenStream) -> TokenStream {
    crate::auto_trait_impl::rebuildable_impl(input)
}

#[proc_macro_derive(Reconcilable)]
pub fn reconcilable_derive(input: TokenStream) -> TokenStream {
    auto_impl("aimer_widget::Reconcilable", input)
}

#[proc_macro_derive(Drawable)]
pub fn drawable_element_derive(input: TokenStream) -> TokenStream {
    auto_impl("aimer_widget::Drawable", input)
}

/// Derives field-by-field interpolation and theme lookup behavior for a named
/// struct.
#[proc_macro_derive(Theme)]
pub fn theme_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let output = style_path().and_then(|style_path| generate_theme_impl(input, style_path));
    output.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Derives recursive interpolation for structs and explicitly-policy enums.
///
/// Named and tuple structs interpolate every field; unit structs return the
/// unit value. Every struct field must implement
/// `aimer_animation::Animatable`, and generated bounds preserve the type's
/// generics and existing `where` clause.
///
/// Enums require `#[animatable(discrete)]` or
/// `#[animatable(fieldwise)]`. Discrete enums clone the source when `t < 0.5`
/// and the target otherwise, so `NaN` selects the target. Fieldwise enums
/// interpolate fields when the variants match and use the discrete rule when
/// they differ. Enum payloads therefore need `Clone`; fieldwise payloads also
/// need `Animatable`.
///
/// The generated implementation delegates `t` unchanged to recursive fields,
/// including endpoint, extrapolation, and non-finite behavior. Custom mappings
/// between unrelated enum variants require a manual implementation.
#[proc_macro_derive(Animatable, attributes(animatable))]
pub fn animatable_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let output =
        animation_path().and_then(|animation_path| generate_animatable_impl(input, animation_path));
    output.unwrap_or_else(syn::Error::into_compile_error).into()
}

#[proc_macro]
/// Generates a unique key for a widget that needs to remember its state.
pub fn key(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as UniqueKeyInput);

    let value = match input.prefix {
        Some(prefix) => format!("{}-{}", prefix.value(), uuid::Uuid::new_v4()),
        None => uuid::Uuid::new_v4().to_string(),
    };

    quote! {
        Key::Static(#value)
    }
    .into()
}
