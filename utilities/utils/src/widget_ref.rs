use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ptr::NonNull;

pub struct WidgetRc<T> {
    ptr: NonNull<WidgetRcInner<T>>,
}

unsafe impl<T> Send for WidgetRc<T> {}
unsafe impl<T> Sync for WidgetRc<T> {}

pub struct WidgetRcInner<T> {
    counter: UnsafeCell<usize>,
    value: T,
}

impl<T> WidgetRc<T> {
    pub fn new(data: T) -> Self {
        let boxed = Box::new(WidgetRcInner {
            counter: UnsafeCell::new(1),
            value: data,
        });

        WidgetRc {
            ptr: unsafe { NonNull::new_unchecked(Box::into_raw(boxed)) },
        }
    }

    fn inner(&self) -> &WidgetRcInner<T> {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> Clone for WidgetRc<T> {
    fn clone(&self) -> Self {
        let inner = self.inner();
        let count_before = unsafe { *inner.counter.get() };

        let count_after = count_before + 1;
        unsafe { *inner.counter.get() = count_after };

        WidgetRc { ptr: self.ptr }
    }
}

impl<T> Deref for WidgetRc<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner().value
    }
}

impl<T> Drop for WidgetRc<T> {
    fn drop(&mut self) {
        let inner = unsafe { self.ptr.as_ref() };
        let count_before = unsafe { *inner.counter.get() };

        let count_after = count_before - 1;
        unsafe { *inner.counter.get() = count_after };

        if count_after == 0 {
            unsafe { drop(Box::from_raw(self.ptr.as_ptr())) }
        }
    }
}
