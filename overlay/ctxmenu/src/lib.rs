//! Context menus, in the shape the pointer that asked for one expects.
//!
//! A context menu is the same idea on every platform — a short list of verbs
//! about the thing under the pointer — drawn two very different ways. A finger
//! gets the **pill**: a squat horizontal bar of two or three verbs, floating
//! clear of what it acts on, because a finger has no cursor to aim with and
//! covers whatever it touches. A mouse gets the **list**: a narrow column
//! opening at the click, because that is what a right-click has meant on every
//! desktop for decades. The rule is the *input device's*, not the operating
//! system's, so a phone browser and a desktop browser each get the shape their
//! user is actually holding — see [`ContextMenuShape::for_source`].
//!
//! [`ContextMenu`] is an ordinary widget, presented through the same overlay
//! host as [`aimer_modal::Modal`] and placed by [`aimer_modal::Floating`]. That
//! is what keeps it clear of every clipping ancestor a `Scrollable` puts in its
//! way, closes it on an outside press or on `Escape`, and keeps it following an
//! anchor that moves — none of which its owner has to route or repaint.
//!
//! The menu is drawn by Aimer rather than by the platform. A native popup
//! exists only on desktop, runs its own event loop — which ends whatever
//! pointer capture the gesture that opened it relies on — and has no equivalent
//! on iOS, Android or the web. Drawing it in-tree renders identically
//! everywhere, needs no dependency, and stays testable without a window.
//!
//! # Examples
//!
//! A few verbs, in the shape the pointer expects:
//!
//! ```no_run
//! use aimer_attribute::Vec2d;
//! use aimer_ctxmenu::{ContextMenu, ContextMenuItem};
//! use aimer_events::pointer::PointerSource;
//!
//! # fn demo(source: PointerSource, at: Vec2d) {
//! let handle = ContextMenu::for_source(source)
//!     .at(at)
//!     .items(vec![
//!         ContextMenuItem::new("Copy").on_select(|| println!("copied")),
//!         ContextMenuItem::new("Select All").on_select(|| println!("all")),
//!     ])
//!     .show();
//!
//! handle.dismiss();
//! # }
//! ```
//!
//! A menu styled to the application's own house style:
//!
//! ```
//! use aimer_ctxmenu::{ContextMenuShape, ContextMenuStyle};
//! use aimer_style::{BoxDecoration, TextStyle};
//! use aimer_widget::base::Color;
//!
//! let style = ContextMenuStyle::for_shape(ContextMenuShape::List)
//!     .panel(
//!         BoxDecoration::new()
//!             .background_color(Color::Rgba(24, 24, 27, 250))
//!             .border_radius(12.0),
//!     )
//!     .label(TextStyle::new().font_size(14).color(Color::Rgba(240, 240, 240, 255)))
//!     .row_height(32.0);
//!
//! assert_eq!(style.row_height, 32.0);
//! ```
//!
//! And a menu holding whatever its author likes, instead of rows:
//!
//! ```no_run
//! use aimer_attribute::Vec2d;
//! use aimer_container::SizedBox;
//! use aimer_ctxmenu::ContextMenu;
//!
//! let _handle = ContextMenu::new()
//!     .at(Vec2d { x: 40.0, y: 60.0 })
//!     .child(SizedBox::new().width(240).height(160))
//!     .show();
//! ```

mod dismiss;
mod item;
mod menu;
mod panel;
mod portable;
mod rows;
mod shape;
mod style;

// Re-exported so a consumer can hold a menu, or anchor one, without depending
// on `aimer_modal` itself.
pub use aimer_modal::{AnchorHandle, ModalHandle};
pub use dismiss::ContextMenuDismiss;
pub use item::ContextMenuItem;
pub use menu::ContextMenu;
pub use rows::ContextMenuRows;
pub use shape::ContextMenuShape;
pub use style::{
    ContextMenuStyle, DISABLED_LABEL_COLOR, HIGHLIGHT_COLOR, LABEL_COLOR, LIST_FONT_SIZE,
    LIST_ITEM_PADDING, LIST_MIN_WIDTH, LIST_RADIUS, LIST_ROW_HEIGHT, LIST_VERTICAL_PADDING,
    PANEL_COLOR, PILL_FONT_SIZE, PILL_GAP, PILL_HEIGHT, PILL_ITEM_PADDING, SEPARATOR_COLOR,
};
