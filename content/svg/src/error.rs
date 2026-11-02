#[derive(Debug, thiserror::Error)]
pub enum SvgError {
    #[error("SVG input is empty")]
    EmptyInput,
    #[error("failed to parse SVG: {0}")]
    Parse(String),
    #[error("invalid SVG path: {0}")]
    InvalidPath(String),
    #[error("invalid SVG selector: {0}")]
    InvalidSelector(String),
    #[error("SVG resource limit exceeded for {resource}: {actual} > {limit}")]
    LimitExceeded {
        resource: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("external SVG resource is not allowed: {0}")]
    ExternalResource(String),
    #[error("SVG selector matched {0} paths; exactly one is required")]
    PathSelection(usize),
    #[error("SVG contains a non-finite value")]
    NonFinite,
    #[error("invalid SVG viewBox: {0}")]
    InvalidViewBox(String),
    #[error("invalid SVG preserveAspectRatio: {0}")]
    InvalidPreserveAspectRatio(String),
}
