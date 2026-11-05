//! Inherited state carried to a place that is built outside of its scope.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

use super::{BuildContext, CurrentBuildConsumer, InheritedStates};

/// The states a [`BuildContext`] passes down to its descendants, captured to be
/// handed to a subtree that is built somewhere else.
///
/// A context scopes its inherited states, the providers among them, to the
/// subtree being built: they are in place while the ancestors that provide them
/// are on the stack. Content that is shown from a callback, such as a modal, or
/// that lives in a tree of its own is built and visited with none of them. A
/// scope taken where the content is requested, with [`Self::capture`], is
/// applied again, with [`Self::with`] or [`Self::fork`], wherever that content
/// is built or visited, so it finds what its requester would have found.
///
/// The captured values are shared, not copied: a provider handle in a scope is
/// the handle of the provider it was taken from, and reads live data.
#[derive(Clone, Default)]
pub struct InheritedScope {
    states: InheritedStates,
}

impl InheritedScope {
    /// The states in scope at `ctx`.
    ///
    /// Not among them is the widget that is building: it is whoever is building
    /// when the scope is applied that subscribes to what it reads, and not the
    /// widget that happened to be building when the scope was taken.
    pub fn capture(ctx: &BuildContext) -> Self {
        let mut states = ctx.inherited_states.borrow().clone();
        states.remove(&TypeId::of::<CurrentBuildConsumer>());
        Self { states }
    }

    /// Runs `callback` with the captured states in place over `ctx`'s own, and
    /// puts `ctx` back as it was afterwards.
    pub fn with<'a, R>(
        &self,
        ctx: &BuildContext<'a>,
        callback: impl FnOnce(&BuildContext<'a>) -> R,
    ) -> R {
        let mut previous = Vec::with_capacity(self.states.iter().len());
        {
            let mut states = ctx.inherited_states.borrow_mut();
            for (type_id, state) in &self.states {
                previous.push((*type_id, states.insert(*type_id, state.clone())));
            }
        }
        let _restore = Restore {
            states: ctx.inherited_states.clone(),
            previous,
        };
        callback(ctx)
    }

    /// A copy of `ctx` with the captured states in place over its own, for a
    /// child context that outlives this call.
    pub fn fork<'a>(&self, ctx: &BuildContext<'a>) -> BuildContext<'a> {
        let mut states = ctx.inherited_states.borrow().clone();
        for (type_id, state) in &self.states {
            states.insert(*type_id, state.clone());
        }
        let mut forked = ctx.clone();
        forked.inherited_states = Rc::new(RefCell::new(states));
        forked
    }
}

/// Puts back what the scope covered, however the scope ends.
struct Restore {
    states: Rc<RefCell<InheritedStates>>,
    previous: Vec<(TypeId, Option<Rc<dyn Any>>)>,
}

impl Drop for Restore {
    fn drop(&mut self) {
        let mut states = self.states.borrow_mut();
        for (type_id, state) in self.previous.drain(..).rev() {
            match state {
                Some(state) => {
                    states.insert(type_id, state);
                }
                None => {
                    states.remove(&type_id);
                }
            }
        }
    }
}
