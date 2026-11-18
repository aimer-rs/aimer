//! The tree against a `Vec`, which it must always agree with.

use std::cmp::Ordering;

use crate::{Bias, Dimension, Item, SumTree, Summary};

/// An item worth `weight` to the dimension [`Weight`], and a count of one.
#[derive(Clone, Debug, PartialEq)]
struct Number {
    id: u32,
    weight: u32,
}

#[derive(Clone, Default, Debug, PartialEq)]
struct Totals {
    weight: u64,
    items: usize,
    /// A maximum, to be sure a summary that is not a sum combines too.
    heaviest: u32,
}

impl Summary for Totals {
    fn add(&mut self, other: &Self) {
        self.weight += other.weight;
        self.items += other.items;
        self.heaviest = self.heaviest.max(other.heaviest);
    }
}

impl Item for Number {
    type Summary = Totals;
    fn summary(&self) -> Totals {
        Totals { weight: u64::from(self.weight), items: 1, heaviest: self.weight }
    }
}

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Weight(u64);

impl Dimension<Totals> for Weight {
    fn add_summary(&mut self, summary: &Totals) {
        self.0 += summary.weight;
    }
}

/// A small deterministic generator, so a failure reproduces.
struct Random(u64);

impl Random {
    fn below(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % below as u64) as usize
    }
}

fn numbers(random: &mut Random, count: usize, next_id: &mut u32) -> Vec<Number> {
    (0..count)
        .map(|_| {
            *next_id += 1;
            // Weight zero is allowed: an item that takes no room still has an index.
            Number { id: *next_id, weight: random.below(5) as u32 }
        })
        .collect()
}

fn totals(items: &[Number]) -> Totals {
    let mut total = Totals::default();
    for item in items {
        total.add(&item.summary());
    }
    total
}

/// Everything the tree can be asked, checked against the plain vector.
#[track_caller]
fn assert_agrees(tree: &SumTree<Number>, oracle: &[Number], random: &mut Random, what: &str) {
    tree.check_invariants();
    assert_eq!(tree.len(), oracle.len(), "{what}: len");
    assert_eq!(tree.summary(), &totals(oracle), "{what}: summary");
    assert_eq!(tree.iter().cloned().collect::<Vec<_>>(), oracle, "{what}: iter");
    for _ in 0..6 {
        if oracle.is_empty() {
            break;
        }
        let from = random.below(oracle.len());
        assert_eq!(tree.iter_from(from).cloned().collect::<Vec<_>>(), oracle[from..], "{what}: iter_from {from}");
        assert_eq!(tree.get(from), oracle.get(from), "{what}: get {from}");
    }
    assert_eq!(tree.get(oracle.len()), None, "{what}: get past the end");
    for index in (0..=oracle.len() + 1).step_by(1 + oracle.len() / 40) {
        let expected: u64 = oracle.iter().take(index).map(|item| u64::from(item.weight)).sum();
        assert_eq!(tree.dimension_before::<Weight>(index).0, expected, "{what}: dimension_before {index}");
    }
    // A search by weight lands on the item a linear scan lands on, in both biases, for targets
    // before, on and past every boundary.
    let total = tree.summary().weight;
    for target in (0..8).map(|_| random.below(total as usize + 3) as u64).chain([0, total, total + 1]) {
        for bias in [Bias::Left, Bias::Right] {
            let mut start = 0u64;
            let mut found = oracle.len();
            let mut found_start = total;
            for (index, item) in oracle.iter().enumerate() {
                let end = start + u64::from(item.weight);
                let reaches = match end.cmp(&target) {
                    Ordering::Greater => true,
                    Ordering::Equal => bias == Bias::Left,
                    Ordering::Less => false,
                };
                if reaches {
                    found = index;
                    found_start = start;
                    break;
                }
                start = end;
            }
            let position = tree.seek(&Weight(target), bias);
            assert_eq!((position.index, position.start.0), (found, found_start), "{what}: seek {target} {bias:?}");
        }
    }
}

#[test]
fn an_empty_tree_has_nothing_and_answers_every_question() {
    let tree = SumTree::<Number>::new();
    assert!(tree.is_empty());
    assert_eq!(tree.len(), 0);
    assert_eq!(tree.get(0), None);
    assert_eq!(tree.iter().count(), 0);
    assert_eq!(tree.seek(&Weight(5), Bias::Left).index, 0);
    tree.check_invariants();
    let (left, right) = tree.split_at(0);
    assert!(left.is_empty() && right.is_empty());
}

#[test]
fn a_tree_built_from_items_agrees_at_every_size_through_several_levels() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    let mut next_id = 0;
    for count in (0..=40).chain([255, 256, 257, 300, 1000, 4096, 4097, 5000]) {
        let oracle = numbers(&mut random, count, &mut next_id);
        let tree: SumTree<Number> = oracle.iter().cloned().collect();
        assert_agrees(&tree, &oracle, &mut random, &format!("built from {count}"));
    }
}

#[test]
fn random_replacements_keep_the_tree_equal_to_the_vector_it_stands_for() {
    let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
    let mut next_id = 0;
    for round in 0..60 {
        let size = random.below(3000);
        let mut oracle = numbers(&mut random, size, &mut next_id);
        let mut tree: SumTree<Number> = oracle.iter().cloned().collect();
        for edit in 0..25 {
            let start = random.below(oracle.len() + 1);
            // Most edits are small; some take a large part, or all, of the tree out.
            let span = match random.below(8) {
                0 => oracle.len() - start,
                1 => random.below(oracle.len() - start + 1),
                _ => random.below(4).min(oracle.len() - start),
            };
            let end = start + span;
            let inserted_count = match random.below(6) {
                0 => 0,
                1 => random.below(600),
                _ => random.below(5),
            };
            let inserted = numbers(&mut random, inserted_count, &mut next_id);
            let before = tree.clone();
            let before_items = oracle.clone();

            oracle.splice(start..end, inserted.iter().cloned());
            tree.replace_range(start..end, inserted);

            let what = format!("round {round} edit {edit}: {start}..{end} -> {inserted_count}");
            assert_agrees(&tree, &oracle, &mut random, &what);
            assert_eq!(before.iter().cloned().collect::<Vec<_>>(), before_items, "{what}: the tree it was cloned from is unchanged");
        }
    }
}

#[test]
fn splitting_and_joining_gives_back_the_same_items_at_every_index() {
    let mut random = Random(0x1234_5678_9ABC_DEF1);
    let mut next_id = 0;
    let oracle = numbers(&mut random, 700, &mut next_id);
    let tree: SumTree<Number> = oracle.iter().cloned().collect();
    for index in (0..=oracle.len()).step_by(7).chain([699, 700]) {
        let (left, right) = tree.split_at(index);
        assert_agrees(&left, &oracle[..index], &mut random, &format!("left of {index}"));
        assert_agrees(&right, &oracle[index..], &mut random, &format!("right of {index}"));
        let mut joined = left;
        joined.append(right);
        assert_agrees(&joined, &oracle, &mut random, &format!("joined at {index}"));
    }
    let slice = tree.slice(100..350);
    assert_agrees(&slice, &oracle[100..350], &mut random, "slice");
}

#[test]
fn pushing_one_at_a_time_stays_balanced() {
    let mut tree = SumTree::new();
    for id in 0..20_000u32 {
        tree.push(Number { id, weight: 1 });
    }
    tree.check_invariants();
    assert_eq!(tree.len(), 20_000);
    // Sixteen to a node: 20,000 items fit in four levels. Joining keeps nodes at least half full
    // as it goes, so the tree is never much deeper than that.
    assert!(tree.height() <= 5, "height {}", tree.height());
    assert_eq!(tree.iter().map(|item| item.id).collect::<Vec<_>>(), (0..20_000).collect::<Vec<_>>());
}

#[test]
fn an_item_is_changed_in_place_without_touching_a_clone() {
    let mut random = Random(0xA5A5_5A5A_1234_4321);
    let mut next_id = 0;
    let mut oracle = numbers(&mut random, 1500, &mut next_id);
    let mut tree: SumTree<Number> = oracle.iter().cloned().collect();
    let before = tree.clone();
    for _ in 0..300 {
        let index = random.below(oracle.len());
        let weight = random.below(9) as u32;
        oracle[index].weight = weight;
        assert!(tree.update(index, |item| item.weight = weight));
    }
    assert!(!tree.update(1500, |_| unreachable!("there is no such item")));
    let mut random = Random(7);
    assert_agrees(&tree, &oracle, &mut random, "after the updates");
    assert_eq!(before.iter().map(|item| item.weight).sum::<u32>(), {
        let mut again = Random(0xA5A5_5A5A_1234_4321);
        let mut id = 0;
        numbers(&mut again, 1500, &mut id).iter().map(|item| item.weight).sum::<u32>()
    }, "the clone still has the weights it had");
}

#[test]
fn an_edit_shares_what_it_does_not_touch() {
    let items: Vec<Number> = (0..100_000u32).map(|id| Number { id, weight: 1 }).collect();
    let tree: SumTree<Number> = items.into_iter().collect();
    let mut edited = tree.clone();
    edited.replace_range(50_000..50_001, [Number { id: u32::MAX, weight: 1 }]);
    // Two trees that differ in one item are both whole and both correct.
    assert_eq!(tree.get(50_000).map(|item| item.id), Some(50_000));
    assert_eq!(edited.get(50_000).map(|item| item.id), Some(u32::MAX));
    assert_eq!(tree.len(), edited.len());
    edited.check_invariants();
}

#[test]
fn many_edits_do_not_make_the_tree_taller_than_its_size_needs() {
    let mut random = Random(0xFEED_FACE_0BAD_F00D);
    let mut tree: SumTree<Number> = (0..100_000u32).map(|id| Number { id, weight: 1 }).collect();
    let fresh_height = tree.height();
    for edit in 0..30_000 {
        let at = random.below(tree.len());
        // Mostly one in, one out; now and then a bigger cut, as a paste or a delete is.
        let removed = if edit % 500 == 0 { random.below(2000).min(tree.len() - at) } else { 1.min(tree.len() - at) };
        tree.replace_range(at..at + removed, [Number { id: 7, weight: 1 }, Number { id: 8, weight: 1 }]);
    }
    tree.check_invariants();
    assert!(tree.height() <= fresh_height + 2, "height {} after the edits, {fresh_height} when built", tree.height());
}

/// What an edit costs as the tree grows, which should be a hair, not a copy of the tree:
///
/// ```text
/// cargo nextest run -p aimer_sum_tree --run-ignored only -E 'test(bench_edits)' --no-capture
/// ```
#[test]
#[ignore = "a timing, not a check"]
fn bench_edits() {
    use std::time::Instant;
    for size in [10_000usize, 100_000, 1_000_000, 4_000_000] {
        let started = Instant::now();
        let mut tree: SumTree<Number> = (0..size as u32).map(|id| Number { id, weight: 3 }).collect();
        let built = started.elapsed();
        let mut random = Random(99);
        let started = Instant::now();
        let edits = 2000;
        for _ in 0..edits {
            let at = random.below(tree.len());
            tree.replace_range(at..at + 1, [Number { id: 1, weight: 3 }, Number { id: 2, weight: 3 }]);
        }
        let edit = started.elapsed() / edits;
        let started = Instant::now();
        let mut checksum = 0;
        for _ in 0..edits {
            checksum += tree.seek(&Weight(random.below(tree.summary().weight as usize) as u64), Bias::Right).index;
        }
        let seek = started.elapsed() / edits;
        let started = Instant::now();
        let snapshot = tree.clone();
        let cloned = started.elapsed();
        println!("{size:>8} items: build {built:>9.2?}  edit {edit:>9.2?}  seek {seek:>9.2?}  clone {cloned:>7.2?}  height {} ({checksum}, {})", tree.height(), snapshot.len());
    }
}
