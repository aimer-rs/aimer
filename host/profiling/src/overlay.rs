use std::time::Duration;

use aimer_cupid::draw_cmd::{Rect, RenderNodeId, RenderPaintSource, RenderTree, RenderTreeError};
use aimer_frame_stats::GpuProbeGuard;
use aimer_rubick::UiAllocator;
use aimer_utils::AnimInstant;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use crate::panel::{self, height_for, push_panel};
use crate::sampler::{Metrics, REFRESH_INTERVAL, Readings, Sampler};

/// A retained diagnostics panel painted above the application.
///
/// The overlay is owned by the UI thread and is toggled by `F3`. While hidden
/// it allocates nothing, reads no clocks and no counters, and asks for no
/// timer. While visible it measures in windows of a quarter of a second: each
/// window's numbers are shown when it closes, and the panel text is re-recorded
/// only then. Geometry and tree changes are applied immediately, so a resize
/// never waits for a refresh.
///
/// A host drives it with these calls:
///
/// - [`handle_key`](PerformanceOverlay::handle_key) for input;
/// - [`note_app_frame`](PerformanceOverlay::note_app_frame) when the
///   application itself changed what a frame shows, then
///   [`record_draw`](PerformanceOverlay::record_draw) once that frame's draw
///   has finished;
/// - [`prepare`](PerformanceOverlay::prepare) once the render tree is
///   synchronized;
/// - [`refresh_in`](PerformanceOverlay::refresh_in) to learn when to wake an
///   idle application so the numbers can fall to zero.
#[derive(Default)]
pub struct PerformanceOverlay {
    visible: bool,
    painted: bool,
    node: Option<RenderNodeId>,
    /// Width, height and scale bits the current commands were recorded for.
    geometry: Option<(u32, u32, u32)>,
    sampler: Sampler,
    /// The last closed window; `None` until the first one closes.
    metrics: Option<Metrics>,
    /// Whether the frame being drawn is the application's, as opposed to one
    /// that exists only to refresh this overlay.
    app_frame_noted: bool,
    /// Keeps GPU frame timing switched on while the overlay is visible.
    gpu_probe: Option<GpuProbeGuard>,
}

impl PerformanceOverlay {
    /// Toggles the overlay when `key` is an unmodified `F3` press.
    ///
    /// Returns `true` when the key belongs to the overlay (including its
    /// release and auto-repeat, which are consumed without toggling) and must
    /// not reach the application. `modified` is whether any of ctrl, alt, meta
    /// or shift is held; a modified `F3` is left to the application.
    ///
    /// Toggling discards every running measurement, and switches GPU frame
    /// timing on while the overlay is shown and off again when it is hidden.
    pub fn handle_key(
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
            self.sampler = Sampler::default();
            self.metrics = None;
            self.app_frame_noted = false;
            self.gpu_probe = self.visible.then(aimer_frame_stats::acquire_gpu_probe);
        }
        true
    }

    /// Returns whether the overlay is currently shown.
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Declares that the frame now being drawn changes what the application
    /// shows.
    ///
    /// Call it from inside the frame, before [`prepare`](Self::prepare), when
    /// the application left damage. A frame that only exists to refresh the
    /// overlay is then counted as neither a draw nor a present, so an idle
    /// application reads 0 FPS instead of the overlay's own refresh rate. The
    /// note applies to the next [`record_draw`](Self::record_draw) only.
    #[inline]
    pub fn note_app_frame(&mut self) {
        self.app_frame_noted = true;
    }

    /// Records one finished draw that began at `started`.
    ///
    /// Call this only while [`is_visible`](Self::is_visible) is true; the host
    /// reads the start instant only then, which keeps a hidden overlay free of
    /// clock reads.
    pub fn record_draw(&mut self, started: AnimInstant) {
        let elapsed = AnimInstant::now().duration_since(started);
        self.record_draw_elapsed(elapsed);
    }

    fn record_draw_elapsed(&mut self, elapsed: Duration) {
        let app_frame = std::mem::take(&mut self.app_frame_noted);
        self.sampler.record_draw(elapsed, app_frame);
    }

    /// How long until the numbers are due to be refreshed, or `None` while the
    /// overlay is hidden.
    ///
    /// The overlay only repaints when a frame runs, so an idle application
    /// would keep showing its last busy numbers. A host arms a one-shot wake for
    /// this long after each frame and draws one more frame when it fires; that
    /// frame measures the idle window and shows zero. `Some(Duration::ZERO)`
    /// means a refresh is overdue.
    pub fn refresh_in(&self) -> Option<Duration> {
        self.visible
            .then(|| self.refresh_in_at(AnimInstant::now()))
            .flatten()
    }

    fn refresh_in_at(&self, now: AnimInstant) -> Option<Duration> {
        if !self.visible {
            return None;
        }
        Some(match self.sampler.window_started() {
            Some(started) => REFRESH_INTERVAL.saturating_sub(now.duration_since(started)),
            None => REFRESH_INTERVAL,
        })
    }

    /// Brings the overlay's render node in line with the current frame.
    ///
    /// Call it after the widget tree has been synchronized into `tree`, so the
    /// overlay paints after the application and its modals. `width` and
    /// `height` are the surface size in physical pixels, `scale` the device
    /// scale factor, `cursor` the pointer position in logical pixels, and
    /// `renderer` the name of the active graphics backend.
    ///
    /// A hidden overlay empties and damages its previous footprint once, then
    /// does nothing. A surface with a zero size or a non-finite or non-positive
    /// `scale` is skipped without creating a node.
    ///
    /// # Errors
    ///
    /// Returns the render tree's error when the retained node cannot be
    /// updated.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        tree: &RenderTree,
        width: u32,
        height: u32,
        scale: f32,
        allocator: &UiAllocator,
        cursor: aimer_attribute::Vec2d,
        renderer: &str,
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
            renderer,
            AnimInstant::now(),
            Readings::read,
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
        renderer: &str,
        now: AnimInstant,
        read: impl FnOnce() -> Readings,
    ) -> Result<(), RenderTreeError> {
        if !scale.is_finite() || scale <= 0.0 || width == 0 || height == 0 {
            return Ok(());
        }
        let logical_width = width as f32 / scale;
        let logical_height = height as f32 / scale;
        if !logical_width.is_finite() || !logical_height.is_finite() {
            return Ok(());
        }
        let geometry = (width, height, scale.to_bits());
        let node_exists = self
            .node
            .is_some_and(|node| tree.element_bounds(node).is_ok());
        let window_open = self
            .sampler
            .window_started()
            .is_some_and(|started| now.duration_since(started) < REFRESH_INTERVAL);
        if node_exists && self.geometry == Some(geometry) && window_open {
            return Ok(());
        }

        // Closes the measurement window when it is due. A repaint for a resize
        // lands mid-window and leaves it open, reading nothing.
        if let Some(metrics) = self.sampler.advance(now, read) {
            self.metrics = Some(metrics);
        }

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
        let performance = performance_text(
            self.metrics.as_ref(),
            allocator.committed_bytes(),
            &memory_limit,
            &cursor,
        );
        let system = format!(
            "System\n{} / {}\nRenderer: {renderer}\nWindow: {width} x {height} px\nScale: {scale:.2}x",
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        let help = help_text();

        let layout = panel::layout(
            logical_width,
            [height_for(&performance), height_for(&system), height_for(&help)],
        );
        let bounds = Rect::new(0.0, 0.0, logical_width, logical_height.min(layout.bottom));
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

        let mut commands = Vec::with_capacity(6);
        let [performance_rect, system_rect, help_rect] = layout.rects;
        push_panel(&mut commands, performance_rect, performance);
        push_panel(&mut commands, system_rect, system);
        push_panel(&mut commands, help_rect, help);
        tree.context(node)?.begin_recording()?.commit(commands)?;
        self.geometry = Some(geometry);
        self.painted = true;
        Ok(())
    }
}

/// The measurement panel. A field with no reading says why instead of showing a
/// number that would be a guess: `sampling` before the first window closes,
/// `idle` when the application drew nothing, `n/a` when the platform or backend
/// has no source.
fn performance_text(
    metrics: Option<&Metrics>,
    heap_committed: usize,
    heap_limit: &str,
    cursor: &str,
) -> String {
    let sampling = || "sampling".to_string();
    let (draw, present, ui, gpu, cpu, rss) = match metrics {
        None => (sampling(), sampling(), sampling(), sampling(), sampling(), sampling()),
        Some(metrics) => (
            format!("{:.1}", metrics.draw_fps),
            format!("{:.1}", metrics.present_fps),
            metrics
                .ui_ms
                .map_or_else(|| "idle".to_string(), |ms| format!("{ms:.2} ms")),
            metrics
                .gpu_ms
                .map_or_else(|| "n/a".to_string(), |ms| format!("{ms:.2} ms")),
            metrics
                .cpu_percent
                .map_or_else(|| "n/a".to_string(), |percent| format!("{percent:.1}%")),
            metrics.resident_bytes.map_or_else(
                || "n/a".to_string(),
                |bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0),
            ),
        ),
    };
    format!(
        "Aimer debug\nDraw FPS: {draw}\nPresent FPS: {present}\nUI draw: {ui}\nGPU frame: {gpu}\nCPU: {cpu}\nRSS: {rss}\nUI heap: {:.2} MiB / {heap_limit}\nCursor: {cursor}",
        heap_committed as f64 / 1_048_576.0,
    )
}

fn help_text() -> String {
    #[cfg(feature = "frame-stats")]
    let timing = {
        let stats = aimer_frame_stats::frame_breakdown();
        format!(
            "Build / encode / present (ms)\n{:.2} / {:.2} / {:.2}",
            stats.build.average().as_secs_f64() * 1000.0,
            stats.encode.average().as_secs_f64() * 1000.0,
            stats.present.average().as_secs_f64() * 1000.0,
        )
    };
    #[cfg(not(feature = "frame-stats"))]
    let timing = "Phase timings: enable frame-stats".to_string();
    format!(
        "F3: hide diagnostics\n{timing}\nRefreshes {}x a second",
        1000 / REFRESH_INTERVAL.as_millis().max(1)
    )
}

#[cfg(test)]
mod tests {
    use aimer_cupid::draw_cmd::DrawCommand;
    use aimer_frame_stats::GpuFrameProbe;

    use super::*;
    use crate::panel::TOP_INSET;
    use crate::process::ProcessSample;

    const SIZE: (u32, u32) = (1024, 640);

    fn f3() -> Key {
        Key::Named(NamedKey::F3)
    }

    fn shown() -> PerformanceOverlay {
        let mut overlay = PerformanceOverlay::default();
        assert!(overlay.handle_key(&f3(), ElementState::Pressed, false, false));
        overlay
    }

    fn quiet() -> Readings {
        Readings::default()
    }

    fn prepare(
        overlay: &mut PerformanceOverlay,
        tree: &RenderTree,
        memory: &aimer_rubick::UiMemory,
        now: AnimInstant,
        read: impl FnOnce() -> Readings,
    ) {
        overlay
            .prepare_at(
                tree,
                SIZE.0,
                SIZE.1,
                1.0,
                &memory.allocator(),
                Default::default(),
                "Metal",
                now,
                read,
            )
            .unwrap();
    }

    fn texts(tree: &RenderTree, overlay: &PerformanceOverlay) -> Vec<String> {
        tree.draw_list_snapshot(overlay.node.unwrap())
            .unwrap()
            .commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::DrawText { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    fn all_text(tree: &RenderTree, overlay: &PerformanceOverlay) -> String {
        texts(tree, overlay).join("\n")
    }

    fn app_frame(overlay: &mut PerformanceOverlay, elapsed: Duration) {
        overlay.note_app_frame();
        overlay.record_draw_elapsed(elapsed);
    }

    #[test]
    fn f3_toggles_once_per_press_and_reserves_release_and_repeat() {
        let mut overlay = PerformanceOverlay::default();
        let key = f3();
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
        let mut overlay = PerformanceOverlay::default();
        assert!(!overlay.handle_key(
            &Key::Named(NamedKey::F2),
            ElementState::Pressed,
            false,
            false
        ));
        assert!(!overlay.handle_key(&f3(), ElementState::Pressed, false, true));
        assert!(!overlay.visible);
    }

    #[test]
    fn retained_overlay_reuses_commands_until_refresh_and_samples_deterministically() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        let now = AnimInstant::now();
        prepare(&mut overlay, &tree, &memory, now, quiet);
        let node = overlay.node.unwrap();
        let revision = tree.draw_list_revision(node).unwrap();
        tree.take_damage();
        app_frame(&mut overlay, Duration::from_millis(3));
        app_frame(&mut overlay, Duration::from_millis(5));
        prepare(
            &mut overlay,
            &tree,
            &memory,
            now + Duration::from_millis(200),
            || panic!("an open window reads nothing"),
        );
        assert_eq!(tree.draw_list_revision(node).unwrap(), revision);
        assert!(tree.take_damage().is_empty());
        prepare(&mut overlay, &tree, &memory, now + REFRESH_INTERVAL, quiet);
        let metrics = overlay.metrics.unwrap();
        assert_eq!(metrics.draw_fps, 8.0);
        assert!((metrics.ui_ms.unwrap() - 4.0).abs() < 1e-9);
        assert!(tree.draw_list_revision(node).unwrap() > revision);
        assert!(!tree.take_damage().is_empty());
    }

    #[test]
    fn hide_restores_damage_and_reopen_reuses_the_node() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        let key = f3();
        let run = |overlay: &mut PerformanceOverlay| {
            overlay
                .prepare(
                    &tree,
                    1024,
                    640,
                    1.0,
                    &memory.allocator(),
                    Default::default(),
                    "Metal",
                )
                .unwrap();
        };
        run(&mut overlay);
        let node = overlay.node.unwrap();
        tree.take_damage();
        overlay.handle_key(&key, ElementState::Pressed, false, false);
        run(&mut overlay);
        assert!(tree.draw_list_snapshot(node).unwrap().commands.is_empty());
        assert!(!tree.take_damage().is_empty());
        run(&mut overlay);
        assert!(tree.take_damage().is_empty());
        overlay.handle_key(&key, ElementState::Pressed, false, false);
        run(&mut overlay);
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
        let mut overlay = shown();
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
                    "Metal",
                )
                .unwrap();
            assert!(overlay.node.is_none());
        }
    }

    #[test]
    fn initial_metrics_show_sampling_and_an_unlimited_pool_truthfully() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(usize::MAX);
        let mut overlay = shown();
        prepare(&mut overlay, &tree, &memory, AnimInstant::now(), quiet);
        let text = all_text(&tree, &overlay);
        assert!(text.starts_with("Aimer debug"), "{text}");
        for line in [
            "Draw FPS: sampling",
            "Present FPS: sampling",
            "UI draw: sampling",
            "GPU frame: sampling",
            "CPU: sampling",
            "RSS: sampling",
        ] {
            assert!(text.contains(line), "missing {line:?} in {text}");
        }
        assert!(text.contains("unlimited"));
    }

    #[test]
    fn system_panel_reports_the_renderer_the_host_names() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        overlay
            .prepare(
                &tree,
                1024,
                640,
                1.0,
                &memory.allocator(),
                Default::default(),
                "TestRenderer",
            )
            .unwrap();
        let texts = texts(&tree, &overlay);
        assert!(
            texts.iter().any(|text| text.contains("Renderer: TestRenderer")),
            "{texts:?}"
        );
    }

    #[test]
    fn every_measured_field_shows_its_reading() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        let t0 = AnimInstant::now();
        let reading = |presented, cpu_ms| Readings {
            presented_frames: presented,
            gpu: GpuFrameProbe {
                frames: 2,
                total: Duration::from_micros(3_000),
            },
            process: ProcessSample {
                resident_bytes: Some(150 << 20),
                cpu_time: Some(Duration::from_millis(cpu_ms)),
            },
        };
        prepare(&mut overlay, &tree, &memory, t0, || reading(10, 1_000));
        for _ in 0..5 {
            app_frame(&mut overlay, Duration::from_millis(2));
        }
        prepare(&mut overlay, &tree, &memory, t0 + REFRESH_INTERVAL, || {
            reading(15, 1_100)
        });
        let text = all_text(&tree, &overlay);
        for line in [
            "Draw FPS: 20.0",
            "Present FPS: 20.0",
            "UI draw: 2.00 ms",
            "GPU frame: 1.50 ms",
            "CPU: 40.0%",
            "RSS: 150.0 MiB",
        ] {
            assert!(text.contains(line), "missing {line:?} in {text}");
        }
    }

    #[test]
    fn an_idle_overlay_decays_to_zero_instead_of_holding_the_last_frame_rate() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        let t0 = AnimInstant::now();
        prepare(&mut overlay, &tree, &memory, t0, quiet);
        for _ in 0..10 {
            app_frame(&mut overlay, Duration::from_millis(2));
        }
        prepare(&mut overlay, &tree, &memory, t0 + REFRESH_INTERVAL, quiet);
        assert!(all_text(&tree, &overlay).contains("Draw FPS: 40.0"));
        // No application frame in the next window, only the refresh itself.
        overlay.record_draw_elapsed(Duration::from_millis(1));
        prepare(&mut overlay, &tree, &memory, t0 + REFRESH_INTERVAL * 2, quiet);
        let text = all_text(&tree, &overlay);
        assert!(text.contains("Draw FPS: 0.0"), "{text}");
        assert!(text.contains("Present FPS: 0.0"), "{text}");
        assert!(text.contains("UI draw: idle"), "{text}");
    }

    #[test]
    fn a_frame_without_application_damage_is_not_counted_as_a_draw() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        let t0 = AnimInstant::now();
        prepare(&mut overlay, &tree, &memory, t0, quiet);
        app_frame(&mut overlay, Duration::from_millis(2));
        overlay.record_draw_elapsed(Duration::from_millis(2));
        assert!(
            !overlay.app_frame_noted,
            "the note belongs to one frame and is consumed by it"
        );
        prepare(&mut overlay, &tree, &memory, t0 + REFRESH_INTERVAL, quiet);
        assert_eq!(
            overlay.metrics.unwrap().draw_fps,
            4.0,
            "one application frame in a quarter second, the refresh frame not counted"
        );
    }

    #[test]
    fn refresh_is_due_only_while_visible_and_counts_down_from_the_window_start() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = PerformanceOverlay::default();
        let t0 = AnimInstant::now();
        assert_eq!(overlay.refresh_in_at(t0), None, "a hidden overlay wakes nothing");
        overlay.handle_key(&f3(), ElementState::Pressed, false, false);
        assert_eq!(overlay.refresh_in_at(t0), Some(REFRESH_INTERVAL));
        prepare(&mut overlay, &tree, &memory, t0, quiet);
        assert_eq!(overlay.refresh_in_at(t0), Some(REFRESH_INTERVAL));
        assert_eq!(
            overlay.refresh_in_at(t0 + Duration::from_millis(100)),
            Some(Duration::from_millis(150))
        );
        assert_eq!(
            overlay.refresh_in_at(t0 + Duration::from_secs(5)),
            Some(Duration::ZERO),
            "an overdue refresh is due now, not negative"
        );
        overlay.handle_key(&f3(), ElementState::Pressed, false, false);
        assert_eq!(overlay.refresh_in_at(t0), None);
    }

    #[test]
    fn gpu_timing_is_collected_only_while_the_overlay_is_visible() {
        let baseline = aimer_frame_stats::gpu_probe_enabled();
        let mut overlay = PerformanceOverlay::default();
        assert_eq!(aimer_frame_stats::gpu_probe_enabled(), baseline);
        overlay.handle_key(&f3(), ElementState::Pressed, false, false);
        assert!(aimer_frame_stats::gpu_probe_enabled());
        overlay.handle_key(&f3(), ElementState::Pressed, false, false);
        assert_eq!(aimer_frame_stats::gpu_probe_enabled(), baseline);
        overlay.handle_key(&f3(), ElementState::Pressed, false, false);
        drop(overlay);
        assert_eq!(
            aimer_frame_stats::gpu_probe_enabled(),
            baseline,
            "dropping a visible overlay releases the probe"
        );
    }

    #[test]
    fn panels_start_below_the_window_controls_inset() {
        let tree = RenderTree::new();
        let memory = aimer_rubick::UiMemory::new(8 * 1_048_576);
        let mut overlay = shown();
        for (width, height) in [(1024, 640), (400, 900)] {
            overlay
                .prepare(
                    &tree,
                    width,
                    height,
                    1.0,
                    &memory.allocator(),
                    Default::default(),
                    "Metal",
                )
                .unwrap();
            let snapshot = tree.draw_list_snapshot(overlay.node.unwrap()).unwrap();
            let rects: Vec<Rect> = snapshot
                .commands
                .iter()
                .filter_map(|command| match command {
                    DrawCommand::FillRect { rect, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(rects.len(), 3);
            assert!(
                rects.iter().all(|rect| rect.y >= TOP_INSET + 8.0),
                "{width}px wide: {rects:?}"
            );
            let bounds = tree.element_bounds(overlay.node.unwrap()).unwrap();
            let bottom = rects.iter().map(|rect| rect.y + rect.height).fold(0.0, f32::max);
            assert!(bounds.height >= bottom, "bounds {bounds:?} cut panels at {bottom}");
        }
        #[cfg(target_os = "macos")]
        assert!(TOP_INSET >= 28.0, "must clear the traffic lights");
    }
}
