//! What the buffer costs at the sizes of files, with the chunk size it really uses: building it,
//! an edit in the middle and anywhere, a snapshot, a position by line, and memory.
//!
//! ```text
//! BENCH_MB=5,50 cargo run -p aimer_text_buffer --example big_text
//! ```

use std::time::Instant;

use aimer_text_buffer::TextBuffer;

fn resident_megabytes() -> f64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

fn main() {
    let sizes = std::env::var("BENCH_MB").unwrap_or_else(|_| "5,50".to_owned());
    for megabytes in sizes.split(',').filter_map(|size| size.trim().parse::<usize>().ok()) {
        let line = "Section heading, with some words in it that make a line of about this length.\n";
        let text = line.repeat(megabytes * 1_000_000 / line.len());
        let before = resident_megabytes();
        let started = Instant::now();
        let mut buffer = TextBuffer::from_text(&text);
        let built = started.elapsed();
        let held = resident_megabytes() - before;

        let edits = 1000;
        let started = Instant::now();
        for step in 0..edits {
            let at = buffer.text().len() / 2 + step * 3;
            let at = buffer.text().floor_grapheme_boundary(at);
            buffer.replace_continuing(at..at, "x");
        }
        let typing = started.elapsed() / edits as u32;

        let started = Instant::now();
        let snapshot = buffer.snapshot();
        let cloned = started.elapsed();

        let started = Instant::now();
        let mut checksum = 0;
        for step in 0..edits {
            checksum += snapshot.line_to_byte(step * 997 % snapshot.line_count());
            checksum += snapshot.byte_to_line(step * 99_991 % snapshot.len());
        }
        let lookup = started.elapsed() / (2 * edits as u32);

        let mut scattered = TextBuffer::from_text(&text);
        let mut state = 5u64;
        let started = Instant::now();
        for _ in 0..edits {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = scattered.text().floor_grapheme_boundary(state as usize % scattered.text().len());
            scattered.replace(at..at, "y");
        }
        let anywhere = started.elapsed() / edits as u32;

        let started = Instant::now();
        let mut undone = 0;
        while scattered.undo().is_some() {
            undone += 1;
        }
        let undo = started.elapsed() / undone.max(1) as u32;
        let after = resident_megabytes() - before;
        println!(
            "{megabytes:>3} MB: build {built:>9.2?}  type {typing:>9.2?}  edit anywhere {anywhere:>9.2?}  undo {undo:>9.2?}  snapshot {cloned:>7.2?}  line lookup {lookup:>8.2?}  held {held:>5.0} MB, after 2x{edits} edits {after:>5.0} MB ({checksum})"
        );
    }
}
