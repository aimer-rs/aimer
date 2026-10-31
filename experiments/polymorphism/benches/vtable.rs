//! Benchmark: compiler-generated vtable (`Box<dyn Trait>`) vs manual vtables.
//! Run with `cargo bench`.

use std::hint::black_box;
use std::time::Instant;

// ---------- the trait under test ----------
pub trait NonDyn: Sized {
    fn hello();
    fn work(&self) -> u64;
}

macro_rules! kinds {
    ($($name:ident($mul:expr)),*) => {$(
        struct $name(u64);
        impl NonDyn for $name {
            fn hello() {}
            #[inline(never)]
            fn work(&self) -> u64 { self.0.wrapping_mul($mul).rotate_left(5) }
        }
    )*};
}
kinds!(A(3), B(5), C(7), D(11));

// ---------- 1. compiler-generated vtable ----------
trait Erased {
    fn work(&self) -> u64;
}
impl<T: NonDyn> Erased for T {
    fn work(&self) -> u64 {
        NonDyn::work(self)
    }
}
struct DynObj(Box<dyn Erased>);
impl DynObj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        Self(Box::new(v))
    }
    #[inline]
    fn work(&self) -> u64 {
        self.0.work()
    }
}

// ---------- 2. manual vtable, fn pointers stored inline (32 bytes) ----------
struct InlineObj {
    data: *mut (),
    work: unsafe fn(*const ()) -> u64,
    drop: unsafe fn(*mut ()),
    _hello: fn(),
}
impl InlineObj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        unsafe fn work<T: NonDyn>(p: *const ()) -> u64 {
            unsafe { (*(p as *const T)).work() }
        }
        unsafe fn drop_data<T>(p: *mut ()) {
            unsafe { drop(Box::from_raw(p as *mut T)) }
        }
        Self {
            data: Box::into_raw(Box::new(v)) as *mut (),
            work: work::<T>,
            drop: drop_data::<T>,
            _hello: T::hello,
        }
    }
    #[inline]
    fn work(&self) -> u64 {
        unsafe { (self.work)(self.data) }
    }
}
impl Drop for InlineObj {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

// ---------- 3. manual vtable, shared static table (16 bytes, like dyn) ----------
struct VTable {
    work: unsafe fn(*const ()) -> u64,
    drop: unsafe fn(*mut ()),
    _hello: fn(),
}
struct Table<T>(std::marker::PhantomData<T>);
impl<T: NonDyn + 'static> Table<T> {
    const VT: VTable = VTable {
        work: {
            unsafe fn work<T: NonDyn>(p: *const ()) -> u64 {
                unsafe { (*(p as *const T)).work() }
            }
            work::<T>
        },
        drop: {
            unsafe fn drop_data<T>(p: *mut ()) {
                unsafe { drop(Box::from_raw(p as *mut T)) }
            }
            drop_data::<T>
        },
        _hello: T::hello,
    };
}
struct TableObj {
    data: *mut (),
    vt: &'static VTable,
}
impl TableObj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        Self {
            data: Box::into_raw(Box::new(v)) as *mut (),
            vt: &Table::<T>::VT,
        }
    }
    #[inline]
    fn work(&self) -> u64 {
        unsafe { (self.vt.work)(self.data) }
    }
}
impl Drop for TableObj {
    fn drop(&mut self) {
        unsafe { (self.vt.drop)(self.data) }
    }
}

// ---------- 4. optimized inline vtable (24 bytes, no unused fields, forced inlining) ----------
struct Inline2Obj {
    data: *mut (),
    work: unsafe fn(*const ()) -> u64,
    drop: unsafe fn(*mut ()),
}
impl Inline2Obj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        #[inline(always)]
        unsafe fn work<T: NonDyn>(p: *const ()) -> u64 {
            unsafe { (*(p as *const T)).work() }
        }
        #[inline(always)]
        unsafe fn drop_data<T>(p: *mut ()) {
            unsafe { drop(Box::from_raw(p as *mut T)) }
        }
        Self {
            data: Box::into_raw(Box::new(v)) as *mut (),
            work: work::<T>,
            drop: drop_data::<T>,
        }
    }
    #[inline(always)]
    fn work(&self) -> u64 {
        unsafe { (self.work)(self.data) }
    }
}
impl Drop for Inline2Obj {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

// ---------- 4b. inline v3: fn pointer is T::work itself (no forwarding wrapper) ----------
// `fn(&T) -> u64` and `unsafe fn(*const ()) -> u64` share one ABI in practice,
// but transmuting between them is not guaranteed by the language (experiment only).
struct Inline3Obj {
    data: *mut (),
    work: unsafe fn(*const ()) -> u64,
    drop: unsafe fn(*mut ()),
}
impl Inline3Obj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        unsafe fn drop_data<T>(p: *mut ()) {
            unsafe { drop(Box::from_raw(p as *mut T)) }
        }
        let work: fn(&T) -> u64 = <T as NonDyn>::work;
        Self {
            data: Box::into_raw(Box::new(v)) as *mut (),
            work: unsafe { std::mem::transmute::<fn(&T) -> u64, unsafe fn(*const ()) -> u64>(work) },
            drop: drop_data::<T>,
        }
    }
    #[inline(always)]
    fn work(&self) -> u64 {
        unsafe { (self.work)(self.data) }
    }
}
impl Drop for Inline3Obj {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

// ---------- 5. thin object: vtable header lives in the same heap allocation as the data (8 bytes) ----------
#[repr(C)]
struct Header {
    work: unsafe fn(*const Header) -> u64,
    drop: unsafe fn(*mut Header),
}
#[repr(C)]
struct Cell<T> {
    h: Header,
    v: T,
}
struct ThinObj(std::ptr::NonNull<Header>);
impl ThinObj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        unsafe fn work<T: NonDyn>(p: *const Header) -> u64 {
            unsafe { (*(p as *const Cell<T>)).v.work() }
        }
        unsafe fn drop_cell<T>(p: *mut Header) {
            unsafe { drop(Box::from_raw(p as *mut Cell<T>)) }
        }
        let b = Box::new(Cell {
            h: Header {
                work: work::<T>,
                drop: drop_cell::<T>,
            },
            v,
        });
        Self(unsafe { std::ptr::NonNull::new_unchecked(Box::into_raw(b) as *mut Header) })
    }
    #[inline(always)]
    fn work(&self) -> u64 {
        let p = self.0.as_ptr();
        unsafe { ((*p).work)(p) }
    }
}
impl Drop for ThinObj {
    fn drop(&mut self) {
        let p = self.0.as_ptr();
        unsafe { ((*p).drop)(p) }
    }
}

// ---------- 6. NEW: inline-payload object (small-buffer optimization, 32 bytes) ----------
// The value lives inside the object, so a call needs no pointer chase to the heap and
// a Vec<SboObj> is one contiguous block. The fn pointer sits next to the data (same cache line).
struct SboObj {
    work: unsafe fn(*const u8) -> u64,
    drop: unsafe fn(*mut u8),
    buf: std::mem::MaybeUninit<[u64; 2]>,
}
impl SboObj {
    fn new<T: NonDyn + 'static>(v: T) -> Self {
        const {
            assert!(std::mem::size_of::<T>() <= 16 && std::mem::align_of::<T>() <= 8);
        }
        unsafe fn work<T: NonDyn>(p: *const u8) -> u64 {
            unsafe { (*(p as *const T)).work() }
        }
        unsafe fn drop_data<T>(p: *mut u8) {
            unsafe { std::ptr::drop_in_place(p as *mut T) }
        }
        let mut buf = std::mem::MaybeUninit::<[u64; 2]>::uninit();
        unsafe { (buf.as_mut_ptr() as *mut T).write(v) };
        Self {
            work: work::<T>,
            drop: drop_data::<T>,
            buf,
        }
    }
    #[inline(always)]
    fn work(&self) -> u64 {
        unsafe { (self.work)(self.buf.as_ptr() as *const u8) }
    }
}
impl Drop for SboObj {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.buf.as_mut_ptr() as *mut u8) }
    }
}

// ---------- 7. NEW: bucketed (amortized) vtable ----------
// Objects are grouped by concrete type into contiguous Vec<T>. The vtable is consulted once
// per *bucket*; inside the bucket the loop is a direct, monomorphized, predictable call
// (and can be inlined/vectorized). Cost per object: no vtable load, no indirect branch,
// no per-object pointer. Trade-off: iteration order across types is not preserved, so it
// only fits order-independent work (sums, updates, ...).
struct Bucket {
    id: std::any::TypeId,
    vec: *mut (),
    run: unsafe fn(*const ()) -> u64,
    drop: unsafe fn(*mut ()),
}
struct Buckets(Vec<Bucket>);
impl Buckets {
    fn push<T: NonDyn + 'static>(&mut self, v: T) {
        unsafe fn run<T: NonDyn>(p: *const ()) -> u64 {
            unsafe {
                (*(p as *const Vec<T>))
                    .iter()
                    .fold(0u64, |a, x| a.wrapping_add(x.work()))
            }
        }
        unsafe fn drop_vec<T>(p: *mut ()) {
            unsafe { drop(Box::from_raw(p as *mut Vec<T>)) }
        }
        let id = std::any::TypeId::of::<T>();
        let idx = match self.0.iter().position(|b| b.id == id) {
            Some(i) => i,
            None => {
                self.0.push(Bucket {
                    id,
                    vec: Box::into_raw(Box::new(Vec::<T>::new())) as *mut (),
                    run: run::<T>,
                    drop: drop_vec::<T>,
                });
                self.0.len() - 1
            }
        };
        unsafe { (*(self.0[idx].vec as *mut Vec<T>)).push(v) }
    }
    /// Run `work` on every object; returns the wrapping sum.
    #[inline]
    fn run_all(&self) -> u64 {
        let mut acc = 0u64;
        for b in &self.0 {
            acc = acc.wrapping_add(unsafe { (b.run)(b.vec) });
        }
        acc
    }
}
impl Drop for Buckets {
    fn drop(&mut self) {
        for b in &self.0 {
            unsafe { (b.drop)(b.vec) }
        }
    }
}

// ---------- 8. NEW: packed polymorphic arena (order-preserving, no per-object heap pointer) ----------
// All objects, whatever their concrete type, are laid out back-to-back in ONE buffer:
//   [ &'static PVt (8B) | payload (size_of::<T> rounded up to 8B) ] [ next record ] ...
// Iteration walks the buffer linearly (the hardware prefetcher loves it) and the stride of
// each record comes from its vtable, i.e. this is a `Vec<dyn Trait>` that Rust does not have.
// Per object: no 16B fat pointer, no heap allocation, no pointer chase, insertion order kept.
struct PVt {
    work: unsafe fn(*const u8) -> u64,
    drop: unsafe fn(*mut u8),
    words: usize, // record length in u64 words, header included
}
struct PTab<T>(std::marker::PhantomData<T>);
impl<T: NonDyn + 'static> PTab<T> {
    const VT: PVt = PVt {
        work: {
            unsafe fn work<T: NonDyn>(p: *const u8) -> u64 {
                unsafe { (*(p as *const T)).work() }
            }
            work::<T>
        },
        drop: {
            unsafe fn drop_data<T>(p: *mut u8) {
                unsafe { std::ptr::drop_in_place(p as *mut T) }
            }
            drop_data::<T>
        },
        words: 1 + (std::mem::size_of::<T>() + 7) / 8,
    };
}
struct PackedArena {
    buf: Vec<u64>,
}
impl PackedArena {
    fn push<T: NonDyn + 'static>(&mut self, v: T) {
        const {
            assert!(std::mem::align_of::<T>() <= 8);
        }
        let vt: &'static PVt = &PTab::<T>::VT;
        let start = self.buf.len();
        self.buf.resize(start + vt.words, 0);
        unsafe {
            let p = self.buf.as_mut_ptr().add(start);
            (p as *mut &'static PVt).write(vt);
            (p.add(1) as *mut T).write(v);
        }
    }
    #[inline]
    fn run_all(&self) -> u64 {
        let base = self.buf.as_ptr();
        let end = self.buf.len();
        let (mut i, mut acc) = (0usize, 0u64);
        while i < end {
            unsafe {
                let p = base.add(i);
                let vt = *(p as *const &'static PVt);
                acc = acc.wrapping_add((vt.work)(p.add(1) as *const u8));
                i += vt.words;
            }
        }
        acc
    }
}
impl Drop for PackedArena {
    fn drop(&mut self) {
        let base = self.buf.as_mut_ptr();
        let end = self.buf.len();
        let mut i = 0usize;
        while i < end {
            unsafe {
                let p = base.add(i);
                let vt = *(p as *const &'static PVt);
                (vt.drop)(p.add(1) as *mut u8);
                i += vt.words;
            }
        }
    }
}

// ---------- 9. NEW: tagged slabs (order-preserving structure-of-arrays) ----------
// Values of one concrete type live in one contiguous Vec<T> (a "slab"). A 1-byte tag per
// object records which slab comes next, so insertion order is preserved. Iteration reads the
// tag stream + sequential slab streams: per object only 1 byte of bookkeeping.
struct Slab {
    id: std::any::TypeId,
    vec: *mut (),
    at: unsafe fn(*const (), usize) -> u64,
    // order-preserving batch map: element k of this slab writes its result to out[idx[k]]
    map: unsafe fn(*const (), &[u32], &mut [u64]),
    idx: Vec<u32>,
    drop: unsafe fn(*mut ()),
}
struct SlabSet {
    tags: Vec<u8>,
    slabs: Vec<Slab>,
}
const MAX_SLABS: usize = 16;
impl SlabSet {
    fn push<T: NonDyn + 'static>(&mut self, v: T) {
        unsafe fn at<T: NonDyn>(p: *const (), i: usize) -> u64 {
            unsafe { (&*(p as *const Vec<T>)).get_unchecked(i).work() }
        }
        unsafe fn map<T: NonDyn>(p: *const (), idx: &[u32], out: &mut [u64]) {
            let v = unsafe { &*(p as *const Vec<T>) };
            for (x, &i) in v.iter().zip(idx) {
                out[i as usize] = x.work();
            }
        }
        unsafe fn drop_vec<T>(p: *mut ()) {
            unsafe { drop(Box::from_raw(p as *mut Vec<T>)) }
        }
        let id = std::any::TypeId::of::<T>();
        let idx = match self.slabs.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => {
                assert!(self.slabs.len() < MAX_SLABS);
                self.slabs.push(Slab {
                    id,
                    vec: Box::into_raw(Box::new(Vec::<T>::new())) as *mut (),
                    at: at::<T>,
                    map: map::<T>,
                    idx: Vec::new(),
                    drop: drop_vec::<T>,
                });
                self.slabs.len() - 1
            }
        };
        self.slabs[idx].idx.push(self.tags.len() as u32);
        unsafe { (*(self.slabs[idx].vec as *mut Vec<T>)).push(v) };
        self.tags.push(idx as u8);
    }
    /// Order-preserving map: out[i] = work(i-th inserted object). One indirect call per
    /// *slab*, then a direct, inlinable loop per slab that scatters into `out`.
    #[inline]
    fn map_into(&self, out: &mut [u64]) {
        for s in &self.slabs {
            unsafe { (s.map)(s.vec, &s.idx, out) }
        }
    }
    #[inline]
    fn run_all(&self) -> u64 {
        let mut cur = [0usize; MAX_SLABS];
        let mut acc = 0u64;
        for &t in &self.tags {
            let t = t as usize;
            let s = &self.slabs[t];
            acc = acc.wrapping_add(unsafe { (s.at)(s.vec, cur[t]) });
            cur[t] += 1;
        }
        acc
    }
}
impl Drop for SlabSet {
    fn drop(&mut self) {
        for s in &self.slabs {
            unsafe { (s.drop)(s.vec) }
        }
    }
}

// ---------- harness ----------
/// Same pseudo-random type sequence as `build`, handed to a push callback.
fn fill(n: usize, mut push: impl FnMut(usize, u64)) {
    let mut s = 0x9E3779B97F4A7C15u64;
    for i in 0..n {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        push((s >> 33) as usize % 4, i as u64);
    }
}

fn build_arena(n: usize) -> PackedArena {
    let mut a = PackedArena { buf: Vec::new() };
    fill(n, |k, v| match k {
        0 => a.push(A(v)),
        1 => a.push(B(v)),
        2 => a.push(C(v)),
        _ => a.push(D(v)),
    });
    a
}

fn build_slabs(n: usize) -> SlabSet {
    let mut a = SlabSet {
        tags: Vec::new(),
        slabs: Vec::new(),
    };
    fill(n, |k, v| match k {
        0 => a.push(A(v)),
        1 => a.push(B(v)),
        2 => a.push(C(v)),
        _ => a.push(D(v)),
    });
    a
}

/// Time a whole-collection pass `f` and report ns per object.
fn bench_pass(label: &str, n: usize, iters: usize, mut f: impl FnMut() -> u64) -> f64 {
    let mut acc = 0u64;
    for _ in 0..iters / 10 + 1 {
        acc = acc.wrapping_add(f());
    }
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        for _ in 0..iters {
            acc = acc.wrapping_add(f());
        }
        best = best.min(t.elapsed().as_nanos() as f64 / (iters * n) as f64);
    }
    black_box(acc);
    println!("  {label:<28} {best:>7.3} ns/call");
    best
}

fn build<O>(n: usize, mk: impl Fn(usize, u64) -> O) -> Vec<O> {
    // pseudo-random type order so the branch predictor / devirtualizer can't cheat
    let mut s = 0x9E3779B97F4A7C15u64;
    (0..n)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            mk((s >> 33) as usize % 4, i as u64)
        })
        .collect()
}

fn bench<O>(label: &str, objs: &[O], f: impl Fn(&O) -> u64, iters: usize) -> f64 {
    // warm up
    let mut acc = 0u64;
    for _ in 0..iters / 10 + 1 {
        for o in objs {
            acc = acc.wrapping_add(f(black_box(o)));
        }
    }
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        for _ in 0..iters {
            for o in objs {
                acc = acc.wrapping_add(f(black_box(o)));
            }
        }
        let ns = t.elapsed().as_nanos() as f64 / (iters * objs.len()) as f64;
        best = best.min(ns);
    }
    black_box(acc);
    println!("  {label:<28} {best:>7.3} ns/call");
    best
}

fn build_buckets(n: usize) -> Buckets {
    // same pseudo-random sequence as `build`
    let mut s = 0x9E3779B97F4A7C15u64;
    let mut b = Buckets(Vec::new());
    for i in 0..n {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let v = i as u64;
        match (s >> 33) as usize % 4 {
            0 => b.push(A(v)),
            1 => b.push(B(v)),
            2 => b.push(C(v)),
            _ => b.push(D(v)),
        }
    }
    b
}

fn bench_buckets(label: &str, b: &Buckets, n: usize, iters: usize) -> f64 {
    let mut acc = 0u64;
    for _ in 0..iters / 10 + 1 {
        acc = acc.wrapping_add(black_box(b).run_all());
    }
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        for _ in 0..iters {
            acc = acc.wrapping_add(black_box(b).run_all());
        }
        best = best.min(t.elapsed().as_nanos() as f64 / (iters * n) as f64);
    }
    black_box(acc);
    println!("  {label:<28} {best:>7.3} ns/call");
    best
}

fn main() {
    println!(
        "sizes: Box<dyn>={}B  inline-vtable={}B  shared-vtable={}B  inline-v2={}B  thin={}B  sbo={}B\n",
        std::mem::size_of::<DynObj>(),
        std::mem::size_of::<InlineObj>(),
        std::mem::size_of::<TableObj>(),
        std::mem::size_of::<Inline2Obj>(),
        std::mem::size_of::<ThinObj>(),
        std::mem::size_of::<SboObj>()
    );
    println!(
        "arena record (A..D) = {}B/object, slab = 8B payload + 1B tag\n",
        PTab::<A>::VT.words * 8
    );

    for &(n, iters) in &[(16usize, 2_000_000usize), (1_000, 20_000), (1_000_000, 20)] {
        println!("{n} objects, mixed concrete types:");
        let d = build(n, |k, v| match k {
            0 => DynObj::new(A(v)),
            1 => DynObj::new(B(v)),
            2 => DynObj::new(C(v)),
            _ => DynObj::new(D(v)),
        });
        let i = build(n, |k, v| match k {
            0 => InlineObj::new(A(v)),
            1 => InlineObj::new(B(v)),
            2 => InlineObj::new(C(v)),
            _ => InlineObj::new(D(v)),
        });
        let t = build(n, |k, v| match k {
            0 => TableObj::new(A(v)),
            1 => TableObj::new(B(v)),
            2 => TableObj::new(C(v)),
            _ => TableObj::new(D(v)),
        });
        let i2 = build(n, |k, v| match k {
            0 => Inline2Obj::new(A(v)),
            1 => Inline2Obj::new(B(v)),
            2 => Inline2Obj::new(C(v)),
            _ => Inline2Obj::new(D(v)),
        });
        let i3 = build(n, |k, v| match k {
            0 => Inline3Obj::new(A(v)),
            1 => Inline3Obj::new(B(v)),
            2 => Inline3Obj::new(C(v)),
            _ => Inline3Obj::new(D(v)),
        });
        let th = build(n, |k, v| match k {
            0 => ThinObj::new(A(v)),
            1 => ThinObj::new(B(v)),
            2 => ThinObj::new(C(v)),
            _ => ThinObj::new(D(v)),
        });
        let sbo = build(n, |k, v| match k {
            0 => SboObj::new(A(v)),
            1 => SboObj::new(B(v)),
            2 => SboObj::new(C(v)),
            _ => SboObj::new(D(v)),
        });
        let bk = build_buckets(n);
        // correctness: every variant must compute the same checksum
        let sum_d = d.iter().fold(0u64, |a, o| a.wrapping_add(o.work()));
        assert_eq!(sum_d, sbo.iter().fold(0u64, |a, o| a.wrapping_add(o.work())));
        assert_eq!(sum_d, bk.run_all());
        let ar = build_arena(n);
        let sl = build_slabs(n);
        assert_eq!(sum_d, ar.run_all());
        assert_eq!(sum_d, sl.run_all());
        // order-preserving map (out[i] = work(object i)): the case a bucketed vtable can't serve
        let mut out_d = vec![0u64; n];
        let mut out_s = vec![0u64; n];
        for (o, x) in d.iter().zip(out_d.iter_mut()) {
            *x = o.work();
        }
        sl.map_into(&mut out_s);
        assert_eq!(out_d, out_s);
        let rd = bench("compiler vtable (Box<dyn>)", &d, |o| o.work(), iters);
        let ri = bench("manual, inline fn ptrs", &i, |o| o.work(), iters);
        let rt = bench("manual, shared static vtable", &t, |o| o.work(), iters);
        let r2 = bench("manual, inline v2 (24B)", &i2, |o| o.work(), iters);
        let r3 = bench("manual, inline v3 (no wrapper)", &i3, |o| o.work(), iters);
        let rh = bench("manual, thin header (8B)", &th, |o| o.work(), iters);
        let rs = bench("NEW inline payload (SBO)", &sbo, |o| o.work(), iters);
        let rb = bench_buckets("NEW bucketed vtable", &bk, n, iters);
        let ra = bench_pass("NEW packed arena", n, iters, || black_box(&ar).run_all());
        let rl = bench_pass("NEW tagged slabs", n, iters, || black_box(&sl).run_all());
        println!("  -- order-preserving map into out[i] --");
        let md = bench_pass("map: Box<dyn> + enumerate", n, iters, || {
            for (o, x) in black_box(&d).iter().zip(out_d.iter_mut()) {
                *x = o.work();
            }
            black_box(&out_d)[n / 2]
        });
        let ms = bench_pass("map: NEW slab scatter", n, iters, || {
            black_box(&sl).map_into(&mut out_s);
            black_box(&out_s)[n / 2]
        });
        println!("  -> slab-scatter map / dyn map = {:.2}x", ms / md);
        println!(
            "  -> inline/dyn = {:.2}x, shared/dyn = {:.2}x, v2/dyn = {:.2}x, v3/dyn = {:.2}x, thin/dyn = {:.2}x, sbo/dyn = {:.2}x, bucketed/dyn = {:.2}x, arena/dyn = {:.2}x, slabs/dyn = {:.2}x\n",
            ri / rd,
            rt / rd,
            r2 / rd,
            r3 / rd,
            rh / rd,
            rs / rd,
            rb / rd,
            ra / rd,
            rl / rd
        );
    }

    // single object, same concrete type: best case for the CPU predictor
    println!("1 object, one concrete type (hot loop):");
    let n = 100_000_000u64;
    let d = DynObj::new(A(1));
    let i = InlineObj::new(A(1));
    let t = TableObj::new(A(1));
    let i2 = Inline2Obj::new(A(1));
    let i3 = Inline3Obj::new(A(1));
    let th = ThinObj::new(A(1));
    let sbo = SboObj::new(A(1));
    let mut bk = Buckets(Vec::new());
    bk.push(A(1));
    let mut ar = PackedArena { buf: Vec::new() };
    ar.push(A(1));
    let mut sl = SlabSet {
        tags: Vec::new(),
        slabs: Vec::new(),
    };
    sl.push(A(1));
    macro_rules! hot {
        ($label:expr, $o:expr) => {{
            let mut best = f64::MAX;
            let mut acc = 0u64;
            for _ in 0..5 {
                let s = Instant::now();
                for _ in 0..n {
                    acc = acc.wrapping_add(black_box(&$o).work());
                }
                best = best.min(s.elapsed().as_nanos() as f64 / n as f64);
            }
            black_box(acc);
            println!("  {:<28} {:>7.3} ns/call", $label, best);
        }};
    }
    hot!("compiler vtable (Box<dyn>)", d);
    hot!("manual, inline fn ptrs", i);
    hot!("manual, shared static vtable", t);
    hot!("manual, inline v2 (24B)", i2);
    hot!("manual, inline v3 (no wrapper)", i3);
    hot!("manual, thin header (8B)", th);
    hot!("NEW inline payload (SBO)", sbo);
    {
        let mut best = f64::MAX;
        let mut acc = 0u64;
        for _ in 0..5 {
            let s = Instant::now();
            for _ in 0..n {
                acc = acc.wrapping_add(black_box(&bk).run_all());
            }
            best = best.min(s.elapsed().as_nanos() as f64 / n as f64);
        }
        black_box(acc);
        println!("  {:<28} {:>7.3} ns/call", "NEW bucketed vtable", best);
    }
    macro_rules! hot_all {
        ($label:expr, $o:expr) => {{
            let mut best = f64::MAX;
            let mut acc = 0u64;
            for _ in 0..5 {
                let s = Instant::now();
                for _ in 0..n {
                    acc = acc.wrapping_add(black_box(&$o).run_all());
                }
                best = best.min(s.elapsed().as_nanos() as f64 / n as f64);
            }
            black_box(acc);
            println!("  {:<28} {:>7.3} ns/call", $label, best);
        }};
    }
    hot_all!("NEW packed arena", ar);
    hot_all!("NEW tagged slabs", sl);
}
