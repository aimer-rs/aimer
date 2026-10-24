//! Bounded ownership of linear-memory regions lent to the permanent host.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ptr::NonNull;

use aimer_anteros::AbiStatus;

use crate::GuestError;

struct Allocation {
    pointer: NonNull<u8>,
    layout: Layout,
}

// SAFETY: `Allocation` uniquely owns its pointer and never dereferences it
// without exclusive access through the enclosing ledger. Moving that ownership
// to another thread cannot create aliases; the export bridge serializes every
// access with a mutex.
unsafe impl Send for Allocation {}

pub(crate) struct AllocationLedger {
    allocations: Vec<Allocation>,
    live_bytes: usize,
    max_live_allocations: usize,
    max_live_bytes: usize,
    max_alignment: usize,
}

impl AllocationLedger {
    pub(crate) const fn new(
        max_live_allocations: u32,
        max_live_bytes: u32,
        max_alignment: u32,
    ) -> Self {
        Self {
            allocations: Vec::new(),
            live_bytes: 0,
            max_live_allocations: max_live_allocations as usize,
            max_live_bytes: max_live_bytes as usize,
            max_alignment: max_alignment as usize,
        }
    }

    pub(crate) fn allocate(
        &mut self,
        length: usize,
        alignment: usize,
    ) -> Result<usize, GuestError> {
        if length == 0
            || alignment == 0
            || !alignment.is_power_of_two()
            || alignment > self.max_alignment
        {
            return Err(GuestError::new(AbiStatus::InvalidArgument));
        }
        let next_bytes = self
            .live_bytes
            .checked_add(length)
            .ok_or_else(|| GuestError::new(AbiStatus::ResourceExhausted))?;
        if self.allocations.len() >= self.max_live_allocations || next_bytes > self.max_live_bytes {
            return Err(GuestError::new(AbiStatus::ResourceExhausted));
        }
        let layout = Layout::from_size_align(length, alignment)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        // SAFETY: `layout` is non-zero and valid. Ownership of the returned
        // allocation moves into the ledger and is released exactly once by an
        // exact-tuple deallocation or this ledger's `Drop` implementation.
        let pointer = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| GuestError::new(AbiStatus::ResourceExhausted))?;
        self.allocations.push(Allocation { pointer, layout });
        self.live_bytes = next_bytes;
        Ok(pointer.as_ptr() as usize)
    }

    pub(crate) fn deallocate(
        &mut self,
        pointer: usize,
        length: usize,
        alignment: usize,
    ) -> Result<(), GuestError> {
        let index = self
            .allocations
            .iter()
            .position(|allocation| {
                allocation.pointer.as_ptr() as usize == pointer
                    && allocation.layout.size() == length
                    && allocation.layout.align() == alignment
            })
            .ok_or_else(|| GuestError::new(AbiStatus::InvalidArgument))?;
        let allocation = self.allocations.swap_remove(index);
        self.live_bytes -= allocation.layout.size();
        // SAFETY: the exact pointer and layout tuple was removed from the
        // ledger, so no later deallocation or drop can release it again.
        unsafe { dealloc(allocation.pointer.as_ptr(), allocation.layout) };
        Ok(())
    }

    pub(crate) fn read(&self, pointer: usize, length: usize) -> Result<&[u8], GuestError> {
        self.find_region(pointer, length)?;
        // SAFETY: `find_region` proves this range starts at a live allocation
        // and does not exceed its layout. Shared access lasts only for `self`.
        Ok(unsafe { std::slice::from_raw_parts(pointer as *const u8, length) })
    }

    pub(crate) fn write(
        &mut self,
        pointer: usize,
        capacity: usize,
        bytes: &[u8],
    ) -> Result<(), GuestError> {
        if bytes.len() > capacity {
            return Err(GuestError::new(AbiStatus::BufferTooSmall));
        }
        self.find_region(pointer, capacity)?;
        // SAFETY: `find_region` proves the destination range belongs to one
        // live allocation, and `bytes` cannot exceed the requested capacity.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len());
        }
        Ok(())
    }

    fn find_region(&self, pointer: usize, length: usize) -> Result<&Allocation, GuestError> {
        let requested_end = pointer
            .checked_add(length)
            .ok_or_else(|| GuestError::new(AbiStatus::InvalidArgument))?;
        self.allocations
            .iter()
            .find(|allocation| {
                let allocation_start = allocation.pointer.as_ptr() as usize;
                allocation_start
                    .checked_add(allocation.layout.size())
                    .is_some_and(|allocation_end| {
                        pointer >= allocation_start && requested_end <= allocation_end
                    })
            })
            .ok_or_else(|| GuestError::new(AbiStatus::InvalidArgument))
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn live_allocations(&self) -> usize {
        self.allocations.len()
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn live_bytes(&self) -> usize {
        self.live_bytes
    }
}

impl Drop for AllocationLedger {
    fn drop(&mut self) {
        for allocation in self.allocations.drain(..) {
            // SAFETY: every remaining entry still uniquely owns its exact
            // allocation tuple, and draining prevents a second release.
            unsafe { dealloc(allocation.pointer.as_ptr(), allocation.layout) };
        }
    }
}

#[cfg(test)]
mod tests {
    use aimer_anteros::AbiStatus;

    use super::AllocationLedger;

    #[test]
    fn allocation_ledger_enforces_exact_ownership_and_recovers_after_mismatch() {
        let mut memory = AllocationLedger::new(2, 24, 16);
        let first = memory.allocate(8, 8).unwrap();
        let second = memory.allocate(16, 16).unwrap();

        assert_eq!(
            memory.allocate(1, 1).unwrap_err().status(),
            AbiStatus::ResourceExhausted
        );
        assert_eq!(
            memory.deallocate(first, 7, 8).unwrap_err().status(),
            AbiStatus::InvalidArgument
        );
        memory.deallocate(first, 8, 8).unwrap();
        memory.deallocate(second, 16, 16).unwrap();

        let replacement = memory.allocate(24, 8).unwrap();
        memory.deallocate(replacement, 24, 8).unwrap();
    }

    #[test]
    fn allocation_ledger_allows_checked_disjoint_subranges_but_not_overflow() {
        let mut memory = AllocationLedger::new(1, 24, 8);
        let pointer = memory.allocate(24, 8).unwrap();

        memory.write(pointer, 8, &[1; 8]).unwrap();
        memory.write(pointer + 8, 16, &[2; 16]).unwrap();
        assert_eq!(memory.read(pointer, 8).unwrap(), &[1; 8]);
        assert_eq!(memory.read(pointer + 8, 16).unwrap(), &[2; 16]);
        assert!(memory.write(pointer + 9, 16, &[]).is_err());
        assert!(memory.read(usize::MAX, 2).is_err());

        memory.deallocate(pointer, 24, 8).unwrap();
    }

    #[test]
    fn allocation_ledger_rejects_zero_overflow_and_unsupported_alignment() {
        let mut memory = AllocationLedger::new(1, 16, 8);

        for (length, alignment) in [(0, 1), (1, 0), (1, 3), (1, 16), (17, 8)] {
            assert!(memory.allocate(length, alignment).is_err());
        }
        assert_eq!(memory.live_allocations(), 0);
        assert_eq!(memory.live_bytes(), 0);
    }
}
