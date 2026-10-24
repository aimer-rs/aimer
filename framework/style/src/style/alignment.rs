#[derive(Default, Clone, Copy)]
pub enum RowAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Default, Clone, Copy)]
pub enum ColumnAlignment {
    #[default]
    Top,
    Middle,
    Bottom,
}
