//! The layout package must carry everything the hand-written `.glslpack` files
//! do, for every built-in shader, in both GLSL dialects.

use std::collections::BTreeSet;
use std::path::PathBuf;

use aimer_cupid_gpu::glsl_package::{self, GlslResource};
use aimer_cupid_gpu::glslpack;
use aimer_transpiler::resolve::FsLoader;

const SHADERS: [(&str, &[u8]); 9] = [
    ("rect", glslpack::RECT),
    ("image", glslpack::IMAGE),
    ("text", glslpack::TEXT),
    ("text_color", glslpack::TEXT_COLOR),
    ("text_decoration", glslpack::TEXT_DECORATION),
    ("svg", glslpack::SVG),
    ("frame_composite", glslpack::FRAME_COMPOSITE),
    ("material", glslpack::MATERIAL),
    ("morph", glslpack::MORPH),
];

fn sources() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../transpiler/shaders-src")
}

/// The slots a resource occupies, without the GLSL names (which differ between
/// the hand-written packs and the generated text).
fn texture_slots(resources: &[GlslResource]) -> BTreeSet<(u32, u32)> {
    resources
        .iter()
        .filter_map(|resource| match resource {
            GlslResource::Texture { group, binding, .. } => Some((*group, *binding)),
            GlslResource::Sampler {
                texture_group,
                texture_binding,
                ..
            } => Some((*texture_group, *texture_binding)),
            GlslResource::UniformBlock { .. } => None,
        })
        .collect()
}

fn block_slots(resources: &[GlslResource]) -> BTreeSet<(u32, u32)> {
    resources
        .iter()
        .filter_map(|resource| match resource {
            GlslResource::UniformBlock { group, binding, .. } => Some((*group, *binding)),
            _ => None,
        })
        .collect()
}

fn sampler_slots(resources: &[GlslResource]) -> BTreeSet<(u32, u32, u32, u32)> {
    resources
        .iter()
        .filter_map(|resource| match resource {
            GlslResource::Sampler {
                texture_group,
                texture_binding,
                sampler_group,
                sampler_binding,
                ..
            } => Some((
                *texture_group,
                *texture_binding,
                *sampler_group,
                *sampler_binding,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn transpiled_glsl_covers_every_binding_of_the_legacy_packs() {
    let loader = FsLoader {
        root: sources().join("lib"),
    };
    for (name, legacy_bytes) in SHADERS {
        let source = std::fs::read_to_string(sources().join(format!("{name}.aimer"))).unwrap();
        let legacy_text = std::str::from_utf8(legacy_bytes).unwrap();
        let legacy = glsl_package::parse(legacy_text).unwrap();

        for (dialect, output) in [
            ("glsl", aimer_transpiler::compile_glsl(&source, name, &loader)),
            (
                "glsl-es",
                aimer_transpiler::compile_glsl_es(&source, name, &loader),
            ),
        ] {
            let output = output.unwrap_or_else(|error| panic!("{name} ({dialect}): {error}"));
            let blob = glsl_package::assemble(
                output.vertex.as_deref(),
                output.fragment.as_deref(),
                &output.layout,
            );
            let package = glsl_package::parse(&blob)
                .unwrap_or_else(|error| panic!("{name} ({dialect}): {error}"));
            let label = format!("{name} ({dialect})");

            assert!(!package.vertex.is_empty(), "{label}: vertex stage");
            assert!(!package.fragment.is_empty(), "{label}: fragment stage");

            assert!(
                block_slots(&legacy.resources).is_subset(&block_slots(&package.resources)),
                "{label}: uniform blocks"
            );
            assert!(
                sampler_slots(&legacy.resources).is_subset(&sampler_slots(&package.resources)),
                "{label}: samplers"
            );
            assert!(
                texture_slots(&legacy.resources).is_subset(&texture_slots(&package.resources)),
                "{label}: textures"
            );

            // Every name the runtime will look up must be declared in the text.
            for resource in &package.resources {
                let (name, in_text) = match resource {
                    GlslResource::UniformBlock { name, .. }
                    | GlslResource::Sampler { name, .. }
                    | GlslResource::Texture { name, .. } => (
                        name,
                        package.vertex.contains(name.as_str())
                            || package.fragment.contains(name.as_str()),
                    ),
                };
                assert!(in_text, "{label}: `{name}` is not declared in the GLSL");
            }
        }
    }
}
