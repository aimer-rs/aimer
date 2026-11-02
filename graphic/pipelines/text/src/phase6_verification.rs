//! The part of the phase 6 verification that needs the GPU-side glyph clipping,
//! which lives in `text_pipeline`. The font-only checks moved to
//! `aimer_cupid_font` with the code they exercise.

use crate::utilities::Mat3;

#[test]
fn phase6_transformed_glyph_box_keeps_clip_and_pixel_alignment_consistent() {
    let transform = Mat3::translate(20.25, 30.75).mul(&Mat3::scale(2.0, 3.0));
    let local_position = [-1.5, -14.0];
    let local_size = [9.0, 18.0];
    let world_position = transform.transform_point(local_position[0], local_position[1]);
    let world_size = [local_size[0] * 2.0, local_size[1] * 3.0];
    let snapped_position = [world_position.0.round(), world_position.1.round()];

    assert_eq!(snapped_position, [17.0, -11.0]);
    assert!(super::glyph_intersects_clip(
        snapped_position,
        world_size,
        [0.0, -20.0, 40.0, 30.0],
    ));
    assert!(!super::glyph_intersects_clip(
        snapped_position,
        world_size,
        [0.0, 45.0, 40.0, 30.0],
    ));
}
