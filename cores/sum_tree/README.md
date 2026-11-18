# aimer_sum_tree

A persistent B+tree whose every node carries a *summary* of the items below it.

A text buffer is a sequence of chunks, a layout index is a sequence of lines, and an editor asks
the same few questions of both: how many bytes (lines, pixels) come before this item, which item
holds this position, replace these items with those. A `SumTree` answers each in O(log n), and
its nodes are shared by `Arc`, so a clone is O(1) and an edit copies only the nodes on one path:
the old tree stays valid and unchanged, which is what makes cheap snapshots (undo, background
layout) possible.

```
use aimer_sum_tree::{Item, SumTree, Summary};

#[derive(Clone, Default)]
struct Bytes(usize);
impl Summary for Bytes {
    fn add(&mut self, other: &Self) { self.0 += other.0; }
}
#[derive(Clone)]
struct Chunk(&'static str);
impl Item for Chunk {
    type Summary = Bytes;
    fn summary(&self) -> Bytes { Bytes(self.0.len()) }
}

let tree: SumTree<Chunk> = [Chunk("hello "), Chunk("world")].into_iter().collect();
assert_eq!(tree.summary().0, 11);
```
