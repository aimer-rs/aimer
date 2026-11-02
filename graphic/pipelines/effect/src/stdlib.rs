//! The `aimer::` modules an effect shader may import.

use aimer_transpiler::error::{Error, Result};
use aimer_transpiler::resolve::ModuleLoader;

/// Serves the shader standard library compiled into the framework.
///
/// These are the same modules the built-in shaders import, so an effect can
/// clip, shape and color exactly as the framework's own pipelines do.
pub(crate) struct StdLib;

impl ModuleLoader for StdLib {
    fn load(&self, module: &[String]) -> Result<String> {
        let name = match module {
            [root, name] if root == "aimer" => name.as_str(),
            _ => "",
        };
        match name {
            "color" => Ok(include_str!("../../../transpiler/shaders-src/lib/color.aimer").to_owned()),
            "sdf" => Ok(include_str!("../../../transpiler/shaders-src/lib/sdf.aimer").to_owned()),
            "clip" => Ok(include_str!("../../../transpiler/shaders-src/lib/clip.aimer").to_owned()),
            _ => Err(Error::new(format!(
                "no shader module `{}`; available: aimer::color, aimer::sdf, aimer::clip",
                module.join("::")
            ))),
        }
    }
}
