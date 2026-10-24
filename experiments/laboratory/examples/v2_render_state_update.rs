use aimer_laboratory::v2_render::{CounterSimulation, RenderOp, V2RenderError};

fn main() -> Result<(), V2RenderError> {
    let mut simulation = CounterSimulation::new()?;
    let elements = simulation.element_ids();

    println!("Initial count: {}", simulation.count());
    println!("Child-sized event bounds: {:?}", simulation.event_bounds()?);
    println!(
        "Multi-child row bounds: {:?}",
        simulation.counter_row_bounds()?
    );
    print_revisions(&simulation, &elements)?;

    let _ = simulation.dispatch_pointer_down((100.0, 100.0))?;
    let frame = simulation
        .dispatch_pointer_down((400.0, 280.0))?
        .expect("the point is inside the counter text bounds");

    println!("\nAfter state update: count = {}", simulation.count());
    println!(
        "Child bounds after layout: {:?}",
        simulation.event_bounds()?
    );
    println!(
        "Multi-child row bounds: {:?}",
        simulation.counter_row_bounds()?
    );
    println!(
        "Handled pointer events: {}",
        simulation.handled_pointer_events()
    );
    println!("Damage: {:?}", frame.damage);
    print_revisions(&simulation, &elements)?;

    let draw_order = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.element.get()),
            RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => None,
        })
        .collect::<Vec<_>>();
    println!("Recomposed elements in paint order: {draw_order:?}");

    Ok(())
}

fn print_revisions(
    simulation: &CounterSimulation,
    elements: &[aimer_laboratory::v2_render::RenderNodeId; 5],
) -> Result<(), V2RenderError> {
    let revisions = elements
        .iter()
        .map(|element| simulation.tree().draw_list_revision(*element))
        .collect::<Result<Vec<_>, _>>()?;
    println!("Local draw-list revisions [C1, C2, C3, count, suffix]: {revisions:?}");
    Ok(())
}
