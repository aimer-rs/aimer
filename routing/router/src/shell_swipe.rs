//! Swipe back inside a persistent shell.
//!
//! Two routes that share a [`shell_key`](crate::Route::shell_key) are one page
//! to the navigator: the shell frame stays mounted and only its outlet changes,
//! so the outlet, not the navigator, owns the motion. A swipe therefore has to
//! be started in the outlet, and the navigator cannot reach it directly: the
//! outlet's route type is the shell's own, erased from the navigator's point of
//! view, and the outlet only knows the route it is showing.
//!
//! Two small channels bridge that. A [`ShellPeek`] lets the navigator learn which
//! child route the *previous* page would show, by building that page's widget
//! and having the [`RoutedShell`](crate::RoutedShell) report its child instead of
//! mounting. A [`SwipePort`] lets the navigator hand that route, and the
//! swipe's [drive](crate::transition_host::SwipeDrive), to the mounted outlet.

use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::swipe_back::SwipeLink;
use crate::transition_host::SwipeDrive;

/// Where a shell reports its child route instead of mounting, while the
/// navigator is only asking what a page would show.
#[derive(Clone, Default)]
pub(crate) struct ShellPeek(Rc<RefCell<Option<Rc<dyn Any>>>>);

impl ShellPeek {
    pub(crate) fn deposit(&self, child: Rc<dyn Any>) {
        *self.0.borrow_mut() = Some(child);
    }

    pub(crate) fn take(&self) -> Option<Rc<dyn Any>> {
        self.0.borrow_mut().take()
    }
}

/// A shell's outlet, as the navigator sees it.
pub(crate) trait OutletSwipe {
    /// Starts a swipe back to `previous` (a child route of the outlet's own
    /// type) and returns whether the outlet took it. Declines when `previous`
    /// is not of its type.
    fn begin(&self, previous: &dyn Any, drive: Rc<SwipeDrive>) -> bool;
}

/// The one outlet of a navigator's current shell page, attached by the shell
/// each time it builds.
///
/// It also carries the swipe link the outlet's host uses. In a shell the outlet,
/// not the navigator's host around the whole shell, owns the touch: the frame (a
/// tab bar, say) is not part of the page a finger drags away, and a drag that
/// begins on it must not pop anything.
#[derive(Clone)]
pub(crate) struct SwipePort {
    outlet: Rc<RefCell<Option<Weak<dyn OutletSwipe>>>>,
    link: Rc<SwipeLink>,
}

impl SwipePort {
    pub(crate) fn new(link: Rc<SwipeLink>) -> Self {
        Self { outlet: Rc::default(), link }
    }

    pub(crate) fn attach(&self, outlet: &Rc<dyn OutletSwipe>) {
        *self.outlet.borrow_mut() = Some(Rc::downgrade(outlet));
    }

    /// The link the attached outlet's host carries.
    pub(crate) fn link(&self) -> Rc<SwipeLink> {
        self.link.clone()
    }

    /// Hands the swipe to the attached outlet. `false` when there is none.
    pub(crate) fn begin(&self, previous: &dyn Any, drive: Rc<SwipeDrive>) -> bool {
        let outlet = self.outlet.borrow().as_ref().and_then(Weak::upgrade);
        outlet.is_some_and(|outlet| outlet.begin(previous, drive))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder(RefCell<Vec<String>>, bool);
    impl OutletSwipe for Recorder {
        fn begin(&self, previous: &dyn Any, _: Rc<SwipeDrive>) -> bool {
            let Some(name) = previous.downcast_ref::<&'static str>() else { return false };
            self.0.borrow_mut().push((*name).to_owned());
            self.1
        }
    }

    fn drive() -> Rc<SwipeDrive> {
        SwipeDrive::new(|| {})
    }

    #[test]
    fn a_peek_hands_over_what_was_deposited_once() {
        let peek = ShellPeek::default();
        assert!(peek.take().is_none());
        peek.deposit(Rc::new("home"));
        let taken = peek.take().expect("a child was deposited");
        assert_eq!(taken.downcast_ref::<&str>(), Some(&"home"));
        assert!(peek.take().is_none());
    }

    #[test]
    fn the_port_hands_the_swipe_to_the_attached_outlet() {
        let port = SwipePort::new(SwipeLink::detached());
        let outlet = Rc::new(Recorder(RefCell::default(), true));
        let erased: Rc<dyn OutletSwipe> = outlet.clone();
        port.attach(&erased);
        assert!(port.begin(&"home", drive()));
        assert_eq!(*outlet.0.borrow(), ["home"]);
    }

    #[test]
    fn a_port_with_no_outlet_or_a_gone_one_or_a_wrong_type_declines() {
        let port = SwipePort::new(SwipeLink::detached());
        assert!(!port.begin(&"home", drive()), "nothing attached");
        {
            let outlet: Rc<dyn OutletSwipe> = Rc::new(Recorder(RefCell::default(), true));
            port.attach(&outlet);
            assert!(!port.begin(&42u32, drive()), "not the outlet's route type");
        }
        assert!(!port.begin(&"home", drive()), "the outlet was dropped; the port holds it weakly");
    }
}
