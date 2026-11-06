//! Reading the composition header, layers and transforms.

use crate::error::LottieError;
use crate::json::Json;
use crate::model::{Composition, Layer, LayerKind, MatteMode, Position, Property, Transform};
use crate::property::{Ctx, number, property};
use crate::shapes::shapes;

impl Composition {
    /// Parses a Lottie document.
    ///
    /// Only a document that cannot be played at all is an error. Anything
    /// outside the supported subset, and any value that cannot be read, is
    /// skipped or replaced by a default and listed in
    /// [`Composition::diagnostics`].
    pub fn parse(source: impl AsRef<[u8]>) -> Result<Composition, LottieError> {
        let json = Json::parse(source.as_ref())
            .map_err(|e| LottieError::Syntax { offset: e.offset, reason: e.kind.reason() })?;
        if !matches!(json, Json::Object(_)) {
            return Err(LottieError::NotAnObject);
        }
        let required = |key: &'static str| {
            json.get(key).and_then(number).ok_or(LottieError::MissingField(key))
        };
        let (width, height) = (required("w")?, required("h")?);
        let (frame_rate, out_frame) = (required("fr")?, required("op")?);
        let in_frame = json.get("ip").and_then(number).unwrap_or(0.0);
        if !(width > 0.0 && width.is_finite()) {
            return Err(LottieError::InvalidField("w"));
        }
        if !(height > 0.0 && height.is_finite()) {
            return Err(LottieError::InvalidField("h"));
        }
        if !(frame_rate > 0.0 && frame_rate.is_finite()) {
            return Err(LottieError::InvalidField("fr"));
        }

        let mut ctx = Ctx::new();
        let mut layers = Vec::new();
        if let Some(Json::Array(items)) = json.get("layers") {
            for item in items {
                if let Some(layer) = layer(&mut ctx, item, in_frame, out_frame) {
                    layers.push(layer);
                }
            }
        }
        resolve_parents(&mut layers, &mut ctx);
        resolve_mattes(&mut layers, &mut ctx);
        Ok(Composition {
            width,
            height,
            frame_rate,
            in_frame,
            out_frame,
            layers,
            diagnostics: ctx.diagnostics,
        })
    }
}

pub(crate) fn name_of(json: &Json) -> String {
    match json.get("nm") {
        Some(Json::String(name)) => name.to_string(),
        _ => String::new(),
    }
}

fn layer(ctx: &mut Ctx, json: &Json, comp_in: f32, comp_out: f32) -> Option<Layer> {
    let name = name_of(json);
    ctx.layer.clone_from(&name);
    let Some(ty) = json.get("ty").and_then(number) else {
        ctx.malformed("layer type");
        return None;
    };
    let ty = ty as i64;
    let kind = match ty {
        3 => LayerKind::Null,
        4 => LayerKind::Shape(shapes(ctx, json.get("shapes"))),
        other => {
            ctx.unsupported(format!("layer type {other}"));
            LayerKind::Unsupported(other)
        }
    };
    let id = |key: &str| json.get(key).and_then(number).map(|n| n as i32);
    Some(Layer {
        index: id("ind"),
        parent: id("parent"),
        name,
        in_frame: json.get("ip").and_then(number).unwrap_or(comp_in),
        out_frame: json.get("op").and_then(number).unwrap_or(comp_out),
        start_frame: json.get("st").and_then(number).unwrap_or(0.0),
        hidden: matches!(json.get("hd"), Some(Json::Bool(true))),
        transform: transform(ctx, json.get("ks")),
        kind,
        matte: match json.get("tt").and_then(number).map(|n| n as i64) {
            Some(1) => Some(MatteMode::Alpha),
            Some(2) => Some(MatteMode::AlphaInverted),
            Some(3) => Some(MatteMode::Luma),
            Some(4) => Some(MatteMode::LumaInverted),
            _ => None,
        },
        is_matte: json.get("td").and_then(number).is_some_and(|n| n != 0.0),
        parent_slot: None,
        matte_slot: None,
        matte_index: id("tp"),
    })
}

/// Reads an object holding the transform keys `a`, `p`, `s`, `r`, `o`: a
/// layer's `ks` or a group's `tr` item.
pub(crate) fn transform(ctx: &mut Ctx, json: Option<&Json>) -> Transform {
    let Some(json) = json else {
        return Transform::default();
    };
    let position = match json.get("p") {
        Some(p) if matches!(p.get("s"), Some(Json::Bool(true))) => Position::Split {
            x: property(ctx, p.get("x"), 0.0, "position x"),
            y: property(ctx, p.get("y"), 0.0, "position y"),
        },
        other => Position::Joint(property(ctx, other, [0.0, 0.0], "position")),
    };
    // Skew would shear the layer; a non-zero value is not rendered.
    if let Some(skew) = json.get("sk")
        && property(ctx, Some(skew), 0.0f32, "skew") != Property::Static(0.0)
    {
        ctx.unsupported("skew");
    }
    Transform {
        anchor: property(ctx, json.get("a"), [0.0, 0.0], "anchor"),
        position,
        scale: property(ctx, json.get("s"), [100.0, 100.0], "scale"),
        rotation: property(ctx, json.get("r"), 0.0, "rotation"),
        opacity: property(ctx, json.get("o"), 100.0, "opacity"),
    }
}

/// Finds each matted layer's matte: the layer `tp` names, or, in files that
/// predate `tp`, the layer listed just before it. A layer whose matte does not
/// exist is drawn as it is, with a diagnostic, rather than hidden.
fn resolve_mattes(layers: &mut [Layer], ctx: &mut Ctx) {
    for i in 0..layers.len() {
        if layers[i].matte.is_none() {
            continue;
        }
        let slot = match layers[i].matte_index {
            Some(index) => layers.iter().position(|layer| layer.index == Some(index)),
            None => i.checked_sub(1),
        };
        match slot.filter(|&slot| slot != i) {
            Some(slot) => {
                layers[i].matte_slot = Some(slot);
                // Only an alpha matte can be applied; the scene leaves the
                // layer out rather than draw it uncut.
                let unsupported = match layers[i].matte {
                    Some(MatteMode::AlphaInverted) => Some("inverted alpha track matte"),
                    Some(MatteMode::Luma) => Some("luma track matte"),
                    Some(MatteMode::LumaInverted) => Some("inverted luma track matte"),
                    _ => None,
                };
                if let Some(kind) = unsupported {
                    ctx.layer.clone_from(&layers[i].name);
                    ctx.unsupported(format!("{kind}, layer not drawn"));
                }
            }
            None => {
                layers[i].matte = None;
                ctx.layer.clone_from(&layers[i].name);
                ctx.malformed("matte reference");
            }
        }
    }
}

/// Turns each layer's `parent` (an `ind` number) into a position in `layers`.
/// A parent that does not exist, or that would make a cycle, is dropped with a
/// diagnostic, which keeps the ancestor walk during evaluation finite.
fn resolve_parents(layers: &mut [Layer], ctx: &mut Ctx) {
    let slot_of = |layers: &[Layer], index: i32| {
        layers.iter().position(|layer| layer.index == Some(index))
    };
    for i in 0..layers.len() {
        let Some(parent) = layers[i].parent else { continue };
        layers[i].parent_slot = slot_of(layers, parent);
        if layers[i].parent_slot.is_none() {
            ctx.layer.clone_from(&layers[i].name);
            ctx.malformed("parent reference");
        }
    }
    // Colored walk: 0 = unseen, 1 = on the current path, 2 = known cycle-free.
    // Reaching a node that is on the current path closes a loop, and the edge
    // that closed it is the one to cut; layers that merely lead into the loop
    // keep their parents.
    let mut state = vec![0u8; layers.len()];
    let mut path = Vec::new();
    for start in 0..layers.len() {
        let mut next = Some(start);
        let mut prev = start;
        while let Some(slot) = next {
            match state[slot] {
                2 => break,
                1 => {
                    layers[prev].parent_slot = None;
                    ctx.layer.clone_from(&layers[prev].name);
                    ctx.malformed("parent reference");
                    break;
                }
                _ => {}
            }
            state[slot] = 1;
            path.push(slot);
            prev = slot;
            next = layers[slot].parent_slot;
        }
        for slot in path.drain(..) {
            state[slot] = 2;
        }
    }
}
