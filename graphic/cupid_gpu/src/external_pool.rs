//! Which producers of shared GPU frames are still alive.
//!
//! A video decoder that shares its pictures through a ring of GPU buffers
//! (`NativeTexture::Dxgi`, `NativeTexture::AndroidHardwareBuffer`, ...) names the
//! ring with a pool id. A backend caches what it opened from those buffers; it
//! asks here which pools still exist so it can let go of the rest, instead of
//! holding a finished video's buffers until the device goes away.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

static NEXT: AtomicU32 = AtomicU32::new(1);
static LIVE: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

/// Keeps a pool id live until dropped.
#[derive(Debug)]
pub struct PoolRegistration {
    id: u32,
}

impl PoolRegistration {
    /// The id to put in frames; never reused within a process.
    #[inline]
    pub fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for PoolRegistration {
    fn drop(&mut self) {
        LIVE.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
    }
}

/// Registers a new pool.
pub fn register() -> PoolRegistration {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    LIVE.lock().unwrap_or_else(PoisonError::into_inner).insert(id);
    PoolRegistration { id }
}

/// Whether the pool `id` still has its registration.
pub fn is_live(id: u32) -> bool {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner).contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pool_is_live_until_its_registration_drops() {
        let pool = register();
        let id = pool.id();
        assert!(is_live(id));
        drop(pool);
        assert!(!is_live(id));
    }

    #[test]
    fn pool_ids_are_never_reused() {
        let first = register();
        let id = first.id();
        drop(first);
        assert_ne!(register().id(), id);
    }

    #[test]
    fn an_unknown_pool_is_not_live() {
        assert!(!is_live(u32::MAX));
    }
}
