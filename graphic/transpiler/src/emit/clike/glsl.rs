//! GLSL 3.30 core and GLSL ES 3.00 output.
//!
//! GLSL has one stage per shader object, so a program becomes a `.vert` and a `.frag` text. Each
//! stage only gets the functions, uniforms and tables its entry point reaches. What GLSL needs that
//! the other C-like targets do not:
//! - an entry `main()` that reads the stage inputs, calls the user's entry function and writes the outputs;
//! - varyings matched by name (`_vs2fs_locationN`), because 3.30 and ES 3.00 only allow `layout(location)`
//!   on vertex inputs and fragment outputs;
//! - texture and sampler pairs fused into one `sampler2D` uniform;
//! - no binding qualifiers: block bindings and sampler units are set by the runtime, from the layout JSON.

use super::*;
use crate::sema::Scalar;

/// The two shader texts of a program and the layout the runtime needs to bind them.
pub struct GlslOutput {
    pub vertex: Option<String>,
    pub fragment: Option<String>,
    /// `<name>.layout.json`: uniform blocks, fused samplers and vertex attributes, per stage.
    pub layout: String,
}

/// A resource and where the bind group layout puts it: (name, group, binding).
type Slot = (String, u32, u32);

/// A vertex attribute: (location, variable name, type, per instance).
type Attribute = (u32, String, String, bool);

/// What the runtime has to know about one stage.
#[derive(Default)]
struct StageLayout {
    /// block name, resource name, group, binding
    blocks: Vec<(String, String, u32, u32)>,
    /// uniform name, texture, sampler
    samplers: Vec<(String, Slot, Option<Slot>)>,
    attributes: Vec<Attribute>,
}

pub fn emit_glsl(p: &Program, es: bool) -> Result<GlslOutput> {
    let mut out = GlslOutput { vertex: None, fragment: None, layout: String::new() };
    let mut layouts = Vec::new();
    for stage in [Stage::Vertex, Stage::Fragment] {
        let mut em = em_new(p, Dialect::Glsl);
        em.es = es;
        let label = if stage == Stage::Vertex { "vertex" } else { "fragment" };
        let r = em.run_glsl(stage).map_err(|e| e.context(format!("{label} stage")))?;
        if let Some((text, layout)) = r {
            layouts.push((label, layout));
            if stage == Stage::Vertex {
                out.vertex = Some(text);
            } else {
                out.fragment = Some(text);
            }
        }
    }
    out.layout = layout_json(&p.name, &layouts);
    Ok(out)
}

fn layout_json(shader: &str, stages: &[(&str, StageLayout)]) -> String {
    let mut s = format!("{{\n  \"shader\": \"{shader}\"");
    for (label, l) in stages {
        s.push_str(&format!(",\n  \"{label}\": {{\n    \"uniform_blocks\": ["));
        let items: Vec<String> = l
            .blocks
            .iter()
            .map(|(b, r, g, n)| format!("{{ \"block\": \"{b}\", \"resource\": \"{r}\", \"group\": {g}, \"binding\": {n} }}"))
            .collect();
        s.push_str(&join_lines(&items));
        s.push_str("],\n    \"samplers\": [");
        let items: Vec<String> = l
            .samplers
            .iter()
            .map(|(u, (t, tg, tb), smp)| {
                let smp = match smp {
                    Some((n, g, b)) => format!("{{ \"name\": \"{n}\", \"group\": {g}, \"binding\": {b} }}"),
                    None => "null".to_string(),
                };
                format!("{{ \"uniform\": \"{u}\", \"texture\": {{ \"name\": \"{t}\", \"group\": {tg}, \"binding\": {tb} }}, \"sampler\": {smp} }}")
            })
            .collect();
        s.push_str(&join_lines(&items));
        s.push(']');
        if *label == "vertex" {
            s.push_str(",\n    \"attributes\": [");
            let items: Vec<String> = l
                .attributes
                .iter()
                .map(|(loc, n, t, inst)| {
                    let step = if *inst { "instance" } else { "vertex" };
                    format!("{{ \"location\": {loc}, \"name\": \"{n}\", \"type\": \"{t}\", \"step\": \"{step}\" }}")
                })
                .collect();
            s.push_str(&join_lines(&items));
            s.push(']');
        }
        s.push_str("\n  }");
    }
    s.push_str("\n}\n");
    s
}

fn join_lines(items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("\n      {}\n    ", items.join(",\n      "))
    }
}

impl<'a> Emitter<'a> {
    pub(super) fn glsl_uniform_name(&self, n: &str) -> String {
        let suffix = if self.stage == Some(Stage::Fragment) { "fs" } else { "vs" };
        format!("{}_{suffix}", self.id(n))
    }

    fn stage_label(&self) -> &'static str {
        if self.stage == Some(Stage::Fragment) { "fragment" } else { "vertex" }
    }

    /// The `sampler2D` uniform for a texture, with or without a sampler. Pairs are recorded in first-use order.
    fn glsl_sampler_name(&mut self, tex: &str, smp: Option<&str>) -> String {
        let pair = (tex.to_string(), smp.map(str::to_string));
        if !self.glsl_pairs.contains(&pair) {
            self.glsl_pairs.push(pair);
        }
        match smp {
            Some(s) => format!("_smp_{}_{}", self.id(tex), self.id(s)),
            None => format!("_smp_{}", self.id(tex)),
        }
    }

    /// The global a texture or sampler expression stands for: a global itself, or a parameter of a
    /// specialised function copy.
    fn glsl_resource_of(&self, e: &Expr) -> Option<String> {
        let Expr::Path(n) = e else { return None };
        if self.env.lookup(n).is_some() {
            return self.glsl_alias.get(n).cloned();
        }
        self.g.resources.contains_key(n).then(|| n.clone())
    }

    /// Does the function take a texture or a sampler? GLSL writes one copy per binding instead.
    fn glsl_takes_resources(&self, it: &Item) -> bool {
        let ItemKind::Fn { params, .. } = &it.kind else { return false };
        params.iter().any(|p| matches!(self.g.resolve_type(&p.ty), Ok(Ty::Texture | Ty::Sampler)))
    }

    /// A call to a function with texture or sampler parameters: request the specialised copy for these
    /// arguments and call it without them. `None` when the callee takes none.
    pub(super) fn glsl_call_spec(&mut self, name: &str, args: &[Expr]) -> Result<Option<String>> {
        let p = self.p;
        let Some(Item { kind: ItemKind::Fn { params, .. }, .. }) =
            p.items.iter().find(|i| matches!(&i.kind, ItemKind::Fn { name: n, .. } if n == name))
        else {
            return Ok(None);
        };
        if !params.iter().any(|q| matches!(self.g.resolve_type(&q.ty), Ok(Ty::Texture | Ty::Sampler))) {
            return Ok(None);
        }
        let mut binds = Vec::new();
        let mut rest: Vec<&Expr> = Vec::new();
        for (prm, arg) in params.iter().zip(args) {
            match self.g.resolve_type(&prm.ty)? {
                Ty::Texture | Ty::Sampler => match self.glsl_resource_of(arg) {
                    Some(r) => binds.push((prm.name.clone(), r)),
                    None => bail!("GLSL: argument `{}` of `{name}` must be a texture or sampler global", prm.name),
                },
                _ => rest.push(arg),
            }
        }
        let mut emitted = self.id(name);
        for (_, r) in &binds {
            emitted.push('_');
            emitted.push_str(&self.id(r));
        }
        if !self.glsl_specs.iter().any(|s| s.emitted == emitted) {
            self.glsl_specs.push(GlslSpec { fn_name: name.to_string(), binds, emitted: emitted.clone() });
        }
        let t = self.emit_operands(&rest)?;
        Ok(Some(format!("{emitted}({})", t.join(", "))))
    }

    pub(super) fn glsl_method(&mut self, recv: &Expr, name: &str, args: &[Expr]) -> Result<String> {
        let tex = match self.glsl_resource_of(recv) {
            Some(t) if matches!(self.g.resources.get(&t), Some(Ty::Texture)) => t,
            _ => bail!("GLSL: `.{name}(..)` must be called on a texture global"),
        };
        let (needs_sampler, count) = match name {
            "sample" => (true, 2),
            "sample_level" => (true, 3),
            "load" => (false, 2),
            "size" => (false, 0),
            _ => bail!("unknown texture method `{name}`"),
        };
        if args.len() != count {
            bail!("`.{name}(..)` called with the wrong number of arguments");
        }
        let smp = if needs_sampler {
            match self.glsl_resource_of(&args[0]) {
                Some(s) if matches!(self.g.resources.get(&s), Some(Ty::Sampler)) => Some(s),
                _ => bail!("GLSL: the sampler of `.{name}(..)` must be a sampler global"),
            }
        } else {
            None
        };
        let rest: Vec<&Expr> = args[usize::from(needs_sampler)..].iter().collect();
        let tys: Vec<Ty> = rest.iter().map(|a| self.type_of(a)).collect::<Result<_>>()?;
        let t = self.emit_operands(&rest)?;
        let comb = self.glsl_sampler_name(&tex, smp.as_deref());
        Ok(match name {
            "sample" => format!("texture({comb}, {})", t[0]),
            "sample_level" => format!("textureLod({comb}, {}, {})", t[0], t[1]),
            "load" => {
                let coord = if tys[0] == Ty::Vec(2, Scalar::I32) { t[0].clone() } else { format!("ivec2({})", t[0]) };
                let lod = if tys[1] == Ty::Scalar(Scalar::I32) { t[1].clone() } else { format!("int({})", t[1]) };
                format!("texelFetch({comb}, {coord}, {lod})")
            }
            _ => format!("uvec2(textureSize({comb}, 0))"),
        })
    }

    /// Functions the entry point can call, itself included.
    fn glsl_reachable(&self, entry: &str) -> HashSet<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack = vec![entry.to_string()];
        while let Some(f) = stack.pop() {
            if !seen.insert(f.clone()) {
                continue;
            }
            for it in &self.p.items {
                if let ItemKind::Fn { name, body, .. } = &it.kind {
                    if *name == f {
                        walk_block(body, &mut |e| {
                            if let Expr::Call { name: c, .. } = e {
                                if self.g.fns.contains_key(c) {
                                    stack.push(c.clone());
                                }
                            }
                        });
                    }
                }
            }
        }
        seen
    }

    fn run_glsl(&mut self, stage: Stage) -> Result<Option<(String, StageLayout)>> {
        let Some(entry) = self.p.entry(stage) else { return Ok(None) };
        let ItemKind::Fn { name: entry_name, .. } = &entry.kind else { unreachable!() };
        self.stage = Some(stage);
        self.locations = assign_locations(self.p, self.g)?;
        self.collect_resources()?;
        self.collect_uses();
        let reach = self.glsl_reachable(entry_name);

        // Only the tables this stage reads.
        let mut tables = Vec::new();
        for it in &self.p.items {
            if let ItemKind::Fn { name, body, .. } = &it.kind {
                if reach.contains(name) {
                    walk_block(body, &mut |e| {
                        if let Expr::Path(n) = e {
                            if self.tables.contains(n) && !tables.contains(n) {
                                tables.push(n.clone());
                            }
                        }
                    });
                }
            }
        }
        self.tables = tables;

        let mut fns = Vec::new();
        for it in self.fn_order() {
            let ItemKind::Fn { name, .. } = &it.kind else { unreachable!() };
            if !reach.contains(name) {
                continue;
            }
            if self.glsl_takes_resources(it) {
                continue; // written per binding, below
            }
            let text = self.emit_fn(it).map_err(|e| e.context(format!("in fn `{name}`")))?;
            if !self.p.external.contains(name) {
                fns.push(text);
            }
        }
        // Specialised copies: a copy can request more copies, so keep going until none are new.
        let mut spec_fns = Vec::new();
        let mut i = 0;
        while i < self.glsl_specs.len() {
            let spec = self.glsl_specs[i].clone();
            let p = self.p;
            let item = p
                .items
                .iter()
                .find(|it| matches!(&it.kind, ItemKind::Fn { name, .. } if *name == spec.fn_name))
                .expect("specialised function");
            self.glsl_alias = spec.binds.iter().cloned().collect();
            self.glsl_spec_name = Some(spec.emitted.clone());
            let r = self.emit_fn(item).map_err(|e| e.context(format!("in fn `{}` ({})", spec.fn_name, spec.emitted)));
            self.glsl_alias.clear();
            self.glsl_spec_name = None;
            spec_fns.push(r?);
            i += 1;
        }
        // Prototypes let ordinary functions call copies that are written after them.
        let protos: Vec<String> = spec_fns
            .iter()
            .filter_map(|t| t.split_once(" {\n").map(|(head, _)| format!("{head};")))
            .collect();
        let (io_decls, main_fn, attributes) = self.glsl_entry(entry, stage)?;

        let mut chunks: Vec<String> = Vec::new();
        chunks.push(if self.es { "#version 300 es".into() } else { "#version 330 core".into() });
        chunks.push(format!(
            "// Generated by aimer_transpiler from `{}.aimer` ({} stage). Do not edit.",
            self.p.name,
            self.stage_label()
        ));
        if self.es {
            chunks.push("precision highp float;\nprecision highp int;\nprecision highp sampler2D;".into());
        }
        for it in self.struct_order() {
            let ItemKind::Struct { name, fields, .. } = &it.kind else { unreachable!() };
            let text = self.emit_struct(it, name, fields).map_err(|e| e.context(format!("struct `{name}`")))?;
            let vertex_only = it.attr("input").is_some() && stage == Stage::Fragment;
            if !self.p.external.contains(name) && !vertex_only {
                chunks.push(text);
            }
        }

        let mut layout = StageLayout { attributes, ..StageLayout::default() };
        let used = self.uses.get(entry_name).cloned().unwrap_or_default();
        let mut decls = Vec::new();
        for r in &self.resources {
            if r.kind != ResourceKind::Uniform || !used.contains(&r.name) || self.p.external.contains(&r.name) {
                continue;
            }
            let ty = self.ty_text(&self.g.resources[&r.name])?;
            let block = format!("{}_block_{}", self.id(&r.name), self.stage_label());
            decls.push(format!("layout(std140) uniform {block} {{ {ty} {}; }};", self.glsl_uniform_name(&r.name)));
            layout.blocks.push((block, r.name.clone(), r.group, r.binding));
        }
        let find = |n: &str| self.resources.iter().find(|r| r.name == n).map(|r| (r.name.clone(), r.group, r.binding));
        let mut sampler_decls = Vec::new();
        for (tex, smp) in self.glsl_pairs.clone() {
            let comb = match &smp {
                Some(s) => format!("_smp_{}_{}", self.id(&tex), self.id(s)),
                None => format!("_smp_{}", self.id(&tex)),
            };
            sampler_decls.push(format!("uniform sampler2D {comb};"));
            let t = find(&tex).expect("texture resource");
            layout.samplers.push((comb, t, smp.as_deref().and_then(find)));
        }
        decls.extend(sampler_decls);
        if !decls.is_empty() {
            chunks.push(decls.join("\n"));
        }
        if !io_decls.is_empty() {
            chunks.push(io_decls.join("\n"));
        }
        for n in self.tables.clone() {
            chunks.push(self.emit_table(&n).map_err(|e| e.context(format!("const `{n}`")))?);
        }
        for tt in self.glsl_rem.clone() {
            chunks.push(format!("{tt} _aimer_rem({tt} x, {tt} y) {{\n    return x - y * trunc(x / y);\n}}"));
        }
        if !protos.is_empty() {
            chunks.push(protos.join("\n"));
        }
        chunks.extend(fns);
        chunks.extend(spec_fns);
        chunks.push(main_fn);
        Ok(Some((chunks.join("\n\n") + "\n", layout)))
    }

    /// The `in`/`out` declarations, `main()`, and the vertex attributes of the stage.
    fn glsl_entry(&mut self, it: &Item, stage: Stage) -> Result<(Vec<String>, String, Vec<Attribute>)> {
        let ItemKind::Fn { name, params, ret, .. } = &it.kind else { unreachable!() };
        let mut decls = Vec::new();
        let mut attributes = Vec::new();
        let mut args = Vec::new();
        let mut lines = Vec::new();
        for prm in params {
            let ty = self.g.resolve_type(&prm.ty)?;
            if let Type::Builtin(b) = &prm.ty {
                args.push(match (b.as_str(), stage) {
                    ("vertex_index", Stage::Vertex) => "uint(gl_VertexID)",
                    ("instance_index", Stage::Vertex) => "uint(gl_InstanceID)",
                    ("frag_coord", Stage::Fragment) => "gl_FragCoord",
                    _ => bail!("`@{b}` cannot be a parameter of the {} entry point", self.stage_label()),
                }
                .to_string());
                continue;
            }
            let Ty::Struct(s) = &ty else { bail!("entry parameter `{}` must be a struct or a builtin", prm.name) };
            let item = self.struct_item(s).expect("struct");
            let (want, what) = if stage == Stage::Vertex { ("input", "#[input]") } else { ("varying", "#[varying]") };
            if item.attr(want).is_none() {
                bail!("entry parameter `{}`: struct `{s}` must be {what} in the {} stage", prm.name, self.stage_label());
            }
            let per_instance = item.attr("input").is_some_and(|a| format!("{:?}", a.args).contains("instance"));
            let mut vals = Vec::new();
            for (f, loc) in self.g.structs[s].iter().zip(&self.locations[s]) {
                let fty = self.g.resolve_type(&f.ty)?;
                if matches!(fty, Ty::Array(..)) {
                    bail!("field `{}.{}`: array inputs are not supported in GLSL", s, f.name);
                }
                if mark(f, "position").is_some() {
                    if stage == Stage::Vertex {
                        bail!("`@position` cannot be a vertex input");
                    }
                    vals.push("gl_FragCoord".to_string());
                    continue;
                }
                let l = loc.expect("location");
                let tt = self.ty_text(&fty)?;
                if stage == Stage::Vertex {
                    let var = format!("_p2vs_location{l}");
                    decls.push(format!("layout(location = {l}) in {tt} {var};"));
                    attributes.push((l, var.clone(), tt, per_instance));
                    vals.push(var);
                } else {
                    let var = format!("_vs2fs_location{l}");
                    let flat = if mark(f, "flat").is_some() { "flat " } else { "" };
                    decls.push(format!("{flat}in {tt} {var};"));
                    vals.push(var);
                }
            }
            args.push(format!("{}({})", self.id(s), vals.join(", ")));
        }

        let call = format!("{}({})", self.id(name), args.join(", "));
        match (stage, ret) {
            (Stage::Vertex, Some(r)) => {
                let Ty::Struct(s) = self.g.resolve_type(r)? else { bail!("a vertex entry point must return a `#[varying]` struct") };
                lines.push(format!("    {} _out = {call};", self.id(&s)));
                for (f, loc) in self.g.structs[&s].iter().zip(&self.locations[&s]) {
                    let fname = self.id(&f.name);
                    if mark(f, "position").is_some() {
                        lines.push(format!("    gl_Position = _out.{fname};"));
                        continue;
                    }
                    let fty = self.g.resolve_type(&f.ty)?;
                    if matches!(fty, Ty::Array(..)) {
                        bail!("field `{}.{}`: array varyings are not supported in GLSL", s, f.name);
                    }
                    let l = loc.expect("location");
                    let flat = if mark(f, "flat").is_some() { "flat " } else { "" };
                    decls.push(format!("{flat}out {} _vs2fs_location{l};", self.ty_text(&fty)?));
                    lines.push(format!("    _vs2fs_location{l} = _out.{fname};"));
                }
            }
            (Stage::Vertex, None) => bail!("a vertex entry point must return a `#[varying]` struct"),
            (Stage::Fragment, Some(Type::Builtin(b))) if b == "frag_color" => {
                decls.push("layout(location = 0) out vec4 _fs2p_location0;".into());
                lines.push(format!("    _fs2p_location0 = {call};"));
            }
            (Stage::Fragment, Some(r)) => {
                let t = self.g.resolve_type(r)?;
                bail!("a fragment entry point returns `@frag_color`, not {}", t.describe());
            }
            (Stage::Fragment, None) => lines.push(format!("    {call};")),
        }
        let main_fn = format!("void main() {{\n{}\n}}", lines.join("\n"));
        Ok((decls, main_fn, attributes))
    }
}
