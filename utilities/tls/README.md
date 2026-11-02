# aimer_tls

Thread-local storage that does not run out of OS TLS keys.

On `aarch64-linux-android` (and other targets without native `#[thread_local]`
support in std) every `std::thread_local!` static is backed by its own
`pthread_key_create` key. Bionic gives a whole process only about 128-141 keys,
and the Android runtime, libc++ and Skia already hold some of them. A GUI
framework that declares one `thread_local!` per subsystem exhausts the rest, and
std then aborts with `fatal runtime error: out of TLS keys`.

`aimer_tls::thread_local!` has the same grammar as `std::thread_local!`, but
every static declared through it, on every target, lives in a single per-thread
table, so all of them together cost at most two OS TLS keys, however many
there are: one for the table and one whose destructor tears it down. On targets
with native `#[thread_local]` they cost none.

```rust
use std::cell::{Cell, RefCell};

aimer_tls::thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static NAMES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

DEPTH.update(|depth| depth + 1);
NAMES.with_borrow_mut(|names| names.push("root".to_owned()));
assert_eq!(DEPTH.get(), 1);
assert_eq!(NAMES.with_borrow(Vec::len), 1);
```

Use it instead of `std::thread_local!` everywhere in the workspace; clippy's
`disallowed-macros` rejects the std macro.

## Reading several keys at once

Each key access looks up the thread's table. Code that touches several keys in
a row can resolve it once with `aimer_tls::scope`:

```rust
use std::cell::Cell;

aimer_tls::thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(4) };
    static HEIGHT: Cell<u32> = const { Cell::new(3) };
}

let area = aimer_tls::scope(|t| t.get(&WIDTH) * t.get(&HEIGHT));
assert_eq!(area, 12);
```

A scope sees the same storage as the keys' own methods, nests with them, and
cannot leave its closure.

## What is supported

`LocalKey` mirrors `std::thread::LocalKey`: `with`, `try_with`, and the `Cell`
(`get`, `set`, `update`, `take`, `replace`) and `RefCell` (`with_borrow`,
`with_borrow_mut`, `set`, `take`, `replace`) helpers. `set` stores the new value
directly without running the initializer, as in std.

## Differences from std

- **Teardown order.** Values are dropped newest slot first, where a slot is
  numbered by the key's first use in the process, so std's per-key order is not
  reproduced. As in std, a destructor may keep using every other key (even
  one never touched before), and only a key whose own destructor has started
  or finished reports `AccessError` from `try_with`, or panics from `with`.
- **Destructor panics** abort the process, as in std.
- **Re-entrant initialization** of the same key is tolerated: the first value
  stored wins and the other is dropped, so the initializer may run twice. std
  documents that case as unsupported.
- **Cost.** A native `#[thread_local]` is a TLS-relative load. Here each access
  also loads the key's slot index, bounds-checks the per-thread table and
  follows a pointer to the boxed value, and the first access to a key
  allocates. `const { .. }` initializers are not free as they are in std.
  `cargo bench -p aimer_tls --profile dev` prints the numbers against std.
- **Allocator.** Values and the table are allocated with `std::alloc::System`,
  never the global allocator, so a `#[global_allocator]` may itself use
  `aimer_tls`, as with std.
- **Table size.** Slot indices come from one process-wide counter and are never
  reused, so a thread's table has one entry per key up to the highest index it
  has touched.

## Hot thread fast path

One thread owns the fast path at a time. Its accesses skip std's TLS lookup and
the table: each key caches the address of the owner's value, and a read is the
thread register compared with the key's cached thread plus one load. The cache
is cleared before the owner stops owning and before any value is destroyed.
Everyone else behaves as before.

The first thread that creates a thread-local value becomes the owner on its own
and keeps it until it exits. A thread that wants the fast path for sure can
register with `aimer_tls::hot()`; its guard is inactive if some thread already
owns it, which the program survives, only slower.

```
use std::cell::Cell;

aimer_tls::thread_local! {
    static FRAMES: Cell<u32> = const { Cell::new(0) };
}

let _hot = aimer_tls::hot();
FRAMES.set(FRAMES.get() + 1);
assert_eq!(FRAMES.get(), 1);
```
