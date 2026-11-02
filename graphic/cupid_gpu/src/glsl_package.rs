//! The GLSL shader blob the OpenGL-family backends compile.
//!
//! [`GpuBackend::create_shader_module`](crate::GpuBackend::create_shader_module)
//! takes one opaque byte blob, but a GLSL program is a vertex text, a fragment
//! text and the bindings the runtime has to set (uniform-block binding points
//! and sampler units, which GLSL 3.30 and ES 3.00 cannot declare in the
//! source). This module reads two containers for that blob:
//!
//! * the **layout package** ([`LAYOUT_HEADER`]) wraps the three files
//!   `aimer_transpiler` writes for its `glsl` and `glsl-es` targets (`.vert`,
//!   `.frag`, `.layout.json`) without changing a byte of them. [`assemble`]
//!   builds it.
//! * the **legacy pack** ([`LEGACY_HEADER`]) is the hand-maintained
//!   `.glslpack` format with a line-based `[RESOURCES]` section.
//!
//! Both come out as a [`GlslPackage`], so a backend links programs the same way
//! whichever container produced them.

mod json;
mod texture_units;

pub use texture_units::{TextureUnit, assign_texture_units};

/// First line of a layout package.
pub const LAYOUT_HEADER: &str = "AIMER-GLSL-LAYOUT-1";

/// First line of a legacy `.glslpack`.
pub const LEGACY_HEADER: &str = "AIMER-OPENGL-SHADER-PACK-1";

/// One binding the runtime sets when it links a program.
///
/// `name` is the identifier in the GLSL text: a uniform block's name or a
/// sampler uniform's name. Groups and bindings are the WebGPU-style slots of the
/// bind group layout the pipeline was created with.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GlslResource {
    /// A std140 uniform block, bound to a buffer at `(group, binding)`.
    UniformBlock {
        group: u32,
        binding: u32,
        name: String,
    },
    /// A fused `sampler2D` that reads the texture at `(texture_group,
    /// texture_binding)` through the sampler at `(sampler_group,
    /// sampler_binding)`.
    Sampler {
        texture_group: u32,
        texture_binding: u32,
        sampler_group: u32,
        sampler_binding: u32,
        name: String,
    },
    /// A `sampler2D` that reads the texture at `(group, binding)` without a
    /// sampler object, such as one fetched by texel coordinate.
    Texture {
        group: u32,
        binding: u32,
        name: String,
    },
}

/// A parsed shader blob: the two stage texts and the bindings to set.
///
/// A stage the program does not have is the empty string.
#[derive(Debug, Eq, PartialEq)]
pub struct GlslPackage<'a> {
    pub vertex: &'a str,
    pub fragment: &'a str,
    pub resources: Vec<GlslResource>,
}

/// Wraps transpiler output as a layout package.
///
/// `vertex` and `fragment` are the `.vert` and `.frag` texts (either may be
/// absent for a one-stage program) and `layout` is the `.layout.json` text.
pub fn assemble(vertex: Option<&str>, fragment: Option<&str>, layout: &str) -> String {
    let mut blob = String::with_capacity(
        LAYOUT_HEADER.len()
            + vertex.map_or(0, str::len)
            + fragment.map_or(0, str::len)
            + layout.len()
            + 48,
    );
    blob.push_str(LAYOUT_HEADER);
    blob.push('\n');
    for (marker, text) in [
        (VERTEX_MARKER, vertex),
        (FRAGMENT_MARKER, fragment),
        (LAYOUT_MARKER, Some(layout)),
    ] {
        let Some(text) = text else { continue };
        blob.push_str(marker);
        blob.push('\n');
        blob.push_str(text);
        // A section ends at the next marker line, so the text must end its line.
        if !text.ends_with('\n') {
            blob.push('\n');
        }
    }
    blob
}

/// Reads a shader blob in either container.
///
/// # Errors
///
/// Returns a message when the header is unknown, a required section is missing
/// or the resource metadata is malformed.
pub fn parse(source: &str) -> Result<GlslPackage<'_>, String> {
    if let Some(body) = source.strip_prefix(LAYOUT_HEADER).and_then(|rest| rest.strip_prefix('\n')) {
        parse_layout_package(body)
    } else if let Some(body) = source.strip_prefix(LEGACY_HEADER).and_then(|rest| rest.strip_prefix('\n')) {
        parse_legacy_pack(body)
    } else {
        Err("unknown GLSL shader package header".to_owned())
    }
}

const VERTEX_MARKER: &str = "[VERTEX]";
const FRAGMENT_MARKER: &str = "[FRAGMENT]";
const LAYOUT_MARKER: &str = "[LAYOUT]";

fn parse_layout_package(body: &str) -> Result<GlslPackage<'_>, String> {
    let (mut vertex, mut fragment, mut layout) = (None, None, None);
    let mut open: Option<(&str, usize)> = None;
    let mut offset = 0;
    // Sections are the text between marker lines; a marker only counts when it
    // is the whole line, which no GLSL or JSON line ever is.
    for line in body.split_inclusive('\n') {
        let marker = line.trim_end_matches(['\n', '\r']);
        if matches!(marker, VERTEX_MARKER | FRAGMENT_MARKER | LAYOUT_MARKER) {
            if let Some((name, start)) = open.take() {
                store_section(name, &body[start..offset], &mut vertex, &mut fragment, &mut layout)?;
            }
            open = Some((marker, offset + line.len()));
        } else if open.is_none() && !line.trim().is_empty() {
            return Err("text before the first section of the GLSL layout package".to_owned());
        }
        offset += line.len();
    }
    if let Some((name, start)) = open {
        store_section(name, &body[start..], &mut vertex, &mut fragment, &mut layout)?;
    }

    let layout = layout.ok_or("GLSL layout package has no [LAYOUT] section")?;
    let vertex = vertex.unwrap_or("");
    let fragment = fragment.unwrap_or("");
    if vertex.is_empty() && fragment.is_empty() {
        return Err("GLSL layout package has no shader stage".to_owned());
    }
    let layout = json::parse(layout).map_err(|error| format!("invalid layout JSON: {error}"))?;
    Ok(GlslPackage {
        vertex,
        fragment,
        resources: layout_resources(&layout)?,
    })
}

fn store_section<'a>(
    name: &str,
    text: &'a str,
    vertex: &mut Option<&'a str>,
    fragment: &mut Option<&'a str>,
    layout: &mut Option<&'a str>,
) -> Result<(), String> {
    let slot = match name {
        VERTEX_MARKER => vertex,
        FRAGMENT_MARKER => fragment,
        _ => layout,
    };
    if slot.replace(text.trim()).is_some() {
        return Err(format!("{name} appears twice in the GLSL layout package"));
    }
    Ok(())
}

/// Reads the bindings the runtime must set from a transpiler `layout.json`.
fn layout_resources(layout: &json::Json<'_>) -> Result<Vec<GlslResource>, String> {
    if !matches!(layout, json::Json::Object(_)) {
        return Err("layout JSON is not an object".to_owned());
    }
    let mut resources = Vec::new();
    for stage in ["vertex", "fragment"] {
        let Some(stage_layout) = layout.get(stage) else {
            continue;
        };
        for (index, block) in list(stage_layout, stage, "uniform_blocks")?.iter().enumerate() {
            let at = format!("{stage}.uniform_blocks[{index}]");
            resources.push(GlslResource::UniformBlock {
                group: field_u32(block, &at, "group")?,
                binding: field_u32(block, &at, "binding")?,
                name: field_str(block, &at, "block")?,
            });
        }
        for (index, sampler) in list(stage_layout, stage, "samplers")?.iter().enumerate() {
            let at = format!("{stage}.samplers[{index}]");
            let name = field_str(sampler, &at, "uniform")?;
            let texture = field(sampler, &at, "texture")?;
            let (texture_group, texture_binding) = (
                field_u32(texture, &format!("{at}.texture"), "group")?,
                field_u32(texture, &format!("{at}.texture"), "binding")?,
            );
            // `null` is a texture sampled without a sampler object.
            resources.push(match field(sampler, &at, "sampler")? {
                json::Json::Null => GlslResource::Texture {
                    group: texture_group,
                    binding: texture_binding,
                    name,
                },
                smp => GlslResource::Sampler {
                    texture_group,
                    texture_binding,
                    sampler_group: field_u32(smp, &format!("{at}.sampler"), "group")?,
                    sampler_binding: field_u32(smp, &format!("{at}.sampler"), "binding")?,
                    name,
                },
            });
        }
    }
    Ok(resources)
}

/// The array under `key`; a stage that omits it has none of that kind.
fn list<'v, 'a>(
    stage_layout: &'v json::Json<'a>,
    stage: &str,
    key: &str,
) -> Result<&'v [json::Json<'a>], String> {
    match stage_layout.get(key) {
        None => Ok(&[]),
        Some(value) => value
            .as_array()
            .ok_or_else(|| format!("layout `{stage}.{key}` is not an array")),
    }
}

fn field<'v, 'a>(
    object: &'v json::Json<'a>,
    at: &str,
    key: &str,
) -> Result<&'v json::Json<'a>, String> {
    object
        .get(key)
        .ok_or_else(|| format!("layout `{at}` has no `{key}`"))
}

fn field_u32(object: &json::Json<'_>, at: &str, key: &str) -> Result<u32, String> {
    field(object, at, key)?
        .as_u32()
        .ok_or_else(|| format!("layout `{at}.{key}` is not an unsigned 32-bit integer"))
}

fn field_str(object: &json::Json<'_>, at: &str, key: &str) -> Result<String, String> {
    field(object, at, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("layout `{at}.{key}` is not a string"))
}

fn parse_legacy_pack(body: &str) -> Result<GlslPackage<'_>, String> {
    let (vertex, rest) = body
        .split_once("[FRAGMENT]\n")
        .ok_or_else(|| "GLSL shader package has no fragment section".to_owned())?;
    let vertex = vertex
        .strip_prefix("[VERTEX]\n")
        .ok_or_else(|| "GLSL shader package has no vertex section".to_owned())?;
    let (fragment, resource_text) = rest
        .split_once("[RESOURCES]\n")
        .ok_or_else(|| "GLSL shader package has no resource section".to_owned())?;
    let mut resources = Vec::new();
    for line in resource_text.lines().filter(|line| !line.is_empty()) {
        let parts: Vec<_> = line.split_whitespace().collect();
        let slot = |text: &str, what: &str| {
            text.parse::<u32>()
                .map_err(|_| format!("invalid {what} in `{line}`"))
        };
        let resource = match parts.as_slice() {
            ["UBO", group, binding, name] => GlslResource::UniformBlock {
                group: slot(group, "UBO group")?,
                binding: slot(binding, "UBO binding")?,
                name: (*name).to_owned(),
            },
            ["TEXTURE", group, binding, name] => GlslResource::Texture {
                group: slot(group, "texture group")?,
                binding: slot(binding, "texture binding")?,
                name: (*name).to_owned(),
            },
            [
                "SAMPLER",
                texture_group,
                texture_binding,
                sampler_group,
                sampler_binding,
                name,
            ] => GlslResource::Sampler {
                texture_group: slot(texture_group, "texture group")?,
                texture_binding: slot(texture_binding, "texture binding")?,
                sampler_group: slot(sampler_group, "sampler group")?,
                sampler_binding: slot(sampler_binding, "sampler binding")?,
                name: (*name).to_owned(),
            },
            _ => return Err(format!("invalid GLSL resource metadata `{line}`")),
        };
        resources.push(resource);
    }
    Ok(GlslPackage {
        vertex: vertex.trim(),
        fragment: fragment.trim(),
        resources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = r#"{
  "shader": "image",
  "vertex": {
    "uniform_blocks": [
      { "block": "viewport_block_vertex", "resource": "viewport", "group": 0, "binding": 0 }
    ],
    "samplers": [],
    "attributes": [
      { "location": 0, "name": "_p2vs_location0", "type": "vec2", "step": "instance" }
    ]
  },
  "fragment": {
    "uniform_blocks": [
      { "block": "viewport_block_fragment", "resource": "viewport", "group": 0, "binding": 0 }
    ],
    "samplers": [
      { "uniform": "_smp_t_diffuse_s_diffuse", "texture": { "name": "t_diffuse", "group": 1, "binding": 0 }, "sampler": { "name": "s_diffuse", "group": 1, "binding": 1 } },
      { "uniform": "_smp_t_diffuse", "texture": { "name": "t_diffuse", "group": 1, "binding": 0 }, "sampler": null }
    ]
  }
}
"#;

    const VERTEX: &str = "#version 330 core\nvoid main() {}\n";
    const FRAGMENT: &str = "#version 330 core\nvoid main() {}\n";

    fn block(group: u32, binding: u32, name: &str) -> GlslResource {
        GlslResource::UniformBlock {
            group,
            binding,
            name: name.to_owned(),
        }
    }

    #[test]
    fn layout_package_round_trips_the_three_files() {
        let blob = assemble(Some(VERTEX), Some(FRAGMENT), LAYOUT);
        let package = parse(&blob).unwrap();

        assert_eq!(package.vertex, VERTEX.trim());
        assert_eq!(package.fragment, FRAGMENT.trim());
        assert_eq!(
            package.resources,
            vec![
                block(0, 0, "viewport_block_vertex"),
                block(0, 0, "viewport_block_fragment"),
                GlslResource::Sampler {
                    texture_group: 1,
                    texture_binding: 0,
                    sampler_group: 1,
                    sampler_binding: 1,
                    name: "_smp_t_diffuse_s_diffuse".to_owned(),
                },
                GlslResource::Texture {
                    group: 1,
                    binding: 0,
                    name: "_smp_t_diffuse".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn layout_package_accepts_a_single_stage() {
        let layout = r#"{ "shader": "x", "fragment": { "uniform_blocks": [], "samplers": [] } }"#;
        let blob = assemble(None, Some(FRAGMENT), layout);
        let package = parse(&blob).unwrap();

        assert_eq!(package.vertex, "");
        assert_eq!(package.fragment, FRAGMENT.trim());
        assert!(package.resources.is_empty());
    }

    #[test]
    fn layout_package_rejects_a_blob_with_no_stage() {
        let blob = assemble(None, None, LAYOUT);
        assert!(parse(&blob).unwrap_err().contains("no shader stage"));
    }

    #[test]
    fn layout_package_requires_a_layout_section() {
        let blob = format!("{LAYOUT_HEADER}\n[VERTEX]\n{VERTEX}[FRAGMENT]\n{FRAGMENT}");
        assert!(parse(&blob).unwrap_err().contains("[LAYOUT]"));
    }

    #[test]
    fn layout_package_rejects_a_repeated_section() {
        let blob = format!(
            "{LAYOUT_HEADER}\n[VERTEX]\n{VERTEX}[VERTEX]\n{VERTEX}[LAYOUT]\n{LAYOUT}"
        );
        assert!(parse(&blob).unwrap_err().contains("twice"));
    }

    #[test]
    fn layout_package_reports_malformed_json() {
        let blob = assemble(Some(VERTEX), Some(FRAGMENT), "{ \"vertex\": ");
        assert!(parse(&blob).unwrap_err().contains("layout"));
    }

    #[test]
    fn layout_package_reports_a_missing_field() {
        let layout = r#"{ "vertex": { "uniform_blocks": [ { "block": "b", "group": 0 } ], "samplers": [] } }"#;
        let blob = assemble(Some(VERTEX), None, layout);
        assert!(parse(&blob).unwrap_err().contains("binding"));
    }

    #[test]
    fn layout_package_rejects_a_slot_that_is_not_a_u32() {
        for slot in ["-1", "1.5", "4294967296", "\"0\""] {
            let layout = format!(
                r#"{{ "vertex": {{ "uniform_blocks": [ {{ "block": "b", "group": {slot}, "binding": 0 }} ], "samplers": [] }} }}"#
            );
            let blob = assemble(Some(VERTEX), None, &layout);
            assert!(parse(&blob).is_err(), "slot {slot} should be rejected");
        }
    }

    #[test]
    fn layout_package_tolerates_unknown_keys_and_string_escapes() {
        let layout = r#"{ "shader": "a\"b", "extra": [1, 2.5e1, true, null, {"k": []}],
            "vertex": { "uniform_blocks": [ { "block": "bA", "resource": "r", "group": 2, "binding": 3 } ], "samplers": [] } }"#;
        let blob = assemble(Some(VERTEX), None, layout);
        assert_eq!(parse(&blob).unwrap().resources, vec![block(2, 3, "bA")]);
    }

    #[test]
    fn layout_package_rejects_trailing_json() {
        let blob = assemble(Some(VERTEX), Some(FRAGMENT), "{} {}");
        assert!(parse(&blob).is_err());
    }

    #[test]
    fn legacy_pack_still_parses() {
        let pack = format!(
            "{LEGACY_HEADER}\n[VERTEX]\n{VERTEX}[FRAGMENT]\n{FRAGMENT}[RESOURCES]\n\
             SAMPLER 1 0 1 1 _group_1_binding_0_fs\n\
             TEXTURE 0 0 tex\n\
             UBO 0 0 Viewport_block_0Fragment\n"
        );
        let package = parse(&pack).unwrap();

        assert_eq!(package.vertex, VERTEX.trim());
        assert_eq!(package.fragment, FRAGMENT.trim());
        assert_eq!(
            package.resources,
            vec![
                GlslResource::Sampler {
                    texture_group: 1,
                    texture_binding: 0,
                    sampler_group: 1,
                    sampler_binding: 1,
                    name: "_group_1_binding_0_fs".to_owned(),
                },
                GlslResource::Texture {
                    group: 0,
                    binding: 0,
                    name: "tex".to_owned(),
                },
                block(0, 0, "Viewport_block_0Fragment"),
            ]
        );
    }

    #[test]
    fn every_builtin_legacy_pack_parses() {
        use crate::glslpack::*;
        for (name, bytes) in [
            ("rect", RECT),
            ("image", IMAGE),
            ("text", TEXT),
            ("text_color", TEXT_COLOR),
            ("text_decoration", TEXT_DECORATION),
            ("svg", SVG),
            ("frame_composite", FRAME_COMPOSITE),
            ("material", MATERIAL),
            ("morph", MORPH),
        ] {
            let text = std::str::from_utf8(bytes).unwrap();
            let package = parse(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(!package.vertex.is_empty() && !package.fragment.is_empty(), "{name}");
        }
    }

    #[test]
    fn unknown_header_is_rejected() {
        assert!(parse("#version 330 core\n").unwrap_err().contains("header"));
        assert!(parse("").unwrap_err().contains("header"));
    }

    #[test]
    fn legacy_pack_rejects_bad_resource_rows() {
        let pack = format!(
            "{LEGACY_HEADER}\n[VERTEX]\n{VERTEX}[FRAGMENT]\n{FRAGMENT}[RESOURCES]\nUBO x 0 name\n"
        );
        assert!(parse(&pack).unwrap_err().contains("UBO"));
    }
}
