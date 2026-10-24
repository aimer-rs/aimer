use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, InheritedStates, Widget};
use smallvec::SmallVec;

use crate::transition_host::{NavigationRequest, RouteBoundary};

/// Type-erased builder for the active child route rendered inside an
/// [`Outlet`].
pub type OutletChildBuilder = Rc<dyn Fn(&BuildContext) -> AnyWidget>;

/// Ambient values captured at a route boundary and restored while retained
/// route content is built or traversed.
///
/// `BuildContext` scopes inherited values dynamically. A route element can
/// outlive the call stack that first built it, though, so a later rebuild may
/// otherwise run without providers that were ancestors when the route was
/// mounted. The values are shared `Rc`s, not cloned provider state; this is a
/// context composition boundary rather than another provider store.
#[derive(Clone)]
pub(crate) struct RouteChildContext {
    states: InheritedStates,
}

impl RouteChildContext {
    pub(crate) fn capture_page(ctx: &BuildContext, key: aimer_widget::Key) -> Self {
        let mut context = Self::capture(ctx);
        context.states.insert(TypeId::of::<RouteBoundary>(), Rc::new(RouteBoundary(key)));
        context
    }

    pub(crate) fn capture(ctx: &BuildContext) -> Self {
        Self {
            states: ctx.inherited_states.borrow().clone(),
        }
    }

    pub(crate) fn with<'a, R>(
        &self,
        ctx: &BuildContext<'a>,
        callback: impl FnOnce(&BuildContext<'a>) -> R,
    ) -> R {
        self.apply(ctx, false, callback)
    }

    pub(crate) fn with_page<'a, R>(&self, ctx: &BuildContext<'a>, callback: impl FnOnce(&BuildContext<'a>) -> R) -> R {
        self.apply(ctx, true, callback)
    }

    pub(crate) fn with_shared_state<'a, T: Any, R>(&self, ctx: &BuildContext<'a>, state: Rc<T>, callback: impl FnOnce(&BuildContext<'a>) -> R) -> R {
        self.with(ctx, |ctx| {
            let type_id = TypeId::of::<T>();
            let old = ctx.inherited_states.borrow_mut().insert(type_id, state);
            let _guard = RouteChildContextGuard {
                states: ctx.inherited_states.clone(), previous: smallvec::smallvec![(type_id, old)],
            };
            callback(ctx)
        })
    }

    fn apply<'a, R>(&self, ctx: &BuildContext<'a>, page: bool, callback: impl FnOnce(&BuildContext<'a>) -> R) -> R {
        // Most scopes restore only the page metadata and one or two providers.
        // Keep that hot-path bookkeeping on the stack; larger scopes can spill.
        let mut previous = SmallVec::new();
        {
            let mut states = ctx.inherited_states.borrow_mut();
            for (type_id, state) in &self.states {
                let metadata = page && (*type_id == TypeId::of::<RouteBoundary>() || *type_id == TypeId::of::<NavigationRequest>());
                if states.get(type_id).is_none_or(|current| metadata && !Rc::ptr_eq(current, state)) {
                    let old = states.insert(*type_id, state.clone());
                    previous.push((*type_id, old));
                }
            }
        }

        let _guard = RouteChildContextGuard {
            states: ctx.inherited_states.clone(),
            previous,
        };
        callback(ctx)
    }
}

type SavedStates = SmallVec<[(TypeId, Option<Rc<dyn Any>>); 8]>;

struct RouteChildContextGuard {
    states: Rc<RefCell<InheritedStates>>,
    previous: SavedStates,
}

impl Drop for RouteChildContextGuard {
    fn drop(&mut self) {
        let mut states = self.states.borrow_mut();
        for (type_id, old) in self.previous.drain(..).rev() {
            if let Some(old) = old { states.insert(type_id, old); }
            else { states.remove(&type_id); }
        }
    }
}

/// State injected by a [`crate::shell::Shell`] into the [`BuildContext`] so a
/// descendant [`Outlet`] knows which child to render. Cheaply cloneable (holds
/// an `Rc` closure).
#[derive(Clone)]
pub struct OutletSlot {
    build: OutletChildBuilder,
}

impl OutletSlot {
    /// Creates a slot from the type-erased active-child builder injected by a
    /// shell.
    pub fn new(build: OutletChildBuilder) -> Self {
        Self { build }
    }

    /// Build the active child widget for the current context.
    pub fn build_child(&self, ctx: &BuildContext) -> AnyWidget {
        (self.build)(ctx)
    }
}

/// A zero-configuration placeholder widget marking where a shell's active child
/// route is rendered.
///
/// Place an `Outlet` anywhere inside a [`crate::shell::Shell`]'s frame; the
/// shell injects an [`OutletSlot`] and the outlet builds the active child from
/// it. An `Outlet` used without an ancestor shell panics — that is a
/// programming error, mirroring `NavigatorController::of`.
pub struct Outlet;

impl aimer_widget::PortableWidget for Outlet {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let build_context = ctx.build_context();
        let Some(slot) = build_context.get_state::<OutletSlot>() else {
            return Err(ctx.unsupported_widget("Outlet", source));
        };
        let child = slot.build_child(&build_context);
        child.into_portable_node(ctx, source)
    }
}

impl Widget for Outlet {
    /// # Panics
    ///
    /// Panics when no [`crate::shell::Shell`] is in scope. The panic is raised
    /// in the body of this `#[track_caller]` method rather than inside a
    /// closure: a closure is an untracked frame, so panicking there blames this
    /// file instead of the code that placed the outlet.
    #[track_caller]
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let Some(slot) = ctx.get_state::<OutletSlot>() else {
            panic!("No Shell found in context. An `Outlet` must be rendered inside a `Shell`.")
        };
        aimer_widget::StatelessElement::from_builder(ctx,
            move |ctx| slot.build_child(ctx).to_element(ctx), None, "Outlet").boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Outlet"
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use aimer_utils::PanicSite;
    use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};

    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    fn context() -> BuildContext<'static> {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let _guard = runtime.enter();
        BuildContext::new(
            canvas,
            ResolvedSize::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn missing_shell_diagnostic_highlights_the_outlet_caller() {
        let watch = PanicSite::watch();
        let rendered = catch_unwind(|| {
            let _ = Outlet.to_element(&context());
        })
        .err()
        .and_then(|_| watch.take_site())
        .expect("the panic site should be recorded")
        .to_string();

        assert!(rendered.starts_with("at "), "{rendered}");
        assert!(rendered.contains(file!()), "{rendered}");
        assert!(
            rendered.contains("Outlet.to_element(&context())"),
            "{rendered}"
        );
        assert!(
            rendered
                .lines()
                .any(|line| line.trim_start().starts_with("^^^^")),
            "{rendered}"
        );
    }
}
