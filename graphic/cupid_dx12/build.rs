//! Compiles the built-in HLSL with FXC at build time so startup does not.
//!
//! Output is a table keyed by the hash of each source file, embedded by
//! `src/precompiled.rs`. Where FXC is unavailable (a non-Windows build host) the
//! table is empty and the runtime compiles as it always did.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

include!("src/shader_hash.rs");

const ENTRIES: [(&str, &str); 2] = [("vs_main", "vs_5_1"), ("fs_main", "ps_5_1")];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let shaders = manifest.join("shaders").join("dx12");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shader_hash.rs");
    println!("cargo:rerun-if-changed={}", shaders.display());

    let mut table = String::from("static PRECOMPILED: &[(u64, &str, &str, &[u8])] = &[\n");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && cfg!(windows) {
        let mut files: Vec<_> = std::fs::read_dir(&shaders)
            .expect("read shaders/dx12")
            .map(|entry| entry.expect("read shader entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "hlsl"))
            .collect();
        files.sort();
        for path in files {
            println!("cargo:rerun-if-changed={}", path.display());
            let source = std::fs::read_to_string(&path).expect("read HLSL");
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            for (entry, target) in ENTRIES {
                let bytecode = compile(&path, &source, entry, target);
                let blob = out_dir.join(format!("{stem}_{entry}.dxbc"));
                std::fs::write(&blob, bytecode).expect("write DXBC");
                writeln!(
                    table,
                    "    ({:#x}, {entry:?}, {target:?}, include_bytes!({:?})),",
                    hash_source(&source),
                    blob.to_str().expect("UTF-8 OUT_DIR")
                )
                .unwrap();
            }
        }
    }
    table.push_str("];\n");
    std::fs::write(out_dir.join("precompiled_shaders.rs"), table).expect("write table");
}

#[cfg(windows)]
fn compile(path: &Path, source: &str, entry: &str, target: &str) -> Vec<u8> {
    use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
    use windows::Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude, D3D_SHADER_MACRO};
    use windows::core::PCSTR;

    let entry = std::ffi::CString::new(entry).unwrap();
    let target = std::ffi::CString::new(target).unwrap();
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // Same call the runtime makes, with FXC's default optimization: the result
    // ships in debug builds too, so there is no startup reason to lower it.
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR(b"cupid-hlsl\0".as_ptr()),
            None::<*const D3D_SHADER_MACRO>,
            None::<&ID3DInclude>,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(target.as_ptr().cast()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    let blob_bytes = |blob: &ID3DBlob| unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
            .to_vec()
    };
    if let Err(error) = result {
        let message = errors
            .as_ref()
            .map(|blob| String::from_utf8_lossy(&blob_bytes(blob)).into_owned())
            .unwrap_or_else(|| error.to_string());
        panic!("{}: {:?} ({}): {message}", path.display(), entry, target.to_string_lossy());
    }
    blob_bytes(code.as_ref().expect("D3DCompile returned no bytecode"))
}

#[cfg(not(windows))]
fn compile(_: &Path, _: &str, _: &str, _: &str) -> Vec<u8> {
    unreachable!("FXC is only available on a Windows build host")
}
