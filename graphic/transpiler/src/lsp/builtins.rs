//! What the language itself provides: keywords, types, built-in functions, attributes.

pub const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "const", "struct", "uniform", "texture", "sampler", "pub", "import", "module", "shader", "if",
    "else", "match", "loop", "while", "for", "in", "return", "break", "continue", "discard", "as", "true", "false",
];

pub const TYPES: &[(&str, &str)] = &[
    ("f32", "32-bit float"),
    ("i32", "32-bit signed integer"),
    ("u32", "32-bit unsigned integer"),
    ("bool", "boolean"),
    ("vec2", "2-component vector, `vec2<T>` for `i32`, `u32` or `bool` (default `f32`)"),
    ("vec3", "3-component vector, `vec3<T>` for `i32`, `u32` or `bool` (default `f32`)"),
    ("vec4", "4-component vector, `vec4<T>` for `i32`, `u32` or `bool` (default `f32`)"),
    ("mat2", "2x2 `f32` matrix"),
    ("mat3", "3x3 `f32` matrix"),
    ("mat4", "4x4 `f32` matrix"),
    ("texture2d", "2D texture resource"),
    ("sampler", "texture sampler resource"),
];

/// `@name` builtin inputs and outputs, and `@location`.
pub const BUILTIN_VARS: &[(&str, &str)] = &[
    ("position", "Vertex output clip-space position."),
    ("vertex_index", "Index of the current vertex (`u32`)."),
    ("frag_coord", "Fragment position in framebuffer pixels (`vec4`)."),
    ("frag_color", "Fragment colour output (`vec4`)."),
    ("location", "`@location(n)`: explicit varying or output location."),
];

pub const ATTRS: &[(&str, &str)] = &[
    ("vertex", "Marks the vertex entry point."),
    ("fragment", "Marks the fragment entry point."),
    ("bind", "`#[bind(group, binding)]`: binding slot of a resource."),
    ("input", "`#[input(step = vertex | instance)]`: vertex buffer input struct."),
    ("varying", "Struct passed from the vertex to the fragment stage."),
    ("select", "On an `if`: emit a `select` / ternary instead of a branch."),
];

pub const SWIZZLES: &[&str] = &["x", "y", "z", "w", "xy", "xyz", "xyzw", "r", "g", "b", "a", "rg", "rgb", "rgba"];

const UNARY: &[&str] = &[
    "abs", "sign", "floor", "ceil", "round", "trunc", "fract", "sqrt", "inverseSqrt", "sin", "cos", "tan", "asin", "acos",
    "atan", "sinh", "cosh", "tanh", "exp", "exp2", "log", "log2", "saturate", "normalize", "dpdx", "dpdy", "fwidth",
    "degrees", "radians",
];
const BINARY: &[&str] = &["min", "max", "pow", "atan2", "reflect", "cross"];
const TERNARY: &[&str] = &["clamp", "mix", "fma"];

/// All built-in function names (math functions and constructors).
pub fn function_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    v.extend_from_slice(UNARY);
    v.extend_from_slice(BINARY);
    v.extend_from_slice(TERNARY);
    v.extend_from_slice(&["step", "smoothstep", "length", "dot", "distance", "all", "any", "bitcast"]);
    v
}

pub const TEXTURE_METHODS: &[(&str, &str, &str)] = &[
    ("sample", "sample(sampler, uv: vec2) -> vec4", "Sample the texture with a sampler (implicit level of detail, fragment stage)."),
    ("sample_level", "sample_level(sampler, uv: vec2, level: f32) -> vec4", "Sample the texture at an explicit mip level."),
    ("load", "load(coord: vec2<i32>, level: i32) -> vec4", "Read one texel without filtering."),
    ("size", "size() -> vec2<u32>", "Texture dimensions in texels."),
];

pub fn type_doc(name: &str) -> Option<String> {
    TYPES.iter().find(|(n, _)| *n == name).map(|(n, d)| format!("```aimer\n{n}\n```\n{d}"))
}

/// Hover text for a built-in function or texture method.
pub fn function_doc(name: &str) -> Option<String> {
    let sig = if UNARY.contains(&name) {
        format!("{name}(x: T) -> T")
    } else if BINARY.contains(&name) {
        format!("{name}(a: T, b: T) -> T")
    } else if TERNARY.contains(&name) {
        format!("{name}(a: T, b: T, c: T) -> T")
    } else {
        match name {
            "step" => "step(edge: T, x: T) -> T".to_string(),
            "smoothstep" => "smoothstep(edge0: T, edge1: T, x: T) -> T".to_string(),
            "length" => "length(v: vecN) -> f32".to_string(),
            "dot" => "dot(a: vecN, b: vecN) -> f32".to_string(),
            "distance" => "distance(a: vecN, b: vecN) -> f32".to_string(),
            "all" | "any" => format!("{name}(v: vecN<bool>) -> bool"),
            "bitcast" => "bitcast<T>(x) -> T".to_string(),
            _ => {
                if let Some((_, sig, doc)) = TEXTURE_METHODS.iter().find(|(n, _, _)| *n == name) {
                    return Some(format!("```aimer\n(method) {sig}\n```\n{doc}"));
                }
                return None;
            }
        }
    };
    Some(format!("```aimer\n(builtin) fn {sig}\n```\nBuilt-in function. `T` is a scalar or vector type."))
}

pub fn builtin_var_doc(name: &str) -> Option<String> {
    BUILTIN_VARS.iter().find(|(n, _)| *n == name).map(|(n, d)| format!("```aimer\n@{n}\n```\n{d}"))
}

pub fn attr_doc(name: &str) -> Option<String> {
    ATTRS.iter().find(|(n, _)| *n == name).map(|(n, d)| format!("```aimer\n#[{n}]\n```\n{d}"))
}
