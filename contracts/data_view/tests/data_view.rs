use aimer_data_view::{
    ChildLoadState, CollectionItem, CollectionSlot, ColumnWidth, DataColumn, DataTable,
    SelectionMode, SortDirection, SortState, TableRow, TreeKey, TreeKeyResult, TreeNodeSpec,
    TreeView, WindowSpec,
};

fn item(id: u64, value: &'static str) -> CollectionItem<u64, &'static str> {
    CollectionItem::new(id, value)
}

#[test]
fn keyed_collection_retains_state_across_reorder_and_drops_removed_keys() {
    let mut collection = aimer_data_view::CollectionModel::<u64, &'static str, &'static str>::new();
    collection
        .set_items([item(1, "one"), item(2, "two"), item(3, "three")])
        .unwrap();
    collection.set_state(2, "focused").unwrap();
    collection.set_state(3, "expanded").unwrap();

    collection
        .replace_items([item(3, "three"), item(4, "four"), item(2, "two")])
        .unwrap();

    assert_eq!(
        collection.keys().copied().collect::<Vec<_>>(),
        vec![3, 4, 2]
    );
    assert_eq!(collection.state(&2), Some(&"focused"));
    assert_eq!(collection.state(&3), Some(&"expanded"));
    assert_eq!(collection.state(&1), None);
    assert!(collection.set_state(1, "stale").is_err());
}

#[test]
fn collection_rejects_duplicate_keys_without_partial_update() {
    let mut collection = aimer_data_view::CollectionModel::<u64, &'static str>::new();
    collection
        .set_items([item(1, "one"), item(2, "two")])
        .unwrap();

    assert!(
        collection
            .replace_items([item(2, "changed"), item(2, "duplicate")])
            .is_err()
    );
    assert_eq!(collection.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(collection.item(&2).unwrap().value(), &"two");
}

#[test]
fn collection_slots_cover_empty_loading_error_and_bounded_windows() {
    let spec = WindowSpec::new(10.0, 20.0, 20.0)
        .unwrap()
        .with_overscan(1)
        .with_max_items(4)
        .unwrap();
    let mut collection = aimer_data_view::CollectionModel::<u64, u64>::new();

    assert!(matches!(
        collection.slot(&spec).unwrap(),
        CollectionSlot::Empty
    ));
    collection.begin_loading();
    assert!(matches!(
        collection.slot(&spec).unwrap(),
        CollectionSlot::Loading
    ));
    collection.fail("network unavailable");
    match collection.slot(&spec).unwrap() {
        CollectionSlot::Error(message) => assert_eq!(message, "network unavailable"),
        other => panic!("unexpected collection slot: {other:?}"),
    }

    collection
        .set_items((0..10_000).map(|id| CollectionItem::new(id, id)))
        .unwrap();
    let window = match collection.slot(&spec).unwrap() {
        CollectionSlot::Items(window) => window,
        other => panic!("unexpected collection slot: {other:?}"),
    };
    assert_eq!(window.range().start(), 1);
    assert_eq!(window.range().end(), 5);
    assert_eq!(window.len(), 4);
    assert_eq!(window.items()[0].key(), &1);
}

#[test]
fn zero_extent_and_empty_windows_have_deterministic_boundaries() {
    assert!(aimer_data_view::WindowSpec::new(0.0, 0.0, 10.0).is_err());
    let spec = WindowSpec::new(10.0, 100.0, 0.0).unwrap();
    let mut collection = aimer_data_view::CollectionModel::<u64, u64>::new();
    collection
        .set_items((0..3).map(|id| CollectionItem::new(id, id)))
        .unwrap();
    let window = collection.window(&spec).unwrap();
    assert!(window.is_empty());
    assert_eq!(window.range().start(), 3);
    assert_eq!(window.range().end(), 3);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Person {
    id: u64,
    name: &'static str,
    score: i32,
}

fn table() -> DataTable<u64, Person> {
    let name = DataColumn::new("name", "Name").cell(|row: &Person| row.name.to_owned());
    let score = DataColumn::new("score", "Score")
        .cell(|row: &Person| row.score.to_string())
        .sortable_by(|left: &Person, right: &Person| left.score.cmp(&right.score));
    let mut table = DataTable::new([name, score]).unwrap();
    table
        .set_rows([
            TableRow::new(
                1,
                Person {
                    id: 1,
                    name: "Ada",
                    score: 80,
                },
            ),
            TableRow::new(
                2,
                Person {
                    id: 2,
                    name: "Bea",
                    score: 95,
                },
            ),
            TableRow::new(
                3,
                Person {
                    id: 3,
                    name: "Cid",
                    score: 65,
                },
            ),
        ])
        .unwrap();
    table.refresh().unwrap();
    table
}

#[test]
fn data_table_sorts_filters_selects_and_windows_rows_by_key() {
    let mut table = table();
    table
        .set_sort(Some(SortState::new("score", SortDirection::Descending)))
        .unwrap();
    assert!(table.is_dirty());
    table.refresh().unwrap();
    assert_eq!(table.visible_keys(), vec![2, 1, 3]);

    table.set_filter("a");
    table.refresh().unwrap();
    assert_eq!(table.visible_keys(), vec![2, 1]);

    table.set_selection_mode(SelectionMode::Multiple);
    table.select(2, true).unwrap();
    table.select(1, true).unwrap();
    assert!(table.is_selected(&2));
    assert!(table.is_selected(&1));

    table
        .set_rows([
            TableRow::new(
                1,
                Person {
                    id: 1,
                    name: "Ada",
                    score: 80,
                },
            ),
            TableRow::new(
                4,
                Person {
                    id: 4,
                    name: "Drew",
                    score: 70,
                },
            ),
            TableRow::new(
                2,
                Person {
                    id: 2,
                    name: "Bea",
                    score: 95,
                },
            ),
        ])
        .unwrap();
    table.refresh().unwrap();
    assert!(table.is_selected(&2));
    assert!(table.is_selected(&1));
    assert!(!table.is_selected(&3));

    let window = WindowSpec::new(20.0, 20.0, 20.0)
        .unwrap()
        .with_max_items(2)
        .unwrap();
    assert!(table.visible_keys_in_window(&window).unwrap().len() <= 2);
}

#[test]
fn data_table_rejects_duplicate_rows_and_validates_column_widths() {
    let mut table = table();
    assert!(
        table
            .set_rows([
                TableRow::new(
                    1,
                    Person {
                        id: 1,
                        name: "Ada",
                        score: 80
                    }
                ),
                TableRow::new(
                    1,
                    Person {
                        id: 1,
                        name: "Again",
                        score: 81
                    }
                ),
            ])
            .is_err()
    );
    assert_eq!(table.row_count(), 3);

    let width = ColumnWidth::fixed(120.0).unwrap();
    table.set_column_width("name", width).unwrap();
    assert_eq!(table.column("name").unwrap().width(), width);
    assert!(ColumnWidth::fixed(0.0).is_err());
    assert!(ColumnWidth::fixed(f32::NAN).is_err());
}

#[test]
fn tree_view_expands_traverses_loads_children_and_rejects_cycles() {
    let mut tree = TreeView::<u64, String>::new();
    tree.insert_root(1, "Root".into()).unwrap();
    tree.insert_child(1, 2, "Lazy branch".into()).unwrap();
    tree.insert_child(1, 3, "Sibling".into()).unwrap();
    tree.mark_lazy(2).unwrap();

    tree.set_expanded(1, true).unwrap();
    tree.focus(1).unwrap();
    assert_eq!(
        tree.handle_key(TreeKey::ArrowDown).unwrap(),
        TreeKeyResult::Focused(2)
    );
    assert_eq!(
        tree.handle_key(TreeKey::ArrowRight).unwrap(),
        TreeKeyResult::RequestChildren(2)
    );
    assert_eq!(tree.children_state(&2), Some(ChildLoadState::Loading));

    tree.complete_children(2, [TreeNodeSpec::new(4, "Loaded child".into())])
        .unwrap();
    assert_eq!(tree.children_state(&2), Some(ChildLoadState::Loaded));
    assert_eq!(tree.visible_ids(), vec![1, 2, 4, 3]);

    assert!(matches!(
        tree.attach_child(2, 1),
        Err(aimer_data_view::TreeError::Cycle { .. })
    ));
}

#[test]
fn tree_keyboard_left_home_end_and_activation_are_deterministic() {
    let mut tree = TreeView::<u64, String>::new();
    tree.insert_root(1, "One".into()).unwrap();
    tree.insert_root(2, "Two".into()).unwrap();
    tree.insert_child(2, 3, "Three".into()).unwrap();
    tree.set_expanded(2, true).unwrap();
    tree.focus(3).unwrap();

    assert_eq!(
        tree.handle_key(TreeKey::ArrowLeft).unwrap(),
        TreeKeyResult::Focused(2)
    );
    assert_eq!(
        tree.handle_key(TreeKey::Home).unwrap(),
        TreeKeyResult::Focused(1)
    );
    assert_eq!(
        tree.handle_key(TreeKey::End).unwrap(),
        TreeKeyResult::Focused(3)
    );
    assert_eq!(
        tree.handle_key(TreeKey::Enter).unwrap(),
        TreeKeyResult::Activated(3)
    );
}

#[test]
fn sort_comparators_have_a_stable_original_order_for_equal_values() {
    let mut table = DataTable::new([DataColumn::new("value", "Value")
        .cell(|row: &i32| row.to_string())
        .sortable_by(|left: &i32, right: &i32| left.cmp(right))])
    .unwrap();
    table
        .set_rows([
            TableRow::new(10, 2),
            TableRow::new(20, 1),
            TableRow::new(30, 2),
        ])
        .unwrap();
    table
        .set_sort(Some(SortState::new("value", SortDirection::Ascending)))
        .unwrap();
    table.refresh().unwrap();
    assert_eq!(table.visible_keys(), vec![20, 10, 30]);
}
