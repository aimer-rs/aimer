# Next Event Tree Prototype

## Goal

The general element tree retains every layout node. Event routing can otherwise
walk through ordinary containers such as `Container`, `Row`, and `Column` before
it reaches an event handler. The prototype keeps all elements in `ElementTree`
while registering every event participant in its alongside `EventTree`.

```text
ElementTree                         EventTree
Container                           Button
└── Row
    └── Column
        └── Row
            └── Container
                └── Container
                    └── Button ────> ElementId
```

Each event entry stores a stable `ElementId`, cached bounds, and links to its
nearest event-handling parent and children. It does not own or borrow the
element. `ElementTree::insert` carries the closest event ancestor through
ordinary wrappers, so the event hierarchy is registered as the general tree is
constructed. Layout updates handler bounds through
`ElementTree::update_event_bounds`. A balanced bounds hierarchy groups
consecutive event registrations; each branch caches the union of descendant
target bounds. Hit testing prunes branches outside the pointer and can yield
either paint order or topmost-first order.

## Spatial Event Hierarchy

The index is a balanced binary hierarchy over registration order. Its leaves
refer to event entries; each branch stores the union rectangle of both
children. Layout updates a target's bounds and recomputes only the path to the
root, taking O(log H) time for H event targets. When registration exceeds the
current power-of-two capacity, the index grows and rebuilds. Hit testing uses a
fixed stack and visits only branches containing the pointer, so it allocates
nothing per query.

An event target without layout bounds stays eligible everywhere, matching
Aimer's general dispatcher. Registering a target inside existing index capacity
refits the affected path too; layout then replaces the unbounded summary with
its cached rectangle.

```text
                 bounds for targets 0..8
                 /                    \
        bounds 0..4                  bounds 4..8
        /       \                    /       \
   targets 0..2  targets 2..4   targets 4..6  targets 6..8
       ...           ...            ...           ...
```

Consecutive registrations are grouped together to preserve paint order without
sorting. This prunes well when registration order also groups nearby layout
regions, as it does for the showcase's category and button sequence. Scattered
bounds can make a branch rectangle broad and reduce pruning, but cannot hide a
target because each branch rectangle contains all descendant rectangles.

## Stress Test

`new_event_tree::tests::sparse_event_tree_stress_keeps_only_event_handlers`
builds 100,000 retained elements. A button at the end of a six-ancestor path
and 999 additional elements are marked as handlers, for 1,000 event entries
total. The remaining 99,000 elements stay in the general tree only.

The test checks that:

- all 100,000 element IDs remain in the general tree;
- the event tree contains exactly 1,000 entries;
- layout can update handler bounds through their element IDs;
- hit tests return the expected handler ID for each registered bound;
- a point between handler bounds returns no hit; and
- a regular, non-handler element does not accept an event-bounds update.

### Showcase-shaped stress case

`new_event_tree::tests::showcase_sidebar_tree_routes_visible_buttons_through_layout_wrappers`
models the currently active tree built by `jaime/src/showcase.rs`. The content
pane is commented out in that build, so the fixture includes the outer
`Container -> Column -> Expanded -> Row`, the sidebar, its `Scrollable`, the
category column, and the divider.

The category data has seven expanded lists and 55 example buttons:

| Category | Buttons |
| --- | ---: |
| Controls | 10 |
| Navigation | 5 |
| State & data | 7 |
| Content | 5 |
| Layout | 8 |
| Media & styling | 12 |
| Interaction & runtime | 8 |

The fixture follows the wrappers visible in the widget builders: each category
has a header `Focusable -> KeyRelay -> GestureDetector` and an expanded
`AnimatedCollapse -> Container -> ListBody Container -> Column`; each button
has a `MouseRegion -> GestureDetector -> Container -> Container -> Row` with
icon and label children. It registers both pointer targets: 55 button tap
targets and 7 category-header tap targets. On macOS it includes the optional
38 px top-margin wrapper around the first category, as the source does.

The test checks all buttons visible in a 720 px viewport at scroll offset zero,
then scrolls to the final button and checks it again. It also checks a category
header, an inter-button gap, and the divider. The button target sits 17 modeled
tree nodes deep on other platforms and 18 on macOS. The fixture contains 623
modeled nodes off macOS and 624 on macOS, compared with 62 event targets.

The ignored comparison
`new_event_tree::tests::compare_pruned_general_tree_walk_with_showcase_event_tree`
uses the same bounds and asks both paths to return the same visible button. The
general-tree walk tests each node's bounds and descends only into branches that
contain the pointer. The event-tree path descends through unions of target
bounds and checks only target leaves in the matching branch. The benchmark runs
1,000 queries per case in the local debug profile; tree construction is outside
the timed region. Two runs measured:

| Scenario | General tree size | Event targets | General bound checks/query | Event hierarchy checks/query | General time/query | Event time/query |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| One showcase sidebar | 624 nodes | 62 | 41 | 13 | 1.38–1.89 µs | 0.24 µs |
| 100 horizontally spaced showcase roots | 62,400 nodes | 6,200 | 140 | 27 | 2.64–2.99 µs | 0.41–0.42 µs |

The hierarchy checked about 3× fewer bounds than the general tree for one
sidebar and about 5× fewer in the scaled case. In these runs, event lookup was
roughly 6–8× faster. These are debug-profile measurements from this machine;
the operation counts are the more repeatable result.

## Dense and Sparse Framework Event Trees

`new_event_tree::tests::dense_sparse_framework_dispatch` builds paired Aimer
`Element` trees from the same `ElementTree` fixture. The dense fixture indexes
every element as a hit-test boundary. The sparse fixture keeps event
participants and their nearest event ancestors; each participant asks the same
spatial index for its direct event children. Both trees use Aimer's real
`EventDispatcher`, `ElementEvent`, `dispatch_focused_event`, and
`broadcast_event` APIs. Both sides now use indexed routing.

The built-in role audit currently opts in the retained `StatefulElement` and
`StatelessElement` wrappers and `FocusScope` as transparent nodes. Their event
callbacks do not add behavior, and their event children are the children the
general dispatcher already traverses; focus trapping remains collected from
the structural tree. `TextButton`, `DragTarget`, and `DropZone` are indexed
targets because they handle events directly and expose ordinary event-child
routing without a custom hit-test boundary. `TextButton` currently reports no
layout bounds, so it remains an unbounded indexed target and still applies its
existing per-line hit test in the callback.

Elements with custom routing use the default indexed hit-test-boundary role,
including `MouseRegion`, gesture detectors, scrollable containers, and
`Stack`'s layer-sorted hit order. Positional routing intersects spatial index
hits with each boundary's current child hit-test view, while focus and
broadcast delivery retain the full event-child view. `Legacy` remains an
alias for the indexed hit-test-boundary role.

`aimer_flex::flex::lazy_tests::indexed_flex_dispatch_matches_boundary_child_and_broadcast_views`
compares a public `Column` widget, lowered to `RawFlex`, with one child using
the conservative boundary role. It verifies that an unpainted child receives
no positional event and still receives a broadcast. The ignored
`compare_warmed_direct_and_boundary_indexed_flex_dispatch` benchmark uses
10,000 eager children, six transparent wrappers per child, finite child bounds,
and a 40 px viewport. A pre-promotion debug-profile run measured setup at
about 83 ms per view and warmed routing at 2.01 µs indexed versus 2.21 µs
general for 2,000 routes. Those timings compare the earlier legacy route and
are historical; rerun the renamed benchmark before drawing conclusions about
the two indexed configurations. Run it with:

```bash
cargo test -p aimer_flex compare_warmed_direct_and_boundary_indexed_flex_dispatch -- --ignored --nocapture
```

Parity tests cover pointer hit order and overlap, simultaneous mouse and touch
captures with equal numeric IDs, captured movement and release outside bounds,
pointer cancellation, character/text/IME input, forward and reverse Tab,
focus/unfocus transitions, file and drag/drop routes, broadcast delivery,
nested event participants, unbounded targets, custom legacy hit-test
boundaries, and moved layout bounds. They compare callback order and effects,
focus ownership, capture owners, and final results. They also verify that
non-consuming overlaps continue to lower targets and that captured follow-up
`DragOver` and `DragDrop` passes reach the target under the pointer. Widget
tests also cover indexed subtree replacement and layout-bound refitting.

The ignored benchmark times a `HoveredFileMoved` route through the warmed real
dispatcher. Each handler reports redraw without consuming, so dispatch visits
all matching event handlers. Build timings separate the model plus event
registration, materializing the full framework tree, and materializing the
sparse event view. A 2,000-update layout microbenchmark compares writing one
target's bounds with writing the same bounds and refitting the event index. The
timings below are local debug-profile measurements; the route iteration counts
and visit counters make the work easier to compare than time alone.

| Scenario | General nodes | Event participants | Build: model + registration | Build: general view | Build: sparse view | Changed target bound: general / event + refit | Warm route: general / sparse | Callback calls per route: general / sparse | Bound reads per route: general / sparse | Spatial checks per route |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| One showcase sidebar | 624 | 62 | 0.40 ms | 0.74 ms | 0.15 ms | 0.06 / 0.92 µs | 12.22 / 1.34 µs | 24 / 2 | 43 / 3 | 13 |
| 100 showcase roots | 62,400 | 6,200 | 17.30 ms | 20.24 ms | 7.94 ms | 0.01 / 0.65 µs | 14.44 / 1.32 µs | 24 / 2 | 142 / 3 | 27 |
| Unbounded 100k stress tree | 100,000 | 1,000 | 8.38 ms | 21.01 ms | 1.11 ms | not measured | 14,642 µs / 1.15 µs | 99,002 / 2 | 100,002 / 3 | 21 |

The 100k route was about 12,700× faster through the sparse view in this run.
It is deliberately an unprunable general-tree fixture: the ordinary layout
nodes report no bounds, which makes them hit-eligible under framework
semantics. The spatial view checked 21 branch and target bounds per route. The
one-sidebar and 100-root cases show smaller but consistent route reductions;
the 100-root route was about 10.9× faster in this run.

The bound-update column isolates one target write. It does not measure the
complete layout pass: normal layout still computes geometry for the general
tree, while `EventTree` adds an O(log H) refit for each event target whose bound
changes. The per-update times are very small and fluctuate between runs.

### Integrated event-tree measurement

After the framework integration, the ignored comparison was rerun with
indexed routing enabled. The sparse framework adapter uses `IndexedTarget`,
and the benchmark exercises `EventDispatcher`'s retained event index. This
remains a debug-profile measurement using laboratory adapter
elements, not the showcase application's production widgets. These figures
were collected before the dense fixture also used the event tree and are kept
as historical results.

| Scenario | General nodes / targets / event roots | Build: model / general / sparse | Bounds update: general / indexed | Warm route: general / indexed | Callback visits: general / indexed | Bounds reads: general / indexed |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| One sidebar, 2,000 routes | 624 / 62 / 62 | 0.15 / 0.27 / 0.08 ms | 0.01 / 0.34 µs | 6.93 / 0.96 µs | 48,000 / 2,000 | 86,000 / 4,000 |
| 100 sidebars, 2,000 routes | 62,400 / 6,200 / 6,200 | 13.88 / 22.30 / 10.21 ms | 0.01 / 0.68 µs | 14.44 / 1.19 µs | 48,000 / 2,000 | 284,000 / 4,000 |
| Unbounded 100k stress, 3 routes | 100,000 / 1,000 / 1,000 | 8.24 / 23.25 / 1.57 ms | not measured | 14,970.97 / 1.06 µs | 297,006 / 3 | 300,006 / 6 |

With hit-generation stamps, active-root/child scratch, and cached target
pointers guarded by root identity and subtree generation, the indexed route is
about 7.2× faster for one sidebar and 12.1× faster for 100 sidebars in this
debug-profile run. The unbounded 100k route is about 14,100× faster. The
showcase has 6,200 event roots in the 100-sidebar case; dispatch now visits the
roots and child links touched by the hit query instead of scanning the full
root list or rewalking wide structural sibling lists to resolve each target.

Rerun with:

```bash
cargo test -p aimer_laboratory compare_real_framework_dispatch_build_layout_and_warm_routes -- --ignored --nocapture
```

## Synthetic General-Tree Comparison

The older ignored manual comparison is
`new_event_tree::tests::compare_general_tree_walk_with_sparse_event_tree`.
It times 200 hit queries on a prebuilt tree with 100,000 elements and 1,000
event handlers. Both paths test the same point and return the same handler ID.

- **General-tree path:** depth-first walk through the element tree with a
  reusable, preallocated stack. This is an unprunable workload: every ordinary
  node is considered hit-eligible, so the walk visits all 100,000 nodes.
- **Event-tree path:** traverse branch unions in reverse paint order and test
  target bounds only in branches containing the point.

One run in the local debug test profile measured:

| Path | Work per query | Time per query |
| --- | ---: | ---: |
| General tree | 100,000 element visits | 4,620.27 µs |
| Spatial event tree | 21 branch/target bound checks | 0.35 µs |

The spatial event tree was about 13,000× faster in this unprunable workload.
These numbers are a synthetic lookup-only baseline; tree construction and
framework dispatch are outside that timed region. Use the real dispatcher
comparison above for end-to-end routing results.

The manual comparisons can be rerun with:

```text
cargo test -p aimer_laboratory compare_general_tree_walk_with_sparse_event_tree -- --ignored --nocapture
cargo test -p aimer_laboratory compare_pruned_general_tree_walk_with_showcase_event_tree -- --ignored --nocapture
cargo test -p aimer_laboratory compare_real_framework_dispatch_build_layout_and_warm_routes -- --ignored --nocapture
cargo test -p aimer_laboratory
```

## Current Limits

The showcase fixture mirrors the source widget topology and authored bounds; it
does not build or render the production `jaime` widgets. The scaled case uses
100 horizontally separated roots in one index and is a handler-count stress
case, not a claim about one real app screen. Indexed event routing is now the
framework default. `Legacy` is a compatibility alias for the conservative
indexed hit-test-boundary role; it no longer selects a separate tree walk.
Elements marked `Transparent` must not add clipping or custom hit-test
boundaries, and indexed elements must report bounds in the same coordinate
space as layout.

The spatial hierarchy groups targets by registration order, so paint orders
whose bounds are spatially scattered can create broad branch unions. The
comparison uses laboratory elements and authored bounds, not production
showcase widgets. The unbounded 100k case is deliberately hard on the general
tree; normal bounded layouts have a much smaller traversal cost. Stateless
helpers such as `dispatch_event` build a temporary index, while the retained
`EventDispatcher` benchmark above measures the warmed path.
