//! Opt-in tracing of pointer handling, for finding out why hover or a resize
//! zone is out of step with the cursor.
//!
//! Nothing is printed unless the `AIMER_DEBUG_POINTER` environment variable is
//! set, and the check is made once.

use std::sync::OnceLock;

/// Whether pointer tracing was asked for with `AIMER_DEBUG_POINTER`.
#[inline]
pub fn pointer_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("AIMER_DEBUG_POINTER").is_some())
}

/// Prints one pointer-trace line to standard error when tracing is enabled.
#[macro_export]
macro_rules! pointer_trace {
    ($($arg:tt)*) => {
        if $crate::pointer_trace_enabled() {
            eprintln!("[pointer] {}", format_args!($($arg)*));
        }
    };
}
