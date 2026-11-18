use std::cmp::Ordering;

use aimer_sum_tree::{Dimension, Item, Summary};

/// One thing the layout places: a hard line, or a segment of a hard line too long to lay out in
/// one piece. Its height starts as an estimate and becomes exact once it has been laid out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Unit {
    pub(crate) bytes: usize,
    pub(crate) graphemes: u32,
    pub(crate) height: f32,
    /// Whether `height` was measured from a layout and not estimated.
    pub(crate) exact: bool,
    /// Whether the unit ends in a line feed, which takes no room.
    pub(crate) feed: bool,
    /// Whether the unit ends a hard line: it ends in a line feed, or it is the last of the text.
    pub(crate) ends_line: bool,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct UnitSummary {
    pub(crate) bytes: usize,
    pub(crate) graphemes: usize,
    pub(crate) height: f64,
    pub(crate) hard_lines: usize,
}

impl Summary for UnitSummary {
    fn add(&mut self, other: &Self) {
        self.bytes += other.bytes;
        self.graphemes += other.graphemes;
        self.height += other.height;
        self.hard_lines += other.hard_lines;
    }
}

impl Item for Unit {
    type Summary = UnitSummary;

    fn summary(&self) -> UnitSummary {
        UnitSummary {
            bytes: self.bytes,
            graphemes: self.graphemes as usize,
            height: f64::from(self.height),
            hard_lines: usize::from(self.ends_line),
        }
    }
}

macro_rules! dimension {
    ($name:ident, $number:ty, $field:ident) => {
        #[derive(Clone, Copy, Default, Debug, PartialEq)]
        pub(crate) struct $name(pub(crate) $number);

        impl Dimension<UnitSummary> for $name {
            fn add_summary(&mut self, summary: &UnitSummary) {
                self.0 += summary.$field as $number;
            }
        }
    };
}

dimension!(ByteStart, usize, bytes);
dimension!(GraphemeStart, usize, graphemes);
dimension!(HardLines, usize, hard_lines);
dimension!(Top, f64, height);

impl Eq for ByteStart {}
impl Eq for GraphemeStart {}
impl Eq for HardLines {}
impl Eq for Top {}

macro_rules! ordered {
    ($name:ident) => {
        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
    };
}

ordered!(ByteStart);
ordered!(GraphemeStart);
ordered!(HardLines);
ordered!(Top);

impl Ord for ByteStart {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl Ord for GraphemeStart {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl Ord for HardLines {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl Ord for Top {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}
