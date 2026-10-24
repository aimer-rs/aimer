# `aimer_rubick` benchmark results

Reproduce with:

```bash
cargo run -p aimer_rubick --example rubick_benchmark            # dev profile
cargo run -p aimer_rubick --release --example rubick_benchmark  # release profile
```

Each scenario runs five rounds and the best round is reported, in nanoseconds
per operation. Measured on an Apple Silicon macOS host, `rustc` stable,
edition 2024. Absolute numbers depend on the machine; the ratios do not.

The payload types are a 16-byte `Tiny`, a 56-byte `Medium` and a 120-byte
`Large`, all erased to `dyn Node`.

## Layout

| Type | Before | After |
| --- | ---: | ---: |
| `Rubick<T>` (4 words) | 80 B, align 16 | 40 B, align 8 |
| `AnyWidget` | 80 B | 72 B |
| `AnyElement` | 80 B | 16 B |
| `Box<dyn Trait>` | 16 B | 16 B |

The default owner spends 32 of its 40 bytes on payload, against 32 of 80
before. `AnyElement` no longer carries an inline buffer it could never use.

## Release profile

| Scenario | `Rubick` | `Box<dyn Node>` |
| --- | ---: | ---: |
| construct + drop, `Tiny` (inline) | **0.87** | 7.89 |
| construct + drop, `Medium` (pooled block) | **4.28** | 8.38 |
| construct + drop, `Medium` at widget capacity (inline) | **1.32** | 8.38 |
| construct + drop, `Large` (pooled block) | **4.91** | 11.05 |
| `replace` into a reused block | **2.53** | n/a |
| dispatch, `&self` method | 1.01 | **0.76** |
| dispatch, `&mut self` method | 1.14 | **0.78** |
| tree rebuild, 8192 nodes per frame | **3.93** | 12.03 |

Against the previous implementation, measured the same way:

| Scenario | Before | After |
| --- | ---: | ---: |
| construct + drop, `Medium` | 8.62 | **4.28** |
| construct + drop, `Large` | 10.07 | **4.91** |
| dispatch, `&self` method | 1.42 | **1.01** |
| tree rebuild, 8192 nodes per frame | 10.42 | **3.93** |

A full tree rebuild is **2.6x faster than before and 3.1x faster than
`Box`**. Construction of a heap-backed value is roughly twice as fast.

Dispatch remains slightly behind `Box`, and that is inherent: `Box` already
holds the fat pointer, while `Rubick` reconstructs it from a template plus the
payload address. The reconstruction is two instructions and no call — erasing
through `ErasedFrom` cut this from 1.42 to 1.01 by removing the adapter call
that the projection path needs.

### What each change contributes

Disabling the block pool alone, keeping every other change, gives:

| Scenario | Pool off | Pool on |
| --- | ---: | ---: |
| construct + drop, `Medium` | 8.37 | **4.28** |
| tree rebuild, 8192 nodes per frame | 10.34 | **3.93** |

So roughly half of the construction win and nearly all of the tree-rebuild win
come from recycling blocks; the rest comes from the smaller owner, the static
operation table and the direct projection.

The pool's per-class budget matters for the tree case: a frame frees thousands
of blocks and immediately asks for them back, so the free list has to be large
enough to hold a whole tree. At the initial 16 KB per class the tree rebuild
saw no benefit at all; at 512 KB it drops to 3.93.

## Dev profile

`aimer_rubick` is compiled with `opt-level = 3` in the dev profile — see the
comment on `[profile.dev.package.aimer_rubick]` in the workspace manifest.
Inline storage, static tables and a thread-local free list are thin
abstractions that only pay for themselves once inlined, and an application
still gets a fully unoptimized build of its own code.

Shipped dev configuration, which is what an application developer sees:

| Scenario | `Rubick` | `Box<dyn Node>` |
| --- | ---: | ---: |
| construct + drop, `Tiny` (inline) | **0.76** | 9.02 |
| construct + drop, `Medium` (pooled block) | **4.60** | 9.52 |
| construct + drop, `Large` (pooled block) | **4.78** | 11.04 |
| dispatch, `&self` method | 0.99 | **0.77** |
| tree rebuild, 8192 nodes per frame | **5.78** | 14.54 |

Without that profile override — every crate, including the benchmark itself,
at `opt-level = 0`:

| Scenario | `Rubick` | `Box<dyn Node>` |
| --- | ---: | ---: |
| construct + drop, `Tiny` (inline) | 28.28 | **22.59** |
| construct + drop, `Medium` (pooled block) | 75.95 | **30.62** |
| dispatch, `&self` method | 8.71 | **3.78** |
| tree rebuild, 8192 nodes per frame | 76.79 | **40.91** |

This is the honest cost of the design at `-O0`: the free list, the layout
bookkeeping and the pointer rebuild all become real calls, and macOS `malloc`
is fast enough to win. The previous implementation measured 65.99 on the same
tree rebuild, so the pool would have cost about 16% in a fully unoptimized
build — which is precisely why the profile override exists.
