//! Build-cost harness for the choice controls. Ignored by default; run with
//! `cargo nextest run -p aimer_selection --run-ignored ignored-only profile --no-capture`
//! in the debug profile, which is the one the framework optimises for.

mod support;

use std::time::Instant;

use aimer_selection::{Checkbox, ChoiceOption, RadioGroup, Select};
use aimer_widget::{Drawable, LayoutElement, State, StatefulWidget, Widget};
use support::context_with_height;

fn options(count: usize) -> Vec<ChoiceOption<usize>> {
    (0..count)
        .map(|index| {
            ChoiceOption::new(
                format!("key-{index}"),
                format!("Option number {index}"),
                index,
            )
        })
        .collect()
}

fn report(name: &str, iterations: u32, started: Instant) {
    let total = started.elapsed();
    println!(
        "{name}: {:.3} ms/iter over {iterations}",
        total.as_secs_f64() * 1000.0 / iterations as f64
    );
}

#[ignore = "profile harness"]
#[tokio::test]
async fn profile_choice_controls() {
    let ctx = context_with_height(4000.0);

    let iterations = 2000;
    let state = Checkbox::new().label("Accept").create_state();
    let started = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(state.build(&ctx).to_element(&ctx));
    }
    report("checkbox rebuild", iterations, started);

    let iterations = 20;
    let started = Instant::now();
    for _ in 0..iterations {
        let group = RadioGroup::new().options(options(500)).selected(Some(3));
        let element = group.to_element(&ctx);
        element.update(&ctx);
        std::hint::black_box(element.layout(&ctx));
        element.update(&ctx);
    }
    report("radio group 500 mount+frames", iterations, started);

    let started = Instant::now();
    for _ in 0..iterations {
        let select = Select::new().options(options(500)).selected(Some(3));
        let element = select.to_element(&ctx);
        element.update(&ctx);
        std::hint::black_box(element.layout(&ctx));
        element.update(&ctx);
    }
    report("select (closed) 500 mount+frames", iterations, started);
}
