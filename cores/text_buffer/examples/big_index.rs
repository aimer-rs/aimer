//! What the layout index costs at the sizes of files: indexing a part at a time, one edit, and
//! looking up a height, for a text of many lines and for one line of the whole size.
//!
//! ```text
//! BENCH_MB=5,50 cargo run -p aimer_text_buffer --example big_index
//! ```

use std::time::Instant;

use aimer_text_buffer::{LayoutIndex, LayoutMetrics, TextBuffer};

const METRICS: LayoutMetrics = LayoutMetrics { content_width: 800.0, line_height: 20.0, advance: 8.0 };

fn resident_megabytes() -> f64 {
    let output = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().expect("ps runs");
    String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

fn run(kind: &str, text: &str) {
    let before = resident_megabytes();
    let mut buffer = TextBuffer::from_text(text);
    let text_held = resident_megabytes() - before;
    let mut index = LayoutIndex::new(METRICS);
    // The first window: what opening costs before the rest is indexed.
    let started = Instant::now();
    index.index_more(buffer.text(), 256 * 1024);
    let first = started.elapsed();
    let started = Instant::now();
    let mut slices = 0;
    while !index.index_more(buffer.text(), 4 * 1024 * 1024) {
        slices += 1;
    }
    let rest = started.elapsed();

    println!("       memory: text {text_held:.0} MB, text + index {:.0} MB ({} units)", resident_megabytes() - before, index.len());
    let edits = 500;
    let started = Instant::now();
    let mut touched = 0;
    let mut each = Vec::new();
    for step in 0..edits {
        let at = buffer.text().floor_grapheme_boundary(buffer.text().len() / 2 + step * 5);
        let delta = buffer.replace_continuing(at..at, "x");
        let reindexed = index.apply_edit(&delta, buffer.text());
        touched += reindexed.added.max(reindexed.removed);
        each.push(reindexed.added.max(reindexed.removed));
    }
    let edit = started.elapsed() / edits as u32;

    let started = Instant::now();
    let mut checksum = 0.0;
    for step in 0..edits {
        let unit = index.unit_at_y(step as f64 * index.total_height() / edits as f64);
        checksum += index.top(unit) + f64::from(index.height(unit));
    }
    let lookup = started.elapsed() / edits as u32;
    each.sort_unstable();
    println!("       units remade per edit: median {} p90 {} p99 {} max {}", each[each.len() / 2], each[each.len() * 9 / 10], each[each.len() * 99 / 100], each[each.len() - 1]);
    println!(
        "{kind:<6} {:>9} units  first window {first:>8.2?}  rest ({slices} slices of 4 MB) {rest:>9.2?}  edit+reindex {edit:>9.2?} ({:.1} units remade)  y lookup {lookup:>8.2?}  ({checksum:.0})",
        index.len(),
        touched as f64 / edits as f64,
    );
}

/// Text of about `bytes`: lines of random words, so that nothing repeats the way a repeated
/// sentence does (a periodic text can keep two sequences of cuts apart for ever).
fn text_of(bytes: usize) -> String {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |below: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % below
    };
    let mut text = String::with_capacity(bytes + 100);
    while text.len() < bytes {
        for _ in 0..4 + next(12) {
            for _ in 0..2 + next(8) {
                text.push((b'a' + next(26) as u8) as char);
            }
            text.push(' ');
        }
        text.push('\n');
    }
    text
}

fn main() {
    let sizes = std::env::var("BENCH_MB").unwrap_or_else(|_| "5,50".to_owned());
    for megabytes in sizes.split(',').filter_map(|size| size.trim().parse::<usize>().ok()) {
        let lines = text_of(megabytes * 1_000_000);
        println!("{megabytes} MB:");
        run("lines", &lines);
        run("giant", &lines.replace('\n', " "));
    }
}
