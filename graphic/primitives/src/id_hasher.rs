//! A hasher for keys that are already identifiers.

use std::hash::{BuildHasherDefault, Hasher};

/// Multiplier of the Fx hash: an odd constant with well-spread bits.
const MIX: u64 = 0x517c_c1b7_2722_0a95;

/// A hasher for keys that are small integers or digests, such as a counter
/// handed out one by one or a compiler-generated type id.
///
/// Such keys need no scrambling beyond spreading their bits, and the maps they
/// index are consulted for every node on every frame. A general-purpose hasher
/// spends many calls per key doing a job one rotate, xor and multiply per word
/// does well enough, which an unoptimized build feels most. The keys come from
/// the program itself, never from input, so the flooding resistance a
/// randomized hasher provides is not needed.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
        // Distinguish `[1]` from `[1, 0]`, which pad to the same word.
        self.write_u64(bytes.len() as u64);
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(MIX);
    }

    #[inline]
    fn write_u128(&mut self, value: u128) {
        self.write_u64(value as u64);
        self.write_u64((value >> 64) as u64);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

/// Builds [`IdHasher`]s, for use as the hasher parameter of a map.
pub type IdBuildHasher = BuildHasherDefault<IdHasher>;

#[cfg(test)]
mod tests {
    use std::hash::BuildHasher;

    use super::*;

    fn hash_of(value: u64) -> u64 {
        IdBuildHasher::default().hash_one(value)
    }

    #[test]
    fn sequential_ids_do_not_collide() {
        let mut hashes = (1..=4096u64).map(hash_of).collect::<Vec<_>>();
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), 4096);
    }

    #[test]
    fn sequential_ids_spread_across_both_ends_of_the_hash() {
        // A table indexes with the low bits and tags entries with the high
        // bits; counters differ only in their low bits to begin with.
        let hashes = (1..=256u64).map(hash_of).collect::<Vec<_>>();
        let mut low = hashes.iter().map(|hash| hash & 0xff).collect::<Vec<_>>();
        let mut high = hashes.iter().map(|hash| hash >> 57).collect::<Vec<_>>();
        low.sort_unstable();
        low.dedup();
        high.sort_unstable();
        high.dedup();
        assert!(
            low.len() >= 160,
            "only {} distinct low bytes for 256 ids",
            low.len()
        );
        assert!(
            high.len() >= 80,
            "only {} distinct tags for 256 ids",
            high.len()
        );
    }

    #[test]
    fn byte_input_is_order_and_value_sensitive() {
        let hash = |bytes: &[u8]| {
            let mut hasher = IdHasher::default();
            hasher.write(bytes);
            hasher.finish()
        };
        assert_eq!(hash(&[1, 2, 3]), hash(&[1, 2, 3]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[3, 2, 1]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[1, 2, 4]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[1, 2, 3, 0]));
        assert_ne!(hash(&[]), hash(&[0]));
        let mut long = [7u8; 40];
        let before = hash(&long);
        long[39] ^= 1;
        assert_ne!(before, hash(&long));
    }

    #[test]
    fn wide_and_pointer_sized_integers_feed_every_bit() {
        let wide = |value: u128| {
            let mut hasher = IdHasher::default();
            hasher.write_u128(value);
            hasher.finish()
        };
        assert_ne!(wide(1), wide(1 << 64));
        assert_ne!(wide(1), wide(2));

        let size = |value: usize| {
            let mut hasher = IdHasher::default();
            hasher.write_usize(value);
            hasher.finish()
        };
        assert_eq!(size(7), hash_of(7));
        assert_ne!(size(7), size(8));
    }

    #[test]
    fn a_map_with_this_hasher_behaves_like_any_other_map() {
        let mut map = hashbrown::HashMap::<u64, u64, IdBuildHasher>::default();
        for id in 1..=500u64 {
            assert_eq!(map.insert(id, id * 2), None);
        }
        for id in 1..=500u64 {
            assert_eq!(map.get(&id), Some(&(id * 2)));
        }
        assert_eq!(map.insert(250, 0), Some(500));
        for id in (1..=500u64).step_by(2) {
            map.remove(&id);
        }
        assert_eq!(map.len(), 250);
        assert_eq!(map.get(&250), Some(&0));
        assert_eq!(map.get(&251), None);
    }
}
