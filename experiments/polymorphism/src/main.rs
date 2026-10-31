pub trait NonDyn: Sized {
    fn hello();

    fn hello_self(&self);
}

/// Type-erased holder built on a manual vtable.
pub struct NonDynObject {
    data: *mut (),
    hello: fn(),
    hello_self: unsafe fn(*const ()),
    drop: unsafe fn(*mut ()),
}

impl NonDynObject {
    pub fn new<T: NonDyn + 'static>(value: T) -> Self {
        unsafe fn hello_self<T: NonDyn>(p: *const ()) {
            unsafe { (*(p as *const T)).hello_self() }
        }
        unsafe fn drop_data<T>(p: *mut ()) {
            unsafe { drop(Box::from_raw(p as *mut T)) }
        }

        Self {
            data: Box::into_raw(Box::new(value)) as *mut (),
            hello: T::hello,
            hello_self: hello_self::<T>,
            drop: drop_data::<T>,
        }
    }

    pub fn hello(&self) {
        (self.hello)()
    }

    pub fn hello_self(&self) {
        // SAFETY: `data` was created from a `Box<T>` matching this fn pointer in `new`.
        unsafe { (self.hello_self)(self.data) }
    }
}

impl Drop for NonDynObject {
    fn drop(&mut self) {
        // SAFETY: `data` came from `Box::into_raw` of the matching `T`, dropped exactly once.
        unsafe { (self.drop)(self.data) }
    }
}

struct Cat(&'static str);

impl NonDyn for Cat {
    fn hello() {
        println!("Cat::hello (static)");
    }

    fn hello_self(&self) {
        println!("Cat::hello_self: {}", self.0);
    }
}

struct Dog(u32);

impl NonDyn for Dog {
    fn hello() {
        println!("Dog::hello (static)");
    }

    fn hello_self(&self) {
        println!("Dog::hello_self: {}", self.0);
    }
}

impl Drop for Dog {
    fn drop(&mut self) {
        println!("Dog {} dropped", self.0);
    }
}

fn main() {
    let objects = vec![NonDynObject::new(Cat("mew")), NonDynObject::new(Dog(7))];

    for o in &objects {
        o.hello();
        o.hello_self();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Probe;

    impl NonDyn for Probe {
        fn hello() {}
        fn hello_self(&self) {}
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drops_inner_value_once() {
        let o = NonDynObject::new(Probe);
        o.hello();
        o.hello_self();
        drop(o);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    }
}
