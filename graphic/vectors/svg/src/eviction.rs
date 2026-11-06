//! Which cached meshes to drop when a mesh cache is over its limits.
//!
//! Shared by the CPU geometry cache and the GPU mesh cache of the SVG pipeline,
//! and kept apart from both so the decision can be tested without a GPU
//! backend.

/// A cached mesh that may be dropped.
///
/// `K` identifies the mesh to the caller; a caller that can drop by age alone
/// uses `()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate<K> {
    /// The cache's usage clock when the mesh was last used.
    pub last_used: u64,
    pub key: K,
    pub bytes: u64,
}

/// The most meshes and bytes the cache may keep.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub meshes: usize,
    pub bytes: u64,
}

impl Limits {
    /// Whether a cache holding `meshes` meshes of `bytes` bytes is over a limit.
    #[inline]
    pub fn exceeded_by(self, meshes: usize, bytes: u64) -> bool {
        meshes > self.meshes || bytes > self.bytes
    }

    /// Where eviction stops: an eighth under each limit.
    ///
    /// A cache that stays at its limit because new geometry arrives every frame
    /// (an animation re-tessellating its shapes) would otherwise evict a mesh
    /// per mesh inserted, and look for the oldest one each time. Stopping short
    /// pays for one search per eighth of the cache instead.
    #[inline]
    fn target(self) -> Self {
        Self {
            meshes: self.meshes - self.meshes / 8,
            bytes: self.bytes - self.bytes / 8,
        }
    }
}

/// Orders `candidates` oldest first and returns how many of them, from the
/// front, to drop so that a cache of `meshes` meshes and `bytes` bytes is back
/// inside `limits` with headroom. Drops fewer when the candidates run out:
/// meshes drawn this frame are never candidates.
pub fn oldest_to_drop<K: Ord>(
    candidates: &mut [Candidate<K>],
    mut meshes: usize,
    mut bytes: u64,
    limits: Limits,
) -> usize {
    let target = limits.target();
    candidates.sort_unstable_by(|a, b| (a.last_used, &a.key).cmp(&(b.last_used, &b.key)));
    let mut dropped = 0;
    for candidate in candidates.iter() {
        if !limits.exceeded_by(meshes, bytes) && !target.exceeded_by(meshes, bytes) {
            break;
        }
        meshes -= 1;
        bytes = bytes.saturating_sub(candidate.bytes);
        dropped += 1;
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(last_used: &[u64]) -> Vec<Candidate<usize>> {
        last_used
            .iter()
            .enumerate()
            .map(|(key, &last_used)| Candidate { last_used, key, bytes: 10 })
            .collect()
    }

    const LIMITS: Limits = Limits { meshes: 80, bytes: 1_000_000 };

    #[test]
    fn a_cache_inside_its_limits_is_not_over() {
        assert!(!LIMITS.exceeded_by(80, 1_000_000));
        assert!(LIMITS.exceeded_by(81, 0));
        assert!(LIMITS.exceeded_by(0, 1_000_001));
    }

    #[test]
    fn the_oldest_meshes_go_first() {
        // 90 meshes against a limit of 80, stopping at 70: twenty go, the
        // twenty least recently drawn.
        let mut all = candidates(&(0..90).rev().collect::<Vec<_>>());
        let dropped = oldest_to_drop(&mut all, 90, 900, LIMITS);

        assert_eq!(dropped, 20);
        assert!(all[..dropped].iter().all(|candidate| candidate.last_used < 20));
        assert!(all[dropped..].iter().all(|candidate| candidate.last_used >= 20));
    }

    #[test]
    fn eviction_stops_an_eighth_below_the_limit() {
        let mut all = candidates(&(0..81).collect::<Vec<_>>());
        let dropped = oldest_to_drop(&mut all, 81, 810, LIMITS);

        assert_eq!(81 - dropped, 70, "80 - 80 / 8");
    }

    #[test]
    fn a_cache_that_just_went_over_is_not_over_again_for_a_while() {
        let mut all = candidates(&(0..81).collect::<Vec<_>>());
        let dropped = oldest_to_drop(&mut all, 81, 810, LIMITS);

        // Ten more meshes arrive: still inside the limit, so nothing to search for.
        assert!(!LIMITS.exceeded_by(81 - dropped + 10, 0));
    }

    #[test]
    fn meshes_that_are_not_candidates_are_never_dropped() {
        // 100 meshes, but only 5 may go: the other 95 are drawn this frame.
        let mut all = candidates(&[1, 2, 3, 4, 5]);
        let dropped = oldest_to_drop(&mut all, 100, 1_000, LIMITS);

        assert_eq!(dropped, 5);
    }

    #[test]
    fn the_byte_limit_drives_eviction_on_its_own() {
        let limits = Limits { meshes: 1_000, bytes: 1_000 };
        let mut all: Vec<_> = (0..20)
            .map(|key| Candidate { last_used: key as u64, key, bytes: 100 })
            .collect();
        // 2,000 bytes held against 1,000: stop at 875, so ten go (to 1,000), then
        // two more (to 800).
        let dropped = oldest_to_drop(&mut all, 20, 2_000, limits);

        assert_eq!(dropped, 12);
    }

    #[test]
    fn equal_ages_are_dropped_in_a_fixed_order() {
        let mut first = candidates(&[7; 90]);
        let mut second = first.clone();
        second.reverse();

        let a = oldest_to_drop(&mut first, 90, 900, LIMITS);
        let b = oldest_to_drop(&mut second, 90, 900, LIMITS);

        assert_eq!(first[..a], second[..b], "the same meshes go whatever the input order");
    }

    #[test]
    fn nothing_over_means_nothing_dropped() {
        let mut all = candidates(&[1, 2, 3]);

        assert_eq!(oldest_to_drop(&mut all, 3, 30, LIMITS), 0);
    }
}
