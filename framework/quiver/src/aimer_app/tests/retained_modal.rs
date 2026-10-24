use std::time::Duration;

use aimer_attribute::Dimension;
use aimer_container::{Container, ZeroSizedBox};
use aimer_ctxmenu::{ContextMenu, ContextMenuItem, ContextMenuShape, ContextMenuStyle};
use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
use aimer_cupid::utilities::Mat3;
use aimer_modal::{Modal, ModalAnimation};
#[cfg(feature = "wgpu")]
use aimer_style::ShadowSide;
use aimer_style::{BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow};

use super::*;

fn retained_element_ids_named<W: Widget + 'static>(
    app: &HeadlessAimerApp<W>,
    name: &str,
) -> Vec<ElementId> {
    let root = app
        .app
        .widget_root
        .as_ref()
        .expect("headless frame built the widget root");
    let mut pending = vec![root.as_ref() as &dyn Element];
    let mut ids = Vec::new();
    while let Some(element) = pending.pop() {
        if element.debug_name() == name {
            ids.push(element.id());
        }
        element.visit_children(&mut |child| pending.push(child));
    }
    ids
}

#[test]
fn animated_modal_keeps_its_child_list_and_uses_compositor_properties() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(
        RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(640, 400),
            scale_factor: 2.0,
        },
    );
    app.render_frame();

    let _handle = Modal::new()
        .animation(ModalAnimation::new().enter_duration(Duration::from_secs(1)))
        .child(
            Container::new()
                .color(Color::BLUE)
                .width(120.0)
                .height(64.0)
                .child(ZeroSizedBox),
        )
        .show();
    let (_, packet) = direct_headless_frame_packet(&mut app);

    let content_id = retained_element_id_named(&app, "Container")
        .expect("the hosted modal content remains a framework element");
    let content_node = app
        .app
        .render_node_for_element(content_id)
        .expect("the modal content has its own retained node");
    assert!(
        packet
            .render_plan()
            .and_then(|plan| plan.local_v2_revision(content_node))
            .is_some()
    );

    let operations = app.app.render_tree().render_all();
    let content_item = operations.iter().find_map(|operation| match operation {
        RenderOp::Draw(item) if item.element == content_node => Some(item),
        _ => None,
    });
    let content_item = content_item.expect("modal content has a retained draw operation");
    assert_ne!(content_item.transform, Mat3::identity());
    let center = (content_item.origin.0 + 60.0, content_item.origin.1 + 32.0);
    let transformed_center = content_item.transform.transform_point(center.0, center.1);
    assert!((transformed_center.0 - center.0).abs() < 0.01);
    assert!((transformed_center.1 - center.1).abs() < 0.01);
    assert!((content_item.bounds.width - 115.2).abs() < 0.05);
    assert!(operations.iter().any(|operation| matches!(
        operation,
        RenderOp::BeginOpacityGroup { opacity, .. } if *opacity == 0.0
    )));
}

#[test]
fn animated_floating_menu_keeps_styled_panel_and_rows_as_local_lists() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(
        RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(640, 400),
            scale_factor: 2.0,
        },
    );
    app.render_frame();

    let border = BorderSlice::new()
        .style(BorderStyle::Solid)
        .stroke(Dimension::Px(1.0))
        .color(Color::Rgba(20, 30, 40, 255));
    let outline = BorderSlice::new()
        .style(BorderStyle::Solid)
        .stroke(Dimension::Px(2.0))
        .color(Color::Rgba(50, 60, 70, 255));
    let style = ContextMenuStyle::list().panel(
        BoxDecoration::new()
            .background_color(Color::Rgba(80, 90, 100, 255))
            .border_radius(8.0)
            .border(BoxBorder::all(border))
            .outline(BoxOutline::all(outline))
            .box_shadow([
                BoxShadow::new()
                    .offset_x(3.0)
                    .offset_y(4.0)
                    .blur(5.0)
                    .spread(2.0),
                BoxShadow::new().inset(true).blur(1.0),
            ]),
    );
    let _handle = ContextMenu::new()
        .shape(ContextMenuShape::List)
        .style(style)
        .at(Vec2d { x: 120.0, y: 80.0 })
        .items(vec![ContextMenuItem::new("Copy")])
        .animation(ModalAnimation::new().enter_duration(Duration::from_secs(1)))
        .show();
    let (_, packet) = direct_headless_frame_packet(&mut app);

    let panel_id = retained_element_id_named(&app, "ContextMenu")
        .expect("the custom panel remains a hosted element");
    let rows_id = retained_element_id_named(&app, "ContextMenuRows")
        .expect("the row list remains a hosted element");
    let panel_node = app.app.render_node_for_element(panel_id).unwrap();
    let rows_node = app.app.render_node_for_element(rows_id).unwrap();
    let plan = packet.render_plan().expect("complete retained frame plan");
    assert!(plan.local_v2_revision(panel_node).is_some());
    assert!(plan.local_v2_revision(rows_node).is_none());
    let row_ids = retained_element_ids_named(&app, "ContextMenuRow");
    assert_eq!(row_ids.len(), 1);
    let row_node = app.app.render_node_for_element(row_ids[0]).unwrap();
    assert!(plan.local_v2_revision(row_node).is_some());

    let panel_commands = app
        .app
        .render_tree()
        .draw_list_snapshot(panel_node)
        .unwrap()
        .commands;
    assert!(matches!(
        panel_commands.first(),
        Some(V2DrawCommand::DrawShadowRect { inset: false, .. })
    ));
    assert!(matches!(
        panel_commands.get(1),
        Some(V2DrawCommand::FillRect {
            border_width,
            outline_width,
            ..
        }) if *border_width == [1.0; 4] && *outline_width == [2.0; 4]
    ));
    assert!(matches!(
        panel_commands.last(),
        Some(V2DrawCommand::DrawShadowRect { inset: true, .. })
    ));
    assert!(
        app.app
            .render_tree()
            .element_bounds(panel_node)
            .unwrap()
            .width
            > 150.0,
        "shadow and outline outsets expand the retained panel bounds"
    );
}

#[cfg(feature = "wgpu")]
#[test]
fn retained_context_menu_decoration_renders_at_one_and_two_x_under_opacity() {
    use aimer_cupid::WgpuBackend;
    use aimer_cupid::damage_region::DamageSet;
    use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, Rect, RenderFrame, RenderOp};
    use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
    use aimer_cupid::renderer::Renderer;

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let require_gpu = std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some();
    let gpu_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create retained-menu GPU runtime");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let request_adapter = |force_fallback_adapter, power_preference| {
        instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: None,
            force_fallback_adapter,
            apply_limit_buckets: true,
        })
    };
    let adapter = gpu_runtime
        .block_on(request_adapter(true, wgpu::PowerPreference::LowPower))
        .or_else(|_| {
            gpu_runtime.block_on(request_adapter(
                false,
                wgpu::PowerPreference::HighPerformance,
            ))
        })
        .ok();
    let Some(adapter) = adapter else {
        if require_gpu {
            panic!("GPU adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: GPU adapter unavailable");
        return;
    };
    let device_queue = gpu_runtime.block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Aimer retained context-menu pixel coverage"),
        ..Default::default()
    }));
    let Ok((device, queue)) = device_queue else {
        if require_gpu {
            panic!("could not request a GPU device for retained-menu pixel coverage");
        }
        eprintln!("skipping: GPU device unavailable");
        return;
    };

    const LOGICAL_WIDTH: u32 = 180;
    const LOGICAL_HEIGHT: u32 = 140;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    let backend = WgpuBackend::new(device.clone(), queue.clone());

    for scale in [1.0_f32, 2.0] {
        let width = LOGICAL_WIDTH * scale as u32;
        let height = LOGICAL_HEIGHT * scale as u32;
        let mut app = AimerApp::start_headless_with(
            RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(width, height),
                scale_factor: f64::from(scale),
            },
        );
        app.render_frame();

        let border = BorderSlice::new()
            .style(BorderStyle::Solid)
            .stroke(Dimension::Px(1.0))
            .color(Color::Rgba(20, 30, 40, 255));
        let outline = BorderSlice::new()
            .style(BorderStyle::Solid)
            .stroke(Dimension::Px(2.0))
            .color(Color::Rgba(50, 60, 70, 255));
        let style = ContextMenuStyle::list().panel(
            BoxDecoration::new()
                .background_color(Color::Rgba(80, 90, 100, 255))
                .border_radius(8.0)
                .border(BoxBorder::all(border))
                .outline(BoxOutline::all(outline))
                .box_shadow([
                    BoxShadow::new()
                        .offset_x(3.0)
                        .offset_y(4.0)
                        .blur(5.0)
                        .spread(2.0),
                    BoxShadow::new()
                        .inset(true)
                        .side(ShadowSide::Bottom)
                        .spread(-2.0),
                ]),
        );
        let _handle = ContextMenu::new()
            .shape(ContextMenuShape::List)
            .style(style)
            .at(Vec2d { x: 48.0, y: 40.0 })
            .items(vec![ContextMenuItem::new("Copy")])
            .animation(
                ModalAnimation::new()
                    .enter_duration(Duration::from_secs(30))
                    .content_scale_from(1.0),
            )
            .show();
        let (_, initial_packet) = direct_headless_frame_packet(&mut app);
        assert!(
            initial_packet
                .render_plan()
                .is_some_and(|plan| plan.is_complete())
        );

        let panel_id = retained_element_id_named(&app, "ContextMenu")
            .expect("the custom context-menu panel is present");
        let panel_node = app.app.render_node_for_element(panel_id).unwrap();
        let tree = app.app.render_tree();
        let initial_operations = tree.render_all();
        let mut cursor = panel_node;
        let mut opacity_node = None;
        while let Some(parent) = tree.parent_of(cursor).unwrap() {
            if initial_operations.iter().any(|operation| {
                matches!(operation, RenderOp::BeginOpacityGroup { element, .. } if *element == parent)
            }) {
                opacity_node = Some(parent);
                break;
            }
            cursor = parent;
        }
        let opacity_node = opacity_node.expect("the animated menu content has an opacity group");

        // Set the retained animation group to its deterministic midpoint instead of
        // depending on wall-clock scheduling to sample a particular timeline frame.
        tree.set_opacity(opacity_node, 0.5)
            .expect("set the animated content opacity");
        let operations = tree.render_all();
        assert!(operations.iter().any(|operation| {
            matches!(operation, RenderOp::BeginOpacityGroup { element, opacity, .. }
                if *element == opacity_node && (*opacity - 0.5).abs() < f32::EPSILON)
        }));

        let panel_snapshot = tree.draw_list_snapshot(panel_node).unwrap();
        let (rect, border_width, outline_width) = panel_snapshot
            .commands
            .iter()
            .find_map(|command| match command {
                V2DrawCommand::FillRect {
                    rect,
                    border_width,
                    outline_width,
                    ..
                } if *border_width == [1.0; 4] && *outline_width == [2.0; 4] => {
                    Some((*rect, *border_width, *outline_width))
                }
                _ => None,
            })
            .expect("retained panel list contains its border and outline");
        assert_eq!(border_width, [1.0; 4]);
        assert_eq!(outline_width, [2.0; 4]);
        let (origin, transform) = operations
            .iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == panel_node => {
                    Some((item.origin, item.transform))
                }
                _ => None,
            })
            .expect("retained panel is in the GPU render plan");

        let packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: vec![Rect::new(
                    0.0,
                    0.0,
                    LOGICAL_WIDTH as f32,
                    LOGICAL_HEIGHT as f32,
                )],
                operations,
            },
            FrameRenderMetadata::new(scale, 1, 1, 1, 1, DamageSet::full(width, height)),
            Frame::new(initial_packet.into_frame().draw_list, width, height),
        )
        .expect("build a complete retained menu packet");

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer retained context-menu pixel target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut renderer = Renderer::new(&backend, FORMAT);
        renderer.render_packet(&backend, &view, &packet, false);

        let unpadded_bytes_per_row = width * 4;
        let bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Aimer retained context-menu pixel readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Aimer retained context-menu readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("map retained context-menu pixels");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("wait for retained context-menu pixels");
        let mapped = slice
            .get_mapped_range()
            .expect("map retained context-menu pixel range");
        let mut pixels = vec![0; (unpadded_bytes_per_row * height) as usize];
        for row in 0..height as usize {
            let source_start = row * bytes_per_row as usize;
            let target_start = row * unpadded_bytes_per_row as usize;
            pixels[target_start..target_start + unpadded_bytes_per_row as usize].copy_from_slice(
                &mapped[source_start..source_start + unpadded_bytes_per_row as usize],
            );
        }
        drop(mapped);
        readback.unmap();

        let pixel_at = |local_x: f32, local_y: f32| {
            let (x, y) =
                transform.transform_point(origin.0 + rect.x + local_x, origin.1 + rect.y + local_y);
            let x = (x * scale).floor() as u32;
            let y = (y * scale).floor() as u32;
            let offset = ((y * width + x) * 4) as usize;
            <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
        };
        let assert_near = |actual: [u8; 4], expected: [u8; 4], location: &str| {
            for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
                assert!(
                    actual.abs_diff(expected) <= 4,
                    "scale {scale} {location} channel {channel}: expected {expected}, got {actual}"
                );
            }
        };
        assert_near(
            pixel_at(rect.width * 0.5, 0.5),
            [10, 15, 20, 128],
            "top border",
        );
        assert_near(
            pixel_at(rect.width * 0.5, -1.0),
            [25, 30, 35, 128],
            "top outline",
        );
        assert_near(
            pixel_at(rect.width - 5.0, rect.height * 0.5),
            [40, 45, 50, 128],
            "panel fill",
        );
        let outer_shadow_y = (4.0 / scale).max(outline_width[2] + 0.5);
        let outer_shadow = pixel_at(rect.width * 0.5, rect.height + outer_shadow_y);
        assert!(
            outer_shadow[3] > 16 && outer_shadow[3] < 112,
            "scale {scale} outer shadow has partial alpha: {outer_shadow:?}"
        );
        assert!(outer_shadow[0] < 4 && outer_shadow[1] < 4 && outer_shadow[2] < 4);
        let inset_shadow = pixel_at(rect.width * 0.5, rect.height - 0.5);
        assert!(
            inset_shadow[3] >= 124 && inset_shadow[3] <= 132,
            "scale {scale} inset shadow remains within the composited panel: {inset_shadow:?}"
        );
        assert!(
            inset_shadow[0] < 10 && inset_shadow[1] < 15 && inset_shadow[2] < 20,
            "scale {scale} inset shadow darkens the bottom border: {inset_shadow:?}"
        );
    }
}

#[test]
fn context_menu_row_choice_runs_and_dismisses_through_the_retained_overlay() {
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(
        RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(640, 400),
            scale_factor: 2.0,
        },
    );
    app.render_frame();

    let selected = Rc::new(Cell::new(false));
    let selected_by_item = Rc::clone(&selected);
    let handle = ContextMenu::new()
        .shape(ContextMenuShape::List)
        .at(Vec2d { x: 48.0, y: 40.0 })
        .item(ContextMenuItem::new("Copy").on_select(move || selected_by_item.set(true)))
        .show();
    let (_, first_packet) = direct_headless_frame_packet(&mut app);
    let plan = first_packet
        .render_plan()
        .expect("the visible menu has a retained frame plan");
    let panel_id = retained_element_id_named(&app, "ContextMenu").unwrap();
    let rows_id = retained_element_id_named(&app, "ContextMenuRows").unwrap();
    let panel_node = app.app.render_node_for_element(panel_id).unwrap();
    let rows_node = app.app.render_node_for_element(rows_id).unwrap();
    assert!(plan.local_v2_revision(panel_node).is_some());
    assert!(plan.local_v2_revision(rows_node).is_none());
    let row_ids = retained_element_ids_named(&app, "ContextMenuRow");
    assert_eq!(row_ids.len(), 1);
    let row_node = app.app.render_node_for_element(row_ids[0]).unwrap();
    assert!(plan.local_v2_revision(row_node).is_some());
    let _ = app.window.take_redraw_request();

    // The menu's hit area comes from the render tree, and agrees with where the
    // canvas laid it out.
    let audit = app.app.bounds_audit().expect("the visible menu has a root");
    assert!(
        audit
            .interaction_adopted_by_type
            .contains_key("ContextMenuRows"),
        "the rows take their hit area from the render tree: {:?}",
        audit.interaction_adopted_by_type
    );
    assert!(
        audit.interaction_disagreements.is_empty(),
        "{:?}",
        audit.interaction_disagreements
    );

    let rows = app.app.render_tree().element_bounds(rows_node).unwrap();
    let click = Vec2d {
        x: rows.x + rows.width * 0.5,
        y: rows.y + rows.height * 0.5,
    };
    dispatch_headless_event(
        &mut app,
        click,
        &ElementEvent::PointerDown(PointerInfo::mouse(click, PointerButton::Primary)),
    );
    assert!(
        app.window.take_redraw_request(),
        "pressing a menu row schedules its frame"
    );
    dispatch_headless_event(
        &mut app,
        click,
        &ElementEvent::PointerUp(PointerInfo::mouse(click, PointerButton::Primary)),
    );
    assert!(
        app.window.take_redraw_request(),
        "choosing a row schedules dismissal"
    );

    let (_, dismissed_packet) = direct_headless_frame_packet(&mut app);
    assert!(selected.get(), "the menu row action runs on release");
    assert!(
        !handle.is_showing(),
        "choosing a row dismisses the hosted menu"
    );
    assert!(
        retained_element_id_named(&app, "ContextMenu").is_none(),
        "the dismissed panel leaves the retained tree"
    );
    assert!(
        dismissed_packet
            .render_plan()
            .is_some_and(|plan| plan.is_complete())
    );
}

#[test]
fn context_menu_hover_invalidates_only_the_old_and_new_row_nodes() {
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(
        RecordingWidget {
            builds: Arc::new(AtomicUsize::new(0)),
            cancels: Arc::new(AtomicUsize::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(640, 400),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    let _handle = ContextMenu::new()
        .shape(ContextMenuShape::List)
        .at(Vec2d { x: 48.0, y: 40.0 })
        .items(vec![
            ContextMenuItem::new("Copy"),
            ContextMenuItem::new("Paste"),
            ContextMenuItem::new("Select All"),
        ])
        .show();
    let (_, initial_packet) = direct_headless_frame_packet(&mut app);
    let initial_plan = initial_packet
        .render_plan()
        .expect("the menu has a retained frame plan");
    let panel_id = retained_element_id_named(&app, "ContextMenu").unwrap();
    let rows_id = retained_element_id_named(&app, "ContextMenuRows").unwrap();
    let panel_node = app.app.render_node_for_element(panel_id).unwrap();
    let rows_node = app.app.render_node_for_element(rows_id).unwrap();
    let row_ids = retained_element_ids_named(&app, "ContextMenuRow");
    assert_eq!(
        row_ids.len(),
        3,
        "each menu item owns a retained row element"
    );
    let tree = app.app.render_tree().clone();
    let panel_revision = initial_plan.local_v2_revision(panel_node).unwrap();
    let rows_revision = initial_plan.local_v2_revision(rows_node);
    assert!(
        rows_revision.is_none(),
        "the rows layout and hit-test parent has no retained paint list"
    );
    let mut row_nodes = row_ids
        .into_iter()
        .map(|id| {
            let node = app.app.render_node_for_element(id).unwrap();
            (
                node,
                tree.element_bounds(node).unwrap(),
                initial_plan.local_v2_revision(node).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    row_nodes.sort_by(|left, right| left.1.y.total_cmp(&right.1.y));

    for row_index in [0, 1] {
        let row_bounds = row_nodes[row_index].1;
        let pointer = Vec2d {
            x: row_bounds.x + row_bounds.width - 8.0,
            y: row_bounds.y + row_bounds.height * 0.5,
        };
        dispatch_headless_event(
            &mut app,
            pointer,
            &ElementEvent::PointerMove(PointerInfo::mouse(pointer, PointerButton::Primary)),
        );
        assert!(
            app.window.take_redraw_request(),
            "row hover schedules a frame"
        );
        let rebuild_before_frame = aimer_widget::rebuild_invalidation_generation();
        let layout_before_frame = aimer_widget::layout_invalidation_generation();
        let (_, hover_packet) = direct_headless_frame_packet(&mut app);
        let hover_plan = hover_packet.render_plan().unwrap();
        assert_eq!(tree.draw_list_revision(panel_node).unwrap(), panel_revision);
        assert_eq!(
            hover_plan.local_v2_revision(panel_node),
            Some(panel_revision),
            "the unchanged panel background is composited beneath row damage"
        );
        assert_eq!(hover_plan.local_v2_revision(rows_node), rows_revision);
        for (index, (row_node, _, revision)) in row_nodes.iter().enumerate() {
            let actual = tree.draw_list_revision(*row_node).unwrap();
            let expected = *revision + u64::from(index == row_index || index + 1 == row_index);
            assert_eq!(actual, expected, "only old/new row {index} is re-recorded");
            assert_eq!(
                hover_plan.local_v2_revision(*row_node),
                (index == row_index || index + 1 == row_index).then_some(expected),
                "only old/new row {index} is submitted"
            );
        }

        let damage = hover_packet.metadata().damage();
        let concurrent_global_invalidation = rebuild_before_frame
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_frame != aimer_widget::layout_invalidation_generation();
        if !concurrent_global_invalidation {
            assert!(
                !damage.is_full(),
                "row hover stayed partial: {:?}",
                damage.regions()
            );
            let first = row_nodes[0].1;
            let last_index = if row_index == 0 { 0 } else { 1 };
            let last = row_nodes[last_index].1;
            let left = first.x.min(last.x) * 2.0;
            let top = first.y.min(last.y) * 2.0;
            let right = (first.x + first.width).max(last.x + last.width) * 2.0;
            let bottom = (first.y + first.height).max(last.y + last.height) * 2.0;
            assert!(
                damage.regions().iter().all(|region| {
                    region.x as f32 >= left - 1.0
                        && region.y as f32 >= top - 1.0
                        && (region.x + region.width) as f32 <= right + 1.0
                        && (region.y + region.height) as f32 <= bottom + 1.0
                }),
                "hover damage escaped the changed row bounds: {:?}",
                damage.regions()
            );
        }

        for row in &mut row_nodes {
            row.2 = tree.draw_list_revision(row.0).unwrap();
        }
        let _ = app.window.take_redraw_request();
    }
}

#[cfg(feature = "wgpu")]
fn context_menu_gpu_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create context-menu GPU runtime");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let request_adapter = |force_fallback_adapter, power_preference| {
        instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: None,
            force_fallback_adapter,
            apply_limit_buckets: true,
        })
    };
    let adapter = runtime
        .block_on(request_adapter(true, wgpu::PowerPreference::LowPower))
        .or_else(|_| {
            runtime.block_on(request_adapter(
                false,
                wgpu::PowerPreference::HighPerformance,
            ))
        })
        .ok()?;
    runtime
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Aimer context-menu retained rendering test"),
            ..Default::default()
        }))
        .ok()
}

#[cfg(feature = "wgpu")]
fn read_context_menu_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let unpadded_bytes_per_row = width * 4;
    let bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Aimer context-menu pixel readback"),
        size: u64::from(bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Aimer context-menu readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("map context-menu pixel buffer");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for context-menu pixel readback");
    let mapped = slice
        .get_mapped_range()
        .expect("map context-menu pixel range");
    let mut pixels = vec![0; (unpadded_bytes_per_row * height) as usize];
    for row in 0..height as usize {
        let source_start = row * bytes_per_row as usize;
        let target_start = row * unpadded_bytes_per_row as usize;
        pixels[target_start..target_start + unpadded_bytes_per_row as usize]
            .copy_from_slice(&mapped[source_start..source_start + unpadded_bytes_per_row as usize]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}

#[cfg(feature = "wgpu")]
fn context_menu_pixel(pixels: &[u8], width: u32, x: f32, y: f32, scale: f32) -> [u8; 4] {
    let x = (x * scale).floor() as u32;
    let y = (y * scale).floor() as u32;
    let offset = ((y * width + x) * 4) as usize;
    pixels[offset..offset + 4]
        .try_into()
        .expect("context-menu pixel")
}

#[cfg(feature = "wgpu")]
fn assert_context_menu_pixel_near(actual: [u8; 4], expected: [u8; 4], label: &str) {
    for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= 4,
            "{label} channel {channel}: expected {expected}, got {actual}"
        );
    }
}

#[cfg(feature = "wgpu")]
#[test]
fn retained_context_menu_hover_updates_only_its_rows_layer() {
    use aimer_cupid::WgpuBackend;
    use aimer_cupid::renderer::Renderer;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some((device, queue)) = context_menu_gpu_device() else {
        if std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some() {
            panic!("GPU adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: GPU adapter unavailable");
        return;
    };

    const LOGICAL_WIDTH: u32 = 640;
    const LOGICAL_HEIGHT: u32 = 400;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    let backend = WgpuBackend::new(device.clone(), queue.clone());

    for scale in [1.0_f32, 2.0] {
        let width = LOGICAL_WIDTH * scale as u32;
        let height = LOGICAL_HEIGHT * scale as u32;
        let mut app = AimerApp::start_headless_with(
            RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(width, height),
                scale_factor: f64::from(scale),
            },
        );
        app.render_frame();
        let style = ContextMenuStyle::list()
            .panel(
                BoxDecoration::new()
                    .background_color(Color::Rgba(42, 52, 68, 255))
                    .border_radius(8.0)
                    .box_shadow([BoxShadow::new().offset_y(4.0).blur(4.0)]),
            )
            .highlight_color(Color::Rgba(10, 220, 40, 255));
        let _handle = ContextMenu::new()
            .shape(ContextMenuShape::List)
            .style(style)
            .barrier_color(Color::Rgba(0, 0, 0, 64))
            .at(Vec2d { x: 48.0, y: 40.0 })
            .items(vec![
                ContextMenuItem::new("Copy"),
                ContextMenuItem::new("Paste"),
            ])
            .animation(
                ModalAnimation::new()
                    .enter_duration(Duration::ZERO)
                    .exit_duration(Duration::ZERO)
                    .content_scale_from(1.0),
            )
            .show();
        let (recorded_scale, first_packet) = direct_headless_frame_packet(&mut app);
        assert_eq!(recorded_scale, scale);
        let first_plan = first_packet
            .render_plan()
            .expect("first menu frame has a retained plan");
        assert!(first_plan.is_complete());
        let panel_id = retained_element_id_named(&app, "ContextMenu").unwrap();
        let rows_id = retained_element_id_named(&app, "ContextMenuRows").unwrap();
        let panel_node = app.app.render_node_for_element(panel_id).unwrap();
        let rows_node = app.app.render_node_for_element(rows_id).unwrap();
        let panel_revision = first_plan.local_v2_revision(panel_node).unwrap();
        assert!(first_plan.local_v2_revision(rows_node).is_none());
        let tree = app.app.render_tree();
        let mut row_nodes = retained_element_ids_named(&app, "ContextMenuRow")
            .into_iter()
            .map(|id| {
                let node = app.app.render_node_for_element(id).unwrap();
                (
                    node,
                    app.app.render_tree().element_bounds(node).unwrap(),
                    first_plan.local_v2_revision(node).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(row_nodes.len(), 2);
        row_nodes.sort_by(|left, right| left.1.y.total_cmp(&right.1.y));
        let rows_bounds = app.app.render_tree().element_bounds(rows_node).unwrap();
        let probe_x = rows_bounds.x + rows_bounds.width - 8.0;
        let first_row_y = rows_bounds.y + 14.0;
        let second_row_y = rows_bounds.y + 42.0;
        let untouched_point = (rows_bounds.x + rows_bounds.width * 0.5, rows_bounds.y - 3.0);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer context-menu hover target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut renderer = Renderer::new(&backend, FORMAT);
        renderer.render_packet(&backend, &view, &first_packet, false);
        let initial_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        let initial_first_row =
            context_menu_pixel(&initial_pixels, width, probe_x, first_row_y, scale);
        let initial_outside_rows = context_menu_pixel(
            &initial_pixels,
            width,
            untouched_point.0,
            untouched_point.1,
            scale,
        );
        let _ = app.window.take_redraw_request();

        dispatch_headless_event(
            &mut app,
            Vec2d {
                x: probe_x,
                y: first_row_y,
            },
            &ElementEvent::PointerMove(PointerInfo::mouse(
                Vec2d {
                    x: probe_x,
                    y: first_row_y,
                },
                PointerButton::Primary,
            )),
        );
        assert!(
            app.window.take_redraw_request(),
            "row hover schedules a frame"
        );
        let rebuild_before_first_hover = aimer_widget::rebuild_invalidation_generation();
        let layout_before_first_hover = aimer_widget::layout_invalidation_generation();
        let (_, first_hover_packet) = direct_headless_frame_packet(&mut app);
        let first_hover_plan = first_hover_packet.render_plan().unwrap();
        assert_eq!(
            first_hover_plan.local_v2_revision(panel_node),
            Some(panel_revision)
        );
        assert_eq!(first_hover_plan.local_v2_revision(rows_node), None);
        for (index, (row_node, _, revision)) in row_nodes.iter().enumerate() {
            assert_eq!(
                first_hover_plan.local_v2_revision(*row_node),
                Some(*revision + u64::from(index == 0)),
                "the first hover changes only row {index}"
            );
        }
        let first_hover_damage = first_hover_packet.metadata().damage();
        let concurrent_first_hover_invalidation = rebuild_before_first_hover
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_first_hover != aimer_widget::layout_invalidation_generation();
        if !concurrent_first_hover_invalidation {
            assert!(
                !first_hover_damage.is_full(),
                "hover damage was promoted to full: {:?}",
                first_hover_damage.regions()
            );
            assert!(
                first_hover_damage.regions().iter().all(|region| {
                    region.x as f32 >= rows_bounds.x * scale - 1.0
                        && region.y as f32 >= rows_bounds.y * scale - 1.0
                        && (region.x + region.width) as f32
                            <= (rows_bounds.x + rows_bounds.width) * scale + 1.0
                        && (region.y + region.height) as f32
                            <= (rows_bounds.y + rows_bounds.height) * scale + 1.0
                }),
                "hover damage escaped the rows bounds: {:?}",
                first_hover_damage.regions()
            );
        }
        renderer.render_packet(&backend, &view, &first_hover_packet, false);
        let first_hover_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert_context_menu_pixel_near(
            context_menu_pixel(&first_hover_pixels, width, probe_x, first_row_y, scale),
            [10, 220, 40, 255],
            "hovered first row",
        );
        assert_eq!(
            context_menu_pixel(
                &first_hover_pixels,
                width,
                untouched_point.0,
                untouched_point.1,
                scale,
            ),
            initial_outside_rows,
            "hover damage preserves the panel outside the rows"
        );
        for row in &mut row_nodes {
            row.2 = first_hover_plan.local_v2_revision(row.0).unwrap();
        }
        let _ = app.window.take_redraw_request();

        dispatch_headless_event(
            &mut app,
            Vec2d {
                x: probe_x,
                y: second_row_y,
            },
            &ElementEvent::PointerMove(PointerInfo::mouse(
                Vec2d {
                    x: probe_x,
                    y: second_row_y,
                },
                PointerButton::Primary,
            )),
        );
        assert!(
            app.window.take_redraw_request(),
            "moving the hover schedules a frame"
        );
        let rebuild_before_second_hover = aimer_widget::rebuild_invalidation_generation();
        let layout_before_second_hover = aimer_widget::layout_invalidation_generation();
        let (_, second_hover_packet) = direct_headless_frame_packet(&mut app);
        let second_hover_plan = second_hover_packet.render_plan().unwrap();
        assert_eq!(
            second_hover_plan.local_v2_revision(panel_node),
            Some(panel_revision)
        );
        assert_eq!(second_hover_plan.local_v2_revision(rows_node), None);
        for (index, (row_node, _, revision)) in row_nodes.iter().enumerate() {
            assert_eq!(
                second_hover_plan.local_v2_revision(*row_node),
                Some(*revision + 1),
                "moving between rows re-records old and new row {index}"
            );
        }
        let second_hover_damage = second_hover_packet.metadata().damage();
        let concurrent_second_hover_invalidation = rebuild_before_second_hover
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_second_hover != aimer_widget::layout_invalidation_generation();
        if !concurrent_second_hover_invalidation {
            assert!(!second_hover_damage.is_full());
            assert!(
                second_hover_damage.regions().iter().all(|region| {
                    region.x as f32 >= rows_bounds.x * scale - 1.0
                        && region.y as f32 >= rows_bounds.y * scale - 1.0
                        && (region.x + region.width) as f32
                            <= (rows_bounds.x + rows_bounds.width) * scale + 1.0
                        && (region.y + region.height) as f32
                            <= (rows_bounds.y + rows_bounds.height) * scale + 1.0
                }),
                "row-transfer damage escaped the rows bounds: {:?}",
                second_hover_damage.regions()
            );
        }
        renderer.render_packet(&backend, &view, &second_hover_packet, false);
        let second_hover_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert_eq!(
            context_menu_pixel(&second_hover_pixels, width, probe_x, first_row_y, scale),
            initial_first_row,
            "moving to the next row clears the old highlight"
        );
        assert_context_menu_pixel_near(
            context_menu_pixel(&second_hover_pixels, width, probe_x, second_row_y, scale),
            [10, 220, 40, 255],
            "hovered second row",
        );
        assert_eq!(
            context_menu_pixel(
                &second_hover_pixels,
                width,
                untouched_point.0,
                untouched_point.1,
                scale,
            ),
            initial_outside_rows,
            "row highlight transfer preserves panel pixels"
        );
    }
}

#[cfg(feature = "wgpu")]
#[test]
fn replacing_a_retained_context_menu_clears_its_old_shadow_and_panel_pixels() {
    use aimer_cupid::WgpuBackend;
    use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, Rect, RenderOp};
    use aimer_cupid::renderer::Renderer;

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some((device, queue)) = context_menu_gpu_device() else {
        if std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some() {
            panic!("GPU adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: GPU adapter unavailable");
        return;
    };

    const LOGICAL_WIDTH: u32 = 500;
    const LOGICAL_HEIGHT: u32 = 340;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    let backend = WgpuBackend::new(device.clone(), queue.clone());

    for scale in [1.0_f32, 2.0] {
        let width = LOGICAL_WIDTH * scale as u32;
        let height = LOGICAL_HEIGHT * scale as u32;
        let mut app = AimerApp::start_headless_with(
            RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(width, height),
                scale_factor: f64::from(scale),
            },
        );
        app.render_frame();

        let first_style = ContextMenuStyle::list().panel(
            BoxDecoration::new()
                .background_color(Color::Rgba(50, 20, 30, 255))
                .border_radius(8.0)
                .box_shadow([BoxShadow::new()
                    .offset_x(3.0)
                    .offset_y(4.0)
                    .blur(5.0)
                    .spread(2.0)]),
        );
        let first_handle = ContextMenu::new()
            .shape(ContextMenuShape::List)
            .style(first_style)
            .at(Vec2d { x: 24.0, y: 24.0 })
            .item(ContextMenuItem::new("Copy"))
            .show();
        let (_, first_packet) = direct_headless_frame_packet(&mut app);
        let first_panel_id = retained_element_id_named(&app, "ContextMenu").unwrap();
        let first_panel_node = app.app.render_node_for_element(first_panel_id).unwrap();
        let tree = app.app.render_tree();
        let first_geometry = tree
            .render_all()
            .iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == first_panel_node => {
                    let rect =
                        item.snapshot()
                            .commands
                            .iter()
                            .find_map(|command| match command {
                                V2DrawCommand::FillRect { rect, .. } => Some(*rect),
                                _ => None,
                            })?;
                    Some((item.origin, item.transform, rect))
                }
                _ => None,
            })
            .expect("first context menu has a retained panel fill");
        let panel_point = |local_x: f32, local_y: f32| {
            let (x, y) = first_geometry.1.transform_point(
                first_geometry.0.0 + first_geometry.2.x + local_x,
                first_geometry.0.1 + first_geometry.2.y + local_y,
            );
            (x, y)
        };
        let old_fill_point =
            panel_point(first_geometry.2.width - 6.0, first_geometry.2.height * 0.5);
        let old_shadow_point = panel_point(
            first_geometry.2.width * 0.5,
            first_geometry.2.height + 4.0 / scale,
        );

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer context-menu replacement target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut renderer = Renderer::new(&backend, FORMAT);
        renderer.render_packet(&backend, &view, &first_packet, false);
        let first_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert!(
            context_menu_pixel(
                &first_pixels,
                width,
                old_fill_point.0,
                old_fill_point.1,
                scale
            )[3] > 240
        );
        assert!(
            context_menu_pixel(
                &first_pixels,
                width,
                old_shadow_point.0,
                old_shadow_point.1,
                scale
            )[3] > 0
        );

        assert!(first_handle.dismiss());
        let (_, close_packet) = direct_headless_frame_packet(&mut app);
        renderer.render_packet(&backend, &view, &close_packet, false);
        let closed_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert_eq!(
            context_menu_pixel(
                &closed_pixels,
                width,
                old_fill_point.0,
                old_fill_point.1,
                scale
            ),
            [0, 0, 0, 0],
            "dismissal clears the old panel at scale {scale}"
        );
        assert_eq!(
            context_menu_pixel(
                &closed_pixels,
                width,
                old_shadow_point.0,
                old_shadow_point.1,
                scale
            ),
            [0, 0, 0, 0],
            "dismissal clears the old shadow at scale {scale}"
        );

        let second_style = ContextMenuStyle::list().panel(
            BoxDecoration::new()
                .background_color(Color::Rgba(30, 70, 110, 255))
                .border_radius(4.0)
                .box_shadow([BoxShadow::new().offset_y(3.0).blur(3.0)]),
        );
        let _second_handle = ContextMenu::new()
            .shape(ContextMenuShape::List)
            .style(second_style)
            .at(Vec2d { x: 320.0, y: 220.0 })
            .item(ContextMenuItem::new("Paste"))
            .show();
        let (_, second_packet) = direct_headless_frame_packet(&mut app);
        let second_panel_id = retained_element_id_named(&app, "ContextMenu").unwrap();
        let second_panel_node = app.app.render_node_for_element(second_panel_id).unwrap();
        let second_geometry = app
            .app
            .render_tree()
            .render_all()
            .iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == second_panel_node => {
                    let rect =
                        item.snapshot()
                            .commands
                            .iter()
                            .find_map(|command| match command {
                                V2DrawCommand::FillRect { rect, .. } => Some(*rect),
                                _ => None,
                            })?;
                    Some((item.origin, item.transform, rect))
                }
                _ => None,
            })
            .expect("replacement context menu has a retained panel fill");
        let (new_x, new_y) = second_geometry.1.transform_point(
            second_geometry.0.0 + second_geometry.2.x + second_geometry.2.width - 6.0,
            second_geometry.0.1 + second_geometry.2.y + second_geometry.2.height * 0.5,
        );
        renderer.render_packet(&backend, &view, &second_packet, false);
        let second_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert_context_menu_pixel_near(
            context_menu_pixel(&second_pixels, width, new_x, new_y, scale),
            [30, 70, 110, 255],
            "replacement panel fill",
        );
        assert_eq!(
            context_menu_pixel(
                &second_pixels,
                width,
                old_fill_point.0,
                old_fill_point.1,
                scale
            ),
            [0, 0, 0, 0],
            "the replaced menu leaves its previous panel area clear at scale {scale}"
        );
        assert_eq!(
            context_menu_pixel(
                &second_pixels,
                width,
                old_shadow_point.0,
                old_shadow_point.1,
                scale
            ),
            [0, 0, 0, 0],
            "the replaced menu leaves its previous shadow area clear at scale {scale}"
        );
    }
}

#[cfg(feature = "wgpu")]
#[test]
fn retained_pill_menu_records_separator_and_hover_pixels_at_one_and_two_x() {
    use aimer_cupid::WgpuBackend;
    use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
    use aimer_cupid::renderer::Renderer;

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some((device, queue)) = context_menu_gpu_device() else {
        if std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_some() {
            panic!("GPU adapter unavailable; run this test in a host GPU session");
        }
        eprintln!("skipping: GPU adapter unavailable");
        return;
    };

    const LOGICAL_WIDTH: u32 = 640;
    const LOGICAL_HEIGHT: u32 = 400;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    let backend = WgpuBackend::new(device.clone(), queue.clone());

    for scale in [1.0_f32, 2.0] {
        let width = LOGICAL_WIDTH * scale as u32;
        let height = LOGICAL_HEIGHT * scale as u32;
        let mut app = AimerApp::start_headless_with(
            RecordingWidget {
                builds: Arc::new(AtomicUsize::new(0)),
                cancels: Arc::new(AtomicUsize::new(0)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(width, height),
                scale_factor: f64::from(scale),
            },
        );
        app.render_frame();
        let separator = Color::Rgba(255, 40, 20, 255);
        let style = ContextMenuStyle::pill()
            .separator_color(separator)
            .highlight_color(Color::Rgba(20, 220, 40, 255));
        let _handle = ContextMenu::new()
            .shape(ContextMenuShape::Pill)
            .style(style)
            .at(Vec2d { x: 100.0, y: 100.0 })
            .items(vec![
                ContextMenuItem::new("Copy"),
                ContextMenuItem::new("Paste"),
                ContextMenuItem::new("Select All"),
            ])
            .show();
        let (_, first_packet) = direct_headless_frame_packet(&mut app);
        let first_plan = first_packet.render_plan().unwrap();
        let panel_id = retained_element_id_named(&app, "ContextMenu").unwrap();
        let rows_id = retained_element_id_named(&app, "ContextMenuRows").unwrap();
        let panel_node = app.app.render_node_for_element(panel_id).unwrap();
        let rows_node = app.app.render_node_for_element(rows_id).unwrap();
        let panel_revision = first_plan.local_v2_revision(panel_node).unwrap();
        assert!(first_plan.local_v2_revision(rows_node).is_none());
        let tree = app.app.render_tree();
        let mut row_nodes = retained_element_ids_named(&app, "ContextMenuRow")
            .into_iter()
            .map(|id| {
                let node = app.app.render_node_for_element(id).unwrap();
                (
                    node,
                    tree.element_bounds(node).unwrap(),
                    first_plan.local_v2_revision(node).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(row_nodes.len(), 3);
        row_nodes.sort_by(|left, right| left.1.x.total_cmp(&right.1.x));
        let separator_node = row_nodes[1].0;
        let separator_rect = tree
            .draw_list_snapshot(separator_node)
            .unwrap()
            .commands
            .iter()
            .find_map(|command| match command {
                V2DrawCommand::FillRect { rect, color, .. }
                    if *color == aimer_cupid::utilities::Color::rgba8(255, 40, 20, 255) =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("the second pill row records its separator as a local fill");
        let (origin, transform) = tree
            .render_all()
            .iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == separator_node => {
                    Some((item.origin, item.transform))
                }
                _ => None,
            })
            .expect("the second pill row has a retained draw operation");
        let _ = app.window.take_redraw_request();

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer retained pill-menu pixel target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut renderer = Renderer::new(&backend, FORMAT);
        renderer.render_packet(&backend, &view, &first_packet, false);
        let initial_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        let row_sample = |bounds: aimer_cupid::draw_cmd_v2::Rect| Vec2d {
            x: bounds.x + 12.0,
            y: bounds.y + bounds.height * 0.5,
        };
        let initial_row_pixels = row_nodes
            .iter()
            .map(|(_, bounds, _)| {
                let sample = row_sample(*bounds);
                context_menu_pixel(&initial_pixels, width, sample.x, sample.y, scale)
            })
            .collect::<Vec<_>>();
        let (separator_x, separator_y) = transform.transform_point(
            origin.0 + separator_rect.x + separator_rect.width * 0.5,
            origin.1 + separator_rect.y + separator_rect.height * 0.5,
        );
        let initial_separator =
            context_menu_pixel(&initial_pixels, width, separator_x, separator_y, scale);
        assert!(
            initial_separator[0] > 180
                && initial_separator[1] < 90
                && initial_separator[2] < 70
                && initial_separator[3] > 220,
            "pill separator is visible at scale {scale}: {initial_separator:?}"
        );

        let first_row = row_sample(row_nodes[0].1);
        dispatch_headless_event(
            &mut app,
            first_row,
            &ElementEvent::PointerMove(PointerInfo::mouse(
                first_row,
                aimer_events::pointer::PointerButton::Primary,
            )),
        );
        assert!(
            app.window.take_redraw_request(),
            "pill-row hover schedules a frame"
        );
        let rebuild_before_hover = aimer_widget::rebuild_invalidation_generation();
        let layout_before_hover = aimer_widget::layout_invalidation_generation();
        let (_, hover_packet) = direct_headless_frame_packet(&mut app);
        let hover_plan = hover_packet.render_plan().unwrap();
        assert_eq!(
            hover_plan.local_v2_revision(panel_node),
            Some(panel_revision)
        );
        assert_eq!(hover_plan.local_v2_revision(rows_node), None);
        assert_eq!(
            hover_plan.local_v2_revision(row_nodes[0].0),
            Some(row_nodes[0].2 + 1),
            "hover changes only the first pill row"
        );
        assert_eq!(
            hover_plan.local_v2_revision(row_nodes[1].0),
            Some(row_nodes[1].2),
            "the second row stays retained"
        );
        assert_eq!(
            hover_plan.local_v2_revision(row_nodes[2].0),
            Some(row_nodes[2].2),
            "the third row stays retained"
        );
        let concurrent_global_invalidation = rebuild_before_hover
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_hover != aimer_widget::layout_invalidation_generation();
        if !concurrent_global_invalidation {
            let damage = hover_packet.metadata().damage();
            assert!(!damage.is_full());
            let bounds = row_nodes[0].1;
            assert!(
                damage.regions().iter().all(|region| {
                    region.x as f32 >= bounds.x * scale - 1.0
                        && region.y as f32 >= bounds.y * scale - 1.0
                        && (region.x + region.width) as f32
                            <= (bounds.x + bounds.width) * scale + 1.0
                        && (region.y + region.height) as f32
                            <= (bounds.y + bounds.height) * scale + 1.0
                }),
                "first pill hover escaped its row bounds: {:?}",
                damage.regions()
            );
        }
        renderer.render_packet(&backend, &view, &hover_packet, false);
        let hovered_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert_context_menu_pixel_near(
            context_menu_pixel(&hovered_pixels, width, first_row.x, first_row.y, scale),
            [20, 220, 40, 255],
            "pill first-row hover",
        );
        let hovered_separator =
            context_menu_pixel(&hovered_pixels, width, separator_x, separator_y, scale);
        assert!(
            hovered_separator[0] > 180
                && hovered_separator[1] < 90
                && hovered_separator[2] < 70
                && hovered_separator[3] > 220,
            "pill separator stays visible under hover at scale {scale}: {hovered_separator:?}"
        );

        let second_row = row_sample(row_nodes[1].1);
        dispatch_headless_event(
            &mut app,
            second_row,
            &ElementEvent::PointerMove(PointerInfo::mouse(
                second_row,
                aimer_events::pointer::PointerButton::Primary,
            )),
        );
        assert!(
            app.window.take_redraw_request(),
            "moving pill hover schedules a frame"
        );
        let rebuild_before_transfer = aimer_widget::rebuild_invalidation_generation();
        let layout_before_transfer = aimer_widget::layout_invalidation_generation();
        let (_, transfer_packet) = direct_headless_frame_packet(&mut app);
        let transfer_plan = transfer_packet.render_plan().unwrap();
        assert_eq!(
            transfer_plan.local_v2_revision(panel_node),
            Some(panel_revision)
        );
        assert_eq!(transfer_plan.local_v2_revision(rows_node), None);
        assert_eq!(
            transfer_plan.local_v2_revision(row_nodes[0].0),
            Some(row_nodes[0].2 + 2),
            "the old pill row clears its highlight"
        );
        assert_eq!(
            transfer_plan.local_v2_revision(row_nodes[1].0),
            Some(row_nodes[1].2 + 1),
            "the new pill row receives the highlight"
        );
        assert_eq!(
            transfer_plan.local_v2_revision(row_nodes[2].0),
            Some(row_nodes[2].2),
            "the third pill row remains retained"
        );
        let concurrent_transfer_invalidation = rebuild_before_transfer
            != aimer_widget::rebuild_invalidation_generation()
            || layout_before_transfer != aimer_widget::layout_invalidation_generation();
        if !concurrent_transfer_invalidation {
            let damage = transfer_packet.metadata().damage();
            assert!(!damage.is_full());
            let left = row_nodes[0].1.x;
            let top = row_nodes[0].1.y;
            let right = row_nodes[1].1.x + row_nodes[1].1.width;
            let bottom = row_nodes[1].1.y + row_nodes[1].1.height;
            assert!(
                damage.regions().iter().all(|region| {
                    region.x as f32 >= left * scale - 1.0
                        && region.y as f32 >= top * scale - 1.0
                        && (region.x + region.width) as f32 <= right * scale + 1.0
                        && (region.y + region.height) as f32 <= bottom * scale + 1.0
                }),
                "pill hover-transfer damage escaped the old/new rows: {:?}",
                damage.regions()
            );
        }
        renderer.render_packet(&backend, &view, &transfer_packet, false);
        let transfer_pixels = read_context_menu_pixels(&device, &queue, &target, width, height);
        assert_eq!(
            context_menu_pixel(&transfer_pixels, width, first_row.x, first_row.y, scale),
            initial_row_pixels[0],
            "moving to the next pill row clears the old highlight"
        );
        assert_context_menu_pixel_near(
            context_menu_pixel(&transfer_pixels, width, second_row.x, second_row.y, scale),
            [20, 220, 40, 255],
            "pill second-row hover",
        );
        let third_row = row_sample(row_nodes[2].1);
        assert_eq!(
            context_menu_pixel(&transfer_pixels, width, third_row.x, third_row.y, scale),
            initial_row_pixels[2],
            "moving between pill rows leaves the third row unchanged"
        );
        let transfer_separator =
            context_menu_pixel(&transfer_pixels, width, separator_x, separator_y, scale);
        assert!(
            transfer_separator[0] > 180
                && transfer_separator[1] < 90
                && transfer_separator[2] < 70
                && transfer_separator[3] > 220,
            "pill separator stays visible after hover transfer at scale {scale}: {transfer_separator:?}"
        );
    }
}
