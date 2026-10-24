//! Jaime's deterministic collection, table, and tree data-view example.
//!
//! W17 adds the page to the central showcase and exposes the model through the
//! umbrella crate.

use aimer::data_view::{
    CollectionItem, CollectionModel, CollectionSlot, DataColumn, DataTable, SortDirection,
    SortState, TableRow, TreeKey, TreeNodeSpec, TreeView, WindowSpec,
};
use aimer::style::{LayoutSpacing, Spacing};
use aimer::{AimerApp, AnyElement, BuildContext, Column, Container, Text, Widget};

struct ExampleRow {
    id: u64,
    name: &'static str,
    group: &'static str,
    score: i32,
}

/// A composed page demonstrating keyed collections, a cached data table, and a
/// lazy tree without requiring a platform-specific collection adapter.
pub struct DataViewExample {
    collection: CollectionModel<u64, String, &'static str>,
    table: DataTable<u64, ExampleRow>,
    tree: TreeView<u64, String>,
}

impl DataViewExample {
    /// Creates the deterministic data-view showcase state.
    pub fn new() -> Self {
        let mut collection = CollectionModel::new();
        collection
            .set_items([
                CollectionItem::new(101, "Alpha".to_owned()),
                CollectionItem::new(102, "Beta".to_owned()),
                CollectionItem::new(103, "Gamma".to_owned()),
                CollectionItem::new(104, "Delta".to_owned()),
            ])
            .expect("showcase collection keys are unique");
        collection
            .set_state(102, "selected")
            .expect("showcase state key is present");

        let name = DataColumn::new("name", "Name").cell(|row: &ExampleRow| row.name.to_owned());
        let group = DataColumn::new("group", "Group").cell(|row: &ExampleRow| row.group.to_owned());
        let score = DataColumn::new("score", "Score")
            .cell(|row: &ExampleRow| {
                row.score
                    .to_string()
            })
            .sortable_by(|left: &ExampleRow, right: &ExampleRow| {
                left.score
                    .cmp(&right.score)
            });
        let mut table: DataTable<u64, ExampleRow> =
            DataTable::new([name, group, score]).expect("showcase column keys are unique");
        table
            .set_rows([
                TableRow::new(
                    201,
                    ExampleRow {
                        id: 201,
                        name: "Ada",
                        group: "Core",
                        score: 92,
                    },
                ),
                TableRow::new(
                    202,
                    ExampleRow {
                        id: 202,
                        name: "Bea",
                        group: "UI",
                        score: 84,
                    },
                ),
                TableRow::new(
                    203,
                    ExampleRow {
                        id: 203,
                        name: "Cid",
                        group: "Core",
                        score: 97,
                    },
                ),
            ])
            .expect("showcase row keys are unique");
        table
            .set_sort(Some(SortState::new("score", SortDirection::Descending)))
            .expect("score is sortable");
        table
            .refresh()
            .expect("showcase table refresh is valid");

        let mut tree = TreeView::new();
        tree.insert_root(301, "Projects".to_owned())
            .expect("showcase root key is unique");
        tree.insert_child(301, 302, "Aimer".to_owned())
            .expect("showcase child key is unique");
        tree.insert_child(301, 303, "Jaime".to_owned())
            .expect("showcase child key is unique");
        tree.mark_lazy(302)
            .expect("empty project is lazy");
        tree.set_expanded(301, true)
            .expect("showcase root exists");
        tree.focus(302)
            .expect("showcase lazy node exists");
        let request = tree
            .handle_key(TreeKey::ArrowRight)
            .expect("showcase lazy request is valid");
        assert!(matches!(
            request,
            aimer::data_view::TreeKeyResult::RequestChildren(302)
        ));
        tree.complete_children(302, [TreeNodeSpec::new(304, "Collection layer".to_owned())])
            .expect("showcase lazy response is valid");
        tree.set_expanded(302, true)
            .expect("showcase lazy node exists");

        Self {
            collection,
            table,
            tree,
        }
    }
}

impl Default for DataViewExample {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for DataViewExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let window = WindowSpec::new(24.0, 24.0, 48.0)
            .expect("showcase window geometry is finite")
            .with_overscan(1)
            .with_max_items(8)
            .expect("showcase window cap is positive");
        let collection_summary = match self
            .collection
            .slot(&window)
            .expect("showcase window geometry is valid")
        {
            CollectionSlot::Empty => "empty collection".to_owned(),
            CollectionSlot::Loading => "loading collection".to_owned(),
            CollectionSlot::Error(message) => format!("collection error: {message}"),
            CollectionSlot::Items(window) => {
                format!(
                    "collection window {:?} ({}/{} rows)",
                    window.range(),
                    window.len(),
                    self.collection
                        .len()
                )
            }
        };
        let table_keys = self
            .table
            .visible_keys_in_window(&window)
            .expect("showcase table window geometry is valid");
        let tree_ids = self
            .tree
            .visible_ids();
        let table_summary = format!(
            "table: {} rows, sorted by score, window keys {:?}",
            self.table
                .row_count(),
            table_keys
        );
        let tree_summary = format!("tree visible IDs: {:?}", tree_ids);

        Container::new()
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new().children([
                    Text::new("Collections and data views").boxed(),
                    Text::new(collection_summary)
                        .wrapped()
                        .boxed(),
                    Text::new(table_summary)
                        .wrapped()
                        .boxed(),
                    Text::new(format!(
                        "selected table keys: {:?}",
                        self.table
                            .selected_keys()
                    ))
                    .wrapped()
                    .boxed(),
                    Text::new(tree_summary)
                        .wrapped()
                        .boxed(),
                ]),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "DataViewExample"
    }
}

impl aimer::PortableWidget for DataViewExample {}

/// Builds the data-view example without starting an application.
pub fn data_view_example() -> impl Widget {
    DataViewExample::new()
}

/// Starts the data-view example as a standalone Jaime application.
pub fn start_data_view_example() {
    AimerApp::start(data_view_example());
}
