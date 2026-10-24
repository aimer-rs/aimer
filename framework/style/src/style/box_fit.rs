#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BoxFit {
    Fill,
    Contain,
    Cover,
    #[default]
    None,
    ScaleDown,
    FitWidth,
    FitHeight,
}
