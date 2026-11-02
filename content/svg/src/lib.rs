mod document;
mod error;
mod selector;
mod source;
mod static_svg;
mod style;
mod widget;

#[cfg(test)]
mod pointer_tests;
#[cfg(test)]
mod static_svg_tests;

pub use aimer_cupid::svg::{
    SvgAspectAlign, SvgAspectMode, SvgBlendMode, SvgClipPath, SvgColor, SvgColorMatrix,
    SvgCompositeOperator, SvgFillRule, SvgFilter, SvgFilterInput, SvgFilterPrimitive, SvgFitError,
    SvgFitPolicy, SvgGradient, SvgGradientStop, SvgGradientUnits, SvgMask, SvgMaskType, SvgNodeId,
    SvgPaint, SvgPattern, SvgPreserveAspectRatio, SvgResourceGraph, SvgResourceUnits,
    SvgSpreadMethod, SvgTransform, SvgViewBox,
};
pub use document::{SvgDiagnostic, SvgDocument, SvgLimits, SvgNodePaint, SvgPath};
pub use error::SvgError;
pub use selector::SvgSelector;
pub use source::{SvgLoadState, SvgLoader, SvgSource};
pub use static_svg::StaticSvg;
pub use style::SvgStyle;
pub use widget::{RawSvg, Svg, SvgAsset, SvgCallback, SvgHit, SvgNodeMetadata};

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::Bounds;
    use aimer_cupid::svg::{
        SvgAspectAlign, SvgAspectMode, SvgColor, SvgElementKind, SvgGradient, SvgNodeStyleOverride,
        SvgPaint, SvgPathCommand, SvgPreserveAspectRatio, SvgSpreadMethod,
    };
    use aimer_widget::Widget;
    use aimer_widget::portable::PortableNativeWidget;

    use crate::{
        SvgAsset, SvgDocument, SvgError, SvgLimits, SvgPath, SvgSelector, SvgStyle, widget,
    };

    #[test]
    fn svg_asset_exposes_the_asset_widget_contract() {
        let widget = SvgAsset::new("assets/icon.svg").width(24.0).height(32.0);

        assert_eq!(widget.debug_name(), "SvgAsset");
    }

    #[test]
    fn svg_exposes_a_native_materializer_for_hot_reload_hosts() {
        fn assert_materializer<T: PortableNativeWidget>() {}

        assert_materializer::<crate::Svg>();
    }

    #[test]
    fn parses_viewbox_groups_transforms_and_solid_path_style() {
        let document = SvgDocument::from_svg(
            br##"<svg viewBox="0 0 24 12" xmlns="http://www.w3.org/2000/svg">
                <g id="layer" class="interactive foreground" transform="translate(2 3)" opacity="0.5">
                    <path id="mark" class="accent" d="M0 0 L10 0 L10 5 Z"
                          fill="#336699" fill-rule="evenodd"
                          stroke="#ff0000" stroke-width="2" stroke-linecap="round" stroke-linejoin="bevel"/>
                </g>
            </svg>"##,
        )
        .expect("valid SVG should parse");

        assert_eq!(document.scene().viewport.width, 24.0);
        assert_eq!(document.scene().viewport.height, 12.0);
        let mark = document.select("#mark").expect("selector should parse");
        assert_eq!(mark.len(), 1);
        let node = document
            .scene()
            .node(mark[0])
            .expect("selected node exists");
        assert_eq!(node.element, SvgElementKind::Path);
        assert_eq!(
            node.classes.iter().map(AsRef::as_ref).collect::<Vec<_>>(),
            ["accent"]
        );
        assert_eq!(node.opacity, 0.5);
        assert_eq!(node.transform.tx, 2.0);
        assert_eq!(node.transform.ty, 3.0);
        assert!(node.fill.is_some());
        assert!(node.stroke.is_some());
    }

    #[test]
    fn embedded_stylesheets_match_tags_ids_and_classes_with_css_cascade() {
        let document = SvgDocument::from_svg(
            br##"<svg width="64" height="16" xmlns="http://www.w3.org/2000/svg">
                <path id="inline" class="painted" d="M0 0h2v2z" fill="#ffffff" style="fill:#0000ff"/>
                <rect id="rectangle" x="3" y="0" width="2" height="2"/>
                <circle id="circle" cx="6" cy="1" r="1"/>
                <path id="ordered" class="ordered" d="M8 0h2v2z"/>
                <path id="important" d="M11 0h2v2z" style="fill:#0000ff"/>
                <path id="inline-important" d="M14 0h2v2z" style="fill:#0000ff!important"/>
                <g class="group"><path id="group-child" d="M17 0h2v2z"/></g>
                <path id="root-inherited" d="M20 0h2v2z"/>
                <path id="presentation" d="M23 0h2v2z" fill="#ffffff"/>
                <style>
                    <![CDATA[
                    svg { fill: #112233; }
                    path { stroke: red; stroke-width: 2; }
                    .painted, #unused { fill: green; }
                    #inline { fill: red; }
                    rect { fill: blue; }
                    circle { fill: yellow; }
                    .ordered { fill: green; }
                    .ordered { fill: blue; }
                    #important { fill: red !important; }
                    #inline-important { fill: red !important; }
                    .group { opacity: 0.5; }
                    #presentation { fill: red; }
                    ]]>
                </style>
            </svg>"##,
        )
        .expect("embedded CSS should parse");

        let node = |id: &str| {
            let selected = document.select(format!("#{id}")).unwrap();
            assert_eq!(selected.len(), 1);
            document.scene().node(selected[0]).unwrap()
        };
        let color = |id: &str| node(id).fill.as_ref().unwrap().color;

        assert_eq!(color("inline"), SvgColor::rgba8(0, 0, 255, 255));
        assert_eq!(color("rectangle"), SvgColor::rgba8(0, 0, 255, 255));
        assert_eq!(color("circle"), SvgColor::rgba8(255, 255, 0, 255));
        assert_eq!(color("ordered"), SvgColor::rgba8(0, 0, 255, 255));
        assert_eq!(color("important"), SvgColor::rgba8(255, 0, 0, 255));
        assert_eq!(color("inline-important"), SvgColor::rgba8(0, 0, 255, 255));
        assert_eq!(color("presentation"), SvgColor::rgba8(255, 0, 0, 255));
        assert_eq!(color("group-child"), SvgColor::rgba8(17, 34, 51, 255));
        assert_eq!(color("root-inherited"), SvgColor::rgba8(17, 34, 51, 255));
        assert_eq!(node("inline").stroke.as_ref().unwrap().width, 2.0);
        assert_eq!(node("group-child").opacity, 0.5);
    }

    #[test]
    fn selectors_match_id_class_and_element_name_in_paint_order() {
        let document = SvgDocument::from_svg(
            br#"<svg width="20" height="10" xmlns="http://www.w3.org/2000/svg">
                <path id="first" class="hot shared" d="M0 0h2v2z"/>
                <path id="second" class="shared" d="M3 0h2v2z"/>
            </svg>"#,
        )
        .unwrap();

        assert_eq!(document.select("path").unwrap().len(), 2);
        assert_eq!(document.select(".shared").unwrap().len(), 2);
        assert_eq!(document.select(".hot").unwrap().len(), 1);
        assert_eq!(document.select("#missing").unwrap(), []);
        assert!(matches!(
            "#".parse::<SvgSelector>(),
            Err(SvgError::InvalidSelector(_))
        ));
    }

    #[test]
    fn standalone_path_data_and_selected_svg_path_are_retained() {
        let path = SvgPath::from_path_data("M1 2 Q3 4 5 6 C7 8 9 10 11 12 Z").unwrap();
        assert!(matches!(path.commands()[0], SvgPathCommand::MoveTo { .. }));
        assert!(matches!(
            path.commands()[1],
            SvgPathCommand::QuadraticTo { .. }
        ));
        assert!(matches!(path.commands()[2], SvgPathCommand::CubicTo { .. }));
        assert!(matches!(path.commands()[3], SvgPathCommand::Close));

        let selected = SvgPath::from_svg(
            br#"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg"><path id="p" d="M0 0h10v10z"/></svg>"#,
            "#p",
        )
        .unwrap();
        assert!(!selected.commands().is_empty());
    }

    #[test]
    fn rejects_empty_malformed_non_finite_and_oversized_input() {
        assert!(matches!(
            SvgDocument::from_svg([]),
            Err(SvgError::EmptyInput)
        ));
        assert!(matches!(
            SvgDocument::from_svg(b"<svg>"),
            Err(SvgError::Parse(_))
        ));
        assert!(matches!(
            SvgPath::from_path_data("M NaN 0"),
            Err(SvgError::InvalidPath(_))
        ));

        let limits = SvgLimits {
            max_source_bytes: 8,
            ..SvgLimits::default()
        };
        assert!(matches!(
            SvgDocument::from_svg_with_limits(b"<svg width='1' height='1'/>", limits),
            Err(SvgError::LimitExceeded {
                resource: "source bytes",
                ..
            })
        ));
    }

    #[test]
    fn rejects_external_resources_and_reports_unsupported_resources() {
        let external = br#"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/a.png"/></svg>"#;
        assert!(matches!(
            SvgDocument::from_svg(external),
            Err(SvgError::ExternalResource(_))
        ));

        let gradient = br#"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g"/></defs><path d="M0 0h10v10z" fill="url(#g)"/></svg>"#;
        let document = SvgDocument::from_svg(gradient).unwrap();
        assert!(
            document
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.feature != "gradient")
        );
    }

    #[test]
    fn exposes_local_filter_definitions_and_reports_skipped_primitives() {
        let document = SvgDocument::from_svg(
            br#"<svg width="10" height="10">
                <style>.soft { filter: url(#blur); }</style>
                <defs><filter id="blur"><feGaussianBlur stdDeviation="2"/></filter></defs>
                <path class="soft" d="M0 0h10v10z" fill="red"/>
            </svg>"#,
        )
        .expect("the filter graph should be retained");

        let filter = document.resources().filter("blur").unwrap();
        assert!(matches!(
            filter.primitives.first(),
            Some(aimer_cupid::svg::SvgFilterPrimitive::GaussianBlur { deviation, .. })
                if *deviation == [2.0, 2.0]
        ));
        assert!(
            document
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.feature == "filter")
        );
    }

    #[test]
    fn applies_root_level_clip_paths_that_reference_local_use_shapes() {
        let document = SvgDocument::from_svg(
            br##"<?xml version="1.0" encoding="utf-8"?>
                <!-- Uploaded to: SVG Repo -->
                <svg width="800px" height="800px" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
                    <defs><path class="icon-path" id="a" d="M-22 2.24h42V22h-42z"/></defs>
                    <clipPath id="b"><use xlink:href="#a" overflow="visible"/></clipPath>
                    <path clip-path="url(#b)" class="icon-path-fill" d="M16.543 8.028c-.023 1.503-.523 3.538-2.867 4.327.734-1.746.846-3.417.326-4.979-.695-2.097-3.014-3.735-4.557-4.627-.527-.306-1.203.074-1.193.683.02 1.112-.318 2.737-1.959 4.378C4.107 9.994 3 12.251 3 14.517 3 17.362 5 21 9 21c-4.041-4.041-1-7.483-1-7.483C8.846 19.431 12.988 21 15 21c1.711 0 5-1.25 5-6.448 0-3.133-1.332-5.511-2.385-6.899-.347-.458-1.064-.198-1.072.375"/>
                </svg>"##,
        )
        .expect("a root-level clip path should not reject the renderable path");

        assert_eq!(document.select(".icon-path-fill").unwrap().len(), 1);
        assert!(document.resources().clip_path("b").is_some());
        assert!(!document.diagnostics().iter().any(|diagnostic| {
            diagnostic.feature == "unsupported-element" && diagnostic.message.contains("use")
        }));
        assert!(
            !document
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.feature == "clip-path")
        );
        assert_eq!(document.resources().clip_path("b").unwrap().nodes.len(), 1);
    }

    #[test]
    fn intrinsic_size_uses_viewport_and_preserves_ratio_for_one_dimension() {
        let viewport = aimer_cupid::svg::SvgViewport {
            width: 40.0,
            height: 20.0,
        };
        assert_eq!(
            widget::resolved_svg_size(viewport, None, None),
            (40.0, 20.0)
        );
        assert_eq!(
            widget::resolved_svg_size(viewport, Some(100.0), None),
            (100.0, 50.0)
        );
        assert_eq!(
            widget::resolved_svg_size(viewport, None, Some(50.0)),
            (100.0, 50.0)
        );
    }

    #[test]
    fn style_overrides_affect_matching_nodes_only() {
        let document = SvgDocument::from_svg(
            br##"<svg width="20" height="10" xmlns="http://www.w3.org/2000/svg">
                <style>.accent { fill: blue; }</style>
                <path id="left" class="accent" d="M0 0h8v8z" fill="#000000"/>
                <path id="right" d="M10 0h8v8z" fill="#000000"/>
            </svg>"##,
        )
        .unwrap();
        let style = SvgStyle::new().fill(aimer_cupid::svg::SvgColor::rgba8(255, 0, 0, 255));
        let overrides =
            widget::overrides_for_rules(document.scene(), &[(".accent".parse().unwrap(), style)]);

        assert_eq!(overrides.len(), 1);
        assert_eq!(
            document
                .scene()
                .node(overrides[0].node_id)
                .unwrap()
                .fill
                .as_ref()
                .unwrap()
                .color,
            aimer_cupid::svg::SvgColor::rgba8(0, 0, 255, 255)
        );
        assert_eq!(
            overrides[0].fill,
            Some(Some(aimer_cupid::svg::SvgColor::rgba8(255, 0, 0, 255)))
        );
        assert_eq!(
            document
                .scene()
                .node(overrides[0].node_id)
                .unwrap()
                .svg_id
                .as_deref(),
            Some("left")
        );
    }

    #[test]
    fn hit_testing_uses_reverse_paint_order_and_nested_transform() {
        let document = SvgDocument::from_svg(
            br##"<svg width="20" height="20" xmlns="http://www.w3.org/2000/svg">
                <path id="back" d="M0 0h10v10z" fill="#000000"/>
                <g transform="translate(2 3)"><path id="front" d="M0 0h10v10z" fill="#ff0000"/></g>
            </svg>"##,
        )
        .unwrap();

        let hit = widget::hit_test_scene(
            document.scene(),
            Bounds::new(0.0, 0.0, 20.0, 20.0),
            5.0,
            5.0,
            &[],
        )
        .unwrap();
        assert_eq!(hit.metadata.svg_id.as_deref(), Some("front"));
        let back = widget::hit_test_scene(
            document.scene(),
            Bounds::new(0.0, 0.0, 20.0, 20.0),
            1.0,
            1.0,
            &[],
        )
        .unwrap();
        assert_eq!(back.metadata.svg_id.as_deref(), Some("back"));
    }

    #[test]
    fn stroke_hit_testing_rejects_points_outside_stroke_width() {
        let document = SvgDocument::from_svg(
            br##"<svg width="20" height="20" xmlns="http://www.w3.org/2000/svg">
                <path id="line" d="M2 10h16" fill="none" stroke="#000000" stroke-width="2"/>
            </svg>"##,
        )
        .unwrap();

        assert!(
            widget::hit_test_scene(
                document.scene(),
                Bounds::new(0.0, 0.0, 20.0, 20.0),
                10.0,
                10.8,
                &[]
            )
            .is_some()
        );
        assert!(
            widget::hit_test_scene(
                document.scene(),
                Bounds::new(0.0, 0.0, 20.0, 20.0),
                10.0,
                13.0,
                &[]
            )
            .is_none()
        );
    }

    #[test]
    fn press_lifecycle_requires_release_on_the_same_path() {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let mut interaction = widget::SvgInteraction::default();
        let node = aimer_cupid::svg::SvgNodeId(4);

        interaction.pointer_down(Some(node));
        assert_eq!(interaction.pointer_up(Some(node)), Some(node));
        observed.set(observed.get() + 1);
        interaction.pointer_down(Some(node));
        assert_eq!(interaction.pointer_up(None), None);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn pressed_svg_requests_capture_and_terminal_release() {
        use aimer_attribute::Vec2d;
        use aimer_events::element::ElementEvent;
        use aimer_events::pointer::{PointerInfo, PointerSource};
        use aimer_widget::{CaptureRequest, EventResult, PointerKey};

        let pointer = PointerKey::new(PointerSource::Touch, 3);
        let down = widget::svg_pointer_capture_effect(
            EventResult::consumed(),
            &ElementEvent::PointerDown(PointerInfo::touch(Vec2d::default(), pointer.id)),
            true,
        );
        let up = widget::svg_pointer_capture_effect(
            EventResult::ignored(),
            &ElementEvent::PointerUp(PointerInfo::touch(Vec2d::default(), pointer.id)),
            false,
        );

        assert_eq!(down.capture_request(), CaptureRequest::Capture(pointer));
        assert_eq!(up.capture_request(), CaptureRequest::Release(pointer));
    }

    #[test]
    fn source_groups_remain_selectable_when_normalizer_flattens_them() {
        let document = SvgDocument::from_svg(
            br#"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg">
                <g class="cluster"><path id="child" d="M0 0h2v2z"/></g>
            </svg>"#,
        )
        .unwrap();

        let groups = document.select("g").unwrap();
        let clusters = document.select(".cluster").unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups, clusters);
        let child = document.select("#child").unwrap()[0];
        assert_eq!(
            document.scene().node(child).unwrap().parent,
            Some(groups[0])
        );
    }

    #[test]
    fn enforces_node_command_and_viewport_limits_and_non_finite_values() {
        let two_paths = br#"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg"><path d="M0 0h1v1z"/><path d="M2 0h1v1z"/></svg>"#;
        let node_limits = SvgLimits {
            max_nodes: 1,
            ..SvgLimits::default()
        };
        assert!(matches!(
            SvgDocument::from_svg_with_limits(two_paths, node_limits),
            Err(SvgError::LimitExceeded {
                resource: "nodes",
                ..
            })
        ));

        let command_limits = SvgLimits {
            max_path_commands: 2,
            ..SvgLimits::default()
        };
        assert!(matches!(
            SvgDocument::from_svg_with_limits(two_paths, command_limits),
            Err(SvgError::LimitExceeded {
                resource: "path commands",
                ..
            })
        ));

        let viewport_limits = SvgLimits {
            max_viewport_dimension: 5.0,
            ..SvgLimits::default()
        };
        assert!(matches!(
            SvgDocument::from_svg_with_limits(two_paths, viewport_limits),
            Err(SvgError::LimitExceeded {
                resource: "viewport dimension",
                ..
            })
        ));

        let non_finite = br#"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg"><path transform="matrix(NaN 0 0 1 0 0)" d="M0 0h1v1z"/></svg>"#;
        assert!(matches!(
            SvgDocument::from_svg(non_finite),
            Err(SvgError::NonFinite)
        ));
    }

    #[test]
    fn parses_viewbox_and_preserve_aspect_ratio_as_a_finite_fit_contract() {
        let document = SvgDocument::from_svg(
            br#"<svg width="200" height="100" viewBox="-10 -20 100 50" preserveAspectRatio="xMinYMax slice" xmlns="http://www.w3.org/2000/svg"><path d="M-10 -20h100v50z"/></svg>"#,
        )
        .unwrap();

        assert_eq!(document.view_box().x, -10.0);
        assert_eq!(document.view_box().height, 50.0);
        assert_eq!(
            document.preserve_aspect_ratio(),
            SvgPreserveAspectRatio {
                align: SvgAspectAlign::XMinYMax,
                mode: SvgAspectMode::Slice,
            }
        );
        let fit = document.fit_transform(300.0, 200.0).unwrap();
        assert!(fit.is_finite());
        assert_eq!(fit.transform_point(-10.0, -20.0), (0.0, 0.0));
    }

    #[test]
    fn rejects_invalid_viewbox_and_preserve_aspect_ratio_before_rendering() {
        assert!(matches!(
            SvgDocument::from_svg(br#"<svg viewBox="0 0 0 10"><path d="M0 0h1v1z"/></svg>"#),
            Err(SvgError::InvalidViewBox(_))
        ));
        assert!(matches!(
            SvgDocument::from_svg(
                br#"<svg preserveAspectRatio="xNopeYNope"><path d="M0 0h1v1z"/></svg>"#
            ),
            Err(SvgError::InvalidPreserveAspectRatio(_))
        ));
    }

    #[test]
    fn retains_gradient_paints_and_reports_deferred_stroke_support() {
        let document = SvgDocument::from_svg(
            br##"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg">
                <defs><linearGradient id="g" spreadMethod="reflect"><stop offset="0" stop-color="#ff0000" stop-opacity="0.5"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
                <path id="painted" d="M0 0h10v10z" fill="url(#g)" stroke="#000" stroke-dasharray="2 1"/>
            </svg>"##,
        )
        .unwrap();
        let node_id = document.select("#painted").unwrap()[0];
        let paint = document.paint_for(node_id).unwrap();
        assert!(matches!(paint.fill, Some(SvgPaint::Linear(_))));
        assert_eq!(document.gradients().len(), 1);
        assert!(matches!(
            document.gradients()[0],
            SvgGradient::Linear {
                spread: SvgSpreadMethod::Reflect,
                ..
            }
        ));
        assert!(
            document
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.feature == "dashed-stroke")
        );
    }

    #[test]
    fn group_style_overrides_propagate_to_renderable_descendants() {
        let document = SvgDocument::from_svg(
            br##"<svg width="10" height="10" xmlns="http://www.w3.org/2000/svg"><g id="group"><path id="child" d="M0 0h5v5z" fill="#000"/></g></svg>"##,
        )
        .unwrap();
        let group_style = SvgStyle::new()
            .fill(SvgColor::rgba8(255, 0, 0, 255))
            .opacity(0.5);
        let overrides = widget::overrides_for_rules(
            document.scene(),
            &[("#group".parse().unwrap(), group_style)],
        );
        let child = document.select("#child").unwrap()[0];
        let child_override = overrides
            .iter()
            .find(|override_| override_.node_id == child)
            .unwrap();
        assert_eq!(
            child_override.fill,
            Some(Some(SvgColor::rgba8(255, 0, 0, 255)))
        );
        assert_eq!(child_override.opacity, Some(0.5));
    }

    #[test]
    fn fit_policy_is_shared_by_hit_testing_when_destination_ratio_changes() {
        let document = SvgDocument::from_svg(
            br#"<svg width="200" height="100" viewBox="0 0 100 50" xmlns="http://www.w3.org/2000/svg"><path id="surface" d="M0 0h100v50z"/></svg>"#,
        )
        .unwrap();
        let node = document.select("#surface").unwrap()[0];
        let source_node = document.scene().node(node).unwrap();
        let compensation = document.fit_compensation(100.0, 100.0).unwrap();
        let overrides = [SvgNodeStyleOverride {
            node_id: node,
            fill: None,
            stroke: None,
            opacity: None,
            transform: Some(compensation.mul(source_node.transform)),
        }];
        let bounds = Bounds::new(0.0, 0.0, 100.0, 100.0);

        assert!(
            widget::hit_test_scene(document.scene(), bounds, 50.0, 50.0, &overrides,).is_some()
        );
        assert!(
            widget::hit_test_scene(document.scene(), bounds, 50.0, 10.0, &overrides,).is_none()
        );
    }
}
