//! `consistency_suite!`: one `#[test]` per case, for one backend.
//!
//! Invoke it inside a `#[cfg(test)]` module of the backend:
//!
//! ```text
//! #[cfg(test)]
//! mod consistency {
//!     crate::consistency::consistency_suite!(super::MyBackend);
//! }
//! ```
//!
//! The list below must name every `pub fn` in `cases.rs`; a test in `mod.rs`
//! checks that.

macro_rules! consistency_suite {
    ($backend:ty) => {
        $crate::consistency::consistency_suite!(@cases $backend;
            routed_event_reaches_stateful_child_once,
            set_state_is_deferred_and_coalesced,
            init_runs_once_before_the_first_build,
            state_updater_rebuilds_only_its_dirty_subtree,
            a_nested_stateful_child_keeps_its_state_when_the_parent_updates,
            create_panic_becomes_error_element,
            init_panic_becomes_error_element,
            initial_build_panic_becomes_error_element,
            initial_child_conversion_panic_becomes_error_element,
            queued_mutation_panic_installs_a_stable_error_child,
            dirty_build_panic_installs_a_stable_error_child,
            rebuild_child_reconciliation_failure_becomes_a_stable_error_element,
            keyed_config_panic_becomes_error_element,
            carry_keeps_the_live_state_and_adopts_the_new_props,
            failed_unkeyed_adoption_keeps_the_old_owner_and_rejects_the_candidate,
            child_reconciliation_failure_keeps_the_old_owner_and_rejects_the_candidate,
            elements_with_the_same_name_but_another_state_type_keep_their_own_state,
            state_below_a_keyed_element_outlives_a_rebuild_of_its_ancestor,
            a_replaced_page_below_a_keyed_element_keeps_its_own_state,
            a_copied_handle_is_rejected_after_unmount,
            a_delayed_callback_is_rejected_after_unmount,
        );
    };
    (@cases $backend:ty; $($case:ident,)*) => {
        $(
            #[test]
            fn $case() {
                $crate::consistency::cases::$case::<$backend>();
            }
        )*
    };
}

pub(crate) use consistency_suite;
