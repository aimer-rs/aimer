use super::*;

#[test]
fn unsupported_media_has_a_typed_state_and_disposes_cleanly() {
    let capabilities = CapabilitySet::new();
    let mut media = MediaElement::new(
        MediaId::new(7),
        MediaKind::Video,
        MediaSource::url("https://example.com/movie.mp4"),
        capabilities,
    )
    .unwrap();
    media.resize(MediaSize::new(640, 360).unwrap()).unwrap();
    media.focus().unwrap();

    let error = media.load().unwrap_err();
    assert!(matches!(
        error,
        MediaError::Unsupported {
            capability: MediaCapability::VideoPlayback,
            ..
        }
    ));
    assert!(matches!(media.state(), MediaState::Unsupported { .. }));
    assert!(media.is_focused());

    media.dispose().unwrap();
    assert_eq!(media.state(), MediaState::Disposed);
    assert!(matches!(media.play().unwrap_err(), MediaError::Disposed));
}

#[test]
fn supported_audio_exposes_load_play_pause_focus_and_dispose_lifecycle() {
    let capabilities = CapabilitySet::new().support(MediaCapability::AudioPlayback);
    let mut media = MediaElement::new(
        MediaId::new(8),
        MediaKind::Audio,
        MediaSource::url("https://example.com/audio.ogg"),
        capabilities,
    )
    .unwrap();

    media.load().unwrap();
    assert_eq!(media.state(), MediaState::Ready);
    media.focus().unwrap();
    media.play().unwrap();
    assert_eq!(media.state(), MediaState::Playing);
    media.pause().unwrap();
    assert_eq!(media.state(), MediaState::Paused);
    media.stop().unwrap();
    assert_eq!(media.state(), MediaState::Stopped);
    media.dispose().unwrap();
    assert_eq!(media.state(), MediaState::Disposed);
}

#[test]
fn capture_contract_preserves_cancel_denied_unavailable_and_limits() {
    let request = CaptureRequest::file_picker(["image/png"], 4);
    let file = MediaFile::new("photo.png", "image/png", 8).unwrap();
    assert!(matches!(
        validate_capture(&request, &file),
        CaptureOutcome::Rejected(CaptureRejection::SizeLimit {
            max_bytes: 4,
            actual_bytes: 8
        })
    ));

    let wrong_type = MediaFile::new("movie.mp4", "video/mp4", 2).unwrap();
    assert!(matches!(
        validate_capture(&request, &wrong_type),
        CaptureOutcome::Rejected(CaptureRejection::TypeNotAllowed { .. })
    ));

    let valid = MediaFile::new("photo.png", "image/png", 2).unwrap();
    assert_eq!(
        validate_capture(&request, &valid),
        CaptureOutcome::Selected(valid.clone())
    );

    let adapter = UnsupportedCaptureAdapter::new("camera is not available on this target");
    assert!(matches!(
        adapter.pick(&request),
        CaptureOutcome::Unsupported {
            capability: MediaCapability::FilePicker,
            ..
        }
    ));

    let camera = CaptureRequest::camera(["video/mp4"], 8);
    assert!(matches!(
        adapter.capture(&camera),
        CaptureOutcome::Unsupported {
            capability: MediaCapability::Camera,
            ..
        }
    ));
}
