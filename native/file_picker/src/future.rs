//! The future type every operation returns.

use std::future::{self, Future};
use std::pin::Pin;

use crate::error::PickResult;

/// A boxed future of a picker operation.
///
/// It is not `Send`: it is meant for a Venus task on the UI thread, which is
/// where the system pickers must be presented from and where a result is applied
/// in the task's frame phase. Dropping a pending future dismisses the picker
/// where the platform allows it.
pub type PickFuture<T> = Pin<Box<dyn Future<Output = PickResult<T>>>>;

/// A future that is already resolved.
#[inline]
pub(crate) fn ready<T: 'static>(result: PickResult<T>) -> PickFuture<T> {
    Box::pin(future::ready(result))
}
