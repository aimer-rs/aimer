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

#[test]
fn a_pair_adjustment_is_remembered_with_the_table_and_a_clone_starts_empty() {
    use super::{PairTable, ValueAdjustment};

    let state = LayoutTableState::parse(&shared_lookup_gsub(), GSUB_TAG).expect("the fixture is a valid GSUB");
    let kern = Some((ValueAdjustment { x_advance: -40, ..ValueAdjustment::default() }, ValueAdjustment::default()));

    state.pair_adjustments.lock().insert((1, 7), kern);
    assert_eq!(state.pair_adjustments.lock().len(), 1, "kept for the next line shaped with this table");
    let found = state.pair_adjustments.lock().get(&(1, 7));
    assert!(found.is_some_and(|answer| answer.is_some_and(|(first, _)| first.x_advance == -40)));
    // A pair with no kerning is an answer too, and is not the same as a pair never asked about.
    state.pair_adjustments.lock().insert((1, 8), None);
    assert!(matches!(state.pair_adjustments.lock().get(&(1, 8)), Some(None)));
    assert!(state.pair_adjustments.lock().get(&(1, 9)).is_none());
    assert!(state.clone().pair_adjustments.lock().get(&(1, 7)).is_none(), "a clone has its own, empty");
    assert!(PairTable::default().get(&(1, 7)).is_none(), "and nothing is allocated until a pair is kept");
}

#[test]
fn two_pairs_that_share_a_slot_never_answer_for_each_other() {
    use super::{PairTable, ValueAdjustment};

    let wide = |x_advance| Some((ValueAdjustment { x_advance, ..ValueAdjustment::default() }, ValueAdjustment::default()));
    let mut table = PairTable::default();
    let mut seen = std::collections::HashMap::new();
    let (first, second) = (0..100_000u32)
        .map(|pair| (7usize, pair))
        .find_map(|key| seen.insert(PairTable::slot_of(&key), key).map(|earlier| (earlier, key)))
        .expect("a hundred thousand pairs fill four thousand slots");

    table.insert(first, wide(1));
    table.insert(second, wide(2));

    assert!(table.get(&second).is_some_and(|answer| answer.is_some_and(|(kerned, _)| kerned.x_advance == 2)));
    assert!(table.get(&first).is_none(), "its slot was taken, so it is looked up again");
}
