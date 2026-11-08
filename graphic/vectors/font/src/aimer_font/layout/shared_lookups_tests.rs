//! Regression tests for lookup sharing between feature plans.

use super::{GSUB_TAG, LayoutTableState};

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// A GSUB table with one single-substitution lookup that three features
/// (`liga`, `isol`, `init`) all reference, and no script list so every
/// feature is selected for every script.
fn shared_lookup_gsub() -> Vec<u8> {
    let mut features = Vec::new();
    push_u16(&mut features, 3);
    for (index, tag) in [*b"liga", *b"isol", *b"init"].into_iter().enumerate() {
        features.extend_from_slice(&tag);
        // Feature tables start after the count and the three 6-byte records.
        push_u16(&mut features, (2 + 3 * 6 + index * 6) as u16);
    }
    for _ in 0..3 {
        push_u16(&mut features, 0); // feature params
        push_u16(&mut features, 1); // lookup count
        push_u16(&mut features, 0); // lookup index
    }

    let mut lookups = Vec::new();
    push_u16(&mut lookups, 1);
    push_u16(&mut lookups, 4); // lookup 0 offset
    push_u16(&mut lookups, 1); // lookup type: single substitution
    push_u16(&mut lookups, 0); // flags
    push_u16(&mut lookups, 1); // subtable count
    push_u16(&mut lookups, 8); // subtable offset, relative to the lookup
    push_u16(&mut lookups, 1); // subtable format
    push_u16(&mut lookups, 6); // coverage offset
    push_u16(&mut lookups, 1); // delta glyph id
    push_u16(&mut lookups, 1); // coverage format
    push_u16(&mut lookups, 1); // glyph count
    push_u16(&mut lookups, 5); // glyph

    let feature_offset = 10_u16;
    let lookup_offset = feature_offset + features.len() as u16;
    let mut table = Vec::new();
    push_u16(&mut table, 1); // major version
    push_u16(&mut table, 0); // minor version
    push_u16(&mut table, 0); // no script list
    push_u16(&mut table, feature_offset);
    push_u16(&mut table, lookup_offset);
    table.extend_from_slice(&features);
    table.extend_from_slice(&lookups);
    table
}

#[test]
fn feature_plans_share_lookup_storage_instead_of_cloning_it() {
    let table = shared_lookup_gsub();
    let state = LayoutTableState::parse(&table, GSUB_TAG).expect("the fixture is a valid GSUB");

    let base = &state.lookups[0];
    assert_eq!(
        base.compiled_subtables.len(),
        1,
        "the shared lookup must be active and compiled for the test to mean anything"
    );

    let referencing: Vec<_> = state
        .feature_lookups
        .iter()
        .flat_map(|feature| feature.lookups.iter())
        .collect();
    // With no script list every script selects these features, so many plans
    // (Latin, Arabic, the Indic and Southeast Asian specs) reference lookup 0.
    assert!(
        referencing.len() >= 3,
        "liga, isol and init each reference lookup 0, got {} plans",
        referencing.len()
    );

    for lookup in referencing {
        assert_eq!(
            lookup.compiled_subtables.as_ptr(),
            base.compiled_subtables.as_ptr(),
            "compiled subtables must be shared, not deep-cloned per feature"
        );
        assert_eq!(
            lookup.subtable_offsets.as_ptr(),
            base.subtable_offsets.as_ptr(),
            "subtable offsets must be shared, not deep-cloned per feature"
        );
    }
}
