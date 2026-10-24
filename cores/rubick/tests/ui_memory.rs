use std::alloc::Layout;

use aimer_rubick::allocator_api2::alloc::Allocator;
use aimer_rubick::allocator_api2::boxed::Box;
use aimer_rubick::allocator_api2::vec::Vec;
use aimer_rubick::{Rubick, UiMemory};

const TWO_MIB: usize = 2 * 1024 * 1024;

#[test]
fn ui_allocator_supports_allocator_api_collections_and_enforces_its_limit() {
    let memory = UiMemory::new(TWO_MIB);
    let allocator = memory.allocator();

    let mut values = Vec::new_in(allocator.clone());
    values.extend([2_u64, 3, 5, 7]);
    let label = Box::new_in(String::from("Rubick"), allocator.clone());
    assert_eq!(&*label, "Rubick");
    assert_eq!(&values, &[2, 3, 5, 7]);

    let large = Layout::from_size_align(TWO_MIB * 2, 16).unwrap();
    assert!(allocator.allocate(large).is_err());
}

#[test]
fn ui_owned_rubick_keeps_its_pool_alive_and_drops_outside_the_scope() {
    let memory = UiMemory::new(TWO_MIB);
    let allocator = memory.allocator();
    let mut value: Rubick<[u8; 64], 1> = Rubick::erase_in([7; 64], &allocator);
    assert!(value.is_heap());
    assert_eq!(allocator.committed_bytes(), TWO_MIB);
    drop(memory);

    assert_eq!(value[0], 7);
    value.replace([9; 64]);
    assert_eq!(value[0], 9);
    drop(value);
}

#[test]
fn ui_heap_grows_by_another_region_when_the_first_is_full() {
    let memory = UiMemory::new(2 * TWO_MIB);
    let allocator = memory.allocator();
    let layout = Layout::from_size_align(3 * TWO_MIB / 4, 16).unwrap();

    let first = allocator.allocate(layout).unwrap();
    assert_eq!(allocator.committed_bytes(), TWO_MIB);
    let second = allocator.allocate(layout).unwrap();
    assert_eq!(allocator.committed_bytes(), 2 * TWO_MIB);

    unsafe {
        allocator.deallocate(first.cast(), layout);
        allocator.deallocate(second.cast(), layout);
    }
}

#[test]
fn ui_allocator_honors_alignment_and_zero_sized_allocations() {
    let memory = UiMemory::new(TWO_MIB);
    let allocator = memory.allocator();
    let aligned = Layout::from_size_align(33, 256).unwrap();
    let block = allocator.allocate(aligned).unwrap();
    assert_eq!(block.as_ptr() as *mut u8 as usize % aligned.align(), 0);
    unsafe { allocator.deallocate(block.cast(), aligned) };

    let no_memory = UiMemory::new(0).allocator();
    let zero_sized = Layout::from_size_align(0, 4096).unwrap();
    let block = no_memory.allocate(zero_sized).unwrap();
    assert_eq!(block.as_ptr() as *mut u8 as usize % zero_sized.align(), 0);
    unsafe { no_memory.deallocate(block.cast(), zero_sized) };
}
