//! Checking an effect shader against the contract before any GPU work.

use aimer_transpiler::ast::{Item, ItemKind, ResourceKind, Type};
use aimer_transpiler::layout;
use aimer_transpiler::lexer::Tok;
use aimer_transpiler::resolve::{Program, Stage};
use aimer_transpiler::sema::Ty;

use crate::header::{HEADER_FIELDS, HEADER_SIZE};
use crate::stdlib::StdLib;

/// An effect shader that passed the contract check.
pub struct EffectProgram {
    program: Program,
    params_size: usize,
    samples_backdrop: bool,
}

impl EffectProgram {
    /// Bytes of parameters the shader's `Frame` struct holds after the header.
    #[inline]
    pub fn params_size(&self) -> usize {
        self.params_size
    }

    /// Whether the shader samples the backdrop.
    #[inline]
    pub fn samples_backdrop(&self) -> bool {
        self.samples_backdrop
    }

    pub(crate) fn program(&self) -> &Program {
        &self.program
    }
}

/// Parses `source` and checks it against the effect contract.
///
/// `samples_backdrop` says whether the effect declares
/// [`ShaderEffect::SAMPLES_BACKDROP`](crate::ShaderEffect::SAMPLES_BACKDROP).
///
/// # Errors
///
/// Returns a message naming the first thing wrong: a parse or type error, a
/// missing entry point, a vertex input, a header field that is missing or out
/// of order, or a resource at an unexpected binding.
pub fn analyze(source: &str, samples_backdrop: bool) -> Result<EffectProgram, String> {
    let program = aimer_transpiler::load_program(source, "effect", &StdLib)
        .map_err(|error| error.to_string())?;

    for (stage, label) in [(Stage::Vertex, "vertex"), (Stage::Fragment, "fragment")] {
        if program.entry(stage).is_none() {
            return Err(format!("the shader has no #[{label}] entry point"));
        }
    }
    if !layout::vertex_layout(&program)
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err(
            "the vertex stage takes a vertex buffer; effects draw from `rect` in the uniform and read only @vertex_index"
                .to_owned(),
        );
    }

    let mut uniform = None;
    let mut texture = None;
    let mut sampler = None;
    for item in &program.items {
        let ItemKind::Resource { kind, name, .. } = &item.kind else {
            continue;
        };
        let slot = bind_slot(item).ok_or_else(|| format!("resource `{name}` has no #[bind(group, binding)]"))?;
        let found = match kind {
            ResourceKind::Uniform => &mut uniform,
            ResourceKind::Texture => &mut texture,
            ResourceKind::Sampler => &mut sampler,
        };
        if found.replace((name.clone(), slot)).is_some() {
            return Err(format!(
                "the shader declares more than one {} (second is `{name}`); an effect has one uniform{}",
                kind_name(*kind),
                if samples_backdrop { ", one backdrop texture and one sampler" } else { "" }
            ));
        }
    }

    let (uniform_name, uniform_slot) = uniform.ok_or("the shader declares no uniform; it needs `#[bind(0, 0)] uniform frame: Frame`")?;
    if uniform_slot != (0, 0) {
        return Err(format!(
            "uniform `{uniform_name}` is bound at {uniform_slot:?}; the frame uniform must be at (0, 0)"
        ));
    }
    for (what, found, expected) in [("texture", &texture, (0, 1)), ("sampler", &sampler, (0, 2))] {
        match (samples_backdrop, found) {
            (true, None) => {
                return Err(format!(
                    "SAMPLES_BACKDROP is set but the shader declares no backdrop {what}; add one at {expected:?}"
                ));
            }
            (true, Some((name, slot))) if *slot != expected => {
                return Err(format!("the backdrop {what} `{name}` is bound at {slot:?}; it must be at {expected:?}"));
            }
            (false, Some((name, _))) => {
                return Err(format!(
                    "the shader declares {what} `{name}` but the effect does not set SAMPLES_BACKDROP"
                ));
            }
            _ => {}
        }
    }

    let Some(Ty::Struct(struct_name)) = program.globals.resources.get(&uniform_name) else {
        return Err(format!("uniform `{uniform_name}` is not a struct"));
    };
    let fields = program
        .globals
        .structs
        .get(struct_name)
        .ok_or_else(|| format!("uniform struct `{struct_name}` is not defined"))?;
    for (index, (name, ty)) in HEADER_FIELDS.iter().enumerate() {
        let ok = fields.get(index).is_some_and(|field| {
            field.name == *name && matches!(&field.ty, Type::Named(found, args) if found == ty && args.is_empty())
        });
        if !ok {
            let found = fields
                .get(index)
                .map(|field| format!("`{}`", field.name))
                .unwrap_or_else(|| "nothing".to_owned());
            return Err(format!(
                "`{struct_name}` must begin with the frame header; field {index} should be `{name}: {ty}` but is {found}"
            ));
        }
    }
    let size = layout::uniform_size(&program.globals, struct_name).map_err(|error| error.to_string())? as usize;
    Ok(EffectProgram {
        program,
        params_size: size - HEADER_SIZE,
        samples_backdrop,
    })
}

fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Uniform => "uniform",
        ResourceKind::Texture => "texture",
        ResourceKind::Sampler => "sampler",
    }
}

/// The `(group, binding)` of a resource's `#[bind(group, binding)]`.
fn bind_slot(item: &Item) -> Option<(u32, u32)> {
    let numbers: Vec<u32> = item
        .attr("bind")?
        .args
        .iter()
        .filter_map(|token| match token {
            Tok::Int(text) => text.parse().ok(),
            _ => None,
        })
        .collect();
    match numbers.as_slice() {
        [group, binding] => Some((*group, *binding)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WATER: &str = include_str!("../tests/shaders/water.aimer");

    fn err(source: &str, backdrop: bool) -> String {
        match analyze(source, backdrop) {
            Ok(_) => panic!("the shader should have been rejected"),
            Err(error) => error,
        }
    }

    #[test]
    fn the_reference_shader_passes() {
        let program = analyze(WATER, true).unwrap();
        // `time`, `amplitude`, `frequency`, `_pad`.
        assert_eq!(program.params_size(), 16);
        assert!(program.samples_backdrop());
    }

    #[test]
    fn a_syntax_error_is_reported() {
        let error = err("shader broken; fn (", false);
        assert!(error.contains("line 1") && error.contains("expected identifier"), "{error}");
    }

    #[test]
    fn a_missing_import_is_reported() {
        let source = WATER.replace("aimer::clip::clip_alpha", "aimer::nowhere::thing");
        assert!(err(&source, true).contains("nowhere"));
    }

    #[test]
    fn a_renamed_header_field_is_reported() {
        let source = WATER.replace("opacity: f32", "alpha: f32");
        let error = err(&source, true);
        assert!(error.contains("opacity") && error.contains("alpha"), "{error}");
    }

    #[test]
    fn a_retyped_header_field_is_reported() {
        let source = WATER.replace("rect: vec4,\n    clip_rect", "rect: vec2,\n    pad2: vec2,\n    clip_rect");
        assert!(err(&source, true).contains("rect"));
    }

    #[test]
    fn a_struct_that_is_all_header_has_no_parameters() {
        let source = WATER
            .replace("    time: f32,\n    amplitude: f32,\n    frequency: f32,\n    _pad: f32,\n", "")
            .replace("frame.time", "0.0")
            .replace("frame.amplitude", "1.0")
            .replace("frame.frequency", "1.0");
        assert_eq!(analyze(&source, true).unwrap().params_size(), 0);
    }

    #[test]
    fn the_uniform_must_be_at_group_zero_binding_zero() {
        let source = WATER.replace("#[bind(0, 0)]\nuniform frame", "#[bind(3, 0)]\nuniform frame");
        let error = err(&source, true);
        assert!(error.contains("(0, 0)"), "{error}");
    }

    #[test]
    fn a_backdrop_effect_must_declare_the_texture_and_sampler() {
        let source = WATER.replace("#[bind(0, 1)]\ntexture backdrop", "#[bind(0, 4)]\ntexture backdrop");
        let error = err(&source, true);
        assert!(error.contains("(0, 1)"), "{error}");
    }

    #[test]
    fn an_effect_that_does_not_ask_for_the_backdrop_may_not_bind_it() {
        let error = err(WATER, false);
        assert!(error.contains("SAMPLES_BACKDROP"), "{error}");
    }

    #[test]
    fn vertex_buffers_are_not_part_of_the_contract() {
        let source = WATER
            .replace("fn vs_main(vi: @vertex_index)", "fn vs_main(vi: @vertex_index, extra: Extra)")
            .replace(
                "#[varying]",
                "#[input(step = instance)]\nstruct Extra {\n    unused: vec4,\n}\n\n#[varying]",
            );
        let error = err(&source, true);
        assert!(error.contains("vertex buffer") || error.contains("Extra"), "{error}");
    }

    #[test]
    fn both_entry_points_are_required() {
        let source = WATER.replace("#[fragment]\nfn fs_main", "fn fs_main");
        let error = err(&source, true);
        assert!(error.contains("fragment"), "{error}");
    }
}
