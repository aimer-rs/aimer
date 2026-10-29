mod assets;
mod auto_trait_impl;
mod capability;
mod codegen;
mod portable_value;
mod portable_widget;
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

/// Derives stable portable widget, property, callback, and child metadata.
///
/// With the `portable-guest` feature enabled in the consuming crate, the
/// derive also emits a checked `aimer_widget::PortableWidget` implementation
/// that moves owned properties, lowers children, and binds reflected callback
/// metadata. Use `#[portable_widget(manual_lowering)]` when a widget retains a
/// handwritten lowering while still using the generated schema and native
/// materializer. A generated lowering may call a checked validation function
/// with `#[portable_widget(validate = path)]` before consuming the widget.
/// A required child may use `#[portable_child(discriminator = n)]` when it
/// must preserve a legacy nested source slot; otherwise the field name supplies
/// the stable discriminator automatically.
#[proc_macro_derive(
    PortableWidget,
    attributes(
        portable_widget,
        portable_child,
        portable_children,
        portable_callback,
        portable_skip,
        portable_optional
    )
)]
pub fn portable_widget_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    portable_widget::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derives a deterministic, bounded, versioned value codec.
///
/// The derive emits the structural [`aimer_widget::portable::PortableEncode`]
/// and [`aimer_widget::portable::PortableDecode`] implementations plus the
/// reflected property, guest encoder, and host materializer contracts used by
/// AWIR BLOBREF values. Use `#[portable_value(...)]` to declare the canonical
/// value identity, version, byte ceiling, and optional structural limits.
#[proc_macro_derive(
    PortableValue,
    attributes(portable_value, portable_field, portable_variant)
)]
pub fn portable_value_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    portable_value::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares one portable native/WASM capability contract.
///
/// The annotated trait is preserved and a `<Trait>Capability` companion type
/// exposes its stable identity, ABI major, SDK release metadata, contract
/// fingerprint, and canonical manifest requirement.
#[proc_macro_attribute]
pub fn capability(args: TokenStream, input: TokenStream) -> TokenStream {
    capability::expand(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

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
///   __generated_entrance_point()` that calls your function. Aimer CLI's
///   private portable-guest compilation omits this browser-only entry point.
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
    let wasm_cfg = if cfg!(feature = "hot-reload") {
        quote!(all(target_arch = "wasm32", not(aimer_portable_guest)))
    } else {
        quote!(target_arch = "wasm32")
    };

    quote! {
        #[allow(unexpected_cfgs)]
        #[cfg(#wasm_cfg)]
        use aimer::wasm_bindgen;
        #[allow(unexpected_cfgs)]
        #[cfg(#wasm_cfg)]
        use aimer::wasm_bindgen::prelude::wasm_bindgen;

        #[inline]
        #input_fn

        #[cfg(not(target_arch = "wasm32"))]
        #[unsafe(no_mangle)]
        pub extern "C" fn __generated_entrance_point(){
            if !aimer::quiver::initialize_hot_reload_host() {
                return;
            }
            #fn_name()
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "C" fn android_main(app: aimer::quiver::winit::platform::android::activity::AndroidApp) {
            aimer::quiver::register_android_app(app);
            if !aimer::quiver::initialize_hot_reload_host() {
                return;
            }
            #fn_name()
        }

        #[allow(unexpected_cfgs)]
        #[cfg(#wasm_cfg)]
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
/// The `impl StatelessWidget`, `impl StatefulWidget` or `impl Router` block you
/// already wrote stays as it is.
///
/// # What it did
///
/// Accepts one of four kinds as its argument:
///
/// | Kind | Target | What is generated |
/// |------|--------|-------------------|
/// | `Stateless` | struct, tuple struct, enum, or union | `impl Widget` via `StatelessWidget::build` |
/// | `Stateful` | struct, tuple struct, enum, or union | `impl Widget` via `StatefulElement` |
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
/// owns the `State` your `StatefulWidget` impl creates and keeps it alive
/// across rebuilds. Your struct must implement `StatefulWidget`.
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
/// #[derive(StatefulWidget)]
/// pub struct Counter {
///     pub initial_count: i32,
/// }
///
/// impl StatefulWidget for Counter {
///     type State = CounterState;
///
///     fn create_state(self) -> CounterState {
///         CounterState { count: self.initial_count }
///     }
/// }
/// ```
#[proc_macro_derive(StatefulWidget)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "hot-reload")]
    #[test]
    fn main_bootstraps_the_native_reload_host_before_application_code() {
        let input: ItemFn = syn::parse_quote! {
            fn application_main() {}
        };

        let expanded = expand_main(input).to_string();
        let entrance = expanded
            .split("fn __generated_entrance_point")
            .nth(1)
            .expect("native entrance point must be generated");
        let bootstrap = entrance
            .find("aimer :: quiver :: initialize_hot_reload_host")
            .expect("native entrance point must bootstrap hot reload");
        let application = entrance
            .find("application_main")
            .expect("native entrance point must call application code");

        assert!(bootstrap < application);
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn main_reserves_the_browser_entry_for_non_guest_wasm() {
        let input: ItemFn = syn::parse_quote! {
            fn application_main() {}
        };

        let expanded = expand_main(input).to_string();

        assert!(expanded.contains("not (aimer_portable_guest)"));
        assert!(expanded.contains("wasm_bindgen (start)"));
    }
}

#[cfg(all(feature = "hot-reload", test))]
mod capability_compile_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    const VALID_CAPABILITY: &str = r#"
#[aimer_macro::capability(
    name = "device-control",
    id = "com.example.device-control",
    abi = 1,
    since = "1.2.0",
)]
pub trait DeviceControl {
    fn enabled(&self) -> aimer_anteros::CapabilityResult<bool>;
    fn set_level(&self, level: u32) -> aimer_anteros::CapabilityResult<u64>;
    fn upload(&self, label: &str, payload: &[u8]) -> aimer_anteros::CapabilityResult<Option<Vec<u8>>>;
}
"#;

    #[cfg(feature = "hot-reload")]
    #[test]
    fn capability_accepts_portable_metadata_and_wire_types() {
        let output = check_case("valid_capability", VALID_CAPABILITY);

        assert_success(&output);
    }

    #[test]
    fn capability_uses_the_persistent_package_namespace() {
        let output = check_case_with_metadata(
            "package_namespace",
            r#"
#[aimer_macro::capability(name = "haptics", abi = 1, since = "1.0.0")]
pub trait Haptics {
    fn trigger(&self, kind: u32) -> aimer_anteros::CapabilityResult<()>;
}

const _: () = {
    assert!(HapticsCapability::CANONICAL_ID.as_bytes()[0] == b'0');
    assert!(HapticsCapability::CANONICAL_ID.as_bytes()[36] == b':');
    let id = HapticsCapability::ID.as_bytes();
    assert!(id[0] == 0x96 && id[1] == 0xBE && id[15] == 0x56);
};
"#,
            "crate-id = \"018f4e8b-7c65-7ad1-9b31-6b376bf90242\"",
        );

        assert_success(&output);
    }

    #[test]
    fn capability_uses_the_crates_io_source_from_aimer_build_metadata() {
        let output = check_case_with_source(
            "crates_io_namespace",
            r#"
#[aimer_macro::capability(name = "haptics", abi = 1, since = "1.0.0")]
pub trait Haptics {
    fn trigger(&self, kind: u32) -> aimer_anteros::CapabilityResult<()>;
}

const EXPECTED_ID: [u8; 16] = [
    0x2D, 0x39, 0x22, 0x77, 0x1B, 0x78, 0x32, 0xEB,
    0x65, 0x1D, 0xDC, 0xBE, 0x1C, 0x4B, 0xA2, 0x2F,
];
const _: () = {
    let id = HapticsCapability::ID.as_bytes();
    let mut index = 0;
    while index < EXPECTED_ID.len() {
        assert!(id[index] == EXPECTED_ID[index]);
        index += 1;
    }
};
"#,
            Some("registry+https://github.com/rust-lang/crates.io-index"),
        );

        assert_success(&output);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn capability_rejects_non_crates_io_sources_without_a_persistent_id() {
        for (name, source) in [
            (
                "alternate_registry_namespace",
                Some("registry+https://packages.example/index"),
            ),
            ("git_namespace", Some("git+https://example.com/sdk.git")),
            ("workspace_namespace", None),
            ("path_namespace", None),
        ] {
            let output = check_case_with_source(
                name,
                r#"
#[aimer_macro::capability(name = "haptics", abi = 1, since = "1.0.0")]
pub trait Haptics {
    fn trigger(&self, kind: u32) -> aimer_anteros::CapabilityResult<()>;
}
"#,
                source,
            );
            assert_failure(
                &output,
                "require `[package.metadata.aimer] crate-id` or an explicit `id`",
            );
        }
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn capability_rejects_a_missing_aimer_package_source_map() {
        let output = check_case(
            "missing_source_map",
            r#"
#[aimer_macro::capability(name = "haptics", abi = 1, since = "1.0.0")]
pub trait Haptics {
    fn trigger(&self, kind: u32) -> aimer_anteros::CapabilityResult<()>;
}
"#,
        );

        assert_failure(
            &output,
            "Aimer capability package source map is unavailable",
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn capability_rejects_missing_and_ambiguous_source_map_entries() {
        let source = r#"
#[aimer_macro::capability(name = "haptics", abi = 1, since = "1.0.0")]
pub trait Haptics {
    fn trigger(&self, kind: u32) -> aimer_anteros::CapabilityResult<()>;
}
"#;
        let missing =
            check_case_with_source_map("missing_source_entry", source, SourceMapCase::Missing);
        assert_failure(
            &missing,
            "source map does not contain the compiling package",
        );

        let ambiguous = check_case_with_source_map(
            "ambiguous_source_entry",
            source,
            SourceMapCase::Ambiguous(Some(
                "registry+https://github.com/rust-lang/crates.io-index",
            )),
        );
        assert_failure(
            &ambiguous,
            "source map contains an ambiguous manifest entry",
        );
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn capability_metadata_matches_the_native_golden_contract_on_wasm() {
        let output = check_wasm_case(
            "wasm_contract_parity",
            r#"
#[aimer_macro::capability(
    name = "device-control",
    id = "com.example.device-control",
    abi = 1,
    since = "1.2.0",
)]
pub trait DeviceControl {
    fn enabled(&self) -> aimer_anteros::CapabilityResult<bool>;
    fn set_level(&self, level: u32) -> aimer_anteros::CapabilityResult<u64>;
    fn upload(&self, label: &str, payload: &[u8]) -> aimer_anteros::CapabilityResult<Option<Vec<u8>>>;
}

const EXPECTED_ID: [u8; 16] = [
    0xC8, 0x05, 0xC5, 0xFD, 0xAA, 0x5F, 0xFC, 0x0E,
    0x9E, 0x91, 0x55, 0xF7, 0x5A, 0x0F, 0xFA, 0x88,
];
const EXPECTED_FINGERPRINT: [u8; 32] = [
    0xD4, 0x14, 0x3B, 0x39, 0xE0, 0x94, 0xC9, 0xEE,
    0x04, 0xA8, 0xBA, 0xFC, 0x93, 0x1A, 0xA8, 0x64,
    0x2E, 0xA4, 0x74, 0xE7, 0x33, 0xA4, 0x2F, 0x41,
    0x8C, 0x15, 0x2F, 0x63, 0xC2, 0x90, 0xEF, 0xF9,
];

const _: () = {
    let id = DeviceControlCapability::ID.as_bytes();
    let fingerprint = DeviceControlCapability::CONTRACT_FINGERPRINT;
    let mut index = 0;
    while index < EXPECTED_ID.len() {
        assert!(id[index] == EXPECTED_ID[index]);
        index += 1;
    }
    index = 0;
    while index < EXPECTED_FINGERPRINT.len() {
        assert!(fingerprint[index] == EXPECTED_FINGERPRINT[index]);
        index += 1;
    }
};
"#,
        );

        assert_success(&output);
    }

    #[test]
    #[cfg(feature = "portable-guest")]
    fn capability_rejects_invalid_contract_declarations() {
        let cases = [
            CompileFailure {
                name: "missing_abi",
                source: r#"
#[aimer_macro::capability(name = "payments", since = "1.0.0")]
pub trait Payments {
    fn charge(&self, cents: u64) -> bool;
}
"#,
                diagnostic: "missing required `abi` capability metadata",
            },
            CompileFailure {
                name: "duplicate_method",
                source: r#"
#[aimer_macro::capability(name = "payments", id = "com.example.payments", abi = 1, since = "1.0.0")]
pub trait Payments {
    fn charge(&self, cents: u64) -> aimer_anteros::CapabilityResult<bool>;
    fn charge(&self, cents: u32) -> aimer_anteros::CapabilityResult<bool>;
}
"#,
                diagnostic: "duplicate capability method `charge`",
            },
            CompileFailure {
                name: "generic_method",
                source: r#"
#[aimer_macro::capability(name = "storage", id = "com.example.storage", abi = 1, since = "1.0.0")]
pub trait Storage {
    fn read<T>(&self, key: u64) -> T;
}
"#,
                diagnostic: "capability methods cannot declare generics",
            },
            CompileFailure {
                name: "borrowed_return",
                source: r#"
#[aimer_macro::capability(name = "locale", id = "com.example.locale", abi = 1, since = "1.0.0")]
pub trait Locale {
    fn language(&self) -> aimer_anteros::CapabilityResult<&str>;
}
"#,
                diagnostic: "capability return values cannot borrow data",
            },
            CompileFailure {
                name: "native_layout",
                source: r#"
#[aimer_macro::capability(name = "storage", id = "com.example.storage", abi = 1, since = "1.0.0")]
pub trait Storage {
    fn open(&self, path: std::path::PathBuf) -> bool;
}
"#,
                diagnostic: "unsupported capability wire type `std::path::PathBuf`",
            },
            CompileFailure {
                name: "raw_pointer",
                source: r#"
#[aimer_macro::capability(name = "memory", id = "com.example.memory", abi = 1, since = "1.0.0")]
pub trait Memory {
    fn inspect(&self, pointer: *const u8) -> u32;
}
"#,
                diagnostic: "native pointers cannot cross a capability boundary",
            },
            CompileFailure {
                name: "async_method",
                source: r#"
#[aimer_macro::capability(name = "storage", id = "com.example.storage", abi = 1, since = "1.0.0")]
pub trait Storage {
    async fn read(&self, key: u64) -> Vec<u8>;
}
"#,
                diagnostic: "async capability methods require a declared asynchronous handle schema",
            },
            CompileFailure {
                name: "unsafe_method",
                source: r#"
#[aimer_macro::capability(name = "memory", id = "com.example.memory", abi = 1, since = "1.0.0")]
pub trait Memory {
    unsafe fn inspect(&self, address: u64) -> u32;
}
"#,
                diagnostic: "capability methods cannot be `unsafe`",
            },
            CompileFailure {
                name: "non_standard_result",
                source: r#"
#[aimer_macro::capability(name = "haptics", id = "com.example.haptics", abi = 1, since = "1.0.0")]
pub trait Haptics {
    fn trigger(&self, kind: u32) -> bool;
}
"#,
                diagnostic: "capability methods must return `CapabilityResult<T>`",
            },
        ];

        for case in cases {
            let output = check_case(case.name, case.source);
            assert_failure(&output, case.diagnostic);
        }
    }

    struct CompileFailure {
        name: &'static str,
        source: &'static str,
        diagnostic: &'static str,
    }

    fn check_case(name: &str, source: &str) -> Output {
        check_case_with_options(name, source, "", None, None)
    }

    fn check_wasm_case(name: &str, source: &str) -> Output {
        check_case_with_options(name, source, "", Some("wasm32-unknown-unknown"), None)
    }

    fn check_case_with_metadata(name: &str, source: &str, aimer_metadata: &str) -> Output {
        check_case_with_options(name, source, aimer_metadata, None, None)
    }

    fn check_case_with_source(name: &str, source: &str, package_source: Option<&str>) -> Output {
        check_case_with_source_map(name, source, SourceMapCase::Package(package_source))
    }

    fn check_case_with_source_map(
        name: &str,
        source: &str,
        source_map: SourceMapCase<'_>,
    ) -> Output {
        check_case_with_options(name, source, "", None, Some(source_map))
    }

    fn check_case_with_options(
        name: &str,
        source: &str,
        aimer_metadata: &str,
        target: Option<&str>,
        source_map: Option<SourceMapCase<'_>>,
    ) -> Output {
        let fixture_dir = fixture_root().join(name);
        if fixture_dir.exists() {
            fs::remove_dir_all(&fixture_dir).unwrap();
        }
        fs::create_dir_all(fixture_dir.join("src")).unwrap();
        fs::write(
            fixture_dir.join("Cargo.toml"),
            fixture_manifest(name, aimer_metadata),
        )
        .unwrap();
        fs::write(fixture_dir.join("src/lib.rs"), source).unwrap();

        let mut command = Command::new(env!("CARGO"));
        command
            .arg("check")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(fixture_dir.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture_root().join("target"));
        if let Some(target) = target {
            command.arg("--target").arg(target);
        }
        if let Some(source_map_case) = source_map {
            let source_map = fixture_dir.join("capability-sources.toml");
            let package_name = format!("aimer_macro_{name}");
            let manifest_path = fixture_dir.join("Cargo.toml");
            let package = |source: Option<&str>| {
                let source = source
                    .map(|source| format!("source = {source:?}\n"))
                    .unwrap_or_default();
                format!(
                    "[[packages]]\nname = {package_name:?}\nmanifest_path = {manifest_path:?}\n{source}",
                )
            };
            let packages = match source_map_case {
                SourceMapCase::Package(source) => package(source),
                SourceMapCase::Missing => format!(
                    "[[packages]]\nname = \"different-package\"\nmanifest_path = {:?}\n",
                    fixture_dir.join("missing/Cargo.toml"),
                ),
                SourceMapCase::Ambiguous(source) => {
                    let package = package(source);
                    format!("{package}\n{package}")
                }
            };
            fs::write(&source_map, format!("version = 1\n\n{packages}")).unwrap();
            command.env("AIMER_CAPABILITY_PACKAGE_SOURCE_MAP", source_map);
        } else {
            command.env_remove("AIMER_CAPABILITY_PACKAGE_SOURCE_MAP");
        }
        command.output().unwrap()
    }

    #[derive(Clone, Copy)]
    enum SourceMapCase<'a> {
        Package(Option<&'a str>),
        Missing,
        Ambiguous(Option<&'a str>),
    }

    fn fixture_manifest(name: &str, aimer_metadata: &str) -> String {
        let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = macro_crate.join("../..");
        format!(
            r#"[package]
name = "aimer_macro_{name}"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
aimer_macro = {{ path = {:?} }}
aimer_anteros = {{ path = {:?} }}

[package.metadata.aimer]
{aimer_metadata}
"#,
            macro_crate,
            workspace_root.join("aimer_anteros")
        )
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/aimer_macro_capability_compile")
    }

    fn assert_success(output: &Output) {
        assert!(
            output.status.success(),
            "fixture failed to compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn assert_failure(output: &Output, diagnostic: &str) {
        assert!(!output.status.success(), "fixture unexpectedly compiled");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(diagnostic),
            "expected diagnostic `{diagnostic}` but compiler reported:\n{stderr}"
        );
    }
}

#[cfg(all(feature = "hot-reload", test))]
mod portable_guest_compile_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    #[cfg(feature = "portable-guest")]
    fn derived_widgets_lower_properties_and_children_through_portable_widget() {
        let fixture = fixture_root();
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), FIXTURE_SOURCE).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(["test", "--quiet", "--features", "portable-guest"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "portable guest fixture failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn fixture_manifest() -> String {
        let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = macro_crate.join("../..");
        format!(
            r#"[package]
name = "portable_guest_fixture"
version = "0.0.0"
edition = "2024"

[workspace]

[features]
portable-guest = []

[dependencies]
aimer_macro = {{ path = {:?} }}
aimer_widget = {{ path = {:?}, features = ["portable-guest"] }}
aimer_anteros = {{ path = {:?} }}
"#,
            macro_crate,
            workspace_root.join("framework/widget"),
            workspace_root.join("aimer_anteros"),
        )
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/aimer_macro_portable_guest_compile")
    }

    const FIXTURE_SOURCE: &str = r#"
use aimer_anteros::{PropertyValue, Version, WidgetSchemaId};
use aimer_macro::PortableWidget;
use aimer_widget::base::BuildContext;
use aimer_widget::portable::{
    PortableBuildContext, PortableLimits, PortableWidgetLimits, PortableWidgetSchema,
    SourceFingerprint, StableId128,
};
use aimer_widget::{
    AnyElement, PortableWidget as PortableWidgetCapability, RequiredChild, Widget,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Leaf;

impl Widget for Leaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

impl PortableWidgetCapability for Leaf {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        context: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<
        aimer_widget::portable::PortableNodeId,
        aimer_widget::portable::PortableBuildError,
    >
    where
        Self: Sized + Widget,
    {
        context.push_node(
            WidgetSchemaId::from_canonical_name("portable_guest_fixture::Leaf"),
            Version::new(1, 0),
            None,
            source,
            &[],
            &[],
        )
    }
}

#[derive(PortableWidget)]
#[portable_widget(id = "portable_guest_fixture::DerivedGuest")]
struct DerivedGuest<W> {
    title: String,
    count: Option<u32>,
    #[portable_child]
    child: W,
}

impl DerivedGuest<RequiredChild> {
    fn new() -> Self {
        Self {
            title: String::new(),
            count: None,
            child: RequiredChild,
        }
    }
}

impl<W> DerivedGuest<W> {
    fn title(mut self, title: String) -> Self {
        self.title = title;
        self
    }

    fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }

    fn child<N: Widget>(self, child: N) -> DerivedGuest<N> {
        DerivedGuest {
            title: self.title,
            count: self.count,
            child,
        }
    }
}

impl<W: Widget + 'static> Widget for DerivedGuest<W> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

struct RecordingLeaf(Rc<RefCell<Option<SourceFingerprint>>>);

impl Widget for RecordingLeaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

impl PortableWidgetCapability for RecordingLeaf {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        context: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<
        aimer_widget::portable::PortableNodeId,
        aimer_widget::portable::PortableBuildError,
    >
    where
        Self: Sized + Widget,
    {
        *self.0.borrow_mut() = Some(source);
        context.push_node(
            WidgetSchemaId::from_canonical_name("portable_guest_fixture::RecordingLeaf"),
            Version::new(1, 0),
            None,
            source,
            &[],
            &[],
        )
    }
}

#[derive(PortableWidget)]
#[portable_widget(id = "portable_guest_fixture::ReorderedGuest", schema_only)]
struct ReorderedGuestFirst<W> {
    title: String,
    #[portable_child]
    child: W,
    count: Option<u32>,
}

#[derive(PortableWidget)]
#[portable_widget(id = "portable_guest_fixture::ReorderedGuest", schema_only)]
struct ReorderedGuestSecond<W> {
    #[portable_child]
    child: W,
    count: Option<u32>,
    title: String,
}

impl<W: Widget + 'static> Widget for ReorderedGuestFirst<W> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

impl<W: Widget + 'static> Widget for ReorderedGuestSecond<W> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

#[derive(PortableWidget)]
#[portable_widget(schema_only)]
struct OptionalGuest<W> {
    #[portable_child(optional)]
    child: Option<W>,
}

impl<W: Widget + 'static> Widget for OptionalGuest<W> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

#[derive(PortableWidget)]
#[portable_widget(schema_only)]
struct CollectionGuest<W> {
    #[portable_children]
    children: Vec<W>,
}

impl<W: Widget + 'static> Widget for CollectionGuest<W> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

static CALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);

fn record_callback() {
    CALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);
}

#[derive(PortableWidget)]
#[portable_widget(schema_only)]
struct CallbackGuest {
    #[portable_callback(version = "1.2", max_bindings = 2)]
    on_press: Option<fn()>,
}

impl Widget for CallbackGuest {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the portable guest fixture does not build native elements")
    }
}

#[test]
fn derived_guest_lowering_emits_the_reflected_node() {
    let mut context = PortableBuildContext::new(
        7,
        0,
        PortableWidgetLimits::new(8, 16, 8, 8, 64, 4_096),
        PortableLimits::new(8, 16, 64, 128, 4_096),
    )
    .unwrap();
    let source = SourceFingerprint::new(StableId128::from_u128(11));
    let node = PortableWidgetCapability::to_portable_node(
        DerivedGuest {
            title: String::from("hello"),
            count: Some(7),
            child: Leaf,
        },
        &mut context,
        source,
    )
    .unwrap();
    let graph = context.finish_graph(node).unwrap();
    let view = graph.node(node).unwrap();
    let schema = <DerivedGuest<Leaf> as PortableWidgetSchema>::SCHEMA;

    assert_eq!(view.widget_type(), schema.widget().id());
    assert_eq!(view.widget_schema(), Version::new(1, 0));
    assert_eq!(view.properties().len(), 2);
    assert_eq!(view.children().count(), 1);
    assert!(view.properties().iter().any(|property| {
        property.property_id() == schema.properties()[0].id()
            && property.value() == PropertyValue::StringRef(0)
    }));
    assert!(view.properties().iter().any(|property| {
        property.property_id() == schema.properties()[1].id()
            && property.value() == PropertyValue::I64(7)
    }));
    assert_eq!(graph.string(0), Some("hello"));
}

#[test]
fn derived_guest_lowering_preserves_optional_and_collection_children() {
    let source = SourceFingerprint::new(StableId128::from_u128(12));
    let mut optional_context = PortableBuildContext::new(
        7,
        0,
        PortableWidgetLimits::new(8, 16, 8, 8, 64, 4_096),
        PortableLimits::new(8, 16, 64, 128, 4_096),
    )
    .unwrap();
    let optional = PortableWidgetCapability::to_portable_node(
        OptionalGuest { child: Some(Leaf) },
        &mut optional_context,
        source,
    )
    .unwrap();
    let optional_graph = optional_context.finish_graph(optional).unwrap();
    assert_eq!(optional_graph.node(optional).unwrap().children().count(), 1);

    let mut collection_context = PortableBuildContext::new(
        7,
        0,
        PortableWidgetLimits::new(8, 16, 8, 8, 64, 4_096),
        PortableLimits::new(8, 16, 64, 128, 4_096),
    )
    .unwrap();
    let collection = PortableWidgetCapability::to_portable_node(
        CollectionGuest {
            children: vec![Leaf, Leaf],
        },
        &mut collection_context,
        source,
    )
    .unwrap();
    let collection_graph = collection_context.finish_graph(collection).unwrap();
    assert_eq!(collection_graph.node_count(), 3);
    assert_eq!(collection_graph.node(collection).unwrap().children().count(), 2);
}

#[test]
fn derived_guest_lowering_keeps_field_reordering_semantically_stable() {
    let source = SourceFingerprint::new(StableId128::from_u128(14));
    let first_child_source = Rc::new(RefCell::new(None));
    let mut first_context = PortableBuildContext::new(
        7,
        0,
        PortableWidgetLimits::new(8, 16, 8, 8, 64, 4_096),
        PortableLimits::new(8, 16, 64, 128, 4_096),
    )
    .unwrap();
    let first = PortableWidgetCapability::to_portable_node(
        ReorderedGuestFirst {
            title: String::from("same"),
            child: RecordingLeaf(Rc::clone(&first_child_source)),
            count: Some(7),
        },
        &mut first_context,
        source,
    )
    .unwrap();
    let first_graph = first_context.finish_graph(first).unwrap();

    let second_child_source = Rc::new(RefCell::new(None));
    let mut second_context = PortableBuildContext::new(
        7,
        0,
        PortableWidgetLimits::new(8, 16, 8, 8, 64, 4_096),
        PortableLimits::new(8, 16, 64, 128, 4_096),
    )
    .unwrap();
    let second = PortableWidgetCapability::to_portable_node(
        ReorderedGuestSecond {
            child: RecordingLeaf(Rc::clone(&second_child_source)),
            count: Some(7),
            title: String::from("same"),
        },
        &mut second_context,
        source,
    )
    .unwrap();
    let second_graph = second_context.finish_graph(second).unwrap();

    let first_schema = <ReorderedGuestFirst<RecordingLeaf> as PortableWidgetSchema>::SCHEMA;
    let second_schema = <ReorderedGuestSecond<RecordingLeaf> as PortableWidgetSchema>::SCHEMA;
    assert_eq!(first_schema.widget().id(), second_schema.widget().id());
    let mut first_schema_properties = first_schema
        .properties()
        .iter()
        .map(|property| property.id())
        .collect::<Vec<_>>();
    let mut second_schema_properties = second_schema
        .properties()
        .iter()
        .map(|property| property.id())
        .collect::<Vec<_>>();
    first_schema_properties.sort_unstable();
    second_schema_properties.sort_unstable();
    assert_eq!(first_schema_properties, second_schema_properties);

    let mut first_properties = first_graph
            .node(first)
            .unwrap()
            .properties()
            .iter()
            .map(|property| (property.property_id(), property.value()))
            .collect::<Vec<_>>();
    let mut second_properties = second_graph
            .node(second)
            .unwrap()
            .properties()
            .iter()
            .map(|property| (property.property_id(), property.value()))
            .collect::<Vec<_>>();
    first_properties.sort_unstable_by_key(|(property, _)| *property);
    second_properties.sort_unstable_by_key(|(property, _)| *property);
    assert_eq!(first_properties, second_properties);
    assert_eq!(
        *first_child_source.borrow(),
        *second_child_source.borrow(),
    );
}

#[test]
fn derived_guest_lowering_binds_callbacks_from_schema_metadata() {
    CALLBACK_CALLS.store(0, Ordering::Relaxed);
    let source = SourceFingerprint::new(StableId128::from_u128(13));
    let mut context = PortableBuildContext::new(
        7,
        0,
        PortableWidgetLimits::new(8, 16, 8, 8, 64, 4_096),
        PortableLimits::new(8, 16, 64, 128, 4_096),
    )
    .unwrap();
    let node = PortableWidgetCapability::to_portable_node(
        CallbackGuest {
            on_press: Some(record_callback),
        },
        &mut context,
        source,
    )
    .unwrap();
    let schema = <CallbackGuest as PortableWidgetSchema>::SCHEMA;
    let callback_id = context.callback_id_for(None, source, schema.callbacks()[0].id());
    let graph = context.finish_graph(node).unwrap();
    assert_eq!(graph.node(node).unwrap().callbacks().len(), 1);

    let registry = context.callback_registry();
    registry.dispatch(callback_id, &mut context).unwrap();
    assert_eq!(CALLBACK_CALLS.load(Ordering::Relaxed), 1);
}
"#;
}

#[cfg(test)]
mod portable_schema_compile_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[cfg(feature = "hot-reload")]
    #[test]
    fn derive_generates_stable_widget_property_and_child_metadata() {
        let fixture = fixture_root();
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), FIXTURE_SOURCE).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(["test", "--quiet"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "portable schema fixture failed:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn derive_rejects_an_unreflected_unannotated_field() {
        let fixture = fixture_root().with_file_name("aimer_macro_portable_schema_unsupported");
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), UNSUPPORTED_SOURCE).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success());
        assert!(diagnostic.contains("NativePaint"), "{diagnostic}");
        assert!(diagnostic.contains("u64"), "{diagnostic}");
        assert!(diagnostic.contains("PortableProperty"), "{diagnostic}");
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn derive_reports_a_missing_guest_property_codec() {
        let fixture = fixture_root().with_file_name("aimer_macro_portable_guest_codec_unsupported");
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), UNSUPPORTED_GUEST_CODEC_SOURCE).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet", "--features", "portable-guest"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success());
        assert!(diagnostic.contains("MissingCodec"), "{diagnostic}");
        assert!(
            diagnostic.contains("PortableEncodeProperty"),
            "{diagnostic}"
        );
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn automatic_materializer_rejects_callbacks_with_the_manual_hint() {
        let fixture = fixture_root().with_file_name("aimer_macro_portable_materializer_callback");
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), CALLBACK_MATERIALIZER_SOURCE).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success());
        assert!(
            diagnostic.contains("does not support callbacks"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("materializer = path"), "{diagnostic}");
    }

    #[cfg(feature = "hot-reload")]
    #[test]
    fn automatic_materializer_rejects_non_single_child_shapes_with_the_manual_hint() {
        for (name, source, expected) in [
            (
                "optional_child",
                OPTIONAL_CHILD_MATERIALIZER_SOURCE,
                "does not support an optional child",
            ),
            (
                "child_collection",
                CHILD_COLLECTION_MATERIALIZER_SOURCE,
                "does not support `#[portable_children]`",
            ),
            (
                "conflicting_options",
                CONFLICTING_MATERIALIZER_OPTIONS_SOURCE,
                "cannot be combined",
            ),
        ] {
            let fixture = fixture_root().with_file_name(format!("aimer_macro_{name}"));
            if fixture.exists() {
                fs::remove_dir_all(&fixture).unwrap();
            }
            fs::create_dir_all(fixture.join("src")).unwrap();
            fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
            fs::write(fixture.join("src/lib.rs"), source).unwrap();

            let output = Command::new(env!("CARGO"))
                .args(["check", "--quiet"])
                .arg("--manifest-path")
                .arg(fixture.join("Cargo.toml"))
                .env("CARGO_TARGET_DIR", fixture.join("target"))
                .output()
                .unwrap();
            let diagnostic = String::from_utf8_lossy(&output.stderr);

            assert!(!output.status.success());
            assert!(diagnostic.contains(expected), "{diagnostic}");
            if name != "conflicting_options" {
                assert!(diagnostic.contains("materializer = path"), "{diagnostic}");
            }
        }
    }

    fn fixture_manifest() -> String {
        let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = macro_crate.join("../..");
        format!(
            r#"[package]
name = "portable_schema_fixture"
version = "0.0.0"
edition = "2024"

[workspace]

[features]
portable-guest = []

[dependencies]
aimer_macro = {{ path = {:?} }}
aimer_widget = {{ path = {:?}, features = ["portable-guest"] }}
aimer_anteros = {{ path = {:?} }}
"#,
            macro_crate,
            workspace_root.join("framework/widget"),
            workspace_root.join("aimer_anteros"),
        )
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/aimer_macro_portable_schema_compile")
    }

    const FIXTURE_SOURCE: &str = r#"
use aimer_anteros::{
    ChildCardinality, ModelLimits, PropertyPresence, PropertyValue, PropertyValueKind,
    ValueSchemaMetadata, Version, WidgetDocument, WidgetDocumentView, WidgetNode, WidgetProperty,
    stable_schema_hash64, validate_portable_widget_schema_metadata,
};
use aimer_macro::PortableWidget;
use aimer_widget::base::BuildContext;
use aimer_widget::portable::{
    PortableMaterializeError, PortableNativeWidget, PortableProperty,
    PortablePropertyConversion, PortablePropertyReflection, PortableWidgetSchema,
};
use aimer_widget::{AnyElement, AnyWidget, RequiredChild, Widget};
use std::cell::RefCell;

thread_local! {
    static BUILD_LOG: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn log_build(step: &'static str) {
    BUILD_LOG.with(|log| log.borrow_mut().push(step));
}

fn clear_build_log() {
    BUILD_LOG.with(|log| log.borrow_mut().clear());
}

fn build_log() -> Vec<&'static str> {
    BUILD_LOG.with(|log| log.borrow().clone())
}

struct PointList;

impl PortableProperty for PointList {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::custom(
        ValueSchemaMetadata::from_canonical_name(
        "aimer.value:portable_schema_fixture::PointList",
        Version::new(1, 0),
        4_096,
    ));
}

struct NativeCache;

#[derive(PortableWidget)]
#[portable_widget(schema_only)]
struct Card<W> {
    title: String,
    count: Option<u32>,
    points: PointList,
    #[portable_callback(
        version = "1.2",
        max_bindings = 2,
        async_version = "2.1",
        max_async_tasks = 3,
        max_completion_bytes = 9,
        max_callback_fuel = 17,
        max_retained_resources = 5,
    )]
    on_press: fn(),
    #[portable_skip]
    native_cache: NativeCache,
    #[portable_child]
    child: W,
}

#[derive(PortableWidget)]
#[portable_widget(id = "example.pinned", version = "2.3", schema_only)]
struct Pinned {
    value: u32,
}

#[derive(PortableWidget)]
#[portable_widget(schema_only)]
struct Stack<W> {
    #[portable_children]
    children: Vec<W>,
}

#[derive(PortableWidget)]
struct BuilderProbe<W = RequiredChild> {
    count: u8,
    label: Option<String>,
    #[portable_child]
    child: W,
}

impl BuilderProbe {
    fn new() -> Self {
        log_build("new");
        Self { count: 0, label: None, child: RequiredChild }
    }
}

impl<W> BuilderProbe<W> {
    fn count(mut self, count: u8) -> Self {
        log_build("count");
        self.count = count;
        self
    }

    fn label(mut self, label: String) -> Self {
        log_build("label");
        self.label = Some(label);
        self
    }

    fn child<N: Widget>(self, child: N) -> BuilderProbe<N> {
        log_build("child");
        BuilderProbe { count: self.count, label: self.label, child }
    }
}

impl<W: Widget + 'static> Widget for BuilderProbe<W> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the derive test observes construction before element creation")
    }
}

#[allow(non_snake_case)]
#[derive(PortableWidget)]
struct CaseProbe {
    foo: u8,
    FOO: u8,
}

#[allow(non_snake_case)]
impl CaseProbe {
    fn new() -> Self {
        Self { foo: 0, FOO: 0 }
    }

    fn foo(mut self, value: u8) -> Self {
        self.foo = value;
        self
    }

    fn FOO(mut self, value: u8) -> Self {
        self.FOO = value;
        self
    }
}

impl Widget for CaseProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the case-distinct identity fixture is compile-only")
    }
}

struct Leaf;

impl Widget for Leaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("the derive test does not create elements")
    }
}

impl aimer_widget::PortableWidget for Leaf {}

#[derive(PortableWidget)]
#[portable_widget(materializer = manual_probe)]
struct ManualProbe {
    value: u32,
}

fn manual_probe(
    _document: &WidgetDocumentView<'_>,
    _node: aimer_anteros::WidgetNodeView<'_>,
    _children: Vec<AnyWidget>,
) -> Result<AnyWidget, PortableMaterializeError> {
    log_build("manual");
    Ok(Leaf.boxed())
}

#[test]
fn generated_metadata_is_canonical_and_reflected() {
    let schema = <Card<()> as PortableWidgetSchema>::SCHEMA;
    let widget = schema.widget();
    assert_eq!(widget.canonical_name(), "aimer.widget:portable_schema_fixture::Card");
    assert_eq!(widget.id().value(), stable_schema_hash64(widget.canonical_name()));
    assert_eq!(widget.min_version(), Version::new(1, 0));
    assert_eq!(schema.children(), ChildCardinality::exactly(1));
    validate_portable_widget_schema_metadata(&[schema]).unwrap();

    let properties = schema.properties();
    assert_eq!(properties.len(), 3);
    assert_eq!(properties[0].value_kind(), PropertyValueKind::StringRef);
    assert_eq!(properties[0].presence(), PropertyPresence::Required);
    assert_eq!(properties[1].value_kind(), PropertyValueKind::I64);
    assert_eq!(properties[1].presence(), PropertyPresence::Optional);
    assert_eq!(properties[2].value_kind(), PropertyValueKind::BlobRef);
    assert_eq!(properties[2].value_schema().unwrap().maximum_encoded_bytes(), 4_096);
    assert_eq!(
        <PointList as PortableProperty>::REFLECTION.conversion(),
        PortablePropertyConversion::CustomValue,
    );
    for property in properties {
        assert_eq!(property.id().value(), stable_schema_hash64(property.canonical_name()));
    }
    assert_eq!(schema.callbacks().len(), 1);
    assert!(schema.callbacks()[0].canonical_name().ends_with("Card:on_press"));
    assert_eq!(schema.callbacks()[0].event_schema(), Version::new(1, 2));
    assert_eq!(schema.callbacks()[0].maximum_bindings(), 2);
    let async_schema = schema.callbacks()[0].async_schema().unwrap();
    assert_eq!(async_schema.contract_version(), Version::new(2, 1));
    assert_eq!(async_schema.maximum_in_flight_tasks(), 3);
    assert_eq!(async_schema.maximum_completion_bytes(), 9);
    assert_eq!(async_schema.maximum_callback_fuel(), 17);
    assert_eq!(async_schema.maximum_retained_resources(), 5);

    let pinned = <Pinned as PortableWidgetSchema>::SCHEMA.widget();
    assert_eq!(pinned.canonical_name(), "aimer.widget:example.pinned");
    assert_eq!(pinned.min_version(), Version::new(2, 3));
    assert_eq!(
        <Pinned as PortableWidgetSchema>::SCHEMA.properties()[0].canonical_name(),
        "aimer.property:example.pinned:value",
    );

    let stack_children = <Stack<()> as PortableWidgetSchema>::SCHEMA.children();
    assert_eq!(stack_children.minimum(), 0);
    assert_eq!(stack_children.maximum(), u32::MAX);
}

#[test]
fn generated_materializer_builds_properties_before_the_last_child_and_allows_override() {
    clear_build_log();
    let schema = <BuilderProbe<RequiredChild> as PortableWidgetSchema>::SCHEMA;
    let properties = [
        WidgetProperty::new(schema.properties()[0].id(), PropertyValue::I64(7)),
        WidgetProperty::new(schema.properties()[1].id(), PropertyValue::StringRef(0)),
    ];
    let children = [1];
    let nodes = [
        WidgetNode::new(schema.widget().id(), Version::new(1, 0))
            .properties(&properties)
            .children(&children),
        WidgetNode::new(schema.widget().id(), Version::new(1, 0)),
    ];
    let limits = ModelLimits::new(4_096, 8, 16, 16).max_widget_depth(4);
    let image = WidgetDocument::new(0, 0, 0, &nodes, &["ready"], &[])
        .encode(limits)
        .unwrap();
    let document = WidgetDocumentView::decode(&image, limits).unwrap();

    let _widget = <BuilderProbe<RequiredChild> as PortableNativeWidget>::materialize_widget(
        &document,
        document.node(0).unwrap(),
        vec![Leaf.boxed()],
    )
    .unwrap();
    assert_eq!(build_log(), ["new", "count", "label", "child"]);

    clear_build_log();
    let child_result = <BuilderProbe<RequiredChild> as PortableNativeWidget>::materialize_widget(
        &document,
        document.node(0).unwrap(),
        Vec::new(),
    );
    let child_error = match child_result {
        Ok(_) => panic!("missing child unexpectedly materialized"),
        Err(error) => error,
    };
    assert_eq!(
        child_error,
        PortableMaterializeError::InvalidChildCount { expected: 1, actual: 0 },
    );
    assert!(build_log().is_empty());

    let extra_child_result =
        <BuilderProbe<RequiredChild> as PortableNativeWidget>::materialize_widget(
            &document,
            document.node(0).unwrap(),
            vec![Leaf.boxed(), Leaf.boxed()],
        );
    let extra_child_error = match extra_child_result {
        Ok(_) => panic!("extra child unexpectedly materialized"),
        Err(error) => error,
    };
    assert_eq!(
        extra_child_error,
        PortableMaterializeError::InvalidChildCount { expected: 1, actual: 2 },
    );
    assert!(build_log().is_empty());

    clear_build_log();
    let default_properties = [WidgetProperty::new(
        schema.properties()[0].id(),
        PropertyValue::I64(8),
    )];
    let default_nodes = [
        WidgetNode::new(schema.widget().id(), Version::new(1, 0))
            .properties(&default_properties)
            .children(&children),
        WidgetNode::new(schema.widget().id(), Version::new(1, 0)),
    ];
    let default_image = WidgetDocument::new(0, 0, 0, &default_nodes, &[], &[])
        .encode(limits)
        .unwrap();
    let default_document = WidgetDocumentView::decode(&default_image, limits).unwrap();
    let _widget = <BuilderProbe<RequiredChild> as PortableNativeWidget>::materialize_widget(
        &default_document,
        default_document.node(0).unwrap(),
        vec![Leaf.boxed()],
    )
    .unwrap();
    assert_eq!(build_log(), ["new", "count", "child"]);

    clear_build_log();
    let invalid_properties = [
        WidgetProperty::new(schema.properties()[0].id(), PropertyValue::I64(256)),
        WidgetProperty::new(schema.properties()[1].id(), PropertyValue::StringRef(0)),
    ];
    let invalid_nodes = [
        WidgetNode::new(schema.widget().id(), Version::new(1, 0))
            .properties(&invalid_properties)
            .children(&children),
        WidgetNode::new(schema.widget().id(), Version::new(1, 0)),
    ];
    let invalid_image = WidgetDocument::new(0, 0, 0, &invalid_nodes, &["ready"], &[])
        .encode(limits)
        .unwrap();
    let invalid_document = WidgetDocumentView::decode(&invalid_image, limits).unwrap();
    assert!(matches!(
        <BuilderProbe<RequiredChild> as PortableNativeWidget>::materialize_widget(
            &invalid_document,
            invalid_document.node(0).unwrap(),
            vec![Leaf.boxed()],
        ),
        Err(PortableMaterializeError::InvalidPropertyValue { .. })
    ));
    assert!(build_log().is_empty());

    clear_build_log();
    let manual_schema = <ManualProbe as PortableWidgetSchema>::SCHEMA;
    let manual_nodes = [WidgetNode::new(manual_schema.widget().id(), Version::new(1, 0))];
    let manual_image = WidgetDocument::new(0, 0, 0, &manual_nodes, &[], &[])
        .encode(limits)
        .unwrap();
    let manual_document = WidgetDocumentView::decode(&manual_image, limits).unwrap();
    let _widget = <ManualProbe as PortableNativeWidget>::materialize_widget(
        &manual_document,
        manual_document.node(0).unwrap(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(build_log(), ["manual"]);
}
"#;

    const UNSUPPORTED_SOURCE: &str = r#"
use aimer_macro::PortableWidget;

struct NativePaint;

#[derive(PortableWidget)]
struct Card {
    decoration: NativePaint,
    large_count: u64,
}
"#;

    const UNSUPPORTED_GUEST_CODEC_SOURCE: &str = r#"
use aimer_anteros::{ValueSchemaMetadata, Version};
use aimer_macro::PortableWidget;
use aimer_widget::base::BuildContext;
use aimer_widget::portable::{PortableProperty, PortablePropertyReflection};
use aimer_widget::{AnyElement, Widget};

struct MissingCodec;

impl PortableProperty for MissingCodec {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::custom(
        ValueSchemaMetadata::from_canonical_name(
            "aimer.value:portable_schema_fixture::MissingCodec",
            Version::new(1, 0),
            32,
        ),
    );
}

#[derive(PortableWidget)]
#[portable_widget(schema_only)]
struct MissingGuestCodec {
    value: MissingCodec,
}

impl Widget for MissingGuestCodec {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("compile-only fixture")
    }
}
"#;

    const CALLBACK_MATERIALIZER_SOURCE: &str = r#"
use aimer_macro::PortableWidget;

#[derive(PortableWidget)]
struct CallbackWidget {
    #[portable_callback]
    on_press: fn(),
}
"#;

    const OPTIONAL_CHILD_MATERIALIZER_SOURCE: &str = r#"
use aimer_macro::PortableWidget;

#[derive(PortableWidget)]
struct OptionalChildWidget<W> {
    #[portable_child(optional)]
    child: W,
}
"#;

    const CHILD_COLLECTION_MATERIALIZER_SOURCE: &str = r#"
use aimer_macro::PortableWidget;

#[derive(PortableWidget)]
struct ChildCollectionWidget<W> {
    #[portable_children]
    children: Vec<W>,
}
"#;

    const CONFLICTING_MATERIALIZER_OPTIONS_SOURCE: &str = r#"
use aimer_macro::PortableWidget;

#[derive(PortableWidget)]
#[portable_widget(schema_only, materializer = custom)]
struct ConflictingWidget;

fn custom() {}
"#;
}

#[cfg(all(feature = "hot-reload", test))]
mod portable_widget_compile_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    #[test]
    fn generated_widgets_use_the_portable_runtime_lifecycle() {
        let output = run_fixture();

        assert!(
            output.status.success(),
            "portable widget fixture failed:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );

        let native = run_fixture_command(&[
            "test",
            "--no-run",
            "--quiet",
            "--no-default-features",
            "--features",
            "unsupported-state",
        ]);
        assert!(
            native.status.success(),
            "unsupported state affected the native expansion:\n{}",
            String::from_utf8_lossy(&native.stderr),
        );

        let portable = run_fixture_command(&[
            "test",
            "--no-run",
            "--quiet",
            "--features",
            "unsupported-state",
        ]);
        assert!(
            !portable.status.success(),
            "unsupported portable state unexpectedly compiled"
        );
        let diagnostic = String::from_utf8_lossy(&portable.stderr);
        for required_bound in ["AimerReflectionType", "PortableApply", "PortableEncode"] {
            assert!(
                diagnostic.contains(required_bound),
                "missing portable bound diagnostic `{required_bound}`:\n{diagnostic}",
            );
        }
    }

    fn run_fixture() -> Output {
        let fixture = fixture_root();
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), FIXTURE_SOURCE).unwrap();

        run_fixture_command(&["test", "--quiet"])
    }

    fn run_fixture_command(arguments: &[&str]) -> Output {
        Command::new(env!("CARGO"))
            .args(arguments)
            .arg("--manifest-path")
            .arg(fixture_root().join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture_root().join("target"))
            .output()
            .unwrap()
    }

    fn fixture_manifest() -> String {
        let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = macro_crate.join("../..");
        format!(
            r#"[package]
name = "aimer_macro_portable_widget"
version = "0.0.0"
edition = "2024"

[workspace]

[features]
default = ["portable-guest"]
portable-guest = []
unsupported-state = []

[dependencies]
aimer_macro = {{ path = {:?} }}
aimer_widget = {{ path = {:?}, features = ["portable-guest"] }}
aimer_anteros = {{ path = {:?} }}
"#,
            macro_crate,
            workspace_root.join("framework/widget"),
            workspace_root.join("aimer_anteros"),
        )
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/aimer_macro_portable_widget_compile")
    }

    const FIXTURE_SOURCE: &str = r#"
extern crate self as aimer;

pub use aimer_widget as widget;

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_anteros::{Version, WidgetSchemaId};
    use aimer_macro::{StatefulWidget, StatelessWidget};
    use aimer_widget::base::BuildContext;
    use aimer_widget::portable::{
        AimerReflectionType, Decoder, Encoder, FieldDescriptor, FieldKind, PortableApply,
        PortableBuildContext, PortableBuildError, PortableEncode, PortableLimits,
        PortableNodeId, PortableWidgetLimits, SourceFingerprint, StableId128, TypeSchema,
    };
    use aimer_widget::{
        AnyElement, State, StateUpdater, StatefulWidget as StatefulWidgetTrait,
        PortableWidget, StatelessWidget as StatelessWidgetTrait, Widget,
    };

    struct PortableLeaf;

    impl Widget for PortableLeaf {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            unreachable!("portable fixture does not build native elements")
        }
    }

    impl PortableWidget for PortableLeaf {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<PortableNodeId, PortableBuildError> {
            ctx.push_node(WidgetSchemaId::new(7), Version::new(1, 0), None, source, &[], &[])
        }
    }

    #[derive(StatelessWidget)]
    struct ErasedStateless;

    impl StatelessWidgetTrait for ErasedStateless {
        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            PortableLeaf.boxed()
        }
    }

    #[cfg(feature = "unsupported-state")]
    #[derive(StatefulWidget)]
    struct UnsupportedWidget;

    #[cfg(feature = "unsupported-state")]
    struct UnsupportedState;

    #[cfg(feature = "unsupported-state")]
    impl StatefulWidgetTrait for UnsupportedWidget {
        type State = UnsupportedState;

        fn create_state(self) -> Self::State {
            UnsupportedState
        }
    }

    #[cfg(feature = "unsupported-state")]
    impl State<UnsupportedWidget> for UnsupportedState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            PortableLeaf
        }
    }

    #[derive(StatefulWidget)]
    struct CounterWidget {
        initial: u32,
        observed: Rc<Cell<u32>>,
        mutate: bool,
    }

    struct CounterState {
        count: u32,
        observed: Rc<Cell<u32>>,
        mutate: bool,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidgetTrait for CounterWidget {
        type State = CounterState;

        fn create_state(self) -> Self::State {
            CounterState {
                count: self.initial,
                observed: self.observed,
                mutate: self.mutate,
                updater: StateUpdater::new(),
            }
        }
    }

    impl State<CounterWidget> for CounterState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        fn adopt_config_from(&mut self, new: Self) {
            self.observed = new.observed;
            self.mutate = new.mutate;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.observed.set(self.count);
            if self.mutate {
                self.updater.set_state(|state| state.count += 1);
            }
            PortableLeaf.boxed()
        }
    }

    const COUNTER_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("count", "u32", FieldKind::Retained),
    ];
    const COUNTER_SCHEMA: TypeSchema = TypeSchema::new(
        "CounterState",
        StableId128::from_path("aimer.type.v1", "fixture::CounterState"),
        COUNTER_FIELDS,
    );

    impl AimerReflectionType for CounterState {
        const TYPE_ID: StableId128 =
            StableId128::from_path("aimer.type.v1", "fixture::CounterState");

        fn schema() -> &'static TypeSchema {
            &COUNTER_SCHEMA
        }
    }

    impl PortableEncode for CounterState {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), aimer_widget::portable::EncodeError> {
            self.count.encode(encoder)
        }
    }

    impl PortableApply for CounterState {
        type Retained = u32;

        fn decode_retained(
            decoder: &mut Decoder<'_>,
        ) -> Result<Self::Retained, aimer_widget::portable::DecodeError> {
            aimer_widget::portable::PortableDecode::decode(decoder)
        }

        fn apply_retained(&mut self, retained: Self::Retained) {
            self.count = retained;
        }
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            1,
            0,
            PortableWidgetLimits::new(16, 16, 16, 16, 256, 4096),
            PortableLimits::new(16, 64, 256, 256, 4096),
        )
        .unwrap()
    }

    fn source(value: u128) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_u128(value))
    }

    #[test]
    fn stateless_build_result_can_be_an_any_widget() {
        let mut context = context();
        let root = PortableWidget::to_portable_node(ErasedStateless, &mut context, source(1)).unwrap();

        context.finish_document(root).unwrap();
    }

    #[test]
    fn state_survives_and_queued_mutation_requests_a_rebuild() {
        let mut context = context();
        let first_observed = Rc::new(Cell::new(0));
        let root = PortableWidget::to_portable_node(
            CounterWidget { initial: 1, observed: first_observed.clone(), mutate: true },
            &mut context,
            source(2),
        )
        .unwrap();
        context.finish_document(root).unwrap();

        assert_eq!(first_observed.get(), 1);
        assert!(context.take_rebuild_request());

        let second_observed = Rc::new(Cell::new(0));
        let root = PortableWidget::to_portable_node(
            CounterWidget { initial: 99, observed: second_observed.clone(), mutate: false },
            &mut context,
            source(2),
        )
        .unwrap();
        context.finish_document(root).unwrap();

        assert_eq!(second_observed.get(), 2);
    }
}
"#;
}

#[cfg(test)]
mod capability_runtime_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[cfg(feature = "hot-reload")]
    #[test]
    fn capability_runtime_expansions_preserve_external_macro_behavior() {
        let fixture = fixture_root();
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), fixture_source()).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(["test", "--quiet"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "capability runtime fixture failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn fixture_manifest() -> String {
        let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = macro_crate.join("../..");
        format!(
            r#"[package]
name = "aimer_macro_capability_runtime"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
aimer_macro = {{ path = {:?} }}
aimer_anteros = {{ path = {:?}, features = ["wasm-hot-reload"] }}
"#,
            macro_crate,
            workspace_root.join("aimer_anteros"),
        )
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/aimer_macro_capability_runtime")
    }

    fn fixture_source() -> String {
        let guest_module = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../host/anteros/tests/support/guest_module.rs");
        FIXTURE_SOURCE.replace(
            "\"__AIMER_GUEST_MODULE_PATH__\"",
            &format!("{guest_module:?}"),
        )
    }

    const FIXTURE_SOURCE: &str = r####"
mod capability_host {
use aimer_anteros::{
    CapabilityError, CapabilityLimits, CapabilityProvider, CapabilityResult, StableId128,
};

#[aimer_macro::capability(
    name = "device-control",
    id = "com.example.device-control",
    abi = 1,
    since = "1.2.0",
)]
trait DeviceControl {
    fn enabled(&self) -> CapabilityResult<bool>;
    fn label(&self, prefix: &str) -> CapabilityResult<Option<String>>;
    fn set_level(&self, level: u32) -> CapabilityResult<u64>;
}

const LIMITS: CapabilityLimits = CapabilityLimits::new(64, 64);
const EXPECTED_ID: StableId128 = StableId128::from_bytes([
    0xC8, 0x05, 0xC5, 0xFD, 0xAA, 0x5F, 0xFC, 0x0E, 0x9E, 0x91, 0x55, 0xF7, 0x5A, 0x0F, 0xFA,
    0x88,
]);

#[test]
fn generated_host_dispatches_canonical_requests_to_one_native_provider() {
    let host = DeviceControlHost::new(NativeDeviceControl, LIMITS);

    assert_eq!(host.descriptor().capability_id(), EXPECTED_ID);
    assert_eq!(host.descriptor().abi_major(), 1);
    assert_eq!(host.descriptor().limits(), LIMITS);
    assert_eq!(host.dispatch(0, &[], 64).unwrap(), [1]);
    assert_eq!(
        host.dispatch(1, &[4, 0, 0, 0, b'm', b'a', b'i', b'n'], 64)
            .unwrap(),
        [
            1, 11, 0, 0, 0, b'm', b'a', b'i', b'n', b'-', b'd', b'e', b'v', b'i', b'c', b'e',
        ]
    );
    assert_eq!(
        host.dispatch(2, &7_u32.to_le_bytes(), 64).unwrap(),
        49_u64.to_le_bytes()
    );
}

#[test]
fn generated_host_rejects_invalid_calls_and_preserves_provider_errors() {
    let host = DeviceControlHost::new(NativeDeviceControl, LIMITS);

    assert_eq!(host.dispatch(99, &[], 64), Err(CapabilityError::InvalidRequest));
    assert_eq!(
        host.dispatch(1, &[4, 0, 0, 0, b'm'], 64),
        Err(CapabilityError::InvalidRequest)
    );
    assert_eq!(
        host.dispatch(2, &0_u32.to_le_bytes(), 64),
        Err(CapabilityError::Denied)
    );
    assert_eq!(
        host.dispatch(1, &[4, 0, 0, 0, b'm', b'a', b'i', b'n'], 4),
        Err(CapabilityError::LimitExceeded)
    );

    let tight_host = DeviceControlHost::new(
        NativeDeviceControl,
        CapabilityLimits::new(3, 64),
    );
    assert_eq!(
        tight_host.dispatch(2, &7_u32.to_le_bytes(), 64),
        Err(CapabilityError::LimitExceeded)
    );
}

struct NativeDeviceControl;

impl DeviceControl for NativeDeviceControl {
    fn enabled(&self) -> CapabilityResult<bool> {
        Ok(true)
    }

    fn label(&self, prefix: &str) -> CapabilityResult<Option<String>> {
        Ok(Some(format!("{prefix}-device")))
    }

    fn set_level(&self, level: u32) -> CapabilityResult<u64> {
        if level == 0 {
            Err(CapabilityError::Denied)
        } else {
            Ok(u64::from(level) * 7)
        }
    }
}
}

mod capability_metadata {
use aimer_anteros::{CapabilityPolicy, CapabilityRequirement, CapabilityResult, StableId128};

#[aimer_macro::capability(
    name = "device-control",
    id = "com.example.device-control",
    abi = 1,
    since = "1.2.0",
)]
pub trait DeviceControl {
    fn enabled(&self) -> CapabilityResult<bool>;
    fn set_level(&self, level: u32) -> CapabilityResult<u64>;
    fn upload(&self, label: &str, payload: &[u8]) -> CapabilityResult<Option<Vec<u8>>>;
}

const EXPECTED_ID: StableId128 = StableId128::from_bytes([
    0xC8, 0x05, 0xC5, 0xFD, 0xAA, 0x5F, 0xFC, 0x0E, 0x9E, 0x91, 0x55, 0xF7, 0x5A, 0x0F, 0xFA,
    0x88,
]);
const EXPECTED_FINGERPRINT: [u8; 32] = [
    0xD4, 0x14, 0x3B, 0x39, 0xE0, 0x94, 0xC9, 0xEE, 0x04, 0xA8, 0xBA, 0xFC, 0x93, 0x1A, 0xA8,
    0x64, 0x2E, 0xA4, 0x74, 0xE7, 0x33, 0xA4, 0x2F, 0x41, 0x8C, 0x15, 0x2F, 0x63, 0xC2, 0x90,
    0xEF, 0xF9,
];

#[test]
fn capability_exposes_stable_manifest_metadata() {
    assert_eq!(
        DeviceControlCapability::CANONICAL_ID,
        "com.example.device-control"
    );
    assert_eq!(DeviceControlCapability::ID, EXPECTED_ID);
    assert_eq!(DeviceControlCapability::ABI_MAJOR, 1);
    assert_eq!(DeviceControlCapability::SINCE, "1.2.0");
    assert_eq!(
        DeviceControlCapability::CONTRACT_FINGERPRINT,
        EXPECTED_FINGERPRINT
    );
    assert_eq!(
        DeviceControlCapability::requirement(CapabilityPolicy::Required),
        CapabilityRequirement::new(
            EXPECTED_ID,
            1,
            CapabilityPolicy::Required,
            EXPECTED_FINGERPRINT,
        )
    );
}
}

mod capability_parity {
use aimer_anteros::CapabilityResult;

mod original {
    use super::CapabilityResult;

    #[aimer_macro::capability(
        name = "device-control",
        id = "com.example.device-control",
        abi = 1,
        since = "1.2.0",
    )]
    pub trait DeviceControl {
        fn enabled(&self) -> CapabilityResult<bool> {
            Ok(true)
        }

        fn set_level(&self, level: u32) -> CapabilityResult<u64> {
            Ok(u64::from(level))
        }

        fn upload(&self, label: &str, payload: &[u8]) -> CapabilityResult<Option<Vec<u8>>> {
            let _ = (label, payload);
            Ok(None)
        }
    }

    pub struct Provider;

    impl DeviceControl for Provider {}
}

mod reordered_and_reimplemented {
    use super::CapabilityResult;

    #[aimer_macro::capability(
        name = "device-control",
        id = "com.example.device-control",
        abi = 1,
        since = "9.9.9",
    )]
    pub trait DeviceControl {
        fn upload(&self, label: &str, payload: &[u8]) -> CapabilityResult<Option<Vec<u8>>> {
            let mut value = label.as_bytes().to_vec();
            value.extend_from_slice(payload);
            Ok(Some(value))
        }

        fn set_level(&self, level: u32) -> CapabilityResult<u64> {
            Ok(u64::from(level) * 7)
        }

        fn enabled(&self) -> CapabilityResult<bool> {
            Ok(false)
        }
    }

    pub struct Provider;

    impl DeviceControl for Provider {}
}

mod changed_contract {
    use super::CapabilityResult;

    #[aimer_macro::capability(
        name = "device-control",
        id = "com.example.device-control",
        abi = 1,
        since = "1.2.0",
    )]
    pub trait DeviceControl {
        fn enabled(&self) -> CapabilityResult<bool>;
        fn set_level(&self, level: u64) -> CapabilityResult<u64>;
        fn upload(&self, label: &str, payload: &[u8]) -> CapabilityResult<Option<Vec<u8>>>;
    }

    pub struct Provider;

    impl DeviceControl for Provider {
        fn enabled(&self) -> CapabilityResult<bool> {
            Ok(true)
        }

        fn set_level(&self, level: u64) -> CapabilityResult<u64> {
            Ok(level)
        }

        fn upload(&self, label: &str, payload: &[u8]) -> CapabilityResult<Option<Vec<u8>>> {
            let _ = (label, payload);
            Ok(None)
        }
    }
}

#[test]
fn method_order_sdk_metadata_and_implementation_do_not_change_the_contract() {
    assert!(original::DeviceControl::enabled(&original::Provider).unwrap());
    assert_eq!(
        original::DeviceControl::set_level(&original::Provider, 3).unwrap(),
        3
    );
    assert_eq!(
        original::DeviceControl::upload(&original::Provider, "main", &[1]).unwrap(),
        None
    );
    assert!(!reordered_and_reimplemented::DeviceControl::enabled(
        &reordered_and_reimplemented::Provider
    )
    .unwrap());
    assert_eq!(
        reordered_and_reimplemented::DeviceControl::set_level(
            &reordered_and_reimplemented::Provider,
            3,
        )
        .unwrap(),
        21
    );
    assert_eq!(
        reordered_and_reimplemented::DeviceControl::upload(
            &reordered_and_reimplemented::Provider,
            "main",
            &[1],
        )
        .unwrap(),
        Some(vec![b'm', b'a', b'i', b'n', 1])
    );
    assert_eq!(
        original::DeviceControlCapability::ID,
        reordered_and_reimplemented::DeviceControlCapability::ID
    );
    assert_eq!(
        original::DeviceControlCapability::CONTRACT_FINGERPRINT,
        reordered_and_reimplemented::DeviceControlCapability::CONTRACT_FINGERPRINT
    );
}

#[test]
fn a_wire_signature_change_changes_only_the_contract_fingerprint() {
    assert!(changed_contract::DeviceControl::enabled(&changed_contract::Provider).unwrap());
    assert_eq!(
        changed_contract::DeviceControl::set_level(&changed_contract::Provider, 3).unwrap(),
        3
    );
    assert_eq!(
        changed_contract::DeviceControl::upload(&changed_contract::Provider, "main", &[1])
            .unwrap(),
        None
    );
    assert_eq!(
        original::DeviceControlCapability::ID,
        changed_contract::DeviceControlCapability::ID
    );
    assert_ne!(
        original::DeviceControlCapability::CONTRACT_FINGERPRINT,
        changed_contract::DeviceControlCapability::CONTRACT_FINGERPRINT
    );
}
}

mod capability_proxy {
use std::cell::RefCell;
use std::rc::Rc;

use aimer_anteros::{
    CapabilityCall, CapabilityError, CapabilityLimits, CapabilityResult, CapabilityTransport,
};

#[aimer_macro::capability(
    name = "device-control",
    id = "com.example.device-control",
    abi = 1,
    since = "1.2.0",
)]
pub trait DeviceControl {
    fn enabled(&self) -> CapabilityResult<bool>;
    fn label(&self, prefix: &str) -> CapabilityResult<Option<String>>;
    fn set_level(&self, level: u32) -> CapabilityResult<u64>;
}

#[test]
fn guest_proxy_encodes_calls_and_decodes_bounded_responses() {
    let transport = RecordingTransport::default();
    let calls = Rc::clone(&transport.calls);
    let proxy = DeviceControlGuest::new(transport, CapabilityLimits::new(64, 64));

    assert!(proxy.enabled().unwrap());
    assert_eq!(proxy.label("main").unwrap().as_deref(), Some("main-device"));
    assert_eq!(proxy.set_level(7).unwrap(), 49);
    assert_eq!(
        calls.borrow().as_slice(),
        &[
            (0, Vec::new()),
            (1, vec![4, 0, 0, 0, b'm', b'a', b'i', b'n']),
            (2, vec![7, 0, 0, 0]),
        ]
    );
}

#[test]
fn guest_proxy_rejects_over_limit_requests_before_transport() {
    let transport = RecordingTransport::default();
    let calls = Rc::clone(&transport.calls);
    let proxy = DeviceControlGuest::new(transport, CapabilityLimits::new(3, 64));

    let error = proxy.set_level(7).unwrap_err();

    assert_eq!(error, CapabilityError::LimitExceeded);
    assert!(calls.borrow().is_empty());
}

#[test]
fn guest_proxy_rejects_over_limit_and_noncanonical_responses() {
    let proxy = DeviceControlGuest::new(
        FixedTransport(vec![1]),
        CapabilityLimits::new(64, 0),
    );
    assert_eq!(proxy.enabled(), Err(CapabilityError::LimitExceeded));

    let proxy = DeviceControlGuest::new(
        FixedTransport(vec![1, 0]),
        CapabilityLimits::new(64, 64),
    );
    assert_eq!(proxy.enabled(), Err(CapabilityError::InvalidResponse));

    let proxy = DeviceControlGuest::new(
        FixedTransport(vec![2]),
        CapabilityLimits::new(64, 64),
    );
    assert_eq!(proxy.label("main"), Err(CapabilityError::InvalidResponse));
}

#[derive(Default)]
struct RecordingTransport {
    calls: RecordedCalls,
}

type RecordedCalls = Rc<RefCell<Vec<(u32, Vec<u8>)>>>;

impl CapabilityTransport for RecordingTransport {
    fn invoke(&self, call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>> {
        assert_eq!(call.capability_id(), DeviceControlCapability::ID);
        assert_eq!(call.abi_major(), 1);
        assert_eq!(call.response_limit(), 64);
        self.calls
            .borrow_mut()
            .push((call.method_id(), call.request().to_vec()));
        match call.method_id() {
            0 => Ok(vec![1]),
            1 => Ok(vec![1, 11, 0, 0, 0, b'm', b'a', b'i', b'n', b'-', b'd', b'e', b'v', b'i', b'c', b'e']),
            2 => Ok(49_u64.to_le_bytes().to_vec()),
            _ => Err(CapabilityError::InvalidRequest),
        }
    }
}

struct FixedTransport(Vec<u8>);

impl CapabilityTransport for FixedTransport {
    fn invoke(&self, _call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>> {
        Ok(self.0.clone())
    }
}
}

mod capability_wasm_parity {
#![allow(dead_code)]

#[path = "__AIMER_GUEST_MODULE_PATH__"]
mod guest_module;

use std::cell::Cell;
use std::rc::Rc;

use aimer_anteros::{
    CapabilityCall, CapabilityError, CapabilityLimits, CapabilityResult, CapabilityTransport,
    GenerationId, ModelLimits, Runtime, RuntimeConfig, RuntimeErrorKind,
};

#[aimer_macro::capability(
    name = "third-party-widget",
    id = "com.example.third-party-widget",
    abi = 1,
    since = "1.0.0",
)]
trait ThirdPartyWidget {
    fn widget_image(&self) -> CapabilityResult<Vec<u8>>;
}

const MODEL_LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 64);
const CAPABILITY_LIMITS: CapabilityLimits = CapabilityLimits::new(0, 132);
const EXPECTED_AWIR: &[u8] = guest_module::AWIR;

#[test]
fn third_party_provider_matches_native_wire_and_interpreted_outputs() {
    assert_eq!(EXPECTED_AWIR.len(), 128);
    let native_calls = Rc::new(Cell::new(0));
    let native = ThirdPartyProvider::available(Rc::clone(&native_calls));
    assert_eq!(native.widget_image().unwrap(), EXPECTED_AWIR);
    let host = ThirdPartyWidgetHost::new(native, CAPABILITY_LIMITS);
    let mut expected_wire = (EXPECTED_AWIR.len() as u32).to_le_bytes().to_vec();
    expected_wire.extend_from_slice(EXPECTED_AWIR);
    assert_eq!(host.dispatch(0, &[], 132).unwrap(), expected_wire);
    assert_eq!(native_calls.get(), 2);

    let interpreted_calls = Rc::new(Cell::new(0));
    let mut registry = aimer_anteros::CapabilityRegistry::new(1);
    registry
        .register_with_staging(
            ThirdPartyWidgetHost::new(
                ThirdPartyProvider::available(Rc::clone(&interpreted_calls)),
                CAPABILITY_LIMITS,
            ),
            aimer_anteros::CapabilityStagingClass::PureQuery,
        )
        .unwrap();
    let module = guest_module::capability_build_guest_with_contract(
        *ThirdPartyWidgetCapability::ID.as_bytes(),
        ThirdPartyWidgetCapability::CONTRACT_FINGERPRINT,
    );
    let runtime = test_runtime();
    let mut guest = runtime
        .instantiate_with_capabilities(
            &module,
            &registry,
            MODEL_LIMITS,
            GenerationId::new(73),
        )
        .unwrap();

    let output = guest.build(MODEL_LIMITS).unwrap();

    assert_eq!(output.as_bytes(), EXPECTED_AWIR);
    assert_eq!(interpreted_calls.get(), 2);
}

#[test]
fn third_party_provider_errors_and_limits_match_across_paths() {
    let denied = ThirdPartyProvider::denied();
    assert_eq!(denied.widget_image(), Err(CapabilityError::Denied));
    let denied_host = ThirdPartyWidgetHost::new(denied, CAPABILITY_LIMITS);
    assert_eq!(denied_host.dispatch(0, &[], 132), Err(CapabilityError::Denied));

    let mut registry = aimer_anteros::CapabilityRegistry::new(1);
    registry
        .register_with_staging(
            denied_host,
            aimer_anteros::CapabilityStagingClass::PureQuery,
        )
        .unwrap();
    let module = guest_module::capability_build_guest_with_contract(
        *ThirdPartyWidgetCapability::ID.as_bytes(),
        ThirdPartyWidgetCapability::CONTRACT_FINGERPRINT,
    );
    let runtime = test_runtime();
    let mut guest = runtime
        .instantiate_with_capabilities(
            &module,
            &registry,
            MODEL_LIMITS,
            GenerationId::new(74),
        )
        .unwrap();
    let denied = guest.build(MODEL_LIMITS).unwrap_err();
    assert_eq!(denied.kind(), RuntimeErrorKind::GuestStatus);
    assert!(denied.to_string().contains("CapabilityDenied"));

    let bounded_host = ThirdPartyWidgetHost::new(
        ThirdPartyProvider::available(Rc::new(Cell::new(0))),
        CapabilityLimits::new(0, 131),
    );
    assert_eq!(
        bounded_host.dispatch(0, &[], 131),
        Err(CapabilityError::LimitExceeded)
    );
}

fn test_runtime() -> Runtime {
    Runtime::new(
        RuntimeConfig::new()
            .fuel_per_call(10_000)
            .max_module_bytes(64 * 1_024)
            .max_memory_pages(2)
            .max_table_elements(16)
            .max_call_depth(64),
    )
}

#[test]
fn third_party_guest_proxy_rejects_malformed_and_over_limit_responses() {
    let malformed = ThirdPartyWidgetGuest::new(
        FixedTransport(vec![4, 0, 0]),
        CAPABILITY_LIMITS,
    );
    assert_eq!(
        malformed.widget_image(),
        Err(CapabilityError::InvalidResponse)
    );

    let over_limit = ThirdPartyWidgetGuest::new(
        FixedTransport(vec![0; 133]),
        CAPABILITY_LIMITS,
    );
    assert_eq!(
        over_limit.widget_image(),
        Err(CapabilityError::LimitExceeded)
    );
}

#[derive(Clone)]
struct ThirdPartyProvider {
    calls: Rc<Cell<u32>>,
    denied: bool,
}

impl ThirdPartyProvider {
    fn available(calls: Rc<Cell<u32>>) -> Self {
        Self {
            calls,
            denied: false,
        }
    }

    fn denied() -> Self {
        Self {
            calls: Rc::new(Cell::new(0)),
            denied: true,
        }
    }
}

impl ThirdPartyWidget for ThirdPartyProvider {
    fn widget_image(&self) -> CapabilityResult<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        if self.denied {
            Err(CapabilityError::Denied)
        } else {
            Ok(EXPECTED_AWIR.to_vec())
        }
    }
}

struct FixedTransport(Vec<u8>);

impl CapabilityTransport for FixedTransport {
    fn invoke(&self, _call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>> {
        Ok(self.0.clone())
    }
}
}
"####;
}
