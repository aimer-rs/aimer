use aimer_platform::capture::Frame;

use super::*;

/// A `w` x `h` image whose pixel `(x, y)` is `[x, y, 7, 255]`, so every pixel is
/// different and a wrong copy cannot look right.
fn grid(w: u32, h: u32, scale: f32) -> Image {
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[x as u8, y as u8, 7, 255]);
        }
    }
    Image::new(w, h, scale, rgba).expect("a valid image")
}

fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width() + x) * 4) as usize;
    image.rgba()[at..at + 4].try_into().unwrap()
}

#[test]
fn an_image_must_have_exactly_the_bytes_its_size_says() {
    assert!(Image::new(2, 2, 1.0, vec![0; 16]).is_some());
    assert!(Image::new(2, 2, 1.0, vec![0; 15]).is_none());
    assert!(Image::new(0, 2, 1.0, Vec::new()).is_none(), "nothing to show");
    assert!(Image::new(2, 2, 0.0, vec![0; 16]).is_none(), "a scale is positive");
    assert!(Image::new(2, 2, f32::NAN, vec![0; 16]).is_none());
}

#[test]
fn a_frame_in_blue_green_red_order_becomes_red_green_blue() {
    let frame = Frame { width: 2, height: 1, scale: 2.0, bgra: true, pixels: vec![1, 2, 3, 255, 10, 20, 30, 128] };
    let image = Image::from_frame(&frame);
    assert_eq!(pixel(&image, 0, 0), [3, 2, 1, 255]);
    assert_eq!(pixel(&image, 1, 0), [30, 20, 10, 128], "alpha stays where it was");
    assert_eq!((image.width(), image.height(), image.scale()), (2, 1, 2.0));
}

#[test]
fn a_frame_already_in_red_green_blue_order_is_left_alone() {
    let frame = Frame { width: 1, height: 1, scale: 1.0, bgra: false, pixels: vec![1, 2, 3, 4] };
    assert_eq!(pixel(&Image::from_frame(&frame), 0, 0), [1, 2, 3, 4]);
}

#[test]
fn a_crop_is_exactly_the_pixels_inside_it() {
    let image = grid(10, 8, 1.0);
    let part = image.crop(3, 2, 4, 3).expect("inside");
    assert_eq!((part.width(), part.height()), (4, 3));
    assert_eq!(pixel(&part, 0, 0), [3, 2, 7, 255]);
    assert_eq!(pixel(&part, 3, 2), [6, 4, 7, 255]);
}

#[test]
fn a_crop_that_hangs_over_the_edge_is_trimmed_and_one_wholly_outside_is_nothing() {
    let image = grid(10, 8, 1.0);
    let part = image.crop(8, 6, 10, 10).expect("partly inside");
    assert_eq!((part.width(), part.height()), (2, 2));
    assert!(image.crop(10, 0, 4, 4).is_none());
    assert!(image.crop(0, 0, 0, 4).is_none());
}

#[test]
fn a_crop_in_logical_pixels_is_taken_at_the_images_scale() {
    // A 2x image: the logical rectangle (2, 1) 3 wide, 2 tall is physical (4, 2) 6 by 4.
    let image = grid(20, 16, 2.0);
    let part = image.crop_logical(2.0, 1.0, 3.0, 2.0).expect("inside");
    assert_eq!((part.width(), part.height()), (6, 4));
    assert_eq!(pixel(&part, 0, 0), [4, 2, 7, 255]);
    assert_eq!(part.scale(), 2.0, "it is still a 2x picture");
}

#[test]
fn shrinking_to_a_width_keeps_the_shape_and_says_how_far_it_shrank() {
    let image = grid(100, 50, 2.0);
    let small = image.scaled_to_width(40);
    assert_eq!((small.width(), small.height()), (40, 20));
    assert!((small.scale() - 0.8).abs() < 1e-4, "2.0 x 40/100: {}", small.scale());
}

#[test]
fn an_image_already_narrow_enough_is_not_enlarged() {
    let image = grid(30, 20, 1.0);
    let same = image.scaled_to_width(100);
    assert_eq!((same.width(), same.height(), same.scale()), (30, 20, 1.0));
    assert_eq!(same.rgba(), image.rgba());
}

#[test]
fn shrinking_a_flat_colour_keeps_the_colour() {
    let flat = Image::new(64, 64, 1.0, [10, 200, 30, 255].repeat(64 * 64)).unwrap();
    let small = flat.scaled_to_width(8);
    assert!((0..8).all(|x| (0..8).all(|y| pixel(&small, x, y) == [10, 200, 30, 255])));
}

#[test]
fn png_keeps_every_pixel_exactly() {
    let image = grid(17, 9, 2.0);
    let bytes = image.encode(Format::Png).expect("encoded");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let back = ::image::load_from_memory(&bytes).expect("decodes").to_rgba8();
    assert_eq!((back.width(), back.height()), (17, 9));
    assert_eq!(back.as_raw(), image.rgba(), "lossless");
}

#[test]
fn jpeg_is_smaller_for_a_photo_like_picture_and_close_to_what_went_in() {
    // A gradient with a little noise on it, like a photograph: PNG cannot predict
    // the noise, and JPEG does not mind it. (A clean gradient is PNG's best case.)
    let mut rgba = Vec::new();
    for y in 0..96u32 {
        for x in 0..96u32 {
            let noise = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 9) as i32 - 4;
            let channel = |base: u32| (base as i32 + noise).clamp(0, 255) as u8;
            rgba.extend_from_slice(&[channel(x * 2), channel(y * 2), 128, 255]);
        }
    }
    let image = Image::new(96, 96, 1.0, rgba).unwrap();
    let jpeg = image.encode(Format::Jpeg { quality: 80 }).expect("encoded");
    let png = image.encode(Format::Png).expect("encoded");
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "a JPEG");
    assert!(jpeg.len() < png.len(), "{} vs {}", jpeg.len(), png.len());
    let back = ::image::load_from_memory(&jpeg).expect("decodes").to_rgb8();
    let (got, want) = (back.get_pixel(48, 48).0, [96u8, 96, 128]);
    assert!(got.iter().zip(want).all(|(p, q)| p.abs_diff(q) < 14), "{got:?} vs {want:?}");
}

#[test]
fn the_format_knows_its_mime_type() {
    assert_eq!(Format::Png.mime(), "image/png");
    assert_eq!(Format::Jpeg { quality: 50 }.mime(), "image/jpeg");
}

#[test]
fn a_jpeg_quality_is_kept_in_range() {
    let image = grid(8, 8, 1.0);
    assert!(image.encode(Format::Jpeg { quality: 0 }).is_ok(), "0 is raised to the lowest, not refused");
    assert!(image.encode(Format::Jpeg { quality: 200 }).is_ok(), "200 is lowered to 100");
}
