//! A window of rows that scrolls: every frame some rows leave and some enter, so the structure
//! changes and the synchronization takes its slow path with thousands of nodes kept.
//!
//! The first test pins what the synchronization keeps and gives; the second is a timing harness
//! to run by hand and by itself (`cargo test -p aimer_scene sync_churn -- --ignored --nocapture`),
//! since benchmarks must not compete with other work.

use super::*;

const PER_ROW: usize = 4;

fn bounds(y: f32) -> Rect {
    Rect::new(0.0, y, 100.0, 20.0)
}

/// The specs of rows `first..first + count` under a list under a root, a row having a node and three
/// leaves, each existing as `known` says (the ids of the nodes of each row the tree has).
fn window(first: usize, count: usize, known: &HashMap<usize, Vec<RenderNodeId>>, root: Option<RenderNodeId>, list: Option<RenderNodeId>) -> Vec<RenderNodeSpec> {
    let mut specs = vec![RenderNodeSpec::new(root, None, bounds(0.0)), RenderNodeSpec::new(list, Some(0), bounds(0.0))];
    for row in first..first + count {
        let ids = known.get(&row);
        let at = specs.len();
        let y = row as f32 * 20.0;
        specs.push(RenderNodeSpec::new(ids.map(|ids| ids[0]), Some(1), bounds(y)));
        for leaf in 1..PER_ROW {
            specs.push(RenderNodeSpec::new(ids.map(|ids| ids[leaf]), Some(at), bounds(y)));
        }
    }
    specs
}

fn remember(first: usize, count: usize, ids: &[RenderNodeId]) -> HashMap<usize, Vec<RenderNodeId>> {
    (0..count).map(|row| (first + row, ids[2 + row * PER_ROW..2 + (row + 1) * PER_ROW].to_vec())).collect()
}

#[test]
fn rows_that_stay_keep_their_nodes_and_rows_that_leave_or_enter_are_gone_or_new() {
    let tree = RenderTree::new();
    let none = HashMap::new();
    let ids = tree.sync_structure(&window(0, 6, &none, None, None)).unwrap();
    let (root, list) = (ids[0], ids[1]);
    let known = remember(0, 6, &ids);
    // The list scrolls by two rows: rows 0 and 1 leave, 6 and 7 enter.
    let next = tree.sync_structure(&window(2, 6, &known, Some(root), Some(list))).unwrap();
    assert_eq!(&next[..2], &[root, list], "the root and the list are the nodes they were");
    for (at, row) in (2..6).enumerate() {
        assert_eq!(next[2 + at * PER_ROW..2 + (at + 1) * PER_ROW], known[&row][..], "row {row} kept its four nodes");
    }
    for gone in [0, 1] {
        for id in &known[&gone] {
            assert!(tree.parent_of(*id).is_err(), "row {gone} left the tree");
        }
    }
    let fresh: Vec<_> = next[2 + 4 * PER_ROW..].to_vec();
    assert_eq!(fresh.len(), 2 * PER_ROW);
    for id in &fresh {
        assert!(!ids.contains(id), "a node of a new row has a new id");
        assert!(tree.parent_of(*id).is_ok());
    }
    // Parents are what the specs said, for kept nodes and new ones.
    assert_eq!(tree.parent_of(next[2 + 3 * PER_ROW]).unwrap(), Some(list));
    assert_eq!(tree.parent_of(next[2 + 3 * PER_ROW + 1]).unwrap(), Some(next[2 + 3 * PER_ROW]));
    assert_eq!(tree.parent_of(root).unwrap(), None);
    // A node's paint survives being kept: a list recorded on a row that stays is still there.
    let kept = known[&3][0];
    let before = tree.draw_list_snapshot(kept).unwrap().revision;
    let again = tree.sync_structure(&window(3, 6, &remember(2, 6, &next), Some(root), Some(list))).unwrap();
    assert_eq!(again[2 + PER_ROW..2 + 2 * PER_ROW][0], next[2 + 2 * PER_ROW], "row 4 is the fourth node it was");
    assert_eq!(tree.draw_list_snapshot(kept).unwrap().revision, before, "the retained list is untouched");
}

#[test]
#[ignore = "a timing harness: run by itself, in the debug build"]
fn timing_of_a_window_of_rows_that_scrolls() {
    const ROWS: usize = 1000;
    const STEP: usize = 20;
    const FRAMES: usize = 200;
    let tree = RenderTree::new();
    let none = HashMap::new();
    let ids = tree.sync_structure(&window(0, ROWS, &none, None, None)).unwrap();
    let (root, list) = (ids[0], ids[1]);
    let mut known = remember(0, ROWS, &ids);
    let mut first = 0;
    let mut elapsed = std::time::Duration::ZERO;
    for _ in 0..FRAMES {
        let specs = window(first + STEP, ROWS, &known, Some(root), Some(list));
        let started = std::time::Instant::now();
        let ids = tree.sync_structure(&specs).unwrap();
        elapsed += started.elapsed();
        first += STEP;
        known = remember(first, ROWS, &ids);
        let _ = tree.take_damage();
    }
    println!("sync_structure, {ROWS} rows of {PER_ROW} nodes, {STEP} rows replaced: {:?} per frame", elapsed / FRAMES as u32);
}
