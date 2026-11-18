//! How long a field takes over a long note.
//!
//! Ignored by default, since these are timings and not checks. Run them with
//!
//! ```text
//! cargo test --release -p aimer_input bench_ -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Each size is laid out the first time, laid out again after a one-character edit
//! (what typing costs), and asked again with nothing changed (what a frame that
//! only scrolls costs), once as plain text and once with a highlighter that styles
//! headings and bold words, which takes the rich-text layout path a markdown note does.

use std::time::{Duration, Instant};

use aimer_cupid::draw_cmd::{Rect, RenderTree};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_widget::EventElement;
use aimer_style::TextOverflow;
use aimer_text::{SpanStyle, StyledRange};
use aimer_widget::base::Color;
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, LayoutElement};

use super::RawTextField;
use super::test_support::{commit, dummy_build_context, focused_multiline_field, key};
use crate::input_field::controller::TextFieldController;
use crate::input_field::highlight::TextHighlighter;

/// About `kilobytes` of markdown-looking text: a heading, a wrapping paragraph with
/// bold words, and two list items, over and over.
///
/// Every section is numbered, so no two chunks of it are alike: a note that repeats
/// one section would let anything that remembers text by its content answer most of
/// it from a single entry, which no real note allows.
fn note(kilobytes: usize) -> String {
    let section = "## A section heading\nSome **bold** words and `code` in a line, with a link and \
        enough text to wrap across the width of the editor pane a little.\n- item one\n- item two\n";
    let sections = kilobytes * 1000 / section.len() + 1;
    (0..sections)
        .map(|index| section.replacen("heading", &format!("heading {index}"), 1).replacen("link", &format!("link {index}"), 1))
        .collect()
}

/// The ranges of one line, relative to its start: the heading and every `**word**`.
fn markdown_line(line: &str) -> Vec<StyledRange> {
    let mut ranges = Vec::new();
    if line.starts_with("## ") {
        ranges.push(StyledRange::new(0..line.trim_end().len(), SpanStyle::new().font_size(24)));
    }
    let mut from = 0;
    while let Some(open) = line[from..].find("**") {
        let open = from + open;
        let Some(close) = line[open + 2..].find("**") else { break };
        let close = open + 2 + close + 2;
        ranges.push(StyledRange::new(
            open..close,
            SpanStyle::new().color(Color::Rgba(200, 30, 30, 255)),
        ));
        from = close;
    }
    ranges
}

/// Styles every heading line and every `**word**`, given the whole text.
fn markdown_like() -> TextHighlighter {
    TextHighlighter::new(|text| {
        let mut ranges = Vec::new();
        let mut line_start = 0;
        for line in text.split_inclusive('\n') {
            ranges.extend(markdown_line(line).into_iter().map(|range| {
                StyledRange::new(range.range.start + line_start..range.range.end + line_start, range.style)
            }));
            line_start += line.len();
        }
        ranges
    })
}

/// The same styling a line at a time.
fn markdown_by_line() -> TextHighlighter {
    TextHighlighter::by_line((), |line: &str, _: &()| (markdown_line(line), ()))
}

fn field(text: &str, highlighter: Option<TextHighlighter>) -> (RawTextField, TextFieldController) {
    let controller = TextFieldController::with_initial(text);
    let mut field = focused_multiline_field(controller.clone(), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.highlighter = highlighter;
    (field, controller)
}

/// Lays the field out and paints it once, as a frame does.
///
/// `ctx` is the window's: a window keeps one canvas, and what it has learned of the
/// text (what it shaped, what it measured) from one frame to the next, so every
/// frame of a scenario is asked of the same one.
fn frame(ctx: &BuildContext<'static>, field: &RawTextField) -> Duration {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 800.0, 600.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    let started = Instant::now();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    started.elapsed()
}

#[test]
#[ignore = "a timing, not a check"]
fn bench_a_long_note() {
    for kilobytes in [50, 200, 800] {
        for highlighted in [false, true] {
            let text = note(kilobytes);
            let (field, controller) = field(&text, highlighted.then(markdown_like));
            let ctx = dummy_build_context(800.0, 600.0);

            let first = frame(&ctx, &field);
            let again = frame(&ctx, &field);

            // One character typed in the middle of the note.
            let mut edited = text.clone();
            edited.insert(text.len() / 2, 'x');
            controller.set_text(edited);
            let edit = frame(&ctx, &field);

            println!(
                "{kilobytes:>4} KB {:<11} first {first:>10.2?}   unchanged {again:>10.2?}   after an edit {edit:>10.2?}",
                if highlighted { "highlighted" } else { "plain" },
            );
        }
    }
}

/// Asks an unchanged field for a frame again and again, for a profiler to look at.
///
/// `BENCH_KB` sets the size of the note (800 by default) and `BENCH_FRAMES` how
/// many frames to ask for.
#[test]
#[ignore = "a timing, not a check"]
fn bench_unchanged_frames() {
    let read = |name: &str, default: usize| {
        std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
    };
    let text = note(read("BENCH_KB", 800));
    let (field, _controller) = field(&text, Some(markdown_like()));
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&ctx, &field);
    let frames = read("BENCH_FRAMES", 400);
    let started = Instant::now();
    for _ in 0..frames {
        frame(&ctx, &field);
    }
    println!("{frames} frames in {:.2?}", started.elapsed());
}

/// Types a character into the middle of a long note again and again, for a profiler
/// to look at. `BENCH_KB` and `BENCH_EDITS` set the size and how many edits, and
/// `BENCH_PLAIN=1` leaves the highlighter off.
#[test]
#[ignore = "a timing, not a check"]
fn bench_edits() {
    let read = |name: &str, default: usize| {
        std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
    };
    let mut text = note(read("BENCH_KB", 800));
    let highlighter = (read("BENCH_PLAIN", 0) == 0).then(markdown_like);
    let (field, controller) = field(&text, highlighter);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&ctx, &field);
    let edits = read("BENCH_EDITS", 40);
    let started = Instant::now();
    for _ in 0..edits {
        text.insert(text.len() / 2, 'x');
        controller.set_text(text.clone());
        frame(&ctx, &field);
    }
    println!("{edits} edits in {:.2?}", started.elapsed());
}

/// How long the highlighter alone takes over a long note, for the first text and for
/// the text after a character typed in the middle: once given the whole text, and
/// once a line at a time. They do the same work on every line they run.
#[test]
#[ignore = "a timing, not a check"]
fn bench_highlighting() {
    for kilobytes in [50, 200, 800] {
        let text = note(kilobytes);
        let mut edited = text.clone();
        edited.insert(text.len() / 2, 'x');
        for (name, highlighter) in [("whole text", markdown_like()), ("by line", markdown_by_line())] {
            let started = Instant::now();
            let first = highlighter.highlight(&text);
            let first_time = started.elapsed();
            let started = Instant::now();
            let after = highlighter.highlight(&edited);
            let edit_time = started.elapsed();
            assert_eq!(first.len(), after.len());
            println!("{kilobytes:>4} KB {name:<10} first {first_time:>10.2?}   after an edit {edit_time:>10.2?}");
        }
    }
}

/// The ranges of the keywords and numbers of one line of code, relative to its start.
fn code_line(line: &str) -> Vec<StyledRange> {
    const KEYWORDS: [&str; 12] = [
        "fn", "let", "mut", "if", "else", "for", "while", "return", "match", "pub", "struct", "impl",
    ];
    let mut ranges = Vec::new();
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' {
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            let word = &line[start..at];
            if KEYWORDS.contains(&word) {
                ranges.push(StyledRange::new(start..at, SpanStyle::new().color(Color::Rgba(190, 80, 200, 255))));
            } else if word.bytes().all(|byte| byte.is_ascii_digit()) {
                ranges.push(StyledRange::new(start..at, SpanStyle::new().color(Color::Rgba(200, 140, 60, 255))));
            }
        } else {
            at += 1;
        }
    }
    ranges
}

/// Source code of about `kilobytes`, numbered so no two functions are alike.
fn code(kilobytes: usize) -> String {
    let function = "pub fn compute_NN(mut value: u32) -> u32 {\n    let step = 42;\n    if value > 100 {\n        \
        return value - step;\n    }\n    for index in 0..8 {\n        value += index * 3;\n    }\n    value\n}\n\n";
    (0..kilobytes * 1000 / function.len() + 1)
        .map(|index| function.replace("NN", &index.to_string()))
        .collect()
}

/// [`bench_highlighting`] with a highlighter that works out every word of the code.
#[test]
#[ignore = "a timing, not a check"]
fn bench_code_highlighting() {
    for kilobytes in [50, 200, 800] {
        let text = code(kilobytes);
        let mut edited = text.clone();
        edited.insert(text.len() / 2, 'x');
        let whole = TextHighlighter::new(|text| {
            let mut ranges = Vec::new();
            let mut line_start = 0;
            for line in text.split_inclusive('\n') {
                ranges.extend(code_line(line).into_iter().map(|range| {
                    StyledRange::new(range.range.start + line_start..range.range.end + line_start, range.style)
                }));
                line_start += line.len();
            }
            ranges
        });
        let by_line = TextHighlighter::by_line((), |line: &str, _: &()| (code_line(line), ()));
        for (name, highlighter) in [("whole text", whole), ("by line", by_line)] {
            let started = Instant::now();
            let first = highlighter.highlight(&text);
            let first_time = started.elapsed();
            let started = Instant::now();
            let after = highlighter.highlight(&edited);
            let edit_time = started.elapsed();
            assert_eq!(first.len(), after.len());
            println!("{kilobytes:>4} KB {name:<10} first {first_time:>10.2?}   after an edit {edit_time:>10.2?}");
        }
    }
}

/// A field that scrolls inside ten lines of its own, as a bounded text area does.
fn scrolling_field(text: &str, highlighter: Option<TextHighlighter>) -> (RawTextField, TextFieldController) {
    let (mut field, controller) = field(text, highlighter);
    field.max_lines = Some(10);
    (field, controller)
}

/// The average of `runs` frames, each after `before` has acted on the field.
fn average_frame(
    ctx: &BuildContext<'static>,
    field: &RawTextField,
    runs: usize,
    mut before: impl FnMut(usize),
) -> Duration {
    let mut total = Duration::ZERO;
    for run in 0..runs {
        before(run);
        total += frame(ctx, field);
    }
    total / runs as u32
}

/// What the user does in a long note besides typing: scrolling, moving the caret,
/// jumping to the end, selecting everything. Each is a frame the field has to answer,
/// averaged over several, after the first frame has laid the note out.
///
/// ```text
/// cargo nextest run -p aimer_input --run-ignored only -E 'test(bench_interactions)' --no-capture
/// ```
#[test]
#[ignore = "a timing, not a check"]
fn bench_interactions() {
    for kilobytes in [50, 200, 800] {
        for highlighted in [false, true] {
            let text = note(kilobytes);
            let (field, _controller) = scrolling_field(&text, highlighted.then(markdown_like));
            let ctx = dummy_build_context(800.0, 600.0);
            let open = frame(&ctx, &field);
            frame(&ctx, &field);

            let scroll = average_frame(&ctx, &field, 20, |run| {
                field.scroll_y.set(((run + 1) * 300) as f32);
            });
            field.scroll_y.set(0.0);
            frame(&ctx, &field);
            let caret = average_frame(&ctx, &field, 20, |_| {
                let _ = field.on_event(&key(NamedKey::ArrowRight));
            });
            let end = average_frame(&ctx, &field, 1, |_| {
                let _ = field.on_event(&key(NamedKey::End));
            });
            let select_all = average_frame(&ctx, &field, 1, |_| {
                let _ = field.on_event(&ElementEvent::KeyInput {
                    key: NamedKey::Other("a".to_owned()),
                    action: KeyAction::Pressed,
                    modifiers: Modifiers { ctrl: true, ..Modifiers::default() },
                });
            });
            println!(
                "{kilobytes:>4} KB {:<11} open {open:>9.2?}  scroll {scroll:>9.2?}  caret move {caret:>9.2?}  End {end:>9.2?}  Select all {select_all:>9.2?}",
                if highlighted { "highlighted" } else { "plain" },
            );
        }
    }
}

/// Typing into the middle of a long note the way a keyboard does: a text input event
/// on the field, with the caret in the middle, and the frame after it. Unlike
/// `bench_edits`, which replaces the whole text, this is the path a keystroke takes.
#[test]
#[ignore = "a timing, not a check"]
fn bench_typing() {
    for kilobytes in [50, 200, 800] {
        for highlighted in [false, true] {
            let text = note(kilobytes);
            let (field, controller) = field(&text, highlighted.then(markdown_like));
            let ctx = dummy_build_context(800.0, 600.0);
            frame(&ctx, &field);
            let middle = text.chars().count() / 2;
            let _ = &controller;
            field.cursor.set_offset(middle);
            field.cursor.set_selection_anchor(None);
            frame(&ctx, &field);

            let typing = average_frame(&ctx, &field, 10, |_| {
                let _ = field.on_event(&commit("x"));
            });
            println!(
                "{kilobytes:>4} KB {:<11} typing a character, event and frame {typing:>9.2?}",
                if highlighted { "highlighted" } else { "plain" },
            );
        }
    }
}

/// What an idle focused field costs the frames the app asks of it while the caret
/// blinks: `update` and then the paint, with nothing changed.
#[test]
#[ignore = "a timing, not a check"]
fn bench_idle_update() {
    for kilobytes in [50, 200, 800] {
        for highlighted in [false, true] {
            let text = note(kilobytes);
            let (field, _controller) = scrolling_field(&text, highlighted.then(markdown_like));
            let ctx = dummy_build_context(800.0, 600.0);
            frame(&ctx, &field);
            field.update(&ctx);
            let runs = 10;
            let started = Instant::now();
            for _ in 0..runs {
                field.update(&ctx);
                frame(&ctx, &field);
            }
            println!(
                "{kilobytes:>4} KB {:<11} an idle frame, update and paint {:>9.2?}",
                if highlighted { "highlighted" } else { "plain" },
                started.elapsed() / runs,
            );
        }
    }
}

/// What a note of many megabytes costs to open, edit and move around in, and how much memory it
/// holds: the sizes in `BENCH_MB` (comma separated, `5,20,50` by default), in a field that scrolls
/// inside ten lines of its own. `BENCH_KIND` is `plain` (default), `giant` (one hard line) or
/// `markdown` (styled a line at a time).
///
/// ```text
/// BENCH_MB=50 cargo nextest run -p aimer_input --run-ignored only -E 'test(bench_huge_note)' --no-capture
/// ```
#[test]
#[ignore = "a timing, not a check"]
fn bench_huge_note() {
    fn resident_megabytes() -> f64 {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .expect("ps runs");
        String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
    }
    let sizes = std::env::var("BENCH_MB").unwrap_or_else(|_| "5,20,50".to_owned());
    for megabytes in sizes.split(',').filter_map(|size| size.trim().parse::<usize>().ok()) {
        let before = resident_megabytes();
        let started = Instant::now();
        let mut text = note(megabytes * 1000);
        // `BENCH_KIND=giant` makes it one hard line, `markdown` styles it a line at a time.
        let kind = std::env::var("BENCH_KIND").unwrap_or_else(|_| "plain".to_owned());
        if kind == "giant" {
            text = text.replace('\n', " ");
        }
        // `BENCH_FILE=path` with `BENCH_KIND=file` opens that file, for text that is not ASCII.
        if kind == "file" {
            text = std::fs::read_to_string(std::env::var("BENCH_FILE").expect("BENCH_FILE names a file")).expect("the file reads");
        }
        let made = started.elapsed();

        let started = Instant::now();
        let (field, controller) = scrolling_field(&text, (kind == "markdown").then(markdown_by_line));
        let controller_made = started.elapsed();
        let after_controller = resident_megabytes() - before;
        let ctx = dummy_build_context(800.0, 600.0);

        let open = frame(&ctx, &field);
        let unchanged = average_frame(&ctx, &field, 5, |_| {});
        let held = resident_megabytes() - before;
        println!("       memory: text made {:.0} MB (the string), + controller {after_controller:.0} MB, + first frames {held:.0} MB", 0.0);

        field.cursor.set_offset(text.chars().count() / 2);
        field.cursor.set_selection_anchor(None);
        frame(&ctx, &field);
        let typing = average_frame(&ctx, &field, 5, |_| {
            let _ = field.on_event(&commit("x"));
        });
        let end = average_frame(&ctx, &field, 1, |_| {
            let _ = field.on_event(&key(NamedKey::End));
        });
        let after_typing = resident_megabytes() - before;
        println!(
            "{megabytes:>3} MB {kind:<8} text made {made:>8.2?}  controller {controller_made:>8.2?}  open {open:>9.2?}  unchanged {unchanged:>9.2?}  typing {typing:>9.2?}  End {end:>9.2?}  held {held:>6.0} MB, after typing {after_typing:>6.0} MB",
        );
        drop((field, controller));
    }
}

#[test]
#[ignore = "memory probe"]
fn probe_memory_layers() {
    fn rss() -> f64 {
        let output = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().expect("ps runs");
        String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
    }
    let base = rss();
    let text = note(50_000);
    let after_text = rss() - base;
    let snapshot = aimer_text_buffer::TextSnapshot::from_text(&text);
    let after_rope = rss() - base;
    let value = crate::editable_text::TextEditingValue::with_text(&text);
    let after_value = rss() - base;
    let controller = crate::TextEditingController::with_text(text.clone());
    let after_controller = rss() - base;
    println!(
        "text {after_text:.0} MB | + rope {:.0} | + value {:.0} | + controller {:.0}  (len {} MB, chunks {})",
        after_rope - after_text,
        after_value - after_rope,
        after_controller - after_value,
        text.len() / 1_000_000,
        snapshot.chunk_count()
    );
    drop((value, controller, snapshot));
}

/// Memory of loading a text by pasting it, as the user does: an empty field takes a text of
/// `BENCH_MB` megabytes in one commit, with a callback that keeps the text as an app would.
#[test]
#[ignore = "memory probe"]
fn probe_paste_memory() {
    fn rss() -> f64 {
        let output = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().expect("ps runs");
        String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
    }
    let megabytes: usize = std::env::var("BENCH_MB").ok().and_then(|value| value.parse().ok()).unwrap_or(30);
    let text = note(megabytes * 1000);
    let (field, controller) = scrolling_field("", None);
    let kept = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let sink = std::rc::Rc::clone(&kept);
    let mut field = field;
    field.on_changed = (move |text: String| *sink.borrow_mut() = text).into();
    let _ = &controller;
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&ctx, &field);
    let base = rss();
    let _ = field.on_event(&commit(&text));
    let committed = rss() - base;
    frame(&ctx, &field);
    let first_frame = rss() - base;
    frame(&ctx, &field);
    let second_frame = rss() - base;
    println!(
        "{megabytes} MB pasted: after the commit {committed:.0} MB, first frame {first_frame:.0} MB, second {second_frame:.0} MB (the text itself {} MB, the app keeps another)",
        text.len() / 1_000_000
    );
}
