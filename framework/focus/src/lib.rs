//! Keyboard focus ownership for Aimer.
//!
//! Exactly one focus target may own keyboard and input-method events at a time.
//! This crate owns that decision and nothing else: it holds no reference to an
//! element tree, so focus policy can be reasoned about — and tested — on its
//! own.
//!
//! The pieces are:
//!
//! - [`FocusNode`], the handle a focusable widget keeps across rebuilds. It
//!   records requests ([`FocusNode::request_focus`], [`FocusNode::unfocus`]) and
//!   reports ownership ([`FocusNode::has_focus`]).
//! - [`FocusCandidate`], one focusable target paired with the identity of the
//!   element it is attached to.
//! - [`FocusManager`], the owner of the focused attachment. It resolves the
//!   candidates of a frame into a target and reports the resulting
//!   [`FocusTransition`].
//! - [`FocusTrap`], a guard that confines focus to one region of the
//!   application — what makes a modal a *mode* — while it is held.
//!
//! The host — `aimer_widget` — supplies the identities, gathers candidates in
//! traversal order, and turns a transition into `FocusLost` / `FocusGained`
//! notifications. That is the whole contract, which is why the manager is
//! generic over the identity type rather than tied to an element.
//!
//! ```
//! use aimer_focus::{FocusCandidate, FocusManager, FocusNode};
//!
//! let mut manager = FocusManager::<u32>::new();
//! let username = FocusNode::new();
//! let password = FocusNode::new();
//! let candidates = [
//!     FocusCandidate::new(1, username.clone(), false),
//!     FocusCandidate::new(2, password.clone(), false),
//! ];
//!
//! username.request_focus();
//! let target = manager.resolve(&candidates);
//! manager.transition(target);
//! assert!(username.has_focus());
//!
//! // Tab moves on, in candidate order.
//! let next = manager.traverse(&candidates, false);
//! manager.transition(next);
//! assert!(password.has_focus());
//! assert!(!username.has_focus());
//! ```
#![deny(missing_docs)]

mod focusable;
mod manager;
mod node;
mod scope;
mod traversal;

pub use crate::focusable::{FocusBehavior, FocusCallback, FocusGate};
pub use crate::manager::{FocusManager, FocusOwner, FocusSync, FocusTransition};
pub use crate::node::{FocusNode, FocusRequest, focus_request_generation, request_focus_resolution};
pub use crate::scope::{
    FocusTrap, FocusTrapId, WindowFocusTraps, active_focus_trap, focus_trap_generation,
};
pub use crate::traversal::{FocusCandidate, FocusCandidates, next_focus_index};
