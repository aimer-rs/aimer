//! Finding bundled assets on the file system.

use std::path::{Path, PathBuf};

/// Where a bundled asset may live, in lookup order: the working directory
/// (development runs), the macOS bundle's `Resources`, then next to the
/// executable (iOS). The same lookup `aimer_assets` and `aimer_audio` use.
pub(crate) fn bundled_candidates(key: &str, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(key)];
    if let Some(dir) = exe.and_then(Path::parent) {
        if let Some(contents) = dir.parent() {
            candidates.push(contents.join("Resources").join(key));
        }
        candidates.push(dir.join(key));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::bundled_candidates;
    use std::path::{Path, PathBuf};

    #[test]
    fn bundled_assets_are_searched_in_the_working_directory_then_the_bundle() {
        let exe = Path::new("/Apps/Demo.app/Contents/MacOS/demo");
        assert_eq!(
            bundled_candidates("assets/a.mp4", Some(exe)),
            vec![
                PathBuf::from("assets/a.mp4"),
                PathBuf::from("/Apps/Demo.app/Contents/Resources/assets/a.mp4"),
                PathBuf::from("/Apps/Demo.app/Contents/MacOS/assets/a.mp4"),
            ]
        );
        assert_eq!(bundled_candidates("a.vtt", None), vec![PathBuf::from("a.vtt")]);
    }
}
