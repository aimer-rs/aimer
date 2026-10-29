//! The hasher behind the type-keyed state map of a [`BuildContext`](super::BuildContext).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::rc::Rc;

/// The hasher for the type-keyed state map.
///
/// A [`TypeId`] is a compiler-generated digest, so running it through SipHash
/// only spends time: the map is consulted for every element on every frame and
/// updated twice per scoped state. The keys are types known at compile time,
/// never input, so the flooding resistance SipHash buys is not needed.
pub use aimer_cupid::utilities::IdHasher as TypeIdHasher;

/// The states a build context passes down to its descendants, by type.
pub type InheritedStates = HashMap<TypeId, Rc<dyn Any>, BuildHasherDefault<TypeIdHasher>>;

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::collections::HashMap;
    use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

    use super::*;

    macro_rules! distinct_types {
        ($($name:ident),* $(,)?) => {
            $(struct $name;)*
            fn type_ids() -> Vec<TypeId> {
                vec![$(TypeId::of::<$name>()),*]
            }
        };
    }

    distinct_types!(
        T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17,
        T18, T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35,
        T36, T37, T38, T39,
    );

    fn hash_of(id: TypeId) -> u64 {
        BuildHasherDefault::<TypeIdHasher>::default().hash_one(id)
    }

    #[test]
    fn equal_type_ids_hash_equal_and_distinct_ones_do_not_collide() {
        let ids = type_ids();
        let hashes = ids.iter().map(|id| hash_of(*id)).collect::<Vec<_>>();
        for (id, hash) in ids.iter().zip(&hashes) {
            assert_eq!(hash_of(*id), *hash);
        }
        let mut unique = hashes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "two type ids collided");
    }

    #[test]
    fn both_ends_of_the_hash_vary_for_the_bits_a_table_reads() {
        // A table indexes with the low bits and tags entries with the high
        // bits, so a hasher that only varied one end would cluster.
        let hashes = type_ids().iter().map(|id| hash_of(*id)).collect::<Vec<_>>();
        let mut low = hashes.iter().map(|hash| hash & 0x3f).collect::<Vec<_>>();
        let mut high = hashes.iter().map(|hash| hash >> 57).collect::<Vec<_>>();
        low.sort_unstable();
        low.dedup();
        high.sort_unstable();
        high.dedup();
        assert!(low.len() >= 15, "only {} distinct low-bit patterns", low.len());
        assert!(high.len() >= 15, "only {} distinct high-bit patterns", high.len());
    }

    #[test]
    fn byte_input_is_order_and_value_sensitive() {
        let hash = |bytes: &[u8]| {
            let mut hasher = TypeIdHasher::default();
            hasher.write(bytes);
            hasher.finish()
        };
        assert_eq!(hash(&[1, 2, 3]), hash(&[1, 2, 3]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[3, 2, 1]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[1, 2, 4]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[1, 2, 3, 0]));
        assert_ne!(hash(&[]), hash(&[0]));
        // A long slice is mixed in full, not just its first word.
        let mut long = [7u8; 40];
        let before = hash(&long);
        long[39] ^= 1;
        assert_ne!(before, hash(&long));
    }

    #[test]
    fn wide_integers_feed_both_halves() {
        let hash = |value: u128| {
            let mut hasher = TypeIdHasher::default();
            hasher.write_u128(value);
            hasher.finish()
        };
        assert_ne!(hash(1), hash(1 << 64));
        assert_ne!(hash(1), hash(2));
    }

    #[test]
    fn a_map_keyed_by_type_id_behaves_like_any_other_map() {
        let ids = type_ids();
        let mut map = HashMap::<TypeId, usize, BuildHasherDefault<TypeIdHasher>>::default();
        for (index, id) in ids.iter().enumerate() {
            assert_eq!(map.insert(*id, index), None);
        }
        for (index, id) in ids.iter().enumerate() {
            assert_eq!(map.get(id), Some(&index));
        }
        assert_eq!(map.insert(ids[3], 99), Some(3));
        for id in ids.iter().step_by(2) {
            map.remove(id);
        }
        assert_eq!(map.len(), ids.len() / 2);
        assert_eq!(map.get(&ids[3]), Some(&99));
        assert_eq!(map.get(&ids[2]), None);
        let _ = (&ids[0]).hash(&mut TypeIdHasher::default());
    }
}
