//! The text form display features cross the Kotlin/Rust boundary in.
//!
//! One feature per `;`-separated entry, seven comma-separated numbers each:
//! `left,top,right,bottom,occludes,half_opened,separating`, edges in logical
//! pixels and the last three `0` or `1`. Text keeps the JNI signature to one
//! `String`, and lets this stay a plain function that is tested on any host.

use crate::{Bounds, DisplayFeature, FoldState};

/// Decodes the wire form.
///
/// An entry that is malformed, short, or carries a non-finite edge is skipped
/// on its own: a platform glitch loses one feature, not the layout of the rest.
pub fn decode_display_features(encoded: &str) -> Vec<DisplayFeature> {
    encoded.split(';').filter_map(decode_one).collect()
}

fn decode_one(entry: &str) -> Option<DisplayFeature> {
    let mut fields = entry.split(',').map(str::trim);
    let mut edge = || {
        let value = fields.next()?.parse::<f32>().ok()?;
        value.is_finite().then_some(value)
    };
    let (left, top, right, bottom) = (edge()?, edge()?, edge()?, edge()?);
    let mut flag = || match fields.next()? {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    };
    let (occludes, half_opened, separating) = (flag()?, flag()?, flag()?);
    // Exactly seven fields: an entry from a newer sender is not guessed at.
    if fields.next().is_some() {
        return None;
    }
    Some(DisplayFeature {
        bounds: Bounds::new(left, top, right, bottom),
        occludes,
        state: if half_opened {
            FoldState::HalfOpened
        } else {
            FoldState::Flat
        },
        separating,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_one_hinge() {
        let features = decode_display_features("1350,0,1434,1800,1,1,1");
        assert_eq!(
            features,
            [DisplayFeature {
                bounds: Bounds::new(1350.0, 0.0, 1434.0, 1800.0),
                occludes: true,
                state: FoldState::HalfOpened,
                separating: true,
            }]
        );
    }

    #[test]
    fn decodes_several_and_a_flat_fold() {
        let features = decode_display_features("0,1100.5,2200,1100.5,0,0,0;1350,0,1434,1800,1,1,1");
        assert_eq!(features.len(), 2);
        assert_eq!(features[0].state, FoldState::Flat);
        assert!(!features[0].occludes && !features[0].separating);
        assert_eq!(features[0].bounds.top, 1100.5);
        assert!(features[1].occludes);
    }

    #[test]
    fn no_features_is_an_empty_list() {
        assert!(decode_display_features("").is_empty());
        assert!(decode_display_features(";").is_empty());
    }

    #[test]
    fn a_bad_entry_loses_only_itself() {
        let features = decode_display_features("1,2,3;oops;10,0,20,100,1,1,1;1,2,3,4,1,1,1,9;NaN,0,1,1,0,0,0");
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].bounds.left, 10.0);
    }

    #[test]
    fn spaces_around_numbers_are_tolerated() {
        assert_eq!(decode_display_features(" 1 , 2 , 3 , 4 , 0 , 1 , 0 ").len(), 1);
    }
}
