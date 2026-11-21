//! A frame resolves the fallback faces of the text it shapes, not of the text it
//! only reads from the layout cache.
//!
//! Scrolling brings a line in nearly every frame, so that frame shapes something
//! new while the rest of the screen is already laid out. Walking every grapheme of
//! the whole screen for fallback faces on each such frame costs the screen, where
//! the work is owed to one line.

use super::*;
use crate::backend::metal::MetalBackend;
use crate::font::{FontFamily, FontStyle};
use crate::text_layout::TextHorizontalAlign;
use crate::utilities::Rgba8;
use objc2_metal::MTLPixelFormat;
use std::sync::Arc;

const FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;
const WIDTH: u32 = 400;
const HEIGHT: u32 = 300;
const LINE_HEIGHT: f32 = 24.0;

fn monospace_line(index: usize) -> TextDrawRequest {
    TextDrawRequest {
        x: 8.0,
        y: index as f32 * LINE_HEIGHT,
        text: Arc::from(format!("let line_{index} = compute({index});").as_str()),
        font_size: 16.0,
        color: Rgba8::new(0, 0, 0, 255),
        bounds_width: WIDTH as f32 - 16.0,
        bounds_height: LINE_HEIGHT,
        overflow: TextOverflowMode::Clip,
        horizontal_align: TextHorizontalAlign::Left,
        writing_mode: crate::text_layout::TextWritingMode::HorizontalTb,
        line_height: None,
        shadow: None,
        draw_glyphs: true,
        font_family: FontFamily::MONOSPACE,
        font_style: FontStyle::Normal,
        font_weight: None,
        language: None,
        italic: false,
        clip_rect: [0.0, 0.0, -1.0, 0.0],
        clip_border_radius: [0.0; 4],
        spans: Vec::new(),
        alpha_ramp: None,
    }
}

#[test]
fn a_new_line_resolves_fallback_faces_for_itself_only() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let mut pipeline =
        TextPipelineV2::<MetalBackend>::new(&backend, FORMAT, crate::AntiAlias::Analytic);

    // Eight lines fit the 300 px surface; the ninth starts at 192 px and does too.
    let mut lines: Vec<_> = (0..8).map(monospace_line).collect();
    for _ in 0..100 {
        pipeline.prepare(&backend, WIDTH, HEIGHT, false, &lines, &[]);
        if !pipeline.has_postponed_preparation() {
            break;
        }
    }

    lines.push(monospace_line(8));
    let profile = pipeline.prepare_profiled(&backend, WIDTH, HEIGHT, false, &lines, &[]);

    assert_eq!(profile.shaping_jobs, 1, "only the new line is shaped");
    assert_eq!(
        profile.fallback_spans, 1,
        "only the span that is shaped has its fallback faces resolved"
    );
}
