#[cfg(feature = "time-cost")]
use std::cell::LazyCell;
#[cfg(feature = "time-cost")]
use std::sync::LazyLock;

use crate::debug;

#[cfg(feature = "time-cost")]
struct ExecGrouping {
    map: std::collections::HashMap<String, Vec<i64>>,
}
#[cfg(feature = "time-cost")]
static mut EXEC_GROUPING: LazyCell<ExecGrouping> = LazyCell::new(|| ExecGrouping {
    map: Default::default(),
});

#[cfg(feature = "time-cost")]
#[macro_export]
macro_rules! time_cost {
    ($label:expr, $f:block) => {{
        let func = || $f;
        $crate::ExecTimes::no_param($label, func)
    }};

    ($label:expr, $f:expr) => {{ $crate::ExecTimes::no_param($label, $f) }};
}

#[cfg(not(feature = "time-cost"))]
#[macro_export]
macro_rules! time_cost {
    ($label:expr, $f:block) => {{ $f }};

    ($label:expr, $f:expr) => {{ $crate::ExecTimes::no_param($label, $f) }};
}

#[cfg(feature = "time-cost")]
const MINIMUM_EXEC_TIME: Option<&str> = option_env!("MINIMUM_EXEC_TIME");
#[cfg(feature = "time-cost")]
static MINIMUM_EXEC_TIME_MS: LazyLock<i64> = LazyLock::new(|| {
    MINIMUM_EXEC_TIME
        .unwrap_or("0")
        .parse::<i64>()
        .unwrap_or(0)
        .max(0)
});

#[cfg(feature = "time-cost")]
fn add_grouping(key: &str, val: i64) {
    let key = key.trim().replace("|-", "");
    let group = unsafe { &raw mut EXEC_GROUPING.map };
    let group = unsafe { &mut *group };
    let times = group.entry(key.to_string()).or_default();
    times.push(val);
}

pub struct ExecTimes;

impl ExecTimes {
    #[cfg(debug_assertions)]
    pub fn cost_grouping() {
        #[cfg(feature = "time-cost")]
        {
            let group = unsafe { &raw mut EXEC_GROUPING.map };
            let group = unsafe { &mut *group };
            for (label, times) in group.iter() {
                let sum = times.iter().sum::<i64>();
                debug!("{:<5}ms -> {}", sum, label);
            }
            group.clear();
        }
    }

    #[cfg(feature = "time-cost")]
    #[inline]
    pub fn no_param<T>(label: &str, f: impl FnOnce() -> T) -> T {
        let start = crate::AnimInstant::now();
        let res = f();
        let delta = start.elapsed().as_millis() as i64;
        if delta < *MINIMUM_EXEC_TIME_MS {
            return res;
        }
        add_grouping(label, delta);
        debug!("{:<5}ms -> {}", delta, label);
        res
    }

    #[cfg(not(feature = "time-cost"))]
    #[inline]
    pub fn no_param<T>(_: &str, f: impl FnOnce() -> T) -> T {
        f()
    }

    pub fn print_time(f: impl FnOnce()) {
        let start = crate::AnimInstant::now();
        f();

        debug!("Used time: {} ms", start.elapsed().as_millis());
    }
}
