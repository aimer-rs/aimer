use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_events::element::{KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
use aimer_rubick::{INLINE_CAPACITY, UiMemory};

use super::*;
use crate::focus::FocusTrap;
use crate::{FocusNode, Key};

mod broadcast;
mod core;
mod dispatch;
mod event_tree;
mod focus;
mod live_children;
mod path_patch;

use core::{IdentityBranch, ReplacementLeaf};
use dispatch::routed_leaf;
use focus::FocusKeyElement;
