use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

pub trait SyncFuture: Future {
    fn block(self) -> Self::Output
    where
        Self: Sized,
    {
        let mut fut = pin!(self);
        let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
        let mut cx = Context::from_waker(&waker);

        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(val) => return val,
                Poll::Pending => thread::park(),
            }
        }
    }
}

impl<F: Future> SyncFuture for F {}

#[cfg(test)]
mod test {
    use crate::SyncFuture;

    #[test]
    fn test_block_on() {
        let fut = async { 10 };

        let res = fut.block();
        assert_eq!(res, 10);
    }
}
