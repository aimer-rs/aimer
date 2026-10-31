# Experiment: Hand-Rolled Polymorphism

Can a hand-written vtable beat `Box<dyn Trait>`, and what does it let us do that
`dyn` cannot?

This crate is a stand-alone experiment (no dependencies, not used by any other
Aimer crate). It has two parts:

- `src/main.rs`: a minimal type-erased holder, `NonDynObject`, built on a manual
  vtable.
- `benches/vtable.rs`: a benchmark that compares the compiler's vtable with
  several manual layouts and with two batch layouts that avoid per-object
  dispatch.

## Why

`dyn Trait` only works for dyn-compatible traits. A trait like

```rust
pub trait NonDyn: Sized {
    fn hello();              // no receiver
    fn hello_self(&self);
}
```

cannot be turned into a trait object: it requires `Sized` and has an associated
function without `self`. A hand-rolled vtable does not have that restriction.
Because the table is built inside a generic `new::<T>`, it can hold `T::hello`
(a plain `fn()`) next to the receiver-taking methods, and a call is still one
indirect jump.

`NonDynObject` stores `data: *mut ()` plus a function pointer per method and a
`drop` function pointer. The unsafe casts are sound because `new::<T>` is the
only place that builds the pointers, and it builds all of them from the same
`T` that was boxed. `Drop` runs the stored `drop` exactly once (covered by the
`drops_inner_value_once` test).

## Layouts compared

Every variant implements the same `work(&self) -> u64` on four concrete types
(`A`..`D`), created in a fixed pseudo-random order so the branch predictor
cannot lock onto a pattern. All variants must produce the same checksum, which
the benchmark asserts before timing anything.

| Variant | Size | Idea |
| --- | --- | --- |
| compiler vtable | 16 B | `Box<dyn Erased>`, the baseline. |
| manual, inline fn ptrs | 32 B | Function pointers stored in the object. |
| manual, shared static vtable | 16 B | Per-type `&'static VTable`, same shape as `dyn`. |
| manual, inline v2 | 24 B | Only the pointers that are used, `#[inline(always)]` wrappers. |
| manual, inline v3 | 24 B | The pointer is `T::work` itself, with no forwarding wrapper. Uses `transmute` between `fn(&T)` and `unsafe fn(*const ())`; the language does not guarantee this, so treat it as an experiment. |
| manual, thin header | 8 B | Vtable header lives in the same allocation as the value. |
| inline payload (SBO) | 32 B | Value (up to 16 B) stored in the object, no heap pointer chase. |
| bucketed vtable | n/a | Objects grouped by type in one `Vec<T>` each. One indirect call per bucket, then a direct loop. Does not preserve order across types. |
| packed arena | 16 B/object | All objects back to back in one buffer, each record starts with its vtable pointer. Order preserving, no per-object allocation. |
| tagged slabs | 8 B + 1 B tag | One `Vec<T>` per type plus a 1-byte tag stream. Order preserving. |

The last two groups also have an order-preserving **map** benchmark
(`out[i] = work(object i)`), which a bucketed vtable cannot serve. Slab scatter
does it with one indirect call per slab and writes through an index list.

## Results

Numbers are `ns` per call, best of 7 runs, from one machine and one run. See
`benches/results/vtable.txt` for the raw output and `benches/results/vtable.png`
for the chart.

| Scenario | `Box<dyn>` | Best per-object manual layout | Bucketed | Slab-scatter map vs `dyn` map |
| --- | --- | --- | --- | --- |
| 16 objects | 0.94 | 0.94 (all within noise) | 0.94 | 1.09x |
| 1,000 objects | 1.44 | 1.42 (all within noise) | 0.70 | 0.50x |
| 1,000,000 objects | 6.01 | 5.02 (v2) | 0.69 | 0.16x |
| 1 object, hot loop | 0.97 | 0.74 (v3) | 1.29 | n/a |

What this says:

- **A manual vtable does not meaningfully beat `dyn` for per-object dispatch.**
  The inline, shared, thin and SBO variants land within a few percent of
  `Box<dyn>` at 16 and 1,000 objects. At 1,000,000 objects everything is bound
  by memory latency, and the spread (5.0 to 6.7 ns) is more likely cache and
  allocator effects than a real difference between layouts.
- **Removing the forwarding wrapper (v3) only wins when everything is hot.** It
  is 24% faster with a single object, but not faster with 1,000 objects, and it
  relies on a `transmute` the language does not promise. Not worth it.
- **The big win is not dispatching per object.** Grouping by concrete type
  gives 2x at 1,000 objects and about 9x at 1,000,000, because the inner loop
  is a direct call that the compiler can inline and the CPU can predict. The
  order-preserving slab map keeps most of that (2x and about 6x).
- **Batching has a fixed cost.** With 16 objects the slab map is 9% slower than
  `dyn`, and with a single object bucketed, arena and slabs are all slower than
  plain dispatch. It pays off only once a bucket holds enough elements.

The practical conclusion for Aimer: use `dyn` (or a manual vtable only when a
trait cannot be dyn-compatible) for heterogeneous collections, and reach for
type-grouped storage when a hot path walks many objects and does not depend on
cross-type order.

Caveats: one machine, one run, no variance reported, and a micro-benchmark whose
`work` is a few arithmetic instructions, so real methods that do more work will
shrink every relative difference. Per the repository guidance, mobile CPUs are
expected to be 3 to 10 times slower than these numbers; the ratios, not the
absolute values, are what to carry over, and they should be re-measured on a
device before relying on them.

## Running

From the workspace root:

```bash
# Demo
cargo run -p polymorphism

# Test (drop runs exactly once)
cargo nextest run -p polymorphism

# Benchmark. Run it alone: concurrent work on the machine invalidates the numbers.
cargo bench -p polymorphism --bench vtable 2>/dev/null > experiments/polymorphism/benches/results/vtable.txt
```

The benchmark uses its own timing loop (`harness = false`, `Instant`, best of
7) instead of `criterion`, to keep the crate dependency-free.

To regenerate the chart (needs `matplotlib`; `.venv` is git-ignored):

```bash
cd experiments/polymorphism
python3 -m venv .venv && .venv/bin/pip install matplotlib
.venv/bin/python benches/plot.py
```

`plot.py` parses `benches/results/vtable.txt`, so adding a variant to the
benchmark only needs a re-run followed by the plot script.

## Limitations

- `NonDynObject` has no `Send`/`Sync` support (it holds a raw pointer) and no
  `Clone`, `Debug` or downcasting.
- SBO and the packed arena only accept types of at most 16 B and alignment 8 B
  (SBO), or alignment 8 B (arena). This is enforced at compile time with a
  `const` assertion.
- The tagged slabs allow at most 16 distinct types (`MAX_SLABS`) and use
  `u32` positions, so they cap at about 4 billion objects.
- None of these layouts support removing an object from the middle.
