//! Bounded frame transport. Only the factory crosses threads; its GPU owner need not be Send.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread::JoinHandle;

use aimer_frame_stats::PhaseTimer;
use aimer_cupid::draw_cmd::DrawList;
use aimer_cupid::frame::FramePacket;
use winit::dpi::PhysicalSize;

struct SubmittedFrame {
    packet: FramePacket,
    queued: PhaseTimer,
    cpu: PhaseTimer,
    generation: u64,
}

/// A renderer constructed, used, and destroyed entirely on its worker thread.
///
/// Unlike [`super::FramePresenter`], this interface allows a non-`Send` GPU
/// owner. Only its factory and completed presentation tokens cross threads.
pub trait WorkerPresenter: 'static {
    /// A presentation result or a token to finish presentation on the UI thread.
    type Output: Send + 'static;
    /// Applies the next frame's dimensions before acquiring its drawable.
    fn resize(&mut self, size: PhysicalSize<u32>);
    /// Encodes a borrowed packet. It remains alive until this call returns.
    fn render(&mut self, packet: &FramePacket) -> Self::Output;
    /// Maximum supported physical target dimension, reported during initialization.
    fn max_texture_dimension(&self) -> u32 {
        u32::MAX
    }
}

/// A completed packet and its reusable command storage.
pub struct CompletedFrame<T> {
    /// Backend result, consumed on the UI thread.
    pub output: T,
    /// Storage that can be returned to the recording canvas.
    pub draw_list: DrawList,
    /// Surface generation recorded with the packet.
    pub generation: u64,
}

/// Messages returned by a [`FrameWorker`].
pub enum WorkerEvent<T> {
    /// Initialization finished; frames may now be submitted.
    Ready {
        /// Maximum supported target dimension.
        max_texture_dimension: u32,
    },
    /// A frame's backend work finished.
    Completed(CompletedFrame<T>),
    /// Initialization or rendering failed. The application must stop rendering.
    Failed(String),
    /// All GPU owners have been destroyed on the worker.
    Stopped,
}

/// Nonblocking transport for at most two outstanding frames.
///
/// A completion occupies its slot until [`Self::poll`] delivers it. Applications
/// finish any main-thread presentation before reserving another frame, so a
/// delayed UI thread cannot accumulate drawables or recycled buffers.
pub struct FrameWorker<T: Send + 'static> {
    messages: Option<SyncSender<SubmittedFrame>>,
    events: Receiver<WorkerEvent<T>>,
    stopping: Arc<AtomicBool>,
    wake_pending: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    outstanding: usize,
    ready: bool,
    stopped: bool,
}

impl<T: Send + 'static> FrameWorker<T> {
    /// Starts a named worker, invoking `create` there before accepting frames.
    ///
    /// `wake` must only notify the event loop; it runs on the worker. Panics in
    /// the factory, presenter, or its destructor become [`WorkerEvent::Failed`].
    /// Panics if the operating system cannot create the worker thread.
    pub fn spawn<P, C, W>(create: C, wake: W) -> Self
    where
        P: WorkerPresenter<Output = T>,
        C: FnOnce() -> Result<P, String> + Send + 'static,
        W: Fn() + Send + 'static,
    {
        let (messages, inbox) = sync_channel::<SubmittedFrame>(2);
        // Ready + two completions + Stopped fit even when the UI is paused.
        let (results, events) = sync_channel(4);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stopping);
        let wake_pending = Arc::new(AtomicBool::new(false));
        let worker_wake = Arc::clone(&wake_pending);
        let worker = std::thread::Builder::new()
            .name("aimer-raster".into())
            .spawn(move || {
                let report = |event| {
                    if results.send(event).is_ok() && !worker_wake.swap(true, Ordering::AcqRel) {
                        wake();
                    }
                };
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    with_worker_pool(|| {
                        let mut presenter = create()?;
                        report(WorkerEvent::Ready {
                            max_texture_dimension: presenter.max_texture_dimension(),
                        });
                        let mut size = PhysicalSize::new(0, 0);
                        while let Ok(submitted) = inbox.recv() {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            submitted.queued.finish_queue();
                            let packet = submitted.packet;
                            let frame = packet.frame();
                            let next_size = PhysicalSize::new(frame.width, frame.height);
                            if next_size != size {
                                presenter.resize(next_size);
                                size = next_size;
                            }
                            let output = presenter.render(&packet);
                            submitted.cpu.finish_cpu_frame();
                            let generation = submitted.generation;
                            let draw_list = packet.into_frame().into_draw_list();
                            report(WorkerEvent::Completed(CompletedFrame {
                                output,
                                draw_list,
                                generation,
                            }));
                        }
                        Ok::<_, String>(())
                    })
                }));
                match run {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => report(WorkerEvent::Failed(error)),
                    Err(panic) => {
                        let error = panic
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                            .unwrap_or_else(|| "render worker panicked".into());
                        report(WorkerEvent::Failed(error));
                    }
                }
                report(WorkerEvent::Stopped);
            })
            .expect("failed to spawn the aimer raster thread");
        Self {
            messages: Some(messages),
            events,
            stopping,
            wake_pending,
            worker: Some(worker),
            outstanding: 0,
            ready: false,
            stopped: false,
        }
    }

    /// Whether recording another frame can proceed without waiting.
    #[inline]
    pub fn can_submit(&self) -> bool {
        self.ready && self.messages.is_some() && self.outstanding < 2
    }

    /// Submits without waiting, returning ownership unchanged when full or stopped.
    pub fn try_submit(&mut self, packet: FramePacket) -> Result<(), FramePacket> {
        self.try_submit_timed(packet, PhaseTimer::start())
    }

    pub(crate) fn try_submit_timed(
        &mut self,
        packet: FramePacket,
        cpu: PhaseTimer,
    ) -> Result<(), FramePacket> {
        let generation = packet.metadata().context_generation();
        self.try_submit_tagged(packet, cpu, generation)
    }

    pub(crate) fn try_submit_tagged(
        &mut self,
        packet: FramePacket,
        cpu: PhaseTimer,
        generation: u64,
    ) -> Result<(), FramePacket> {
        if !self.can_submit() {
            return Err(packet);
        }
        let submitted = SubmittedFrame {
            packet,
            queued: PhaseTimer::start(),
            cpu,
            generation,
        };
        match self.messages.as_ref().unwrap().try_send(submitted) {
            Ok(()) => {
                self.outstanding += 1;
                Ok(())
            }
            Err(TrySendError::Full(submitted) | TrySendError::Disconnected(submitted)) => {
                Err(submitted.packet)
            }
        }
    }

    /// Delivers a completion without waiting; finish its presentation before submitting again.
    pub fn poll(&mut self) -> Option<WorkerEvent<T>> {
        let event = match self.events.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Empty) => {
                // Rearm before checking again: a producer racing this empty
                // read must either be observed here or issue another wake.
                self.wake_pending.store(false, Ordering::Release);
                self.events.try_recv().ok()?
            }
            Err(TryRecvError::Disconnected) => return None,
        };
        Some(self.accept(event))
    }

    /// Delivers the next event, waiting up to `timeout` for one to arrive.
    ///
    /// Only for a UI thread that must show a frame in the turn that asked for
    /// it, such as a live window resize; everything else should use
    /// [`Self::poll`]. The same delivery rules apply: finish the completion's
    /// presentation before submitting again.
    pub fn wait(&mut self, timeout: std::time::Duration) -> Option<WorkerEvent<T>> {
        let event = self.events.recv_timeout(timeout).ok()?;
        Some(self.accept(event))
    }

    /// Frames submitted whose completion has not been delivered yet.
    #[inline]
    pub fn outstanding(&self) -> usize {
        self.outstanding
    }

    fn accept(&mut self, event: WorkerEvent<T>) -> WorkerEvent<T> {
        match &event {
            WorkerEvent::Ready { .. } => self.ready = true,
            WorkerEvent::Completed(_) => self.outstanding -= 1,
            WorkerEvent::Failed(_) => self.ready = false,
            WorkerEvent::Stopped => {
                self.ready = false;
                self.stopped = true;
            }
        }
        event
    }

    /// Cancels queued frames and wakes a parked worker. Does not wait for GPU work.
    pub fn stop(&mut self) {
        self.ready = false;
        self.stopping.store(true, Ordering::Release);
        self.messages = None;
    }

    /// Whether the worker has released its GPU objects and can be joined without waiting.
    pub fn is_stopped(&self) -> bool {
        self.stopped
            || self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
    }

    /// Joins after stopping. Native Windows callers must keep pumping messages until stopped.
    pub fn join(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl<T: Send + 'static> Drop for FrameWorker<T> {
    fn drop(&mut self) {
        self.join();
    }
}

fn with_worker_pool<R>(run: impl FnOnce() -> R) -> R {
    #[cfg(all(feature = "native", any(target_os = "macos", target_os = "ios")))]
    {
        objc2::rc::autoreleasepool(|_| run())
    }
    #[cfg(not(all(feature = "native", any(target_os = "macos", target_os = "ios"))))]
    {
        run()
    }
}
