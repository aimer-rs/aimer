use std::ops::Range;
use std::sync::{Arc, Mutex};

use ash::vk;

use super::VulkanCore;

const DEVICE_LOCAL_BLOCK_SIZE: u64 = 64 * 1024 * 1024;
const HOST_VISIBLE_BLOCK_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResourceClass {
    Linear,
    OptimalImage,
}

pub(super) struct MemoryAllocator {
    core: Arc<VulkanCore>,
    blocks: Mutex<Vec<Arc<MemoryBlock>>>,
    non_coherent_atom_size: u64,
}

struct MemoryBlock {
    core: Arc<VulkanCore>,
    memory: vk::DeviceMemory,
    memory_type_index: u32,
    memory_flags: vk::MemoryPropertyFlags,
    class: ResourceClass,
    size: u64,
    mapped_address: usize,
    free_ranges: Mutex<Vec<Range<u64>>>,
}

struct AllocationInner {
    block: Arc<MemoryBlock>,
    offset: u64,
    size: u64,
    reserved_size: u64,
}

#[derive(Clone)]
pub(super) struct Allocation(Arc<AllocationInner>);

impl MemoryAllocator {
    pub(super) fn new(core: Arc<VulkanCore>) -> Self {
        Self {
            non_coherent_atom_size: core.properties.limits.non_coherent_atom_size.max(1),
            core,
            blocks: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn allocate(
        &self,
        requirements: vk::MemoryRequirements,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
        class: ResourceClass,
    ) -> Allocation {
        let memory_type = self
            .find_memory_type(requirements.memory_type_bits, required, preferred)
            .unwrap_or_else(|| {
                panic!(
                    "Vulkan has no memory type matching {:?} for {} bytes",
                    required, requirements.size
                )
            });
        let memory_flags =
            self.core.memory_properties.memory_types[memory_type as usize].property_flags;
        let non_coherent_host_memory = memory_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
            && !memory_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT);
        let atom_alignment = if non_coherent_host_memory {
            self.non_coherent_atom_size
        } else {
            1
        };
        let allocation_alignment = requirements.alignment.max(atom_alignment).max(1);
        let reserved_size = align_up(requirements.size, atom_alignment);
        let mut blocks = self
            .blocks
            .lock()
            .expect("Vulkan memory allocator mutex is not poisoned");

        for block in blocks
            .iter()
            .filter(|block| block.memory_type_index == memory_type && block.class == class)
        {
            if let Some(offset) = allocate_range(
                &mut block
                    .free_ranges
                    .lock()
                    .expect("Vulkan memory block mutex is not poisoned"),
                reserved_size,
                allocation_alignment,
            ) {
                return Allocation(Arc::new(AllocationInner {
                    block: block.clone(),
                    offset,
                    size: requirements.size,
                    reserved_size,
                }));
            }
        }

        let default_size = if memory_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) {
            HOST_VISIBLE_BLOCK_SIZE
        } else {
            DEVICE_LOCAL_BLOCK_SIZE
        };
        let block_size = align_up(reserved_size.max(default_size), allocation_alignment);
        let allocation_info = vk::MemoryAllocateInfo::default()
            .allocation_size(block_size)
            .memory_type_index(memory_type);
        let memory = unsafe {
            self.core
                .device
                .allocate_memory(&allocation_info, None)
                .expect("allocate Vulkan device memory")
        };
        let mapped_address = if memory_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) {
            let mapped = unsafe {
                self.core
                    .device
                    .map_memory(memory, 0, block_size, vk::MemoryMapFlags::empty())
                    .expect("map host-visible Vulkan memory")
            };
            mapped as usize
        } else {
            0
        };
        let block = Arc::new(MemoryBlock {
            core: self.core.clone(),
            memory,
            memory_type_index: memory_type,
            memory_flags,
            class,
            size: block_size,
            mapped_address,
            free_ranges: Mutex::new(vec![0..block_size]),
        });
        let offset = allocate_range(
            &mut block
                .free_ranges
                .lock()
                .expect("Vulkan memory block mutex is not poisoned"),
            reserved_size,
            allocation_alignment,
        )
        .expect("new Vulkan memory block must fit its first allocation");
        blocks.push(block.clone());
        Allocation(Arc::new(AllocationInner {
            block,
            offset,
            size: requirements.size,
            reserved_size,
        }))
    }

    fn find_memory_type(
        &self,
        type_bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        let properties = &self.core.memory_properties;
        (0..properties.memory_type_count)
            .filter(|index| type_bits & (1 << index) != 0)
            .filter(|index| {
                properties.memory_types[*index as usize]
                    .property_flags
                    .contains(required)
            })
            .max_by_key(|index| {
                (properties.memory_types[*index as usize].property_flags & preferred)
                    .as_raw()
                    .count_ones()
            })
    }
}

impl Allocation {
    pub(super) fn memory(&self) -> vk::DeviceMemory {
        self.0.block.memory
    }

    pub(super) fn offset(&self) -> u64 {
        self.0.offset
    }

    pub(super) fn is_host_visible(&self) -> bool {
        self.0.block.mapped_address != 0
    }

    pub(super) fn write(&self, offset: u64, bytes: &[u8]) {
        assert!(
            self.is_host_visible(),
            "Vulkan allocation is not host-visible"
        );
        assert!(offset.saturating_add(bytes.len() as u64) <= self.0.size);
        let destination = (self.0.block.mapped_address as *mut u8)
            .wrapping_add((self.0.offset + offset) as usize);
        // SAFETY: the block is persistently mapped, and the preceding bounds
        // check keeps this exact copy within the allocation's requested size.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
        self.flush(offset, bytes.len() as u64);
    }

    pub(super) fn read(&self, size: u64) -> Vec<u8> {
        assert!(
            self.is_host_visible(),
            "Vulkan allocation is not host-visible"
        );
        assert!(size <= self.0.size);
        self.invalidate(0, size);
        let source =
            (self.0.block.mapped_address as *const u8).wrapping_add(self.0.offset as usize);
        let mut bytes = vec![0; size as usize];
        // SAFETY: the block is persistently mapped and `size` was checked
        // against this allocation before constructing the source pointer.
        unsafe { std::ptr::copy_nonoverlapping(source, bytes.as_mut_ptr(), size as usize) };
        bytes
    }

    pub(super) fn flush(&self, offset: u64, size: u64) {
        self.flush_or_invalidate(offset, size, true);
    }

    pub(super) fn invalidate(&self, offset: u64, size: u64) {
        self.flush_or_invalidate(offset, size, false);
    }

    fn flush_or_invalidate(&self, offset: u64, size: u64, flush: bool) {
        if size == 0
            || self
                .0
                .block
                .memory_flags
                .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
        {
            return;
        }
        let atom = self
            .0
            .block
            .core
            .properties
            .limits
            .non_coherent_atom_size
            .max(1);
        let start = (self.0.offset + offset) / atom * atom;
        let end = align_up(self.0.offset + offset + size, atom).min(self.0.block.size);
        let range = vk::MappedMemoryRange::default()
            .memory(self.0.block.memory)
            .offset(start)
            .size(end.saturating_sub(start));
        unsafe {
            if flush {
                self.0
                    .block
                    .core
                    .device
                    .flush_mapped_memory_ranges(&[range])
            } else {
                self.0
                    .block
                    .core
                    .device
                    .invalidate_mapped_memory_ranges(&[range])
            }
            .expect("synchronize non-coherent Vulkan memory");
        }
    }
}

impl Drop for AllocationInner {
    fn drop(&mut self) {
        let mut free = self
            .block
            .free_ranges
            .lock()
            .expect("Vulkan memory block mutex is not poisoned");
        free_range(&mut free, self.offset..self.offset + self.reserved_size);
    }
}

impl Drop for MemoryBlock {
    fn drop(&mut self) {
        unsafe {
            if self.mapped_address != 0 {
                self.core.device.unmap_memory(self.memory);
            }
            self.core.device.free_memory(self.memory, None);
        }
    }
}

fn allocate_range(ranges: &mut Vec<Range<u64>>, size: u64, alignment: u64) -> Option<u64> {
    for index in 0..ranges.len() {
        let range = ranges[index].clone();
        let start = align_up(range.start, alignment);
        let end = start.checked_add(size)?;
        if end > range.end {
            continue;
        }
        ranges.remove(index);
        if range.start < start {
            ranges.push(range.start..start);
        }
        if end < range.end {
            ranges.push(end..range.end);
        }
        ranges.sort_unstable_by_key(|range| range.start);
        return Some(start);
    }
    None
}

fn free_range(ranges: &mut Vec<Range<u64>>, range: Range<u64>) {
    ranges.push(range);
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<u64>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(previous) = merged.last_mut()
            && previous.end >= range.start
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
}

fn align_up(value: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    value.saturating_add(alignment - 1) / alignment * alignment
}

#[cfg(test)]
mod tests {
    use super::{allocate_range, free_range};

    #[test]
    fn allocation_ranges_honor_alignment_and_split_free_space() {
        let mut ranges = vec![3..64];
        assert_eq!(allocate_range(&mut ranges, 8, 16), Some(16));
        assert_eq!(ranges, vec![3..16, 24..64]);
    }

    #[test]
    fn freeing_adjacent_ranges_coalesces_them() {
        let mut ranges = vec![0..8, 16..24, 24..40];
        free_range(&mut ranges, 8..16);
        assert_eq!(ranges, vec![0..40]);
    }
}
