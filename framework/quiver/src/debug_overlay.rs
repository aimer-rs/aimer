use std::sync::Arc;
use std::time::Duration;

use aimer_canvas::{FontFamily, FontStyle};
use aimer_cupid::draw_cmd_v2::{
    Color, DrawCommand, Rect, RenderNodeId, RenderPaintSource, RenderTree, RenderTreeError, Vec2d,
};
use aimer_cupid::text_pipeline::{TextOverflowMode, text_layout::TextHorizontalAlign};
use aimer_rubick::UiAllocator;
use aimer_utils::AnimInstant;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

const REFRESH_INTERVAL: Duration = Duration::from_millis(250);

// Owned by the UI thread. Hidden overlays allocate nothing and read no clocks
// on the frame path. Metric refreshes are throttled to four times a second;
// geometry and tree changes update immediately.
#[derive(Default)]
pub(crate) struct DebugOverlay {
    visible: bool,
    painted: bool,
    node: Option<RenderNodeId>,
    metrics: Option<(u32, u32, u32)>,
    refreshed: Option<AnimInstant>,
    sample_started: Option<AnimInstant>,
    frames: u64,
    ui_time: Duration,
    fps: Option<f64>,
    average_ui_ms: f64,
}

impl DebugOverlay {
    pub(crate) fn handle_key(
        &mut self,
        key: &Key,
        state: ElementState,
        repeat: bool,
        modified: bool,
    ) -> bool {
        if modified || *key != Key::Named(NamedKey::F3) {
            return false;
        }
        if state == ElementState::Pressed && !repeat {
            self.visible = !self.visible;
            self.refreshed = None;
            self.sample_started = None;
            self.frames = 0;
            self.ui_time = Duration::ZERO;
            self.fps = None;
            self.average_ui_ms = 0.0;
        }
        true
    }

    #[inline]
    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn record_draw(&mut self, started: AnimInstant) {
        let now = AnimInstant::now();
        self.record_draw_at(started, now.duration_since(started));
    }

    fn record_draw_at(&mut self, started: AnimInstant, elapsed: Duration) {
        self.sample_started.get_or_insert(started);
        self.frames += 1;
        self.ui_time += elapsed;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        tree: &RenderTree,
        width: u32,
        height: u32,
        scale: f32,
        allocator: &UiAllocator,
        cursor: aimer_attribute::Vec2d,
    ) -> Result<(), RenderTreeError> {
        if !self.visible {
            if self.painted
                && let Some(node) = self.node
                && let Ok(context) = tree.context(node)
            {
                // Empty the old list and damage its footprint, restoring the
                // application pixels underneath it on the next presentation.
                context.begin_recording()?.commit(Vec::new())?;
                tree.set_bounds(node, Rect::new(0.0, 0.0, 0.0, 0.0))?;
            }
            self.painted = false;
            return Ok(());
        }
        self.prepare_at(
            tree,
            width,
            height,
            scale,
            allocator,
            cursor,
            AnimInstant::now(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_at(
        &mut self,
        tree: &RenderTree,
        width: u32,
        height: u32,
        scale: f32,
        allocator: &UiAllocator,
        cursor: aimer_attribute::Vec2d,
        now: AnimInstant,
    ) -> Result<(), RenderTreeError> {
        if !scale.is_finite() || scale <= 0.0 || width == 0 || height == 0 {
            return Ok(());
        }
        let logical_width = width as f32 / scale;
        let logical_height = height as f32 / scale;
        if !logical_width.is_finite() || !logical_height.is_finite() {
            return Ok(());
        }
        let metrics = (width, height, scale.to_bits());
        let node_exists = self
            .node
            .is_some_and(|node| tree.element_bounds(node).is_ok());
        if node_exists
            && self.metrics == Some(metrics)
            && self
                .refreshed
                .is_some_and(|last| now.duration_since(last) < REFRESH_INTERVAL)
        {
            return Ok(());
        }

        if let Some(started) = self.sample_started {
            let elapsed = now.duration_since(started);
            if elapsed >= REFRESH_INTERVAL {
                self.fps = Some(self.frames as f64 / elapsed.as_secs_f64());
                self.average_ui_ms = if self.frames == 0 {
                    0.0
                } else {
                    self.ui_time.as_secs_f64() * 1000.0 / self.frames as f64
                };
                self.frames = 0;
                self.ui_time = Duration::ZERO;
                self.sample_started = Some(now);
            }
        }

        let wide = logical_width >= 760.0;
        let bounds = Rect::new(
            0.0,
            0.0,
            logical_width,
            logical_height.min(if wide { 216.0 } else { 328.0 }),
        );
        // A widget-tree synchronization can remove this extra root. Recreate
        // it after synchronization so it always paints after app and modals,
        // using identities from the same tree as the renderer's node caches.
        let node = if node_exists {
            self.node.expect("the retained overlay node exists")
        } else {
            let node = tree.add_root(bounds)?;
            self.node = Some(node);
            tree.set_paint_source(node, RenderPaintSource::LocalV2)?;
            node
        };
        tree.set_bounds(node, bounds)?;
        tree.set_clip(
            node,
            Some(Rect::new(0.0, 0.0, logical_width, logical_height)),
        )?;

        let fps = self
            .fps
            .map_or_else(|| "sampling".to_string(), |fps| format!("{fps:.1}"));
        let ui_time = if self.fps.is_some() {
            format!("{:.2} ms", self.average_ui_ms)
        } else {
            "sampling".to_string()
        };
        let memory_limit = if allocator.limit_bytes() == usize::MAX {
            "unlimited".to_string()
        } else {
            format!("{:.2} MiB", allocator.limit_bytes() as f64 / 1_048_576.0)
        };
        let cursor = if Rect::new(0.0, 0.0, logical_width, logical_height)
            .contains_point(cursor.x, cursor.y)
        {
            format!("{:.1}, {:.1}", cursor.x, cursor.y)
        } else {
            "outside window".to_string()
        };
        let performance = format!(
            "Aimer debug\nDraw FPS: {fps}\nUI draw: {ui_time}\nUI memory: {:.2} MiB / {memory_limit}\nCursor: {cursor}",
            allocator.committed_bytes() as f64 / 1_048_576.0,
        );
        let backend = std::any::type_name::<crate::render_ctx::AimerRenderContext>()
            .rsplit("::")
            .next()
            .unwrap_or("unknown");
        let system = format!(
            "System\n{} / {}\nRenderer: {backend}\nWindow: {width} x {height} px\nScale: {scale:.2}x",
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        let panel_width = (logical_width - 16.0).clamp(0.0, 360.0);
        let mut commands = Vec::with_capacity(8);
        panel(
            &mut commands,
            Rect::new(8.0, 8.0, panel_width, 104.0),
            performance,
        );
        panel(
            &mut commands,
            Rect::new(
                if wide {
                    logical_width - panel_width - 8.0
                } else {
                    8.0
                },
                if wide { 8.0 } else { 120.0 },
                panel_width,
                104.0,
            ),
            system,
        );
        #[cfg(feature = "frame-stats")]
        let timing = {
            let stats = crate::frame_stats::frame_breakdown();
            format!(
                "Build / encode / present (ms)\n{:.2} / {:.2} / {:.2}",
                stats.build.average().as_secs_f64() * 1000.0,
                stats.encode.average().as_secs_f64() * 1000.0,
                stats.present.average().as_secs_f64() * 1000.0,
            )
        };
        #[cfg(not(feature = "frame-stats"))]
        let timing = "Phase timings: enable frame-stats".to_string();
        panel(
            &mut commands,
            Rect::new(8.0, if wide { 120.0 } else { 232.0 }, panel_width, 88.0),
            format!("F3: hide diagnostics\n{timing}\nUpdates when the app draws"),
        );
        tree.context(node)?.begin_recording()?.commit(commands)?;
        self.metrics = Some(metrics);
        self.refreshed = Some(now);
        self.painted = true;
        Ok(())
    }
}

fn panel(commands: &mut Vec<DrawCommand>, rect: Rect, text: String) {
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

    #[test]
    fn f3_toggles_once_per_press_and_reserves_release_and_repeat() {
        let mut overlay = DebugOverlay::default();
        let key = Key::Named(NamedKey::F3);
        assert!(!overlay.visible);
        assert!(overlay.handle_key(&key, ElementState::Pressed, false, false));
        assert!(overlay.visible);
        assert!(overlay.handle_key(&key, ElementState::Pressed, true, false));
        assert!(overlay.visible);
        assert!(overlay.handle_key(&key, ElementState::Released, false, false));
        assert!(overlay.visible);
        assert!(overlay.handle_key(&key, ElementState::Pressed, false, false));
        assert!(!overlay.visible);
    }

    #[test]
    fn other_keys_and_modified_f3_are_left_to_the_application() {
        let mut overlay = DebugOverlay::default();
        assert!(!overlay.handle_key(
            &Key::Named(NamedKey::F2),
            ElementState::Pressed,
            false,
            false
        ));
        assert!(!overlay.handle_key(
            &Key::Named(NamedKey::F3),
            ElementState::Pressed,
            false,
            true
        ));
        assert!(!overlay.visible);
    }

    #[test]
    fn retained_overlay_reuses_commands_until_refresh_and_samples_deterministically() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = DebugOverlay::default();
        overlay.handle_key(
            &Key::Named(NamedKey::F3),
            ElementState::Pressed,
            false,
            false,
        );
        let now = AnimInstant::now();
        overlay
            .prepare_at(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
                now,
            )
            .unwrap();
        let node = overlay.node.unwrap();
        let revision = tree.draw_list_revision(node).unwrap();
        tree.take_damage();
        overlay.record_draw_at(now, Duration::from_millis(3));
        overlay.record_draw_at(now + Duration::from_millis(100), Duration::from_millis(5));
        overlay
            .prepare_at(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
                now + Duration::from_millis(200),
            )
            .unwrap();
        assert_eq!(tree.draw_list_revision(node).unwrap(), revision);
        assert!(tree.take_damage().is_empty());
        overlay
            .prepare_at(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
                now + REFRESH_INTERVAL,
            )
            .unwrap();
        assert_eq!(overlay.fps, Some(8.0));
        assert!((overlay.average_ui_ms - 4.0).abs() < 1e-9);
        assert!(tree.draw_list_revision(node).unwrap() > revision);
        assert!(!tree.take_damage().is_empty());
    }

    #[test]
    fn hide_restores_damage_and_reopen_reuses_the_node() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = DebugOverlay::default();
        let key = Key::Named(NamedKey::F3);
        overlay.handle_key(&key, ElementState::Pressed, false, false);
        overlay
            .prepare(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
            )
            .unwrap();
        let node = overlay.node.unwrap();
        tree.take_damage();
        overlay.handle_key(&key, ElementState::Pressed, false, false);
        overlay
            .prepare(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
            )
            .unwrap();
        assert!(tree.draw_list_snapshot(node).unwrap().commands.is_empty());
        assert!(!tree.take_damage().is_empty());
        overlay
            .prepare(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
            )
            .unwrap();
        assert!(tree.take_damage().is_empty());
        overlay.handle_key(&key, ElementState::Pressed, false, false);
        overlay
            .prepare(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
            )
            .unwrap();
        assert_eq!(
            overlay.node,
            Some(node),
            "repeated toggles must not accumulate empty roots"
        );
    }

    #[test]
    fn unusable_window_metrics_do_not_create_a_node() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = DebugOverlay::default();
        overlay.handle_key(
            &Key::Named(NamedKey::F3),
            ElementState::Pressed,
            false,
            false,
        );
        for (width, height, scale) in [
            (0, 640, 1.0),
            (1024, 0, 1.0),
            (1024, 640, 0.0),
            (1024, 640, f32::NAN),
            (1024, 640, f32::INFINITY),
            (1024, 640, -1.0),
            (1024, 640, f32::MIN_POSITIVE),
        ] {
            overlay
                .prepare(
                    &tree,
                    width,
                    height,
                    scale,
                    &memory.allocator(),
                    Default::default(),
                )
                .unwrap();
            assert!(overlay.node.is_none());
        }
    }

    #[test]
    fn initial_metrics_show_sampling_and_an_unlimited_pool_truthfully() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(usize::MAX);
        let mut overlay = DebugOverlay::default();
        overlay.handle_key(
            &Key::Named(NamedKey::F3),
            ElementState::Pressed,
            false,
            false,
        );
        overlay
            .prepare(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
            )
            .unwrap();
        let snapshot = tree.draw_list_snapshot(overlay.node.unwrap()).unwrap();
        let text = snapshot
            .commands
            .iter()
            .find_map(|command| match command {
                DrawCommand::DrawText { text, .. } => Some(text.as_ref()),
                _ => None,
            })
            .unwrap();
        assert!(text.contains("UI draw: sampling"));
        assert!(text.contains("unlimited"));
    }
}
