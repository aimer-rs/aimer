//! Dependency-free parser for Cupid's retained SVG scene format.
//!
//! The parser intentionally converts source documents to Cupid's own path and
//! scene types. GPU rendering and path tessellation operate on those types.

#[cfg(test)]
mod icon_tests;

mod shapes;
use shapes::element_path;

use std::collections::HashMap;
use std::sync::Arc;

use super::resources::{
    SvgBlendMode, SvgClipPath, SvgColorMatrix, SvgCompositeOperator, SvgFilter, SvgFilterInput,
    SvgFilterPrimitive, SvgMask, SvgMaskType, SvgPattern, SvgResourceGraph, SvgResourceUnits,
};
use super::{
    SvgColor, SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgGradient, SvgGradientStop,
    SvgGradientUnits, SvgLineCap, SvgLineJoin, SvgNode, SvgNodeId, SvgPaint, SvgPaintOrder,
    SvgPathCommand, SvgScene, SvgSpreadMethod, SvgStroke, SvgTransform, SvgViewport,
};
use super::{SvgFitPolicy, SvgPreserveAspectRatio, SvgViewBox};

const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODES: usize = 16_384;
const MAX_COMMANDS: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum SvgParseError {
    Empty,
    TooLarge,
    InvalidUtf8,
    InvalidXml(&'static str),
    InvalidRoot,
    InvalidNumber,
    InvalidPath,
    Unsupported(&'static str),
    InvalidViewBox,
    InvalidPreserveAspectRatio,
    ExternalResource(String),
    LimitExceeded(&'static str),
}

impl std::fmt::Display for SvgParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("SVG source is empty"),
            Self::TooLarge => formatter.write_str("SVG source exceeds the parser limit"),
            Self::InvalidUtf8 => formatter.write_str("SVG source is not UTF-8"),
            Self::InvalidXml(message) => write!(formatter, "invalid SVG XML: {message}"),
            Self::InvalidRoot => formatter.write_str("SVG root requires positive width and height"),
            Self::InvalidNumber => formatter.write_str("SVG contains an invalid number"),
            Self::InvalidPath => formatter.write_str("SVG contains invalid path data"),
            Self::Unsupported(feature) => write!(formatter, "unsupported SVG feature: {feature}"),
            Self::InvalidViewBox => formatter.write_str("invalid SVG viewBox"),
            Self::InvalidPreserveAspectRatio => {
                formatter.write_str("invalid SVG preserveAspectRatio")
            }
            Self::ExternalResource(resource) => write!(
                formatter,
                "external SVG resource is not allowed: {resource}"
            ),
            Self::LimitExceeded(resource) => write!(formatter, "SVG {resource} limit exceeded"),
        }
    }
}

/// Parsed SVG paint metadata retained beside the renderer-ready scene.
#[derive(Clone, Debug, Default)]
pub struct SvgParsedPaints {
    /// Fill paint, including references to parsed gradients.
    pub fill: Option<SvgPaint>,
    /// Stroke paint, including references to parsed gradients.
    pub stroke: Option<SvgPaint>,
}

/// Full Cupid-owned result of parsing an SVG document.
#[derive(Clone, Debug)]
pub struct SvgParsedDocument {
    /// Retained geometry and styling consumed by the renderer.
    pub scene: SvgScene,
    /// Root user-space rectangle, or a viewport-sized fallback.
    pub view_box: SvgViewBox,
    /// Root aspect-ratio rule.
    pub preserve_aspect_ratio: SvgPreserveAspectRatio,
    /// Whether the source declared a `viewBox`.
    pub has_view_box: bool,
    /// Root mapping already composed into retained node transforms.
    pub root_transform: SvgTransform,
    /// Gradient definitions found in the document.
    pub gradients: Arc<[SvgGradient]>,
    /// Clip paths, masks, filters, and patterns defined by the SVG.
    pub resources: Arc<SvgResourceGraph>,
    /// Source paint values keyed by retained scene node id.
    pub node_paints: HashMap<SvgNodeId, SvgParsedPaints>,
    /// Element subtrees that were skipped because the renderer does not support them.
    pub unsupported_elements: Arc<[Arc<str>]>,
}

impl std::error::Error for SvgParseError {}

#[derive(Clone)]
struct Style {
    color: SvgColor,
    fill_current_color: bool,
    stroke_current_color: bool,
    fill_opacity: f32,
    stroke_opacity: f32,
    fill: Option<SvgColor>,
    stroke: Option<SvgColor>,
    stroke_width: f32,
    fill_rule: SvgFillRule,
    opacity: f32,
    visible: bool,
    cap: SvgLineCap,
    join: SvgLineJoin,
    clip_path: Option<Arc<str>>,
    mask: Option<Arc<str>>,
    filter: Option<Arc<str>>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: SvgColor::rgba8(0, 0, 0, 255),
            fill_current_color: false,
            stroke_current_color: false,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            fill: Some(SvgColor::rgba8(0, 0, 0, 255)),
            stroke: None,
            stroke_width: 1.0,
            fill_rule: SvgFillRule::NonZero,
            opacity: 1.0,
            visible: true,
            cap: SvgLineCap::Butt,
            join: SvgLineJoin::Miter,
            clip_path: None,
            mask: None,
            filter: None,
        }
    }
}

impl Style {
    fn fill_color(&self) -> Option<SvgColor> {
        self.fill.map(|mut color| { color.a *= self.fill_opacity; color })
    }

    fn stroke_color(&self) -> Option<SvgColor> {
        self.stroke.map(|mut color| { color.a *= self.stroke_opacity; color })
    }
}

struct OpenNode {
    id: SvgNodeId,
    style: Style,
    transform: SvgTransform,
    tag: String,
    resource_owner: Option<Arc<str>>,
}

struct UseReference {
    node: SvgNodeId,
    target_id: String,
    attrs: HashMap<String, String>,
    style: Style,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CssSelector {
    Id(String),
    Class(String),
    Element(String),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CssSpecificity {
    ids: u8,
    classes: u8,
    elements: u8,
}

impl CssSelector {
    fn specificity(&self) -> CssSpecificity {
        match self {
            Self::Id(_) => CssSpecificity {
                ids: 1,
                classes: 0,
                elements: 0,
            },
            Self::Class(_) => CssSpecificity {
                ids: 0,
                classes: 1,
                elements: 0,
            },
            Self::Element(_) => CssSpecificity {
                ids: 0,
                classes: 0,
                elements: 1,
            },
        }
    }
}

#[derive(Clone)]
struct CssDeclaration {
    property: String,
    value: String,
    important: bool,
    order: usize,
}

struct CssRule {
    declarations: Vec<CssDeclaration>,
}

#[derive(Default)]
struct Stylesheet {
    rules: Vec<CssRule>,
    by_id: HashMap<String, Vec<(usize, CssSpecificity)>>,
    by_class: HashMap<String, Vec<(usize, CssSpecificity)>>,
    by_element: HashMap<String, Vec<(usize, CssSpecificity)>>,
    next_order: usize,
}

/// Parses an SVG document into Cupid-owned scene and geometry data.
///
/// Supported elements are `svg`, `g`, `path`, `rect`, `circle`, `ellipse`,
/// `line`, `polyline`, and `polygon`. Local `<use>` references to those shapes
/// are supported. Paths support the SVG M/L/H/V/C/S/Q/T/Z commands in both
/// absolute and relative forms. Embedded `<style>` rules support element, ID,
/// and class selectors for those elements. Definition shapes are retained for
/// supported local SVG resources. External resources, scripts, and animation
/// are rejected.
pub fn parse_svg(bytes: impl AsRef<[u8]>) -> Result<SvgScene, SvgParseError> {
    parse_svg_document(bytes).map(|document| document.scene)
}

/// Parses an SVG while retaining its root fit policy and deferred paints.
pub fn parse_svg_document(bytes: impl AsRef<[u8]>) -> Result<SvgParsedDocument, SvgParseError> {
    let bytes = bytes.as_ref();
    if bytes.is_empty() {
        return Err(SvgParseError::Empty);
    }
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(SvgParseError::TooLarge);
    }
    let source = std::str::from_utf8(bytes).map_err(|_| SvgParseError::InvalidUtf8)?;
    if source.contains("<!DOCTYPE") || source.contains("<!ENTITY") {
        return Err(SvgParseError::Unsupported("DTD and entities"));
    }
    let stylesheet = collect_stylesheet_rules(source)?;

    let mut viewport = None;
    let mut root_transform = SvgTransform::default();
    let mut root_style = Style::default();
    let mut declared_view_box = None;
    let mut preserve_aspect_ratio = SvgPreserveAspectRatio::default();
    let mut nodes: Vec<SvgNode> = Vec::new();
    let mut geometries = Vec::new();
    let mut unsupported_elements = Vec::new();
    let mut node_ids = HashMap::<String, SvgNodeId>::new();
    let mut id_attributes = HashMap::<String, (String, HashMap<String, String>)>::new();
    let mut use_references = Vec::new();
    let mut stack: Vec<OpenNode> = Vec::new();
    let mut cursor = 0;
    let mut command_count = 0usize;
    while let Some(relative) = source[cursor..].find('<') {
        cursor += relative;
        if source[cursor..].starts_with("<!--") {
            let end = source[cursor + 4..]
                .find("-->")
                .ok_or(SvgParseError::InvalidXml("unterminated comment"))?;
            cursor += 4 + end + 3;
            continue;
        }
        if source[cursor..].starts_with("<?") {
            let end = source[cursor + 2..]
                .find("?>")
                .ok_or(SvgParseError::InvalidXml("unterminated declaration"))?;
            cursor += 2 + end + 2;
            continue;
        }
        let end = find_tag_end(source, cursor + 1)?;
        let mut body = source[cursor + 1..end].trim();
        cursor = end + 1;
        if body.starts_with('!') {
            return Err(SvgParseError::Unsupported("XML declarations"));
        }
        if let Some(close) = body.strip_prefix('/') {
            let name = close.trim();
            let open = stack
                .pop()
                .ok_or(SvgParseError::InvalidXml("unexpected close tag"))?;
            if open.tag != name {
                return Err(SvgParseError::InvalidXml("mismatched close tag"));
            }
            continue;
        }
        let self_closing = body.ends_with('/');
        if self_closing {
            body = body[..body.len() - 1].trim_end();
        }
        let (tag, attrs) = parse_tag(body)?;
        if tag == "image" {
            unsupported_elements.push(Arc::from(tag));
            if let Some(href) = attrs.get("href").or_else(|| attrs.get("xlink:href")) {
                if !href.trim().is_empty() && !href.trim().starts_with('#') {
                    return Err(SvgParseError::ExternalResource(href.clone()));
                }
            }
            skip_element_subtree(source, &mut cursor, tag, self_closing)?;
            continue;
        }
        if stack.is_empty() && tag != "svg" {
            return Err(SvgParseError::InvalidRoot);
        }
        if tag == "svg" {
            if viewport.is_some() {
                return Err(SvgParseError::Unsupported("nested SVG viewport"));
            }
            let view_box = attrs
                .get("viewBox")
                .map(|v| parse_viewbox(v).ok_or(SvgParseError::InvalidViewBox))
                .transpose()?;
            preserve_aspect_ratio = attrs
                .get("preserveAspectRatio")
                .map(|v| {
                    v.parse::<SvgPreserveAspectRatio>()
                        .map_err(|_| SvgParseError::InvalidPreserveAspectRatio)
                })
                .transpose()?
                .unwrap_or_default();
            let w = attrs
                .get("width")
                .and_then(|v| parse_length(v))
                .or_else(|| view_box.map(|b| b[2]))
                .unwrap_or(300.0);
            let h = attrs
                .get("height")
                .and_then(|v| parse_length(v))
                .or_else(|| view_box.map(|b| b[3]))
                .unwrap_or(150.0);
            if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
                return Err(SvgParseError::InvalidRoot);
            }
            viewport = Some(SvgViewport {
                width: w,
                height: h,
            });
            if let Some([x, y, width, height]) = view_box {
                let box_ = SvgViewBox::try_new(x, y, width, height)
                    .map_err(|_| SvgParseError::InvalidRoot)?;
                root_transform = box_
                    .fit_transform(
                        w,
                        h,
                        SvgFitPolicy::PreserveAspectRatio(preserve_aspect_ratio),
                    )
                    .map_err(|_| SvgParseError::InvalidViewBox)?;
                declared_view_box = Some(box_);
            }
            root_style = parse_style(Style::default(), &attrs, tag, &stylesheet)?;
            let content_transform = attrs
                .get("transform")
                .map(|value| parse_transform(value))
                .transpose()?
                .map_or(root_transform, |transform| root_transform.mul(transform));
            if !self_closing {
                stack.push(OpenNode {
                    id: SvgNodeId(u32::MAX),
                    style: root_style.clone(),
                    transform: content_transform,
                    tag: tag.to_owned(),
                    resource_owner: None,
                });
            }
            continue;
        }
        if tag == "defs" {
            if !self_closing {
                stack.push(OpenNode {
                    id: SvgNodeId(u32::MAX),
                    style: Style::default(),
                    transform: root_transform,
                    tag: tag.to_owned(),
                    resource_owner: None,
                });
            }
            continue;
        }
        if tag == "style" {
            if !self_closing {
                let (_, close_end) = find_style_end(source, cursor)?;
                cursor = close_end + 1;
            }
            continue;
        }
        if matches!(tag, "title" | "desc" | "metadata") {
            if !self_closing {
                stack.push(OpenNode {
                    id: SvgNodeId(u32::MAX),
                    style: root_style.clone(),
                    transform: root_transform,
                    tag: tag.to_owned(),
                    resource_owner: None,
                });
            }
            continue;
        }
        if tag == "use" {
            let Some(href) = attrs.get("href").or_else(|| attrs.get("xlink:href")) else {
                unsupported_elements.push(Arc::from(tag));
                skip_element_subtree(source, &mut cursor, tag, self_closing)?;
                continue;
            };
            let href = href.trim();
            if !href.starts_with('#') {
                return Err(SvgParseError::ExternalResource(href.to_owned()));
            }
            let Some(target_id) = href.strip_prefix('#').filter(|id| !id.is_empty()) else {
                unsupported_elements.push(Arc::from(tag));
                skip_element_subtree(source, &mut cursor, tag, self_closing)?;
                continue;
            };
            if nodes.len() >= MAX_NODES {
                return Err(SvgParseError::LimitExceeded("node"));
            }
            let inherited = stack
                .last()
                .map(|node| node.style.clone())
                .unwrap_or_default();
            let style = parse_style(inherited, &attrs, tag, &stylesheet)?;
            let own_transform = attrs
                .get("transform")
                .map(|value| parse_transform(value))
                .transpose()?
                .unwrap_or_default();
            let x = attrs
                .get("x")
                .map(|value| parse_length(value).ok_or(SvgParseError::InvalidNumber))
                .transpose()?
                .unwrap_or(0.0);
            let y = attrs
                .get("y")
                .map(|value| parse_length(value).ok_or(SvgParseError::InvalidNumber))
                .transpose()?
                .unwrap_or(0.0);
            let placement = stack
                .last()
                .map(|node| node.transform)
                .unwrap_or(root_transform)
                .mul(own_transform)
                .mul(SvgTransform {
                    tx: x,
                    ty: y,
                    ..SvgTransform::default()
                });
            let id = SvgNodeId(nodes.len() as u32);
            let parent = stack
                .last()
                .map(|node| node.id)
                .filter(|parent| parent.0 != u32::MAX);
            let svg_id = attrs
                .get("id")
                .filter(|value| !value.is_empty())
                .map(|value| Arc::<str>::from(value.as_str()));
            let classes: Arc<[Arc<str>]> = attrs
                .get("class")
                .map(|value| {
                    value
                        .split_ascii_whitespace()
                        .map(Arc::<str>::from)
                        .collect::<Vec<_>>()
                        .into()
                })
                .unwrap_or_else(|| Arc::from([]));
            let definition_owner = stack
                .iter()
                .rev()
                .find_map(|node| node.resource_owner.clone());
            let is_definition = stack.iter().any(|node| {
                matches!(
                    node.tag.as_str(),
                    "defs"
                        | "clipPath"
                        | "mask"
                        | "filter"
                        | "pattern"
                        | "linearGradient"
                        | "radialGradient"
                )
            }) || definition_owner.is_some();
            nodes.push(SvgNode {
                node_id: id,
                svg_id,
                classes,
                element: SvgElementKind::Group,
                is_definition,
                definition_owner,
                clip_path: style.clip_path.clone(),
                mask: style.mask.clone(),
                filter: style.filter.clone(),
                parent,
                children: Arc::from([]),
                transform: placement,
                opacity: style.opacity,
                geometry: None,
                fill_rule: style.fill_rule,
                fill: style.fill_color().map(|color| SvgFill {
                    color,
                    rule: style.fill_rule,
                }),
                stroke: style.stroke_color().map(|color| SvgStroke {
                    color,
                    width: style.stroke_width,
                    line_cap: style.cap,
                    line_join: style.join,
                    miter_limit: 4.0,
                    dash_array: Arc::from([]),
                    dash_offset: 0.0,
                }),
                fill_paint: None,
                stroke_paint: None,
                paint_order: SvgPaintOrder::FillAndStroke,
                visible: style.visible,
            });
            if let Some(parent) = parent {
                let mut children = nodes[parent.0 as usize].children.to_vec();
                children.push(id);
                nodes[parent.0 as usize].children = children.into();
            }
            if let Some(svg_id) = attrs.get("id").filter(|value| !value.is_empty()) {
                node_ids.entry(svg_id.clone()).or_insert(id);
            }
            use_references.push(UseReference {
                node: id,
                target_id: target_id.to_owned(),
                attrs: attrs.clone(),
                style,
            });
            if !self_closing {
                skip_element_subtree(source, &mut cursor, tag, false)?;
            }
            continue;
        }
        if matches!(
            tag,
            "text"
                | "script"
                | "animate"
                | "animateMotion"
                | "animateTransform"
                | "set"
                | "foreignObject"
        ) {
            unsupported_elements.push(Arc::from(tag));
            skip_element_subtree(source, &mut cursor, tag, self_closing)?;
            continue;
        }
        if stack.iter().any(|node| {
            matches!(
                node.tag.as_str(),
                "defs"
                    | "clipPath"
                    | "mask"
                    | "filter"
                    | "pattern"
                    | "linearGradient"
                    | "radialGradient"
            )
        }) && (matches!(
            tag,
            "symbol"
                | "linearGradient"
                | "radialGradient"
                | "clipPath"
                | "mask"
                | "filter"
                | "pattern"
                | "stop"
        ) || tag.starts_with("fe"))
        {
            if tag == "symbol" {
                unsupported_elements.push(Arc::from(tag));
            }
            let resource_owner = if matches!(tag, "clipPath" | "mask" | "pattern") {
                Some(Arc::<str>::from(
                    attrs
                        .get("id")
                        .ok_or(SvgParseError::InvalidXml("resource id is required"))?
                        .as_str(),
                ))
            } else {
                stack
                    .iter()
                    .rev()
                    .find_map(|node| node.resource_owner.clone())
            };
            let own_transform = attrs
                .get("transform")
                .map(|value| parse_transform(value))
                .transpose()?
                .unwrap_or_default();
            let parent_transform = stack
                .last()
                .map(|node| node.transform)
                .unwrap_or(root_transform);
            if !self_closing {
                stack.push(OpenNode {
                    id: SvgNodeId(u32::MAX),
                    style: Style::default(),
                    transform: parent_transform.mul(own_transform),
                    tag: tag.to_owned(),
                    resource_owner,
                });
            }
            continue;
        }
        if matches!(
            tag,
            "clipPath" | "mask" | "filter" | "pattern" | "linearGradient" | "radialGradient"
        ) {
            let resource_owner = if matches!(tag, "clipPath" | "mask" | "pattern") {
                Some(Arc::<str>::from(
                    attrs
                        .get("id")
                        .ok_or(SvgParseError::InvalidXml("resource id is required"))?
                        .as_str(),
                ))
            } else {
                None
            };
            let own_transform = attrs
                .get("transform")
                .map(|value| parse_transform(value))
                .transpose()?
                .unwrap_or_default();
            let parent_transform = stack
                .last()
                .map(|node| node.transform)
                .unwrap_or(root_transform);
            if !self_closing {
                stack.push(OpenNode {
                    id: SvgNodeId(u32::MAX),
                    style: Style::default(),
                    transform: parent_transform.mul(own_transform),
                    tag: tag.to_owned(),
                    resource_owner,
                });
            }
            continue;
        }
        if matches!(
            tag,
            "defs" | "symbol" | "clipPath" | "mask" | "filter" | "pattern"
        ) {
            return Err(SvgParseError::Unsupported(
                "definitions, clip paths, masks, filters, and patterns",
            ));
        }
        if !matches!(
            tag,
            "g" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
        ) {
            unsupported_elements.push(Arc::from(tag));
            skip_element_subtree(source, &mut cursor, tag, self_closing)?;
            continue;
        }
        if nodes.len() >= MAX_NODES {
            return Err(SvgParseError::LimitExceeded("node"));
        }
        let inherited = stack.last().map(|n| n.style.clone()).unwrap_or_default();
        let style = parse_style(inherited, &attrs, tag, &stylesheet)?;
        let own_transform = attrs
            .get("transform")
            .map(|v| parse_transform(v))
            .transpose()?
            .unwrap_or_default();
        let parent_transform = stack.last().map(|n| n.transform).unwrap_or_default();
        let transform = parent_transform.mul(own_transform);
        let id = SvgNodeId(nodes.len() as u32);
        let parent = stack.last().map(|n| n.id).filter(|id| id.0 != u32::MAX);
        let classes: Arc<[Arc<str>]> = attrs
            .get("class")
            .map(|v| {
                v.split_ascii_whitespace()
                    .map(Arc::<str>::from)
                    .collect::<Vec<_>>()
                    .into()
            })
            .unwrap_or_else(|| Arc::from([]));
        let svg_id = attrs
            .get("id")
            .filter(|v| !v.is_empty())
            .map(|v| Arc::<str>::from(v.as_str()));
        let geometry = if tag == "g" {
            None
        } else {
            let commands = element_path(tag, &attrs)?;
            command_count = command_count
                .checked_add(commands.len())
                .ok_or(SvgParseError::LimitExceeded("path command"))?;
            if command_count > MAX_COMMANDS {
                return Err(SvgParseError::LimitExceeded("path command"));
            }
            let index = geometries.len();
            geometries.push(SvgGeometry {
                commands: commands.into(),
            });
            Some(index)
        };
        let fill = style.fill_color().map(|color| SvgFill {
            color,
            rule: style.fill_rule,
        });
        let stroke = style.stroke_color().map(|color| SvgStroke {
            color,
            width: style.stroke_width,
            line_cap: style.cap,
            line_join: style.join,
            miter_limit: 4.0,
            dash_array: Arc::from([]),
            dash_offset: 0.0,
        });
        let definition_owner = stack
            .iter()
            .rev()
            .find_map(|node| node.resource_owner.clone());
        let is_definition = stack.iter().any(|node| {
            matches!(
                node.tag.as_str(),
                "defs"
                    | "clipPath"
                    | "mask"
                    | "filter"
                    | "pattern"
                    | "linearGradient"
                    | "radialGradient"
            )
        }) || definition_owner.is_some();
        nodes.push(SvgNode {
            node_id: id,
            svg_id,
            classes,
            element: if tag == "g" {
                SvgElementKind::Group
            } else {
                SvgElementKind::Path
            },
            is_definition,
            definition_owner,
            clip_path: style.clip_path.clone(),
            mask: style.mask.clone(),
            filter: style.filter.clone(),
            parent,
            children: Arc::from([]),
            transform,
            opacity: style.opacity,
            geometry,
            fill_rule: style.fill_rule,
            fill,
            stroke,
            fill_paint: None,
            stroke_paint: None,
            paint_order: SvgPaintOrder::FillAndStroke,
            visible: style.visible,
        });
        if let Some(parent) = parent {
            let mut children = nodes[parent.0 as usize].children.to_vec();
            children.push(id);
            nodes[parent.0 as usize].children = children.into();
        }
        if let Some(svg_id) = attrs.get("id").filter(|value| !value.is_empty()) {
            node_ids.entry(svg_id.clone()).or_insert(id);
            id_attributes
                .entry(svg_id.clone())
                .or_insert_with(|| (tag.to_owned(), attrs.clone()));
        }
        if !self_closing {
            stack.push(OpenNode {
                id,
                style,
                transform,
                tag: tag.to_owned(),
                resource_owner: nodes[id.0 as usize].definition_owner.clone(),
            });
        }
    }
    if !stack.is_empty() {
        return Err(SvgParseError::InvalidXml("unclosed tag"));
    }
    let viewport = viewport.ok_or(SvgParseError::InvalidRoot)?;
    let view_box = declared_view_box.unwrap_or(
        SvgViewBox::try_new(0.0, 0.0, viewport.width, viewport.height)
            .map_err(|_| SvgParseError::InvalidViewBox)?,
    );
    let use_targets = resolve_use_references(
        &use_references,
        &node_ids,
        &id_attributes,
        &stylesheet,
        root_transform,
        &mut nodes,
        &mut unsupported_elements,
    );
    let gradients = collect_gradients(source)?;
    let mut node_paints =
        collect_node_paints(source, &nodes, &gradients, &stylesheet, &use_targets)?;
    for reference in &use_references {
        let Some(target_node) = use_targets.get(&reference.node) else {
            continue;
        };
        let mut paints = node_paints.get(target_node).cloned().unwrap_or_default();
        if computed_style_value(&reference.attrs, "use", "fill", &stylesheet).is_some() {
            paints.fill = style_paint(&reference.attrs, "use", "fill", &gradients, &stylesheet)?;
        }
        if computed_style_value(&reference.attrs, "use", "stroke", &stylesheet).is_some() {
            paints.stroke =
                style_paint(&reference.attrs, "use", "stroke", &gradients, &stylesheet)?;
        }
        node_paints.insert(reference.node, paints);
    }
    for node in &mut nodes {
        if let Some(paints) = node_paints.get(&node.node_id) {
            node.fill_paint = paints.fill.clone();
            node.stroke_paint = paints.stroke.clone();
        }
    }
    let resources = collect_resources(source, &nodes, &gradients, root_transform)?;
    let scene = SvgScene {
        viewport,
        nodes: nodes.into(),
        geometries: geometries.into(),
        resources: Arc::new(resources.clone()),
    };
    Ok(SvgParsedDocument {
        scene,
        view_box,
        preserve_aspect_ratio,
        has_view_box: declared_view_box.is_some(),
        root_transform,
        gradients: gradients.into(),
        resources: Arc::new(resources),
        node_paints,
        unsupported_elements: unsupported_elements.into(),
    })
}

fn skip_element_subtree(
    source: &str,
    cursor: &mut usize,
    root_tag: &str,
    self_closing: bool,
) -> Result<(), SvgParseError> {
    if self_closing {
        return Ok(());
    }
    let mut open_tags = vec![root_tag];
    while !open_tags.is_empty() {
        let relative = source[*cursor..]
            .find('<')
            .ok_or(SvgParseError::InvalidXml("unclosed unsupported element"))?;
        *cursor += relative;
        if source[*cursor..].starts_with("<!--") {
            let end = source[*cursor + 4..]
                .find("-->")
                .ok_or(SvgParseError::InvalidXml("unterminated comment"))?;
            *cursor += 4 + end + 3;
            continue;
        }
        let end = find_tag_end(source, *cursor + 1)?;
        let mut body = source[*cursor + 1..end].trim();
        *cursor = end + 1;
        if let Some(close) = body.strip_prefix('/') {
            let name = close.trim();
            if open_tags.pop() != Some(name) {
                return Err(SvgParseError::InvalidXml(
                    "mismatched tag in unsupported element",
                ));
            }
            continue;
        }
        if body.starts_with('!') || body.starts_with('?') {
            continue;
        }
        let self_closing = body.ends_with('/');
        if self_closing {
            body = body[..body.len() - 1].trim_end();
        }
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let tag = &body[..name_end];
        if tag.is_empty() {
            return Err(SvgParseError::InvalidXml("empty nested tag"));
        }
        if !self_closing {
            open_tags.push(tag);
        }
    }
    Ok(())
}

fn find_tag_end(source: &str, mut cursor: usize) -> Result<usize, SvgParseError> {
    let bytes = source.as_bytes();
    let mut quote = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = byte;
        } else if byte == b'>' {
            return Ok(cursor);
        }
        cursor += 1;
    }
    Err(SvgParseError::InvalidXml("unterminated tag"))
}

fn parse_tag(
    body: &str,
) -> Result<(&str, std::collections::HashMap<String, String>), SvgParseError> {
    let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = &body[..name_end];
    if name.is_empty() {
        return Err(SvgParseError::InvalidXml("empty tag name"));
    }
    let mut attrs = std::collections::HashMap::new();
    let bytes = body.as_bytes();
    let mut i = name_end;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b':' | b'_' | b'-'))
        {
            i += 1;
        }
        if start == i {
            return Err(SvgParseError::InvalidXml("invalid attribute name"));
        }
        let key = &body[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            return Err(SvgParseError::InvalidXml("attribute missing value"));
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *bytes
            .get(i)
            .ok_or(SvgParseError::InvalidXml("attribute missing quote"))?;
        if quote != b'\'' && quote != b'"' {
            return Err(SvgParseError::InvalidXml("attribute must be quoted"));
        }
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i == bytes.len() {
            return Err(SvgParseError::InvalidXml("unterminated attribute"));
        }
        let value = body[value_start..i]
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        i += 1;
        attrs.insert(key.to_owned(), value);
    }
    Ok((name, attrs))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CascadePriority {
    important: bool,
    inline: bool,
    ids: u8,
    classes: u8,
    elements: u8,
    order: usize,
}

impl CascadePriority {
    const fn presentation() -> Self {
        Self {
            important: false,
            inline: false,
            ids: 0,
            classes: 0,
            elements: 0,
            order: 0,
        }
    }

    const fn stylesheet(important: bool, specificity: CssSpecificity, order: usize) -> Self {
        Self {
            important,
            inline: false,
            ids: specificity.ids,
            classes: specificity.classes,
            elements: specificity.elements,
            order,
        }
    }

    const fn inline(important: bool, order: usize) -> Self {
        Self {
            important,
            inline: true,
            ids: 0,
            classes: 0,
            elements: 0,
            order,
        }
    }
}

fn parse_style(
    mut style: Style,
    attrs: &HashMap<String, String>,
    tag: &str,
    stylesheet: &Stylesheet,
) -> Result<Style, SvgParseError> {
    // These SVG properties apply to an element and are not inherited.
    style.clip_path = None;
    style.mask = None;
    style.filter = None;
    let mut declarations = HashMap::<String, (String, CascadePriority)>::new();
    for (key, value) in attrs {
        if is_supported_style_property(key) {
            insert_cascaded_declaration(
                &mut declarations,
                key,
                value,
                CascadePriority::presentation(),
            );
        }
    }

    let mut matching_rules = Vec::new();
    if let Some(id) = attrs.get("id") {
        if let Some(rules) = stylesheet.by_id.get(id) {
            matching_rules.extend(rules.iter().copied());
        }
    }
    if let Some(classes) = attrs.get("class") {
        for class in classes.split_ascii_whitespace() {
            if let Some(rules) = stylesheet.by_class.get(class) {
                matching_rules.extend(rules.iter().copied());
            }
        }
    }
    if let Some(rules) = stylesheet.by_element.get(tag) {
        matching_rules.extend(rules.iter().copied());
    }
    for (rule_index, specificity) in matching_rules {
        let rule = &stylesheet.rules[rule_index];
        for declaration in &rule.declarations {
            insert_cascaded_declaration(
                &mut declarations,
                &declaration.property,
                &declaration.value,
                CascadePriority::stylesheet(declaration.important, specificity, declaration.order),
            );
        }
    }

    if let Some(inline) = attrs.get("style") {
        let mut order = stylesheet.next_order;
        for declaration in parse_css_declarations(inline, &mut order) {
            insert_cascaded_declaration(
                &mut declarations,
                &declaration.property,
                &declaration.value,
                CascadePriority::inline(declaration.important, declaration.order),
            );
        }
    }

    let value = |property: &str| declarations.get(property).map(|(value, _)| value.as_str());
    if let Some(value) = value("color") {
        if !value.trim().eq_ignore_ascii_case("currentColor") {
            style.color = parse_color(value)?.ok_or(SvgParseError::Unsupported("color: none"))?;
        }
    }
    if let Some(value) = value("fill") {
        style.fill_current_color = value.trim().eq_ignore_ascii_case("currentColor");
        if !style.fill_current_color {
            style.fill = parse_color(value)?;
        }
    }
    if let Some(value) = value("stroke") {
        style.stroke_current_color = value.trim().eq_ignore_ascii_case("currentColor");
        if !style.stroke_current_color {
            style.stroke = parse_color(value)?;
        }
    }
    if style.fill_current_color {
        style.fill = Some(style.color);
    }
    if style.stroke_current_color {
        style.stroke = Some(style.color);
    }
    if let Some(value) = value("stroke-width") {
        style.stroke_width = parse_length(value).ok_or(SvgParseError::InvalidNumber)?;
    }
    if let Some(value) = value("fill-rule") {
        style.fill_rule = match value {
            "nonzero" => SvgFillRule::NonZero,
            "evenodd" => SvgFillRule::EvenOdd,
            _ => return Err(SvgParseError::Unsupported("fill-rule")),
        };
    }
    if let Some(value) = value("opacity") {
        style.opacity *= parse_unit(value)?;
    }
    if let Some(value) = value("fill-opacity") {
        style.fill_opacity = parse_unit(value)?;
    }
    if let Some(value) = value("stroke-opacity") {
        style.stroke_opacity = parse_unit(value)?;
    }
    if value("display").is_some_and(|value| value == "none")
        || value("visibility").is_some_and(|value| value == "hidden")
    {
        style.visible = false;
    }
    if let Some(value) = value("stroke-linecap") {
        style.cap = match value {
            "butt" => SvgLineCap::Butt,
            "round" => SvgLineCap::Round,
            "square" => SvgLineCap::Square,
            _ => return Err(SvgParseError::Unsupported("stroke-linecap")),
        };
    }
    if let Some(value) = value("stroke-linejoin") {
        style.join = match value {
            "miter" => SvgLineJoin::Miter,
            "round" => SvgLineJoin::Round,
            "bevel" => SvgLineJoin::Bevel,
            _ => return Err(SvgParseError::Unsupported("stroke-linejoin")),
        };
    }
    if let Some(value) = value("clip-path") {
        style.clip_path = parse_local_reference(value)?;
    }
    if let Some(value) = value("mask") {
        style.mask = parse_local_reference(value)?;
    }
    if let Some(value) = value("filter") {
        style.filter = parse_local_reference(value)?;
    }
    Ok(style)
}

fn parse_local_reference(value: &str) -> Result<Option<Arc<str>>, SvgParseError> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let Some(reference) = value
        .strip_prefix("url(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Err(SvgParseError::Unsupported("resource reference syntax"));
    };
    let reference = reference.trim().trim_matches(['\'', '"']);
    let Some(id) = reference.strip_prefix('#').filter(|id| !id.is_empty()) else {
        return Err(SvgParseError::ExternalResource(reference.to_owned()));
    };
    Ok(Some(Arc::from(id)))
}

fn insert_cascaded_declaration(
    declarations: &mut HashMap<String, (String, CascadePriority)>,
    property: &str,
    value: &str,
    priority: CascadePriority,
) {
    let should_replace = declarations
        .get(property)
        .is_none_or(|(_, current_priority)| priority > *current_priority);
    if should_replace {
        declarations.insert(property.to_owned(), (value.to_owned(), priority));
    }
}

fn is_supported_style_property(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "fill"
            | "stroke"
            | "stroke-width"
            | "fill-rule"
            | "opacity"
            | "fill-opacity"
            | "stroke-opacity"
            | "display"
            | "visibility"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "clip-path"
            | "mask"
            | "filter"
    )
}

fn collect_stylesheet_rules(source: &str) -> Result<Stylesheet, SvgParseError> {
    let mut stylesheet = Stylesheet::default();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('<') {
        cursor += relative;
        if source[cursor..].starts_with("<!--") {
            let end = source[cursor + 4..]
                .find("-->")
                .ok_or(SvgParseError::InvalidXml("unterminated comment"))?;
            cursor += 4 + end + 3;
            continue;
        }
        if source[cursor..].starts_with("<?") {
            let end = source[cursor + 2..]
                .find("?>")
                .ok_or(SvgParseError::InvalidXml("unterminated declaration"))?;
            cursor += 2 + end + 2;
            continue;
        }
        let end = find_tag_end(source, cursor + 1)?;
        let mut body = source[cursor + 1..end].trim();
        cursor = end + 1;
        if body.starts_with('/') || body.starts_with('!') {
            continue;
        }
        let self_closing = body.ends_with('/');
        if self_closing {
            body = body[..body.len() - 1].trim_end();
        }
        let (tag, _) = parse_tag(body)?;
        if tag == "style" && !self_closing {
            let (style_start, close_end) = find_style_end(source, cursor)?;
            parse_stylesheet(&source[cursor..style_start], &mut stylesheet);
            cursor = close_end + 1;
        }
    }
    Ok(stylesheet)
}

fn find_style_end(source: &str, from: usize) -> Result<(usize, usize), SvgParseError> {
    let mut cursor = from;
    while let Some(relative) = source[cursor..].find("</style") {
        let close_start = cursor + relative;
        let name_end = close_start + "</style".len();
        if source
            .as_bytes()
            .get(name_end)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
        {
            let close_end = find_tag_end(source, close_start + 1)?;
            if source[close_start + 2..close_end].trim() == "style" {
                return Ok((close_start, close_end));
            }
        }
        cursor = name_end;
    }
    Err(SvgParseError::InvalidXml("unclosed tag"))
}

fn parse_stylesheet(source: &str, stylesheet: &mut Stylesheet) {
    let source = strip_css_comments(source)
        .replace("<!--", " ")
        .replace("-->", " ")
        .replace("<![CDATA[", " ")
        .replace("]]>", " ");
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'}')
        {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        let selector_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b'{' && bytes[cursor] != b'}' {
            cursor += 1;
        }
        if cursor == bytes.len() || bytes[cursor] == b'}' {
            continue;
        }
        let selector_text = source[selector_start..cursor].trim();
        let Some(block_end) = find_css_block_end(bytes, cursor) else {
            break;
        };
        if !selector_text.starts_with('@')
            && let Some(selectors) = parse_selector_list(selector_text)
        {
            let mut declarations =
                parse_css_declarations(&source[cursor + 1..block_end], &mut stylesheet.next_order);
            if !declarations.is_empty() {
                let rule_index = stylesheet.rules.len();
                for selector in selectors {
                    let entry = match &selector {
                        CssSelector::Id(id) => stylesheet.by_id.entry(id.clone()).or_default(),
                        CssSelector::Class(class) => {
                            stylesheet.by_class.entry(class.clone()).or_default()
                        }
                        CssSelector::Element(element) => {
                            stylesheet.by_element.entry(element.clone()).or_default()
                        }
                    };
                    entry.push((rule_index, selector.specificity()));
                }
                stylesheet.rules.push(CssRule {
                    declarations: std::mem::take(&mut declarations),
                });
            }
        }
        cursor = block_end + 1;
    }
}

fn find_css_block_end(bytes: &[u8], open_brace: usize) -> Option<usize> {
    let mut depth = 1usize;
    for (offset, byte) in bytes.iter().enumerate().skip(open_brace + 1) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_selector_list(source: &str) -> Option<Vec<CssSelector>> {
    source
        .split(',')
        .map(parse_css_selector)
        .collect::<Option<Vec<_>>>()
        .filter(|selectors| !selectors.is_empty())
}

fn parse_css_selector(source: &str) -> Option<CssSelector> {
    let selector = source.trim();
    if let Some(id) = selector.strip_prefix('#') {
        return is_css_identifier(id).then(|| CssSelector::Id(id.to_owned()));
    }
    if let Some(class) = selector.strip_prefix('.') {
        return is_css_identifier(class).then(|| CssSelector::Class(class.to_owned()));
    }
    matches!(
        selector,
        "svg" | "g" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
    )
    .then(|| CssSelector::Element(selector.to_owned()))
}

fn is_css_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let first_is_valid = first.is_ascii_alphabetic()
        || first == '_'
        || (first == '-'
            && chars
                .clone()
                .next()
                .is_some_and(|next| next.is_ascii_alphabetic() || next == '_' || next == '-'));
    first_is_valid
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

fn parse_css_declarations(source: &str, next_order: &mut usize) -> Vec<CssDeclaration> {
    let mut declarations = Vec::new();
    for source_declaration in source.split(';') {
        let order = *next_order;
        *next_order = next_order.saturating_add(1);
        let Some((property, value)) = source_declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        if !is_supported_style_property(&property) {
            continue;
        }
        let (value, important) = strip_important(value.trim());
        if value.is_empty() {
            continue;
        }
        declarations.push(CssDeclaration {
            property,
            value: value.to_owned(),
            important,
            order,
        });
    }
    declarations
}

fn strip_important(value: &str) -> (&str, bool) {
    const IMPORTANT: &str = "!important";
    if value.len() >= IMPORTANT.len()
        && value[value.len() - IMPORTANT.len()..].eq_ignore_ascii_case(IMPORTANT)
    {
        (value[..value.len() - IMPORTANT.len()].trim_end(), true)
    } else {
        (value, false)
    }
}

fn strip_css_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut remaining = source;
    while let Some(start) = remaining.find("/*") {
        result.push_str(&remaining[..start]);
        let comment_start = start + 2;
        let Some(relative_end) = remaining[comment_start..].find("*/") else {
            result.push(' ');
            return result;
        };
        result.push(' ');
        remaining = &remaining[comment_start + relative_end + 2..];
    }
    result.push_str(remaining);
    result
}

fn parse_color(value: &str) -> Result<Option<SvgColor>, SvgParseError> {
    let value = value.trim();
    if value == "none" {
        return Ok(None);
    }
    if value.starts_with("url(") {
        return Ok(None);
    }
    let color = if let Some(hex) = value.strip_prefix('#') {
        match hex.len() {
            3 => SvgColor::rgba8(
                expand(hex.as_bytes()[0])?,
                expand(hex.as_bytes()[1])?,
                expand(hex.as_bytes()[2])?,
                255,
            ),
            4 => SvgColor::rgba8(
                expand(hex.as_bytes()[0])?,
                expand(hex.as_bytes()[1])?,
                expand(hex.as_bytes()[2])?,
                expand(hex.as_bytes()[3])?,
            ),
            6 => SvgColor::rgba8(byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 255),
            8 => SvgColor::rgba8(
                byte(&hex[0..2])?,
                byte(&hex[2..4])?,
                byte(&hex[4..6])?,
                byte(&hex[6..8])?,
            ),
            _ => return Err(SvgParseError::Unsupported("color syntax")),
        }
    } else if let Some(args) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let values = parse_numbers(args);
        if values.len() != 3 {
            return Err(SvgParseError::InvalidNumber);
        }
        let channel = |v: f32| -> Result<u8, SvgParseError> {
            if !v.is_finite() {
                return Err(SvgParseError::InvalidNumber);
            }
            Ok(v.round().clamp(0.0, 255.0) as u8)
        };
        SvgColor::rgba8(
            channel(values[0])?,
            channel(values[1])?,
            channel(values[2])?,
            255,
        )
    } else {
        match value {
            "black" => SvgColor::rgba8(0, 0, 0, 255),
            "white" => SvgColor::rgba8(255, 255, 255, 255),
            "red" => SvgColor::rgba8(255, 0, 0, 255),
            "green" => SvgColor::rgba8(0, 128, 0, 255),
            "blue" => SvgColor::rgba8(0, 0, 255, 255),
            "yellow" => SvgColor::rgba8(255, 255, 0, 255),
            "transparent" => SvgColor::rgba8(0, 0, 0, 0),
            _ => return Err(SvgParseError::Unsupported("named color")),
        }
    };
    Ok(Some(color))
}

fn byte(value: &str) -> Result<u8, SvgParseError> {
    u8::from_str_radix(value, 16).map_err(|_| SvgParseError::InvalidNumber)
}
fn expand(value: u8) -> Result<u8, SvgParseError> {
    let digit = (value as char)
        .to_digit(16)
        .ok_or(SvgParseError::InvalidNumber)? as u8;
    Ok(digit * 17)
}
fn parse_unit(value: &str) -> Result<f32, SvgParseError> {
    let v = value
        .trim()
        .strip_suffix('%')
        .map(|v| v.parse::<f32>().map(|n| n / 100.0))
        .unwrap_or_else(|| value.trim().parse::<f32>())
        .map_err(|_| SvgParseError::InvalidNumber)?;
    if v.is_finite() {
        Ok(v.clamp(0.0, 1.0))
    } else {
        Err(SvgParseError::InvalidNumber)
    }
}
fn parse_length(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let end = trimmed
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')))
        .unwrap_or(trimmed.len());
    let unit = trimmed[end..].trim();
    if !unit.is_empty() && !matches!(unit, "px" | "pt" | "pc" | "mm" | "cm" | "in") {
        return None;
    }
    let n = trimmed[..end].parse::<f32>().ok()?;
    n.is_finite().then_some(n)
}
fn parse_viewbox(value: &str) -> Option<[f32; 4]> {
    let values = parse_numbers(value);
    (values.len() == 4
        && values.iter().all(|v| v.is_finite())
        && values[2] > 0.0
        && values[3] > 0.0)
        .then(|| [values[0], values[1], values[2], values[3]])
}

enum GradientBuilder {
    Linear {
        id: Arc<str>,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        units: SvgGradientUnits,
        transform: SvgTransform,
        spread: SvgSpreadMethod,
        stops: Vec<SvgGradientStop>,
    },
    Radial {
        id: Arc<str>,
        cx: f32,
        cy: f32,
        radius: f32,
        fx: f32,
        fy: f32,
        focal_radius: f32,
        units: SvgGradientUnits,
        transform: SvgTransform,
        spread: SvgSpreadMethod,
        stops: Vec<SvgGradientStop>,
    },
}

fn collect_gradients(source: &str) -> Result<Vec<SvgGradient>, SvgParseError> {
    let mut gradients = Vec::new();
    let mut current: Option<GradientBuilder> = None;
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find('<') {
        cursor += offset;
        if source[cursor..].starts_with("<!--") {
            let end = source[cursor + 4..]
                .find("-->")
                .ok_or(SvgParseError::InvalidXml("unterminated comment"))?;
            cursor += 4 + end + 3;
            continue;
        }
        let end = find_tag_end(source, cursor + 1)?;
        let body = source[cursor + 1..end].trim();
        cursor = end + 1;
        if body.starts_with('/') || body.starts_with('?') || body.starts_with('!') {
            if body.starts_with('/')
                && matches!(
                    body.trim_start_matches('/').trim(),
                    "linearGradient" | "radialGradient"
                )
            {
                if let Some(builder) = current.take() {
                    gradients.push(finish_gradient(builder)?);
                }
            }
            continue;
        }
        let self_closing = body.ends_with('/');
        let body = body.strip_suffix('/').unwrap_or(body).trim_end();
        let (tag, attrs) = parse_tag(body)?;
        if tag == "linearGradient" || tag == "radialGradient" {
            let id = attrs
                .get("id")
                .cloned()
                .ok_or(SvgParseError::InvalidXml("gradient id is required"))?;
            let units = if attrs
                .get("gradientUnits")
                .is_some_and(|v| v == "userSpaceOnUse")
            {
                SvgGradientUnits::UserSpaceOnUse
            } else {
                SvgGradientUnits::ObjectBoundingBox
            };
            let transform = attrs
                .get("gradientTransform")
                .map(|v| parse_transform(v))
                .transpose()?
                .unwrap_or_default();
            let spread = match attrs
                .get("spreadMethod")
                .map(String::as_str)
                .unwrap_or("pad")
            {
                "pad" => SvgSpreadMethod::Pad,
                "reflect" => SvgSpreadMethod::Reflect,
                "repeat" => SvgSpreadMethod::Repeat,
                _ => return Err(SvgParseError::Unsupported("gradient spread method")),
            };
            current = Some(if tag == "linearGradient" {
                GradientBuilder::Linear {
                    id: Arc::from(id),
                    x1: gradient_number(&attrs, "x1", 0.0)?,
                    y1: gradient_number(&attrs, "y1", 0.0)?,
                    x2: gradient_number(&attrs, "x2", 1.0)?,
                    y2: gradient_number(&attrs, "y2", 0.0)?,
                    units,
                    transform,
                    spread,
                    stops: Vec::new(),
                }
            } else {
                let cx = gradient_number(&attrs, "cx", 0.5)?;
                let cy = gradient_number(&attrs, "cy", 0.5)?;
                GradientBuilder::Radial {
                    id: Arc::from(id),
                    cx,
                    cy,
                    radius: gradient_number(&attrs, "r", 0.5)?,
                    fx: gradient_number(&attrs, "fx", cx)?,
                    fy: gradient_number(&attrs, "fy", cy)?,
                    focal_radius: gradient_number(&attrs, "fr", 0.0)?,
                    units,
                    transform,
                    spread,
                    stops: Vec::new(),
                }
            });
            if self_closing {
                if let Some(builder) = current.take() {
                    gradients.push(finish_gradient(builder)?);
                }
            }
            continue;
        }
        if tag == "stop" {
            if let Some(builder) = current.as_mut() {
                let offset = attrs
                    .get("offset")
                    .map(|v| gradient_offset(v))
                    .transpose()?
                    .unwrap_or(0.0);
                let mut color = attrs
                    .get("stop-color")
                    .map(|v| parse_color(v))
                    .transpose()?
                    .flatten()
                    .unwrap_or(SvgColor::rgba8(0, 0, 0, 255));
                if let Some(opacity) = attrs.get("stop-opacity") {
                    color.a *= parse_unit(opacity)?;
                }
                let stops = match builder {
                    GradientBuilder::Linear { stops, .. }
                    | GradientBuilder::Radial { stops, .. } => stops,
                };
                stops.push(SvgGradientStop {
                    offset: offset.clamp(0.0, 1.0),
                    color,
                });
            }
        }
    }
    if let Some(builder) = current {
        gradients.push(finish_gradient(builder)?);
    }
    Ok(gradients)
}

fn finish_gradient(builder: GradientBuilder) -> Result<SvgGradient, SvgParseError> {
    let gradient = match builder {
        GradientBuilder::Linear {
            id,
            x1,
            y1,
            x2,
            y2,
            units,
            transform,
            spread,
            stops,
        } => SvgGradient::Linear {
            id,
            x1,
            y1,
            x2,
            y2,
            units,
            transform,
            spread,
            stops: normalize_gradient_stops(stops),
        },
        GradientBuilder::Radial {
            id,
            cx,
            cy,
            radius,
            fx,
            fy,
            focal_radius,
            units,
            transform,
            spread,
            stops,
        } => SvgGradient::Radial {
            id,
            cx,
            cy,
            radius,
            fx,
            fy,
            focal_radius,
            units,
            transform,
            spread,
            stops: normalize_gradient_stops(stops),
        },
    };
    if gradient.is_finite() {
        Ok(gradient)
    } else {
        Err(SvgParseError::InvalidNumber)
    }
}

fn normalize_gradient_stops(stops: Vec<SvgGradientStop>) -> Arc<[SvgGradientStop]> {
    let mut previous = 0.0_f32;
    stops
        .into_iter()
        .map(|mut stop| {
            stop.offset = stop.offset.clamp(previous, 1.0);
            previous = stop.offset;
            stop
        })
        .collect::<Vec<_>>()
        .into()
}

fn gradient_number(
    attrs: &HashMap<String, String>,
    name: &str,
    default: f32,
) -> Result<f32, SvgParseError> {
    let Some(value) = attrs.get(name) else {
        return Ok(default);
    };
    let value = value.trim();
    let (number, percent) = value
        .strip_suffix('%')
        .map_or((value, false), |v| (v, true));
    let number = number
        .parse::<f32>()
        .map_err(|_| SvgParseError::InvalidNumber)?;
    let number = if percent { number / 100.0 } else { number };
    number
        .is_finite()
        .then_some(number)
        .ok_or(SvgParseError::InvalidNumber)
}
fn gradient_offset(value: &str) -> Result<f32, SvgParseError> {
    let (value, percent) = value
        .trim()
        .strip_suffix('%')
        .map_or((value.trim(), false), |v| (v, true));
    let number = value
        .parse::<f32>()
        .map_err(|_| SvgParseError::InvalidNumber)?;
    let number = if percent { number / 100.0 } else { number };
    number
        .is_finite()
        .then_some(number)
        .ok_or(SvgParseError::InvalidNumber)
}

fn collect_node_paints(
    source: &str,
    nodes: &[SvgNode],
    gradients: &[SvgGradient],
    stylesheet: &Stylesheet,
    use_targets: &HashMap<SvgNodeId, SvgNodeId>,
) -> Result<HashMap<SvgNodeId, SvgParsedPaints>, SvgParseError> {
    let mut authored = HashMap::new();
    let mut unnamed = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find('<') {
        cursor += offset;
        let end = find_tag_end(source, cursor + 1)?;
        let body = source[cursor + 1..end].trim();
        cursor = end + 1;
        if body.starts_with('/') || body.starts_with('!') || body.starts_with('?') {
            continue;
        }
        let body = body.strip_suffix('/').unwrap_or(body).trim_end();
        let (tag, attrs) = parse_tag(body)?;
        if !matches!(
            tag,
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
        ) {
            continue;
        }
        let fill = style_paint(&attrs, tag, "fill", gradients, stylesheet)?;
        let stroke = style_paint(&attrs, tag, "stroke", gradients, stylesheet)?;
        let paints = (fill, stroke);
        if let Some(id) = attrs.get("id") {
            authored.insert(id.clone(), paints);
        } else {
            unnamed.push(paints);
        }
    }

    let mut unnamed = unnamed.into_iter();
    let mut result = HashMap::new();
    for node in nodes {
        if node.element != SvgElementKind::Path || use_targets.contains_key(&node.node_id) {
            continue;
        }
        let paints = node
            .svg_id
            .as_deref()
            .and_then(|id| authored.get(id).cloned())
            .or_else(|| unnamed.next());
        let fallback_fill = node.fill.as_ref().map(|fill| SvgPaint::Solid(fill.color));
        let fallback_stroke = node
            .stroke
            .as_ref()
            .map(|stroke| SvgPaint::Solid(stroke.color));
        let (fill, stroke) = paints.unwrap_or((None, None));
        result.insert(
            node.node_id,
            SvgParsedPaints {
                // Solid node paints already include inherited paint opacity.
                fill: match fill {
                    Some(SvgPaint::Solid(_)) => fallback_fill,
                    other => other.or(fallback_fill),
                },
                stroke: match stroke {
                    Some(SvgPaint::Solid(_)) => fallback_stroke,
                    other => other.or(fallback_stroke),
                },
            },
        );
    }
    Ok(result)
}

fn resolve_use_references(
    references: &[UseReference],
    node_ids: &HashMap<String, SvgNodeId>,
    id_attributes: &HashMap<String, (String, HashMap<String, String>)>,
    stylesheet: &Stylesheet,
    root_transform: SvgTransform,
    nodes: &mut [SvgNode],
    unsupported_elements: &mut Vec<Arc<str>>,
) -> HashMap<SvgNodeId, SvgNodeId> {
    let pending_by_node = references
        .iter()
        .enumerate()
        .map(|(index, reference)| (reference.node, index))
        .collect::<HashMap<_, _>>();
    let mut waiting_on = HashMap::<SvgNodeId, Vec<usize>>::new();
    let mut ready = Vec::new();
    for (index, reference) in references.iter().enumerate() {
        let Some(target) = node_ids.get(&reference.target_id).copied() else {
            continue;
        };
        if nodes[target.0 as usize].geometry.is_some() {
            ready.push(index);
        } else if pending_by_node.contains_key(&target) {
            waiting_on.entry(target).or_default().push(index);
        }
    }

    let mut use_targets = HashMap::new();
    while let Some(index) = ready.pop() {
        let reference = &references[index];
        let Some(target_id) = node_ids.get(&reference.target_id).copied() else {
            continue;
        };
        let target = nodes[target_id.0 as usize].clone();
        let Some(geometry) = target.geometry else {
            continue;
        };
        let parent_transform = target
            .parent
            .and_then(|parent| nodes.get(parent.0 as usize))
            .map_or(root_transform, |parent| parent.transform);
        let Some(parent_inverse) = parent_transform.inverse() else {
            continue;
        };
        let local_transform = parent_inverse.mul(target.transform);
        let target_style = id_attributes.get(&reference.target_id);
        let target_has_property = |property| {
            target_style.is_some_and(|(tag, attrs)| {
                computed_style_value(attrs, tag, property, stylesheet).is_some()
            })
        };
        let use_node = &mut nodes[reference.node.0 as usize];
        use_node.element = target.element;
        use_node.geometry = Some(geometry);
        use_node.transform = use_node.transform.mul(local_transform);
        use_node.fill_rule = if target_has_property("fill-rule") {
            target.fill_rule
        } else {
            reference.style.fill_rule
        };
        use_node.fill = if target_has_property("fill") {
            target.fill.clone()
        } else {
            reference.style.fill_color().map(|color| SvgFill {
                color,
                rule: use_node.fill_rule,
            })
        };
        use_node.stroke = if target_has_property("stroke")
            || target_has_property("stroke-width")
            || target_has_property("stroke-linecap")
            || target_has_property("stroke-linejoin")
        {
            target.stroke.clone()
        } else {
            reference.style.stroke_color().map(|color| SvgStroke {
                color,
                width: reference.style.stroke_width,
                line_cap: reference.style.cap,
                line_join: reference.style.join,
                miter_limit: 4.0,
                dash_array: Arc::from([]),
                dash_offset: 0.0,
            })
        };
        use_node.opacity *= target.opacity;
        use_node.visible &= target.visible;
        use_node.clip_path = target
            .clip_path
            .clone()
            .or_else(|| use_node.clip_path.clone());
        use_node.mask = target.mask.clone().or_else(|| use_node.mask.clone());
        use_node.filter = target.filter.clone().or_else(|| use_node.filter.clone());
        use_node.paint_order = target.paint_order;

        let paint_source = use_targets.get(&target_id).copied().unwrap_or(target_id);
        use_targets.insert(reference.node, paint_source);
        if let Some(waiting) = waiting_on.remove(&reference.node) {
            ready.extend(waiting);
        }
    }

    for reference in references {
        if !use_targets.contains_key(&reference.node) {
            unsupported_elements.push(Arc::from("use"));
        }
    }
    use_targets
}

fn style_paint(
    attrs: &HashMap<String, String>,
    tag: &str,
    name: &str,
    gradients: &[SvgGradient],
    stylesheet: &Stylesheet,
) -> Result<Option<SvgPaint>, SvgParseError> {
    let Some(value) = computed_style_value(attrs, tag, name, stylesheet) else {
        return Ok(None);
    };
    let value = value.trim();
    // The retained node already resolved this inherited paint against `color`.
    // Returning no authored override lets collect_node_paints use that value.
    if value.trim().eq_ignore_ascii_case("currentColor") {
        return Ok(None);
    }
    if let Some(reference) = value
        .strip_prefix("url(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let reference = reference.trim().trim_matches(['\'', '"']);
        let Some(id) = reference.strip_prefix('#').filter(|id| !id.is_empty()) else {
            return Err(SvgParseError::ExternalResource(reference.to_owned()));
        };
        let paint = gradients
            .iter()
            .find(|gradient| gradient.id() == id)
            .map(|gradient| match gradient {
                SvgGradient::Linear { .. } => SvgPaint::Linear(gradient.clone()),
                SvgGradient::Radial { .. } => SvgPaint::Radial(gradient.clone()),
            })
            .unwrap_or_else(|| SvgPaint::Pattern { id: Arc::from(id) });
        return Ok(Some(paint));
    }
    Ok(parse_color(value)?.map(SvgPaint::Solid))
}

fn computed_style_value(
    attrs: &HashMap<String, String>,
    tag: &str,
    property: &str,
    stylesheet: &Stylesheet,
) -> Option<String> {
    let mut declarations = HashMap::<String, (String, CascadePriority)>::new();
    for (key, value) in attrs {
        if key == property {
            insert_cascaded_declaration(
                &mut declarations,
                key,
                value,
                CascadePriority::presentation(),
            );
        }
    }
    let mut rules = Vec::new();
    if let Some(id) = attrs.get("id") {
        if let Some(entries) = stylesheet.by_id.get(id) {
            rules.extend(entries.iter().copied());
        }
    }
    if let Some(classes) = attrs.get("class") {
        for class in classes.split_ascii_whitespace() {
            if let Some(entries) = stylesheet.by_class.get(class) {
                rules.extend(entries.iter().copied());
            }
        }
    }
    if let Some(entries) = stylesheet.by_element.get(tag) {
        rules.extend(entries.iter().copied());
    }
    for (rule_index, specificity) in rules {
        for declaration in &stylesheet.rules[rule_index].declarations {
            if declaration.property == property {
                insert_cascaded_declaration(
                    &mut declarations,
                    property,
                    &declaration.value,
                    CascadePriority::stylesheet(
                        declaration.important,
                        specificity,
                        declaration.order,
                    ),
                );
            }
        }
    }
    if let Some(inline) = attrs.get("style") {
        let mut order = stylesheet.next_order;
        for declaration in parse_css_declarations(inline, &mut order) {
            if declaration.property == property {
                insert_cascaded_declaration(
                    &mut declarations,
                    property,
                    &declaration.value,
                    CascadePriority::inline(declaration.important, declaration.order),
                );
            }
        }
    }
    declarations.get(property).map(|(value, _)| value.clone())
}

fn collect_resources(
    source: &str,
    nodes: &[SvgNode],
    gradients: &[SvgGradient],
    root_transform: SvgTransform,
) -> Result<SvgResourceGraph, SvgParseError> {
    struct FilterBuilder {
        id: Arc<str>,
        units: SvgResourceUnits,
        primitive_units: SvgResourceUnits,
        region: [f32; 4],
        primitives: Vec<SvgFilterPrimitive>,
    }
    fn finish_filter(builder: FilterBuilder) -> SvgFilter {
        SvgFilter {
            id: builder.id,
            units: builder.units,
            primitive_units: builder.primitive_units,
            region: builder.region,
            primitives: builder.primitives.into(),
        }
    }

    let mut graph = SvgResourceGraph {
        root_transform,
        gradients: Arc::from(gradients.to_vec()),
        ..SvgResourceGraph::default()
    };
    let mut clip_paths = Vec::new();
    let mut masks = Vec::new();
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    let mut filter: Option<FilterBuilder> = None;
    let mut merge_inputs: Option<Vec<SvgFilterInput>> = None;
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find('<') {
        cursor += offset;
        if source[cursor..].starts_with("<!--") {
            let end = source[cursor + 4..]
                .find("-->")
                .ok_or(SvgParseError::InvalidXml("unterminated comment"))?;
            cursor += 4 + end + 3;
            continue;
        }
        let end = find_tag_end(source, cursor + 1)?;
        let mut body = source[cursor + 1..end].trim();
        cursor = end + 1;
        if let Some(name) = body.strip_prefix('/') {
            match name.trim() {
                "filter" => {
                    if let Some(builder) = filter.take() {
                        filters.push(finish_filter(builder));
                    }
                }
                "feMerge" => {
                    if let (Some(builder), Some(inputs)) = (filter.as_mut(), merge_inputs.take()) {
                        builder.primitives.push(SvgFilterPrimitive::Merge {
                            inputs: inputs.into(),
                        });
                    }
                }
                _ => {}
            }
            continue;
        }
        if body.starts_with('!') || body.starts_with('?') {
            continue;
        }
        let self_closing = body.ends_with('/');
        if self_closing {
            body = body[..body.len() - 1].trim_end();
        }
        let (tag, attrs) = parse_tag(body)?;
        match tag {
            "clipPath" => {
                let id = resource_id(&attrs)?;
                clip_paths.push(SvgClipPath {
                    id: id.clone(),
                    units: resource_units(
                        &attrs,
                        "clipPathUnits",
                        SvgResourceUnits::UserSpaceOnUse,
                    )?,
                    transform: attrs
                        .get("transform")
                        .map(|value| parse_transform(value))
                        .transpose()?
                        .unwrap_or_default(),
                    nodes: definition_nodes(nodes, &id),
                });
            }
            "mask" => {
                let id = resource_id(&attrs)?;
                let mask_type = match attrs
                    .get("mask-type")
                    .map(String::as_str)
                    .or_else(|| {
                        attrs
                            .get("style")
                            .and_then(|value| style_property(value, "mask-type"))
                    })
                    .unwrap_or("luminance")
                {
                    "alpha" => SvgMaskType::Alpha,
                    "luminance" => SvgMaskType::Luminance,
                    _ => return Err(SvgParseError::Unsupported("mask-type")),
                };
                masks.push(SvgMask {
                    id: id.clone(),
                    units: resource_units(
                        &attrs,
                        "maskUnits",
                        SvgResourceUnits::ObjectBoundingBox,
                    )?,
                    content_units: resource_units(
                        &attrs,
                        "maskContentUnits",
                        SvgResourceUnits::UserSpaceOnUse,
                    )?,
                    mask_type,
                    region: [
                        resource_number(&attrs, "x", -0.1)?,
                        resource_number(&attrs, "y", -0.1)?,
                        resource_number(&attrs, "width", 1.2)?,
                        resource_number(&attrs, "height", 1.2)?,
                    ],
                    nodes: definition_nodes(nodes, &id),
                });
            }
            "pattern" => {
                let id = resource_id(&attrs)?;
                patterns.push(SvgPattern {
                    id: id.clone(),
                    units: resource_units(
                        &attrs,
                        "patternUnits",
                        SvgResourceUnits::ObjectBoundingBox,
                    )?,
                    content_units: resource_units(
                        &attrs,
                        "patternContentUnits",
                        SvgResourceUnits::UserSpaceOnUse,
                    )?,
                    tile: [
                        resource_number(&attrs, "x", 0.0)?,
                        resource_number(&attrs, "y", 0.0)?,
                        resource_number(&attrs, "width", 0.0)?,
                        resource_number(&attrs, "height", 0.0)?,
                    ],
                    transform: attrs
                        .get("patternTransform")
                        .map(|value| parse_transform(value))
                        .transpose()?
                        .unwrap_or_default(),
                    nodes: definition_nodes(nodes, &id),
                });
            }
            "filter" => {
                let id = resource_id(&attrs)?;
                let builder = FilterBuilder {
                    id,
                    units: resource_units(
                        &attrs,
                        "filterUnits",
                        SvgResourceUnits::ObjectBoundingBox,
                    )?,
                    primitive_units: resource_units(
                        &attrs,
                        "primitiveUnits",
                        SvgResourceUnits::UserSpaceOnUse,
                    )?,
                    region: [
                        resource_number(&attrs, "x", -0.1)?,
                        resource_number(&attrs, "y", -0.1)?,
                        resource_number(&attrs, "width", 1.2)?,
                        resource_number(&attrs, "height", 1.2)?,
                    ],
                    primitives: Vec::new(),
                };
                if self_closing {
                    filters.push(finish_filter(builder));
                } else {
                    filter = Some(builder);
                }
            }
            "feMerge" if filter.is_some() => {
                merge_inputs = Some(Vec::new());
            }
            "feMergeNode" if merge_inputs.is_some() => {
                if let Some(inputs) = merge_inputs.as_mut() {
                    inputs.push(filter_input(
                        attrs.get("in").map(String::as_str),
                        SvgFilterInput::SourceGraphic,
                    ));
                }
            }
            name if name.starts_with("fe") && filter.is_some() => {
                if let Some(builder) = filter.as_mut() {
                    let default_input = if builder.primitives.is_empty() {
                        SvgFilterInput::SourceGraphic
                    } else {
                        SvgFilterInput::Previous
                    };
                    let primitive = parse_filter_primitive(name, &attrs, default_input)?;
                    builder.primitives.push(primitive);
                }
            }
            _ => {}
        }
    }
    if let Some(builder) = filter {
        filters.push(finish_filter(builder));
    }
    graph.clip_paths = clip_paths.into();
    graph.masks = masks.into();
    graph.patterns = patterns.into();
    graph.filters = filters.into();
    Ok(graph)
}

fn resource_id(attrs: &HashMap<String, String>) -> Result<Arc<str>, SvgParseError> {
    attrs
        .get("id")
        .filter(|id| !id.trim().is_empty())
        .map(|id| Arc::from(id.as_str()))
        .ok_or(SvgParseError::InvalidXml("resource id is required"))
}

fn resource_units(
    attrs: &HashMap<String, String>,
    name: &str,
    default: SvgResourceUnits,
) -> Result<SvgResourceUnits, SvgParseError> {
    SvgResourceUnits::from_attribute(attrs.get(name).map(String::as_str), default)
        .ok_or(SvgParseError::Unsupported("resource coordinate units"))
}

fn resource_number(
    attrs: &HashMap<String, String>,
    name: &str,
    default: f32,
) -> Result<f32, SvgParseError> {
    let Some(value) = attrs.get(name) else {
        return Ok(default);
    };
    let value = value.trim();
    let (number, percent) = value
        .strip_suffix('%')
        .map_or((value, false), |value| (value, true));
    let number = number
        .parse::<f32>()
        .map_err(|_| SvgParseError::InvalidNumber)?;
    let number = if percent { number / 100.0 } else { number };
    number
        .is_finite()
        .then_some(number)
        .ok_or(SvgParseError::InvalidNumber)
}

fn definition_nodes(nodes: &[SvgNode], id: &str) -> Arc<[u32]> {
    nodes
        .iter()
        .filter(|node| node.definition_owner.as_deref() == Some(id))
        .map(|node| node.node_id.0)
        .collect::<Vec<_>>()
        .into()
}

fn style_property<'a>(style: &'a str, wanted: &str) -> Option<&'a str> {
    style.split(';').find_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        property
            .trim()
            .eq_ignore_ascii_case(wanted)
            .then_some(value.trim())
    })
}

fn filter_input(value: Option<&str>, default: SvgFilterInput) -> SvgFilterInput {
    match value.unwrap_or(match &default {
        SvgFilterInput::SourceGraphic => "SourceGraphic",
        SvgFilterInput::SourceAlpha => "SourceAlpha",
        SvgFilterInput::Previous => "previous",
        SvgFilterInput::Named(name) => name,
    }) {
        "SourceGraphic" => SvgFilterInput::SourceGraphic,
        "SourceAlpha" => SvgFilterInput::SourceAlpha,
        "previous" => SvgFilterInput::Previous,
        name => SvgFilterInput::Named(Arc::from(name)),
    }
}

fn parse_filter_primitive(
    name: &str,
    attrs: &HashMap<String, String>,
    default_input: SvgFilterInput,
) -> Result<SvgFilterPrimitive, SvgParseError> {
    let input = || filter_input(attrs.get("in").map(String::as_str), default_input.clone());
    let input2 = || {
        filter_input(
            attrs.get("in2").map(String::as_str),
            SvgFilterInput::SourceGraphic,
        )
    };
    match name {
        "feGaussianBlur" => {
            let deviations = attrs
                .get("stdDeviation")
                .map(|value| parse_numbers(value))
                .unwrap_or_else(|| vec![0.0]);
            if !(1..=2).contains(&deviations.len())
                || deviations
                    .iter()
                    .any(|value| !value.is_finite() || *value < 0.0)
            {
                return Err(SvgParseError::InvalidNumber);
            }
            Ok(SvgFilterPrimitive::GaussianBlur {
                input: input(),
                deviation: [deviations[0], *deviations.get(1).unwrap_or(&deviations[0])],
            })
        }
        "feOffset" => Ok(SvgFilterPrimitive::Offset {
            input: input(),
            dx: resource_number(attrs, "dx", 0.0)?,
            dy: resource_number(attrs, "dy", 0.0)?,
        }),
        "feFlood" => {
            let mut color = attrs
                .get("flood-color")
                .map(|value| parse_color(value))
                .transpose()?
                .flatten()
                .unwrap_or(SvgColor::rgba8(0, 0, 0, 255));
            if let Some(opacity) = attrs.get("flood-opacity").map(String::as_str).or_else(|| {
                attrs
                    .get("style")
                    .and_then(|style| style_property(style, "flood-opacity"))
            }) {
                color.a *= parse_unit(opacity)?;
            }
            Ok(SvgFilterPrimitive::Flood { color })
        }
        "feComposite" => {
            let operator = match attrs.get("operator").map(String::as_str).unwrap_or("over") {
                "over" => SvgCompositeOperator::Over,
                "in" => SvgCompositeOperator::In,
                "out" => SvgCompositeOperator::Out,
                "atop" => SvgCompositeOperator::Atop,
                "xor" => SvgCompositeOperator::Xor,
                "lighter" => SvgCompositeOperator::Lighter,
                "arithmetic" => SvgCompositeOperator::Arithmetic,
                _ => {
                    return Ok(SvgFilterPrimitive::Unsupported(Arc::from(
                        "feComposite operator",
                    )));
                }
            };
            let mut coefficients = [0.0; 4];
            for (index, name) in ["k1", "k2", "k3", "k4"].into_iter().enumerate() {
                coefficients[index] = resource_number(attrs, name, 0.0)?;
            }
            Ok(SvgFilterPrimitive::Composite {
                input: input(),
                input2: input2(),
                operator,
                coefficients,
            })
        }
        "feBlend" => {
            let mode = match attrs.get("mode").map(String::as_str).unwrap_or("normal") {
                "normal" => SvgBlendMode::Normal,
                "multiply" => SvgBlendMode::Multiply,
                "screen" => SvgBlendMode::Screen,
                "darken" => SvgBlendMode::Darken,
                "lighten" => SvgBlendMode::Lighten,
                _ => return Ok(SvgFilterPrimitive::Unsupported(Arc::from("feBlend mode"))),
            };
            Ok(SvgFilterPrimitive::Blend {
                input: input(),
                input2: input2(),
                mode,
            })
        }
        "feColorMatrix" => {
            let matrix_type = attrs.get("type").map(String::as_str).unwrap_or("matrix");
            let values = attrs
                .get("values")
                .map(|value| parse_numbers(value))
                .unwrap_or_default();
            let matrix = match matrix_type {
                "matrix" if values.len() == 20 && values.iter().all(|value| value.is_finite()) => {
                    SvgColorMatrix::Matrix(values.into())
                }
                "saturate" => SvgColorMatrix::Saturate(values.first().copied().unwrap_or(1.0)),
                "hueRotate" => SvgColorMatrix::HueRotate(values.first().copied().unwrap_or(0.0)),
                "luminanceToAlpha" => SvgColorMatrix::LuminanceToAlpha,
                _ => return Err(SvgParseError::InvalidNumber),
            };
            Ok(SvgFilterPrimitive::ColorMatrix {
                input: input(),
                matrix,
            })
        }
        // `feMerge` is assembled from its child `feMergeNode` tags in the scanner.
        "feMerge" | "feMergeNode" => Ok(SvgFilterPrimitive::Unsupported(Arc::from(name))),
        _ => Ok(SvgFilterPrimitive::Unsupported(Arc::from(name))),
    }
}

fn parse_transform(value: &str) -> Result<SvgTransform, SvgParseError> {
    let mut result = SvgTransform::default();
    for part in value.split(')').filter(|s| !s.trim().is_empty()) {
        let (name, args) = part.split_once('(').ok_or(SvgParseError::InvalidNumber)?;
        let values = parse_numbers(args);
        let current = match name.trim() {
            "matrix" if values.len() == 6 => SvgTransform {
                sx: values[0],
                ky: values[1],
                kx: values[2],
                sy: values[3],
                tx: values[4],
                ty: values[5],
            },
            "translate" if (1..=2).contains(&values.len()) => SvgTransform {
                tx: values[0],
                ty: *values.get(1).unwrap_or(&0.0),
                ..SvgTransform::default()
            },
            "scale" if (1..=2).contains(&values.len()) => SvgTransform {
                sx: values[0],
                sy: *values.get(1).unwrap_or(&values[0]),
                ..SvgTransform::default()
            },
            "rotate" if (1..=3).contains(&values.len()) => {
                let a = values[0].to_radians();
                let rotation = SvgTransform {
                    sx: a.cos(),
                    ky: a.sin(),
                    kx: -a.sin(),
                    sy: a.cos(),
                    ..SvgTransform::default()
                };
                if values.len() == 3 {
                    SvgTransform {
                        tx: values[1],
                        ty: values[2],
                        ..SvgTransform::default()
                    }
                    .mul(rotation)
                    .mul(SvgTransform {
                        tx: -values[1],
                        ty: -values[2],
                        ..SvgTransform::default()
                    })
                } else {
                    rotation
                }
            }
            "skewX" if values.len() == 1 => SvgTransform {
                kx: values[0].to_radians().tan(),
                ..SvgTransform::default()
            },
            "skewY" if values.len() == 1 => SvgTransform {
                ky: values[0].to_radians().tan(),
                ..SvgTransform::default()
            },
            _ => return Err(SvgParseError::Unsupported("transform")),
        };
        if !current.is_finite() {
            return Err(SvgParseError::InvalidNumber);
        }
        result = result.mul(current);
    }
    Ok(result)
}


fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> Result<Vec<SvgPathCommand>, SvgParseError> {
    if rx == 0.0 || ry == 0.0 {
        return Ok(Vec::new());
    }
    let k = 0.552_284_8;
    Ok(vec![
        m(cx + rx, cy),
        c(cx + rx, cy + k * ry, cx + k * rx, cy + ry, cx, cy + ry),
        c(cx - k * rx, cy + ry, cx - rx, cy + k * ry, cx - rx, cy),
        c(cx - rx, cy - k * ry, cx - k * rx, cy - ry, cx, cy - ry),
        c(cx + k * rx, cy - ry, cx + rx, cy - k * ry, cx + rx, cy),
        SvgPathCommand::Close,
    ])
}

fn parse_path(data: &str) -> Result<Vec<SvgPathCommand>, SvgParseError> {
    let mut scan = Numbers::new(data);
    let mut out = Vec::new();
    let mut command = ' ';
    let mut current = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let mut last_cubic = None;
    let mut last_quad = None;
    while scan.skip_separators() {
        if let Some(c) = scan.peek_char().filter(|c| c.is_ascii_alphabetic()) {
            command = c;
            scan.bump();
        } else if command == ' ' {
            return Err(SvgParseError::InvalidPath);
        }
        let relative = command.is_ascii_lowercase();
        let upper = command.to_ascii_uppercase();
        match upper {
            'Z' => {
                out.push(SvgPathCommand::Close);
                current = start;
                last_cubic = None;
                last_quad = None;
                command = ' ';
            }
            'M' | 'L' => {
                let (mut x, mut y) = (scan.number()?, scan.number()?);
                if relative {
                    x += current.0;
                    y += current.1;
                }
                if upper == 'M' {
                    out.push(m(x, y));
                    start = (x, y);
                    command = if relative { 'l' } else { 'L' };
                } else {
                    out.push(l(x, y));
                }
                current = (x, y);
                last_cubic = None;
                last_quad = None;
            }
            'H' => {
                let mut x = scan.number()?;
                if relative {
                    x += current.0;
                }
                current.0 = x;
                out.push(l(x, current.1));
                last_cubic = None;
                last_quad = None;
            }
            'V' => {
                let mut y = scan.number()?;
                if relative {
                    y += current.1;
                }
                current.1 = y;
                out.push(l(current.0, y));
                last_cubic = None;
                last_quad = None;
            }
            'C' => {
                let (mut x1, mut y1, mut x2, mut y2, mut x, mut y) = (
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                );
                if relative {
                    x1 += current.0;
                    y1 += current.1;
                    x2 += current.0;
                    y2 += current.1;
                    x += current.0;
                    y += current.1;
                }
                out.push(c(x1, y1, x2, y2, x, y));
                current = (x, y);
                last_cubic = Some((x2, y2));
                last_quad = None;
            }
            'S' => {
                let (mut x2, mut y2, mut x, mut y) = (
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                );
                if relative {
                    x2 += current.0;
                    y2 += current.1;
                    x += current.0;
                    y += current.1;
                }
                let (x1, y1) = last_cubic
                    .map(|p| (2.0 * current.0 - p.0, 2.0 * current.1 - p.1))
                    .unwrap_or(current);
                out.push(c(x1, y1, x2, y2, x, y));
                current = (x, y);
                last_cubic = Some((x2, y2));
                last_quad = None;
            }
            'Q' => {
                let (mut x1, mut y1, mut x, mut y) = (
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                    scan.number()?,
                );
                if relative {
                    x1 += current.0;
                    y1 += current.1;
                    x += current.0;
                    y += current.1;
                }
                out.push(q(x1, y1, x, y));
                current = (x, y);
                last_quad = Some((x1, y1));
                last_cubic = None;
            }
            'T' => {
                let (mut x, mut y) = (scan.number()?, scan.number()?);
                if relative {
                    x += current.0;
                    y += current.1;
                }
                let (x1, y1) = last_quad
                    .map(|p| (2.0 * current.0 - p.0, 2.0 * current.1 - p.1))
                    .unwrap_or(current);
                out.push(q(x1, y1, x, y));
                current = (x, y);
                last_quad = Some((x1, y1));
                last_cubic = None;
            }
            'A' => {
                let (rx, ry, rotation) = (
                    scan.number()?.abs(),
                    scan.number()?.abs(),
                    scan.number()?.to_radians(),
                );
                let large = scan.flag()?;
                let sweep = scan.flag()?;
                let (mut x, mut y) = (scan.number()?, scan.number()?);
                if relative {
                    x += current.0;
                    y += current.1;
                }
                if rx == 0.0 || ry == 0.0 {
                    out.push(l(x, y));
                } else {
                    out.extend(arc_to_cubics(
                        current,
                        (x, y),
                        rx,
                        ry,
                        rotation,
                        large,
                        sweep,
                    )?);
                }
                current = (x, y);
                last_cubic = None;
                last_quad = None;
            }
            _ => return Err(SvgParseError::InvalidPath),
        }
        if out.len() > MAX_COMMANDS {
            return Err(SvgParseError::LimitExceeded("path command"));
        }
    }
    if out.is_empty() {
        Err(SvgParseError::InvalidPath)
    } else {
        Ok(out)
    }
}

fn arc_to_cubics(
    start: (f32, f32),
    end: (f32, f32),
    mut rx: f32,
    mut ry: f32,
    phi: f32,
    large: bool,
    sweep: bool,
) -> Result<Vec<SvgPathCommand>, SvgParseError> {
    if start == end {
        return Ok(Vec::new());
    }
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx = (start.0 - end.0) * 0.5;
    let dy = (start.1 - end.1) * 0.5;
    let xp = cos_phi * dx + sin_phi * dy;
    let yp = -sin_phi * dx + cos_phi * dy;
    let lambda = xp * xp / (rx * rx) + yp * yp / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }
    let numerator = (rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp).max(0.0);
    let denominator = rx * rx * yp * yp + ry * ry * xp * xp;
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let factor = if denominator <= f32::EPSILON {
        0.0
    } else {
        sign * (numerator / denominator).sqrt()
    };
    let cxp = factor * (rx * yp / ry);
    let cyp = factor * (-ry * xp / rx);
    let cx = cos_phi * cxp - sin_phi * cyp + (start.0 + end.0) * 0.5;
    let cy = sin_phi * cxp + cos_phi * cyp + (start.1 + end.1) * 0.5;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let ux = (xp - cxp) / rx;
    let uy = (yp - cyp) / ry;
    let vx = (-xp - cxp) / rx;
    let vy = (-yp - cyp) / ry;
    let theta = uy.atan2(ux);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let count = (delta.abs() / (std::f32::consts::FRAC_PI_2)).ceil() as usize;
    if count == 0 || count > 4 {
        return Err(SvgParseError::InvalidPath);
    }
    let map = |u: f32, v: f32| {
        (
            cx + rx * cos_phi * u - ry * sin_phi * v,
            cy + rx * sin_phi * u + ry * cos_phi * v,
        )
    };
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let a = theta + delta * index as f32 / count as f32;
        let b = theta + delta * (index + 1) as f32 / count as f32;
        let step = b - a;
        let k = 4.0 / 3.0 * (step * 0.25).tan();
        let (sa, ca) = a.sin_cos();
        let (sb, cb) = b.sin_cos();
        let p1 = map(ca - k * sa, sa + k * ca);
        let p2 = map(cb + k * sb, sb - k * cb);
        let p = map(cb, sb);
        out.push(c(p1.0, p1.1, p2.0, p2.1, p.0, p.1));
    }
    if let Some(SvgPathCommand::CubicTo { x, y, .. }) = out.last_mut() {
        *x = end.0;
        *y = end.1;
    }
    if out.iter().all(command_finite) {
        Ok(out)
    } else {
        Err(SvgParseError::InvalidNumber)
    }
}

fn command_finite(command: &SvgPathCommand) -> bool {
    match command {
        SvgPathCommand::MoveTo { x, y } | SvgPathCommand::LineTo { x, y } => {
            x.is_finite() && y.is_finite()
        }
        SvgPathCommand::QuadraticTo {
            control_x,
            control_y,
            x,
            y,
        } => [control_x, control_y, x, y]
            .into_iter()
            .all(|v| v.is_finite()),
        SvgPathCommand::CubicTo {
            control1_x,
            control1_y,
            control2_x,
            control2_y,
            x,
            y,
        } => [control1_x, control1_y, control2_x, control2_y, x, y]
            .into_iter()
            .all(|v| v.is_finite()),
        SvgPathCommand::Close => true,
    }
}

fn parse_numbers(s: &str) -> Vec<f32> {
    let mut scanner = Numbers::new(s);
    let mut out = Vec::new();
    while scanner.skip_separators() {
        match scanner.number() {
            Ok(v) => out.push(v),
            Err(_) => return Vec::new(),
        }
    }
    out
}
struct Numbers<'a> {
    s: &'a str,
    i: usize,
}
impl<'a> Numbers<'a> {
    fn new(s: &'a str) -> Self {
        Self { s, i: 0 }
    }
    fn flag(&mut self) -> Result<bool, SvgParseError> {
        self.skip_separators();
        // Arc flags are single characters, so `001.5` is flags 0, 0
        // followed by coordinate 1.5 rather than one numeric token.
        let flag = match self.peek_char() {
            Some('0') => false,
            Some('1') => true,
            _ => return Err(SvgParseError::InvalidPath),
        };
        self.bump();
        Ok(flag)
    }
    fn peek_char(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }
    fn bump(&mut self) {
        if let Some(c) = self.peek_char() {
            self.i += c.len_utf8();
        }
    }
    fn skip_separators(&mut self) -> bool {
        while let Some(c) = self.peek_char() {
            if c.is_ascii_whitespace() || c == ',' {
                self.bump()
            } else {
                break;
            }
        }
        self.i < self.s.len()
    }
    fn number(&mut self) -> Result<f32, SvgParseError> {
        self.skip_separators();
        let start = self.i;
        if self.peek_char().is_some_and(|c| c == '+' || c == '-') {
            self.bump()
        }
        while self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
            self.bump()
        }
        if self.peek_char() == Some('.') {
            self.bump();
            while self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                self.bump()
            }
        }
        if self.peek_char().is_some_and(|c| c == 'e' || c == 'E') {
            self.bump();
            if self.peek_char().is_some_and(|c| c == '+' || c == '-') {
                self.bump()
            }
            while self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                self.bump()
            }
        }
        if self.i == start {
            return Err(SvgParseError::InvalidNumber);
        }
        let n = self.s[start..self.i]
            .parse::<f32>()
            .map_err(|_| SvgParseError::InvalidNumber)?;
        if n.is_finite() {
            Ok(n)
        } else {
            Err(SvgParseError::InvalidNumber)
        }
    }
}

fn m(x: f32, y: f32) -> SvgPathCommand {
    SvgPathCommand::MoveTo { x, y }
}
fn l(x: f32, y: f32) -> SvgPathCommand {
    SvgPathCommand::LineTo { x, y }
}
fn q(control_x: f32, control_y: f32, x: f32, y: f32) -> SvgPathCommand {
    SvgPathCommand::QuadraticTo {
        control_x,
        control_y,
        x,
        y,
    }
}
fn c(
    control1_x: f32,
    control1_y: f32,
    control2_x: f32,
    control2_y: f32,
    x: f32,
    y: f32,
) -> SvgPathCommand {
    SvgPathCommand::CubicTo {
        control1_x,
        control1_y,
        control2_x,
        control2_y,
        x,
        y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_content_is_retained_for_resource_references() {
        let parsed = parse_svg_document(
            br#"<svg width="8" height="8"><defs><clipPath id="clip"><rect id="clip-rect" width="4" height="8"/></clipPath></defs><path id="target" d="M0 0h8v8z" clip-path="url(#clip)"/></svg>"#,
        )
        .expect("referenced definitions should parse");

        let clip_rect = parsed
            .scene
            .nodes
            .iter()
            .find(|node| node.svg_id.as_deref() == Some("clip-rect"))
            .expect("clip path geometry should be retained");
        assert!(parsed.scene.geometry(clip_rect).is_some());
    }

    #[test]
    fn root_level_clip_path_definitions_are_retained() {
        let parsed = parse_svg_document(
            br#"<svg width="8" height="8"><clipPath id="clip"><rect id="clip-rect" width="4" height="8"/></clipPath><path id="target" d="M0 0h8v8z" clip-path="url(#clip)"/></svg>"#,
        )
        .expect("clip paths may be direct children of the SVG root");

        assert_eq!(parsed.resources.clip_paths.len(), 1);
        assert_eq!(parsed.resources.clip_paths[0].nodes.len(), 1);
        let clip_rect = parsed
            .scene
            .node(SvgNodeId(parsed.resources.clip_paths[0].nodes[0]))
            .unwrap();
        assert!(clip_rect.is_definition);
        assert_eq!(clip_rect.definition_owner.as_deref(), Some("clip"));
    }

    #[test]
    fn use_resolves_local_shape_references_inside_clip_paths() {
        let parsed = parse_svg_document(
            br##"<svg width="8" height="8">
                <clipPath id="clip"><use href="#clip-shape" x="1" y="2"/></clipPath>
                <defs><rect id="clip-shape" transform="translate(3 4)" width="4" height="2"/></defs>
                <path id="target" d="M0 0h8v8z" clip-path="url(#clip)"/>
            </svg>"##,
        )
        .expect("local use references should resolve after the full document is parsed");

        assert!(
            parsed
                .unsupported_elements
                .iter()
                .all(|tag| tag.as_ref() != "use")
        );
        assert_eq!(parsed.resources.clip_paths[0].nodes.len(), 1);
        let clip_node = parsed
            .scene
            .node(SvgNodeId(parsed.resources.clip_paths[0].nodes[0]))
            .unwrap();
        assert!(clip_node.is_definition);
        assert_eq!(clip_node.definition_owner.as_deref(), Some("clip"));
        assert!(parsed.scene.geometry(clip_node).is_some());
        assert_eq!(clip_node.transform.tx, 4.0);
        assert_eq!(clip_node.transform.ty, 6.0);
    }

    #[test]
    fn unresolved_local_use_is_skipped_and_external_use_is_rejected() {
        let parsed =
            parse_svg_document(br##"<svg width="8" height="8"><use href="#missing"/></svg>"##)
                .expect("unresolved local references should not discard the whole SVG");
        assert!(
            parsed
                .unsupported_elements
                .iter()
                .any(|tag| tag.as_ref() == "use")
        );

        let error = parse_svg_document(
            br##"<svg width="8" height="8"><use href="https://example.com/icon.svg#shape"/></svg>"##,
        )
        .expect_err("external use references must remain disallowed");
        assert!(matches!(error, SvgParseError::ExternalResource(_)));
    }

    #[test]
    fn local_resources_and_css_effect_references_are_retained() {
        let parsed = parse_svg_document(
            br##"<svg width="20" height="10">
                <defs>
                    <clipPath id="clip" clipPathUnits="objectBoundingBox"><rect width="0.5" height="1"/></clipPath>
                    <mask id="mask" mask-type="alpha"><rect width="10" height="10" fill="#80ffffff"/></mask>
                    <pattern id="dots" width="4" height="4"><circle cx="2" cy="2" r="1"/></pattern>
                    <filter id="fx"><feGaussianBlur stdDeviation="2 3"/><feOffset dx="1" dy="-1"/><feFlood flood-color="#ff0000" flood-opacity="0.5"/><feComposite in="SourceGraphic" in2="SourceAlpha" operator="atop"/><feBlend in="previous" in2="SourceGraphic" mode="multiply"/><feMerge><feMergeNode in="SourceGraphic"/><feMergeNode in="previous"/></feMerge><feColorMatrix type="saturate" values="0.25"/></filter>
                </defs>
                <path id="target" d="M0 0h20v10z"/>
                <style>#target { fill: url(#dots); clip-path: url(#clip); mask: url(#mask); filter: url(#fx); }</style>
            </svg>"##,
        )
        .expect("supported local resources should parse");

        assert_eq!(parsed.resources.clip_paths.len(), 1);
        assert_eq!(
            parsed.resources.clip_paths[0].units,
            SvgResourceUnits::ObjectBoundingBox
        );
        assert_eq!(parsed.resources.clip_paths[0].nodes.len(), 1);
        assert_eq!(parsed.resources.masks[0].mask_type, SvgMaskType::Alpha);
        assert_eq!(parsed.resources.patterns[0].nodes.len(), 1);
        assert!(matches!(
            parsed.resources.filters[0].primitives[0],
            SvgFilterPrimitive::GaussianBlur {
                deviation: [2.0, 3.0],
                ..
            }
        ));
        assert!(matches!(
            parsed.resources.filters[0].primitives[3],
            SvgFilterPrimitive::Composite {
                operator: SvgCompositeOperator::Atop,
                ..
            }
        ));
        assert!(matches!(
            parsed.resources.filters[0].primitives[4],
            SvgFilterPrimitive::Blend {
                mode: SvgBlendMode::Multiply,
                ..
            }
        ));
        assert!(matches!(
            parsed.resources.filters[0].primitives[5],
            SvgFilterPrimitive::Merge { .. }
        ));
        assert!(matches!(
            parsed.resources.filters[0].primitives[6],
            SvgFilterPrimitive::ColorMatrix { .. }
        ));

        let target = parsed
            .scene
            .nodes
            .iter()
            .find(|node| node.svg_id.as_deref() == Some("target"))
            .unwrap();
        assert_eq!(target.clip_path.as_deref(), Some("clip"));
        assert_eq!(target.mask.as_deref(), Some("mask"));
        assert_eq!(target.filter.as_deref(), Some("fx"));
        assert!(
            matches!(target.fill_paint, Some(SvgPaint::Pattern { ref id }) if id.as_ref() == "dots")
        );
    }

    #[test]
    fn external_effect_references_are_rejected() {
        let error = parse_svg_document(
            br##"<svg width="1" height="1"><path d="M0 0h1v1z" filter="url(https://example.com/fx.svg#blur)"/></svg>"##,
        )
        .expect_err("external filter references must not load");

        assert!(matches!(error, SvgParseError::ExternalResource(_)));
    }

    #[test]
    fn unsupported_element_subtrees_are_skipped_without_losing_supported_siblings() {
        let parsed = parse_svg_document(
            br#"<svg width="8" height="8"><text><tspan>label</tspan></text><path id="visible" d="M0 0h8v8z" fill="red"/></svg>"#,
        ).expect("unsupported text should not discard renderable siblings");

        assert!(
            parsed
                .unsupported_elements
                .iter()
                .any(|tag| tag.as_ref() == "text")
        );
        assert!(
            parsed
                .scene
                .nodes
                .iter()
                .any(|node| node.svg_id.as_deref() == Some("visible"))
        );
    }

    #[test]
    fn embedded_stylesheets_cascade_over_attributes_and_match_source_tags() {
        let parsed = parse_svg_document(
            br##"<svg width="12" height="4" fill="#ffffff">
                <path id="inline" d="M0 0h1v1z" fill="yellow" style="fill:blue"/>
                <path id="specific" class="painted" d="M2 0h1v1z" fill="yellow"/>
                <path id="important" d="M4 0h1v1z" style="fill:blue"/>
                <rect id="rectangle" x="6" width="1" height="1"/>
                <circle id="inherited" cx="9" cy="1" r="1"/>
                <style>
                    svg { fill: #112233; }
                    path { fill: black; }
                    .painted { fill: green; }
                    #specific { fill: red; }
                    #important { fill: red !important; }
                    rect { fill: blue; }
                </style>
            </svg>"##,
        )
        .expect("valid embedded stylesheet should parse");

        let color = |id: &str| {
            parsed
                .scene
                .nodes
                .iter()
                .find(|node| node.svg_id.as_deref() == Some(id))
                .and_then(|node| node.fill.as_ref())
                .expect("node should have a fill")
                .color
        };

        assert_eq!(color("inline"), SvgColor::rgba8(0, 0, 255, 255));
        assert_eq!(color("specific"), SvgColor::rgba8(255, 0, 0, 255));
        assert_eq!(color("important"), SvgColor::rgba8(255, 0, 0, 255));
        assert_eq!(color("rectangle"), SvgColor::rgba8(0, 0, 255, 255));
        assert_eq!(color("inherited"), SvgColor::rgba8(17, 34, 51, 255));
    }
}
