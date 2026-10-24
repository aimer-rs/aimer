//! Data-table columns, cached sorting/filtering, and keyed selection.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use crate::{WindowError, WindowSpec};

type CellRenderer<R> = Rc<dyn Fn(&R) -> String>;
type RowComparator<R> = Rc<dyn Fn(&R, &R) -> Ordering>;

/// A table column's width policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    /// Size the column to the table's content/layout policy.
    Auto,
    /// Reserve an exact positive logical width.
    Fixed(f32),
    /// Allocate a positive relative share of remaining width.
    Flex(f32),
}

/// A requested column width is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnWidthError {
    /// The width is NaN, infinite, or not strictly positive.
    Invalid,
}

impl ColumnWidth {
    /// Creates a positive fixed width.
    pub fn fixed(width: f32) -> Result<Self, ColumnWidthError> {
        if width.is_finite() && width > 0.0 {
            Ok(Self::Fixed(width))
        } else {
            Err(ColumnWidthError::Invalid)
        }
    }

    /// Creates a positive relative flex width.
    pub fn flex(factor: f32) -> Result<Self, ColumnWidthError> {
        if factor.is_finite() && factor > 0.0 {
            Ok(Self::Flex(factor))
        } else {
            Err(ColumnWidthError::Invalid)
        }
    }
}

/// A column definition and its public cell/sort projections.
#[derive(Clone)]
pub struct DataColumn<R> {
    id: String,
    title: String,
    width: ColumnWidth,
    cell: CellRenderer<R>,
    comparator: Option<RowComparator<R>>,
}

impl<R> DataColumn<R> {
    /// Creates a column with an automatic width and an empty default cell.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            width: ColumnWidth::Auto,
            cell: Rc::new(|_| String::new()),
            comparator: None,
        }
    }

    /// Replaces the projection used for rendering and text filtering.
    pub fn cell(mut self, renderer: impl Fn(&R) -> String + 'static) -> Self {
        self.cell = Rc::new(renderer);
        self
    }

    /// Adds the comparator used when this column is sorted.
    pub fn sortable_by(mut self, comparator: impl Fn(&R, &R) -> Ordering + 'static) -> Self {
        self.comparator = Some(Rc::new(comparator));
        self
    }

    /// Sets the width policy for this column.
    pub fn with_width(mut self, width: ColumnWidth) -> Self {
        self.width = width;
        self
    }

    /// Returns the stable column identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the human-readable column title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the current width policy.
    pub fn width(&self) -> ColumnWidth {
        self.width
    }

    /// Returns whether this column has a sort comparator.
    pub fn is_sortable(&self) -> bool {
        self.comparator.is_some()
    }

    #[inline]
    pub(crate) fn render(&self, row: &R) -> String {
        (self.cell)(row)
    }

    #[inline]
    pub(crate) fn compare(&self, left: &R, right: &R) -> Option<Ordering> {
        self.comparator.as_ref().map(|compare| compare(left, right))
    }
}

/// A row paired with the stable identity used for selection and retention.
#[derive(Debug, PartialEq, Eq)]
pub struct TableRow<K, R> {
    key: K,
    value: R,
}

impl<K, R> TableRow<K, R> {
    /// Creates a table row with a stable key.
    pub fn new(key: K, value: R) -> Self {
        Self { key, value }
    }

    /// Returns the stable row key.
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns the row value.
    pub fn value(&self) -> &R {
        &self.value
    }

    /// Splits this row into its key and value.
    pub fn into_parts(self) -> (K, R) {
        (self.key, self.value)
    }
}

/// The direction applied to a table sort comparator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    /// Lowest comparator values first.
    Ascending,
    /// Highest comparator values first.
    Descending,
}

/// A selected sortable column and direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortState {
    column: String,
    direction: SortDirection,
}

impl SortState {
    /// Creates a sort request for a column identifier.
    pub fn new(column: impl Into<String>, direction: SortDirection) -> Self {
        Self {
            column: column.into(),
            direction,
        }
    }

    /// Returns the sorted column identifier.
    pub fn column(&self) -> &str {
        &self.column
    }

    /// Returns the sort direction.
    pub fn direction(&self) -> SortDirection {
        self.direction
    }
}

/// Controls whether a table allows one or many selected row keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    /// Selecting one row clears any previous selection.
    Single,
    /// Selecting rows accumulates keys.
    Multiple,
}

/// A table operation referred to an invalid or unsupported item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError<K> {
    /// Two columns have the same or empty identifier.
    InvalidColumnId(String),
    /// Two rows in one replacement have the same key.
    DuplicateRowKey(K),
    /// A requested column does not exist.
    UnknownColumn(String),
    /// A requested column exists but has no comparator.
    UnsortableColumn(String),
    /// A selection operation referred to a missing row key.
    UnknownRowKey(K),
}

/// A data table whose expensive view transformation is explicit and cached.
///
/// `set_rows`, [`DataTable::set_filter`], and [`DataTable::set_sort`] only
/// invalidate the cached row order. Call [`DataTable::refresh`] at the model
/// update boundary; reading a visible window then touches only the bounded
/// slice of the cached order and does not rescan the full data set.
pub struct DataTable<K, R> {
    columns: Vec<DataColumn<R>>,
    rows: Vec<TableRow<K, R>>,
    view: Vec<usize>,
    filter: String,
    sort: Option<SortState>,
    selection: HashSet<K>,
    selection_mode: SelectionMode,
    dirty: bool,
}

impl<K, R> DataTable<K, R>
where
    K: Eq + Hash + Clone,
{
    /// Creates a table and validates that every column has a unique ID.
    pub fn new(columns: impl IntoIterator<Item = DataColumn<R>>) -> Result<Self, TableError<K>> {
        let columns: Vec<_> = columns.into_iter().collect();
        let mut ids = HashSet::with_capacity(columns.len());
        for column in &columns {
            if column.id.is_empty() || !ids.insert(&column.id) {
                return Err(TableError::InvalidColumnId(column.id.clone()));
            }
        }
        Ok(Self {
            columns,
            rows: Vec::new(),
            view: Vec::new(),
            filter: String::new(),
            sort: None,
            selection: HashSet::new(),
            selection_mode: SelectionMode::Single,
            dirty: false,
        })
    }

    /// Replaces rows atomically and preserves selection for keys that remain.
    pub fn set_rows(
        &mut self,
        rows: impl IntoIterator<Item = TableRow<K, R>>,
    ) -> Result<(), TableError<K>> {
        let rows: Vec<_> = rows.into_iter().collect();
        let mut keys = HashSet::with_capacity(rows.len());
        for row in &rows {
            if !keys.insert(&row.key) {
                return Err(TableError::DuplicateRowKey(row.key.clone()));
            }
        }

        self.selection.retain(|key| keys.contains(key));
        self.rows = rows;
        self.dirty = true;
        Ok(())
    }

    /// Returns the number of logical rows, before filtering.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Returns the number of configured columns.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Returns all source rows in source order.
    pub fn rows(&self) -> &[TableRow<K, R>] {
        &self.rows
    }

    /// Returns a column by stable ID.
    pub fn column(&self, id: &str) -> Option<&DataColumn<R>> {
        self.columns.iter().find(|column| column.id == id)
    }

    /// Returns all columns in declaration order.
    pub fn columns(&self) -> &[DataColumn<R>] {
        &self.columns
    }

    /// Changes one column's width policy.
    pub fn set_column_width(&mut self, id: &str, width: ColumnWidth) -> Result<(), TableError<K>> {
        let column = self
            .columns
            .iter_mut()
            .find(|column| column.id == id)
            .ok_or_else(|| TableError::UnknownColumn(id.to_owned()))?;
        column.width = width;
        Ok(())
    }

    /// Sets the case-insensitive text filter and invalidates the cached view.
    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.dirty = true;
    }

    /// Returns the current filter query.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Sets or clears sorting after validating the requested column.
    pub fn set_sort(&mut self, sort: Option<SortState>) -> Result<(), TableError<K>> {
        if let Some(sort) = &sort {
            let column = self
                .column(sort.column())
                .ok_or_else(|| TableError::UnknownColumn(sort.column().to_owned()))?;
            if !column.is_sortable() {
                return Err(TableError::UnsortableColumn(sort.column().to_owned()));
            }
        }
        self.sort = sort;
        self.dirty = true;
        Ok(())
    }

    /// Returns the current sort request, if any.
    pub fn sort(&self) -> Option<&SortState> {
        self.sort.as_ref()
    }

    /// Returns whether a data/filter/sort mutation needs a refresh.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Rebuilds the filtered and sorted row-index cache.
    pub fn refresh(&mut self) -> Result<(), TableError<K>> {
        let query = self.filter.to_ascii_lowercase();
        let mut view = Vec::with_capacity(self.rows.len());
        for (index, row) in self.rows.iter().enumerate() {
            let matches = query.is_empty()
                || self.columns.iter().any(|column| {
                    column
                        .render(&row.value)
                        .to_ascii_lowercase()
                        .contains(&query)
                });
            if matches {
                view.push(index);
            }
        }

        if let Some(sort) = &self.sort {
            let column = self
                .column(sort.column())
                .ok_or_else(|| TableError::UnknownColumn(sort.column().to_owned()))?;
            if !column.is_sortable() {
                return Err(TableError::UnsortableColumn(sort.column().to_owned()));
            }
            view.sort_by(|left, right| {
                let ordering = column
                    .compare(&self.rows[*left].value, &self.rows[*right].value)
                    .expect("validated sortable column has a comparator");
                let ordering = match sort.direction() {
                    SortDirection::Ascending => ordering,
                    SortDirection::Descending => ordering.reverse(),
                };
                ordering.then_with(|| left.cmp(right))
            });
        }

        self.view = view;
        self.dirty = false;
        Ok(())
    }

    /// Returns filtered/sorted keys in cached view order.
    ///
    /// Call [`DataTable::refresh`] after changing rows, sorting, or filtering.
    pub fn visible_keys(&self) -> Vec<K> {
        self.view
            .iter()
            .map(|index| self.rows[*index].key.clone())
            .collect()
    }

    /// Returns cached source indices in filtered/sorted view order.
    pub fn visible_row_indices(&self) -> &[usize] {
        &self.view
    }

    /// Returns at most the bounded row keys intersecting a viewport window.
    ///
    /// The window is applied to the cached filtered/sorted order, so this call
    /// does not perform a full-table filter or sort.
    pub fn visible_keys_in_window(&self, spec: &WindowSpec) -> Result<Vec<K>, WindowError> {
        let range = spec.range(self.view.len());
        Ok(self.view[range.as_range()]
            .iter()
            .map(|index| self.rows[*index].key.clone())
            .collect())
    }

    /// Changes the selection policy, reducing an existing multi-selection when
    /// switching to single-selection mode.
    pub fn set_selection_mode(&mut self, mode: SelectionMode) {
        self.selection_mode = mode;
        if mode == SelectionMode::Single {
            let first = self
                .rows
                .iter()
                .find(|row| self.selection.contains(&row.key))
                .map(|row| row.key.clone());
            self.selection.clear();
            if let Some(key) = first {
                self.selection.insert(key);
            }
        }
    }

    /// Returns the current selection policy.
    pub fn selection_mode(&self) -> SelectionMode {
        self.selection_mode
    }

    /// Selects or deselects one row key.
    pub fn select(&mut self, key: K, selected: bool) -> Result<(), TableError<K>> {
        if !self.rows.iter().any(|row| row.key == key) {
            return Err(TableError::UnknownRowKey(key));
        }
        if selected {
            if self.selection_mode == SelectionMode::Single {
                self.selection.clear();
            }
            self.selection.insert(key);
        } else {
            self.selection.remove(&key);
        }
        Ok(())
    }

    /// Toggles one row key and returns its new selected state.
    pub fn toggle_selection(&mut self, key: K) -> Result<bool, TableError<K>> {
        let selected = self.selection.contains(&key);
        self.select(key.clone(), !selected)?;
        Ok(!selected)
    }

    /// Returns whether a row key is selected.
    pub fn is_selected(&self, key: &K) -> bool {
        self.selection.contains(key)
    }

    /// Returns selected keys in current source order.
    pub fn selected_keys(&self) -> Vec<K> {
        self.rows
            .iter()
            .filter(|row| self.selection.contains(&row.key))
            .map(|row| row.key.clone())
            .collect()
    }
}
