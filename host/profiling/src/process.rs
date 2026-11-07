//! Resident memory and CPU time of the whole process.
//!
//! These are the operating system's numbers, not the framework's: the UI heap
//! counter in the overlay says how much the widget tree maps, this says what
//! the process costs. Each reading is `None` where the platform offers no
//! cheap, honest source (the browser has none), and the overlay then says so
//! instead of showing zero.

use std::time::Duration;

use aimer_utils::AnimInstant;

/// One reading of the process's resources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProcessSample {
    /// Physical memory the process currently has resident, in bytes.
    pub(crate) resident_bytes: Option<u64>,
    /// User plus system CPU time spent by every thread since the process
    /// started.
    pub(crate) cpu_time: Option<Duration>,
}

/// Reads the process's resident memory and cumulative CPU time.
///
/// Cheap enough to call a few times a second; it makes one or two system calls.
pub(crate) fn sample() -> ProcessSample {
    ProcessSample {
        resident_bytes: resident_bytes(),
        cpu_time: cpu_time(),
    }
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    /// The calling task's port. `mach_task_self()` in Apple's headers is a macro
    /// that reads this variable; libSystem sets it before `main` runs.
    static mach_task_self_: libc::mach_port_t;
}

/// Resident set size from the Mach task, which is what Activity Monitor's
/// "Real Memory" reports. `phys_footprint` (Xcode's gauge) also counts
/// compressed pages, so the two differ under memory pressure.
#[cfg(target_vendor = "apple")]
fn resident_bytes() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::zeroed();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: `info` is a zeroed `mach_task_basic_info`, the exact structure
    // `MACH_TASK_BASIC_INFO` fills, and `count` is its size in `natural_t`
    // units as the call requires. `mach_task_self_` is initialised by libSystem
    // before any Rust code runs and names this process's own task, which is
    // always valid. The kernel writes at most `count` units into `info`.
    let status = unsafe {
        libc::task_info(
            mach_task_self_,
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS {
        return None;
    }
    // SAFETY: `task_info` succeeded, so it initialised the structure.
    let info = unsafe { info.assume_init() };
    // The structure is `packed(4)`; copy the field out instead of referencing it.
    let resident = { info.resident_size };
    Some(resident)
}

/// Resident pages from `/proc/self/statm` (its second field), times the page
/// size. Read into a stack buffer: the file is a single short line.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn resident_bytes() -> Option<u64> {
    use std::io::Read;

    let mut buffer = [0u8; 128];
    let read = std::fs::File::open("/proc/self/statm")
        .ok()?
        .read(&mut buffer)
        .ok()?;
    let pages: u64 = std::str::from_utf8(&buffer[..read])
        .ok()?
        .split_ascii_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    // SAFETY: `sysconf` has no preconditions; it only reads a system constant.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = u64::try_from(page_size).ok().filter(|size| *size > 0)?;
    pages.checked_mul(page_size)
}

/// The working set, which is the Windows name for resident memory.
#[cfg(windows)]
fn resident_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = std::mem::MaybeUninit::<PROCESS_MEMORY_COUNTERS>::zeroed();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `counters` is a zeroed `PROCESS_MEMORY_COUNTERS` and `size` is its
    // exact byte size, which the API reads as the structure's version. The
    // pseudo-handle from `GetCurrentProcess` is always valid and needs no close.
    let ok = unsafe {
        (*counters.as_mut_ptr()).cb = size;
        K32GetProcessMemoryInfo(GetCurrentProcess(), counters.as_mut_ptr(), size)
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: the call succeeded, so the structure is initialised.
    Some(unsafe { counters.assume_init() }.WorkingSetSize as u64)
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn resident_bytes() -> Option<u64> {
    None
}

/// User plus system time of every thread, from `getrusage`. Unlike the Mach
/// task's own times, this includes threads that are still running.
#[cfg(unix)]
fn cpu_time() -> Option<Duration> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` is a zeroed `rusage`, which is what `RUSAGE_SELF` fills.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the call succeeded, so the structure is initialised.
    let usage = unsafe { usage.assume_init() };
    let user = timeval_duration(usage.ru_utime.tv_sec as i64, usage.ru_utime.tv_usec as i64)?;
    let system = timeval_duration(usage.ru_stime.tv_sec as i64, usage.ru_stime.tv_usec as i64)?;
    user.checked_add(system)
}

#[cfg(unix)]
fn timeval_duration(seconds: i64, microseconds: i64) -> Option<Duration> {
    let seconds = u64::try_from(seconds).ok()?;
    let nanos = u32::try_from(microseconds).ok()?.checked_mul(1_000)?;
    Some(Duration::new(seconds, nanos))
}

/// Kernel plus user time from `GetProcessTimes`, in 100 ns ticks.
#[cfg(windows)]
fn cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: all four out-pointers refer to live `FILETIME`s, and the
    // pseudo-handle from `GetCurrentProcess` is always valid.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return None;
    }
    let ticks = |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    let total = ticks(kernel).checked_add(ticks(user))?;
    Some(Duration::new(
        total / 10_000_000,
        ((total % 10_000_000) * 100) as u32,
    ))
}

#[cfg(not(any(unix, windows)))]
fn cpu_time() -> Option<Duration> {
    None
}

/// Turns successive cumulative CPU readings into a utilisation percentage.
#[derive(Debug, Default)]
pub(crate) struct CpuMeter {
    last: Option<(AnimInstant, Duration)>,
}

impl CpuMeter {
    /// Feeds a reading taken at `now` and returns the share of one core used
    /// since the previous reading, in percent.
    ///
    /// The result can exceed 100 when several threads run at once, as in
    /// Activity Monitor and `top`. It is `None` for the first reading, when the
    /// platform has no CPU time, and when no wall time has passed.
    pub(crate) fn update(&mut self, now: AnimInstant, cpu: Option<Duration>) -> Option<f64> {
        let Some(cpu) = cpu else {
            // No reading means no interval to measure against later either.
            self.last = None;
            return None;
        };
        let Some((then, before)) = self.last else {
            self.last = Some((now, cpu));
            return None;
        };
        let wall = now.duration_since(then);
        if wall.is_zero() {
            // Keep the earlier reading: the interval it opened is still open.
            return None;
        }
        self.last = Some((now, cpu));
        // A counter that moved backwards (a platform reset it) rebases the
        // interval; it is not a negative load.
        let busy = cpu.checked_sub(before)?;
        Some(busy.as_secs_f64() / wall.as_secs_f64() * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(unix, windows))]
    #[test]
    fn resident_memory_of_a_running_test_process_is_plausible() {
        let bytes = sample().resident_bytes.expect("this platform reports RSS");
        assert!(bytes > 1 << 20, "a Rust test binary is more than 1 MiB: {bytes}");
        assert!(bytes < 1 << 40, "and less than 1 TiB: {bytes}");
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn resident_memory_grows_when_pages_are_touched() {
        let before = sample().resident_bytes.unwrap();
        // Touch every page: untouched anonymous memory is not resident.
        let mut block = vec![0u8; 64 << 20];
        for byte in block.iter_mut().step_by(4096) {
            *byte = 1;
        }
        let after = sample().resident_bytes.unwrap();
        assert!(
            after >= before + (32 << 20),
            "RSS should reflect 64 MiB of touched pages: {before} -> {after}"
        );
        std::hint::black_box(&block);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn cpu_time_is_monotonic_and_counts_busy_work() {
        let start = sample().cpu_time.expect("this platform reports CPU time");
        let mut now = start;
        let mut sink = 0u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while now < start + Duration::from_millis(20) {
            for value in 0..200_000u64 {
                sink = sink.wrapping_add(value.wrapping_mul(31));
            }
            std::hint::black_box(sink);
            let reading = sample().cpu_time.unwrap();
            assert!(reading >= now, "CPU time never goes backwards");
            now = reading;
            assert!(std::time::Instant::now() < deadline, "CPU time never advanced");
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn the_browser_reports_nothing_rather_than_zero() {
        assert_eq!(sample(), ProcessSample::default());
    }

    #[test]
    fn cpu_meter_reports_the_share_of_wall_time() {
        let t0 = AnimInstant::now();
        let mut meter = CpuMeter::default();
        assert_eq!(meter.update(t0, Some(Duration::from_millis(100))), None);
        let pct = meter
            .update(t0 + Duration::from_millis(250), Some(Duration::from_millis(225)))
            .unwrap();
        assert!((pct - 50.0).abs() < 1e-6, "{pct}");
        let pct = meter
            .update(t0 + Duration::from_millis(500), Some(Duration::from_millis(725)))
            .unwrap();
        assert!((pct - 200.0).abs() < 1e-6, "two busy cores read 200%: {pct}");
    }

    #[test]
    fn cpu_meter_ignores_unusable_intervals() {
        let t0 = AnimInstant::now();
        let mut meter = CpuMeter::default();
        meter.update(t0, Some(Duration::from_millis(10)));
        assert_eq!(
            meter.update(t0, Some(Duration::from_millis(20))),
            None,
            "no wall time elapsed"
        );
        assert_eq!(
            meter.update(t0 + Duration::from_millis(250), Some(Duration::from_millis(5))),
            None,
            "a counter that went backwards is not a negative load"
        );
        let pct = meter
            .update(t0 + Duration::from_millis(500), Some(Duration::from_millis(130)))
            .unwrap();
        assert!((pct - 50.0).abs() < 1e-6, "the backwards reading rebased: {pct}");
        assert_eq!(meter.update(t0 + Duration::from_millis(750), None), None);
        assert_eq!(
            meter.update(t0 + Duration::from_millis(1000), Some(Duration::from_millis(9))),
            None,
            "a missing reading restarts the interval"
        );
    }
}
