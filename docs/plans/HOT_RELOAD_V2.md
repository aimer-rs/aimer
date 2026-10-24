# Aimer Hot Reload V2: Source-Aware Literal and Property Patching

> This document is a proposal. V1 remains the compatibility and correctness
> path until every V2 acceptance criterion passes.

## Summary

V1 treats every portable guest source change as a new guest generation:

```text
source change
  -> regenerate the automatic shadow
  -> compile a new WASM guest
  -> upload the complete module
  -> materialize and commit a candidate tree
```

V2 adds a narrower fast path for edits whose proven semantic effect is one
bounded property-value replacement. The CLI may then send a small, authenticated
property patch instead of regenerating the shadow, compiling WASM, and uploading
the complete module.

The fast path must be conservative. If the CLI cannot prove that the edit is a
single supported literal/property change, it falls back to V1. V2 must never
guess the meaning of arbitrary Rust, macros, loops, conditionals, helper calls,
or context-dependent expressions.

## Example: changing a button's width

The application source might contain a `SizedBox` around a button:

```rust,ignore
SizedBox::new()
    .width(100.0)
    .height(40.0)
    .child(Button::new().child(Text::new("Save")))
```

The developer changes only the width literal:

```rust,ignore
SizedBox::new()
    .width(120.0)
    .height(40.0)
    .child(Button::new().child(Text::new("Save")))
```

For a V2 patch, the compiler or derive-generated metadata must have recorded a
descriptor similar to:

```text
source file:       src/ui/save_button.rs
source span:       width argument
source anchor:     <stable source-site identity>
instance ID:       <stable SizedBox instance identity>
widget schema:     SizedBox 1.0
property:          width
value codec:       finite pixel dimension encoded as F64
previous value:    100.0
```

The CLI then sends a payload like:

```text
kind:               property-patch
protocol version:   1
base generation:    3
application ID:     <stable application identity>
instance ID:        <stable SizedBox instance identity>
widget schema:      SizedBox 1.0
property ID:        <SizedBox.width>
previous fingerprint:<fingerprint of 100.0>
new value:          F64(120.0)
```

The host validates the base generation, instance, schema, property, previous
value, and value limits. At the normal reload safe point it changes the width,
invalidates the affected layout, redraws, and commits generation 4 while
retaining state and callback registrations.

No shadow regeneration, guest compilation, or full WASM upload is needed for
this case.

If the width is computed dynamically, the `SizedBox` is created inside an
ambiguous loop, the node identity changed, the property codec is unsupported,
or the old value no longer matches the running generation, V2 must reject the
fast path and run the normal V1 rebuild.

## Current V1 foundations

The repository already provides important pieces of the eventual contract:

| Existing foundation | Current role | V2 reuse |
| --- | --- | --- |
| `WidgetSchemaId` and `PropertyId` | Identify widget types and schema fields | Identify the patch target's type and field |
| `WidgetProperty` and `PropertyValue` | Carry validated typed AWIR properties | Carry the typed patch value |
| `StableSlotId` / node key | Identify one generated node in a document | Address one widget instance |
| AWIR schema validation | Check property types, limits, and versions | Validate patch values with the same rules |
| reload generations and safe points | Stage, commit, reject, and recover a complete module | Apply or reject one patch atomically |
| native materializers | Build `SizedBox`, `Text`, `Container`, and other widgets from AWIR | Provide property-specific mutation adapters |
| authenticated reload transport | Transfer a complete module securely | Authenticate patch payloads as well |

The current V1 path still uploads complete module bytes. `ModuleMetadata` binds
the request to a complete module identity, and the CLI's `push_reload` interface
accepts `module: &[u8]`; it has no property-patch variant. The host materializer
also constructs a disconnected tree from a complete Widget IR document rather
than exposing a live property mutation interface.

## Identity model

V2 must distinguish three identities:

1. **Schema identity** — for example, `SizedBox`. This identifies the widget
   type, not one instance.
2. **Instance identity** — the stable node key used to find one `SizedBox` in
   the active tree. A document-local node index is not sufficient because node
   ordering can change.
3. **Property identity** — for example, `SizedBox.width`. This identifies the
   field and its value codec.

The current `slot_for` implementation derives an unkeyed slot from a source
fingerprint and a keyed slot from an explicit key. That distinction is useful,
but the current expression fingerprint is not automatically a V2-safe patch
identity: the existing tests intentionally show that changing a property
literal changes the expression fingerprint. A patch target must therefore use
one of these strategies:

- require an explicit stable key for the first patchable widgets;
- derive a source-site identity that excludes mutable property literals and is
  stable under formatting changes; or
- introduce a compiler-generated source anchor with a documented stability
  policy for moved, duplicated, and repeated nodes.

Explicit keys are the safest first implementation. Automatically generated
anchors may be added later, but the rules must be deterministic and must reject
ambiguous matches rather than selecting one by accident.

## Requirements

### Functional requirements

1. **Exact-edit proof**

   V2 may activate only when the old and new source match one metadata
   descriptor exactly and the semantic change is one supported property value.

2. **Stable target**

   The patch must address a stable instance identity, schema identity, and
   property identity. A document-local node index or source line alone is not a
   valid target.

3. **Typed values**

   The patch must use the same bounded value contract as AWIR. Values must be
   finite, range-checked, versioned, and decoded without arbitrary Rust
   evaluation. `SizedBox.width(100.0) -> SizedBox.width(120.0)` is eligible only
   when the expression is a supported finite pixel dimension.

4. **Generation safety**

   Every patch carries the generation it was based on. The host rejects stale,
   duplicated, or out-of-order patches. A committed patch becomes the next
   generation and participates in reconnect and result recovery.

5. **Atomic safe-point commit**

   A patch must be validated and prepared without changing the active tree. It
   becomes visible only at the same safe point used by complete guest reloads.
   If validation, layout, or application fails, the previous generation remains
   active.

6. **State and callback preservation**

   A property patch must not recreate or replace unrelated state cells,
   callback routes, retained resources, focus ownership, or pointer capture.

7. **Correct invalidation**

   Each patchable property declares its effects. A width or height patch must
   invalidate layout and dependent paint; a text patch must invalidate text
   measurement and paint; a color patch must invalidate paint without causing
   unrelated subtree rebuilds.

8. **Deterministic fallback**

   Any uncertain case falls back to V1 and reports why the patch was not used.
   A failed V2 attempt must not leave a partially updated source index or host
   tree.

### Security and resource requirements

9. **Authenticated payloads**

   Property patches use the existing authenticated session transport. The host
   must not accept a patch from an unauthenticated or wrong application session.

10. **Bounded input**

    Patch count, metadata size, string/blob size, numeric ranges, and diagnostic
    size use explicit protocol limits. A patch must not introduce an allocation
    path that is less bounded than a complete AWIR document.

11. **Schema compatibility**

    The host checks widget schema version, property ID, codec, and capability
    digest before applying a patch. Unknown or incompatible fields are rejected
    or routed to V1 according to the schema's compatibility policy.

### Performance requirements

12. **No guest build on a successful patch**

    A successful literal patch must not invoke shadow generation, Cargo guest
    compilation, WASM validation, or full-module upload.

13. **Small payload**

    The wire payload must be proportional to the changed property, not the
    complete guest module or Widget IR document.

14. **Measured benefit**

    Benchmarks or instrumented acceptance runs must compare source-change to
    visible-frame latency, payload bytes, host allocations, and safe-point commit
    time against V1.

## Missing modules and interfaces

### 1. Patch metadata producer

The portable derive/compiler path needs to emit a patch descriptor for each
eligible property. A descriptor should contain:

```text
descriptor ID
source file and stable source anchor
source span for the editable value
old literal fingerprint and canonical value
widget schema ID and schema version
instance identity recipe or resolved instance ID
property ID and value codec
invalidation class
```

The metadata may be a bounded sidecar in the generated guest package, a section
of the generated guest metadata, or another artifact consumed by the CLI. The
choice must preserve source remapping and must not require executing arbitrary
application code in the CLI.

The metadata producer must understand nested generated widgets. A parent source
anchor and child discriminator are not enough if a mutable literal is included
in the identity hash; the identity recipe must explicitly separate structural
identity from property values.

### 2. CLI `PropertyPatchPlanner` module

The watcher currently classifies a change as `RebuildGuest`, after which the
pipeline calls `build_guest` and uploads a complete module. V2 needs a deep
planner at that seam:

```rust,ignore
enum ReloadCandidate {
    Property(PropertyPatch),
    FullGuestRebuild { reason: FallbackReason },
}

trait PropertyPatchPlanner {
    fn plan(
        &self,
        change: &SourceChange,
        active: &ActivePatchIndex,
    ) -> Result<ReloadCandidate, PlannerError>;
}
```

The interface should return a candidate, not mutate the project or active
generation. This keeps planning deterministic and makes parser, metadata, and
fallback behavior testable without a device.

The planner must:

- load the descriptor for the changed source region;
- compare the recorded old fingerprint with the active source value;
- parse only the supported literal form;
- encode the new value through the property's declared codec;
- reject zero, multiple, stale, or ambiguous matches;
- update the patch index only after the host commits the patch; and
- choose V1 when the proof is incomplete.

### 3. Patch transport contract

The current reload command carries a complete module. V2 needs a versioned
payload family, conceptually:

```rust,ignore
enum ReloadPayload {
    CompleteModule {
        metadata: ModuleMetadata,
        module: Vec<u8>,
    },
    PropertyPatch {
        base_generation: u64,
        application_id: StableId128,
        instance_id: StableId128,
        widget_schema: WidgetSchemaId,
        schema_version: Version,
        property_id: PropertyId,
        previous_value: ValueFingerprint,
        value: PropertyValue,
        invalidation: InvalidationClass,
    },
}
```

The real public interface should remain smaller than this sketch if possible;
wire details can stay inside the protocol module. Both variants need request
IDs, authenticated framing, bounded decoding, acknowledgements, terminal
results, reconnect recovery, and diagnostics that do not expose secrets.

A patch acknowledgement should identify the committed generation and whether
the result was `Committed`, `Rejected`, or `RecoverableFailure`. A lost
connection must not cause the CLI to apply the same patch twice or advance its
source index before the host confirms the commit.

### 4. Host `PropertyPatchApplier` module

The host currently validates and materializes a complete Widget IR document. V2
needs an adapter behind the reload coordinator that can:

1. locate the active node by stable instance ID;
2. verify its schema and property contract;
3. verify the previous-value fingerprint;
4. validate and decode the new value;
5. apply the property to the semantic/document or retained-element layer;
6. run the declared invalidation work;
7. prepare rollback information; and
8. commit the change at the safe point.

The first implementation should target properties whose host mutation is
unambiguous. For `SizedBox.width`, the existing host materializer already reads
the numeric property and constructs a `SizedBox`; V2 still needs a live update
operation that changes the active layout element or prepares only the affected
subtree. Reusing the materializer's value validation is desirable, but calling
the one-shot complete-tree materializer for every patch would defeat the main
latency goal.

### 5. Active patch index

The host and CLI need a generation-scoped record of patchable values. It must
track at least:

```text
active generation
application identity
source anchor
instance identity
schema/property identity
canonical current value
current value fingerprint
```

After a successful `100 -> 120` patch, the active index must record `120` so a
later `120 -> 140` edit can be patched without a full rebuild. A rejected patch
must leave the index at `100`.

The index must be invalidated or rebuilt whenever a complete guest generation
changes the relevant structure, keys, schema, callbacks, or source metadata.

## Fast-path algorithm

```text
file watcher
  -> guest source change
  -> load active patch index
  -> PropertyPatchPlanner
       |-- exact supported literal/property edit
       |     -> encode PropertyPatch
       |     -> authenticated transport
       |     -> host validation
       |     -> safe-point commit
       |     -> update active patch index
       |
       `-- anything else
             -> regenerate shadow
             -> compile and validate guest WASM
             -> upload complete module
             -> normal candidate commit
```

The planner must run before `build_guest`. A successful patch therefore bypasses
the expensive V1 operations entirely. The source file still changes normally;
the patch index is an optimization of the running generation, not a replacement
for the application's source of truth.

## Initial supported-property set

V2 should begin with a deliberately small allowlist:

| Property | Initial eligible form | Required effect |
| --- | --- | --- |
| `SizedBox.width` | finite `Dimension::Px` literal encoded as `F64` | layout and dependent paint |
| `SizedBox.height` | finite `Dimension::Px` literal encoded as `F64` | layout and dependent paint |
| `Text.text` | bounded string literal | text measurement and paint |
| `Container.color` | bounded constant color literal | paint |

The exact set must be confirmed against each schema's current guest validator,
host materializer, and mutation capability. Percent dimensions, `Auto`, values
returned by functions, values read from state or context, callbacks, child
lists, and provider-dependent values should remain V1-only until their semantics
are explicitly modeled.

## Identity and source-matching policy

The first release must choose one policy rather than silently mixing policies:

### Recommended Phase 1 policy: explicit patch keys

Patchable nodes require a stable developer-supplied key through a new documented
portable/hot-reload mechanism. The key is used only for instance identity; the
source span and literal fingerprint still identify the property edit. This makes
duplicate widgets and moved siblings unambiguous.

### Later policy: generated source anchors

The derive/compiler path may generate an anchor from package, module, structural
call path, and a literal-independent syntax fingerprint. It must define behavior
for formatting changes, inserted siblings, moved expressions, duplicated call
sites, loops, and conditional branches. If the anchor cannot be proven unique,
the planner falls back to V1.

A schema ID alone is never an instance identity: two `SizedBox` nodes can share
the same schema and property IDs.

## Acceptance tests

### Planner tests

- `100.0 -> 120.0` produces exactly one `SizedBox.width` patch.
- `120.0 -> 140.0` patches from the post-commit index without a guest build.
- Formatting-only changes do not create a value patch.
- Two matching literals produce an ambiguous result and fall back to V1.
- A changed old value or stale generation produces a fallback or rejection,
  according to the chosen retry policy.
- Dynamic expressions, macros, loops, branches, changed children, and changed
  callbacks use V1.
- Unsupported dimensions and non-finite or out-of-range values are rejected.

### Protocol tests

- Patch payloads authenticate and round-trip through the bounded decoder.
- Wrong application, schema, property, base generation, or previous fingerprint
  is rejected without changing the active generation.
- Duplicate and out-of-order request IDs are idempotent and recoverable.
- A disconnect after host commit can recover the terminal result without a
  second application.

### Host tests

- A width patch changes layout and paint at the next safe point.
- Unrelated state, callbacks, focus, pointer capture, and retained resources
  remain unchanged.
- A failed validation or layout application leaves the old tree and old index
  active.
- Patch and full-rebuild paths produce equivalent visible Widget IR/native
  behavior for the supported property set.

### End-to-end performance tests

The acceptance run should prove that a successful property patch:

- does not create `.aimer-hot-reload-shadow-*` staging output;
- does not invoke the guest compiler;
- does not upload a complete WASM module;
- transfers only the bounded patch payload; and
- reaches the visible frame faster than the V1 path on the same fixture.

## Implementation phases

### Phase 1: identity and metadata contract

- Separate schema, instance, property, source-anchor, and value-fingerprint
  terminology in the portable metadata.
- Choose explicit keys or implement literal-independent source anchors.
- Add generated patch descriptors for the initial property set.
- Add fixtures proving duplicate and moved-node behavior.

### Phase 2: protocol and generation semantics

- Add the versioned `PropertyPatch` payload and bounded codec.
- Extend acknowledgements, terminal results, and reconnect recovery.
- Define patch generation increments, idempotency, and stale-base behavior.
- Keep complete module payloads unchanged as the V1 fallback.

### Phase 3: host patch application

- Add a deep `PropertyPatchApplier` interface behind the reload coordinator.
- Implement node lookup, schema/property validation, previous-value checks,
  invalidation, rollback, and safe-point commit.
- Implement `SizedBox.width` and `SizedBox.height` first.

### Phase 4: CLI planning and integration

- Add the `PropertyPatchPlanner` seam before `build_guest`.
- Maintain the active generation-scoped patch index.
- Update the index only after a committed acknowledgement.
- Route all uncertain cases to V1 and expose a verbose diagnostic reason.

### Phase 5: expansion and measurement

- Add `Text.text`, color, and other properties only after each has a precise
  codec and invalidation contract.
- Add benchmarks and real-device acceptance runs.
- Measure memory, payload size, compile avoidance, and source-change latency.

## Non-goals

V2 is not a general Rust hot-patching system. It does not replace the guest
compiler, execute arbitrary source in the CLI, patch callback code, patch state
schemas, alter child topology, or guarantee that every source edit can be
applied without compilation. V1 remains the universal fallback.

## Definition of done

V2 is ready for default use only when a supported literal edit such as
`SizedBox.width(100.0) -> SizedBox.width(120.0)` is source-matched,
authenticated, validated, applied at a safe point, visibly laid out without a
guest build, and covered by deterministic planner, protocol, host, recovery,
rollback, and end-to-end tests. All unsupported or ambiguous edits must still
produce the same correct V1 result.
