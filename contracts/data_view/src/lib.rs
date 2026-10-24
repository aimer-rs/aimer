//! Platform-neutral data-view models for keyed collections, tables, and trees.
//!
//! The crate deliberately stops at model seams. [`WindowSpec`] and
//! [`VisibleRange`] describe the bounded work a renderer should request from an
//! existing `FlexList`/scroll viewport, while the table and tree models keep
//! identity and interaction state independent of a renderer.

#![deny(missing_docs)]

mod collection;
mod table;
mod tree;

pub use collection::{
    CollectionError, CollectionItem, CollectionModel, CollectionSlot, CollectionStatus,
    CollectionWindow, VisibleRange, WindowError, WindowSpec,
};
pub use table::{
    ColumnWidth, ColumnWidthError, DataColumn, DataTable, SelectionMode, SortDirection, SortState,
    TableError, TableRow,
};
pub use tree::{ChildLoadState, TreeError, TreeKey, TreeKeyResult, TreeNodeSpec, TreeView};
