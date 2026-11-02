//! The set of effects an application has registered.
//!
//! Registration is process-wide and backend-neutral: the application names its
//! effects once, typically before the window opens, and every renderer (one per
//! window, and a fresh one after a device is lost) installs a pipeline for each
//! on its own backend. Renderers poll [`generation`], one atomic load a frame,
//! to notice an effect added later.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::{EffectProgram, ShaderEffect, analyze};

/// A registered effect: its name and its checked shader.
pub struct EffectSpec {
    name: &'static str,
    source: &'static str,
    program: EffectProgram,
}

impl EffectSpec {
    /// The pipeline name draw commands address.
    #[inline]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The checked shader.
    #[inline]
    pub fn program(&self) -> &EffectProgram {
        &self.program
    }
}

static EFFECTS: Mutex<Vec<Arc<EffectSpec>>> = Mutex::new(Vec::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Registers effect `E` with every renderer, present and future.
///
/// Registering the same effect twice is allowed and does nothing the second
/// time.
///
/// # Errors
///
/// Returns a message when the shader breaks the [`ShaderEffect`] contract, or
/// when a different effect already uses [`E::NAME`](ShaderEffect::NAME).
pub fn register<E: ShaderEffect>() -> Result<(), String> {
    let mut effects = EFFECTS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = effects.iter().find(|spec| spec.name == E::NAME) {
        return if existing.source == E::SOURCE && existing.program.samples_backdrop() == E::SAMPLES_BACKDROP {
            Ok(())
        } else {
            Err(format!("an effect named `{}` is already registered with a different shader", E::NAME))
        };
    }
    let program = analyze(E::SOURCE, E::SAMPLES_BACKDROP).map_err(|error| format!("effect `{}`: {error}", E::NAME))?;
    effects.push(Arc::new(EffectSpec {
        name: E::NAME,
        source: E::SOURCE,
        program,
    }));
    GENERATION.fetch_add(1, Ordering::Release);
    Ok(())
}

/// A counter that changes whenever an effect is registered.
#[inline]
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Whether `name` is a registered effect whose shader samples the backdrop.
///
/// The renderer asks this per custom draw command when it decides whether a
/// frame needs a snapshot to read from, so it does not allocate.
pub fn samples_backdrop(name: &str) -> bool {
    EFFECTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|spec| spec.name == name && spec.program.samples_backdrop())
}

/// Every registered effect, in registration order.
pub fn registered() -> Vec<Arc<EffectSpec>> {
    EFFECTS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParamWriter;

    const WATER: &str = include_str!("../tests/shaders/water.aimer");

    // The registry is process-wide and tests share a process under
    // `cargo test`, so every test uses names no other test does.
    macro_rules! effect {
        ($ty:ident, $name:literal, $source:expr, $backdrop:literal) => {
            struct $ty;
            impl ShaderEffect for $ty {
                const NAME: &'static str = $name;
                const SOURCE: &'static str = $source;
                const SAMPLES_BACKDROP: bool = $backdrop;
                fn write_params(&self, params: &mut ParamWriter) {
                    params.f32(0.0).f32(0.0).f32(0.0).pad(4);
                }
            }
        };
    }

    fn named(name: &str) -> Option<Arc<EffectSpec>> {
        registered().into_iter().find(|spec| spec.name() == name)
    }

    #[test]
    fn a_valid_effect_is_registered_and_bumps_the_generation() {
        effect!(Fresh, "registry-fresh", WATER, true);
        let before = generation();
        register::<Fresh>().unwrap();
        assert!(generation() > before);
        let spec = named("registry-fresh").expect("registered");
        assert_eq!(spec.program().params_size(), 16);
    }

    #[test]
    fn only_registered_backdrop_effects_report_sampling_the_backdrop() {
        effect!(Samples, "registry-samples", WATER, true);
        effect!(Opaque, "registry-opaque", include_str!("../tests/shaders/solid.aimer"), false);
        register::<Samples>().unwrap();
        register::<Opaque>().unwrap();
        assert!(samples_backdrop("registry-samples"));
        assert!(!samples_backdrop("registry-opaque"));
        assert!(!samples_backdrop("registry-never-registered"));
        assert!(!samples_backdrop(""));
    }

    #[test]
    fn registering_the_same_effect_twice_changes_nothing() {
        effect!(Twice, "registry-twice", WATER, true);
        register::<Twice>().unwrap();
        let after_first = generation();
        let count = registered().len();
        register::<Twice>().unwrap();
        assert_eq!(generation(), after_first);
        assert_eq!(registered().len(), count);
    }

    #[test]
    fn a_name_cannot_be_reused_for_a_different_shader() {
        effect!(First, "registry-clash", WATER, true);
        effect!(Second, "registry-clash", "shader other;", false);
        register::<First>().unwrap();
        let error = register::<Second>().unwrap_err();
        assert!(error.contains("registry-clash") && error.contains("different shader"), "{error}");
    }

    #[test]
    fn a_shader_that_breaks_the_contract_is_not_registered() {
        effect!(Bad, "registry-bad", "shader bad;", false);
        let before = generation();
        let error = register::<Bad>().unwrap_err();
        assert!(error.contains("registry-bad"), "{error}");
        assert!(named("registry-bad").is_none());
        assert_eq!(generation(), before);
    }
}
