use std::collections::HashSet;
use std::fs;
use std::path::Path;

use aimer_anteros::{StableId128, capability_contract_fingerprint};
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Expr, FnArg, GenericArgument, ItemTrait, Lit, MetaNameValue, Pat, PathArguments, ReturnType,
    Token, TraitItem, TraitItemFn, Type, TypePath,
};

const SOURCE_MAP_ENV: &str = "AIMER_CAPABILITY_PACKAGE_SOURCE_MAP";
const CRATES_IO_GIT_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CRATES_IO_SPARSE_SOURCE: &str = "sparse+https://index.crates.io/";

pub(crate) fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = syn::parse2::<CapabilityArgs>(args)?;
    let item = syn::parse2::<ItemTrait>(item)?;
    validate_trait(&item)?;

    let canonical_id = resolve_canonical_id(&args)?;
    let methods = canonical_methods(&item)?;
    let contract = encode_contract(&canonical_id, args.abi, &methods);
    let capability_id = StableId128::derive_capability(&canonical_id);
    let fingerprint = capability_contract_fingerprint(&contract);
    let capability_id = capability_id.as_bytes();
    let abi = args.abi;
    let since = args.since;
    let visibility = &item.vis;
    let metadata_name = format_ident!("{}Capability", item.ident);
    let anteros = anteros_path()?;
    let guest_proxy = generate_guest_proxy(&item, &methods, &metadata_name, &anteros)?;
    let host_adapter = generate_host_adapter(&item, &methods, &metadata_name, &anteros)?;

    Ok(quote! {
        #item

        #[doc = "Canonical metadata generated for this Aimer capability contract."]
        #visibility struct #metadata_name;

        impl #metadata_name {
            #[doc = "The complete stable package-scoped capability identity."]
            pub const CANONICAL_ID: &'static str = #canonical_id;
            #[doc = "The deterministic 128-bit manifest identity."]
            pub const ID: #anteros::StableId128 =
                #anteros::StableId128::from_bytes([#(#capability_id),*]);
            #[doc = "The incompatible wire-contract major version."]
            pub const ABI_MAJOR: u32 = #abi;
            #[doc = "The SDK release that first exposed this contract."]
            pub const SINCE: &'static str = #since;
            #[doc = "The deterministic hash of the canonical wire contract."]
            pub const CONTRACT_FINGERPRINT: [u8; 32] = [#(#fingerprint),*];

            #[doc = "Creates a canonical manifest requirement for this contract."]
            #[inline]
            pub const fn requirement(
                policy: #anteros::CapabilityPolicy,
            ) -> #anteros::CapabilityRequirement {
                #anteros::CapabilityRequirement::new(
                    Self::ID,
                    Self::ABI_MAJOR,
                    policy,
                    Self::CONTRACT_FINGERPRINT,
                )
            }
        }

        #guest_proxy
        #host_adapter
    })
}

struct CapabilityArgs {
    name: String,
    id: Option<String>,
    abi: u32,
    since: String,
    name_span: Span,
}

impl Parse for CapabilityArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let entries = Punctuated::<MetaNameValue, Token![,]>::parse_terminated(input)?;
        let mut name = None;
        let mut id = None;
        let mut abi = None;
        let mut since = None;

        for entry in entries {
            let Some(key) = entry.path.get_ident() else {
                return Err(syn::Error::new_spanned(
                    entry.path,
                    "capability metadata keys must be identifiers",
                ));
            };
            match key.to_string().as_str() {
                "name" => set_once(&mut name, string_value(&entry)?, key)?,
                "id" => set_once(&mut id, string_value(&entry)?, key)?,
                "abi" => set_once(&mut abi, integer_value(&entry)?, key)?,
                "since" => set_once(&mut since, string_value(&entry)?, key)?,
                _ => {
                    return Err(syn::Error::new_spanned(
                        key,
                        "unsupported capability metadata; expected `name`, `id`, `abi`, or `since`",
                    ));
                }
            }
        }

        let (name, name_span) = name
            .map(|value| (value, Span::call_site()))
            .ok_or_else(|| {
                syn::Error::new(
                    Span::call_site(),
                    "missing required `name` capability metadata",
                )
            })?;
        let abi = abi.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "missing required `abi` capability metadata",
            )
        })?;
        let since = since.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "missing required `since` capability metadata",
            )
        })?;

        validate_component("name", &name, name_span)?;
        if let Some(id) = &id {
            validate_component("id", id, Span::call_site())?;
        }
        if abi == 0 {
            return Err(syn::Error::new(
                Span::call_site(),
                "capability `abi` must be greater than zero",
            ));
        }
        validate_since(&since)?;

        Ok(Self {
            name,
            id,
            abi,
            since,
            name_span,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, key: &Ident) -> syn::Result<()> {
    if slot.replace(value).is_some() {
        return Err(syn::Error::new_spanned(
            key,
            format!("duplicate `{key}` capability metadata"),
        ));
    }
    Ok(())
}

fn string_value(entry: &MetaNameValue) -> syn::Result<String> {
    match &entry.value {
        Expr::Lit(literal) => match &literal.lit {
            Lit::Str(value) => Ok(value.value()),
            _ => Err(syn::Error::new_spanned(
                &entry.value,
                "capability metadata value must be a string literal",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            &entry.value,
            "capability metadata value must be a string literal",
        )),
    }
}

fn integer_value(entry: &MetaNameValue) -> syn::Result<u32> {
    match &entry.value {
        Expr::Lit(literal) => match &literal.lit {
            Lit::Int(value) => value.base10_parse(),
            _ => Err(syn::Error::new_spanned(
                &entry.value,
                "capability `abi` must be an integer literal",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            &entry.value,
            "capability `abi` must be an integer literal",
        )),
    }
}

fn validate_component(kind: &str, value: &str, span: Span) -> syn::Result<()> {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return Err(syn::Error::new(
            span,
            format!("capability `{kind}` must be a non-empty identity without whitespace"),
        ));
    }
    Ok(())
}

fn validate_since(since: &str) -> syn::Result<()> {
    let mut parts = since.split('.');
    let valid = (0..3).all(|_| {
        parts.next().is_some_and(|part| {
            !part.is_empty() && part.chars().all(|value| value.is_ascii_digit())
        })
    }) && parts.next().is_none();
    if !valid {
        return Err(syn::Error::new(
            Span::call_site(),
            "capability `since` must use `major.minor.patch` numeric syntax",
        ));
    }
    Ok(())
}

fn resolve_canonical_id(args: &CapabilityArgs) -> syn::Result<String> {
    if let Some(id) = &args.id {
        return Ok(id.clone());
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        syn::Error::new(
            args.name_span,
            "cannot resolve the capability package namespace; supply an explicit `id`",
        )
    })?;
    let package_name = std::env::var("CARGO_PKG_NAME").map_err(|_| {
        syn::Error::new(
            args.name_span,
            "cannot resolve the Cargo package name; supply an explicit capability `id`",
        )
    })?;
    if let Some(crate_id) = manifest_crate_id(Path::new(&manifest_dir))? {
        return Ok(format!("{crate_id}::{}", args.name));
    }
    let source = package_source(Path::new(&manifest_dir), &package_name)?;
    if matches!(
        source.as_deref(),
        Some(CRATES_IO_GIT_SOURCE | CRATES_IO_SPARSE_SOURCE)
    ) {
        return Ok(format!("crates.io::{package_name}::{}", args.name));
    }

    Err(syn::Error::new(
        args.name_span,
        "workspace, path, Git, and alternate-registry capability crates require `[package.metadata.aimer] crate-id` or an explicit `id`",
    ))
}

fn manifest_crate_id(manifest_dir: &Path) -> syn::Result<Option<String>> {
    let manifest_path = manifest_dir.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("failed to read `{}`: {error}", manifest_path.display()),
        )
    })?;
    let manifest = toml::from_str::<toml::Table>(&source).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("failed to parse `{}`: {error}", manifest_path.display()),
        )
    })?;
    let crate_id = manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("aimer"))
        .and_then(|aimer| aimer.get("crate-id"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    if let Some(crate_id) = &crate_id {
        validate_component("crate-id", crate_id, Span::call_site())?;
    }
    Ok(crate_id)
}

fn package_source(manifest_dir: &Path, package_name: &str) -> syn::Result<Option<String>> {
    let source_map_path = std::env::var(SOURCE_MAP_ENV).map_err(|_| {
        syn::Error::new(
            Span::call_site(),
            "Aimer capability package source map is unavailable; build through Aimer or declare `[package.metadata.aimer] crate-id` or an explicit capability `id`",
        )
    })?;
    let source = fs::read_to_string(&source_map_path).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!(
                "failed to read Aimer capability package source map `{source_map_path}`: {error}"
            ),
        )
    })?;
    let map = toml::from_str::<toml::Table>(&source).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!(
                "failed to parse Aimer capability package source map `{source_map_path}`: {error}"
            ),
        )
    })?;
    if map.get("version").and_then(toml::Value::as_integer) != Some(1) {
        return Err(syn::Error::new(
            Span::call_site(),
            "unsupported Aimer capability package source-map version",
        ));
    }
    let packages = map
        .get("packages")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "Aimer capability package source map has no package table",
            )
        })?;
    let manifest_path = fs::canonicalize(manifest_dir.join("Cargo.toml")).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("failed to resolve the capability package manifest: {error}"),
        )
    })?;
    let mut matching_source = None;
    for package in packages {
        let Some(package) = package.as_table() else {
            return Err(syn::Error::new(
                Span::call_site(),
                "Aimer capability package source map contains a non-table package",
            ));
        };
        let Some(candidate_path) = package.get("manifest_path").and_then(toml::Value::as_str)
        else {
            return Err(syn::Error::new(
                Span::call_site(),
                "Aimer capability package source entry has no manifest path",
            ));
        };
        let Ok(candidate_path) = fs::canonicalize(candidate_path) else {
            continue;
        };
        if candidate_path != manifest_path {
            continue;
        }
        let candidate_name = package.get("name").and_then(toml::Value::as_str);
        if candidate_name != Some(package_name) {
            return Err(syn::Error::new(
                Span::call_site(),
                "Aimer capability package source entry does not match the compiling Cargo package",
            ));
        }
        if matching_source.is_some() {
            return Err(syn::Error::new(
                Span::call_site(),
                "Aimer capability package source map contains an ambiguous manifest entry",
            ));
        }
        matching_source = Some(
            package
                .get("source")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
        );
    }
    matching_source.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "Aimer capability package source map does not contain the compiling package",
        )
    })
}

fn validate_trait(item: &ItemTrait) -> syn::Result<()> {
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "capability traits cannot declare generics",
        ));
    }
    if !item.supertraits.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.supertraits,
            "capability traits cannot declare supertraits",
        ));
    }
    Ok(())
}

struct CanonicalMethod {
    name: String,
    receiver: u8,
    parameters: Vec<String>,
    result: String,
}

fn canonical_methods(item: &ItemTrait) -> syn::Result<Vec<CanonicalMethod>> {
    let mut names = HashSet::new();
    let mut methods = Vec::new();
    for trait_item in &item.items {
        let TraitItem::Fn(method) = trait_item else {
            return Err(syn::Error::new_spanned(
                trait_item,
                "capability traits may contain methods only",
            ));
        };
        let name = method.sig.ident.to_string();
        if !names.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                &method.sig.ident,
                format!("duplicate capability method `{name}`"),
            ));
        }
        methods.push(canonical_method(method)?);
    }
    methods.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(methods)
}

fn canonical_method(method: &TraitItemFn) -> syn::Result<CanonicalMethod> {
    if method.sig.constness.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "capability methods cannot be `const`",
        ));
    }
    if method.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "async capability methods require a declared asynchronous handle schema",
        ));
    }
    if method.sig.unsafety.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "capability methods cannot be `unsafe`",
        ));
    }
    if method.sig.abi.is_some() || method.sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "capability methods cannot declare an external ABI or variadic arguments",
        ));
    }
    if !method.sig.generics.params.is_empty() || method.sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig.generics,
            "capability methods cannot declare generics",
        ));
    }

    let mut receiver = 0;
    let mut parameters = Vec::new();
    for input in &method.sig.inputs {
        match input {
            FnArg::Receiver(value) => {
                if receiver != 0
                    || value.reference.is_none()
                    || value.mutability.is_some()
                    || value.colon_token.is_some()
                {
                    return Err(syn::Error::new_spanned(
                        value,
                        "capability receivers must be `&self`",
                    ));
                }
                receiver = 1;
            }
            FnArg::Typed(value) => {
                if !matches!(value.pat.as_ref(), Pat::Ident(_)) {
                    return Err(syn::Error::new_spanned(
                        &value.pat,
                        "capability parameters require simple identifier patterns",
                    ));
                }
                parameters.push(canonical_wire_type(&value.ty, WirePosition::Parameter)?);
            }
        }
    }

    if receiver == 0 {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "capability methods require an `&self` provider receiver",
        ));
    }

    let result = match &method.sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &method.sig.output,
                "capability methods must return `CapabilityResult<T>`",
            ));
        }
        ReturnType::Type(_, value) => {
            canonical_wire_type(capability_result_value(value)?, WirePosition::Return)?
        }
    };
    Ok(CanonicalMethod {
        name: method.sig.ident.to_string(),
        receiver,
        parameters,
        result,
    })
}

#[derive(Clone, Copy)]
enum WirePosition {
    Parameter,
    Return,
}

fn canonical_wire_type(value: &Type, position: WirePosition) -> syn::Result<String> {
    match value {
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok("unit".to_owned()),
        Type::Ptr(_) => Err(syn::Error::new_spanned(
            value,
            "native pointers cannot cross a capability boundary",
        )),
        Type::Reference(reference) => {
            if matches!(position, WirePosition::Return) {
                return Err(syn::Error::new_spanned(
                    value,
                    "capability return values cannot borrow data",
                ));
            }
            if reference.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    value,
                    "capability input references must be immutable",
                ));
            }
            match reference.elem.as_ref() {
                Type::Path(path) if simple_path_name(path).as_deref() == Some("str") => {
                    Ok("string".to_owned())
                }
                Type::Slice(slice)
                    if simple_path_name_from_type(&slice.elem).as_deref() == Some("u8") =>
                {
                    Ok("bytes".to_owned())
                }
                _ => Err(unsupported_wire_type(value)),
            }
        }
        Type::Path(path) => canonical_path_type(path, position),
        _ => Err(unsupported_wire_type(value)),
    }
}

fn canonical_path_type(path: &TypePath, position: WirePosition) -> syn::Result<String> {
    let Some(segment) = path.path.segments.last() else {
        return Err(unsupported_wire_type(&Type::Path(path.clone())));
    };
    let name = segment.ident.to_string();
    if path.qself.is_none() && path.path.segments.len() == 1 {
        match name.as_str() {
            "bool" | "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64" | "f32"
            | "f64" => return Ok(name),
            "String" => return Ok("string".to_owned()),
            "Vec" => {
                let element = one_type_argument(&segment.arguments)?;
                let element = canonical_wire_type(element, WirePosition::Return)?;
                if element != "u8" {
                    return Err(syn::Error::new_spanned(
                        path,
                        "only `Vec<u8>` is supported by the initial capability wire contract",
                    ));
                }
                return Ok("bytes".to_owned());
            }
            "Option" => {
                if matches!(position, WirePosition::Parameter) {
                    return Err(syn::Error::new_spanned(
                        path,
                        "optional capability parameters are not supported by the initial wire contract",
                    ));
                }
                let value = one_type_argument(&segment.arguments)?;
                return Ok(format!(
                    "option<{}>",
                    canonical_wire_type(value, WirePosition::Return)?
                ));
            }
            _ => {}
        }
    }
    Err(unsupported_wire_type(&Type::Path(path.clone())))
}

fn capability_result_value(value: &Type) -> syn::Result<&Type> {
    let Type::Path(path) = value else {
        if matches!(value, Type::Reference(_)) {
            return Err(syn::Error::new_spanned(
                value,
                "capability return values cannot borrow data",
            ));
        }
        return Err(syn::Error::new_spanned(
            value,
            "capability methods must return `CapabilityResult<T>`",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            value,
            "capability methods must return `CapabilityResult<T>`",
        ));
    };
    if segment.ident != "CapabilityResult" {
        return Err(syn::Error::new_spanned(
            value,
            "capability methods must return `CapabilityResult<T>`",
        ));
    }
    one_type_argument(&segment.arguments)
}

fn one_type_argument(arguments: &PathArguments) -> syn::Result<&Type> {
    Ok(type_arguments(arguments, 1)?[0])
}

fn type_arguments(arguments: &PathArguments, expected: usize) -> syn::Result<Vec<&Type>> {
    let PathArguments::AngleBracketed(arguments) = arguments else {
        return Err(syn::Error::new_spanned(
            arguments,
            "capability collection and result wire types require explicit type arguments",
        ));
    };
    let values: Vec<_> = arguments
        .args
        .iter()
        .filter_map(|value| match value {
            GenericArgument::Type(value) => Some(value),
            _ => None,
        })
        .collect();
    if values.len() != expected || arguments.args.len() != expected {
        return Err(syn::Error::new_spanned(
            arguments,
            format!("capability wire type requires exactly {expected} type argument(s)"),
        ));
    }
    Ok(values)
}

fn simple_path_name(path: &TypePath) -> Option<String> {
    (path.qself.is_none() && path.path.segments.len() == 1)
        .then(|| path.path.segments[0].ident.to_string())
}

fn simple_path_name_from_type(value: &Type) -> Option<String> {
    match value {
        Type::Path(path) => simple_path_name(path),
        _ => None,
    }
}

fn unsupported_wire_type(value: &Type) -> syn::Error {
    let display = value.to_token_stream().to_string().replace(' ', "");
    syn::Error::new_spanned(
        value,
        format!("unsupported capability wire type `{display}`"),
    )
}

fn encode_contract(canonical_id: &str, abi: u32, methods: &[CanonicalMethod]) -> Vec<u8> {
    let mut output = Vec::new();
    write_string(&mut output, canonical_id);
    output.extend_from_slice(&abi.to_le_bytes());
    output.extend_from_slice(&(methods.len() as u32).to_le_bytes());
    for method in methods {
        write_string(&mut output, &method.name);
        output.push(method.receiver);
        output.extend_from_slice(&(method.parameters.len() as u32).to_le_bytes());
        for parameter in &method.parameters {
            write_string(&mut output, parameter);
        }
        write_string(&mut output, &method.result);
    }
    output
}

fn write_string(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn generate_guest_proxy(
    item: &ItemTrait,
    methods: &[CanonicalMethod],
    metadata_name: &Ident,
    anteros: &TokenStream,
) -> syn::Result<TokenStream> {
    let visibility = &item.vis;
    let trait_name = &item.ident;
    let guest_name = format_ident!("{}Guest", trait_name);
    let implementations = item
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method),
            _ => None,
        })
        .map(|method| {
            let name = method.sig.ident.to_string();
            let method_id = methods
                .iter()
                .position(|candidate| candidate.name == name)
                .ok_or_else(|| syn::Error::new_spanned(method, "missing canonical method"))?
                as u32;
            let signature = &method.sig;
            let encoders = method
                .sig
                .inputs
                .iter()
                .filter_map(|input| match input {
                    FnArg::Typed(value) => Some(value),
                    FnArg::Receiver(_) => None,
                })
                .map(|input| {
                    let Pat::Ident(pattern) = input.pat.as_ref() else {
                        return Err(syn::Error::new_spanned(
                            &input.pat,
                            "capability parameters require simple identifier patterns",
                        ));
                    };
                    encode_wire_value(&input.ty, &pattern.ident, anteros)
                })
                .collect::<syn::Result<Vec<_>>>()?;
            let result = match &method.sig.output {
                ReturnType::Type(_, value) => {
                    decode_wire_value(capability_result_value(value)?, anteros)?
                }
                ReturnType::Default => unreachable!("validated capability result"),
            };

            Ok(quote! {
                #signature {
                    let mut request = #anteros::CapabilityEncoder::new(
                        self.limits.max_request_bytes(),
                    );
                    #(#encoders)*
                    let request = request.into_bytes();
                    let response = self.transport.invoke(#anteros::CapabilityCall::new(
                        #metadata_name::ID,
                        #metadata_name::ABI_MAJOR,
                        #method_id,
                        &request,
                        self.limits.max_response_bytes(),
                    ))?;
                    let mut response = #anteros::CapabilityDecoder::new(
                        &response,
                        self.limits.max_response_bytes(),
                    )?;
                    let value = #result;
                    response.finish()?;
                    Ok(value)
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        #[doc = "A bounded guest-side proxy for this capability contract."]
        #visibility struct #guest_name<T> {
            transport: T,
            limits: #anteros::CapabilityLimits,
        }

        impl<T> #guest_name<T> {
            #[doc = "Creates a proxy over one target-specific capability transport."]
            #[inline]
            pub const fn new(transport: T, limits: #anteros::CapabilityLimits) -> Self {
                Self { transport, limits }
            }

            #[doc = "Returns the underlying target-specific transport."]
            #[inline]
            pub fn into_inner(self) -> T {
                self.transport
            }
        }

        #[cfg(target_arch = "wasm32")]
        impl #guest_name<#anteros::WasmCapabilityTransport> {
            #[doc = "Creates a proxy over the canonical native-interpreter import."]
            #[inline]
            pub const fn wasm(limits: #anteros::CapabilityLimits) -> Self {
                Self::new(#anteros::WasmCapabilityTransport::new(), limits)
            }
        }

        impl<T: #anteros::CapabilityTransport> #trait_name for #guest_name<T> {
            #(#implementations)*
        }
    })
}

fn generate_host_adapter(
    item: &ItemTrait,
    methods: &[CanonicalMethod],
    metadata_name: &Ident,
    anteros: &TokenStream,
) -> syn::Result<TokenStream> {
    let visibility = &item.vis;
    let trait_name = &item.ident;
    let host_name = format_ident!("{}Host", trait_name);
    let dispatch_arms = item
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method),
            _ => None,
        })
        .map(|method| {
            let name = method.sig.ident.to_string();
            let method_id = methods
                .iter()
                .position(|candidate| candidate.name == name)
                .ok_or_else(|| syn::Error::new_spanned(method, "missing canonical method"))?
                as u32;
            let method_name = &method.sig.ident;
            let parameters = method
                .sig
                .inputs
                .iter()
                .filter_map(|input| match input {
                    FnArg::Typed(value) => Some(value),
                    FnArg::Receiver(_) => None,
                })
                .map(|input| {
                    let Pat::Ident(pattern) = input.pat.as_ref() else {
                        return Err(syn::Error::new_spanned(
                            &input.pat,
                            "capability parameters require simple identifier patterns",
                        ));
                    };
                    decode_provider_parameter(&input.ty, &pattern.ident)
                })
                .collect::<syn::Result<Vec<_>>>()?;
            let decoders = parameters.iter().map(|(decoder, _)| decoder);
            let arguments = parameters.iter().map(|(_, argument)| argument);
            let result = match &method.sig.output {
                ReturnType::Type(_, value) => encode_provider_result(
                    capability_result_value(value)?,
                    &format_ident!("value"),
                    anteros,
                )?,
                ReturnType::Default => unreachable!("validated capability result"),
            };

            Ok(quote! {
                #method_id => {
                    let mut request = #anteros::CapabilityDecoder::new_request(
                        request,
                        self.limits.max_request_bytes(),
                    )?;
                    #(#decoders)*
                    request.finish()?;
                    let value = #trait_name::#method_name(&self.provider, #(#arguments),*)?;
                    let mut response = #anteros::CapabilityEncoder::new(
                        response_limit.min(self.limits.max_response_bytes()),
                    );
                    #result
                    Ok(response.into_bytes())
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        #[doc = "A bounded native host adapter for this capability contract."]
        #visibility struct #host_name<P> {
            provider: P,
            limits: #anteros::CapabilityLimits,
        }

        impl<P> #host_name<P> {
            #[doc = "Creates a type-erased host adapter around one native provider."]
            #[inline]
            pub const fn new(provider: P, limits: #anteros::CapabilityLimits) -> Self {
                Self { provider, limits }
            }

            #[doc = "Returns the underlying native provider."]
            #[inline]
            pub fn into_inner(self) -> P {
                self.provider
            }
        }

        impl<P: #trait_name> #host_name<P> {
            #[doc = "Dispatches one bounded canonical request to the native provider."]
            pub fn dispatch(
                &self,
                method_id: u32,
                request: &[u8],
                response_limit: u32,
            ) -> #anteros::CapabilityResult<Vec<u8>> {
                match method_id {
                    #(#dispatch_arms,)*
                    _ => Err(#anteros::CapabilityError::InvalidRequest),
                }
            }
        }

        impl<P: #trait_name> #anteros::CapabilityProvider for #host_name<P> {
            #[inline]
            fn descriptor(&self) -> #anteros::CapabilityDescriptor {
                #anteros::CapabilityDescriptor::new(
                    #metadata_name::ID,
                    #metadata_name::ABI_MAJOR,
                    #metadata_name::CONTRACT_FINGERPRINT,
                    self.limits,
                )
            }

            fn invoke(
                &self,
                _generation: #anteros::CapabilityGeneration,
                method_id: u32,
                request: &[u8],
                response_limit: u32,
            ) -> #anteros::CapabilityResult<Vec<u8>> {
                self.dispatch(method_id, request, response_limit)
            }
        }
    })
}

fn decode_provider_parameter(
    value: &Type,
    name: &Ident,
) -> syn::Result<(TokenStream, TokenStream)> {
    match value {
        Type::Reference(reference) => match reference.elem.as_ref() {
            Type::Path(path) if simple_path_name(path).as_deref() == Some("str") => {
                Ok((quote!(let #name = request.read_string()?;), quote!(&#name)))
            }
            Type::Slice(slice)
                if simple_path_name_from_type(&slice.elem).as_deref() == Some("u8") =>
            {
                Ok((quote!(let #name = request.read_bytes()?;), quote!(&#name)))
            }
            _ => Err(unsupported_wire_type(value)),
        },
        Type::Path(path) => {
            let segment = path.path.segments.last().unwrap();
            let reader = match segment.ident.to_string().as_str() {
                "bool" => quote!(read_bool),
                "i8" => quote!(read_i8),
                "u8" => quote!(read_u8),
                "i16" => quote!(read_i16),
                "u16" => quote!(read_u16),
                "i32" => quote!(read_i32),
                "u32" => quote!(read_u32),
                "i64" => quote!(read_i64),
                "u64" => quote!(read_u64),
                "f32" => quote!(read_f32),
                "f64" => quote!(read_f64),
                "String" => quote!(read_string),
                "Vec" => quote!(read_bytes),
                _ => return Err(unsupported_wire_type(value)),
            };
            Ok((quote!(let #name = request.#reader()?;), quote!(#name)))
        }
        _ => Err(unsupported_wire_type(value)),
    }
}

fn encode_provider_result(
    value: &Type,
    name: &Ident,
    anteros: &TokenStream,
) -> syn::Result<TokenStream> {
    match value {
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(TokenStream::new()),
        Type::Path(path) => {
            let segment = path.path.segments.last().unwrap();
            match segment.ident.to_string().as_str() {
                "bool" => Ok(quote!(response.write_bool(#name)?;)),
                "i8" => Ok(quote!(response.write_i8(#name)?;)),
                "u8" => Ok(quote!(response.write_u8(#name)?;)),
                "i16" => Ok(quote!(response.write_i16(#name)?;)),
                "u16" => Ok(quote!(response.write_u16(#name)?;)),
                "i32" => Ok(quote!(response.write_i32(#name)?;)),
                "u32" => Ok(quote!(response.write_u32(#name)?;)),
                "i64" => Ok(quote!(response.write_i64(#name)?;)),
                "u64" => Ok(quote!(response.write_u64(#name)?;)),
                "f32" => Ok(quote!(response.write_f32(#name)?;)),
                "f64" => Ok(quote!(response.write_f64(#name)?;)),
                "String" => Ok(quote!(response.write_string(&#name)?;)),
                "Vec" => Ok(quote!(response.write_bytes(&#name)?;)),
                "Option" => {
                    let inner = one_type_argument(&segment.arguments)?;
                    let inner_name = format_ident!("inner_value");
                    let inner = encode_provider_result(inner, &inner_name, anteros)?;
                    Ok(quote! {
                        match #name {
                            None => response.write_u8(0)?,
                            Some(#inner_name) => {
                                response.write_u8(1)?;
                                #inner
                            }
                        }
                    })
                }
                _ => Err(unsupported_wire_type(value)),
            }
        }
        _ => {
            let _ = anteros;
            Err(unsupported_wire_type(value))
        }
    }
}

fn encode_wire_value(
    value: &Type,
    name: &Ident,
    anteros: &TokenStream,
) -> syn::Result<TokenStream> {
    match value {
        Type::Reference(reference) => match reference.elem.as_ref() {
            Type::Path(path) if simple_path_name(path).as_deref() == Some("str") => {
                Ok(quote!(request.write_string(#name)?;))
            }
            Type::Slice(slice)
                if simple_path_name_from_type(&slice.elem).as_deref() == Some("u8") =>
            {
                Ok(quote!(request.write_bytes(#name)?;))
            }
            _ => Err(unsupported_wire_type(value)),
        },
        Type::Path(path) => {
            let segment = path.path.segments.last().unwrap();
            let type_name = segment.ident.to_string();
            match type_name.as_str() {
                "bool" => Ok(quote!(request.write_bool(#name)?;)),
                "i8" => Ok(quote!(request.write_i8(#name)?;)),
                "u8" => Ok(quote!(request.write_u8(#name)?;)),
                "i16" => Ok(quote!(request.write_i16(#name)?;)),
                "u16" => Ok(quote!(request.write_u16(#name)?;)),
                "i32" => Ok(quote!(request.write_i32(#name)?;)),
                "u32" => Ok(quote!(request.write_u32(#name)?;)),
                "i64" => Ok(quote!(request.write_i64(#name)?;)),
                "u64" => Ok(quote!(request.write_u64(#name)?;)),
                "f32" => Ok(quote!(request.write_f32(#name)?;)),
                "f64" => Ok(quote!(request.write_f64(#name)?;)),
                "String" => Ok(quote!(request.write_string(&#name)?;)),
                "Vec" => Ok(quote!(request.write_bytes(&#name)?;)),
                _ => Err(unsupported_wire_type(value)),
            }
        }
        _ => {
            let _ = anteros;
            Err(unsupported_wire_type(value))
        }
    }
}

fn decode_wire_value(value: &Type, anteros: &TokenStream) -> syn::Result<TokenStream> {
    match value {
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(quote!(())),
        Type::Path(path) => {
            let segment = path.path.segments.last().unwrap();
            let type_name = segment.ident.to_string();
            match type_name.as_str() {
                "bool" => Ok(quote!(response.read_bool()?)),
                "i8" => Ok(quote!(response.read_i8()?)),
                "u8" => Ok(quote!(response.read_u8()?)),
                "i16" => Ok(quote!(response.read_i16()?)),
                "u16" => Ok(quote!(response.read_u16()?)),
                "i32" => Ok(quote!(response.read_i32()?)),
                "u32" => Ok(quote!(response.read_u32()?)),
                "i64" => Ok(quote!(response.read_i64()?)),
                "u64" => Ok(quote!(response.read_u64()?)),
                "f32" => Ok(quote!(response.read_f32()?)),
                "f64" => Ok(quote!(response.read_f64()?)),
                "String" => Ok(quote!(response.read_string()?)),
                "Vec" => Ok(quote!(response.read_bytes()?)),
                "Option" => {
                    let inner = one_type_argument(&segment.arguments)?;
                    let inner = decode_wire_value(inner, anteros)?;
                    Ok(quote! {
                        match response.read_u8()? {
                            0 => None,
                            1 => Some(#inner),
                            _ => return Err(#anteros::CapabilityError::InvalidResponse),
                        }
                    })
                }
                _ => Err(unsupported_wire_type(value)),
            }
        }
        _ => Err(unsupported_wire_type(value)),
    }
}

fn anteros_path() -> syn::Result<TokenStream> {
    if let Ok(found) = crate_name("aimer") {
        return Ok(match found {
            FoundCrate::Itself => quote!(::aimer::anteros),
            FoundCrate::Name(name) => {
                let name = Ident::new(&name, Span::call_site());
                quote!(::#name::anteros)
            }
        });
    }
    match crate_name("aimer_anteros") {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let name = Ident::new(&name, Span::call_site());
            Ok(quote!(::#name))
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!("capability declarations require `aimer` or `aimer_anteros`: {error}"),
        )),
    }
}
