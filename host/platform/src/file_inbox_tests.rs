use std::ffi::OsString;

use super::*;

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

#[test]
fn files_come_out_in_the_order_they_went_in_and_only_once() {
    let inbox = Inbox::default();
    inbox.post(paths(&["/a.rs"]));
    inbox.post(paths(&["/b.rs", "/c.rs"]));
    assert_eq!(inbox.drain(), paths(&["/a.rs", "/b.rs", "/c.rs"]));
    assert!(inbox.drain().is_empty());
}

#[test]
fn posting_says_whether_the_loop_needs_waking() {
    let inbox = Inbox::default();
    assert!(inbox.post(paths(&["/a.rs"])), "the first post after a drain wakes the loop");
    assert!(!inbox.post(paths(&["/b.rs"])), "one already pending: the wake-up is on its way");
    inbox.drain();
    assert!(inbox.post(paths(&["/c.rs"])));
}

#[test]
fn an_empty_path_or_list_posts_nothing_and_wakes_nothing() {
    let inbox = Inbox::default();
    assert!(!inbox.post(Vec::new()));
    assert!(!inbox.post([PathBuf::new()]));
    assert!(inbox.drain().is_empty());
}

#[test]
fn a_flood_keeps_the_newest_files_and_never_grows_without_bound() {
    let inbox = Inbox::default();
    inbox.post((0..Inbox::CAPACITY + 10).map(|n| PathBuf::from(format!("/f{n}"))));
    let files = inbox.drain();
    assert_eq!(files.len(), Inbox::CAPACITY);
    assert_eq!(files.last(), Some(&PathBuf::from(format!("/f{}", Inbox::CAPACITY + 9))));
}

#[test]
fn posting_from_another_thread_reaches_the_drain() {
    let inbox = std::sync::Arc::new(Inbox::default());
    let poster = inbox.clone();
    std::thread::spawn(move || poster.post(paths(&["/from-appkit.rs"]))).join().unwrap();
    assert_eq!(inbox.drain(), paths(&["/from-appkit.rs"]));
}

#[test]
fn a_file_url_names_the_path_it_encodes() {
    assert_eq!(file_url_to_path("file:///Users/me/main.rs"), Some(PathBuf::from("/Users/me/main.rs")));
    assert_eq!(file_url_to_path("file://localhost/tmp/a.rs"), Some(PathBuf::from("/tmp/a.rs")));
    assert_eq!(file_url_to_path("FILE:///tmp/a.rs"), Some(PathBuf::from("/tmp/a.rs")), "the scheme is case-blind");
    assert_eq!(file_url_to_path("file:///"), Some(PathBuf::from("/")), "the root is a path too");
}

#[test]
fn escapes_are_decoded_and_a_query_or_fragment_is_not_part_of_the_path() {
    assert_eq!(file_url_to_path("file:///tmp/my%20file%23.rs"), Some(PathBuf::from("/tmp/my file#.rs")));
    assert_eq!(file_url_to_path("file:///tmp/%E6%97%A5%E6%9C%AC.rs"), Some(PathBuf::from("/tmp/日本.rs")));
    assert_eq!(file_url_to_path("file:///tmp/a.rs#L10"), Some(PathBuf::from("/tmp/a.rs")));
    assert_eq!(file_url_to_path("file:///tmp/a.rs?x=1"), Some(PathBuf::from("/tmp/a.rs")));
}

#[test]
fn a_windows_drive_loses_the_slash_a_url_puts_before_it() {
    assert_eq!(file_url_to_path("file:///C:/work/main.rs"), Some(PathBuf::from("C:/work/main.rs")));
}

#[test]
fn a_url_that_names_no_local_path_names_nothing() {
    for url in [
        "https://example.com/a.rs",
        "file://server/share/a.rs",
        "file:relative.rs",
        "file://",
        "file:///tmp/bad%zz",
        "file:///tmp/cut%2",
        "file:///tmp/nul%00.rs",
        "",
    ] {
        assert_eq!(file_url_to_path(url), None, "{url:?}");
    }
}

#[test]
fn launch_arguments_that_name_something_are_files_and_the_rest_are_not() {
    let args = ["--verbose", "-x", "/work/a.rs", "serve", "", "/work/b.rs", "8080"].map(OsString::from);
    let exists = |path: &Path| path.starts_with("/work");
    assert_eq!(launch_files(args, exists), paths(&["/work/a.rs", "/work/b.rs"]));
}

#[test]
fn a_program_started_without_arguments_opens_nothing() {
    assert!(launch_files(Vec::<OsString>::new(), |_| true).is_empty());
}
