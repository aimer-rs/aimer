//! Reading animatable properties: `{"a":0,"k":value}` and
//! `{"a":1,"k":[keyframes]}`.

use crate::json::Json;
use crate::model::{Bezier, Diagnostic, Interpolation, Keyframe, Property, Rgba, Vec2};

/// Collects diagnostics while a document is read, tagged with the layer that is
/// being read.
pub(crate) struct Ctx {
    pub layer: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl Ctx {
    pub(crate) fn new() -> Self {
        Self { layer: String::new(), diagnostics: Vec::new() }
    }

    pub(crate) fn unsupported(&mut self, feature: impl Into<String>) {
        self.diagnostics.push(Diagnostic::Unsupported {
            layer: self.layer.clone(),
            feature: feature.into(),
        });
    }

    pub(crate) fn expression(&mut self) {
        self.diagnostics.push(Diagnostic::Expression { layer: self.layer.clone() });
    }

    pub(crate) fn malformed(&mut self, what: &'static str) {
        self.diagnostics.push(Diagnostic::Malformed { layer: self.layer.clone(), what });
    }
}

#[inline]
pub(crate) fn number(json: &Json) -> Option<f32> {
    match json {
        Json::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// A value a property can hold.
pub(crate) trait Value: Sized {
    fn from_json(json: &Json) -> Option<Self>;
}

impl Value for f32 {
    /// Static values are bare numbers, keyframe values one-element arrays.
    fn from_json(json: &Json) -> Option<Self> {
        match json {
            Json::Array(items) => items.first().and_then(number),
            other => number(other),
        }
    }
}

impl Value for Vec2 {
    /// Lottie often stores a third (z) component; it is ignored.
    fn from_json(json: &Json) -> Option<Self> {
        match json {
            Json::Array(items) if items.len() >= 2 => Some([number(&items[0])?, number(&items[1])?]),
            _ => None,
        }
    }
}

impl Value for Rgba {
    fn from_json(json: &Json) -> Option<Self> {
        match json {
            Json::Array(items) if items.len() >= 3 => Some([
                number(&items[0])?,
                number(&items[1])?,
                number(&items[2])?,
                items.get(3).and_then(number).unwrap_or(1.0),
            ]),
            _ => None,
        }
    }
}

fn vec2_list(json: Option<&Json>) -> Option<Vec<Vec2>> {
    match json? {
        Json::Array(items) => items.iter().map(<Vec2 as Value>::from_json).collect(),
        _ => None,
    }
}

impl Value for Bezier {
    /// A path is an object; inside keyframes it is wrapped in a one-element array.
    fn from_json(json: &Json) -> Option<Self> {
        match json {
            Json::Array(items) => items.first().and_then(<Bezier as Value>::from_json),
            Json::Object(_) => {
                let vertices = vec2_list(json.get("v"))?;
                let in_tangents = vec2_list(json.get("i"))?;
                let out_tangents = vec2_list(json.get("o"))?;
                if in_tangents.len() != vertices.len() || out_tangents.len() != vertices.len() {
                    return None;
                }
                Some(Bezier {
                    closed: matches!(json.get("c"), Some(Json::Bool(true))),
                    vertices,
                    in_tangents,
                    out_tangents,
                })
            }
            _ => None,
        }
    }
}

/// Reads the property object `json`, or `default` when it is absent. `what`
/// names the property in diagnostics.
pub(crate) fn property<T: Value>(
    ctx: &mut Ctx,
    json: Option<&Json>,
    default: T,
    what: &'static str,
) -> Property<T> {
    let Some(json) = json else {
        return Property::Static(default);
    };
    if matches!(json.get("x"), Some(Json::String(_))) {
        ctx.expression();
    }
    let Some(k) = json.get("k") else {
        ctx.malformed(what);
        return Property::Static(default);
    };
    let animated = match json.get("a") {
        Some(Json::Number(n)) => *n != 0.0,
        // Files that omit `a` still mark keyframes by an array of objects.
        _ => matches!(k, Json::Array(items) if matches!(items.first(), Some(Json::Object(_)))),
    };
    if animated {
        let Json::Array(items) = k else {
            ctx.malformed(what);
            return Property::Static(default);
        };
        let keys = keyframes::<T>(ctx, items, what);
        if keys.is_empty() {
            ctx.malformed(what);
            return Property::Static(default);
        }
        return Property::Animated(keys);
    }
    match T::from_json(k) {
        Some(value) => Property::Static(value),
        None => {
            ctx.malformed(what);
            Property::Static(default)
        }
    }
}

fn keyframes<T: Value>(ctx: &mut Ctx, items: &[Json], what: &'static str) -> Vec<Keyframe<T>> {
    let mut keys = Vec::with_capacity(items.len());
    // Older exporters put a segment's end value in `e` and leave `s` off the
    // last keyframe; carry `e` forward as that keyframe's value.
    let mut carried: Option<T> = None;
    let mut warned_spatial = false;
    for item in items {
        let time = item.get("t").and_then(number);
        let value = item.get("s").and_then(T::from_json).or_else(|| carried.take());
        let (Some(time), Some(value)) = (time, value) else {
            ctx.malformed(what);
            continue;
        };
        carried = item.get("e").and_then(T::from_json);
        if !warned_spatial && has_spatial_tangents(item) {
            warned_spatial = true;
            ctx.unsupported("spatial tangents");
        }
        let interpolation = if matches!(item.get("h"), Some(Json::Number(n)) if *n != 0.0) {
            Interpolation::Hold
        } else {
            match (item.get("o").and_then(ease_point), item.get("i").and_then(ease_point)) {
                (Some(ease_out), Some(ease_in)) => Interpolation::Bezier { ease_out, ease_in },
                _ => Interpolation::Linear,
            }
        };
        keys.push(Keyframe { time, value, interpolation });
    }
    keys
}

/// `{"x":[0.5],"y":[1]}` or `{"x":0.5,"y":1}`. Per-component easing of
/// multi-dimensional values is reduced to its first component.
fn ease_point(json: &Json) -> Option<Vec2> {
    Some([f32::from_json(json.get("x")?)?, f32::from_json(json.get("y")?)?])
}

/// Whether a keyframe bends its motion path, which would need spatial bezier
/// interpolation. All-zero tangents are a straight line and cost nothing.
fn has_spatial_tangents(item: &Json) -> bool {
    ["to", "ti"].iter().any(|key| match item.get(key) {
        Some(Json::Array(values)) => values.iter().any(|v| matches!(v, Json::Number(n) if *n != 0.0)),
        _ => false,
    })
}
