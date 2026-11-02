//! Memory layout the runtime has to match: std140 uniform blocks and vertex buffer formats.
//!
//! - Uniform structs are checked against std140. The compiler never inserts padding silently: a hole
//!   between fields, a vec3 tail, or an array element that std140 would round up to 16 bytes is an error,
//!   so the Rust struct that fills the buffer is the same on every target.
//! - Vertex input structs get a format per field (`as unorm8x4`, or the type's default) and tightly packed
//!   offsets. The result is written to `<name>.layout.json`, with the WebGPU, Metal and DXGI format names.

use crate::ast::*;
use crate::emit::common::assign_locations;
use crate::error::{Result, bail};
use crate::lexer::Tok;
use crate::resolve::{Program, Stage};
use crate::sema::{Globals, Scalar, Ty};

fn round_up(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

/// (alignment, size) of a type in a std140 uniform block. `path` names the field in errors.
fn std140(g: &Globals, ty: &Ty, path: &str) -> Result<(u32, u32)> {
    Ok(match ty {
        Ty::Scalar(Scalar::Bool) | Ty::Vec(_, Scalar::Bool) => {
            bail!("{path}: `bool` cannot be stored in a uniform block, use `u32`")
        }
        Ty::Scalar(_) => (4, 4),
        Ty::Vec(2, _) => (8, 8),
        Ty::Vec(3, _) => (16, 12),
        Ty::Vec(_, _) => (16, 16),
        Ty::Mat(4) => (16, 64),
        Ty::Mat(n) => bail!(
            "{path}: `mat{n}` has a different column stride in std140 and WGSL; use `mat4` or `vec4` fields in a uniform block"
        ),
        Ty::Array(elem, n) => {
            let Some(count) = g.const_int(n) else { bail!("{path}: uniform array size must be a constant") };
            let (_, es) = std140(g, elem, path)?;
            let stride = round_up(es, 16);
            if stride != es {
                bail!(
                    "{path}: array element is {es} bytes but std140 pads it to {stride}; make the element a multiple of 16 bytes (for example `vec4`)"
                );
            }
            (16, stride * count as u32)
        }
        Ty::Struct(name) => {
            let (align, size, _) = struct_layout(g, name)?;
            (align, size)
        }
        other => bail!("{path}: {} cannot be stored in a uniform block", other.describe()),
    })
}

/// (alignment, size, fields as (name, offset, size)) of a struct, with no implicit padding allowed.
fn struct_layout(g: &Globals, name: &str) -> Result<(u32, u32, Vec<(String, u32, u32)>)> {
    let Some(fields) = g.structs.get(name) else { bail!("unknown struct `{name}`") };
    let mut offset = 0u32;
    let mut align = 16u32;
    let mut out = Vec::new();
    for f in fields {
        let path = format!("uniform struct `{name}`, field `{}`", f.name);
        let ty = g.resolve_type(&f.ty)?;
        let (a, s) = std140(g, &ty, &path)?;
        let at = round_up(offset, a);
        if at != offset {
            bail!(
                "{path}: needs {a}-byte alignment but the previous field ends at byte {offset}; std140 would leave {} bytes of padding. Add an explicit `_pad` field",
                at - offset
            );
        }
        out.push((f.name.clone(), at, s));
        offset = at + s;
        align = align.max(a);
    }
    let size = round_up(offset, align);
    if size != offset {
        bail!(
            "uniform struct `{name}` is {offset} bytes but std140 rounds its size up to {size}. Add explicit padding fields so the Rust side can use the same size"
        );
    }
    Ok((align, size, out))
}

/// Byte size of a uniform struct, after checking it.
pub fn uniform_size(g: &Globals, name: &str) -> Result<u32> {
    Ok(struct_layout(g, name)?.1)
}

/// Check every uniform struct and every vertex input struct of the program.
pub fn check(p: &Program) -> Result<()> {
    for it in &p.items {
        if let ItemKind::Resource { kind: ResourceKind::Uniform, name, .. } = &it.kind {
            if let Some(Ty::Struct(s)) = p.globals.resources.get(name) {
                struct_layout(&p.globals, s).map_err(|e| e.context(format!("uniform `{name}`")))?;
            }
        }
        if let ItemKind::Struct { name, fields, uniform, .. } = &it.kind {
            if *uniform {
                struct_layout(&p.globals, name)?;
            }
            if it.attr("input").is_none() {
                if let Some(f) = fields.iter().find(|f| f.format.is_some()) {
                    bail!("field `{name}.{}`: `as <format>` is only valid in an `#[input]` struct", f.name);
                }
            }
        }
    }
    vertex_layout(p)?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Uint,
    Sint,
    Unorm,
    Snorm,
    Float,
}

/// A WebGPU vertex format.
#[derive(Clone, Debug)]
pub struct Format {
    pub name: String,
    kind: Kind,
    bits: u32,
    count: u32,
    packed: bool,
}

impl Format {
    pub fn parse(name: &str) -> Option<Format> {
        if name == "unorm10_10_10_2" {
            return Some(Format { name: name.into(), kind: Kind::Unorm, bits: 10, count: 4, packed: true });
        }
        let (kind, rest) = [("unorm", Kind::Unorm), ("snorm", Kind::Snorm), ("uint", Kind::Uint), ("sint", Kind::Sint), ("float", Kind::Float)]
            .iter()
            .find_map(|(p, k)| name.strip_prefix(p).map(|r| (*k, r)))?;
        let (bits, count) = match rest.split_once('x') {
            Some((b, c)) => (b.parse::<u32>().ok()?, c.parse::<u32>().ok()?),
            None => (rest.parse::<u32>().ok()?, 1),
        };
        let ok = match (kind, bits) {
            (Kind::Float, 16) => matches!(count, 2 | 4),
            (Kind::Float, 32) => (1..=4).contains(&count),
            (Kind::Uint | Kind::Sint, 32) => (1..=4).contains(&count),
            (Kind::Uint | Kind::Sint | Kind::Unorm | Kind::Snorm, 8 | 16) => matches!(count, 2 | 4),
            _ => false,
        };
        // `float32` has no `x1` spelling, and `float32x1` is not a format.
        if rest.contains("x1") || !ok {
            return None;
        }
        Some(Format { name: name.into(), kind, bits, count, packed: false })
    }

    /// The format a field gets without `as`.
    fn default_for(ty: &Ty) -> Option<Format> {
        let (s, n) = match ty {
            Ty::Scalar(s) => (*s, 1),
            Ty::Vec(n, s) => (*s, *n as u32),
            _ => return None,
        };
        let base = match s {
            Scalar::F32 => "float32",
            Scalar::U32 => "uint32",
            Scalar::I32 => "sint32",
            Scalar::Bool => return None,
        };
        Format::parse(&if n == 1 { base.to_string() } else { format!("{base}x{n}") })
    }

    pub fn size(&self) -> u32 {
        if self.packed { 4 } else { self.bits / 8 * self.count }
    }

    /// Attribute offsets must be a multiple of min(4, size).
    pub fn align(&self) -> u32 {
        self.size().min(4)
    }

    /// The shader type the format is read as.
    fn shader_ty(&self) -> Ty {
        let s = match self.kind {
            Kind::Uint => Scalar::U32,
            Kind::Sint => Scalar::I32,
            _ => Scalar::F32,
        };
        if self.count == 1 { Ty::Scalar(s) } else { Ty::Vec(self.count as u8, s) }
    }

    pub fn metal(&self) -> String {
        if self.packed {
            return "MTLVertexFormatUInt1010102Normalized".into();
        }
        let norm = matches!(self.kind, Kind::Unorm | Kind::Snorm);
        let base = match (self.kind, self.bits) {
            (Kind::Float, 16) => "Half",
            (Kind::Float, _) => "Float",
            (Kind::Uint | Kind::Unorm, 8) => "UChar",
            (Kind::Sint | Kind::Snorm, 8) => "Char",
            (Kind::Uint | Kind::Unorm, 16) => "UShort",
            (Kind::Sint | Kind::Snorm, 16) => "Short",
            (Kind::Uint | Kind::Unorm, _) => "UInt",
            _ => "Int",
        };
        let count = if self.count > 1 { self.count.to_string() } else { String::new() };
        format!("MTLVertexFormat{base}{count}{}", if norm { "Normalized" } else { "" })
    }

    pub fn dxgi(&self) -> String {
        if self.packed {
            return "DXGI_FORMAT_R10G10B10A2_UNORM".into();
        }
        let comps: String = "RGBA".chars().take(self.count as usize).map(|c| format!("{c}{}", self.bits)).collect();
        let suffix = match self.kind {
            Kind::Uint => "UINT",
            Kind::Sint => "SINT",
            Kind::Unorm => "UNORM",
            Kind::Snorm => "SNORM",
            Kind::Float => "FLOAT",
        };
        format!("DXGI_FORMAT_{comps}_{suffix}")
    }

    pub fn wgpu(&self) -> String {
        self.name.replace('_', "-")
    }
}

#[derive(Debug, Clone)]
pub struct VertexAttribute {
    pub location: u32,
    pub field: String,
    pub format: Format,
    pub offset: u32,
}

#[derive(Debug, Clone)]
pub struct VertexBuffer {
    pub struct_name: String,
    pub per_instance: bool,
    pub stride: u32,
    pub attributes: Vec<VertexAttribute>,
}

/// One buffer per `#[input]` struct taken by the vertex entry point, fields packed in declaration order.
pub fn vertex_layout(p: &Program) -> Result<Vec<VertexBuffer>> {
    let Some(Item { kind: ItemKind::Fn { params, .. }, .. }) = p.entry(Stage::Vertex) else { return Ok(Vec::new()) };
    let g = &p.globals;
    let locations = assign_locations(p, g)?;
    let mut buffers: Vec<VertexBuffer> = Vec::new();
    for prm in params {
        let Ok(Ty::Struct(s)) = g.resolve_type(&prm.ty) else { continue };
        let Some(item) = p.items.iter().find(|i| matches!(&i.kind, ItemKind::Struct { name, .. } if *name == s)) else { continue };
        let Some(attr) = item.attr("input") else { continue };
        if buffers.iter().any(|b| b.struct_name == s) {
            continue;
        }
        let per_instance = attr.args.iter().any(|t| matches!(t, Tok::Ident(i) if i == "instance"));
        let mut offset = 0u32;
        let mut attributes = Vec::new();
        for (f, loc) in g.structs[&s].iter().zip(&locations[&s]) {
            let ty = g.resolve_type(&f.ty)?;
            let path = format!("input `{s}.{}`", f.name);
            let format = match &f.format {
                Some(n) => {
                    let Some(fmt) = Format::parse(n) else { bail!("{path}: unknown vertex format `{n}`") };
                    if fmt.shader_ty() != ty {
                        bail!(
                            "{path}: format `{n}` is read as {} but the field is {}",
                            fmt.shader_ty().describe(),
                            ty.describe()
                        );
                    }
                    fmt
                }
                None => match Format::default_for(&ty) {
                    Some(f) => f,
                    None => bail!(
                        "{path}: {} cannot be a vertex attribute; use a scalar or vector (split matrices into `vec4` columns)",
                        ty.describe()
                    ),
                },
            };
            offset = round_up(offset, format.align());
            attributes.push(VertexAttribute { location: loc.expect("input location"), field: f.name.clone(), offset, format: format.clone() });
            offset += format.size();
        }
        buffers.push(VertexBuffer { struct_name: s, per_instance, stride: round_up(offset, 4), attributes });
    }
    Ok(buffers)
}

/// `<name>.layout.json` for every target: vertex buffers and uniform block sizes.
pub fn json(p: &Program) -> Result<String> {
    let buffers = vertex_layout(p)?;
    let mut s = format!("{{\n  \"shader\": \"{}\",\n  \"vertex_buffers\": [", p.name);
    let items: Vec<String> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let attrs: Vec<String> = b
                .attributes
                .iter()
                .map(|a| {
                    format!(
                        "        {{ \"location\": {}, \"field\": \"{}\", \"offset\": {}, \"format\": \"{}\", \"metal\": \"{}\", \"dxgi\": \"{}\" }}",
                        a.location,
                        a.field,
                        a.offset,
                        a.format.wgpu(),
                        a.format.metal(),
                        a.format.dxgi()
                    )
                })
                .collect();
            format!(
                "\n    {{\n      \"buffer\": {i},\n      \"struct\": \"{}\",\n      \"step\": \"{}\",\n      \"stride\": {},\n      \"attributes\": [\n{}\n      ]\n    }}",
                b.struct_name,
                if b.per_instance { "instance" } else { "vertex" },
                b.stride,
                attrs.join(",\n")
            )
        })
        .collect();
    s.push_str(&items.join(","));
    s.push_str(if items.is_empty() { "],\n" } else { "\n  ],\n" });
    s.push_str("  \"uniform_blocks\": [");
    let mut blocks = Vec::new();
    for it in &p.items {
        if let ItemKind::Resource { kind: ResourceKind::Uniform, name, .. } = &it.kind {
            if let Some(Ty::Struct(st)) = p.globals.resources.get(name) {
                let bind = it.attr("bind").map(|a| a.args.iter().filter_map(|t| if let Tok::Int(n) = t { Some(n.clone()) } else { None }).collect::<Vec<_>>());
                let (g_, b_) = match bind.as_deref() {
                    Some([g, b]) => (g.clone(), b.clone()),
                    _ => ("0".into(), "0".into()),
                };
                blocks.push(format!(
                    "\n    {{ \"resource\": \"{name}\", \"struct\": \"{st}\", \"group\": {g_}, \"binding\": {b_}, \"size\": {} }}",
                    uniform_size(&p.globals, st)?
                ));
            }
        }
    }
    s.push_str(&blocks.join(","));
    s.push_str(if blocks.is_empty() { "]\n}\n" } else { "\n  ]\n}\n" });
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_parse() {
        for ok in ["float32", "float32x3", "unorm8x4", "uint16x2", "sint32x4", "float16x4", "unorm10_10_10_2", "snorm16x2"] {
            assert!(Format::parse(ok).is_some(), "{ok}");
        }
        for bad in ["float32x1", "unorm8", "unorm8x3", "float8x2", "uint", "unorm32x2", "foo"] {
            assert!(Format::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn native_names() {
        let f = Format::parse("unorm8x4").unwrap();
        assert_eq!(f.metal(), "MTLVertexFormatUChar4Normalized");
        assert_eq!(f.dxgi(), "DXGI_FORMAT_R8G8B8A8_UNORM");
        let f = Format::parse("float32x3").unwrap();
        assert_eq!(f.metal(), "MTLVertexFormatFloat3");
        assert_eq!(f.dxgi(), "DXGI_FORMAT_R32G32B32_FLOAT");
        assert_eq!(Format::parse("uint32").unwrap().metal(), "MTLVertexFormatUInt");
        assert_eq!(Format::parse("sint16x2").unwrap().metal(), "MTLVertexFormatShort2");
    }
}
