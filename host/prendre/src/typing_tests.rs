use std::time::Duration;

use super::*;

fn chunks(text: &str) -> Vec<String> {
    plan(text, None).steps.into_iter().map(|(chunk, _)| chunk).collect()
}

fn total(plan: &Plan) -> Duration {
    plan.steps.iter().map(|(_, after)| *after).sum()
}

#[test]
fn text_is_typed_one_character_at_a_time_in_order() {
    assert_eq!(chunks("hello"), ["h", "e", "l", "l", "o"]);
    assert_eq!(chunks("héllo 世界"), ["h", "é", "l", "l", "o", " ", "世", "界"]);
    assert_eq!(chunks("a\nb"), ["a", "\n", "b"]);
    assert!(plan("", None).steps.is_empty(), "nothing to type");
}

#[test]
fn a_character_that_needs_several_code_points_is_one_keystroke() {
    // `e` and a combining accent, a woman-technologist emoji joined with ZWJ, a
    // thumbs up with a skin tone, and a heart with a variation selector.
    assert_eq!(chunks("e\u{301}x"), ["e\u{301}", "x"]);
    assert_eq!(chunks("\u{1F469}\u{200D}\u{1F4BB}!"), ["\u{1F469}\u{200D}\u{1F4BB}", "!"]);
    assert_eq!(chunks("\u{1F44D}\u{1F3FD}"), ["\u{1F44D}\u{1F3FD}"]);
    assert_eq!(chunks("\u{2764}\u{FE0F}"), ["\u{2764}\u{FE0F}"]);
    let all: String = chunks("e\u{301}\u{1F469}\u{200D}\u{1F4BB}x").concat();
    assert_eq!(all, "e\u{301}\u{1F469}\u{200D}\u{1F4BB}x", "nothing is lost by splitting");
}

#[test]
fn the_rhythm_is_uneven_like_a_person_and_the_same_every_time() {
    let first = plan("the quick brown fox", None);
    assert_eq!(first, plan("the quick brown fox", None), "no randomness: a test and a demo repeat");
    // The last key waits for nothing, so it is left out of the rhythm.
    let letters: Vec<Duration> = first.steps[..first.steps.len() - 1].iter().filter(|(c, _)| c != " ").map(|(_, d)| *d).collect();
    assert!(letters.iter().any(|d| *d != letters[0]), "not a metronome: {letters:?}");
    for delay in &letters {
        assert!(*delay >= Duration::from_millis(30) && *delay <= Duration::from_millis(120), "{delay:?}");
    }
}

#[test]
fn a_person_pauses_after_a_word_and_longer_after_a_sentence() {
    let p = plan("ab cd. ef", None);
    let after = |chunk: &str| p.steps.iter().find(|(c, _)| c == chunk).map(|(_, d)| *d).expect("typed");
    let slowest_letter = p.steps.iter().filter(|(c, _)| c.chars().all(char::is_alphabetic)).map(|(_, d)| *d).max().unwrap();
    assert!(after(" ") > slowest_letter / 2, "a word ends");
    assert!(after(".") > slowest_letter, "a sentence ends: {:?} vs {slowest_letter:?}", after("."));
}

#[test]
fn the_last_character_waits_for_nothing() {
    let p = plan("abc", None);
    assert_eq!(p.steps.last().map(|(_, d)| *d), Some(Duration::ZERO));
}

#[test]
fn the_time_asked_for_is_the_time_taken() {
    let p = plan("twenty characters...", Some(1000));
    let took = total(&p);
    assert!(took >= Duration::from_millis(995) && took <= Duration::from_millis(1005), "{took:?}");
    // Rhythm survives scaling.
    let delays: Vec<_> = p.steps.iter().map(|(_, d)| *d).collect();
    assert!(delays[..delays.len() - 1].iter().any(|d| *d != delays[0]));
}

#[test]
fn without_a_time_it_follows_the_length_and_never_outlasts_what_a_client_waits() {
    let short = total(&plan("hello", None));
    let longer = total(&plan("hello hello hello", None));
    assert!(longer > short, "more text takes longer");
    let essay = "word ".repeat(400);
    let took = total(&plan(&essay, None));
    assert!(took <= Duration::from_millis(crate::command::MAX_GLIDE_MILLIS as u64 + 5), "{took:?}");
    assert_eq!(plan(&essay, None).steps.len(), essay.chars().count(), "still every character");
}

#[test]
fn a_time_outside_the_limits_is_brought_inside_them() {
    let huge = total(&plan("abc def ghi", Some(u32::MAX)));
    assert!(huge <= Duration::from_millis(crate::command::MAX_GLIDE_MILLIS as u64 + 5));
    let tiny = total(&plan("abc def ghi", Some(0)));
    assert!(tiny <= Duration::from_millis(11), "0 is the fastest there is, not a refusal: {tiny:?}");
}
