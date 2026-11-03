//! An effect shader in the language a backend consumes.

use aimer_cupid_gpu::ShaderLanguage;
use aimer_transpiler::Target;

use crate::EffectProgram;

impl EffectProgram {
    /// The shader as the blob [`GpuBackend::create_shader_module`] takes for
    /// `language`.
    ///
    /// WGSL, MSL and HLSL are the text itself; the GLSL languages are a
    /// [`glsl_package`](aimer_cupid_gpu::glsl_package) of the generated
    /// vertex and fragment text and their binding layout.
    ///
    /// [`GpuBackend::create_shader_module`]: aimer_cupid_gpu::GpuBackend::create_shader_module
    ///
    /// # Errors
    ///
    /// Returns the transpiler's message when the shader cannot be expressed in
    /// `language`.
    pub fn module_source(&self, language: ShaderLanguage) -> Result<Vec<u8>, String> {
        let (target, ext) = match language {
            ShaderLanguage::Wgsl => (Target::Wgsl, "wgsl"),
            ShaderLanguage::Msl => (Target::Msl, "metal"),
            ShaderLanguage::Hlsl => (Target::Hlsl, "hlsl"),
            ShaderLanguage::Glsl => (Target::Glsl, ""),
            ShaderLanguage::GlslEs => (Target::GlslEs, ""),
        };
        let output = aimer_transpiler::emit(self.program(), target).map_err(|error| error.to_string())?;
        let file = |ext: &str| output.get(ext).map(str::to_owned);
        if ext.is_empty() {
            let layout = file("layout.json").ok_or("the transpiler wrote no GLSL layout")?;
            return Ok(
                aimer_cupid_gpu::glsl_package::assemble(
                    file("vert").as_deref(),
                    file("frag").as_deref(),
                    &layout,
                )
                .into_bytes(),
            );
        }
        file(ext)
            .map(String::into_bytes)
            .ok_or_else(|| format!("the transpiler wrote no `{ext}` file"))
    }
}

#[cfg(test)]
mod tests {
    use aimer_cupid_gpu::glsl_package::{self, GlslResource};

    use super::*;
    use crate::analyze;

    const WATER: &str = include_str!("../tests/shaders/water.aimer");

    fn water() -> EffectProgram {
        analyze(WATER, true).unwrap()
    }

    fn text(language: ShaderLanguage) -> String {
        String::from_utf8(water().module_source(language).unwrap()).unwrap()
    }

    #[test]
    fn wgsl_declares_both_entry_points() {
        let wgsl = text(ShaderLanguage::Wgsl);
        assert!(wgsl.contains("fn vs_main") && wgsl.contains("fn fs_main"));
    }

    // The Vulkan compiler is built only for Linux and Android.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn wgsl_compiles_for_vulkan() {
        let wgsl = text(ShaderLanguage::Wgsl);
        // The Vulkan backend turns this very text into SPIR-V at run time.
        let spirv = aimer_vulkan::compile_wgsl_to_spirv(&wgsl).unwrap();
        assert_eq!(&spirv[..4], 0x0723_0203_u32.to_le_bytes());
    }

    #[test]
    fn msl_and_hlsl_declare_both_entry_points() {
        let msl = text(ShaderLanguage::Msl);
        assert!(msl.contains("vertex VertexOutput vs_main") && msl.contains("fragment float4 fs_main"));
        let hlsl = text(ShaderLanguage::Hlsl);
        assert!(hlsl.contains("vs_main") && hlsl.contains("fs_main"));
    }

    #[test]
    fn glsl_languages_come_as_a_package_that_binds_the_frame_and_backdrop() {
        for (language, version) in [
            (ShaderLanguage::Glsl, "#version 330 core"),
            (ShaderLanguage::GlslEs, "#version 300 es"),
        ] {
            let blob = water().module_source(language).unwrap();
            let blob = String::from_utf8(blob).unwrap();
            let package = glsl_package::parse(&blob).unwrap();
            assert!(package.vertex.starts_with(version), "{language:?}");
            assert!(package.fragment.starts_with(version), "{language:?}");

            let frame_blocks = package
                .resources
                .iter()
                .filter(|resource| matches!(resource, GlslResource::UniformBlock { group: 0, binding: 0, .. }))
                .count();
            assert_eq!(frame_blocks, 2, "the frame block is declared in both stages: {language:?}");
            assert!(
                package.resources.iter().any(|resource| matches!(
                    resource,
                    GlslResource::Sampler {
                        texture_group: 0,
                        texture_binding: 1,
                        sampler_group: 0,
                        sampler_binding: 2,
                        ..
                    }
                )),
                "{language:?}: {:?}",
                package.resources
            );
        }
    }

    #[test]
    fn each_language_is_distinct() {
        let mut seen = std::collections::HashSet::new();
        for language in [
            ShaderLanguage::Wgsl,
            ShaderLanguage::Msl,
            ShaderLanguage::Hlsl,
            ShaderLanguage::Glsl,
            ShaderLanguage::GlslEs,
        ] {
            assert!(seen.insert(text(language)), "{language:?} repeats another language");
        }
    }
}
