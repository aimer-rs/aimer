//! Delivers notifications after a delay, for platforms whose notification
//! service cannot be asked to.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Notification, NotificationId};

struct State {
    pending: Vec<(Instant, Notification)>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A thread that sleeps until the earliest pending notification is due and
/// hands it to `fire` with its delay cleared.
///
/// The thread parks on a condition variable while nothing is pending, so an
/// idle scheduler costs no wake-ups. Dropping the scheduler discards whatever is
/// still pending and lets the thread exit.
pub(crate) struct Scheduler {
    shared: Arc<Shared>,
}

impl Scheduler {
    pub(crate) fn new(fire: impl Fn(Notification) + Send + 'static) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State { pending: Vec::new(), shutdown: false }),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("aimer-notification-scheduler".into())
            .spawn(move || run(&worker, fire))?;
        Ok(Self { shared })
    }

    /// Delivers `notification` after `delay`, replacing a pending one with the
    /// same id.
    pub(crate) fn schedule(&self, notification: Notification, delay: Duration) {
        let due = Instant::now() + delay;
        let mut state = self.shared.lock();
        state.pending.retain(|(_, pending)| pending.id != notification.id);
        state.pending.push((due, notification));
        drop(state);
        self.shared.wake.notify_one();
    }

    /// Drops the pending notification named `id`; whether there was one.
    pub(crate) fn cancel(&self, id: NotificationId) -> bool {
        let mut state = self.shared.lock();
        let before = state.pending.len();
        state.pending.retain(|(_, pending)| pending.id != id);
        before != state.pending.len()
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.shutdown = true;
        state.pending.clear();
        drop(state);
        self.shared.wake.notify_all();
    }
}

fn run(shared: &Shared, fire: impl Fn(Notification)) {
    loop {
        let due = {
            let mut state = shared.lock();
            loop {
                if state.shutdown {
                    return;
                }
                let now = Instant::now();
                let next = state
                    .pending
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(index, (at, _))| (index, *at));
                match next {
                    None => {
                        state = shared.wake.wait(state).unwrap_or_else(|p| p.into_inner());
                    }
                    Some((index, at)) if at <= now => {
                        break state.pending.swap_remove(index).1;
                    }
                    Some((_, at)) => {
                        state = shared
                            .wake
                            .wait_timeout(state, at - now)
                            .unwrap_or_else(|p| p.into_inner())
                            .0;
                    }
                }
            }
        };
        // Fired without the lock, so a slow backend never blocks `schedule`.
        let mut due = due;
        due.delay = None;
        fire(due);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    const SOON: Duration = Duration::from_millis(20);
    const PATIENCE: Duration = Duration::from_secs(5);

    fn note(id: u64, title: &str) -> Notification {
        Notification::new(NotificationId(id)).title(title).delay(SOON)
    }

    fn scheduler() -> (Scheduler, mpsc::Receiver<Notification>) {
        let (tx, rx) = mpsc::channel();
        let scheduler = Scheduler::new(move |n| {
            let _ = tx.send(n);
        })
        .expect("a test thread can be spawned");
        (scheduler, rx)
    }

    #[test]
    fn fires_after_delay_with_delay_cleared() {
        let (scheduler, rx) = scheduler();
        let start = Instant::now();
        scheduler.schedule(note(1, "a"), SOON);
        let fired = rx.recv_timeout(PATIENCE).expect("fires");
        assert!(start.elapsed() >= SOON);
        assert_eq!(fired.id, NotificationId(1));
        assert_eq!(fired.delay, None);
    }

    #[test]
    fn fires_in_due_order() {
        let (scheduler, rx) = scheduler();
        scheduler.schedule(note(1, "late"), Duration::from_millis(80));
        scheduler.schedule(note(2, "early"), SOON);
        assert_eq!(rx.recv_timeout(PATIENCE).expect("first").id, NotificationId(2));
        assert_eq!(rx.recv_timeout(PATIENCE).expect("second").id, NotificationId(1));
    }

    #[test]
    fn cancel_prevents_delivery() {
        let (scheduler, rx) = scheduler();
        scheduler.schedule(note(1, "a"), Duration::from_millis(150));
        assert!(scheduler.cancel(NotificationId(1)));
        assert!(!scheduler.cancel(NotificationId(1)), "nothing left to cancel");
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
    }

    #[test]
    fn same_id_replaces_pending() {
        let (scheduler, rx) = scheduler();
        scheduler.schedule(note(1, "old"), Duration::from_millis(100));
        scheduler.schedule(note(1, "new"), SOON);
        assert_eq!(rx.recv_timeout(PATIENCE).expect("fires").title, "new");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the old one is gone");
    }

    #[test]
    fn drop_discards_pending() {
        let (scheduler, rx) = scheduler();
        scheduler.schedule(note(1, "a"), Duration::from_millis(100));
        drop(scheduler);
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
    }
}
