//! Texture-unit assignment for the fused samplers of a GLSL program.
//!
//! GLSL has no separate texture and sampler objects, so every `sampler2D`
//! uniform reads one texture through (at most) one sampler and owns a texture
//! unit. One texture can therefore need several units: transpiler output fuses
//! `texture + sampler` and `texture + no sampler` into two uniforms, and one
//! sampler is shared by every texture sampled through it. The backend binds a
//! bound texture or sampler to *each* unit that names it.

use std::collections::BTreeSet;

use super::GlslResource;

/// One texture unit and what the backend binds to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextureUnit<L> {
    /// The unit index, assigned from zero in a deterministic order.
    pub unit: u32,
    /// The `sampler2D` uniform's name.
    pub name: String,
    /// What the backend returned for this uniform, such as its location.
    pub location: L,
    /// The `(group, binding)` of the texture this unit reads.
    pub texture: (u32, u32),
    /// The `(group, binding)` of the sampler object, if the uniform has one.
    pub sampler: Option<(u32, u32)>,
}

/// Assigns a texture unit to every active sampler uniform in `resources`.
///
/// `locate` looks a uniform up in the linked program and returns `None` for
/// one the compiler removed because the shader never reads it; such a uniform
/// takes no unit. Samplers come first, then texture-only uniforms, each in
/// slot order, so the assignment does not depend on the order of `resources`.
///
/// # Errors
///
/// Returns a message when the program needs more than `max_units` units.
pub fn assign_texture_units<L: Copy>(
    resources: &[GlslResource],
    mut locate: impl FnMut(&str) -> Option<L>,
    max_units: u32,
) -> Result<Vec<TextureUnit<L>>, String> {
    // Sets, not the input order, decide the unit numbers: a program must get the
    // same assignment however its resource list happened to be written, and
    // listing a uniform twice must not cost a second unit.
    let mut samplers = BTreeSet::new();
    let mut textures = BTreeSet::new();
    for resource in resources {
        match resource {
            GlslResource::Sampler {
                texture_group,
                texture_binding,
                sampler_group,
                sampler_binding,
                name,
            } => {
                samplers.insert((
                    *texture_group,
                    *texture_binding,
                    *sampler_group,
                    *sampler_binding,
                    name.as_str(),
                ));
            }
            GlslResource::Texture {
                group,
                binding,
                name,
            } => {
                textures.insert((*group, *binding, name.as_str()));
            }
            GlslResource::UniformBlock { .. } => {}
        }
    }

    let mut units: Vec<TextureUnit<L>> = Vec::new();
    let next_unit = |units: &Vec<TextureUnit<L>>| -> Result<u32, String> {
        let unit = units.len() as u32;
        if unit >= max_units {
            Err(format!(
                "texture count exceeds the texture-unit limit of {max_units}"
            ))
        } else {
            Ok(unit)
        }
    };
    for (texture_group, texture_binding, sampler_group, sampler_binding, name) in samplers {
        let Some(location) = locate(name) else {
            continue;
        };
        units.push(TextureUnit {
            unit: next_unit(&units)?,
            name: name.to_owned(),
            location,
            texture: (texture_group, texture_binding),
            sampler: Some((sampler_group, sampler_binding)),
        });
    }
    for (group, binding, name) in textures {
        // A pack may list the same uniform as both a sampler and a texture.
        if units.iter().any(|unit| unit.name == name) {
            continue;
        }
        let Some(location) = locate(name) else {
            continue;
        };
        units.push(TextureUnit {
            unit: next_unit(&units)?,
            name: name.to_owned(),
            location,
            texture: (group, binding),
            sampler: None,
        });
    }
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sampler(texture: (u32, u32), sampler: (u32, u32), name: &str) -> GlslResource {
        GlslResource::Sampler {
            texture_group: texture.0,
            texture_binding: texture.1,
            sampler_group: sampler.0,
            sampler_binding: sampler.1,
            name: name.to_owned(),
        }
    }

    fn texture(slot: (u32, u32), name: &str) -> GlslResource {
        GlslResource::Texture {
            group: slot.0,
            binding: slot.1,
            name: name.to_owned(),
        }
    }

    fn all_active(name: &str) -> Option<i32> {
        Some(name.len() as i32)
    }

    #[test]
    fn a_sampler_takes_the_first_unit() {
        let units =
            assign_texture_units(&[sampler((1, 0), (1, 1), "t")], all_active, 8).unwrap();
        assert_eq!(
            units,
            vec![TextureUnit {
                unit: 0,
                name: "t".to_owned(),
                location: 1,
                texture: (1, 0),
                sampler: Some((1, 1)),
            }]
        );
    }

    #[test]
    fn one_texture_gets_a_unit_per_fused_uniform() {
        // The shape the transpiler emits for `material`'s backdrop: the same
        // texture read with a sampler and, separately, without one.
        let units = assign_texture_units(
            &[
                texture((0, 1), "_smp_backdrop"),
                sampler((0, 1), (0, 2), "_smp_backdrop_backdrop_sampler"),
            ],
            all_active,
            8,
        )
        .unwrap();

        let by_name: Vec<_> = units
            .iter()
            .map(|unit| (unit.unit, unit.name.as_str(), unit.texture, unit.sampler))
            .collect();
        assert_eq!(
            by_name,
            vec![
                (0, "_smp_backdrop_backdrop_sampler", (0, 1), Some((0, 2))),
                (1, "_smp_backdrop", (0, 1), None),
            ]
        );
    }

    #[test]
    fn a_shared_sampler_appears_on_every_unit_that_uses_it() {
        let units = assign_texture_units(
            &[
                sampler((0, 3), (0, 2), "b"),
                sampler((0, 1), (0, 2), "a"),
            ],
            all_active,
            8,
        )
        .unwrap();
        assert_eq!(units.iter().filter(|unit| unit.sampler == Some((0, 2))).count(), 2);
        assert_eq!(units[0].texture, (0, 1), "units follow slot order");
    }

    #[test]
    fn a_removed_uniform_takes_no_unit() {
        let units = assign_texture_units(
            &[sampler((0, 0), (0, 1), "dead"), sampler((0, 2), (0, 1), "live")],
            |name| (name == "live").then_some(7),
            8,
        )
        .unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!((units[0].unit, units[0].location), (0, 7));
    }

    #[test]
    fn a_texture_row_naming_a_sampler_uniform_is_not_assigned_twice() {
        let units = assign_texture_units(
            &[sampler((0, 0), (0, 1), "same"), texture((0, 0), "same")],
            all_active,
            8,
        )
        .unwrap();
        assert_eq!(units.len(), 1);
        assert!(units[0].sampler.is_some());
    }

    #[test]
    fn order_of_resources_does_not_change_the_assignment() {
        let a = [sampler((0, 3), (0, 2), "b"), sampler((0, 1), (0, 2), "a"), texture((2, 0), "c")];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(
            assign_texture_units(&a, all_active, 8).unwrap(),
            assign_texture_units(&b, all_active, 8).unwrap()
        );
    }

    #[test]
    fn running_out_of_units_is_an_error() {
        let resources = [sampler((0, 0), (0, 1), "a"), texture((0, 2), "b")];
        assert!(assign_texture_units(&resources, all_active, 2).is_ok());
        let error = assign_texture_units(&resources, all_active, 1).unwrap_err();
        assert!(error.contains("texture-unit limit"), "{error}");
        assert!(assign_texture_units(&resources, all_active, 0).is_err());
    }

    #[test]
    fn uniform_blocks_take_no_unit() {
        let block = GlslResource::UniformBlock {
            group: 0,
            binding: 0,
            name: "b".to_owned(),
        };
        assert!(assign_texture_units(&[block], all_active, 8).unwrap().is_empty());
    }
}
