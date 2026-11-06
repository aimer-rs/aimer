//! Reading the `shapes` list of a shape layer.

use crate::json::Json;
use crate::model::{
    Bezier, Ellipse, Fill, FillRule, Group, LineCap, LineJoin, PathShape, Rect, Shape, Stroke,
    Trim,
};
use crate::parse::{name_of, transform};
use crate::property::{Ctx, number, property};

/// Reads a list of shape items. Unsupported and hidden items are skipped.
pub(crate) fn shapes(ctx: &mut Ctx, items: Option<&Json>) -> Vec<Shape> {
    let Some(Json::Array(items)) = items else {
        return Vec::new();
    };
    items.iter().filter_map(|item| shape(ctx, item)).collect()
}

fn shape(ctx: &mut Ctx, json: &Json) -> Option<Shape> {
    if matches!(json.get("hd"), Some(Json::Bool(true))) {
        return None;
    }
    let Some(Json::String(ty)) = json.get("ty") else {
        ctx.malformed("shape type");
        return None;
    };
    Some(match ty.as_ref() {
        "gr" => Shape::Group(group(ctx, json)),
        "rc" => Shape::Rect(Rect {
            position: property(ctx, json.get("p"), [0.0, 0.0], "rectangle position"),
            size: property(ctx, json.get("s"), [0.0, 0.0], "rectangle size"),
            roundness: property(ctx, json.get("r"), 0.0, "rectangle roundness"),
        }),
        "el" => Shape::Ellipse(Ellipse {
            position: property(ctx, json.get("p"), [0.0, 0.0], "ellipse position"),
            size: property(ctx, json.get("s"), [0.0, 0.0], "ellipse size"),
        }),
        "sh" => {
            let empty = Bezier {
                closed: false,
                vertices: Vec::new(),
                in_tangents: Vec::new(),
                out_tangents: Vec::new(),
            };
            Shape::Path(PathShape { path: property(ctx, json.get("ks"), empty, "path") })
        }
        "fl" => Shape::Fill(Fill {
            color: property(ctx, json.get("c"), [1.0; 4], "fill color"),
            opacity: property(ctx, json.get("o"), 100.0, "fill opacity"),
            rule: match code(json, "r") {
                2 => FillRule::EvenOdd,
                _ => FillRule::NonZero,
            },
        }),
        "st" => Shape::Stroke(Stroke {
            color: property(ctx, json.get("c"), [1.0; 4], "stroke color"),
            opacity: property(ctx, json.get("o"), 100.0, "stroke opacity"),
            width: property(ctx, json.get("w"), 1.0, "stroke width"),
            cap: match code(json, "lc") {
                2 => LineCap::Round,
                3 => LineCap::Square,
                _ => LineCap::Butt,
            },
            join: match code(json, "lj") {
                2 => LineJoin::Round,
                3 => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            miter_limit: json.get("ml").and_then(number).unwrap_or(4.0),
        }),
        "tm" => Shape::Trim(Trim {
            start: property(ctx, json.get("s"), 0.0, "trim start"),
            end: property(ctx, json.get("e"), 100.0, "trim end"),
            offset: property(ctx, json.get("o"), 0.0, "trim offset"),
            simultaneous: code(json, "m") != 2,
        }),
        // A transform outside a group has nothing to apply to.
        "tr" => return None,
        other => {
            ctx.unsupported(format!("shape type `{other}`"));
            return None;
        }
    })
}

/// An enum-like integer field (`lc`, `lj`, `r`, `m`); `0` when absent.
fn code(json: &Json, key: &str) -> i32 {
    json.get(key).and_then(number).map_or(0, |n| n as i32)
}

/// A group's own transform is its `tr` item; it is lifted out of the items.
fn group(ctx: &mut Ctx, json: &Json) -> Group {
    let Some(Json::Array(raw)) = json.get("it") else {
        return Group { name: name_of(json), items: Vec::new(), transform: Default::default() };
    };
    let tr = raw.iter().find(|item| matches!(item.get("ty"), Some(Json::String(t)) if t == "tr"));
    Group {
        name: name_of(json),
        items: raw.iter().filter_map(|item| shape(ctx, item)).collect(),
        transform: transform(ctx, tr),
    }
}
