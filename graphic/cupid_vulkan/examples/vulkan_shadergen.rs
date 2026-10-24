use std::path::PathBuf;

// Regenerate checked-in Vulkan shader modules with:
// `cargo run -p aimer_cupid --example vulkan_shadergen`
// Pass a shader name (for example `-- material`) to regenerate only that module.

const SHADERS: &[(&str, &str)] = &[
    (
        "rect",
        include_str!("../src/pipeline/shaders/wgsl/rect.wgsl"),
    ),
    (
        "image",
        concat!(
            include_str!("../src/pipeline/shaders/wgsl/color.wgsl"),
            include_str!("../src/pipeline/shaders/wgsl/image.wgsl")
        ),
    ),
    (
        "image.android",
        concat!(
            include_str!("../src/pipeline/shaders/wgsl/android_color.wgsl"),
            include_str!("../src/pipeline/shaders/wgsl/image.wgsl")
        ),
    ),
    (
        "text",
        include_str!("../src/pipeline/shaders/wgsl/text.wgsl"),
    ),
    (
        "text_color",
        include_str!("../src/pipeline/shaders/wgsl/text_color.wgsl"),
    ),
    (
        "text_decoration",
        include_str!("../src/pipeline/shaders/wgsl/text_decoration.wgsl"),
    ),
    (
        "svg",
        concat!(
            include_str!("../src/pipeline/shaders/wgsl/color.wgsl"),
            include_str!("../src/pipeline/shaders/wgsl/svg.wgsl")
        ),
    ),
    (
        "svg.android",
        concat!(
            include_str!("../src/pipeline/shaders/wgsl/android_color.wgsl"),
            include_str!("../src/pipeline/shaders/wgsl/svg.wgsl")
        ),
    ),
    (
        "frame_composite",
        include_str!("../src/pipeline/shaders/wgsl/frame_composite.wgsl"),
    ),
    (
        "material",
        include_str!("../src/pipeline/shaders/wgsl/material.wgsl"),
    ),
];

fn main() {
    let selected = std::env::args().nth(1);
    if let Some(name) = selected.as_deref() {
        assert!(
            SHADERS.iter().any(|(shader, _)| *shader == name),
            "unknown shader: {name}"
        );
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/pipeline/shaders/vulkan");
    std::fs::create_dir_all(&output).expect("create Vulkan shader output directory");
    for (name, source) in SHADERS {
        if selected
            .as_deref()
            .is_some_and(|selected| selected != *name)
        {
            continue;
        }
        let bytes =
            compile_spirv(source).unwrap_or_else(|error| panic!("compile {name}.wgsl: {error}"));
        std::fs::write(output.join(format!("{name}.spv")), bytes)
            .unwrap_or_else(|error| panic!("write {name}.spv: {error}"));
    }
}

fn compile_spirv(source: &str) -> Result<Vec<u8>, String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|error| error.to_string())?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| error.to_string())?;
    let options = naga::back::spv::Options {
        flags: naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE
            | naga::back::spv::WriterFlags::LABEL_VARYINGS
            | naga::back::spv::WriterFlags::CLAMP_FRAG_DEPTH,
        ..Default::default()
    };
    let words = naga::back::spv::write_vec(&module, &info, &options, None)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{SHADERS, compile_spirv};

    #[test]
    fn checked_in_spirv_matches_wgsl_sources() {
        for (name, source) in SHADERS {
            let expected = checked_in_shader(name);
            let actual = compile_spirv(source).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                actual.as_slice(),
                expected,
                "{name}.spv is stale; regenerate the checked-in shaders"
            );
        }
    }

    fn checked_in_shader(name: &str) -> &'static [u8] {
        match name {
            "rect" => include_bytes!("../src/pipeline/shaders/vulkan/rect.spv"),
            "image" => include_bytes!("../src/pipeline/shaders/vulkan/image.spv"),
            "image.android" => include_bytes!("../src/pipeline/shaders/vulkan/image.android.spv"),
            "text" => include_bytes!("../src/pipeline/shaders/vulkan/text.spv"),
            "text_color" => include_bytes!("../src/pipeline/shaders/vulkan/text_color.spv"),
            "text_decoration" => {
                include_bytes!("../src/pipeline/shaders/vulkan/text_decoration.spv")
            }
            "svg" => include_bytes!("../src/pipeline/shaders/vulkan/svg.spv"),
            "svg.android" => include_bytes!("../src/pipeline/shaders/vulkan/svg.android.spv"),
            "frame_composite" => {
                include_bytes!("../src/pipeline/shaders/vulkan/frame_composite.spv")
            }
            "material" => include_bytes!("../src/pipeline/shaders/vulkan/material.spv"),
            _ => panic!("unlisted Vulkan shader {name}"),
        }
    }
}
