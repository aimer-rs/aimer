#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::mem::{align_of, size_of};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    use crate::{DEFAULT_WORDS, ErasedFrom, INLINE_ALIGNMENT, INLINE_CAPACITY, Rubick};

    /// A word-sized owner reference plus the payload buffer.
    const OWNER_OVERHEAD: usize = size_of::<usize>();

    trait Value {
        fn value(&self) -> usize;
        fn set_value(&mut self, value: usize);
    }

    // SAFETY: The template is `null::<V>()` coerced to the target, so it
    // carries exactly `V`'s vtable and a null data address.
    unsafe impl<V: Value + 'static> ErasedFrom<V> for dyn Value {
        const TEMPLATE: *const Self = std::ptr::null::<V>() as *const dyn Value;
    }

    #[derive(Debug)]
    struct ExactBoundary([u8; INLINE_CAPACITY]);

    impl Value for ExactBoundary {
        fn value(&self) -> usize {
            usize::from(self.0[0])
        }

        fn set_value(&mut self, value: usize) {
            self.0[0] = value as u8;
        }
    }

    struct Oversized([u8; INLINE_CAPACITY + 1]);

    impl Value for Oversized {
        fn value(&self) -> usize {
            usize::from(self.0[0])
        }

        fn set_value(&mut self, value: usize) {
            self.0[0] = value as u8;
        }
    }

    #[repr(align(32))]
    struct OverAligned(u8);

    #[repr(C, align(16))]
    struct MaximallyAligned([u8; INLINE_CAPACITY]);

    #[repr(align(32))]
    struct OverAlignedZeroSized;

    struct Envelope {
        prefix: usize,
        value: usize,
        suffix: usize,
    }

    fn project_envelope_value(value: &Envelope) -> &usize {
        assert_eq!(value.prefix, 0xA1);
        assert_eq!(value.suffix, 0xB2);
        &value.value
    }

    fn project_envelope_value_mut(value: &mut Envelope) -> &mut usize {
        assert_eq!(value.prefix, 0xA1);
        assert_eq!(value.suffix, 0xB2);
        &mut value.value
    }

    fn project_value<U: Value + 'static>(value: &U) -> &(dyn Value + 'static) {
        value
    }

    fn project_value_mut<U: Value + 'static>(value: &mut U) -> &mut (dyn Value + 'static) {
        value
    }

    #[test]
    fn the_default_capacity_matches_representative_aimer_values() {
        assert_eq!(DEFAULT_WORDS, 4);
        assert_eq!(INLINE_CAPACITY, DEFAULT_WORDS * size_of::<usize>());
        assert_eq!(INLINE_ALIGNMENT, align_of::<usize>());
        assert_eq!(
            <Rubick<dyn Value>>::INLINE_CAPACITY,
            INLINE_CAPACITY,
            "the default capacity must agree with the crate constant"
        );
    }

    #[test]
    fn owners_cost_their_payload_plus_one_word() {
        assert_eq!(
            size_of::<Rubick<u32>>(),
            INLINE_CAPACITY + OWNER_OVERHEAD,
            "no storage tag and no per-instance operation table"
        );
        assert_eq!(
            size_of::<Rubick<dyn Value>>(),
            INLINE_CAPACITY + OWNER_OVERHEAD
        );
        assert_eq!(align_of::<Rubick<u32>>(), align_of::<usize>());
    }

    #[test]
    fn capacity_is_selected_per_alias() {
        assert_eq!(size_of::<Rubick<dyn Value, 1>>(), 2 * size_of::<usize>());
        assert_eq!(size_of::<Rubick<dyn Value, 8>>(), 9 * size_of::<usize>());
        assert_eq!(
            <Rubick<dyn Value, 8>>::INLINE_CAPACITY / size_of::<usize>(),
            8
        );

        let thin: Rubick<dyn Value, 1> = Rubick::erase(ExactBoundary([1; INLINE_CAPACITY]));
        let roomy: Rubick<dyn Value, 8> = Rubick::erase(ExactBoundary([1; INLINE_CAPACITY]));

        assert!(thin.is_heap(), "a thin owner never inlines a large payload");
        assert!(roomy.is_inline(), "a roomy owner inlines the same payload");
        assert_eq!(thin.value(), 1);
        assert_eq!(roomy.value(), 1);
    }

    #[test]
    fn a_roomy_owner_inlines_a_container_sized_value() {
        struct Container {
            fields: [usize; 8],
        }

        impl Value for Container {
            fn value(&self) -> usize {
                self.fields[0]
            }

            fn set_value(&mut self, value: usize) {
                self.fields[0] = value;
            }
        }

        let container: Rubick<dyn Value, 8> = Rubick::erase(Container { fields: [3; 8] });

        assert_eq!(size_of::<Container>() / size_of::<usize>(), 8);
        assert!(
            container.is_inline(),
            "eight words of capacity must hold an eight word value"
        );
        assert_eq!(container.value(), 3);
    }

    #[test]
    fn sized_values_dereference_and_mutate() {
        let mut value = Rubick::new(String::from("Aimer"));
        value.push_str(" GUI");

        assert_eq!(&*value, "Aimer GUI");
        assert_eq!(value.as_ref(), "Aimer GUI");
    }

    #[test]
    fn storage_selection_checks_size_alignment_and_zero_sized_values() {
        assert!(Rubick::new(()).is_inline());
        assert!(Rubick::new(ExactBoundary([0; INLINE_CAPACITY])).is_inline());
        assert!(Rubick::new(Oversized([0; INLINE_CAPACITY + 1])).is_heap());
        let over_aligned = Rubick::new(OverAligned(7));
        assert!(over_aligned.is_heap());
        assert_eq!(over_aligned.0, 7);
        assert!(align_of::<OverAligned>() > INLINE_ALIGNMENT);
    }

    #[test]
    fn storage_selection_handles_both_alignment_boundaries() {
        let maximally_aligned = Rubick::new(MaximallyAligned([7; INLINE_CAPACITY]));
        let over_aligned_zero_sized = Rubick::new(OverAlignedZeroSized);

        assert!(align_of::<MaximallyAligned>() > INLINE_ALIGNMENT);
        assert_eq!(maximally_aligned.0[INLINE_CAPACITY - 1], 7);
        assert!(
            maximally_aligned.is_heap(),
            "inline storage only guarantees word alignment"
        );

        assert_eq!(size_of::<OverAlignedZeroSized>(), 0);
        assert!(align_of::<OverAlignedZeroSized>() > INLINE_ALIGNMENT);
        assert!(over_aligned_zero_sized.is_heap());
    }

    #[test]
    fn erased_values_dispatch_without_an_adapter_inline_and_on_heap() {
        let mut inline: Rubick<dyn Value> = Rubick::erase(ExactBoundary([3; INLINE_CAPACITY]));
        let mut heap: Rubick<dyn Value> = Rubick::erase(Oversized([5; INLINE_CAPACITY + 1]));

        inline.set_value(11);
        heap.set_value(13);

        assert!(inline.is_inline());
        assert!(heap.is_heap());
        assert!(inline.is_direct());
        assert!(heap.is_direct());
        assert_eq!(inline.value(), 11);
        assert_eq!(heap.value(), 13);
    }

    #[test]
    fn erasing_stores_no_adapters_alongside_the_value() {
        let erased: Rubick<dyn Value> = Rubick::erase(ExactBoundary([0; INLINE_CAPACITY]));
        let projected: Rubick<dyn Value> = Rubick::new_projected(
            ExactBoundary([0; INLINE_CAPACITY]),
            project_value,
            project_value_mut,
        );

        assert!(
            erased.is_inline(),
            "a payload at the capacity boundary stays inline when erased"
        );
        assert!(!projected.is_direct());
        assert!(
            projected.is_inline(),
            "zero sized adapters do not consume capacity"
        );
    }

    #[test]
    fn capturing_adapters_count_toward_inline_capacity() {
        let flag = Rc::new(Cell::new(0_usize));
        let projected: Rubick<dyn Value> = Rubick::new_projected(
            ExactBoundary([0; INLINE_CAPACITY]),
            {
                let flag = Rc::clone(&flag);
                move |value: &ExactBoundary| {
                    flag.set(flag.get() + 1);
                    value as &(dyn Value + 'static)
                }
            },
            |value: &mut ExactBoundary| value as &mut (dyn Value + 'static),
        );

        assert!(projected.is_heap());
        assert_eq!(projected.value(), 0);
        assert_eq!(flag.get(), 1);
    }

    #[test]
    fn projected_trait_values_dispatch_inline_and_on_heap() {
        let mut inline: Rubick<dyn Value> = Rubick::new_projected(
            ExactBoundary([3; INLINE_CAPACITY]),
            project_value,
            project_value_mut,
        );
        let mut heap: Rubick<dyn Value> = Rubick::new_projected(
            Oversized([5; INLINE_CAPACITY + 1]),
            project_value,
            project_value_mut,
        );

        inline.set_value(11);
        heap.set_value(13);

        assert!(inline.is_inline());
        assert!(heap.is_heap());
        assert_eq!(inline.value(), 11);
        assert_eq!(heap.value(), 13);
    }

    #[test]
    fn moves_swaps_and_collection_growth_rebuild_projection() {
        let first: Rubick<dyn Value> = Rubick::erase(ExactBoundary([1; INLINE_CAPACITY]));
        let second: Rubick<dyn Value> = Rubick::new_projected(
            Oversized([2; INLINE_CAPACITY + 1]),
            project_value,
            project_value_mut,
        );
        let mut values = Vec::with_capacity(1);
        values.extend([first, second]);
        values.swap(0, 1);

        assert_eq!(values[0].value(), 2);
        assert_eq!(values[1].value(), 1);
    }

    #[test]
    fn repeated_relocation_preserves_a_projection_to_an_interior_field() {
        let owner: Rubick<usize> = Rubick::new_projected(
            Envelope {
                prefix: 0xA1,
                value: 7,
                suffix: 0xB2,
            },
            project_envelope_value,
            project_envelope_value_mut,
        );
        let mut owners = Vec::with_capacity(1);
        owners.push(owner);

        for expected in 8..=256 {
            owners.push(Rubick::new(expected));
            let last = owners.len() - 1;
            owners.swap(0, last);
            owners.swap(0, last);
            *owners[0] += 1;
            assert_eq!(*owners[0], expected);
        }
    }

    #[test]
    fn captured_projection_adapters_keep_state_across_owner_moves() {
        let shared_calls = Rc::new(Cell::new(0));
        let mutable_calls = Rc::new(Cell::new(0));
        let mut owner: Rubick<usize> = Rubick::new_projected(
            10_usize,
            {
                let shared_calls = Rc::clone(&shared_calls);
                move |value: &usize| {
                    shared_calls.set(shared_calls.get() + 1);
                    value
                }
            },
            {
                let mutable_calls = Rc::clone(&mutable_calls);
                move |value: &mut usize| {
                    mutable_calls.set(mutable_calls.get() + 1);
                    value
                }
            },
        );

        owner = std::hint::black_box(owner);
        assert_eq!(*owner, 10);
        *owner = 12;
        assert_eq!(*owner, 12);
        assert_eq!(shared_calls.get(), 2);
        assert_eq!(mutable_calls.get(), 1);
    }

    #[test]
    fn panicking_projection_does_not_corrupt_the_owned_value() {
        let should_panic = Rc::new(Cell::new(true));
        let mut owner: Rubick<usize> = Rubick::new_projected(
            41_usize,
            {
                let should_panic = Rc::clone(&should_panic);
                move |value: &usize| {
                    assert!(!should_panic.replace(false), "project once with a panic");
                    value
                }
            },
            |value: &mut usize| value,
        );

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = *owner;
        }));
        assert!(result.is_err());

        *owner += 1;
        assert_eq!(*owner, 42);
    }

    struct DropValue<const N: usize> {
        drops: Rc<Cell<usize>>,
        bytes: [u8; N],
    }

    impl<const N: usize> Drop for DropValue<N> {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    trait DropTarget {}

    impl<const N: usize> DropTarget for DropValue<N> {}

    // SAFETY: The template is `null::<D>()` coerced to the target.
    unsafe impl<D: DropTarget + 'static> ErasedFrom<D> for dyn DropTarget {
        const TEMPLATE: *const Self = std::ptr::null::<D>() as *const dyn DropTarget;
    }

    fn project_drop_target<U: DropTarget + 'static>(value: &U) -> &(dyn DropTarget + 'static) {
        value
    }

    fn project_drop_target_mut<U: DropTarget + 'static>(
        value: &mut U,
    ) -> &mut (dyn DropTarget + 'static) {
        value
    }

    #[test]
    fn values_drop_exactly_once_in_both_modes() {
        let inline_drops = Rc::new(Cell::new(0));
        let heap_drops = Rc::new(Cell::new(0));
        {
            let _inline = Rubick::new(DropValue::<0> {
                drops: Rc::clone(&inline_drops),
                bytes: [],
            });
            let _heap = Rubick::new(DropValue::<INLINE_CAPACITY> {
                drops: Rc::clone(&heap_drops),
                bytes: [0; INLINE_CAPACITY],
            });
        }

        assert_eq!(inline_drops.get(), 1);
        assert_eq!(heap_drops.get(), 1);
    }

    #[test]
    fn replacing_an_owner_drops_each_value_once() {
        let first_drops = Rc::new(Cell::new(0));
        let second_drops = Rc::new(Cell::new(0));
        let mut owner = Rubick::new(DropValue::<0> {
            drops: Rc::clone(&first_drops),
            bytes: [],
        });

        let old = std::mem::replace(
            &mut owner,
            Rubick::new(DropValue::<0> {
                drops: Rc::clone(&second_drops),
                bytes: [],
            }),
        );
        drop(old);
        assert_eq!(first_drops.get(), 1);
        assert_eq!(second_drops.get(), 0);

        drop(owner);
        assert_eq!(second_drops.get(), 1);
    }

    #[test]
    fn replace_destroys_the_old_value_and_installs_the_new_one() {
        let first_drops = Rc::new(Cell::new(0));
        let second_drops = Rc::new(Cell::new(0));
        let mut owner: Rubick<dyn DropTarget> = Rubick::erase(DropValue::<0> {
            drops: Rc::clone(&first_drops),
            bytes: [],
        });

        owner.replace(DropValue::<0> {
            drops: Rc::clone(&second_drops),
            bytes: [],
        });
        assert_eq!(first_drops.get(), 1);
        assert_eq!(second_drops.get(), 0);

        drop(owner);
        assert_eq!(second_drops.get(), 1);
    }

    #[test]
    fn replace_reuses_a_heap_block_of_the_same_class() {
        let drops = Rc::new(Cell::new(0));
        let mut owner: Rubick<dyn DropTarget> = Rubick::erase(DropValue::<INLINE_CAPACITY> {
            drops: Rc::clone(&drops),
            bytes: [0; INLINE_CAPACITY],
        });
        assert!(owner.is_heap());
        let block = (&*owner) as *const dyn DropTarget as *const u8;

        owner.replace(DropValue::<INLINE_CAPACITY> {
            drops: Rc::clone(&drops),
            bytes: [1; INLINE_CAPACITY],
        });

        assert_eq!(drops.get(), 1);
        assert!(owner.is_heap());
        assert_eq!(
            (&*owner) as *const dyn DropTarget as *const u8,
            block,
            "an identical layout must reuse the existing block"
        );
        drop(owner);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn replace_switches_between_storage_modes() {
        let drops = Rc::new(Cell::new(0));
        let mut owner: Rubick<dyn DropTarget> = Rubick::erase(DropValue::<0> {
            drops: Rc::clone(&drops),
            bytes: [],
        });
        assert!(owner.is_inline());

        owner.replace(DropValue::<INLINE_CAPACITY> {
            drops: Rc::clone(&drops),
            bytes: [7; INLINE_CAPACITY],
        });
        assert!(owner.is_heap());
        assert_eq!(drops.get(), 1);

        owner.replace(DropValue::<0> {
            drops: Rc::clone(&drops),
            bytes: [],
        });
        assert!(owner.is_inline());
        assert_eq!(drops.get(), 2);

        drop(owner);
        assert_eq!(drops.get(), 3);
    }

    #[test]
    fn values_drop_during_panic_unwinding() {
        let inline_drops = Rc::new(Cell::new(0));
        let heap_drops = Rc::new(Cell::new(0));
        let result = catch_unwind(AssertUnwindSafe({
            let inline_drops = Rc::clone(&inline_drops);
            let heap_drops = Rc::clone(&heap_drops);
            move || {
                let _inline = Rubick::new(DropValue::<0> {
                    drops: inline_drops,
                    bytes: [],
                });
                let _heap = Rubick::new(DropValue::<INLINE_CAPACITY> {
                    drops: heap_drops,
                    bytes: [0; INLINE_CAPACITY],
                });
                panic!("exercise unwind drop");
            }
        }));

        assert!(result.is_err());
        assert_eq!(inline_drops.get(), 1);
        assert_eq!(heap_drops.get(), 1);
    }

    #[test]
    fn nested_and_mixed_mode_owners_drop_every_value_once() {
        const OWNER_COUNT: usize = 128;

        let drops = Rc::new(Cell::new(0));
        let mut owners = Vec::with_capacity(1);
        for index in 0..OWNER_COUNT {
            let owner: Rubick<dyn DropTarget> = if index % 2 == 0 {
                Rubick::erase(DropValue::<0> {
                    drops: Rc::clone(&drops),
                    bytes: [],
                })
            } else {
                Rubick::new_projected(
                    DropValue::<INLINE_CAPACITY> {
                        drops: Rc::clone(&drops),
                        bytes: [0; INLINE_CAPACITY],
                    },
                    project_drop_target,
                    project_drop_target_mut,
                )
            };
            if index % 2 == 0 {
                assert!(owner.is_inline());
            } else {
                assert!(owner.is_heap());
            }
            owners.push(owner);
        }
        owners.reverse();
        owners.rotate_left(37);
        drop(owners);
        assert_eq!(drops.get(), OWNER_COUNT);

        let nested_drops = Rc::new(Cell::new(0));
        let inner = Rubick::new(DropValue::<0> {
            drops: Rc::clone(&nested_drops),
            bytes: [],
        });
        let outer = Rubick::new(inner);
        assert!(
            outer.is_heap(),
            "a five word owner does not fit four words of capacity"
        );
        drop(outer);
        assert_eq!(nested_drops.get(), 1);
    }

    #[test]
    fn recycled_blocks_keep_distinct_live_payloads_separate() {
        const OWNER_COUNT: usize = 512;

        let mut owners: Vec<Rubick<dyn Value>> = Vec::with_capacity(OWNER_COUNT);
        for index in 0..OWNER_COUNT {
            let mut payload = Oversized([0; INLINE_CAPACITY + 1]);
            payload.0[0] = index as u8;
            owners.push(Rubick::erase(payload));
        }
        for (index, owner) in owners.iter().enumerate() {
            assert_eq!(owner.value(), index as u8 as usize);
        }

        owners.truncate(OWNER_COUNT / 2);
        for index in 0..OWNER_COUNT / 2 {
            let mut payload = Oversized([0; INLINE_CAPACITY + 1]);
            payload.0[0] = (index + 1) as u8;
            owners.push(Rubick::erase(payload));
        }
        for (index, owner) in owners.iter().enumerate() {
            let expected = if index < OWNER_COUNT / 2 {
                index as u8
            } else {
                (index - OWNER_COUNT / 2 + 1) as u8
            };
            assert_eq!(owner.value(), usize::from(expected));
        }
    }

    #[test]
    fn owner_layout_and_unpin_contract_are_fixed() {
        fn assert_unpin<T: Unpin>() {}

        assert_unpin::<Rubick<u32>>();
        assert_eq!(size_of::<Rubick<u32>>(), INLINE_CAPACITY + OWNER_OVERHEAD);
        assert_eq!(align_of::<Rubick<u32>>(), INLINE_ALIGNMENT);
    }

    #[test]
    fn owners_are_conservatively_not_send_or_sync() {
        trait AmbiguousIfSend<A> {
            fn check() {}
        }

        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

        trait AmbiguousIfSync<A> {
            fn check() {}
        }

        impl<T: ?Sized> AmbiguousIfSync<()> for T {}
        impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}

        <Rubick<u32> as AmbiguousIfSend<_>>::check();
        <Rubick<u32> as AmbiguousIfSync<_>>::check();
    }

    #[test]
    fn take_moves_the_value_out_and_drops_it_exactly_once() {
        let drops = Rc::new(Cell::new(0));
        let owner = Rubick::new(DropValue::<0> {
            drops: Rc::clone(&drops),
            bytes: [],
        });
        assert!(owner.is_inline());

        // SAFETY: The consumer moves the value out exactly once.
        let value = unsafe { owner.take(|target| target.read()) };
        assert_eq!(drops.get(), 0, "taking must not run the destructor");

        drop(value);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn take_returns_a_heap_block_to_the_pool() {
        let drops = Rc::new(Cell::new(0));
        let owner = Rubick::new(DropValue::<INLINE_CAPACITY> {
            drops: Rc::clone(&drops),
            bytes: [0; INLINE_CAPACITY],
        });
        assert!(owner.is_heap());
        let block = (&*owner) as *const DropValue<INLINE_CAPACITY> as *const u8;

        // SAFETY: The consumer moves the value out exactly once.
        let value = unsafe { owner.take(|target| target.read()) };
        drop(value);
        assert_eq!(drops.get(), 1);

        let recycled = Rubick::new(DropValue::<INLINE_CAPACITY> {
            drops: Rc::clone(&drops),
            bytes: [0; INLINE_CAPACITY],
        });
        assert_eq!(
            (&*recycled) as *const DropValue<INLINE_CAPACITY> as *const u8,
            block,
            "a taken block must go back to its free list instead of leaking"
        );
    }

    #[test]
    fn take_does_not_drop_twice_when_the_consumer_unwinds() {
        let drops = Rc::new(Cell::new(0));
        let result = catch_unwind(AssertUnwindSafe({
            let drops = Rc::clone(&drops);
            move || {
                let owner = Rubick::new(DropValue::<INLINE_CAPACITY> {
                    drops,
                    bytes: [0; INLINE_CAPACITY],
                });
                // SAFETY: The consumer moves the value out exactly once; the
                // moved value is then dropped by the unwind.
                unsafe {
                    owner.take(|target| -> () {
                        let _moved = target.read();
                        panic!("exercise unwind out of take");
                    })
                }
            }
        }));

        assert!(result.is_err());
        assert_eq!(drops.get(), 1, "the moved value is destroyed by the unwind");
    }

    #[test]
    fn test_fixture_uses_heap_payload_bytes() {
        let value = DropValue::<3> {
            drops: Rc::new(Cell::new(0)),
            bytes: [1, 2, 3],
        };
        assert_eq!(value.bytes.len(), 3);
    }
}
