use super::*;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[test]
fn timestamps_with_and_without_hours_and_both_separators() {
    assert_eq!(parse_timestamp("00:01.500"), Some(ms(1_500)));
    assert_eq!(parse_timestamp("01:02:03.004"), Some(ms(3_723_004)));
    assert_eq!(parse_timestamp("01:02:03,004"), Some(ms(3_723_004)));
    assert_eq!(parse_timestamp("10:00:00.1"), Some(ms(36_000_100)), "a short fraction is tenths");
    assert_eq!(parse_timestamp("100:00:00.000"), Some(ms(360_000_000)), "hours may exceed two digits");
}

#[test]
fn malformed_timestamps_are_rejected() {
    for bad in ["", "1.000", "00:60.000", "00:00:61.000", "aa:00.000", "00:01.1234", "00:01.", "0:1:2:3.000", "-1:00.000"] {
        assert_eq!(parse_timestamp(bad), None, "{bad:?}");
    }
}

#[test]
fn a_webvtt_file_with_header_notes_styles_and_ids_parses_its_cues() {
    let track = CaptionTrack::parse(
        "\u{feff}WEBVTT - with a title\r\nKind: captions\r\n\r\n\
         NOTE written by hand\r\nacross lines\r\n\r\n\
         STYLE\r\n::cue { color: yellow }\r\n\r\n\
         intro\r\n00:00.500 --> 00:02.000 align:start line:0\r\n<v Ada>Hello</v> &amp; <i>welcome</i>\r\nsecond line\r\n\r\n\
         00:02.500 --> 00:04.000\r\nNo id here\r\n",
    )
    .unwrap();
    assert_eq!(
        track.cues(),
        &[
            Cue::new(ms(500), ms(2_000), "Hello & welcome\nsecond line"),
            Cue::new(ms(2_500), ms(4_000), "No id here"),
        ]
    );
}

#[test]
fn an_srt_file_parses_and_drops_its_positioning_tags() {
    let track = CaptionTrack::parse(
        "1\n00:00:01,000 --> 00:00:02,500\n{\\an8}<i>Top</i> line\n\n2\n00:00:03,000 --> 00:00:04,000\nSecond\n",
    )
    .unwrap();
    assert_eq!(
        track.cues(),
        &[Cue::new(ms(1_000), ms(2_500), "Top line"), Cue::new(ms(3_000), ms(4_000), "Second")]
    );
}

#[test]
fn a_malformed_cue_is_skipped_not_the_file() {
    let track = CaptionTrack::parse(
        "WEBVTT\n\n00:01.000 -> 00:02.000\nbroken arrow\n\n00:03.000 --> 00:04.000\nkept\n\n00:05.000 --> 00:04.000\nends before it starts\n",
    )
    .unwrap();
    assert_eq!(track.cues(), &[Cue::new(ms(3_000), ms(4_000), "kept")]);
}

#[test]
fn text_that_is_not_captions_is_invalid_but_an_empty_vtt_is_fine() {
    assert_eq!(CaptionTrack::parse("<html>nope</html>"), Err(CaptionError::Invalid));
    assert_eq!(CaptionTrack::parse("WEBVTT\n").unwrap().cues(), &[]);
    assert_eq!(CaptionTrack::parse("").unwrap().cues(), &[]);
}

#[test]
fn entities_decode_and_unknown_ones_stay_as_written() {
    assert_eq!(clean_line("a &lt;b&gt; &amp; c&nbsp;d"), "a <b> & c\u{a0}d");
    assert_eq!(clean_line("Q&A &copy; & alone"), "Q&A &copy; & alone");
    assert_eq!(clean_line("<c.yellow.bg_blue>styled</c> <00:00:01.000>karaoke"), "styled karaoke");
}

#[test]
fn the_text_showing_follows_the_playhead_and_joins_overlapping_cues() {
    let track = CaptionTrack::from_cues(vec![
        Cue::new(ms(3_000), ms(6_000), "second"),
        Cue::new(ms(1_000), ms(4_000), "first"),
        Cue::new(ms(10_000), ms(11_000), "later"),
    ]);
    assert_eq!(track.text_at(ms(500)), None);
    assert_eq!(track.text_at(ms(1_000)).as_deref(), Some("first"), "the start is inclusive");
    assert_eq!(track.text_at(ms(3_500)).as_deref(), Some("first\nsecond"));
    assert_eq!(track.text_at(ms(4_000)).as_deref(), Some("second"), "the end is exclusive");
    assert_eq!(track.text_at(ms(8_000)), None);
    assert_eq!(track.text_at(ms(10_500)).as_deref(), Some("later"));
}

#[test]
fn a_long_cue_is_found_even_after_many_short_ones_start() {
    let mut cues = vec![Cue::new(ms(0), ms(100_000), "whole scene")];
    cues.extend((1..1_000).map(|i| Cue::new(ms(i * 50), ms(i * 50 + 10), "blip")));
    let track = CaptionTrack::from_cues(cues);
    assert_eq!(track.text_at(ms(49_995)).as_deref(), Some("whole scene"));
}

#[test]
fn the_showing_identity_changes_exactly_with_the_text() {
    let track = CaptionTrack::from_cues(vec![
        Cue::new(ms(1_000), ms(2_000), "a"),
        Cue::new(ms(2_000), ms(3_000), "b"),
    ]);
    assert_eq!(track.showing_at(ms(1_100)), track.showing_at(ms(1_900)));
    assert_ne!(track.showing_at(ms(1_900)), track.showing_at(ms(2_100)));
    assert_eq!(track.showing_at(ms(500)).1, 0);
}

#[test]
fn chapters_find_the_one_playing_and_span_the_whole_video() {
    let chapters = Chapters::new(vec![
        Chapter::new(ms(60_000), "Middle"),
        Chapter::new(ms(0), "Intro"),
        Chapter::new(ms(150_000), "End"),
        Chapter::new(ms(150_000), "Duplicate start"),
    ]);
    assert_eq!(chapters.chapters().len(), 3);
    assert_eq!(chapters.at(ms(59_999)).map(Chapter::title), Some("Intro"));
    assert_eq!(chapters.at(ms(60_000)).map(Chapter::title), Some("Middle"));
    assert_eq!(chapters.at(ms(999_000)).map(Chapter::title), Some("End"));

    let spans = chapters.spans(ms(200_000));
    assert_eq!(spans, vec![(0.0, 0.3), (0.3, 0.75), (0.75, 1.0)]);
    assert!(chapters.spans(Duration::ZERO).is_empty());
}

#[test]
fn a_first_chapter_after_zero_still_starts_the_track() {
    let chapters = Chapters::new(vec![Chapter::new(ms(5_000), "Late start")]);
    assert_eq!(chapters.spans(ms(10_000)), vec![(0.0, 1.0)]);
    assert_eq!(chapters.at(ms(1_000)), None, "nothing before the first chapter");
}

#[test]
fn a_webvtt_chapters_file_becomes_chapters() {
    let chapters = Chapters::parse(
        "WEBVTT\n\nch1\n00:00.000 --> 00:30.000\nOpening\n\nch2\n00:30.000 --> 01:00.000\nThe <b>main</b>\npart\n",
    )
    .unwrap();
    let titles: Vec<_> = chapters.chapters().iter().map(Chapter::title).collect();
    assert_eq!(titles, ["Opening", "The main part"]);
    assert_eq!(chapters.chapters()[1].start(), ms(30_000));
}

#[test]
fn sources_read_text_files_and_report_missing_ones() {
    let path = std::env::temp_dir().join(format!("aimer_video_captions_{}.vtt", std::process::id()));
    std::fs::write(&path, "WEBVTT\n\n00:01.000 --> 00:02.000\nfrom disk\n").unwrap();
    let track = CaptionTrack::load(&CaptionSource::File(path.clone())).unwrap();
    assert_eq!(track.text_at(ms(1_500)).as_deref(), Some("from disk"));
    let _ = std::fs::remove_file(path);

    assert!(matches!(
        CaptionTrack::load(&CaptionSource::File("/definitely/missing.vtt".into())),
        Err(CaptionError::NotFound(_))
    ));
    assert!(matches!(
        CaptionTrack::load(&CaptionSource::Bundled("assets/missing.vtt".into())),
        Err(CaptionError::NotFound(_))
    ));
    let inline = CaptionTrack::load(&CaptionSource::Text("WEBVTT\n\n00:00.000 --> 00:01.000\nhi\n".into())).unwrap();
    assert_eq!(inline.cues().len(), 1);
}
