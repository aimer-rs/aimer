//! A flex container is asked for its layout under more than one constraint in a
//! frame, and each answer has to stay cached.
//!
//! The pass that sizes a scrolled column asks it with an unbounded height, and
//! the pass that places and paints it asks with the height it measured. Holding
//! a single table made those two passes throw each other's answer away, so every
//! container below a scroll view measured all of its children three times per
//! frame instead of once.

use std::cell::Cell;
use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::LayoutElement;

use crate::flex::raw_flex::RawFlex;
use crate::flex::test_support::{CountingChild, dummy_build_context};
use crate::flex::FlexDirection;

const CHILDREN: usize = 5;

fn column(measured: &Rc<Cell<usize>>) -> RawFlex {
    let drawn = Rc::new(Cell::new(0));
    let children = (0..CHILDREN)
        .map(|_| CountingChild::boxed_new(40.0, 10.0, measured, &drawn))
        .collect();
    RawFlex::new(FlexDirection::Column, children, "Column")
}

/// The constraints one container meets in a frame: the extent it measured and an
/// unbounded one, each at its own width and at an unbounded one.
fn constraints() -> [BuildContext<'static>; 4] {
    let base = dummy_build_context(100.0, 100.0, None);
    [
        (100.0, 100.0),
        (100.0, f32::MAX),
        (f32::MAX, 100.0),
        (f32::MAX, f32::MAX),
    ]
    .map(|(width, height)| {
        let mut ctx = base.clone();
        ctx.box_constraint.max_width = width;
        ctx.box_constraint.max_height = height;
        ctx
    })
}

#[test]
fn a_flex_asked_under_several_constraints_in_turn_measures_its_children_once_for_each() {
    let measured = Rc::new(Cell::new(0));
    let column = column(&measured);
    let contexts = constraints();

    for ctx in &contexts {
        column.computed_size(ctx);
    }
    let after_first_round = measured.get();
    assert!(after_first_round >= CHILDREN, "the first round measured nothing");
    for _ in 0..3 {
        for ctx in &contexts {
            column.computed_size(ctx);
        }
    }

    assert_eq!(
        measured.get(),
        after_first_round,
        "the children were measured again by a constraint the container had already answered"
    );
}

#[test]
fn each_constraint_gets_the_answer_a_fresh_container_would_give() {
    let measured = Rc::new(Cell::new(0));
    let column = column(&measured);
    let tight = dummy_build_context(100.0, 30.0, None);
    let loose = dummy_build_context(100.0, 1000.0, None);
    column.computed_size(&tight);
    column.computed_size(&loose);

    for ctx in [&tight, &loose, &tight, &loose] {
        let fresh = self::column(&Rc::new(Cell::new(0)));
        assert_eq!(column.computed_size(ctx), fresh.computed_size(ctx));
    }
}
