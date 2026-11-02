use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    if std::env::var("RUST_ANALYZER").is_ok() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = workspace_root(&manifest_dir).unwrap_or_else(|| manifest_dir.clone());
    let mut source_files = Vec::new();
    collect_source_files(&workspace_root, &mut source_files);
    source_files.sort();

    let mut generated = String::from("pub(super) static SOURCES: &[(&str, &str)] = &[\n");
    let mut source_directories = BTreeSet::new();
    for source_file in source_files {
        let Ok(relative_path) = source_file.strip_prefix(&workspace_root) else {
            continue;
        };
        let Some(relative_path) = relative_path.to_str() else {
            continue;
        };
        let Some(source_path) = source_file.to_str() else {
            continue;
        };
        let relative_path = relative_path.replace('\\', "/");
        generated.push_str(&format!(
            "    ({relative_path:?}, include_str!({source_path:?})),\n"
        ));
        if let Some(parent) = source_file.parent() {
            source_directories.insert(parent.to_owned());
        }
    }
    generated.push_str("];\n");

    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("embedded_sources.rs");
    fs::write(output, generated).unwrap();

    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("Cargo.toml").display()
    );
    for directory in source_directories {
        println!("cargo:rerun-if-changed={}", directory.display());
    }
}

fn workspace_root(manifest_dir: &Path) -> Option<PathBuf> {
    manifest_dir.ancestors().find_map(|directory| {
        let manifest = fs::read_to_string(directory.join("Cargo.toml")).ok()?;
        manifest
            .contains("[workspace]")
            .then(|| directory.to_owned())
    })
}

fn collect_source_files(directory: &Path, source_files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !is_ignored_directory(&path) {
                collect_source_files(&path, source_files);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            source_files.push(path);
        }
    }
}

fn is_ignored_directory(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        matches!(
            name.to_str(),
            Some(
                ".git"
                    | ".junie"
                    | "node_modules"
                    | "target"
                    | ".github"
                    | ".codegraph"
                    | "Cargo.lock"
            )
        )
    })
}
