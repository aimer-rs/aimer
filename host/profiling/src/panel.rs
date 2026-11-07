//! Where the overlay's panels go and how one is drawn.
//!
//! There is no layout engine here: the overlay is three text boxes whose height
//! follows their line count, placed in one of two arrangements.

use std::sync::Arc;

use aimer_canvas::{FontFamily, FontStyle};
use aimer_cupid::draw_cmd::{Color, DrawCommand, Rect, Vec2d};
use aimer_cupid::text_pipeline::{TextOverflowMode, text_layout::TextHorizontalAlign};

/// Logical pixels kept clear above the panels. On macOS the window controls
/// float over the content of a full-size window, in the top-left corner. Their
/// position is the application's to choose, so this clears the usual strip
/// rather than reading it. Other platforms draw no controls over the content.
#[cfg(target_os = "macos")]
pub(crate) const TOP_INSET: f32 = 36.0;
#[cfg(not(target_os = "macos"))]
pub(crate) const TOP_INSET: f32 = 0.0;

const MARGIN: f32 = 8.0;
const MAX_PANEL_WIDTH: f32 = 360.0;
/// From this logical width the panels spread over both sides of the window.
const WIDE_FROM: f32 = 760.0;
const LINE_HEIGHT: f32 = 16.0;
/// Padding above the first and below the last line, plus the first baseline.
const PANEL_CHROME: f32 = 24.0;

/// The height a panel needs to show every line of `text`.
pub(crate) fn height_for(text: &str) -> f32 {
    let lines = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    PANEL_CHROME + LINE_HEIGHT * lines as f32
}

/// Where the three panels sit and how far down the overlay reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Layout {
    /// Performance, system and help panels, in that order.
    pub(crate) rects: [Rect; 3],
    /// The lowest edge of any panel plus the margin: the height the overlay's
    /// node must cover.
    pub(crate) bottom: f32,
}

/// Places the panels for a window `logical_width` wide.
///
/// A wide window shows the performance panel with the help panel beneath it on
/// the left and the system panel on the right. A narrow one stacks all three.
pub(crate) fn layout(logical_width: f32, heights: [f32; 3]) -> Layout {
    let [performance, system, help] = heights;
    let width = (logical_width - 2.0 * MARGIN).clamp(0.0, MAX_PANEL_WIDTH);
    let top = TOP_INSET + MARGIN;
    if logical_width >= WIDE_FROM {
        let help_top = top + performance + MARGIN;
        Layout {
            rects: [
                Rect::new(MARGIN, top, width, performance),
                Rect::new(logical_width - width - MARGIN, top, width, system),
                Rect::new(MARGIN, help_top, width, help),
            ],
            bottom: (help_top + help).max(top + system) + MARGIN,
        }
    } else {
        let system_top = top + performance + MARGIN;
        let help_top = system_top + system + MARGIN;
        Layout {
            rects: [
                Rect::new(MARGIN, top, width, performance),
                Rect::new(MARGIN, system_top, width, system),
                Rect::new(MARGIN, help_top, width, help),
            ],
            bottom: help_top + help + MARGIN,
        }
    }
}

/// Appends one translucent panel: a background and its text.
pub(crate) fn push_panel(commands: &mut Vec<DrawCommand>, rect: Rect, text: String) {
    commands.push(DrawCommand::FillRect {
        rect,
        color: Color::rgba8(25, 28, 35, 185),
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    });
    commands.push(DrawCommand::DrawText {
        position: Vec2d::new(rect.x + 8.0, rect.y + 21.0),
        text: Arc::from(text),
        font_size: 13.0,
        color: Color::rgba8(240, 242, 245, 255),
        bounds_width: Some((rect.width - 16.0).max(0.0)),
        bounds_height: Some((rect.height - 16.0).max(0.0)),
        overflow: TextOverflowMode::Clip,
        horizontal_align: TextHorizontalAlign::Left,
        font_family: FontFamily::MONOSPACE,
        font_style: FontStyle::Normal,
        font_weight: 400,
        shadow: None,
        draw_glyphs: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEIGHTS: [f32; 3] = [168.0, 104.0, 88.0];

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    #[test]
    fn height_follows_the_line_count() {
        assert_eq!(height_for("one"), 40.0);
        assert_eq!(height_for("one\ntwo\nthree\nfour\nfive"), 104.0);
        assert_eq!(height_for("a\nb\nc\nd"), 88.0);
    }

    #[test]
    fn a_wide_window_splits_the_panels_and_never_overlaps_them() {
        let layout = layout(1024.0, HEIGHTS);
        let [performance, system, help] = layout.rects;
        assert_eq!(system.x + system.width, 1024.0 - MARGIN, "right-aligned");
        assert_eq!(performance.y, system.y, "performance and system share a row");
        assert!(help.y >= performance.y + performance.height + MARGIN, "help below performance");
        assert!(!overlap(performance, help) && !overlap(performance, system));
        assert!(layout.bottom >= help.y + help.height + MARGIN);
    }

    #[test]
    fn a_narrow_window_stacks_all_three() {
        let layout = layout(400.0, HEIGHTS);
        let [performance, system, help] = layout.rects;
        assert!(system.y >= performance.y + performance.height + MARGIN);
        assert!(help.y >= system.y + system.height + MARGIN);
        assert_eq!(layout.bottom, help.y + help.height + MARGIN);
        assert!(layout.rects.iter().all(|rect| rect.x + rect.width <= 400.0));
    }

    #[test]
    fn every_panel_starts_below_the_inset() {
        for width in [200.0, 759.9, 760.0, 1600.0] {
            for rect in layout(width, HEIGHTS).rects {
                assert!(rect.y >= TOP_INSET + MARGIN, "{width}: {rect:?}");
            }
        }
    }

    #[test]
    fn panels_never_exceed_the_window_or_the_maximum_width() {
        for width in [0.0, 10.0, 100.0, 380.0, 2000.0] {
            for rect in layout(width, HEIGHTS).rects {
                assert!(rect.width >= 0.0 && rect.width <= MAX_PANEL_WIDTH, "{width}: {rect:?}");
            }
        }
    }
}
