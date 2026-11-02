# DRAFT: ASL syntax (expression-oriented)

Status: draft, implemented for the WGSL target (see `draft/DRAFT_ASL.md`, "Implementation status"). Source files use the `.aimer` extension. Companion to `draft/DRAFT_ASL.md` (language goals, backends, compiler layout). This file covers the surface syntax only.

The code samples are illustrative. They follow the helpers and structure found in `shaders/wgsl/*`, but they are not copied verbatim from those files.

Revision 2: declaration syntax now follows the friendlier prefix style of `draft/DRAFT_ASL.md` (`#[bind(0, 0)] uniform ..`, `#[input(step = instance)] struct`, `#[varying] struct`, `@position`, `@vertex_index` / `@frag_coord` / `@frag_color` builtin types). The tail-expression rule is kept and sharpened: the last expression of a block, written without `;`, is the block's value, so `return` is only needed for early exit (section 1.1). The `#[...]` attribute form is kept for the vertex input marker (`#[input(step = vertex | instance)]` on a struct), entry-point markers (`#[vertex]`, `#[fragment]`) and expression hints (`#[branch]`, `#[select]`).

Revision 3: `bind(g, b) uniform` and `varying struct` became attributes (`#[bind(g, b)]`, `#[varying]`), so every declaration-level marker is a `#[...]` attribute written on the line above the item. Builtin types keep the `@` prefix (`@vertex_index`, `@frag_coord`, `@frag_color`), as do varying field markers (`@position`, `@flat`, `@location(n)`).

Revision 4: `const` is always inlined in the generated code when its value is known at transpile time (new section 4.4). That is every `const`, since a `const` initialiser must be a compile-time expression. The only constant that keeps a declaration in the output is an array table indexed by a runtime value.

## 1. Principle: everything is an expression

As in Rust:

- A block `{ ... }` is an expression. Its value is the last expression, if that expression has no trailing `;`.
- `if`, `match`, `loop` and plain blocks are expressions and can appear anywhere an expression can.
- A function body is a block, so the tail expression is the return value. `return` is only for early exit.
- Statements are only: `let` bindings, items, and expressions ending in `;`. A `;` discards the value and makes the block's value `()`.
- Assignment (`=`, `+=`, ...) is an expression of type `()`. It is not usable as a value.

```
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { pow((c + 0.055) / 1.055, 2.4) }
}
```

No `return`, no temporary variable, no ternary operator. Each branch is a block whose value is its tail expression.

### 1.1 Tail expression rule

The last expression of a block, with no `;` after it, is the value of the block. That includes a function body:

```
fn sdf_box(p: vec2, half: vec2) -> f32 {
    let d = abs(p) - half;
    length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0)     // no `return`, no `;`
}
```

| Written | Block type | Meaning |
|---|---|---|
| `{ a; b }` | type of `b` | tail expression, `b` is the value |
| `{ a; b; }` | `()` | the `;` throws `b` away |
| `{ a; return b }` | `!` | `return` is itself an expression, legal as a tail without `;` |
| `{ a; return b; }` | `!` after the statement | legal, but the linter reports `redundant_return` at the end of a function |

Rules:

- A function `-> R` whose body ends in `;` is an error unless the body diverges: `fn f() -> f32 { 1.0; }` fails with "expected f32, found ()", and the message points at the `;`.
- `return e;` stays for early exit, usually in a guard: `if x <= 0.0 { return 1.0; }`. The body of such an `if` is a block ending in `;`, so it is `()`, as an `if` without `else` must be.
- The tail rule applies to every block, not only function bodies: `if` and `match` arms, `loop` bodies, and plain blocks.
- An entry point follows the same rule, so a vertex function ends with its output value (a struct literal, or the variable holding it, with no `;`).

```
fn clip_alpha(enabled: f32, d: f32) -> f32 {
    if enabled < 0.5 { return 1.0; }       // early exit, statement form
    1.0 - smoothstep(-0.5, 0.5, d)          // tail value
}
```

## 2. Lexical

| Element | Rule |
|---|---|
| Comments | `// line`, `/* block */` (nestable), `/// doc` |
| Identifiers | `[A-Za-z_][A-Za-z0-9_]*`. Reserved words of any target (`in`, `distance`, `sample`, `input`, ...) are legal in ASL and renamed by the emitter. |
| Float literals | `1.0`, `.5`, `1e-3`, `2.4f32`. Always `f32`. |
| Integer literals | `1` is `i32`, `1u` is `u32`, `0xFFu` hex. |
| Bool | `true`, `false` |
| Suffix | `f32`, `i32`, `u32`. A suffix is only needed to disambiguate. |

No implicit numeric conversion, as in Rust: `1 + 1.0` is an error, use `f32(1) + 1.0`. Two shader-specific exceptions, since all targets do them: scalar-with-vector arithmetic (`v * 2.0`) and splat construction (`vec2(0.5)`).

## 3. Types

```
f32  i32  u32  bool
vec2  vec3  vec4              // f32 vectors, short for vec2<f32>
vec2<i32>  vec3<u32>  ...     // other element types
mat2  mat3  mat4              // f32 column-major
[T; N]                        // fixed size array, N is a const expression
()                            // unit, the type of `;`-terminated blocks and assignments
!                             // never: type of `return`, `break`, `continue`
texture2d  sampler            // opaque resource types
```

Not in the language: tuples, references, generics, closures, traits, recursion. None of the current shaders need them, and WGSL, MSL, HLSL and GLSL differ too much on them.

Bool-vector results (`a < b` on vectors) exist, and `if` requires a scalar `bool`, so use `all(..)` or `any(..)`.

## 4. Items

### 4.1 Declarations

```
shader rect;                              // one per file: names the pipeline

import aimer::color::*;
import aimer::sdf::sdf_rounded_rect;

const SURFACE_ANDROID: f32 = 1.5;
const CORNERS: [vec2; 6] = [
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
];

uniform struct Viewport {                 // std140-compatible layout, checked by the compiler
    size: vec2,
    surface_is_srgb: f32,
    _pad: f32,
}

#[bind(0, 0)]
uniform viewport: Viewport;
#[bind(1, 0)]
texture t_diffuse: texture2d;
#[bind(1, 1)]
sampler s_diffuse;
```

Resources carry a `#[bind(group, binding)]` attribute on the line above the declaration. The keyword (`uniform`, `texture`, `sampler`) says what the resource is, and the attribute says where it lives. A resource without `#[bind]` is a compile error. `uniform struct` marks a struct that may be used as a uniform block, which turns on the std140 layout check.

### 4.2 Vertex inputs, varyings, outputs

```
#[input(step = instance)]
struct RectInstance {
    pos: vec2,
    size: vec2,
    color: vec4 as unorm8x4,     // format metadata, drives the MSL vertex descriptor
    border_radius: vec4,
}

#[varying]
struct RectVaryings {
    @position position: vec4,    // vertex-only builtin
    color: vec4,                 // location 0, assigned in declaration order
    local_pos: vec2,             // location 1
    @flat side_type: u32,        // integer varyings need `flat`; the emitter checks this
}
```

- Locations are auto-assigned in declaration order. `@location(n)` overrides.
- A `#[varying]` struct is both the vertex output and the fragment input, so a mismatch is a compile error.
- Field markers use `@`: `@position`, `@flat`, `@location(n)`. They are only legal on `varying` fields.

### 4.3 Functions and entry points

```
fn name(a: T, b: U) -> R { ... }          // R omitted means ()

#[vertex]
fn vs_main(vi: @vertex_index, inst: RectInstance) -> RectVaryings {
    let corner = CORNERS[vi];
    let pixel = inst.pos + corner * inst.size;
    let ndc = vec2(
        (pixel.x / viewport.size.x) * 2.0 - 1.0,
        1.0 - (pixel.y / viewport.size.y) * 2.0,
    );

    let mut out: RectVaryings;     // zero-initialised, see rule 2 in section 6
    out.position = vec4(ndc, 0.0, 1.0);
    out.color = inst.color;
    out.local_pos = corner * inst.size;
    out                            // tail expression: the value of the function
}

#[fragment]
fn fs_main(v: RectVaryings, frag: @frag_coord) -> @frag_color {   // @frag_color = location 0 output
    let d = sdf_rounded_rect(v.local_pos, v.rect_size, v.border_radius);
    if d > 0.5 { discard; }
    finalize_color(v.color, clip_alpha(frag.xy))
}
```

Builtin types are written with `@`: `@vertex_index` (`u32`), `@frag_coord` (`vec4`), `@frag_color` (`vec4`, location 0). The `@` is part of the type name, so `vi: @vertex_index` is a parameter of the builtin type, and user types can never clash with it. A vertex function may also end in a struct literal instead of a mutable `out`. Both forms produce the same output, and the literal needs every field:

```
    RectVaryings {
        position: vec4(ndc, 0.0, 1.0),
        color: inst.color,
        local_pos: corner * inst.size,
        rect_size: inst.size,
    }
```

Multiple render targets return a struct whose fields carry `@location(n)`.

Functions may read global resources (`viewport`) directly. The compiler threads them as hidden parameters for MSL.

### 4.4 Constants are inlined

```
const SURFACE_ANDROID: f32 = 1.5;
const SIDE_LAST: f32 = 7.5;
const HALF_SURFACE: f32 = SURFACE_ANDROID * 0.5;     // built from another const
const COUNT: u32 = 6u;
const CORNERS: [vec2; COUNT] = [ /* 6 entries */ ];
```

Rule: a `const` has no storage and no name in the output. The emitter replaces each use with the value, so the generated shader contains no `const` declaration for it (no WGSL `const`, no MSL file-scope `constant`, no HLSL `static const`, no GLSL `const`).

**What counts as known at transpile time.** A `const` initialiser may only use literals, other `const`s, operators, type constructors and casts (`vec2(0.0, 1.0)`, `f32(COUNT)`), array literals, and indexing or swizzling of those. It may not call user functions, read a resource or a `let`. The type checker rejects anything else, so every `const` is known and can be inlined.

| Use of a `const` | Output |
|---|---|
| Scalar or vector literal: `x >= SURFACE_ANDROID` | `x >= 1.5` |
| Const built from other consts: `HALF_SURFACE` | `(1.5 * 0.5)`, the initialiser inlined and parenthesised |
| Array size or loop bound: `[vec2; COUNT]`, `for i in 0u..COUNT` | the number: `array<vec2<f32>, 6>`, `i < 6u` |
| `match` arm pattern: `COUNT => ..` | the literal: `case 6u:` |
| Array table indexed by a constant: `CORNERS[2]` | the element expression: `vec2<f32>(0.0, 1.0)` |
| Array table indexed by a runtime value: `CORNERS[vi]` | not inlinable. The table is emitted once where it is used: a local `var` array in WGSL, GLSL and GLSL ES, a file-scope `constant` in MSL, `static const` in HLSL. |

No folding. Inlining is textual substitution of the initialiser, so `HALF_SURFACE` becomes `(1.5 * 0.5)` and the compiler never computes `0.75`. That keeps the "no hidden maths" rule: the emitters do not fold, reassociate or rewrite, so the byte-equality tests against the wgpu render stay valid. The parentheses keep precedence correct (`2.0 * HALF_SURFACE` is `2.0 * (1.5 * 0.5)`). A scalar literal initialiser is substituted as-is, without parentheses.

Literal spelling is fixed per type so output is stable: `f32` always has a decimal point (`1.0`, never `1`), `u32` has the `u` suffix (`6u`), `i32` has none, and negative values are parenthesised when used as an operand. WGSL abstract-numeric evaluation can differ from f32 arithmetic for expressions made only of literals, so the WGSL emitter types such an inlined expression (`f32(1.5 * 0.5)`) when the substitution has no other typed operand.

Benefits:

- The name never reaches the target, so reserved-word clashes (`distance`, `in`, `sample`) are impossible for consts.
- Constants work as GLSL ES 3.00 array sizes and `switch` labels without extra rules, and MSL and HLSL need no `constant` address-space handling for scalars.
- A `const` used only in an unreached function disappears with that function (reachability pass), and an unused `const` is never emitted.

To debug generated code, `--annotate-consts` appends the name as a comment next to each inlined value (`1.5 /* SURFACE_ANDROID */`). It is off by default so the default output is byte-stable.

## 5. Expressions

### 5.1 Blocks

```
let len = {
    let d = b - a;
    sqrt(dot(d, d))
};
```

Names declared inside a block are not visible outside. Shadowing with a new `let` is allowed, as in Rust.

### 5.2 `if`

```
let weight = if side < 1.5 { 0.0 } else if side < 2.5 { 1.0 } else { 2.0 };
```

- `if` without `else` has type `()`, so its body must be `()`.
- With `else`, both branches must have the same type, or one of them is `!`.
- The condition is a scalar `bool`, no parentheses needed, braces are mandatory.
- `if let` does not exist because there are no enums yet. See `match` below.

### 5.3 `match`

```
let w = match side_type {
    0u => 0.0,
    1u | 2u => 1.0,
    3u..=7u => 2.0,
    _ => 3.0,
};
```

- Scrutinee: integer or bool only. Float thresholds such as `side_type < 7.5` in the current shaders stay as `if / else if` chains.
- Arms are expressions, `_` is required unless the match is exhaustive over `bool`.
- No guards, no bindings, no fallthrough. Maps to `switch` in WGSL, MSL, HLSL and GLSL, and to an `if` chain when ranges are present.
- When enums arrive (later phase), `match` is the way to consume them. The current float-mode checks become `const` names first.

### 5.4 Loops

```
let sum = loop_sum(8u);

fn loop_sum(n: u32) -> f32 {
    let mut i = 0u;
    let mut acc = 0.0;
    loop {
        if i >= n { break acc; }      // `loop` is an expression: `break value` gives its value
        acc += weight(i);
        i += 1u;
    }
}

for i in 0..8 { acc += f(i); }        // bounds may be const or runtime values, `for` has type ()
while x > 0.0 { x -= step; }          // type ()
```

- `loop` has the type of its `break` values, or `!` if it never breaks.
- `while` and `for` always have type `()`. They cannot `break` with a value, same as Rust.
- `for` only iterates a range `a..b` or `a..=b`. No iterators.

### 5.5 Control transfer and `discard`

- `return e` has type `!`, so `let x = if c { 1.0 } else { return vec4(0.0) };` is legal.
- `break` and `continue` have type `!`.
- `discard` has type `()`, not `!`. WGSL `discard` (demote semantics) and MSL `discard_fragment()` do not stop execution, and the fragment keeps running as a helper invocation for derivatives. Typing it as `!` would let the compiler lie about reachability. Idiom:

```
if alpha <= 0.0 { discard; }
```

### 5.6 Operators

Same precedence and associativity as Rust, minus what does not exist here.

| Prec | Operators | Assoc |
|---|---|---|
| 1 | postfix: `f()`, `.field`, `.swizzle`, `[i]`, `.method()` | left |
| 2 | unary: `-` `!` `~` | right |
| 3 | `as` (explicit conversion) | left |
| 4 | `*` `/` `%` | left |
| 5 | `+` `-` | left |
| 6 | `<<` `>>` | left |
| 7 | `&` | left |
| 8 | `^` | left |
| 9 | `\|` | left |
| 10 | `==` `!=` `<` `>` `<=` `>=` (non-associative) | none |
| 11 | `&&` | left |
| 12 | `\|\|` | left |
| 13 | `..` `..=` (only in `for` and `match` patterns) | none |
| 14 | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` | right |

- Conversions: `f32(x)`, `u32(x)` as call-style constructors like the targets, plus `x as f32` for scalars. Bit reinterpretation is `bitcast<u32>(x)`, a separate intrinsic, never `as`.
- There is no `?:`. The `if` expression replaces it.
- Struct literals `Name { a: e, b: e }` are expressions. `..base` update syntax is allowed: `RectVaryings { color: c, ..base }`.
- Assignment targets: variables, fields, indices, and swizzles (`v.xy = e`, but not `v.xx = e`).

### 5.7 Method syntax

Methods exist only for resource types. Math stays as free functions, matching WGSL and GLSL:

```
let c = t_diffuse.sample(s_diffuse, uv);
let a = atlas.sample_level(atlas_sampler, uv, 0.0);
let p = frame.load(vec2<i32>(frag.xy), 0);
let n = backdrop.size();
let l = length(v);        // not v.length()
```

## 6. Rules the type checker enforces

1. Every block has exactly one type. `a; b; c` has the type `()`. `a; b; c` without the last `;` has the type of `c`.
2. `let mut x: T;` with no initialiser is allowed only for `let mut`, and means zero-initialised. WGSL's `var x: T;` zero-initialises but MSL, HLSL and GLSL do not, so the emitter writes an explicit zero value there (`T x = T{};` in MSL, `T x = (T)0;` in HLSL, `T x = T(0.0);` in GLSL). An immutable `let x: T;` is an error. Prefer `let x = <expr>;` whenever the value is known at the declaration.
3. `let` is immutable and `let mut` is mutable.
4. Every path in a function returning `R` must end in an `R` or in `!`.
5. Array indexing by a runtime value on a `const` table is allowed, and is the one case where the table is emitted as a declaration instead of being inlined (section 4.4). Out-of-range is undefined behaviour on some targets, so the compiler warns when it cannot prove the index is in range.
5a. A `const` initialiser must be a compile-time expression (section 4.4), so every `const` can be inlined. A cyclic `const` chain is an error.
6. Variables used in an `if` condition must be scalar `bool`.
7. Unused `let` and non-`()` expression statements (`f(x);` where `f` returns a value) are errors. Write `let _ = f(x);` to discard on purpose.

## 7. Lowering: expressions to statements

WGSL, MSL, HLSL and GLSL are statement-oriented: only the `?:` ternary (WGSL: `select`) exists as an expression-form conditional, and there are no block expressions. The compiler runs an A-normal-form pass: every sub-expression that contains control flow is hoisted into temporaries in the target, in order.

### 7.1 Evaluation order

ASL evaluates left to right, depth first, like Rust. Hoisting must preserve this. If a hoisted expression has a side effect or reads state that an earlier sibling mutates, the siblings before it are hoisted too.

```
let a = foo(x) + if c { bar(y) } else { 0.0 };
```

WGSL:

```
let _t0 = foo(x);
var _t1: f32;
if c { _t1 = bar(y); } else { _t1 = 0.0; }
let a = _t0 + _t1;
```

### 7.2 `if` expression: three strategies

Chosen per use, not per target, so all targets stay equivalent.

| Position | Lowering | Example |
|---|---|---|
| Function tail or `return` operand | `if { return a; } return b;` branches, no temp | `srgb_to_linear` |
| Initialiser, operand, or argument, where both arms are trivial | ternary or `select(b, a, c)` | `let k = if c { 1.0 } else { 2.0 };` |
| Anything else | `var _t: T;` plus an `if` statement assigning `_t` in each arm | arms that sample textures, call user functions, assign, or loop |

"Trivial" means: no user function calls, no resource access with implicit derivatives, no assignment, no loop, no `discard`, and no use of `/` or `%` on integers, so evaluating both arms is always safe. Only then is the select form chosen, because `select` and the MSL, HLSL and GLSL ternary may or may not short-circuit depending on the target and driver.

An attribute can force a strategy: `#[branch] if ...` always lowers to an `if` statement, and `#[select] if ...` requests `select` or `?:` and is a compile error if the arms are not trivial. This matters for the byte-identical goal in the main draft: the Metal tests compare against the wgpu output, and the existing files already use both forms.

### 7.3 Blocks, `match`, `loop`

| ASL | Lowering |
|---|---|
| Block expression in value position | hoist the block's statements, bind the tail to a temp |
| `match` in value position | `var _t: T; switch (x) { case 0u: { _t = ..; } ... default: { _t = ..; } }`. MSL and GLSL emit `break;` per case, WGSL does not. |
| `match` with ranges | an `if / else if` chain |
| `loop` with `break value` | `var _t: T; loop { ... { _t = value; break; } ... }`. `T` is inferred from the breaks, no annotation is needed. As the tail of a function it is followed by `return _t;`. |
| `return` or `break` in an operand position | the preceding hoisted statements run first, then the jump. The enclosing expression is dropped. |
| Struct literal | WGSL: constructor in declaration order. MSL, HLSL, GLSL: a local plus field assignments. Field order is normalised to declaration order after hoisting any non-trivial field in source order. |

### 7.4 `&&` and `||`

Source semantics are short-circuit. WGSL, MSL and GLSL short-circuit. HLSL historically does not, and uses `and()` and `or()` for non-short-circuit boolean ops. The HLSL emitter lowers `a && b` to a nested `if` into a temp when `b` is not trivial, so the semantics hold on every target.

### 7.5 Worked example

ASL:

```
fn finalize_color(color: vec4, alpha: f32) -> vec4 {
    let sc = if viewport.surface_is_srgb >= SURFACE_ANDROID {
        color
    } else {
        srgb_color_to_linear(color)
    };
    vec4(sc.rgb * sc.a, sc.a) * alpha
}
```

`SURFACE_ANDROID` is a `const`, so it is inlined as `1.5` and does not appear by name in any output.

WGSL (the `else` arm calls a user function, so it is not trivial):

```
fn finalize_color(color: vec4<f32>, alpha: f32) -> vec4<f32> {
    var sc: vec4<f32>;
    if viewport.surface_is_srgb >= 1.5 {
        sc = color;
    } else {
        sc = srgb_color_to_linear(color);
    }
    return vec4<f32>(sc.rgb * sc.a, sc.a) * alpha;
}
```

MSL (the global `viewport` becomes a parameter automatically):

```
float4 finalize_color(constant Viewport& viewport, float4 color, float alpha) {
    float4 sc;
    if (viewport.surface_is_srgb >= 1.5) {
        sc = color;
    } else {
        sc = srgb_color_to_linear(color);
    }
    return float4(sc.rgb * sc.a, sc.a) * alpha;
}
```

GLSL (3.30 and ES 3.00): the same as MSL, with `vec4`, and `viewport.surface_is_srgb` read from the uniform block.

## 8. Grammar sketch

```
file       = shader_decl? item*
shader_decl= "shader" IDENT ";"
item       = attr* ( import | const | struct | uniform | texture | sampler | fn )
import     = "import" path ("::" "*")? ";"
const      = "const" IDENT ":" type "=" expr ";"          // expr must be a compile-time expression, inlined at every use
struct     = attr* "uniform"? "struct" IDENT "{" (field_mark* IDENT ":" type ("as" IDENT)? ",")* "}"
                                       // vertex input struct: attr = #[input(step = vertex | instance)]
field_mark = "@" IDENT ("(" tokens ")")?
uniform    = attr* "uniform" IDENT ":" type ";"          // attr: #[bind(g, b)]
texture    = attr* "texture" IDENT ":" type ";"          // attr: #[bind(g, b)]
sampler    = attr* "sampler" IDENT ";"                   // attr: #[bind(g, b)]
builtin    = "@" IDENT                                    // a type: @vertex_index, @frag_coord, @frag_color
fn         = "fn" IDENT "(" (IDENT ":" type ",")* ")" ("->" type)? block     // attr here is #[vertex] / #[fragment]
attr       = "#[" path ("(" tokens ")")? "]"

block      = "{" stmt* expr? "}"                // the last expr has no ";" and is the value of the block
stmt       = "let" "mut"? IDENT (":" type)? "=" expr ";"
           | item
           | expr ";"
           | expr_with_block            // `if`, `match`, `loop`, block, no `;` required when value is ()
expr       = literal | path | struct_lit | array_lit | call | method | field
           | index | unary | binary | cast | assign
           | block | if | match | loop | while | for
           | "return" expr? | "break" expr? | "continue" | "discard"
if         = "if" expr block ("else" (if | block))?
match      = "match" expr "{" (pat ("|" pat)* "=>" expr ",")* "}"
loop       = "loop" block
while      = "while" expr block
for        = "for" IDENT "in" expr (".." | "..=") expr block
```

Same ambiguity rule as Rust: a struct literal is not allowed directly in an `if`, `match`, `while` or `for` header unless parenthesised.

## 9. Side-by-side with WGSL

| WGSL | ASL |
|---|---|
| `fn f(a: vec2<f32>) -> f32 { return a.x; }` | `fn f(a: vec2) -> f32 { a.x }` |
| `select(b, a, c)` (arguments reversed) | `if c { a } else { b }` |
| `var x: f32; if c { x = 1.0; } else { x = 2.0; }` | `let x = if c { 1.0 } else { 2.0 };` |
| `for (var i = 0u; i < n; i++) { ... }` | `for i in 0u..n { ... }` |
| `switch x { case 0u: { ... } default: { ... } }` | `match x { 0u => ..., _ => ... }` |
| `@group(0) @binding(0) var<uniform> v: Viewport;` | `#[bind(0, 0)] uniform v: Viewport;` |
| `@builtin(vertex_index) vi: u32` | `vi: @vertex_index` |
| `@location(1) uv: vec2<f32>` | field in a `#[varying]` struct, location is automatic |
| `let x = 1.0;` | `let x = 1.0;` |

## 10. Open questions

1. **`if` lowering default:** is "select only when both arms are trivial, otherwise `if` statement" acceptable, or should the default always be an `if` statement (literal, closest to the existing hand-written shaders) with `#[select]` as the opt-in?
2. **Derivatives in branches:** `sample` (implicit LOD) inside a non-uniform branch is rejected by WGSL validation. Should ASL check uniformity itself, or leave this to the backend validators and only document `sample_level`?
3. **Implicit literal typing:** keep Rust's strict rule (`1` is `i32`, `1.0` is `f32`, no implicit conversion), or follow WGSL's abstract-int rule where `1` converts to `f32` in a float context? Strict is simpler to emit, WGSL is more convenient for porting.
4. **`uniform struct`:** keep `uniform struct Viewport { .. }` plus `#[bind(0, 0)] uniform viewport: Viewport;` (two places say `uniform`), or let the binding line alone mark the struct as a uniform block?
5. **`match` scope:** is integer and bool only enough, or do you want float range patterns (`x if x < 7.5`-style guards) for the `side_type` chains in `rect` and `morph`?
6. **Unused values:** should `f(x);` on a value-returning function be an error (as above) or a warning?
7. **Const inlining and exactness:** is textual inlining without folding (`HALF_SURFACE` becomes `(1.5 * 0.5)`) acceptable, or should the compiler fold const expressions to one literal? Folding gives shorter output but breaks the "no hidden maths" rule if the compiler's f32 arithmetic ever differs from the GPU's.
8. **Large tables:** inlining a constant index into a big array copies the element expression at each use. Is that fine, or should repeated uses of one non-trivial element share a local?
