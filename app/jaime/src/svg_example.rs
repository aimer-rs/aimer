use aimer::{Dimension, Svg, SvgDocument, SvgStyle};

const SVG_DEMO: &[u8] = br##"<svg viewBox="-10 -10 140 100" preserveAspectRatio="xMidYMid meet" xmlns="http://www.w3.org/2000/svg">
    <defs>
        <linearGradient id="deferred-gradient" x1="0" y1="0" x2="1" y2="1" spreadMethod="reflect">
            <stop offset="0" stop-color="#ffffff"/>
            <stop offset="1" stop-color="#777777"/>
        </linearGradient>
    </defs>
    <rect id="background" x="-10" y="-10" width="140" height="100" rx="8" fill="#111111"/>
    <path id="mark" d="M20 68 L60 12 L100 68 Z" fill="#f4f4f4" stroke="#999999" stroke-width="3"/>
    <path id="deferred-paint" d="M32 58 L60 22 L88 58 Z" fill="url(#deferred-gradient)"/>
</svg>"##;

/// Parses the bounded SVG example used by the W12 handoff tests.
pub fn svg_document() -> SvgDocument {
    SvgDocument::from_svg(SVG_DEMO).expect("the bounded SVG demo should be valid")
}

/// Builds the standalone SVG showcase widget.
///
/// The visible mark uses solid paints supported by the current Cupid pipeline.
/// The document also contains a retained gradient feature so its explicit
/// deferred diagnostic can be inspected without silently changing that paint.
pub fn svg_example() -> impl aimer::Widget {
    Svg::new(svg_document())
        .width(Dimension::Px(320.0))
        .height(Dimension::Px(240.0))
        .style("#mark", SvgStyle::new().opacity(0.92))
}
