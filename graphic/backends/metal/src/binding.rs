//! Mapping from the renderer's `(group, binding)` bind-group model onto
//! Metal's per-stage argument tables.
//!
//! Metal has no bind groups. Instead each shader stage has three independent
//! argument tables — buffers, textures, and samplers — addressed by a single
//! integer each. The trait hands the backend `(group, binding)` pairs, so a
//! Metal backend must flatten them, and the flattening rule has to be knowable
//! by three parties that never talk to each other:
//!
//! 1. the hand-written MSL, where every bound resource carries an explicit
//!    `[[buffer(N)]]` / `[[texture(N)]]` / `[[sampler(N)]]`;
//! 2. [`super::MetalBackend`]'s `set_bind_group`, which issues the
//!    `setVertexBuffer:offset:atIndex:` family; and
//! 3. the vertex streams, because `setVertexBuffer:atIndex:` writes into the
//!    *same* table as buffer arguments, so a vertex stream and a uniform bound
//!    at the same index silently alias.
//!
//! The rule is deliberately a pure function of the group index rather than an
//! accumulation across the pipeline layout, because the trait's
//! `create_bind_group` only receives the layout it was built from and never
//! learns which group index it will be bound at. A pure function means the MSL
//! side can hard-code indices without a build step.
//!
//! Vertex streams are placed at the top of the buffer table and counted
//! downward, which keeps them clear of the argument range for any group count
//! the renderer uses today (at most two) and any binding count per group (at
//! most four).

/// Bind groups the index rule can address without overlap.
///
/// Sized from the real inventory: the widest pipeline in the renderer uses
/// two groups, so four leaves headroom while keeping every index below the
/// vertex-stream reservation.
pub(crate) const MAX_BIND_GROUPS: u32 = 4;

/// Binding slots reserved per group.
///
/// Sized from the real inventory: material binds four entries in one group,
/// so four is the tightest power of two that fits.
pub(crate) const BIND_SLOTS_PER_GROUP: u32 = 4;

/// Highest argument index Metal accepts for a buffer, texture, or sampler.
///
/// Metal 2 raised the per-stage buffer limit to 31 indices (`0..=30`);
/// textures and samplers top out lower but every resource the renderer binds
/// fits.
pub(crate) const MAX_ARGUMENT_INDEX: u32 = 30;

/// Number of vertex streams reserved at the top of the buffer table.
pub(crate) const MAX_VERTEX_BUFFERS: u32 = 8;

/// Highest buffer index a group binding may produce.
///
/// Only read by the tests below, which assert that the two ranges in the buffer
/// table stay disjoint; the runtime never needs the boundary because the
/// assertions on `group` and `binding` are what enforce it.
#[cfg(test)]
const ARGUMENT_INDEX_MAX: u32 =
    (MAX_BIND_GROUPS - 1) * BIND_SLOTS_PER_GROUP + (BIND_SLOTS_PER_GROUP - 1);

/// Lowest buffer index a vertex stream may occupy.
const VERTEX_BUFFER_BASE: u32 = MAX_ARGUMENT_INDEX - MAX_VERTEX_BUFFERS + 1;

/// Number of vertex slots the rule can address.
const VERTEX_SLOT_COUNT: u32 = MAX_VERTEX_BUFFERS;

/// Buffer indices claimed by vertex streams, ascending.
#[cfg(test)]
pub(crate) const VERTEX_BUFFER_INDICES: std::ops::RangeInclusive<u32> =
    VERTEX_BUFFER_BASE..=MAX_ARGUMENT_INDEX;

/// Metal argument-table index for a `(group, binding)` pair.
///
/// The same value is used in each of the three tables independently, which is
/// legal because Metal's buffer, texture, and sampler tables do not overlap: a
/// texture at index 4 and a sampler at index 4 are distinct bindings.
///
/// Panics in debug builds when the pair cannot be addressed without colliding
/// with the vertex streams.
pub(crate) fn argument_index(group: u32, binding: u32) -> u32 {
    debug_assert!(
        group < MAX_BIND_GROUPS,
        "bind group {group} exceeds MAX_BIND_GROUPS ({MAX_BIND_GROUPS}); \
         raising it also requires moving VERTEX_BUFFER_BASE so the two ranges \
         stay disjoint"
    );
    debug_assert!(
        binding < BIND_SLOTS_PER_GROUP,
        "binding {binding} exceeds the {BIND_SLOTS_PER_GROUP} slots reserved per bind group"
    );
    group * BIND_SLOTS_PER_GROUP + binding
}

/// Buffer-table index for vertex stream `slot`.
///
/// Counted down from the top of the table so it can never collide with
/// [`argument_index`], which counts up from the bottom.
pub(crate) fn vertex_buffer_index(slot: u32) -> u32 {
    debug_assert!(
        slot < VERTEX_SLOT_COUNT,
        "vertex slot {slot} exceeds the {MAX_VERTEX_BUFFERS} reserved vertex streams"
    );
    VERTEX_BUFFER_BASE + slot
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `(group, binding)` pair the six pipelines actually declare, as
    /// `(pipeline, group, bindings-in-that-group)`. Kept in sync with the
    /// `*_generic` constructors in `src/pipeline/`.
    const REAL_BINDINGS: &[(&str, u32, &[u32])] = &[
        // rect_pipeline.rs: one group, one uniform.
        ("rect", 0, &[0]),
        // image_pipeline.rs: uniform group plus a texture/sampler group.
        ("image", 0, &[0]),
        ("image", 1, &[0, 1]),
        // text_pipeline.rs: one group holding uniform, atlas and sampler.
        ("text", 0, &[0, 1, 2]),
        // material/render.rs: dynamic uniform, capture, sampler and blur texture.
        ("material", 0, &[0, 1, 2, 3]),
        // frame_composite.rs: one texture only.
        ("frame_composite", 0, &[0]),
        // svg_pipeline.rs builds a pipeline layout with no bind groups.
        ("svg", 0, &[]),
    ];

    fn assert_within_table_limits(group: u32, binding: u32) {
        let index = argument_index(group, binding);
        assert!(
            index <= MAX_ARGUMENT_INDEX,
            "(group {group}, binding {binding}) maps to index {index}, above the \
             Metal limit of {MAX_ARGUMENT_INDEX}"
        );
        assert!(
            !VERTEX_BUFFER_INDICES.contains(&index),
            "(group {group}, binding {binding}) maps to index {index}, which the \
             vertex streams own"
        );
    }

    #[test]
    fn every_real_binding_pair_is_addressable_and_clear_of_vertex_streams() {
        for (pipeline, group, bindings) in REAL_BINDINGS {
            assert!(
                *group < MAX_BIND_GROUPS,
                "{pipeline} uses group {group}, above MAX_BIND_GROUPS"
            );
            for binding in *bindings {
                assert_within_table_limits(*group, *binding);
            }
        }
    }

    #[test]
    fn pairs_in_one_group_map_to_distinct_indices() {
        let mut seen = Vec::new();
        for binding in 0..BIND_SLOTS_PER_GROUP {
            seen.push(argument_index(0, binding));
        }
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "{seen:?} collide");
    }

    #[test]
    fn groups_never_overlap() {
        for a in 0..MAX_BIND_GROUPS {
            for b in 0..MAX_BIND_GROUPS {
                if a == b {
                    continue;
                }
                for ia in 0..BIND_SLOTS_PER_GROUP {
                    for ib in 0..BIND_SLOTS_PER_GROUP {
                        assert_ne!(argument_index(a, ia), argument_index(b, ib));
                    }
                }
            }
        }
    }

    #[test]
    fn worst_real_case_is_rect_and_material_at_three_indices_apart() {
        assert_eq!(argument_index(0, 0), 0);
        assert_eq!(argument_index(0, 2), 2);
        assert_eq!(argument_index(1, 1), 5);
    }

    #[test]
    fn vertex_streams_count_down_from_the_top_of_the_buffer_table() {
        assert_eq!(vertex_buffer_index(0), 23);
        assert_eq!(vertex_buffer_index(1), 24);
        assert_eq!(vertex_buffer_index(7), MAX_ARGUMENT_INDEX);
    }

    #[test]
    fn the_widest_supported_layout_still_leaves_the_tables_consistent() {
        // Pushing the rule to its own limit must keep argument indices below
        // the vertex reservation; if this fails, MAX_BIND_GROUPS is too large
        // for the vertex reservation and one of the two must move.
        let highest = argument_index(MAX_BIND_GROUPS - 1, BIND_SLOTS_PER_GROUP - 1);
        assert_eq!(highest, ARGUMENT_INDEX_MAX);
        assert!(highest < vertex_buffer_index(0));
    }
}
