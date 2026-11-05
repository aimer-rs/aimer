# aimer_venus

Aimer's UI-thread async runtime.

Venus is **not** a replacement for Tokio. It owns the one thing a
general-purpose runtime cannot express, *when*, relative to a frame,
asynchronous work runs, and it owns one small pool for work that has to leave
the UI thread. Bring your own I/O runtime for sockets; Venus schedules the UI.

## Why it exists

A server runtime optimises throughput across thousands of sockets. A UI runtime
optimises something else entirely:

| Concern         | Server runtime      | Venus                              |
|-----------------|---------------------|------------------------------------|
| Unit of time    | none, run ASAP      | the frame (16.6 / 8.3 ms)          |
| Thread model    | work-stealing pool  | one UI thread + optional `spawn_blocking` |
| `Send` bound    | required            | only at the `spawn_blocking` boundary   |
| Wake → run      | µs, unordered      | before *this* frame's build phase  |
| Backpressure    | queue depth         | frame budget, deadline-aware       |
| Cancellation    | rare                | constant, tied to the element tree |

Three of those are load-bearing for a GUI:

- **Tasks are not `Send`.** A handler may `await` while holding a `StateUpdater`,
  a controller, or any other `Rc` from the element tree, which is what 95 % of
  real handlers capture. A runtime that polls on a pool cannot allow that at any
  price.
- **Tasks run in a phase.** "As soon as possible" is not a schedule a UI can be
  built on. A resolved future's `set_state` has to be visible to *this* frame's
  build, not the next one; anything else is a one-frame input lag that is
  structural, not tunable.
- **Background work is budgeted.** An 8.3 ms frame has no room for a 40 ms
  decode, and a scheduler that cannot measure the slack it is spending will
  eventually spend a frame.

## The frame

```text
input ──▶ frame tasks ──▶ microtasks ──▶ build / layout / paint ──▶ idle ──▶ present
              │                │                                      │
       animation ticks   drained to        the caller's work    budget-gated,
       once per frame    exhaustion                             resumable
```

| Phase       | Runs                       | For                                        |
|-------------|----------------------------|--------------------------------------------|
| `Microtask` | before the build, to empty | `set_state`, focus change, notify          |
| `Frame`     | once per frame             | animation ticks, after-layout callbacks    |
| `Idle`      | while the frame has room   | image decode, glyph raster, prefetch       |

Everything unsliceable goes to `Venus::spawn_blocking`, the single place `Send` is
required, and where it is genuinely true.

Two small pieces sit on top of it for code that talks to the platform:

* `Lane` runs blocking calls one at a time, in the order they were issued, on the
  pool, so a read issued after a write sees it. Secret stores and file storage use
  it. It can carry shared state and hooks for "a job panicked" and "the queue ran
  empty".
* `completion()` is a one-value rendezvous for a result that arrives on a thread of
  the platform's choosing, such as a system callback: the producer is `Send`, the
  awaiting task is not, and a producer dropped without a value resolves the task
  with `Interrupted` instead of leaving it parked.

## Using it

```rust
use std::cell::Cell;
use std::rc::Rc;

use aimer_venus::Venus;

let venus = Venus::for_refresh_rate(120.0);
let counter = Rc::new(Cell::new(0));

// A task holding an `Rc`, resolved before the build that reads it.
let counted = counter.clone();
venus.spawn(async move { counted.set(counted.get() + 1) });

let read = counter.clone();
venus.drive_frame(|| assert_eq!(read.get(), 1));
```

Blocking work leaves the thread and comes back into the frame:

```rust
# use std::cell::Cell;
# use std::rc::Rc;
use aimer_venus::Venus;

let venus = Venus::new();
let state = Rc::new(Cell::new(0));

let runtime = venus.clone();
let mutated = state.clone();
venus.spawn(async move {
    let bytes = runtime.spawn_blocking(|| vec![0_u8; 1024]).await;
    // Still on the UI thread, still holding the `Rc`.
    mutated.set(bytes.len());
});

while venus.task_count() > 0 {
    venus.run_microtasks();
}
assert_eq!(state.get(), 1024);
```

An element that owns tasks owns a scope, and unmounting cancels them:

```rust
# use aimer_venus::Venus;
let venus = Venus::new();
let scope = venus.scope();

venus.spawn_in(scope.id(), async { /* fetch, decode, … */ });

drop(scope); // the element unmounted: the task is simply gone
assert_eq!(venus.task_count(), 0);
```

Long background work slices itself against the deadline:

```rust
# use aimer_venus::{Venus, yield_if_over_budget};
let venus = Venus::new();

venus.spawn_idle(async {
    for _tile in 0..4096 {
        // decode_one_tile();
        yield_if_over_budget().await;
    }
});

venus.run_idle(&venus.idle_budget());
```

## Time

Venus has no clock thread of its own; timers are the host's, supplied through
`PollContext::sleep`, so they wake a parked loop and are cancelled by dropping
them. A host without timers gets `TimeoutError::TimerUnavailable` rather than a
sleep that silently never ends.

```rust
use std::time::Duration;
use aimer_venus::Venus;

let venus = Venus::new();

// Await inside a task. Every one of these also has a free function that uses
// the runtime installed for the thread: `sleep`, `delayed`, `timeout`, ...
let runtime = venus.clone();
venus.spawn(async move {
    let _ = runtime.sleep(Duration::from_millis(250)).await;
    let _ = runtime.delayed(Duration::from_secs(1), || "tooltip").await;
});

// Fire and forget; abort the task, or drop its scope, to cancel.
let _ = venus.spawn_delayed(Duration::from_secs(2), || { /* hide toast */ });

// A drift-free ticker that skips ticks it missed instead of bursting.
let mut ticker = venus.interval(Duration::from_secs(1));
venus.spawn(async move {
    while ticker.tick().await.is_ok() { /* refresh clock */ }
});
```

Bursty events are rate limited by two timer-driven helpers, so nothing has to
poll them each frame. Both cancel what they hold when dropped, so an element
that owns one cleans up on unmount.

```rust
use std::time::Duration;
use aimer_venus::Venus;

let venus = Venus::new();

// Runs once, with the last action, after the calls stop for 250 ms.
let search = venus.debouncer(Duration::from_millis(250));
let _ = search.call(|| { /* run the query */ });

// The first call runs now; the newest call made inside the window runs when
// it closes, so the final value of a burst is never lost.
let scroll = venus.throttler(Duration::from_millis(100));
let _ = scroll.call(|| { /* save the offset */ });
```

## Combining futures

Plain combinators, polled by the task that awaits them: nothing is spawned,
nothing needs to be `Send`, and dropping one drops every future inside it, so a
scope cancellation reaches them.

```rust
use aimer_venus::{Either, Venus, join, join_all, race, try_join};

let venus = Venus::new();
venus.spawn(async {
    // Different types, run together, one tuple back.
    let (width, name) = join!(async { 320 }, async { "avatar" }).await;

    // Any number of the same type, results in input order.
    let squares = join_all((0..4).map(|n| async move { n * n })).await;

    // Stops at the first error and cancels the rest.
    let both = try_join!(async { Ok::<_, ()>(1) }, async { Ok(2) }).await;

    // The first to finish wins and the other is dropped.
    let first = race(async { "a" }, std::future::pending::<()>()).await;
    assert!(matches!(first, Either::Left("a")));
    # let _ = (width, name, squares, both);
});
venus.run_microtasks();
```

A task whose result you want back is spawned with `spawn_with_handle`. The
`JoinHandle` is a future of the output, and it **owns the task**: dropping it
cancels the work, `detach()` lets it run on, and a task cut short by `abort` or
its scope's end resolves the handle to `JoinError::Cancelled`.

```rust
use aimer_venus::{JoinError, Venus};

let venus = Venus::new();
let scope = venus.scope();
let request = venus.spawn_with_handle_in(scope.id(), std::future::pending::<u8>());

drop(scope); // the element unmounted
venus.spawn(async move { assert_eq!(request.await, Err(JoinError::Cancelled)) });
venus.run_microtasks();
```

Also: `try_join_all`, `race_all`, and a free `spawn_blocking` that uses the
installed runtime (the method form is `Venus::spawn_blocking`).

## Talking between tasks

Single-threaded primitives: nothing here needs `Send`, and each is cancellation
safe, so a task dropped while waiting never swallows a signal meant for another.

| Type                      | For                                                          |
|---------------------------|--------------------------------------------------------------|
| `oneshot::channel`        | one value from one task to another                           |
| `mpsc::channel(n)` / `unbounded` | many producers, one consumer; bounded gives backpressure |
| `watch::channel`          | shared state observers re-read when it changes (latest wins) |
| `Notify`                  | an edge: "something happened, look again"                    |
| `Semaphore`               | cap concurrency (N decodes, N requests), first come first served |
| `OnceCell`                | initialise once; concurrent askers wait for the one initialiser |
| `Shared`                  | one in-flight operation, many awaiters, cancelled with the last |

For a result that arrives on a platform thread, use `completion()` instead.

```rust
use aimer_venus::{Semaphore, StreamExt, Venus, mpsc};

let venus = Venus::new();
let (tx, rx) = mpsc::channel(8);
let gate = Semaphore::new(2); // at most two jobs at once

for job in 0..4 {
    let (tx, gate) = (tx.clone(), gate.clone());
    venus.spawn(async move {
        let _permit = gate.acquire().await;
        tx.send(job * job).await.ok();
    });
}
drop(tx);

// A receiver is a `Stream`.
let squares = venus.run_until(rx.collect_vec());
assert_eq!(squares.len(), 4);
```

`Stream`/`StreamExt` is a small core of its own (`next`, `map`, `filter`,
`take`, `for_each`, `collect_vec`, `stream::iter`), because the standard library
has no stable stream trait and Venus takes no dependencies.

## Hopping phases

`yield_now` comes back in the same phase. `next_frame().await` resumes at the
next frame-task pass and so before that frame's build; `run_in_phase(Phase::Idle,
work).await` runs `work` when the frame has slack.

```rust
# use aimer_venus::{Phase, Venus};
let venus = Venus::new();
let runtime = venus.clone();
venus.spawn(async move {
    runtime.next_frame().await.unwrap();
    let _parsed = runtime.run_in_phase(Phase::Idle, async { 40 + 2 }).await;
});
```

## Retrying and chaining

```rust
use std::time::Duration;
use aimer_venus::{Backoff, FutureExt, Venus};

let venus = Venus::new();
venus.install();

// 100 ms, 200 ms, 400 ms ... capped, up to 5 tries; the last error comes back.
let _fetch = venus.retry(Backoff::exponential(Duration::from_millis(100)).attempts(5), || async {
    Ok::<_, &str>("payload")
});

// Method forms of the combinators.
let _bounded = async { 1 }.timeout(Duration::from_secs(5));
let _handle = async { 2 }.into_handle(); // owned: dropping it cancels
Venus::uninstall();
```

Backoff has no jitter: Venus has no source of randomness, and the delays stay
deterministic and testable. With no host timer there is nothing to wait with, so
a failed attempt is final rather than retried in a tight loop.

## Latest wins

`Latest` is a slot that runs one async operation at a time: starting the next
drops the previous future, so the slow answer to an old query can never land
after the answer to the new one. Dropping the slot cancels what is in flight.
Pair it with a `Debouncer` for search-as-you-type.

```rust
# use aimer_venus::Venus;
let venus = Venus::new();
let search = venus.latest();

search.run(async { /* fetch("ru").await; show(results) */ });
search.run(async { /* fetch("rust").await; show(results) */ }); // the first is gone
```

## Driving it without an event loop

Tests and tools have no loop to run the phases. `run_until` spawns a future and
drives every phase, sleeping between looks so a result from a worker thread is
picked up, until it finishes. It panics after ten seconds so a stuck test fails
rather than hangs; `run_until_for` takes the limit and returns `None` on
timeout after cancelling the future. `run_until_stalled` runs until nothing is
ready and leaves work parked on a timer or a worker parked.

```rust
# use aimer_venus::{Venus, yield_now};
let venus = Venus::new();
let answer = venus.run_until(async {
    yield_now().await;
    6 * 7
});
assert_eq!(answer, 42);
```

## Driving it from an event loop

```text
venus.begin_frame();
venus.run_frame_tasks();     // animation ticks
venus.run_microtasks();      // every resolved effect, before the build
build_layout_paint();
venus.run_idle(&venus.idle_budget());
venus.end_frame();           // records whether the frame overran
```

`Venus::drive_frame` does exactly that. A loop that needs to interleave its own
work calls the phases individually, which is what Aimer's own loop does: the
frame tasks and the microtask drain sit at the end of
`AimerApplicationHandler::begin_frame`, after this frame's input and before the
tree is built, and the idle pass sits in `end_frame` once it has been drawn.
Both the windowed loop and the headless application go through those two, so a
frame costs the same work whichever asked for it.

The runtime is installed for the drawing thread when the application starts, so
a handler eleven elements deep reaches it through `Venus::current` or
`spawn_local` rather than being handed a spawner from above.

Two things a loop should know:

- `has_ready_work()`, whether it may wait for OS events or has to come round
  again immediately.
- `set_notifier(..)`, how a worker wakes a parked loop. Called *only* for wakes
  raised off the UI thread; a wake raised on the UI thread never pings it,
  because the loop is demonstrably awake.

## Hosting another runtime's futures

A future from `reqwest`, `tokio::fs` or `tokio::time` does not need its runtime
to *poll* it, it needs that runtime to be **findable** while it is polled,
because it builds its resources on the first poll and looks the runtime up in a
thread-local. Venus polls on the UI thread, where that lookup finds nothing, so
such a future would panic before it reached the network.

One hook fixes that for every spawn path at once. The host installs an adapter
and Venus enters it around each poll, and only around the poll, never across an
`await`:

```rust
use std::cell::Cell;
use std::rc::Rc;

use aimer_venus::{PollContext, Venus};

// Aimer's own adapter is `aimer_quiver::poll_context::TokioPollContext`,
// whose `enter` is `let _guard = self.handle.enter(); poll();`.
struct MarkThread(Rc<Cell<bool>>);

impl PollContext for MarkThread {
    fn enter(&self, poll: &mut dyn FnMut()) {
        self.0.set(true);
        poll();
        self.0.set(false);
    }
}

let inside = Rc::new(Cell::new(false));
let venus = Venus::new();
venus.set_poll_context(MarkThread(inside.clone()));

let observed = Rc::new(Cell::new(false));
let seen = observed.clone();
let marked = inside.clone();
venus.spawn(async move { seen.set(marked.get()) });
venus.run_microtasks();

assert!(observed.get());
```

Venus still names no runtime and depends on none: it knows only how to run a
closure inside whatever the host handed it. Aimer's application loop installs
the adapter for its async runtime next to `Venus::install`, which is why
`examples/http_request_button.rs` can `spawn_local` a request from a button
handler with nothing else in the way.

The completion side needed no work at all. The other runtime's reactor wakes
Venus's waker from its own thread, and the task resumes in the phase it was
spawned into, so an HTTP response lands *before this frame's build*, which is
more than the runtime that fetched it can promise. Its driver threads keep the
handshake and the decoding off the UI thread entirely.

## Costs, honestly

- A task pays one `Box::pin` at spawn and nothing per poll. Polling a ready task
  is a `VecDeque::pop_front`, a slab index and one `Future::poll`, tens of
  nanoseconds, so draining ten thousand microtasks fits comfortably inside a
  120 Hz frame.
- A cooperative scheduler cannot preempt. An `async` handler that does 12 ms of
  straight-line work with no `await` will drop a frame; builds with the
  `venus-debug` feature print the offending poll's duration so the author hears
  about it before a user does.
- A microtask drain is deliberately unbudgeted, so a microtask that re-wakes
  itself forever hangs the frame. Builds with the `venus-debug` feature warn
  when one poll exceeds 1 ms and assert once the drain passes 100 000 polls.
- An installed poll context costs one `Rc` clone and one dynamic call per poll,
  plus whatever the host's guard is, a thread-local swap, for Tokio. With no
  context installed the poll path reads an empty `Option` and nothing else.
