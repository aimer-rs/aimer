//! Expansion of `aimer::assets!`: turns the `[assets]` and `[fonts]` tables of
//! an application's `Aimer.toml` into a struct of typed associated consts, each
//! one of the `Bundle*` types chosen from the file's extension.

use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token, Visibility};

/// The `[vis] [struct] [Name]` argument of `assets!`.
struct AssetsInput {
    vis: Visibility,
    name: Ident,
}

impl Parse for AssetsInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let vis: Visibility = input.parse()?;
        let _: Option<Token![struct]> = input.parse()?;
        let name = if input.is_empty() {
            format_ident!("Assets")
        } else {
            input.parse()?
        };
        Ok(Self { vis, name })
    }
}

/// The `Bundle*` type of `aimer_assets` an entry becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bundle {
    Svg,
    Image,
    Font,
    Documents,
    Other,
}

impl Bundle {
    /// Classifies `path` by its extension, ignoring ASCII case. Everything under
    /// `[fonts]` is a font whatever it is called; the extension decides for
    /// `[assets]`.
    fn of(path: &str, in_fonts_table: bool) -> Self {
        if in_fonts_table {
            return Self::Font;
        }
        let extension = path
            .rsplit(['/', '\\'])
            .next()
            .and_then(|file| file.rsplit_once('.'))
            .map(|(_, extension)| extension.to_ascii_lowercase());
        match extension.as_deref() {
            Some("svg") => Self::Svg,
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico") => Self::Image,
            Some("ttf" | "otf" | "ttc" | "woff" | "woff2") => Self::Font,
            Some(
                "md" | "markdown" | "txt" | "json" | "toml" | "yaml" | "yml" | "csv" | "xml"
                | "html" | "htm" | "pdf",
            ) => Self::Documents,
            _ => Self::Other,
        }
    }

    fn type_name(self) -> &'static str {
        match self {
            Self::Svg => "BundleSvg",
            Self::Image => "BundleImage",
            Self::Font => "BundleFont",
            Self::Documents => "BundleDocuments",
            Self::Other => "BundleOther",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Svg => "SVG",
            Self::Image => "image",
            Self::Font => "font",
            Self::Documents => "document",
            Self::Other => "file",
        }
    }
}

/// One `name = "path"` entry, already validated.
struct Entry {
    const_name: Ident,
    name: String,
    path: String,
    bundle: Bundle,
}

/// Expands `declare_assets!` input against the `Aimer.toml` found in
/// `manifest_dir`, declaring the struct as well as its consts.
pub(crate) fn expand_in(manifest_dir: &Path, input: TokenStream) -> syn::Result<TokenStream> {
    let AssetsInput { vis, name } = syn::parse2(input)?;
    let consts = impl_consts(manifest_dir, &vis, &name)?;
    Ok(quote! {
        #[allow(dead_code)]
        #vis struct #name;

        #consts
    })
}

/// Expands `#[assets]` on a unit struct, keeping the struct as written (its
/// attributes, docs and visibility) and adding the consts beside it.
pub(crate) fn expand_attribute_in(
    manifest_dir: &Path,
    attr: TokenStream,
    item: TokenStream,
) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[assets] takes no arguments; the struct's own name and visibility are used",
        ));
    }
    let item: syn::ItemStruct = syn::parse2(item).map_err(|error| {
        syn::Error::new(error.span(), "#[assets] applies to a unit struct: `struct Assets;`")
    })?;
    if !matches!(item.fields, syn::Fields::Unit) || !item.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item,
            "#[assets] applies to a unit struct without generics: `struct Assets;`",
        ));
    }
    let consts = impl_consts(manifest_dir, &item.vis, &item.ident)?;
    Ok(quote! {
        #item

        #consts
    })
}

/// The manifest-tracking const and the `impl` holding one const per entry.
fn impl_consts(manifest_dir: &Path, vis: &Visibility, name: &Ident) -> syn::Result<TokenStream> {
    let manifest_path = manifest_dir.join("Aimer.toml");
    let text = std::fs::read_to_string(&manifest_path).map_err(|error| {
        error_at(format!(
            "assets needs an Aimer.toml in the crate root, but `{}` could not be read: {error}",
            manifest_path.display()
        ))
    })?;
    let manifest: toml::Table = text.parse().map_err(|error| {
        error_at(format!("`{}` is not valid TOML: {error}", manifest_path.display()))
    })?;

    let mut entries = Vec::new();
    collect(&manifest, "assets", false, manifest_dir, &mut entries)?;
    collect(&manifest, "fonts", true, manifest_dir, &mut entries)?;

    let mut seen = BTreeSet::new();
    for entry in &entries {
        if !seen.insert(entry.const_name.to_string()) {
            return Err(error_at(format!(
                "`{}` maps to the const `{}`, which another [assets] or [fonts] entry already uses",
                entry.name, entry.const_name
            )));
        }
    }

    let consts = entries.iter().map(|entry| {
        let Entry {
            const_name,
            name,
            path,
            bundle,
        } = entry;
        let ty = format_ident!("{}", bundle.type_name());
        let ty = quote!(::aimer::#ty);
        let doc = format!(
            "The {} `{name}` registered as `{path}` in `Aimer.toml`.",
            bundle.describe()
        );
        quote! {
            #[doc = #doc]
            #vis const #const_name: #ty = #ty::new(#name, #path);
        }
    });

    // Reading the manifest through `include_bytes!` makes cargo rebuild the
    // crate, and so re-run this macro, whenever `Aimer.toml` changes. The const
    // is unnamed, so the bytes never reach the binary.
    let tracked = manifest_path.to_string_lossy().into_owned();

    Ok(quote! {
        const _: &[u8] = include_bytes!(#tracked);

        #[allow(dead_code)]
        impl #name {
            #(#consts)*
        }
    })
}

/// Appends the validated entries of `[table]`, if the manifest has it.
fn collect(
    manifest: &toml::Table,
    table: &str,
    font: bool,
    manifest_dir: &Path,
    entries: &mut Vec<Entry>,
) -> syn::Result<()> {
    let Some(value) = manifest.get(table) else {
        return Ok(());
    };
    let Some(map) = value.as_table() else {
        return Err(error_at(format!("[{table}] in Aimer.toml must be a table of name = \"path\"")));
    };

    for (name, path) in map {
        let Some(path) = path.as_str() else {
            return Err(error_at(format!(
                "[{table}] entry `{name}` in Aimer.toml must be a string path"
            )));
        };
        if !manifest_dir.join(path).is_file() {
            return Err(error_at(format!(
                "[{table}] entry `{name}` points at `{path}`, which is not a file under `{}`",
                manifest_dir.display()
            )));
        }
        entries.push(Entry {
            const_name: const_ident(name)?,
            name: name.clone(),
            path: path.to_owned(),
            bundle: Bundle::of(path, font),
        });
    }
    Ok(())
}

/// `my-image` becomes `MY_IMAGE`; a leading digit gets an underscore prefix.
fn const_ident(name: &str) -> syn::Result<Ident> {
    let mut converted = String::with_capacity(name.len() + 1);
    for ch in name.chars() {
        converted.push(if ch.is_ascii_alphanumeric() {
            ch.to_ascii_uppercase()
        } else {
            '_'
        });
    }
    if !converted.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return Err(error_at(format!(
            "`{name}` has no letters or digits to build a const name from"
        )));
    }
    if converted.starts_with(|ch: char| ch.is_ascii_digit()) {
        converted.insert(0, '_');
    }
    Ok(Ident::new(&converted, Span::call_site()))
}

fn error_at(message: String) -> syn::Error {
    syn::Error::new(Span::call_site(), message)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use quote::quote;
    use syn::{ImplItem, Item};

    use super::*;

    /// A throwaway manifest directory holding an `Aimer.toml` and empty files.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, manifest: &str, files: &[&str]) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "aimer_macro_assets_{name}_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("Aimer.toml"), manifest).unwrap();
            for file in files {
                let path = dir.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"x").unwrap();
            }
            Self(dir)
        }

        fn expand(&self, input: TokenStream) -> syn::Result<TokenStream> {
            expand_in(&self.0, input)
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parse(tokens: TokenStream) -> syn::File {
        syn::parse2(tokens).expect("expansion should be valid Rust items")
    }

    fn struct_named<'a>(file: &'a syn::File, name: &str) -> &'a syn::ItemStruct {
        file.items
            .iter()
            .find_map(|item| match item {
                Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no struct `{name}` in expansion"))
    }

    fn consts(file: &syn::File) -> Vec<String> {
        file.items
            .iter()
            .filter_map(|item| match item {
                Item::Impl(item) => Some(item),
                _ => None,
            })
            .flat_map(|item| item.items.iter())
            .filter_map(|item| match item {
                ImplItem::Const(item) => Some(item.ident.to_string()),
                _ => None,
            })
            .collect()
    }

    const MANIFEST: &str = r#"
[application]
name = "demo"

[assets]
my_image = "assets/my_image.png"
logo-mark = "assets/logo.svg"

[fonts]
inter = "assets/fonts/Inter.ttf"
"#;
    const FILES: &[&str] = &[
        "assets/my_image.png",
        "assets/logo.svg",
        "assets/fonts/Inter.ttf",
    ];

    #[test]
    fn emits_one_const_per_asset_and_font() {
        let project = Project::new("consts", MANIFEST, FILES);
        let file = parse(project.expand(quote!(pub Assets)).unwrap());

        assert!(matches!(
            struct_named(&file, "Assets").vis,
            syn::Visibility::Public(_)
        ));
        assert_eq!(consts(&file), ["LOGO_MARK", "MY_IMAGE", "INTER"]);
    }

    /// The Rust type a manifest entry became, read from the generated source.
    fn type_of(source: &str, const_name: &str) -> String {
        let source: String = source.split_whitespace().collect();
        let marker = format!("const{const_name}:");
        let start = source.find(&marker).unwrap_or_else(|| panic!("no const {const_name}"))
            + marker.len();
        let end = start + source[start..].find('=').unwrap();
        source[start..end].to_owned()
    }

    #[test]
    fn each_entry_gets_the_bundle_type_of_its_file() {
        let manifest = r#"
[assets]
photo = "a.PNG"
logo = "b.svg"
readme = "c.md"
config = "d.json"
blob = "e.bin"
no_extension = "f"
embedded_font = "g.ttf"

[fonts]
inter = "h.ttf"
"#;
        let files = ["a.PNG", "b.svg", "c.md", "d.json", "e.bin", "f", "g.ttf", "h.ttf"];
        let project = Project::new("types", manifest, &files);
        let source = project.expand(quote!(pub Assets)).unwrap().to_string();

        assert_eq!(type_of(&source, "PHOTO"), "::aimer::BundleImage");
        assert_eq!(type_of(&source, "LOGO"), "::aimer::BundleSvg");
        assert_eq!(type_of(&source, "README"), "::aimer::BundleDocuments");
        assert_eq!(type_of(&source, "CONFIG"), "::aimer::BundleDocuments");
        assert_eq!(type_of(&source, "BLOB"), "::aimer::BundleOther");
        assert_eq!(type_of(&source, "NO_EXTENSION"), "::aimer::BundleOther");
        assert_eq!(type_of(&source, "EMBEDDED_FONT"), "::aimer::BundleFont");
        assert_eq!(type_of(&source, "INTER"), "::aimer::BundleFont");
    }

    #[test]
    fn everything_under_fonts_is_a_font_whatever_its_extension() {
        let manifest = "[fonts]\nodd = \"a.data\"\n";
        let project = Project::new("fontext", manifest, &["a.data"]);
        let source = project.expand(quote!()).unwrap().to_string();

        assert_eq!(type_of(&source, "ODD"), "::aimer::BundleFont");
    }

    #[test]
    fn the_constructor_receives_the_name_and_the_path() {
        let project = Project::new("ctor", MANIFEST, FILES);
        let source = project.expand(quote!(pub Assets)).unwrap().to_string();
        let source: String = source.split_whitespace().collect();

        assert!(source.contains(
            r#"MY_IMAGE:::aimer::BundleImage=::aimer::BundleImage::new("my_image","assets/my_image.png")"#
        ));
        assert!(source.contains(
            r#"INTER:::aimer::BundleFont=::aimer::BundleFont::new("inter","assets/fonts/Inter.ttf")"#
        ));
    }

    #[test]
    fn accepts_an_optional_struct_keyword_and_defaults_the_name() {
        let project = Project::new("input", MANIFEST, FILES);

        let named = parse(project.expand(quote!(pub(crate) struct Bundle)).unwrap());
        assert!(matches!(
            struct_named(&named, "Bundle").vis,
            syn::Visibility::Restricted(_)
        ));

        let default = parse(project.expand(quote!()).unwrap());
        assert!(matches!(
            struct_named(&default, "Assets").vis,
            syn::Visibility::Inherited
        ));
    }

    #[test]
    fn tracks_the_manifest_so_cargo_reruns_the_macro() {
        let project = Project::new("track", MANIFEST, FILES);
        let source = project.expand(quote!()).unwrap().to_string();

        assert!(source.contains("include_bytes"));
        assert!(source.contains("Aimer.toml"));
    }

    #[test]
    fn a_manifest_without_the_tables_expands_to_an_empty_struct() {
        let project = Project::new("empty", "[application]\nname = \"x\"\n", &[]);
        let file = parse(project.expand(quote!()).unwrap());

        assert!(consts(&file).is_empty());
    }

    #[test]
    fn reports_a_missing_file() {
        let project = Project::new("missing", MANIFEST, &["assets/logo.svg"]);
        let error = project.expand(quote!()).unwrap_err().to_string();

        assert!(error.contains("my_image"), "{error}");
        assert!(error.contains("assets/my_image.png"), "{error}");
    }

    #[test]
    fn reports_a_missing_manifest() {
        let project = Project::new("nomanifest", "", &[]);
        fs::remove_file(project.0.join("Aimer.toml")).unwrap();

        let error = project.expand(quote!()).unwrap_err().to_string();
        assert!(error.contains("Aimer.toml"), "{error}");
    }

    #[test]
    fn reports_a_non_string_entry() {
        let project = Project::new("nonstring", "[assets]\nlogo = 3\n", &[]);
        let error = project.expand(quote!()).unwrap_err().to_string();

        assert!(error.contains("logo"), "{error}");
        assert!(error.contains("string"), "{error}");
    }

    #[test]
    fn reports_names_that_collide_after_conversion_to_const_names() {
        let manifest = "[assets]\nmy-image = \"a.png\"\nmy_image = \"b.png\"\n";
        let project = Project::new("collide", manifest, &["a.png", "b.png"]);
        let error = project.expand(quote!()).unwrap_err().to_string();

        assert!(error.contains("MY_IMAGE"), "{error}");
    }

    #[test]
    fn reports_a_name_shared_by_an_asset_and_a_font() {
        let manifest = "[assets]\ninter = \"a.png\"\n[fonts]\ninter = \"b.ttf\"\n";
        let project = Project::new("shared", manifest, &["a.png", "b.ttf"]);
        let error = project.expand(quote!()).unwrap_err().to_string();

        assert!(error.contains("INTER"), "{error}");
    }

    #[test]
    fn names_starting_with_a_digit_get_a_valid_const_name() {
        let manifest = "[assets]\n\"2x_logo\" = \"a.png\"\n";
        let project = Project::new("digit", manifest, &["a.png"]);
        let file = parse(project.expand(quote!()).unwrap());

        assert_eq!(consts(&file), ["_2X_LOGO"]);
    }

    #[test]
    fn rejects_a_name_with_no_usable_characters() {
        let manifest = "[assets]\n\"\" = \"a.png\"\n";
        let project = Project::new("blank", manifest, &["a.png"]);

        assert!(project.expand(quote!()).is_err());
    }

    #[test]
    fn attribute_form_keeps_the_struct_and_adds_the_consts() {
        let project = Project::new("attr", MANIFEST, FILES);
        let file = parse(
            expand_attribute_in(
                &project.0,
                quote!(),
                quote!(
                    #[derive(Debug)]
                    pub struct Bundle;
                ),
            )
            .unwrap(),
        );

        let item = struct_named(&file, "Bundle");
        assert!(matches!(item.vis, syn::Visibility::Public(_)));
        assert_eq!(item.attrs.len(), 1, "the derive must survive");
        assert_eq!(consts(&file), ["LOGO_MARK", "MY_IMAGE", "INTER"]);
    }

    #[test]
    fn attribute_form_rejects_arguments_and_non_unit_structs() {
        let project = Project::new("attr_bad", MANIFEST, FILES);
        let run = |attr: TokenStream, item: TokenStream| {
            expand_attribute_in(&project.0, attr, item)
                .unwrap_err()
                .to_string()
        };

        assert!(run(quote!(pub), quote!(struct A;)).contains("no arguments"));
        assert!(run(quote!(), quote!(struct A {})).contains("unit struct"));
        assert!(run(quote!(), quote!(struct A(u8);)).contains("unit struct"));
        assert!(run(quote!(), quote!(struct A<T>(T);)).contains("unit struct"));
        assert!(run(quote!(), quote!(fn a() {})).contains("unit struct"));
    }

    #[test]
    fn attribute_form_reports_a_missing_file_too() {
        let project = Project::new("attr_missing", MANIFEST, &[]);
        let error = expand_attribute_in(&project.0, quote!(), quote!(struct A;))
            .unwrap_err()
            .to_string();

        assert!(error.contains("logo-mark"), "{error}");
    }
}
