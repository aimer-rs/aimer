//! Module loading, symbol tables, const validation and reachability.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::*;
use crate::error::{Error, Result, bail};
use crate::parser::parse;
use crate::rename::Scope;
use crate::sema::{ConstInfo, FnSig, Globals, Ty, is_constructor};

/// Supplies the source of an imported module, e.g. `aimer::color`.
pub trait ModuleLoader {
    fn load(&self, module: &[String]) -> Result<String>;

    /// Is this module supplied by the runtime? Its items are still checked, but not emitted.
    fn is_external(&self, _module: &str) -> bool {
        false
    }
}

/// Wraps a loader and marks some modules (`aimer::color`) as supplied by the runtime.
pub struct ExternalModules<'a> {
    pub inner: &'a dyn ModuleLoader,
    pub modules: Vec<String>,
}

impl ModuleLoader for ExternalModules<'_> {
    fn load(&self, module: &[String]) -> Result<String> {
        self.inner.load(module)
    }

    fn is_external(&self, module: &str) -> bool {
        self.modules.iter().any(|m| m == module) || self.inner.is_external(module)
    }
}

/// `aimer::color` is read from `<root>/color.aimer`, `aimer::fx::blur` from `<root>/fx/blur.aimer`.
/// The leading `aimer` segment names the standard library and is not part of the path.
pub struct FsLoader {
    pub root: PathBuf,
}

impl ModuleLoader for FsLoader {
    fn load(&self, module: &[String]) -> Result<String> {
        let segs = if module.first().map(String::as_str) == Some("aimer") { &module[1..] } else { module };
        if segs.is_empty() {
            bail!("cannot import the bare module `{}`", module.join("::"));
        }
        let mut path = self.root.clone();
        for s in segs {
            path.push(s);
        }
        path.set_extension("aimer");
        std::fs::read_to_string(&path).map_err(|e| Error::new(format!("cannot read module `{}` ({}): {e}", module.join("::"), path.display())))
    }
}

/// Serves module sources held in memory. Keys are module paths such as `aimer::color` or `a::b`.
#[derive(Debug, Default, Clone)]
pub struct MemoryLoader {
    modules: HashMap<String, String>,
}

impl MemoryLoader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a module, e.g. `insert("aimer::color", src)`.
    pub fn insert(&mut self, module: impl Into<String>, src: impl Into<String>) -> &mut Self {
        self.modules.insert(module.into(), src.into());
        self
    }

    pub fn with(mut self, module: impl Into<String>, src: impl Into<String>) -> Self {
        self.insert(module, src);
        self
    }
}

impl ModuleLoader for MemoryLoader {
    fn load(&self, module: &[String]) -> Result<String> {
        let key = module.join("::");
        self.modules.get(&key).cloned().ok_or_else(|| Error::new(format!("cannot read module `{key}`: not provided")))
    }
}

/// A loader with no modules, for sources that do not import anything.
pub struct NoLoader;

impl ModuleLoader for NoLoader {
    fn load(&self, module: &[String]) -> Result<String> {
        bail!("cannot import `{}`: no module search path", module.join("::"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}

#[derive(Debug)]
pub struct Program {
    pub name: String,
    /// Imports removed. Modules first, then the shader's own items, reachable functions only.
    pub items: Vec<Item>,
    pub globals: Globals,
    /// Final names of items that belong to `--extern` modules. Checked, but not emitted.
    pub external: HashSet<String>,
}

impl Program {
    pub fn entry(&self, stage: Stage) -> Option<&Item> {
        let want = match stage {
            Stage::Vertex => "vertex",
            Stage::Fragment => "fragment",
        };
        self.items.iter().find(|i| i.attr(want).is_some() && matches!(i.kind, ItemKind::Fn { .. }))
    }
}

pub fn item_name(item: &Item) -> Option<&str> {
    match &item.kind {
        ItemKind::Const { name, .. } | ItemKind::Struct { name, .. } | ItemKind::Resource { name, .. } | ItemKind::Fn { name, .. } => Some(name),
        ItemKind::Import { .. } => None,
    }
}

/// Key of the shader's own file in the scope tables. Module keys are `a::b`, so this cannot clash.
const MAIN: &str = "";

fn is_entry(item: &Item) -> bool {
    item.attr("vertex").is_some() || item.attr("fragment").is_some()
}

fn named_items(items: &[Item]) -> impl Iterator<Item = (&str, &Item)> {
    items.iter().filter_map(|i| item_name(i).map(|n| (n, i)))
}

/// Split an import into its module key and the wanted item (`None` for a glob).
fn import_target(path: &[String], glob: bool) -> Result<(String, Option<String>)> {
    if glob {
        Ok((path.join("::"), None))
    } else if path.len() < 2 {
        bail!("`import {};` must name an item inside a module, or end in `::*`", path.join("::"))
    } else {
        Ok((path[..path.len() - 1].join("::"), Some(path[path.len() - 1].clone())))
    }
}

/// Choose the final name of every item. Names that are unique stay as written, and the shader's own
/// items always keep theirs. A module item whose name clashes gets a `<module>_` prefix.
fn final_names(files: &[(String, &[Item])]) -> HashMap<(String, String), String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for (_, items) in files {
        for (n, _) in named_items(items) {
            *count.entry(n).or_default() += 1;
        }
    }
    let mut out = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut clashing = Vec::new();
    for (key, items) in files {
        for (n, it) in named_items(items) {
            if key == MAIN || count[n] == 1 || is_entry(it) {
                out.insert((key.clone(), n.to_string()), n.to_string());
                used.insert(n.to_string());
            } else {
                clashing.push((key.clone(), n.to_string()));
            }
        }
    }
    for (key, n) in clashing {
        let base = format!("{}_{n}", key.rsplit("::").next().unwrap_or(&key));
        let mut cand = base.clone();
        let mut i = 2;
        while used.contains(&cand) {
            cand = format!("{base}{i}");
            i += 1;
        }
        used.insert(cand.clone());
        out.insert((key, n), cand);
    }
    out
}

/// What each name written in `key` means: own items, then explicit imports, then glob imports.
fn build_scope(
    key: &str,
    items: &[Item],
    modules: &HashMap<String, Vec<Item>>,
    finals: &HashMap<(String, String), String>,
) -> Result<Scope> {
    let fin = |owner: &str, n: &str| finals[&(owner.to_string(), n.to_string())].clone();
    let mut scope = Scope::default();
    let mut seen = HashSet::new();
    for (n, _) in named_items(items) {
        if !seen.insert(n) {
            bail!("`{n}` is defined more than once");
        }
        scope.names.insert(n.to_string(), fin(key, n));
    }
    let own: HashSet<String> = scope.names.keys().cloned().collect();
    let mut explicit: HashMap<String, String> = HashMap::new();
    for it in items {
        let ItemKind::Import { path, glob: false } = &it.kind else { continue };
        let (module, wanted) = import_target(path, false)?;
        let w = wanted.expect("explicit import");
        let item = modules[&module].iter().find(|i| item_name(i) == Some(&w)).ok_or_else(|| Error::new(format!("module `{module}` has no item `{w}`")))?;
        if !item.public {
            bail!("`{w}` is private in module `{module}`; mark it `pub` to import it");
        }
        if own.contains(&w) {
            bail!("`import {};` clashes with `{w}` defined in this file", path.join("::"));
        }
        let target = fin(&module, &w);
        if explicit.get(&w).is_some_and(|t| *t != target) {
            bail!("`{w}` is imported from two different modules");
        }
        explicit.insert(w.clone(), target.clone());
        scope.names.insert(w, target);
    }
    let mut glob_src: HashMap<String, (String, String)> = HashMap::new();
    for it in items {
        let ItemKind::Import { path, glob: true } = &it.kind else { continue };
        let (module, _) = import_target(path, true)?;
        for (n, item) in named_items(&modules[&module]) {
            if !item.public || own.contains(n) || explicit.contains_key(n) {
                continue;
            }
            let target = fin(&module, n);
            match glob_src.get(n) {
                Some((m, t)) if *t != target => {
                    scope.ambiguous.insert(n.to_string(), vec![format!("`{m}`"), format!("`{module}`")]);
                }
                Some(_) => {}
                None => {
                    glob_src.insert(n.to_string(), (module.clone(), target.clone()));
                    scope.names.insert(n.to_string(), target);
                }
            }
        }
    }
    // Every other item name (as written or as renamed) is hidden from this file.
    for ((_, n), f) in finals {
        for name in [n, f] {
            if !scope.names.contains_key(name) && !scope.ambiguous.contains_key(name) {
                scope.hidden.insert(name.clone());
            }
        }
    }
    Ok(scope)
}

fn rename_file(key: &str, items: &[Item], scope: &Scope, finals: &HashMap<(String, String), String>) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    for it in items.iter().filter(|i| !matches!(i.kind, ItemKind::Import { .. })) {
        let mut it = it.clone();
        let old = item_name(&it).expect("named item").to_string();
        let new = finals[&(key.to_string(), old)].clone();
        crate::rename::rename_item(&mut it, scope, &new).map_err(|e| if key == MAIN { e } else { e.context(format!("in module `{key}`")) })?;
        out.push(it);
    }
    Ok(out)
}

pub fn build_program(main: File, default_name: &str, loader: &dyn ModuleLoader) -> Result<Program> {
    let name = main.shader.clone().unwrap_or_else(|| default_name.to_string());
    let mut modules: HashMap<String, Vec<Item>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    load_imports(&main.items, loader, &mut modules, &mut order, &mut stack)?;

    let mut files: Vec<(String, &[Item])> = order.iter().map(|k| (k.clone(), modules[k].as_slice())).collect();
    files.push((MAIN.to_string(), main.items.as_slice()));
    let finals = final_names(&files);

    let mut items: Vec<Item> = Vec::new();
    for (key, file_items) in &files {
        let scope = build_scope(key, file_items, &modules, &finals).map_err(|e| if key == MAIN { e } else { e.context(format!("in module `{key}`")) })?;
        let renamed = rename_file(key, file_items, &scope, &finals)?;
        items.extend(renamed);
    }
    let main_count = main.items.iter().filter(|i| !matches!(i.kind, ItemKind::Import { .. })).count();
    let module_item_count = items.len() - main_count;

    // `pub import m::*;` re-exports the public functions of `m`.
    let mut extra_roots: Vec<String> = Vec::new();
    for it in main.items.iter().filter(|i| i.public) {
        let ItemKind::Import { path, glob } = &it.kind else { continue };
        let (key, wanted) = import_target(path, *glob)?;
        let Some(module_items) = modules.get(&key) else { continue };
        for (n, item) in named_items(module_items) {
            if item.public && matches!(item.kind, ItemKind::Fn { .. }) && wanted.as_deref().is_none_or(|w| w == n) {
                extra_roots.push(finals[&(key.clone(), n.to_string())].clone());
            }
        }
    }

    let mut external = HashSet::new();
    for key in order.iter().filter(|k| loader.is_external(k)) {
        for (n, _) in named_items(&modules[key]) {
            external.insert(finals[&(key.clone(), n.to_string())].clone());
        }
    }

    let globals = build_globals(&items)?;
    validate(&items, &globals)?;
    let items = prune(items, module_item_count, &extra_roots);
    Ok(Program { name, items, globals, external })
}

fn load_imports(
    items: &[Item],
    loader: &dyn ModuleLoader,
    modules: &mut HashMap<String, Vec<Item>>,
    order: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> Result<()> {
    for it in items {
        let ItemKind::Import { path, glob } = &it.kind else { continue };
        let (module, wanted) = if *glob {
            (path.clone(), None)
        } else {
            if path.len() < 2 {
                bail!("`import {};` must name an item inside a module, or end in `::*`", path.join("::"));
            }
            (path[..path.len() - 1].to_vec(), Some(path[path.len() - 1].clone()))
        };
        let key = module.join("::");
        if stack.contains(&key) {
            bail!("import cycle through module `{key}`");
        }
        if !modules.contains_key(&key) {
            let src = loader.load(&module)?;
            let file = parse(&src).map_err(|e| e.context(format!("in module `{key}`")))?;
            match &file.module {
                Some(m) if *m == module => {}
                Some(m) => bail!("module file for `{key}` declares `module {};`", m.join("::")),
                None => bail!("module file for `{key}` has no `module {key};` line"),
            }
            stack.push(key.clone());
            load_imports(&file.items, loader, modules, order, stack)?;
            stack.pop();
            modules.insert(key.clone(), file.items);
            order.push(key.clone());
        }
        if let Some(w) = wanted {
            if !modules[&key].iter().any(|i| item_name(i) == Some(&w)) {
                bail!("module `{key}` has no item `{w}`");
            }
        }
    }
    Ok(())
}

fn build_globals(items: &[Item]) -> Result<Globals> {
    let mut g = Globals::default();
    for it in items {
        match &it.kind {
            ItemKind::Struct { name, fields, .. } => {
                g.structs.insert(name.clone(), fields.clone());
            }
            ItemKind::Const { name, value, .. } => {
                // Type is filled in below, once every struct and const value is known.
                g.consts.insert(name.clone(), ConstInfo { ty: Ty::Unit, value: value.clone() });
            }
            _ => {}
        }
    }
    check_const_cycles(&g)?;
    for it in items {
        match &it.kind {
            ItemKind::Struct { name, fields, .. } => {
                for f in fields {
                    g.resolve_type(&f.ty).map_err(|e| e.context(format!("field `{}.{}`", name, f.name)))?;
                }
            }
            ItemKind::Const { name, ty, .. } => {
                let t = g.resolve_type(ty).map_err(|e| e.context(format!("const `{name}`")))?;
                g.consts.get_mut(name).unwrap().ty = t;
            }
            ItemKind::Resource { kind, name, ty } => {
                let t = match (kind, ty) {
                    (ResourceKind::Sampler, None) => Ty::Sampler,
                    (_, Some(t)) => g.resolve_type(t).map_err(|e| e.context(format!("resource `{name}`")))?,
                    (_, None) => bail!("resource `{name}` needs a type"),
                };
                match (kind, &t) {
                    (ResourceKind::Texture, Ty::Texture) | (ResourceKind::Sampler, Ty::Sampler) | (ResourceKind::Uniform, Ty::Struct(_)) => {}
                    _ => bail!("resource `{name}` has type {}, which does not match its kind", t.describe()),
                }
                g.resources.insert(name.clone(), t);
            }
            ItemKind::Fn { name, params, ret, .. } => {
                let ctx = |e: Error| e.context(format!("fn `{name}`"));
                let ps = params.iter().map(|p| g.resolve_type(&p.ty)).collect::<Result<Vec<_>>>().map_err(ctx)?;
                let ret = match ret {
                    Some(r) => g.resolve_type(r).map_err(ctx)?,
                    None => Ty::Unit,
                };
                g.fns.insert(name.clone(), FnSig { params: ps, ret });
            }
            ItemKind::Import { .. } => {}
        }
    }
    Ok(g)
}

fn const_refs(e: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    walk_expr(e, &mut |x| {
        if let Expr::Path(n) = x {
            out.push(n.clone());
        }
    });
    out
}

fn check_const_cycles(g: &Globals) -> Result<()> {
    fn visit(g: &Globals, n: &str, path: &mut Vec<String>, done: &mut HashSet<String>) -> Result<()> {
        if done.contains(n) {
            return Ok(());
        }
        if path.iter().any(|p| p == n) {
            bail!("const `{n}` depends on itself");
        }
        let Some(c) = g.consts.get(n) else { return Ok(()) };
        path.push(n.to_string());
        for r in const_refs(&c.value) {
            visit(g, &r, path, done)?;
        }
        path.pop();
        done.insert(n.to_string());
        Ok(())
    }
    let mut done = HashSet::new();
    let mut names: Vec<_> = g.consts.keys().cloned().collect();
    names.sort();
    for n in names {
        visit(g, &n, &mut Vec::new(), &mut done)?;
    }
    Ok(())
}

/// A `const` initialiser is a compile-time expression, so every use can be inlined.
fn check_const_expr(g: &Globals, e: &Expr) -> Result<()> {
    let mut err: Option<String> = None;
    walk_expr(e, &mut |x| {
        if err.is_some() {
            return;
        }
        let bad = match x {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unary(..) | Expr::Binary(..) | Expr::Cast(..) | Expr::ArrayLit(_) | Expr::Index(..) => None,
            Expr::Field(..) => None,
            Expr::Path(n) if g.consts.contains_key(n) => None,
            Expr::Path(n) => Some(format!("`{n}` is not a const")),
            Expr::Call { name, .. } if is_constructor(name) => None,
            Expr::Call { name, .. } => Some(format!("call to `{name}`")),
            _ => Some("this expression kind".to_string()),
        };
        if let Some(b) = bad {
            err = Some(b);
        }
    });
    match err {
        Some(b) => bail!("a const initialiser must be a compile-time expression, but it contains {b}"),
        None => Ok(()),
    }
}

fn validate(items: &[Item], g: &Globals) -> Result<()> {
    for it in items {
        match &it.kind {
            ItemKind::Const { name, ty, value } => {
                let ctx = |e: Error| e.context(format!("const `{name}`"));
                check_const_expr(g, value).map_err(ctx)?;
                let want = g.resolve_type(ty).map_err(ctx)?;
                let got = g.type_of(&mut crate::sema::Env::new(), value).map_err(ctx)?;
                if want != got {
                    bail!("const `{name}`: declared {}, initialiser is {}", want.describe(), got.describe());
                }
            }
            ItemKind::Resource { name, .. } => {
                if it.attr("bind").is_none() {
                    bail!("resource `{name}` needs a `#[bind(group, binding)]` attribute");
                }
                bind_args(it).map_err(|e| e.context(format!("resource `{name}`")))?;
            }
            _ => {}
        }
    }
    let mut vertex = 0;
    let mut fragment = 0;
    for it in items {
        if let ItemKind::Fn { name, .. } = &it.kind {
            if it.attr("vertex").is_some() {
                vertex += 1;
            }
            if it.attr("fragment").is_some() {
                fragment += 1;
            }
            if it.attr("vertex").is_some() && it.attr("fragment").is_some() {
                bail!("fn `{name}` cannot be both `#[vertex]` and `#[fragment]`");
            }
        }
    }
    if vertex > 1 || fragment > 1 {
        bail!("a shader has at most one `#[vertex]` and one `#[fragment]` entry point");
    }
    Ok(())
}

/// `#[bind(group, binding)]` as numbers.
pub fn bind_args(item: &Item) -> Result<(u32, u32)> {
    use crate::lexer::Tok;
    let a = item.attr("bind").ok_or_else(|| Error::new("missing `#[bind(group, binding)]`"))?;
    match a.args.as_slice() {
        [Tok::Int(g), Tok::Punct(","), Tok::Int(b)] => {
            let p = |s: &String| crate::sema::parse_int_literal(s).and_then(|v| u32::try_from(v).ok());
            match (p(g), p(b)) {
                (Some(g), Some(b)) => Ok((g, b)),
                _ => bail!("`#[bind]` arguments must be non-negative integers"),
            }
        }
        _ => bail!("expected `#[bind(group, binding)]`"),
    }
}

/// Keep functions reachable from the entry points. Without entry points (a library-like shader),
/// every function of the shader's own file is a root. A `pub import` re-exports the public functions of
/// the module it names, so those are roots too (`extra_roots`, final names).
fn prune(items: Vec<Item>, module_item_count: usize, extra_roots: &[String]) -> Vec<Item> {
    let fns: HashMap<&str, &Item> = items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Fn { name, .. } => Some((name.as_str(), i)),
            _ => None,
        })
        .collect();
    let mut roots: Vec<&str> = items
        .iter()
        .filter(|i| i.attr("vertex").is_some() || i.attr("fragment").is_some())
        .filter_map(item_name)
        .collect();
    if roots.is_empty() {
        roots = items
            .iter()
            .skip(module_item_count)
            .filter(|i| matches!(i.kind, ItemKind::Fn { .. }))
            .filter_map(item_name)
            .collect();
    }
    roots.extend(extra_roots.iter().map(String::as_str));
    let mut live: HashSet<String> = HashSet::new();
    let mut work: Vec<String> = roots.iter().map(|s| s.to_string()).collect();
    while let Some(n) = work.pop() {
        if !live.insert(n.clone()) {
            continue;
        }
        if let Some(Item { kind: ItemKind::Fn { body, .. }, .. }) = fns.get(n.as_str()).copied() {
            walk_block(body, &mut |e| {
                if let Expr::Call { name, .. } = e {
                    if fns.contains_key(name.as_str()) {
                        work.push(name.clone());
                    }
                }
            });
        }
    }
    items
        .into_iter()
        .filter(|i| match &i.kind {
            ItemKind::Fn { name, .. } => live.contains(name),
            _ => true,
        })
        .collect()
}
