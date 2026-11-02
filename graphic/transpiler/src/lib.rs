//! `.aimer` shader language transpiler.
//!
//! Pipeline: lexer -> parser (AST) -> resolver (modules, symbols, const checks, reachability)
//! -> type checker -> emitter (WGSL, MSL, HLSL, GLSL or GLSL ES).
//!
//! # Using it as a library
//!
//! ```
//! use aimer_transpiler::{Target, compile, resolve::NoLoader};
//!
//! let src = "fn twice(x: f32) -> f32 { x * 2.0 }";
//! let out = compile(src, "demo", &NoLoader, Target::Wgsl).unwrap();
//! assert_eq!(out.files[0].ext, "wgsl");
//! assert!(out.files[0].text.contains("fn twice"));
//! ```
//!
//! Imports (`import aimer::color;`) are served by a [`resolve::ModuleLoader`]:
//! [`resolve::FsLoader`] reads a folder, [`resolve::MemoryLoader`] serves sources held in memory,
//! and [`resolve::ExternalModules`] marks modules the runtime already provides.
//!
//! [`compile`] runs the whole pipeline for one [`Target`]. To emit several targets from one source,
//! call [`load_program`] once and then [`emit`] for each target.
//!
//! # Cargo features
//!
//! - `lsp` (off by default): the `lsp` module with editor support (diagnostics, hover, completion,
//!   go to definition). Built for `wasm32-unknown-unknown` it exports a small C ABI that the
//!   VS Code extension loads. Without the feature none of that code is compiled.

pub mod ast;
pub mod emit;
pub mod error;
pub mod layout;
pub mod lexer;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod parser;
pub mod rename;
pub mod resolve;
pub mod sema;

use error::Result;
use resolve::{ModuleLoader, Program};

pub use error::Error;

/// What to transpile to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Wgsl,
    Msl,
    Hlsl,
    /// GLSL 3.30 core.
    Glsl,
    /// GLSL ES 3.00.
    GlslEs,
    /// Only the `layout.json` description (vertex buffers and uniform block sizes).
    Layout,
}

impl Target {
    pub const ALL: [Target; 6] = [Target::Wgsl, Target::Msl, Target::Hlsl, Target::Glsl, Target::GlslEs, Target::Layout];

    /// The name used on the command line: `wgsl`, `msl`, `hlsl`, `glsl`, `glsl-es`, `layout`.
    pub fn name(self) -> &'static str {
        match self {
            Target::Wgsl => "wgsl",
            Target::Msl => "msl",
            Target::Hlsl => "hlsl",
            Target::Glsl => "glsl",
            Target::GlslEs => "glsl-es",
            Target::Layout => "layout",
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Target {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Target::ALL.into_iter().find(|t| t.name() == s).ok_or_else(|| Error::new(format!("unknown target `{s}` (available: wgsl, msl, hlsl, glsl, glsl-es, layout)")))
    }
}

/// One generated file: `ext` is the file extension (`wgsl`, `metal`, `hlsl`, `vert`, `frag`, `layout.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFile {
    pub ext: &'static str,
    pub text: String,
}

/// The result of emitting one target. Single-file targets give one file; GLSL gives
/// `vert` and/or `frag` plus `layout.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub target: Target,
    pub files: Vec<OutputFile>,
}

impl Output {
    /// The text of the file with this extension.
    pub fn get(&self, ext: &str) -> Option<&str> {
        self.files.iter().find(|f| f.ext == ext).map(|f| f.text.as_str())
    }
}

/// Emit an already loaded program for `target`.
pub fn emit(program: &Program, target: Target) -> Result<Output> {
    let one = |ext, text| vec![OutputFile { ext, text }];
    let glsl = |o: emit::clike::GlslOutput| {
        let mut files = Vec::new();
        if let Some(text) = o.vertex {
            files.push(OutputFile { ext: "vert", text });
        }
        if let Some(text) = o.fragment {
            files.push(OutputFile { ext: "frag", text });
        }
        files.push(OutputFile { ext: "layout.json", text: o.layout });
        files
    };
    let files = match target {
        Target::Wgsl => one("wgsl", emit::wgsl::emit(program)?),
        Target::Msl => one("metal", emit::msl::emit(program)?),
        Target::Hlsl => one("hlsl", emit::hlsl::emit(program)?),
        Target::Glsl => glsl(emit::glsl::emit(program)?),
        Target::GlslEs => glsl(emit::glsl::emit_es(program)?),
        Target::Layout => one("layout.json", layout::json(program)?),
    };
    Ok(Output { target, files })
}

/// Parse, resolve, check and emit one `.aimer` source for `target`.
pub fn compile(src: &str, default_name: &str, loader: &dyn ModuleLoader, target: Target) -> Result<Output> {
    let program = load_program(src, default_name, loader)?;
    emit(&program, target)
}

/// Parse and resolve one `.aimer` source (and the modules it imports).
pub fn load_program(src: &str, default_name: &str, loader: &dyn ModuleLoader) -> Result<Program> {
    let file = parser::parse(src)?;
    let program = resolve::build_program(file, default_name, loader)?;
    layout::check(&program)?;
    Ok(program)
}

/// Transpile one `.aimer` source to WGSL.
pub fn compile_wgsl(src: &str, default_name: &str, loader: &dyn ModuleLoader) -> Result<String> {
    let program = load_program(src, default_name, loader)?;
    emit::wgsl::emit(&program)
}

/// Transpile one `.aimer` source to Metal Shading Language.
pub fn compile_msl(src: &str, default_name: &str, loader: &dyn ModuleLoader) -> Result<String> {
    let program = load_program(src, default_name, loader)?;
    emit::msl::emit(&program)
}

/// Transpile one `.aimer` source to HLSL.
pub fn compile_hlsl(src: &str, default_name: &str, loader: &dyn ModuleLoader) -> Result<String> {
    let program = load_program(src, default_name, loader)?;
    emit::hlsl::emit(&program)
}

/// Transpile one `.aimer` source to GLSL 3.30 core (vertex and fragment shader plus layout).
pub fn compile_glsl(src: &str, default_name: &str, loader: &dyn ModuleLoader) -> Result<emit::clike::GlslOutput> {
    let program = load_program(src, default_name, loader)?;
    emit::glsl::emit(&program)
}

/// Transpile one `.aimer` source to GLSL ES 3.00.
pub fn compile_glsl_es(src: &str, default_name: &str, loader: &dyn ModuleLoader) -> Result<emit::clike::GlslOutput> {
    let program = load_program(src, default_name, loader)?;
    emit::glsl::emit_es(&program)
}
