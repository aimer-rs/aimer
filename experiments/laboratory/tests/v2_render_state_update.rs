use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::draw_cmd::DrawList;
use aimer_cupid::draw_cmd_v2::DrawCommand as V2DrawCommand;
use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
use aimer_laboratory::v2_render::{CounterSimulation, RenderOp};

#[test]
fn state_update_relayouts_both_row_children_and_rerecords_only_the_counter() {
    let mut simulation = CounterSimulation::new().unwrap();
    let elements = simulation.element_ids();
    let initial_bounds = simulation.counter_row_bounds().unwrap();
    assert_eq!(initial_bounds[0].width, 128.0);
    assert_eq!(initial_bounds[1].x, 508.0);

    assert!(
        simulation
            .dispatch_pointer_down((100.0, 100.0))
            .unwrap()
            .is_none()
    );
    assert_eq!(simulation.count(), 9);

    let frame = simulation
        .dispatch_pointer_down((400.0, 280.0))
        .unwrap()
        .unwrap();
    let updated_bounds = simulation.counter_row_bounds().unwrap();

    assert_eq!(simulation.count(), 10);
    assert_eq!(simulation.handled_pointer_events(), 1);
    assert_eq!(updated_bounds[0].width, 142.0);
    assert_eq!(updated_bounds[1].x, 522.0);
    assert_eq!(frame.damage.len(), 1);
    assert_eq!(frame.damage[0].x, 372.0);
    assert_eq!(frame.damage[0].y, 262.0);
    assert_eq!(frame.damage[0].width, 214.0);
    assert_eq!(
        elements
            .iter()
            .map(|element| simulation.tree().draw_list_revision(*element).unwrap())
            .collect::<Vec<_>>(),
        [1, 1, 1, 2, 1]
    );

    let draw_order = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.element),
            RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(draw_order, elements);
}

/// Every command of every drawn item in `operations`, as recorded.
fn recorded_commands(operations: &[RenderOp]) -> Vec<V2DrawCommand> {
    operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.snapshot().commands.to_vec()),
            RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => None,
        })
        .flatten()
        .collect()
}

#[test]
fn counter_simulation_builds_retained_packets_for_initial_and_incremental_frames() {
    let simulation = CounterSimulation::new().unwrap();
    let metadata = || FrameRenderMetadata::new(1.0, 17, 2, 3, 4, DamageSet::new(1000, 800));
    let packet = |frame| {
        FramePacket::from_v2_direct_with_frame(
            frame,
            metadata(),
            Frame::new(DrawList::new(), 1000, 800),
        )
        .unwrap()
    };

    let initial_frame = simulation.initial_frame().unwrap();
    let fills = recorded_commands(&initial_frame.operations)
        .iter()
        .filter(|command| matches!(command, V2DrawCommand::FillRect { .. }))
        .count();
    assert_eq!(fills, 3);
    let initial = packet(initial_frame);
    assert_eq!((initial.frame().width, initial.frame().height), (1000, 800));
    assert!(initial.metadata().damage().is_full());
    assert!(initial.render_plan().is_some_and(|plan| plan.is_complete()));
    assert!(
        initial.frame().draw_list.commands().is_empty(),
        "local lists are not flattened into a frame-wide buffer"
    );

    let updated = simulation.increment().unwrap();
    let text_updated = recorded_commands(&updated.operations).iter().any(
        |command| matches!(command, V2DrawCommand::DrawText { text, .. } if &**text == "Count: 10"),
    );
    let packet = packet(updated);
    assert_eq!(
        packet.metadata().damage().regions(),
        &[aimer_cupid::damage_region::DamageRect::new(
            372, 262, 214, 48
        )]
    );
    assert!(text_updated);
}
