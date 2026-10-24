pub mod recovery_end;
/// Browser-only gesture analysis. Compiled for the host as well so its logic
/// stays covered by the workspace test run.
#[cfg(any(target_arch = "wasm32", test))]
pub mod web_overscroll;
/// Browser-only gesture termination. Compiled for the host as well so its
/// logic stays covered by the workspace test run.
#[cfg(any(target_arch = "wasm32", test))]
pub mod web_recovery_end;
