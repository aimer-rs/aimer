use super::*;

#[test]
fn draining_timing_windows_preserves_each_sample_once() {
    let phase = AtomicPhase::default();
    phase.record(Duration::from_millis(2));
    let (first, _) = phase.take();
    phase.record(Duration::from_millis(4));
    let (second, _) = phase.take();
    assert_eq!(first.samples + second.samples, 2);
    assert_eq!(first.total + second.total, Duration::from_millis(6));
    assert_eq!(phase.snapshot(), PhaseSamples::default());
}

#[test]
fn concurrent_timing_drains_do_not_erase_worker_samples() {
    let phase = AtomicPhase::default();
    let done = std::sync::atomic::AtomicBool::new(false);
    let mut count = 0;
    let mut total = Duration::ZERO;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..10_000 {
                phase.record(Duration::from_nanos(10));
            }
            done.store(true, Ordering::Release);
        });
        while !done.load(Ordering::Acquire) {
            let (window, _) = phase.take();
            count += window.samples;
            total += window.total;
            std::thread::yield_now();
        }
    });
    let (last, _) = phase.take();
    assert_eq!(count + last.samples, 10_000);
    assert_eq!(total + last.total, Duration::from_nanos(100_000));
}
