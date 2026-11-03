#[cfg(any(target_os = "linux", target_os = "android"))]
use std::path::PathBuf;

// Regenerate checked-in Vulkan shader modules with:
// `cargo run -p aimer_vulkan --example vulkan_shadergen`
// Pass a shader name (for example `-- material`) to regenerate only that module.

#[cfg(any(target_os = "linux", target_os = "android"))]
const SHADERS: &[(&str, &str)] = &[
    (
        "rect",
        include_str!("../../../cupid_gpu/shaders/wgsl/rect.wgsl"),
    ),
    (
        "image",
        concat!(
            include_str!("../../../cupid_gpu/shaders/wgsl/color.wgsl"),
            include_str!("../../../cupid_gpu/shaders/wgsl/image.wgsl")
        ),
    ),
    (
        "image.android",
        concat!(
            include_str!("../../../cupid_gpu/shaders/wgsl/android_color.wgsl"),
            include_str!("../../../cupid_gpu/shaders/wgsl/image.wgsl")
        ),
    ),
    (
        "text",
        include_str!("../../../cupid_gpu/shaders/wgsl/text.wgsl"),
    ),
    (
        "text_color",
        include_str!("../../../cupid_gpu/shaders/wgsl/text_color.wgsl"),
    ),
    (
        "text_decoration",
        include_str!("../../../cupid_gpu/shaders/wgsl/text_decoration.wgsl"),
    ),
    (
        "svg",
        concat!(
            include_str!("../../../cupid_gpu/shaders/wgsl/color.wgsl"),
            include_str!("../../../cupid_gpu/shaders/wgsl/svg.wgsl")
        ),
    ),
    (
        "svg.android",
        concat!(
            include_str!("../../../cupid_gpu/shaders/wgsl/android_color.wgsl"),
            include_str!("../../../cupid_gpu/shaders/wgsl/svg.wgsl")
        ),
    ),
    (
        "frame_composite",
        include_str!("../../../cupid_gpu/shaders/wgsl/frame_composite.wgsl"),
    ),
    (
        "material",
        include_str!("../../../cupid_gpu/shaders/wgsl/material.wgsl"),
    ),
];

#[cfg(any(target_os = "linux", target_os = "android"))]
fn main() {
    let selected = std::env::args().nth(1);
    if let Some(name) = selected.as_deref() {
        assert!(
            SHADERS.iter().any(|(shader, _)| *shader == name),
            "unknown shader: {name}"
        );
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders/vulkan");
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

#[cfg(any(target_os = "linux", target_os = "android"))]
fn compile_spirv(source: &str) -> Result<Vec<u8>, String> {
    aimer_vulkan::compile_wgsl_to_spirv(source)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
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
            "rect" => include_bytes!("../shaders/vulkan/rect.spv"),
            "image" => include_bytes!("../shaders/vulkan/image.spv"),
            "image.android" => include_bytes!("../shaders/vulkan/image.android.spv"),
            "text" => include_bytes!("../shaders/vulkan/text.spv"),
            "text_color" => include_bytes!("../shaders/vulkan/text_color.spv"),
            "text_decoration" => {
                include_bytes!("../shaders/vulkan/text_decoration.spv")
            }
            "svg" => include_bytes!("../shaders/vulkan/svg.spv"),
            "svg.android" => include_bytes!("../shaders/vulkan/svg.android.spv"),
            "frame_composite" => {
                include_bytes!("../shaders/vulkan/frame_composite.spv")
            }
            "material" => include_bytes!("../shaders/vulkan/material.spv"),
            _ => panic!("unlisted Vulkan shader {name}"),
        }
    }
}

// The WGSL to SPIR-V compiler is built only for Linux and Android.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn main() {
    eprintln!("vulkan_shadergen runs only on Linux and Android hosts");
}
