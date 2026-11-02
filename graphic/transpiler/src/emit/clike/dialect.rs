//! What differs between Metal Shading Language and HLSL: names, type spelling, intrinsics.

use super::Dialect;
use crate::sema::{Scalar, Ty, is_constructor, is_intrinsic};

const CXX_KEYWORDS: &[&str] = &[
    "alignas", "alignof", "and", "and_eq", "asm", "auto", "bitand", "bitor", "bool", "break", "case", "catch", "char",
    "class", "compl", "const", "const_cast", "constexpr", "continue", "decltype", "default", "delete", "do", "double",
    "dynamic_cast", "else", "enum", "explicit", "export", "extern", "false", "float", "for", "friend", "goto", "if",
    "inline", "int", "long", "main", "mutable", "namespace", "new", "noexcept", "not", "not_eq", "nullptr", "operator",
    "or", "or_eq", "private", "protected", "public", "register", "reinterpret_cast", "return", "short", "signed",
    "sizeof", "static", "static_assert", "static_cast", "struct", "switch", "template", "this", "throw", "true", "try",
    "typedef", "typeid", "typename", "union", "unsigned", "using", "virtual", "void", "volatile", "while", "xor",
    "xor_eq",
];

const MSL_EXTRA: &[&str] = &[
    "in", "out", "half", "uint", "uchar", "ushort", "ulong", "texture", "sampler", "constant", "device", "thread",
    "threadgroup", "threadgroup_imageblock", "kernel", "vertex", "fragment", "metal", "level", "select", "rint",
    "fmod", "rsqrt", "dfdx", "dfdy", "as_type", "discard_fragment", "array", "vec", "matrix", "atomic", "bfloat",
    "lerp", "frac", "mad", "ddx", "ddy",
];

const HLSL_EXTRA: &[&str] = &[
    "in", "out", "inout", "uint", "half", "dword", "vector", "matrix", "texture", "sampler", "sampler_state",
    "SamplerState", "Texture2D", "cbuffer", "tbuffer", "register", "packoffset", "precise", "linear", "nointerpolation",
    "noperspective", "centroid", "sample", "point", "line", "triangle", "shared", "uniform", "interface", "row_major",
    "column_major", "snorm", "unorm", "string", "technique", "pass", "compile", "target", "groupshared", "globallycoherent",
    "asfloat", "asint", "asuint", "lerp", "frac", "mad", "ddx", "ddy", "rsqrt", "rint", "fmod", "select", "level",
    "dfdx", "dfdy", "discard", "fxgroup", "pixelfragment", "vertexfragment", "NULL", "min16float", "min10float",
    "min16int", "min12int", "min16uint", "refract", "faceforward", "mul", "transpose", "determinant", "lit",
];

const GLSL_EXTRA: &[&str] = &[
    "attribute", "varying", "uniform", "buffer", "shared", "layout", "centroid", "flat", "smooth", "noperspective",
    "patch", "sample", "in", "out", "inout", "input", "output", "common", "partition", "active", "filter", "resource",
    "precision", "highp", "mediump", "lowp", "invariant", "coherent", "volatile", "restrict", "readonly",
    "writeonly", "subroutine", "fixed", "half", "long", "short", "double", "unsigned", "hvec2", "hvec3", "hvec4",
    "fvec2", "fvec3", "fvec4", "dvec2", "dvec3", "dvec4", "sizeof", "cast", "packed", "texture", "texture2D",
    "sampler1D", "sampler2D", "sampler3D", "samplerCube", "sampler2DShadow", "isampler2D", "usampler2D", "image2D",
    "external", "noinline", "lessThan", "lessThanEqual", "greaterThan", "greaterThanEqual", "equal", "notEqual",
    "not", "mod", "modf", "isnan", "isinf", "textureLod", "texelFetch", "textureSize", "textureGrad", "floatBitsToInt",
    "floatBitsToUint", "intBitsToFloat", "uintBitsToFloat", "roundEven", "dFdx", "dFdy", "inversesqrt", "smoothstep",
    "transpose", "determinant", "inverse", "faceforward", "refract", "length", "dot", "mix", "step", "min", "max",
    "abs", "clamp", "main", "radians", "degrees", "outerProduct", "matrixCompMult", "discard", "lowp",
];

/// Source name to a legal name in the target. Reserved words, intrinsics and type names get a `_`.
pub fn ident(d: Dialect, s: &str) -> String {
    let extra = match d {
        Dialect::Msl => MSL_EXTRA,
        Dialect::Hlsl => HLSL_EXTRA,
        Dialect::Glsl => GLSL_EXTRA,
    };
    let is_type_name = ["float", "int", "uint", "bool"]
        .iter()
        .any(|b| s.strip_prefix(b).is_some_and(|r| matches!(r, "2" | "3" | "4" | "2x2" | "3x3" | "4x4")))
        || ["vec", "ivec", "uvec", "bvec"].iter().any(|b| s.strip_prefix(b).is_some_and(|r| matches!(r, "2" | "3" | "4")))
        || s.strip_prefix("mat").is_some_and(|r| matches!(r, "2" | "3" | "4" | "2x2" | "3x3" | "4x4"));
    if CXX_KEYWORDS.contains(&s)
        || extra.contains(&s)
        || is_intrinsic(s)
        || is_constructor(s)
        || is_type_name
    {
        format!("{s}_")
    } else if d == Dialect::Glsl && (s.starts_with("gl_") || s.contains("__")) {
        format!("v_{s}")
    } else if s == "_" || s.starts_with("__") || s.strip_prefix('_').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase())) {
        format!("v{s}")
    } else {
        s.to_string()
    }
}

fn glsl_vec_prefix(s: Scalar) -> &'static str {
    match s {
        Scalar::F32 => "",
        Scalar::I32 => "i",
        Scalar::U32 => "u",
        Scalar::Bool => "b",
    }
}

fn scalar_name(s: Scalar) -> &'static str {
    match s {
        Scalar::F32 => "float",
        Scalar::I32 => "int",
        Scalar::U32 => "uint",
        Scalar::Bool => "bool",
    }
}

/// Type spelling for everything that is not a struct or an array.
pub fn scalar_like_text(d: Dialect, t: &Ty) -> Option<String> {
    if d == Dialect::Glsl {
        return Some(match t {
            Ty::Scalar(s) => scalar_name(*s).to_string(),
            Ty::Vec(n, s) => format!("{}vec{n}", glsl_vec_prefix(*s)),
            Ty::Mat(n) => format!("mat{n}"),
            _ => return None,
        });
    }
    Some(match t {
        Ty::Scalar(s) => scalar_name(*s).to_string(),
        Ty::Vec(n, s) => format!("{}{n}", scalar_name(*s)),
        Ty::Mat(n) => format!("float{n}x{n}"),
        Ty::Texture => match d {
            Dialect::Msl => "texture2d<float>".into(),
            Dialect::Hlsl => "Texture2D<float4>".into(),
            Dialect::Glsl => unreachable!(),
        },
        Ty::Sampler => match d {
            Dialect::Msl => "sampler".into(),
            Dialect::Hlsl => "SamplerState".into(),
            Dialect::Glsl => unreachable!(),
        },
        _ => return None,
    })
}

/// Rewrites a call to a shared intrinsic. `args` are already rendered, `arg0` is the type of the first one.
pub fn intrinsic_call(d: Dialect, name: &str, args: &[String], arg0: Option<&Ty>) -> String {
    let join = args.join(", ");
    match (d, name) {
        (Dialect::Glsl, "inverseSqrt") => format!("inversesqrt({join})"),
        (Dialect::Glsl, "round") => format!("roundEven({join})"),
        (Dialect::Glsl, "dpdx") => format!("dFdx({join})"),
        (Dialect::Glsl, "dpdy") => format!("dFdy({join})"),
        (Dialect::Glsl, "atan2") => format!("atan({join})"),
        (Dialect::Glsl, "saturate") => format!("clamp({join}, 0.0, 1.0)"),
        (Dialect::Msl, "inverseSqrt") | (Dialect::Hlsl, "inverseSqrt") => format!("rsqrt({join})"),
        // WGSL `round` rounds halves to even: that is `rint` in MSL and `round` in HLSL.
        (Dialect::Msl, "round") => format!("rint({join})"),
        (Dialect::Msl, "dpdx") => format!("dfdx({join})"),
        (Dialect::Msl, "dpdy") => format!("dfdy({join})"),
        (Dialect::Hlsl, "dpdx") => format!("ddx({join})"),
        (Dialect::Hlsl, "dpdy") => format!("ddy({join})"),
        (Dialect::Hlsl, "fract") => format!("frac({join})"),
        (Dialect::Hlsl, "mix") => format!("lerp({join})"),
        (Dialect::Hlsl, "fma") => format!("mad({join})"),
        // HLSL `sign` returns an int.
        (Dialect::Hlsl, "sign") => {
            let ty = arg0.and_then(|t| scalar_like_text(d, t)).unwrap_or_else(|| "float".into());
            format!("(({ty})sign({join}))")
        }
        (Dialect::Msl, "degrees") => format!("({join} * 57.29578)"),
        (Dialect::Msl, "radians") => format!("({join} * 0.017453292)"),
        _ => format!("{name}({join})"),
    }
}
