//! What kind of media the picker offers.

use aimer_file_picker::FileFilter;

/// The kinds of media an [`ImagePicker`](crate::ImagePicker) offers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MediaKind {
    /// Still images only. The default.
    #[default]
    Images,
    /// Videos only.
    Videos,
    /// Both, in one library view.
    ImagesAndVideos,
}

impl MediaKind {
    /// Whether videos are offered.
    #[inline]
    pub fn includes_videos(self) -> bool {
        matches!(self, Self::Videos | Self::ImagesAndVideos)
    }

    /// Whether still images are offered.
    #[inline]
    pub fn includes_images(self) -> bool {
        matches!(self, Self::Images | Self::ImagesAndVideos)
    }

    /// The file filters that offer exactly this media. iOS has its own picker
    /// and does not use them.
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    pub(crate) fn filters(self) -> Vec<FileFilter> {
        let mut filters = Vec::with_capacity(2);
        if self.includes_images() {
            filters.push(FileFilter::images());
        }
        if self.includes_videos() {
            filters.push(FileFilter::videos());
        }
        filters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_the_default() {
        assert_eq!(MediaKind::default(), MediaKind::Images);
    }

    #[test]
    fn kinds_report_what_they_include() {
        assert!(MediaKind::Images.includes_images() && !MediaKind::Images.includes_videos());
        assert!(!MediaKind::Videos.includes_images() && MediaKind::Videos.includes_videos());
        assert!(MediaKind::ImagesAndVideos.includes_images() && MediaKind::ImagesAndVideos.includes_videos());
    }

    #[test]
    fn each_kind_has_the_matching_filters() {
        let names = |kind: MediaKind| kind.filters().iter().map(|f| f.name().to_owned()).collect::<Vec<_>>();
        assert_eq!(names(MediaKind::Images), ["Images"]);
        assert_eq!(names(MediaKind::Videos), ["Videos"]);
        assert_eq!(names(MediaKind::ImagesAndVideos), ["Images", "Videos"]);
    }
}
