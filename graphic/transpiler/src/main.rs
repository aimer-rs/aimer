use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aimer_transpiler::Target;
use aimer_transpiler::resolve::{ExternalModules, FsLoader};

const USAGE: &str = "\
aimer_transpiler: .aimer shader transpiler

USAGE:
    aimer_transpiler build <input.aimer | dir> [-o <out dir>] [--target wgsl,msl,hlsl,glsl,glsl-es,layout] [--lib <dir>] [--extern <module>]...
    aimer_transpiler check <input.aimer | dir> [--lib <dir>] [--extern <module>]...

    build  Transpile. Without -o the result is printed to stdout (one input and one target only).
           With -o, files are written to <out dir>/<target>/<shader>.<ext>
           (wgsl -> .wgsl, msl -> .metal, hlsl -> .hlsl). glsl and glsl-es write
           <shader>.vert, <shader>.frag and <shader>.layout.json (bindings the runtime must set).
    check  Parse, resolve and type check, without writing anything.

    --lib  Module search root for `import aimer::...` (default: <input dir>/lib).
    --extern  A module the runtime already provides (`aimer::color`). Its items are checked
           but not written to the output, so they are not defined twice. Repeatable.
    A directory input compiles every *.aimer file in it, except the `lib/` folder.
";

struct Args {
    cmd: String,
    input: PathBuf,
    out: Option<PathBuf>,
    targets: Vec<Target>,
    lib: Option<PathBuf>,
    externs: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or("missing command")?;
    if cmd == "-h" || cmd == "--help" || cmd == "help" {
        return Err(String::new());
    }
    if cmd != "build" && cmd != "check" {
        return Err(format!("unknown command `{cmd}`"));
    }
    let mut input = None;
    let mut out = None;
    let mut targets = Vec::new();
    let mut lib = None;
    let mut externs = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = Some(PathBuf::from(it.next().ok_or("`-o` needs a directory")?)),
            "--target" => {
                for t in it.next().ok_or("`--target` needs a list")?.split(',') {
                    let t = t.trim();
                    targets.push(t.parse::<Target>().map_err(|_| format!("target `{t}` is not implemented (available: wgsl, msl, hlsl, glsl, glsl-es, layout)"))?);
                }
            }
            "--lib" => lib = Some(PathBuf::from(it.next().ok_or("`--lib` needs a directory")?)),
            "--extern" => externs.push(it.next().ok_or("`--extern` needs a module name")?),
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("unknown option `{s}`")),
            s => {
                if input.replace(PathBuf::from(s)).is_some() {
                    return Err("only one input is allowed".into());
                }
            }
        }
    }
    if targets.is_empty() {
        targets.push(Target::Wgsl);
    }
    Ok(Args { cmd, input: input.ok_or("missing input")?, out, targets, lib, externs })
}

fn collect_inputs(input: &Path) -> Result<Vec<PathBuf>, String> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(input)
        .map_err(|e| format!("cannot read {}: {e}", input.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "aimer"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no .aimer files in {}", input.display()));
    }
    Ok(files)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let inputs = collect_inputs(&args.input)?;
    let base_dir = if args.input.is_dir() { args.input.clone() } else { args.input.parent().map(Path::to_path_buf).unwrap_or_default() };
    let lib_root = args.lib.clone().unwrap_or_else(|| base_dir.join("lib"));
    let fs_loader = FsLoader { root: lib_root };
    let loader = ExternalModules { inner: &fs_loader, modules: args.externs.clone() };

    if args.cmd == "build" && args.out.is_none() && (inputs.len() > 1 || args.targets.len() > 1) {
        return Err("several inputs or targets need `-o <out dir>`".into());
    }

    let mut failed = false;
    for path in &inputs {
        let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("shader");
        let program = match aimer_transpiler::load_program(&src, stem, &loader) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        if args.cmd == "check" {
            // Emitting is what type checks function bodies. Every target is tried, so target limits show up too.
            for t in &args.targets {
                if let Err(e) = aimer_transpiler::emit(&program, *t) {
                    eprintln!("error: {} ({t}): {e}", path.display());
                    failed = true;
                }
            }
            continue;
        }
        for t in &args.targets {
            let files = match aimer_transpiler::emit(&program, *t) {
                Ok(o) => o.files,
                Err(e) => {
                    eprintln!("error: {}: {e}", path.display());
                    failed = true;
                    continue;
                }
            };
            match &args.out {
                None => {
                    for f in files {
                        if matches!(t, Target::Glsl | Target::GlslEs) {
                            println!("// ==== {}.{} ====", program.name, f.ext);
                        }
                        print!("{}", f.text);
                    }
                }
                Some(out) => {
                    let dir = out.join(t.name());
                    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
                    for f in files {
                        let file = dir.join(format!("{}.{}", program.name, f.ext));
                        std::fs::write(&file, f.text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
                        eprintln!("wrote {}", file.display());
                    }
                }
            }
        }
    }
    if failed { Err("build failed".into()) } else { Ok(()) }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(m) if m.is_empty() => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("error: {m}");
            if m.starts_with("missing") || m.starts_with("unknown") {
                eprintln!("\n{USAGE}");
            }
            ExitCode::FAILURE
        }
    }
}
