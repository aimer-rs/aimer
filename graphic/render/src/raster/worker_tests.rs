use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use super::{FrameWorker, WorkerEvent, WorkerPresenter};
use aimer_cupid::draw_cmd::DrawList;
use aimer_cupid::frame::{Frame, FramePacket};
use winit::dpi::PhysicalSize;

struct LocalPresenter {
    owner: Rc<std::thread::ThreadId>,
    dropped: mpsc::Sender<std::thread::ThreadId>,
}

impl WorkerPresenter for LocalPresenter {
    type Output = std::thread::ThreadId;

    fn resize(&mut self, _: PhysicalSize<u32>) {}

    fn render(&mut self, _: &FramePacket) -> Self::Output {
        assert_eq!(*self.owner, std::thread::current().id());
        *self.owner
    }
}

impl Drop for LocalPresenter {
    fn drop(&mut self) {
        self.dropped.send(std::thread::current().id()).unwrap();
    }
}

fn packet() -> FramePacket {
    FramePacket::full(Frame::new(DrawList::new(), 8, 8))
}

#[test]
fn constructs_uses_and_destroys_a_non_send_presenter_on_the_worker() {
    let ui = std::thread::current().id();
    let (dropped, destruction) = mpsc::channel();
    let (wake, wakes) = mpsc::channel();
    let mut worker = FrameWorker::spawn(
        move || {
            Ok(LocalPresenter {
                owner: Rc::new(std::thread::current().id()),
                dropped,
            })
        },
        move || {
            let _ = wake.send(());
        },
    );
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(worker.poll(), Some(WorkerEvent::Ready { .. })));
    assert!(worker.try_submit(packet()).is_ok());
    let rendered = loop {
        if let Some(WorkerEvent::Completed(frame)) = worker.poll() {
            break frame.output;
        }
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    };
    assert_ne!(rendered, ui);
    worker.stop();
    worker.join();
    assert_eq!(
        destruction.recv_timeout(Duration::from_secs(5)).unwrap(),
        rendered
    );
}

struct StalledPresenter {
    release: mpsc::Receiver<()>,
    started: mpsc::Sender<u32>,
}

impl WorkerPresenter for StalledPresenter {
    type Output = u32;
    fn resize(&mut self, _: PhysicalSize<u32>) {}
    fn render(&mut self, packet: &FramePacket) -> u32 {
        let width = packet.frame().width;
        self.started.send(width).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        width
    }
}

#[test]
fn a_stalled_presenter_bounds_admission_and_returns_rejected_packets_unchanged() {
    let (release, gate) = mpsc::channel();
    let (started, rendering) = mpsc::channel();
    let (wake, wakes) = mpsc::channel();
    let mut worker = FrameWorker::spawn(
        move || {
            Ok(StalledPresenter {
                release: gate,
                started,
            })
        },
        move || {
            let _ = wake.send(());
        },
    );
    assert!(!worker.can_submit());
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(worker.poll(), Some(WorkerEvent::Ready { .. })));
    assert!(worker.try_submit(packet()).is_ok());
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(worker.try_submit(packet()).is_ok());
    assert!(!worker.can_submit());
    let rejected = worker
        .try_submit(FramePacket::full(Frame::new(DrawList::new(), 42, 24)))
        .err()
        .expect("the third frame must be returned immediately");
    assert_eq!((rejected.frame().width, rejected.frame().height), (42, 24));
    // Even finished backend work occupies its slot until the UI receives it.
    release.send(()).unwrap();
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!worker.can_submit());
    loop {
        if let Some(WorkerEvent::Completed(frame)) = worker.poll() {
            assert_eq!(frame.output, 8);
            break;
        }
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    assert!(worker.can_submit());
    release.send(()).unwrap();
    worker.stop();
    worker.join();
    assert!(worker.try_submit(packet()).is_err());
}

#[test]
fn stopping_cancels_the_queued_frame_before_destroying_the_worker_owner() {
    let (release, gate) = mpsc::channel();
    let (started, rendering) = mpsc::channel();
    let (wake, wakes) = mpsc::channel();
    let mut worker = FrameWorker::spawn(
        move || {
            Ok(StalledPresenter {
                release: gate,
                started,
            })
        },
        move || {
            let _ = wake.send(());
        },
    );
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(worker.poll(), Some(WorkerEvent::Ready { .. })));
    worker
        .try_submit(packet())
        .unwrap_or_else(|_| panic!("first admission"));
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    worker
        .try_submit(packet())
        .unwrap_or_else(|_| panic!("second admission"));
    worker.stop();
    assert!(!worker.can_submit());
    release.send(()).unwrap();
    worker.join();
    assert!(worker.is_stopped());
    assert!(
        rendering.try_recv().is_err(),
        "the queued second frame must be cancelled"
    );
    let mut completions = 0;
    let mut stopped = false;
    while let Some(event) = worker.poll() {
        match event {
            WorkerEvent::Completed(_) => completions += 1,
            WorkerEvent::Stopped => stopped = true,
            _ => panic!("unexpected shutdown event"),
        }
    }
    assert_eq!(completions, 1);
    assert!(stopped);
}

#[test]
fn an_initialization_failure_is_reported_and_never_admits_frames() {
    let (wake, wakes) = mpsc::channel();
    let mut worker = FrameWorker::spawn::<LocalPresenter, _, _>(
        || Err("device unavailable".into()),
        move || {
            let _ = wake.send(());
        },
    );
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!worker.can_submit());
    assert!(
        matches!(worker.poll(), Some(WorkerEvent::Failed(error)) if error == "device unavailable")
    );
    worker.join();
    assert!(matches!(worker.poll(), Some(WorkerEvent::Stopped)));
    assert!(worker.try_submit(packet()).is_err());
}
