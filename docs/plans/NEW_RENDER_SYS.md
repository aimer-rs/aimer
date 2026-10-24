# New Render System

## Goal

Move Aimer from one frame-wide command list to a retained render tree. Each
painted element owns a small local draw list. A state change records only the
affected element, while rendering replays all elements that intersect the
damaged pixels in parent-first paint order. This keeps clean siblings' command
lists intact and avoids rebuilding unrelated widget content.

The framework migration is incremental. The existing frame renderer remains
available while v2 support is added, and an unported subtree can be represented
as one legacy paint island in the v2 order.

## Agreed design

- `aimer_cupid::draw_cmd_v2::RenderTree` owns node bounds, clips, opacity,
  parent-child order, local `DrawList`s, revisions, and pending logical damage.
- `RenderNodeId` is the render identity. The framework keeps a stable mapping
  from an element identity to its optional render node.
- Each window handler owns one `RenderTree` on the UI thread. The tree uses
  `Shared`/`RefCell` data and is not sent to the raster thread. A finished
  frame contains owned, transferable paint data.
- A v2 paint hook records only the element's own commands. Children are
  traversed and recorded independently. An element can participate in layout
  or event handling without owning a paint list.
- An element that has not opted into v2 is captured as one bounded legacy
  subtree island at its place in paint order. This allows v2 and legacy
  subtrees to coexist during migration.
- A layout change damages the old and new subtree footprints. A paint change
  invalidates only that element's local list. The composition plan includes
  clean overlapping nodes so damaged pixels can be reconstructed in order.
- A node with children and opacity below `1.0` is an opacity group. Its
  descendants are rendered to an intermediate GPU surface and the group is
  composited once. Applying the opacity independently to each child is not
  equivalent when those children overlap.
- Event routing remains in the event tree. Render node lookup is associated
  with stable element IDs; the renderer does not search the widget tree for
  every draw operation.

## Phased implementation

### 1. Frame adapter and pixel proof

Connect the lab render plan to Cupid's existing `FramePacket` and renderer.
The adapter lowers opaque `RenderOp::Draw` items into the current flat command
list, preserving item origin, clip, transform state, and paint order. It maps
logical damage to device pixels with outward rounding at the captured scale,
then normalizes and clips the result through `DamageSet`.

If normalization or target metadata requires a full repaint, the v2 frame must
carry full-target damage and the complete paint plan; the adapter rejects an
incomplete full repaint instead of clearing pixels whose paint operations were
culled from the frame.

The initial flattened compatibility adapter rejects opacity groups instead of
turning group opacity into per-command alpha. Direct retained replay adds the
offscreen group compositor in phase 5.

**Checkpoint:** implemented in `aimer_cupid::v2_frame_adapter`; the laboratory
counter simulation exposes its initial full frame, and its initial and
incremental frames are packet-tested. A Metal GPU test checks the first frame
and verifies that a later child update preserves pixels outside its damage.

### 2. Per-window render ownership

Add a render tree to `AimerApplicationHandler`, with one tree per window and
UI-thread ownership. Keep the identity mapping alongside the retained element
tree, and update node parentage and order during build/reconciliation. Resize,
surface replacement, renderer/context/resource generation changes, and
unknown target contents must request a full repaint.

Do not put layout-only or event-only wrappers in the paint tree unless they
own a clip, opacity group, or other render effect. Keep event target identity
and hit testing in the event tree.

**Checkpoint:** `AimerApplicationHandler` now owns a UI-thread `RenderTree` and
maps stable `ElementId`s to retained `RenderNodeId`s. The frame drawer refreshes
that structure when element, layout, target, scale, or root identity changes.
Structure sync preserves local lists for surviving IDs, removes stale nodes,
updates child order, and damages old/new bounds.

### 3. Opt-in element painting and legacy islands

Add a self-only v2 paint hook with a default legacy fallback. A successful v2
hook means the element is handled even if it emits no commands; its children
remain separate render nodes. If the hook declines, retain the complete legacy
subtree as one paint island and skip lowering its descendants individually.

Move widgets over gradually. Start with stable backgrounds and text, then
containers and common controls. Keep texture upload/removal, clear operations,
custom pipelines, and retained-layer commands on the legacy island path until
v2 owns their resource and invalidation rules.

**Checkpoint:** `Drawable::paint_local_v2` now lets an element record only its
own retained commands. Unported elements become legacy islands with a
frame-local command range, and render-order traversal skips their descendants.
`SizedBox` now records its solid background as a local v2 fill, and a regression
test verifies that repainting it leaves a sibling's list revision untouched.
Plain top-left text with bounded, normal-font styling now records a local v2
text run; advanced paragraph, selectable, decorated, and unbounded text remains
on the legacy fallback. Text repaint coverage checks a changed string, logical
coordinates at 2x scale, and an unchanged sibling list. `ModalHost` records an
empty local list and draws hosted modal content through its separate legacy
overlay node, so the automatic app wrapper no longer hides descendant v2
items. `FramePacket::from_v2_with_legacy` accepts the matching legacy frame,
inserts island command ranges in render-tree order, and reconstructs their
incoming canvas state. `FrameDrawer` now validates and hands that mixed packet
to the existing renderer; it keeps the complete legacy frame when synchronization
or validation fails. Packet tests cover interleaving, clipping, damage, missing
frames, invalid ranges, and a headless 2x-scale handoff. `RawContainer` now
records its plain rectangular background as a local list and synchronizes its
rectangular child clip with the retained node. Cases with margins, padding,
borders, rounded corners, outlines, shadows, or unbounded dimensions remain
legacy islands. `RawImageWidget` now records image draws locally with a shared
retained RGBA resource for built-in file, asset, and network sources. The
generic renderer uploads a missing texture or a changed resource revision when
replaying the retained list, so the v2 command remains usable after the GPU
texture cache is lost or evicted. Existing providers that expose only a texture
ID still require that texture to be resident in the current renderer
generation; loading and failed images stay on the legacy path. Initial decoding
now creates a retained image resource without queuing a legacy upload command.
The local v2 list owns the resource and uploads it on replay; legacy islands
and adapter fallbacks carry the same resource-bearing draw command. A legacy
island retries local-v2 eligibility after asynchronous content arrives.
Bounded `Svg` scenes now record the
existing retained SVG command with persistent, hover, and pressed style
overrides intact. Redraw results from visual events invalidate that element's
local list for one frame. A ready `SvgAsset` exposes its loaded child to the
retained tree, where bounded SVGs record independently; async child changes
invalidate tree and layout generations. Loading/error fallbacks and scenes
that cannot be proven inside their bounds remain legacy islands. `RawTextButton`
now records safe labels through `RawTextWidget`'s v2 text command. Color-only
hover changes dirty that element's list; style changes that alter metrics or
effects keep the existing legacy path and conservative repaint behavior.
`aimer_input::Button` now reaches separate retained nodes for its stateful
wrapper, mouse/gesture wrappers, decorated container, and child label. The
no-paint wrappers record empty local lists; supported container backgrounds
and labels can update independently, while the existing press/hover/disabled
state and callbacks remain in place. Container decorations outside the v2
subset continue through the legacy fallback.

`AspectRatio`, `Padding`, and `Margin` now opt in as empty local nodes. They
publish the child constraints used by their layout and, for spacing widgets,
the child's translated retained bounds. Invalid scales and unbounded wrapper
dimensions remain on the legacy path. Other built-in widgets and resource
commands still need migration.

`Grid` now contributes an empty retained node for valid resolved layouts. Its
items keep their existing alignment and paint order, and per-cell clips are
translated into each child's local coordinates and intersected with child
clips. Layout errors, invalid geometry, and overflowing items continue through
the legacy island so its diagnostics and paint behavior remain available.

`Resizable` retains its interactive size in the existing layout/event path
while exposing its effective child constraint and rectangular clip to v2.
Invalid or oversized extents use a legacy island. `ZeroSizedBox` opts in as an
empty retained leaf.

Transparent focus-scope, collapsible-key, and navigation-key relay wrappers
now retain empty local lists and continue forwarding paint-geometry sync to
their children. Their event behavior remains on the existing event path.

`Align` now records no local paint and positions its independently retained
child at the legacy alignment offset. Invalid scales, unbounded parents, or
invalid child bounds keep the complete subtree on the legacy path.

`Stack` now keeps its original event/reconciliation order while the retained
visitor emits children in the cached layer order used for painting. It supplies
the same child constraints as legacy traversal and invalidates its hit-test
index during geometry sync.

`Positioned` exposes its child to the retained tree for identity and translation
transforms, preserving anchor offsets and child-local clipping. Scale and rotate
transforms remain a legacy island until the retained tree carries full affine
transforms.

`ProviderScope` and `StoreProviderScope` now retain empty nodes and fork child
contexts with the same provided value and dispatcher state used by legacy
traversal. The fork snapshots the inherited state map so v2 layout and paint
callbacks keep those values after the scope callback returns.

`DragTarget` and `DropZone` now retain empty local lists while keeping their
cached hit bounds current in the geometry-sync path. `Draggable` remains on the
legacy path while its active child and placeholder are selected dynamically.

`NavigatorScope` and `ShellScope` now expose their active route as an independent
retained child while snapshotting the route context, controller, and outlet
slot into its build context. Their dynamic event and routing behavior remains
unchanged.

`Anchor` now retains as an empty node and refreshes its shared hit bounds during
geometry synchronization. Modal and floating overlays with animated placement
remain on the legacy path.

`aimer_selection::Checkbox` now records its default indicator, padded surface,
and label as independent retained nodes. Transparent focus, keyboard, opacity,
and flex wrappers expose their descendants; the rounded indicator paints as a
leaf so it does not need a rounded child clip. Checked, hover, pressed, and
disabled visuals still come from the existing state builder. The shared
indicator is also used by Switch, Radio, and grouped selection controls, so
those indicators use the same retained leaf. Custom visual builders keep their
current state input and use retained painting where their descendants support
it, with unsupported decorations falling back as before.

`aimer_selection::Select` records its trigger and open option list through the
same retained helpers. Keyboard and pointer focus update the focused option's
indicator and label, while selection, disabled styling, and open/close rebuilds
remain state-driven. The existing Select model has no loading state; asynchronous
loading is represented by `Autocomplete`. The open list remains in its existing
inline layout.

`aimer_selection::Autocomplete` records its caller-controlled query display,
loading and error labels, and filtered suggestion rows as retained children.
Focused suggestions highlight on keyboard movement and pointer entry; focus is
restored by stable option key when the controlled configuration rebuilds.
Loading still hides suggestions and selection remains disabled while loading.
The query is still caller-controlled text; this widget does not currently
contain an editable text field.

`aimer_range::Slider` now retains its default rail, active trail, and thumb as
separate visual nodes. The slider keeps pointer capture, value mapping, and
keyboard handling; the trail slot owns the live active-segment clip, and
pressed/focused/disabled default colors rebuild only the affected visual
children. Custom visual children can record locally when supported and keep the
legacy fallback otherwise. Pointer hover remains tracked without a dedicated
default hover color.

`aimer_range::RangeSlider` now retains its rail, selected segment, and both
thumbs independently. The selected segment clip follows the lower and upper
thumb positions. Active-thumb selection, pointer capture, keyboard updates,
focus, and disabled styling remain in the current state and event paths; custom
visual children retain the legacy fallback when they cannot record locally.

`aimer_input::TextField` and `TextArea` record supported decorations, selection
rectangles, placeholder or text, and composition underlines in retained local
lists. TextArea wrapping, vertical scroll-to-caret, and multiline selection use
the editable geometry already shared with hit testing; the caret remains a
separate retained child driven by the existing focus and blink state. Script
language, glyph and box shadows, text backgrounds, built-in decorations,
transforms, spacing, and overflow all record locally. The field's text list is
recorded through the same `RawTextWidget` fragment painter used by standalone
text. The enclosing `SelectionArea` is a no-paint retained node so the field
can keep its own list while the selection session continues to manage handles
and context menus.

`RawRichText`, selectable `Text`, and paragraph-backed `RawTextWidget` record
retained commands from their prepared fragments. Multiline wrapping and
ellipsis, inline font/color/weight/style changes, transformed text, indentation,
line height, letter and word spacing, span backgrounds, and decorations all use
the same measured fragment geometry as the legacy painter. Linked runs retain
their hover color in the local list; geometry sync rebuilds source-aware link
and selection hit regions, while touch holds, selection handles, and context
menus remain on their existing event/overlay paths. Retained node outsets now
cover glyph shadows, decorations, and italic overhang while leaving layout and
hit regions at their original bounds. Built-in paragraph alignment and custom
overflow tags use the prepared fragment commands, and plain text preserves its
legacy clip behavior for unknown overflow values.

Plain selectable `Text` uses the same retained fragment commands, with its
selection fill and source-aware hit regions in the retained node. Date,
date-time, time, and color picker labels and values now use
retained local commands with their existing theme states; color sliders keep
their separate child nodes and offsets. Context-menu row labels and hover or
pressed fills, plus error-widget diagnostics, also record locally. The debug
overflow warning paints as a retained decoration child after the clipped
content, preserving its striped overlay and message order.

`aimer_scroll::ScrollBar` now records its track, optional buttons, and rounded
thumb in a retained local list for either axis. The legacy and v2 paths share
thumb geometry and runtime updates, including hover/drag colors, hit-test bounds,
and the existing show/hide alpha transition. Invalid dimensions use the legacy
fallback; scrolling and pointer capture remain owned by `ScrollState`. The
retained walker refreshes the small local list only when its sampled paint
values change, including during the fade, and reuses it while the bar is idle.

`aimer_scroll::RawScrollableContainer` now acts as a retained viewport node in
v2 frames. Its content remains a separate subtree; the child receives the
scroll-axis constraint and visible window used by virtualization, while the
render tree updates that child's translated bounds and fixed viewport clip as
the offset changes. Scrollbars remain sibling nodes outside the content clip.
The retained legacy layer remains available when drawing outside the v2 tree.
Focused Quiver tests now cover the translated content offset, composed child
clips, viewport bounds, viewport-only damage, and virtualized row-window
updates. Windowed flex sources advance a separate retained-render-structure
generation when their live child range changes, so Quiver refreshes the node
map without invalidating event-path indexes or layout caches. Quiver records
new local-v2 nodes from post-draw sync contexts and composes them in the same
frame; non-local additions still use the legacy fallback. Structure sync
applies updated bounds and clips atomically to keep intermediate offscreen
damage out of the viewport. Cupid also tests that disjoint nested clips cull
descendants and their damage. Windowed `RawFlex` nodes now remain local-v2
parents; indexed child sync preserves row offsets so visible rows appear as
independent operations in the retained plan. A headless `FramePacket` test
scrolls the row window and verifies the newly exposed rows have local-v2 lists
in the submitted plan while packet damage stays inside the viewport. Structural
sync now damages changed clipped subtrees instead of the full app root, and
scroll damage intersects the antialiasing pad with the viewport clip. The Metal
GPU acceptance test advances a two-row window by one row, submits the viewport
damage through a direct `FramePacket`, and checks the GPU readback: changed rows
appear inside the clip, content overdraw stays clipped, and every pixel outside
the viewport matches the prior frame. Quiver also renders a packet from its real
headless `SizedBox` / `Scrollable` / windowed `Column` widget tree on Metal. That
test verifies rows 20–23 were materialized into the packet, their pixels reached
the target, and pixels beyond the 100×80 viewport match the initial frame.
It then resubmits the partial-scroll packet to an enlarged target, verifies a
full repaint, checks that the original image is restored, and confirms the new
target area outside the recorded frame is cleared.
The same test then sends a resize event to the actual headless Quiver app. Its
percent-sized scroll viewport grows, the row window adds two visible rows, and
the newly built packet renders all six rows on Metal with a full-target repaint.
Finally it jumps to the clamped list end, checks the bottom six row pixels, and
confirms the follow-up damage remains inside the enlarged viewport. It repeats
the end-clamp check with a horizontal virtualized `Row`, verifies the final five
items render at the right edge, and checks pixels and damage outside its clip.
While the horizontal list is at its old end, another app resize recomputes its
maximum extent, keeps the offset clamped, and renders the new nine-item window.
It then shrinks the horizontal source in place to three items while at the end,
checks that the root, viewport, and controller survive, and confirms the
controller reclamps to zero while removed tail pixels clear. It repeats the
in-place shrink-at-end update for the vertical list, checks root and viewport
identity, and verifies the removed bottom rows are cleared too.
After restoring the full horizontal source, it scrolls to a middle offset and
shrinks the offscreen tail. The test checks that the offset and visible row
identities remain stable while packet damage stays within the viewport.
For a stateful update with unknown event bounds, Quiver accepts the retained
tree's damage only when the updated element's entire subtree uses local v2 paint.
The retained tree then owns damage from local draw-list changes and layout sync.
Legacy, removed, untracked, or incomplete render plans retain the full-frame
fallback.
The same Metal test updates one visible virtualized row's color in place and
checks that only that row's list revision advances, damage stays within its
bounds, and readback pixels outside it remain unchanged. It then shrinks that
row's height from 80 to 60 pixels and checks that the retained row identity
survives, only its list revision advances, damage remains inside the old/new
bounds union while covering the removed tail, and the Metal readback clears
that tail without changing neighboring pixels. It then grows the row back to
80 pixels and checks that damage includes the newly exposed strip while staying
inside the new/old bounds union, neighboring lists remain unchanged, and the
Metal readback repaints the strip. The headless test checks the same bounds,
damage, and list-revision behavior for both height changes.
Run it on macOS with:

```text
cargo test -p aimer_quiver --no-default-features --features wgpu --lib actual_headless_virtualized_scroll_packet_renders_and_resizes_on_metal
```

### 4. Direct damage rendering

Once mixed-tree packet output is correct, let each backend consume the retained
render plan directly. Keep a persistent scene target per window. For a partial
frame, clear the damaged region in that target, then replay every intersecting
v2 item and legacy island in paint order under the region's GPU scissor. Present
the reconstructed target after the updates.

This redraws only damaged pixels while still repainting clean content beneath
transparent or overlapping paint. Use a full repaint when the target is new or
invalid, damage covers enough of the window that partial work costs more, or a
backend cannot preserve the target. Add WGPU and Metal coverage for target
reuse, clipped writes, resize, and damage promotion.

**Checkpoint (direct retained replay):** Windowed Quiver frames now transfer a
complete ordered plan to the backend without flattening v2 commands into the
legacy frame list. Local v2 items carry shared immutable snapshots; legacy
islands point to their original command ranges and include the captured canvas
state needed to replay those ranges independently. Each item carries
conservative device-pixel bounds. WGPU, Metal, WebGPU, OpenGL, Vulkan, and DX12
frame builders preserve the plan, and their generic renderers select intersecting
operations in paint order for each damage scissor. The renderer caches lowered
v2 lists by node ID, recording revision, scale, origin, and clip. Target resize
and recreation paths promote the complete plan to full damage. Headless and
compatibility paths retain the flattened adapter for inspection and fallback.
A Cupid unit test verifies the direct packet keeps local v2 commands out of the
legacy draw list while preserving operation order and legacy command ranges.
Metal pixel coverage exercises a mixed local-v2/legacy frame and a later child
update while checking pixels outside its damage remain intact.

**Remaining:** Direct retained-presentation frames now omit legacy paint
commands from local-v2 nodes while retaining their canvas-state recording and
draw-time side effects. Legacy islands resume paint recording for their command
ranges, and failed or incomplete retained plans repaint through the full legacy
path. `RawTextWidget` skips its old painter in this mode, so standalone and
button-label text no longer measures or emits legacy text commands on every
frame. Rich text, selectable text, and supported TextField/TextArea paths now
sync link, selection, touch-hold, scroll, hover, caret, and IME state before
recording their retained lists, then skip their legacy painters. Text fields
resolve deferred clicks, selection changes, native preedit, and chosen menu
actions during that pre-paint sync; the context menu stays in its separate
overlay. Unsupported paint and invalid geometry still use a legacy island.
Other local-v2 widgets keep compatibility traversal for state updates and child
visitation, while resource bookkeeping remains available for fallback content.
Next, split remaining widget side effects from legacy painting, then remove the
compatibility traversal after equivalent multi-backend, target-recreation, and
performance coverage is in place.

### 5. Opacity groups

Implement `BeginOpacityGroup` and `EndOpacityGroup` with a transparent
intermediate surface clipped to the group's effective bounds. Render all group
descendants into it in their normal order, then blend the group into its parent
once using the group's opacity. Track group damage from child paint, child
layout, clip changes, and opacity changes.

**Checkpoint:** direct packets retain balanced group begin/end boundaries, and
leaf opacity is represented as a one-item group so it uses the same alpha
compositing rule. The generic GPU renderer reuses an intermediate target
cropped to each group's device-pixel bounds at its nesting depth. It translates
retained transforms into the cropped surface's local coordinates, clears the
cropped target before reuse, replays only operations intersecting the damage
scissor in child order, then composites the group at its original position
under that scissor with premultiplied-alpha blending. Group targets follow the
window's renderer/context/resource identity and resize with their bounds.
Multisampled frames use a matching per-depth multisample surface and resolve
into the retained group target.
Opacity-only changes keep local list revisions stable. Cupid tests group
boundary lowering, partial damage selection, cropped coordinate transforms,
and leaf opacity. A GPU pixel test requests a fallback adapter first, then any
available backend; it covers nested overlapping children, nested clips,
opacity-only updates, and damage reaching the output target's right and bottom
edges.

**Coverage:** native Metal pixel coverage now exercises cropped nested groups
with 4x MSAA. A native Vulkan test covers nested composition and target-edge
damage, but the test is target-gated and has not been compiled or run on this
macOS host. The WGPU adapter pixel test also needs a host with an available GPU.

**Remaining:** run Vulkan coverage on Linux/Android and exercise multisampled
cropped groups on another native backend.

### 6. Migration and cutover

Port the remaining widgets by responsibility, maintaining the legacy-island
fallback until their command and resource behavior is represented in v2. Use
the lab simulations for state changes that affect only paint, layout, or both.
Remove the compatibility render path only after every shipped widget and
backend meets the same visual, event, resize, resource, and damage criteria.

## Validation and performance evidence

- Test command lowering, paint order, origin transforms, clips, invalid scale,
  outward damage rounding, target clipping, direct opacity groups, and explicit
  rejection by the flat compatibility adapter.
- Keep a simulation assertion that a count update changes the text list
  revision, leaves unrelated lists clean, and damages both old and new layout
  footprints.
- Compare GPU pixels before and after a child update, including pixels outside
  the damaged area. Exercise both WGPU and Metal as their adapters become
  available in the test environment.
- Measure local list recordings, commands copied by the adapter, damaged area,
  and GPU work before claiming a speedup. The direct backend phase should
  remove the adapter's full-list flattening and copies.
- Run focused crate tests first, then workspace checks for framework-wide
  changes. Preserve existing tests and report platform GPU tests that could not run because no adapter was available.
