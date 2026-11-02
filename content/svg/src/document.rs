use std::collections::HashMap;
use std::sync::Arc;

use aimer_cupid::svg::{
    SvgFitPolicy, SvgGradient, SvgNodeId, SvgPaint, SvgPathCommand, SvgPreserveAspectRatio,
    SvgResourceGraph, SvgScene, SvgTransform, SvgViewBox, SvgParsedDocument, parse_svg_document,
};

use crate::{SvgError, SvgSelector};

#[derive(Clone, Copy, Debug)]
pub struct SvgLimits {
    pub max_source_bytes: usize,
    pub max_nodes: usize,
    pub max_path_commands: usize,
    pub max_viewport_dimension: f32,
}

impl Default for SvgLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024,
            max_nodes: 16_384,
            max_path_commands: 1_000_000,
            max_viewport_dimension: 1_000_000.0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SvgDiagnostic {
    pub feature: &'static str,
    pub message: Arc<str>,
}

/// Parsed source paints retained beside Cupid's renderer-ready scene.
#[derive(Clone, Debug, Default)]
pub struct SvgNodePaint {
    pub fill: Option<SvgPaint>,
    pub stroke: Option<SvgPaint>,
}

#[derive(Clone)]
pub struct SvgDocument {
    source: Arc<str>,
    scene: Arc<SvgScene>,
    diagnostics: Arc<[SvgDiagnostic]>,
    view_box: SvgViewBox,
    preserve_aspect_ratio: SvgPreserveAspectRatio,
    fit_policy: SvgFitPolicy,
    root_transform: SvgTransform,
    gradients: Arc<[SvgGradient]>,
    resources: Arc<SvgResourceGraph>,
    node_paints: Arc<HashMap<SvgNodeId, SvgNodePaint>>,
}

impl SvgDocument {
    pub fn from_svg(bytes: impl AsRef<[u8]>) -> Result<Self, SvgError> {
        Self::from_svg_with_limits(bytes, SvgLimits::default())
    }

    pub fn from_svg_with_limits(
        bytes: impl AsRef<[u8]>,
        limits: SvgLimits,
    ) -> Result<Self, SvgError> {
        let result = Self::parse_with_limits(bytes, limits);
        if let Err(error) = &result {
            aimer_utils::warn!("SVG parsing failed: {}", error);
        }
        result
    }

    pub(crate) fn from_svg_for_loader(bytes: impl AsRef<[u8]>) -> Result<Self, SvgError> {
        Self::parse_with_limits(bytes, SvgLimits::default())
    }

    fn parse_with_limits(bytes: impl AsRef<[u8]>, limits: SvgLimits) -> Result<Self, SvgError> {
        let bytes = bytes.as_ref();
        if bytes.is_empty() {
            return Err(SvgError::EmptyInput);
        }
        check_limit("source bytes", bytes.len(), limits.max_source_bytes)?;
        check_limit(
            "source bytes",
            bytes.len(),
            SvgLimits::default().max_source_bytes,
        )?;
        let source =
            std::str::from_utf8(bytes).map_err(|error| SvgError::Parse(error.to_string()))?;
        reject_non_finite_literals(source)?;
        let parsed = parse_svg_document(bytes).map_err(map_parse_error)?;
        let viewport = parsed.scene.viewport;
        if !viewport.width.is_finite() || !viewport.height.is_finite() {
            return Err(SvgError::NonFinite);
        }
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return Err(SvgError::Parse(
                "SVG viewport width and height must be positive".to_owned(),
            ));
        }
        check_limit(
            "viewport dimension",
            viewport.width.max(viewport.height) as usize,
            limits.max_viewport_dimension as usize,
        )?;
        check_limit("nodes", parsed.scene.nodes.len(), limits.max_nodes)?;
        let command_count = parsed
            .scene
            .geometries
            .iter()
            .map(|geometry| geometry.commands.len())
            .sum();
        check_limit("path commands", command_count, limits.max_path_commands)?;

        let diagnostics = collect_diagnostics(
            source,
            &parsed.resources,
            &parsed.scene,
            &parsed.unsupported_elements,
        );
        for diagnostic in &diagnostics {
            aimer_utils::warn!("SVG {}: {}", diagnostic.feature, diagnostic.message);
        }

        Ok(Self::from_parsed(source, parsed, diagnostics))
    }

    /// Reconstructs a document from build-time validated parser output.
    ///
    /// Generated code must preserve the original source and every retained
    /// field, and reject unsupported elements or diagnostics at build time.
    /// This entry point deliberately performs no parsing or source scanning.
    #[doc(hidden)]
    pub fn __from_precompiled(source: &'static str, parsed: SvgParsedDocument) -> Self {
        Self::from_parsed(source, parsed, Vec::new())
    }

    fn from_parsed(source: &str, parsed: SvgParsedDocument, diagnostics: Vec<SvgDiagnostic>) -> Self {
        let fit_policy = if parsed.has_view_box {
            SvgFitPolicy::PreserveAspectRatio(parsed.preserve_aspect_ratio)
        } else {
            SvgFitPolicy::Stretch
        };
        let root_transform = parsed.root_transform;
        let node_paints = parsed
            .node_paints
            .into_iter()
            .map(|(node_id, paints)| {
                (
                    node_id,
                    SvgNodePaint {
                        fill: paints.fill,
                        stroke: paints.stroke,
                    },
                )
            })
            .collect();

        Self {
            source: Arc::from(source),
            scene: Arc::new(parsed.scene),
            diagnostics: diagnostics.into(),
            view_box: parsed.view_box,
            preserve_aspect_ratio: parsed.preserve_aspect_ratio,
            fit_policy,
            root_transform,
            gradients: parsed.gradients,
            resources: parsed.resources,
            node_paints: Arc::new(node_paints),
        }
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub fn scene(&self) -> &Arc<SvgScene> {
        &self.scene
    }

    pub fn diagnostics(&self) -> &[SvgDiagnostic] {
        &self.diagnostics
    }

    pub fn view_box(&self) -> SvgViewBox {
        self.view_box
    }

    pub fn preserve_aspect_ratio(&self) -> SvgPreserveAspectRatio {
        self.preserve_aspect_ratio
    }

    pub fn fit_policy(&self) -> SvgFitPolicy {
        self.fit_policy
    }

    pub fn fit_transform(
        &self,
        destination_width: f32,
        destination_height: f32,
    ) -> Result<SvgTransform, SvgError> {
        self.view_box
            .fit_transform(destination_width, destination_height, self.fit_policy)
            .map_err(|error| match error {
                aimer_cupid::svg::SvgFitError::NonFinite(_) => SvgError::NonFinite,
                aimer_cupid::svg::SvgFitError::NonPositive(message) => {
                    SvgError::Parse(message.to_owned())
                }
                aimer_cupid::svg::SvgFitError::InvalidViewBox(message) => {
                    SvgError::InvalidViewBox(message)
                }
                aimer_cupid::svg::SvgFitError::InvalidPreserveAspectRatio(message) => {
                    SvgError::InvalidPreserveAspectRatio(message)
                }
            })
    }

    pub fn gradients(&self) -> &[SvgGradient] {
        &self.gradients
    }

    /// Returns locally defined clips, masks, filters, and patterns.
    pub fn resources(&self) -> &SvgResourceGraph {
        &self.resources
    }

    pub fn paint_for(&self, node_id: SvgNodeId) -> Option<&SvgNodePaint> {
        self.node_paints.get(&node_id)
    }

    pub(crate) fn fit_compensation(
        &self,
        destination_width: f32,
        destination_height: f32,
    ) -> Result<SvgTransform, SvgError> {
        let desired = self.fit_transform(destination_width, destination_height)?;
        let source_to_destination = SvgTransform {
            sx: destination_width / self.scene.viewport.width,
            sy: destination_height / self.scene.viewport.height,
            ..SvgTransform::default()
        };
        let source_inverse = source_to_destination.inverse().ok_or(SvgError::NonFinite)?;
        let root_inverse = self.root_transform.inverse().ok_or(SvgError::NonFinite)?;
        let compensation = source_inverse.mul(desired).mul(root_inverse);
        compensation
            .is_finite()
            .then_some(compensation)
            .ok_or(SvgError::NonFinite)
    }

    pub fn select(
        &self,
        selector: impl TryInto<SvgSelector, Error = SvgError>,
    ) -> Result<Vec<SvgNodeId>, SvgError> {
        let selector = selector.try_into()?;
        Ok(self
            .scene
            .nodes
            .iter()
            .filter(|node| selector.matches(node))
            .map(|node| node.node_id)
            .collect())
    }
}

#[derive(Clone)]
pub struct SvgPath {
    commands: Arc<[SvgPathCommand]>,
}

impl SvgPath {
    pub fn from_path_data(data: &str) -> Result<Self, SvgError> {
        if data.trim().is_empty() {
            return Err(SvgError::InvalidPath("path data is empty".to_owned()));
        }
        let svg = format!(
            r#"<svg width="1" height="1" xmlns="http://www.w3.org/2000/svg"><path id="aimer-path" d="{data}"/></svg>"#
        );
        Self::from_svg(svg.as_bytes(), "#aimer-path").map_err(|error| match error {
            SvgError::Parse(message) => SvgError::InvalidPath(message),
            SvgError::PathSelection(count) => {
                SvgError::InvalidPath(format!("normalized path count was {count}"))
            }
            SvgError::NonFinite => {
                SvgError::InvalidPath("path contains a non-finite value".to_owned())
            }
            other => other,
        })
    }

    pub fn from_svg(
        bytes: impl AsRef<[u8]>,
        selector: impl TryInto<SvgSelector, Error = SvgError>,
    ) -> Result<Self, SvgError> {
        let document = SvgDocument::from_svg(bytes)?;
        let matches = document.select(selector)?;
        let path_nodes: Vec<_> = matches
            .into_iter()
            .filter_map(|node_id| document.scene.node(node_id))
            .filter(|node| node.element == aimer_cupid::svg::SvgElementKind::Path)
            .collect();
        if path_nodes.len() != 1 {
            return Err(SvgError::PathSelection(path_nodes.len()));
        }
        let geometry = document
            .scene
            .geometry(path_nodes[0])
            .ok_or(SvgError::PathSelection(0))?;
        Ok(Self {
            commands: geometry.commands.clone(),
        })
    }

    pub fn commands(&self) -> &[SvgPathCommand] {
        &self.commands
    }
}

fn map_parse_error(error: aimer_cupid::svg::SvgParseError) -> SvgError {
    use aimer_cupid::svg::SvgParseError as ParseError;
    match error {
        ParseError::Empty => SvgError::EmptyInput,
        ParseError::TooLarge => SvgError::LimitExceeded {
            resource: "source bytes",
            actual: 0,
            limit: SvgLimits::default().max_source_bytes,
        },
        ParseError::InvalidUtf8 => SvgError::Parse("SVG source is not UTF-8".to_owned()),
        ParseError::InvalidXml(message) => SvgError::Parse(message.to_owned()),
        ParseError::InvalidRoot => SvgError::Parse("invalid SVG root or dimensions".to_owned()),
        ParseError::InvalidNumber => SvgError::NonFinite,
        ParseError::InvalidPath => SvgError::InvalidPath("invalid SVG path data".to_owned()),
        ParseError::Unsupported(feature) => {
            SvgError::Parse(format!("unsupported SVG feature: {feature}"))
        }
        ParseError::InvalidViewBox => SvgError::InvalidViewBox("invalid viewBox".to_owned()),
        ParseError::InvalidPreserveAspectRatio => {
            SvgError::InvalidPreserveAspectRatio("invalid preserveAspectRatio".to_owned())
        }
        ParseError::ExternalResource(resource) => SvgError::ExternalResource(resource),
        ParseError::LimitExceeded(resource) => SvgError::LimitExceeded {
            resource,
            actual: 0,
            limit: 0,
        },
    }
}

fn collect_diagnostics(
    source: &str,
    resources: &SvgResourceGraph,
    scene: &SvgScene,
    unsupported_elements: &[Arc<str>],
) -> Vec<SvgDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut found = std::collections::HashSet::new();
    let lower = source.to_ascii_lowercase();
    let skipped_elements = unsupported_elements
        .iter()
        .map(|element| element.as_ref())
        .collect::<std::collections::BTreeSet<_>>();
    if !skipped_elements.is_empty() {
        diagnostics.push(SvgDiagnostic {
            feature: "unsupported-element",
            message: Arc::from(format!(
                "unsupported SVG element contents were skipped: {}",
                skipped_elements.into_iter().collect::<Vec<_>>().join(", ")
            )),
        });
    }
    if scene.nodes.iter().any(|node| {
        node.filter.as_deref().is_some_and(|id| {
            node.element != aimer_cupid::svg::SvgElementKind::Path
                || !resources.filter(id).is_some_and(|filter| {
                    filter.primitives.iter().all(|primitive| match primitive {
                        aimer_cupid::svg::SvgFilterPrimitive::Offset { input, .. }
                        | aimer_cupid::svg::SvgFilterPrimitive::ColorMatrix { input, .. } => {
                            matches!(
                                input,
                                aimer_cupid::svg::SvgFilterInput::SourceGraphic
                                    | aimer_cupid::svg::SvgFilterInput::Previous
                            )
                        }
                        _ => false,
                    })
                })
        })
    }) && found.insert("filter")
    {
        diagnostics.push(SvgDiagnostic {
            feature: "filter",
            message: Arc::from("offset and color-matrix filters are applied; blur, flood, blend, composite, merge, and other filter operations are retained and skipped"),
        });
    }
    if scene.nodes.iter().any(|node| {
        node.clip_path.as_deref().is_some_and(|id| {
            node.element != aimer_cupid::svg::SvgElementKind::Path
                || !resources.clip_path(id).is_some_and(|clip| {
                    clip.nodes.len() == 1
                        && scene
                            .node(aimer_cupid::svg::SvgNodeId(clip.nodes[0]))
                            .and_then(|node| scene.geometry(node))
                            .is_some_and(is_rect_geometry)
                })
        })
    }) && found.insert("clip-path")
    {
        diagnostics.push(SvgDiagnostic {
            feature: "clip-path",
            message: Arc::from("only a single axis-aligned rectangular clip shape is applied; other clip paths are retained and skipped"),
        });
    }
    if scene.nodes.iter().any(|node| {
        node.mask.as_deref().is_some_and(|id| {
            node.element != aimer_cupid::svg::SvgElementKind::Path
                || !resources.mask(id).is_some_and(|mask| {
                    mask.nodes.len() == 1
                        && scene
                            .node(aimer_cupid::svg::SvgNodeId(mask.nodes[0]))
                            .is_some_and(|mask_node| {
                                mask_node
                                    .fill_paint
                                    .as_ref()
                                    .map_or(mask_node.fill.is_some(), |paint| {
                                        matches!(paint, aimer_cupid::svg::SvgPaint::Solid(_))
                                    })
                                    && mask_node.stroke.is_none()
                                    && scene.geometry(mask_node).is_some_and(is_rect_geometry)
                            })
                })
        })
    }) && found.insert("mask")
    {
        diagnostics.push(SvgDiagnostic {
            feature: "mask",
            message: Arc::from("only a single solid rectangular mask shape is applied; other masks are retained and skipped"),
        });
    }
    if scene.nodes.iter().any(|node| {
        node.fill_paint
            .iter()
            .chain(node.stroke_paint.iter())
            .any(|paint| match paint {
                aimer_cupid::svg::SvgPaint::Pattern { id } => {
                    node.element != aimer_cupid::svg::SvgElementKind::Path
                        || !resources.pattern(id).is_some_and(|pattern| {
                            pattern.tile[2] > 0.0
                                && pattern.tile[3] > 0.0
                                && is_identity_transform(pattern.transform)
                                && node
                                    .geometry
                                    .and_then(|index| scene.geometries.get(index))
                                    .is_some_and(is_rect_geometry)
                                && pattern.nodes.iter().all(|node_id| {
                                    let Some(pattern_node) =
                                        scene.node(aimer_cupid::svg::SvgNodeId(*node_id))
                                    else {
                                        return false;
                                    };
                                    pattern_node.geometry.is_none()
                                        || (pattern_node.visible
                                            && pattern_node.clip_path.is_none()
                                            && pattern_node.mask.is_none()
                                            && pattern_node.filter.is_none()
                                            && !pattern_node.fill_paint.as_ref().is_some_and(
                                                |paint| {
                                                    matches!(
                                                        paint,
                                                        aimer_cupid::svg::SvgPaint::Pattern { .. }
                                                    )
                                                },
                                            )
                                            && !pattern_node.stroke_paint.as_ref().is_some_and(
                                                |paint| {
                                                    matches!(
                                                        paint,
                                                        aimer_cupid::svg::SvgPaint::Pattern { .. }
                                                    )
                                                },
                                            )
                                            && pattern_node
                                                .stroke
                                                .as_ref()
                                                .map_or(true, |stroke| {
                                                    stroke.dash_array.is_empty()
                                                }))
                                })
                        })
                }
                _ => false,
            })
    }) && found.insert("pattern")
    {
        diagnostics.push(SvgDiagnostic {
            feature: "pattern",
            message: Arc::from("only untransformed patterns used on rectangular paths are applied; other pattern paints are retained and skipped"),
        });
    }
    if lower.contains("stroke-dasharray")
        && !lower.contains("stroke-dasharray=\"none\"")
        && !lower.contains("stroke-dasharray='none'")
        && found.insert("dashed-stroke")
    {
        diagnostics.push(SvgDiagnostic {
            feature: "dashed-stroke",
            message: Arc::from(
                "dashed stroke is retained in the model; renderer support is deferred",
            ),
        });
    }
    diagnostics
}

fn is_identity_transform(transform: aimer_cupid::svg::SvgTransform) -> bool {
    let epsilon = f32::EPSILON * 8.0;
    (transform.sx - 1.0).abs() <= epsilon
        && (transform.sy - 1.0).abs() <= epsilon
        && transform.ky.abs() <= epsilon
        && transform.kx.abs() <= epsilon
        && transform.tx.abs() <= epsilon
        && transform.ty.abs() <= epsilon
}

fn is_rect_geometry(geometry: &aimer_cupid::svg::SvgGeometry) -> bool {
    let [
        aimer_cupid::svg::SvgPathCommand::MoveTo { x: x0, y: y0 },
        aimer_cupid::svg::SvgPathCommand::LineTo { x: x1, y: y1 },
        aimer_cupid::svg::SvgPathCommand::LineTo { x: x2, y: y2 },
        aimer_cupid::svg::SvgPathCommand::LineTo { x: x3, y: y3 },
        aimer_cupid::svg::SvgPathCommand::Close,
    ] = geometry.commands.as_ref()
    else {
        return false;
    };
    y0 == y1 && x1 == x2 && y2 == y3 && x3 == x0 && x0 != x1 && y0 != y2
}

fn reject_non_finite_literals(source: &str) -> Result<(), SvgError> {
    for value in source.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
    }) {
        match value.to_ascii_lowercase().as_str() {
            "nan" | "+nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity"
            | "-infinity" => return Err(SvgError::NonFinite),
            _ => {}
        }
    }
    Ok(())
}

fn check_limit(resource: &'static str, actual: usize, limit: usize) -> Result<(), SvgError> {
    if actual > limit {
        Err(SvgError::LimitExceeded {
            resource,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}
