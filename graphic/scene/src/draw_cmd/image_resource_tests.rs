use super::ImageResource;

#[test]
fn the_same_pixels_always_get_the_same_texture_id_and_revision() {
    let rgba = vec![7_u8; 2 * 3 * 4];
    assert_eq!(
        ImageResource::texture_id_for(&rgba, 2, 3),
        ImageResource::texture_id_for(&rgba.clone(), 2, 3)
    );
    assert_eq!(
        ImageResource::content_revision_for(&rgba, 2, 3),
        ImageResource::content_revision_for(&rgba.clone(), 2, 3)
    );
}

#[test]
fn the_size_is_part_of_the_identity() {
    let rgba = vec![7_u8; 2 * 3 * 4];
    assert_ne!(ImageResource::texture_id_for(&rgba, 2, 3), ImageResource::texture_id_for(&rgba, 3, 2));
}

#[test]
fn a_change_outside_the_sampled_bytes_still_changes_the_revision() {
    // Large enough that the middle byte lies outside both sampled ends.
    let width = 64;
    let mut rgba = vec![0_u8; width * width * 4];
    let id = ImageResource::texture_id_for(&rgba, width as u32, width as u32);
    let revision = ImageResource::content_revision_for(&rgba, width as u32, width as u32);
    let middle = rgba.len() / 2;
    rgba[middle] = 255;

    assert_eq!(ImageResource::texture_id_for(&rgba, width as u32, width as u32), id);
    assert_ne!(ImageResource::content_revision_for(&rgba, width as u32, width as u32), revision);
}
