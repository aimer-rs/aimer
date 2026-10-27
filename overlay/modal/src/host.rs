use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use aimer_animation::{AnimInstant, Curve};
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::Canvas;
use aimer_events::element::ElementEvent;
use aimer_events::window::request_animation_frame;
use aimer_macro::Rebuildable;
use aimer_widget::base::BuildContext;
use aimer_widget::focus::FocusTrap;
use aimer_widget::{
    AnyElement, Drawable, Element, EventDispatcher, EventElement, EventResult, EventTreeRole,
    LayoutElement, PointerKey, Rebuildable as RebuildableTrait, RequiredChild, VisitorElement,
    Widget, broadcast_event, dispatch_focused_event,
};

use crate::ModalAnimation;

type EntryBuilder =
    Box<dyn FnOnce(&BuildContext, ModalId, Rc<RefCell<ModalTimeline>>) -> AnyElement>;

aimer_tls::thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static COMMANDS: RefCell<VecDeque<ModalCommand>> = const { RefCell::new(VecDeque::new()) };
    // The entries and contents are shared with the overlay elements built for
    // this window, so what the thread holds is a handle to swap, not the data.
    static ENTRIES: RefCell<Rc<HostedModalEntries>> = RefCell::new(Rc::default());
    static LAYERS: RefCell<Vec<HostedLayer>> = const { RefCell::new(Vec::new()) };
    static CONTENTS: RefCell<Rc<HostedContents>> = RefCell::new(Rc::default());
}

/// The modals, layers and hosted content of one window, kept with the window
/// while another is active.
///
/// A dialog belongs to the window that presented it. With several windows on a
/// thread the registry cannot be one: a dialog shown in a window would appear
/// in every window's overlay. Each window holds its own registry and puts it on
/// the thread for as long as it is the active one; the overlay a window builds
/// keeps the entries it was built with.
#[doc(hidden)]
#[derive(Default)]
pub struct WindowModalState {
    commands: VecDeque<ModalCommand>,
    entries: Rc<HostedModalEntries>,
    layers: Vec<HostedLayer>,
    contents: Rc<HostedContents>,
}

impl WindowModalState {
    /// Moves what this registry holds that no window was around to own — modal
    /// commands, layers and hosted content — onto the thread's, leaving it
    /// empty.
    ///
    /// Called right after a window's registry is put on the thread, with the
    /// registry that was there before: whatever was asked while no window was
    /// active is given to the window that is active now. The entries are not
    /// moved; they exist only once a window has processed a command.
    pub fn absorb_into_thread(&mut self) {
        COMMANDS.with(|commands| commands.borrow_mut().append(&mut self.commands));
        LAYERS.with(|layers| layers.borrow_mut().append(&mut self.layers));
        let orphaned = std::mem::take(self.contents.items_mut());
        if !orphaned.is_empty() {
            hosted_contents().items_mut().extend(orphaned);
        }
        self.entries = Rc::default();
    }

    /// Exchanges this registry with the thread's. Calling it twice puts both
    /// back.
    pub fn swap_with_thread(&mut self) {
        COMMANDS.with(|commands| std::mem::swap(&mut *commands.borrow_mut(), &mut self.commands));
        ENTRIES.with(|entries| std::mem::swap(&mut *entries.borrow_mut(), &mut self.entries));
        LAYERS.with(|layers| std::mem::swap(&mut *layers.borrow_mut(), &mut self.layers));
        CONTENTS.with(|contents| std::mem::swap(&mut *contents.borrow_mut(), &mut self.contents));
    }
}

fn hosted_entries() -> Rc<HostedModalEntries> {
    ENTRIES.with(|entries| entries.borrow().clone())
}

fn hosted_contents() -> Rc<HostedContents> {
    CONTENTS.with(|contents| contents.borrow().clone())
}

fn with_entries<R>(callback: impl FnOnce(&[HostedModal]) -> R) -> R {
    // The handle is cloned out first: the callback may run code that makes
    // another window active, which swaps the handle.
    let entries = hosted_entries();
    callback(entries.entries())
}

fn with_entries_mut<R>(callback: impl FnOnce(&mut Vec<HostedModal>) -> R) -> R {
    let entries = hosted_entries();
    callback(entries.entries_mut())
}

/// A painter installed above every modal, receiving no events.
///
/// The painter records into the overlay's own retained list. Something larger
/// than a few rectangles, such as an element that follows the pointer, is
/// hosted with [`OverlayLayer::host_element`] instead, which gives it a render
/// node of its own.
///
/// A modal is a *mode*: presenting one deliberately cancels the gestures
/// underneath it and puts a barrier between the user and the rest of the
/// application. Some overlays are the opposite of that — drag feedback follows
/// a gesture that must keep running, and must never intercept the pointer it is
/// chasing. Those install a layer instead: it paints last, above the modals,
/// and that is all it does.
///
/// The painter returns whether it should stay installed, so an overlay that
/// finishes an animation can retire itself from inside the frame that finished
/// it.
///
/// # Examples
///
/// ```no_run
/// use std::rc::Rc;
///
/// use aimer_modal::OverlayLayer;
///
/// // Paints for exactly one frame, then removes itself.
/// let handle = OverlayLayer::install(Rc::new(|_ctx, _canvas| false));
/// handle.remove();
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayLayer;

/// Identifies an installed [`OverlayLayer`] so it can be taken down again.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OverlayLayerHandle(u64);

/// Paints one frame of an overlay layer, returning whether to keep it.
///
/// The layer records into the overlay's own retained list through `canvas`, in
/// logical window coordinates: the overlay covers the window from its origin.
/// `ctx` supplies the scale and the window.
pub type OverlayPainter = Rc<dyn Fn(&BuildContext, &Canvas) -> bool>;

struct HostedLayer {
    id: OverlayLayerHandle,
    paint: OverlayPainter,
}

impl OverlayLayer {
    /// Installs `paint` above every modal and returns its handle.
    pub fn install(paint: OverlayPainter) -> OverlayLayerHandle {
        let id = NEXT_ID.with(|next_id| {
            let id = OverlayLayerHandle(next_id.get());
            next_id.set(next_id.get().wrapping_add(1).max(1));
            id
        });
        LAYERS.with_borrow_mut(|layers| layers.push(HostedLayer { id, paint }));
        request_animation_frame();
        id
    }

    /// Returns whether any layer is installed.
    pub fn is_installed() -> bool {
        LAYERS.with_borrow(|layers| !layers.is_empty())
    }

    /// Hosts `element` above every modal as a retained node of its own.
    ///
    /// `position` is asked for the element's top-left corner every time the
    /// render tree synchronizes, in device pixels relative to the overlay (the
    /// same unit [`LayoutElement::pos`] reports), so the element follows a
    /// pointer without being rebuilt. The element takes no events: it is not
    /// part of the overlay's routed children.
    #[doc(hidden)]
    pub fn host_element(
        element: AnyElement,
        position: Rc<dyn Fn() -> aimer_attribute::Vec2d>,
    ) -> OverlayContentHandle {
        let id = NEXT_ID.with(|next_id| {
            let id = next_id.get();
            next_id.set(id.wrapping_add(1).max(1));
            id
        });
        hosted_contents().items_mut().push(HostedContent {
            id,
            element: OverlayFollower {
                child: element,
                position,
            }
            .boxed(),
        });
        aimer_widget::notify_hosted_element_tree_changed();
        request_animation_frame();
        OverlayContentHandle(id)
    }
}

/// Identifies content hosted with [`OverlayLayer::host_element`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayContentHandle(u64);

impl OverlayContentHandle {
    /// Takes the content down. Repeated calls are harmless.
    pub fn remove(self) {
        let removed = {
            let contents = hosted_contents();
            let items = contents.items_mut();
            let before = items.len();
            items.retain(|item| item.id != self.0);
            before != items.len()
        };
        if removed {
            aimer_widget::notify_hosted_element_tree_changed();
        }
        request_animation_frame();
    }
}

struct HostedContent {
    id: u64,
    element: AnyElement,
}

/// UI-thread-owned overlay content, shared with the retained overlay's visitors
/// under the same discipline as [`HostedModalEntries`]: it changes only between
/// traversals, never while a visitor holds a reference.
#[derive(Default)]
struct HostedContents {
    items: UnsafeCell<Vec<HostedContent>>,
}

impl HostedContents {
    fn items(&self) -> &[HostedContent] {
        // SAFETY: mutated only between traversals on the UI thread.
        unsafe { &*self.items.get() }
    }

    fn items_mut(&self) -> &mut Vec<HostedContent> {
        // SAFETY: as above; callers never hold a visited reference across this.
        unsafe { &mut *self.items.get() }
    }
}

/// Places one hosted element, and positions nothing else.
struct OverlayFollower {
    child: AnyElement,
    position: Rc<dyn Fn() -> aimer_attribute::Vec2d>,
}

impl VisitorElement for OverlayFollower {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "OverlayFollower"
    }
}

impl EventElement for OverlayFollower {}

impl RebuildableTrait for OverlayFollower {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for OverlayFollower {
    fn pos(&self) -> Option<aimer_attribute::Vec2d> {
        Some((self.position)())
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        // The child is as big as it wants to be, up to the window.
        let mut loose = ctx.clone();
        loose.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: ctx.parent_size.width,
            max_height: ctx.parent_size.height,
        };
        self.child.computed_size(&loose)
    }
}

impl Drawable for OverlayFollower {
    fn update(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = size;
        child_ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        child_ctx.visible_rect = None;
        self.child.update(&child_ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer_canvas::Canvas::of(ctx).finish();
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
        _child_index: usize,
    ) -> Option<BuildContext<'a>> {
        let size = self.computed_size(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = size;
        child_ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        child_ctx.visible_rect = None;
        Some(child_ctx)
    }
}

impl OverlayLayerHandle {
    /// Takes the layer down. Repeated calls are harmless.
    pub fn remove(self) {
        LAYERS.with_borrow_mut(|layers| layers.retain(|layer| layer.id != self));
        request_animation_frame();
    }
}

/// Records the installed layers into `canvas`, dropping the ones that asked to
/// retire.
///
/// The list is taken out of the slot for the duration of the walk, so a painter
/// is free to install or remove a layer while it runs.
fn record_layers(ctx: &BuildContext, canvas: &Canvas) {
    let mut layers = LAYERS.with_borrow_mut(std::mem::take);
    if layers.is_empty() {
        return;
    }
    layers.retain(|layer| (layer.paint)(ctx, canvas));
    LAYERS.with_borrow_mut(|installed| {
        layers.append(installed);
        *installed = layers;
    });
}

/// Stable identity assigned to a presented modal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModalId(u64);

/// A handle that can dismiss the modal returned by [`crate::Modal::show`].
#[derive(Clone)]
pub struct ModalHandle {
    id: ModalId,
    dismissed: Rc<Cell<bool>>,
}

impl ModalHandle {
    /// Returns this modal's stable identity.
    pub fn id(&self) -> ModalId {
        self.id
    }

    /// Begins dismissal. Repeated calls are harmless and return `false`.
    pub fn dismiss(&self) -> bool {
        if self.dismissed.replace(true) {
            return false;
        }
        enqueue(ModalCommand::Dismiss(self.id));
        true
    }

    /// Whether this modal is still presented.
    ///
    /// A modal is not only closed by its owner: the barrier, `Escape` and
    /// [`ModalController::dismiss_top`] all close it by id, leaving the handle
    /// none the wiser. An owner that acts on "my panel is open" — keeping a
    /// selection alive for it, say — must ask the registry rather than trust
    /// its own last request.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn show() -> aimer_modal::ModalHandle { unimplemented!() }
    /// let handle = show();
    /// assert!(handle.is_showing());
    ///
    /// handle.dismiss();
    /// assert!(!handle.is_showing());
    /// ```
    pub fn is_showing(&self) -> bool {
        !self.dismissed.get() && is_presented(self.id)
    }
}

/// Whether `id` is presented, or on its way to being presented.
///
/// A queued `Show` counts, because a modal asked for during a frame is only
/// built by the next one; a queued `Dismiss` does not, so an owner learns of a
/// closure in the same frame it happened rather than one later.
fn is_presented(id: ModalId) -> bool {
    let closing = COMMANDS.with_borrow(|commands| {
        commands
            .iter()
            .any(|command| matches!(command, ModalCommand::Dismiss(dismissed) if *dismissed == id))
    });
    if closing {
        return false;
    }
    let queued = COMMANDS.with_borrow(|commands| {
        commands
            .iter()
            .any(|command| matches!(command, ModalCommand::Show { id: shown, .. } if *shown == id))
    });
    queued
        || with_entries(|entries| {
            entries
                .iter()
                .any(|entry| entry.id == id && !entry.timeline.borrow().is_closing())
        })
}

/// Access to the application-wide modal overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModalController;

impl ModalController {
    /// Begins dismissal of the topmost modal, if one exists.
    pub fn dismiss_top() -> bool {
        let has_modal = with_entries(|entries| !entries.is_empty())
            || COMMANDS.with(|commands| {
                commands
                    .borrow()
                    .iter()
                    .any(|command| matches!(command, ModalCommand::Show { .. }))
            });
        if has_modal {
            enqueue(ModalCommand::DismissTop);
        }
        has_modal
    }

    /// Returns whether a modal is active or waiting for the first host frame.
    pub fn is_showing() -> bool {
        !with_entries(|entries| entries.is_empty())
            || COMMANDS.with(|commands| {
                commands
                    .borrow()
                    .iter()
                    .any(|command| matches!(command, ModalCommand::Show { .. }))
            })
    }
}

/// Root overlay that paints application modals above its child.
///
/// `AimerApp` installs this host automatically. It remains public for embedded
/// render roots and tests that construct widget trees without `AimerApp`.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_modal::ModalHost", schema_only)]
pub struct ModalHost<W = RequiredChild> {
    #[portable_child]
    child: W,
}

impl Default for ModalHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalHost {
    /// Creates an incomplete host builder.
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
        }
    }

    /// Attaches the application root and completes the host.
    pub fn child<W: Widget>(self, child: W) -> ModalHost<W> {
        ModalHost { child }
    }
}

impl<W: Widget + 'static> Widget for ModalHost<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawModalHost {
            child: self.child.to_element(ctx),
            overlay: RawModalOverlay::default().boxed(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "ModalHost"
    }
}

struct RawModalHost {
    child: AnyElement,
    overlay: AnyElement,
}

impl aimer_widget::Rebuildable for RawModalHost {
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl RawModalHost {
    fn prepare_pending_commands(&self, ctx: &BuildContext) {
        if process_commands(ctx) {
            let _ = broadcast_event(self.child.as_ref(), &ElementEvent::Cancel);
        }
    }
}

/// Applies queued modal changes before the app builds its retained render tree.
///
/// `ModalHost` keeps modal widgets in a window-wide overlay registry rather
/// than the ordinary widget child. Preparing the host first makes those live
/// entries visible to retained traversal on the same frame they are built.
#[doc(hidden)]
pub fn prepare_retained_render_tree(root: &dyn Element, ctx: &BuildContext) {
    if let Some(host) = root
        .option_any()
        .and_then(|value| value.downcast_ref::<RawModalHost>())
    {
        host.prepare_pending_commands(ctx);
        return;
    }
    root.visit_children(&mut |child| prepare_retained_render_tree(child, ctx));
}

impl Drop for RawModalHost {
    fn drop(&mut self) {
        clear_registry();
    }
}

impl Drawable for RawModalHost {
    fn update(&self, ctx: &BuildContext) {
        self.prepare_pending_commands(ctx);
        self.child.update(ctx);
        self.overlay.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        // ModalHost owns no paint of its own. Its app child and modal overlay
        // are separate render nodes. Hosted entries keep independent local
        // lists while their animation updates opacity and transform state.
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }
}

impl EventElement for RawModalHost {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.overlay.as_ref().on_event(event)
    }
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
        visitor(self.overlay.as_ref());
    }
}

impl LayoutElement for RawModalHost {
    fn size(&self) -> Option<Size> {
        None
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }
}

impl VisitorElement for RawModalHost {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
        visitor(self.overlay.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "ModalHost"
    }
}

#[derive(Rebuildable)]
struct RawModalOverlay {
    entries: Rc<HostedModalEntries>,
    /// Elements hosted by overlay layers, above every entry.
    contents: Rc<HostedContents>,
    promoted_captures: RefCell<HashSet<PointerKey>>,
    /// Whether the list recorded last frame held any layer.
    recorded_layers: Cell<bool>,
}

impl Default for RawModalOverlay {
    fn default() -> Self {
        Self {
            entries: hosted_entries(),
            contents: hosted_contents(),
            promoted_captures: RefCell::new(HashSet::new()),
            recorded_layers: Cell::new(false),
        }
    }
}

impl RawModalOverlay {
    fn draw_entries(&self, ctx: &BuildContext) {
        draw_hosted_entries(&self.entries, ctx);
        for content in self.contents.items() {
            content.element.update(ctx);
        }
    }
}

fn draw_hosted_entries(entries: &HostedModalEntries, ctx: &BuildContext) {
    let now = AnimInstant::now();
    for entry in entries.entries() {
        entry.timeline.borrow_mut().tick(now, entry.animation.as_ref());
        entry.element.update(ctx);
    }

    let finished: HashSet<_> = entries
        .entries()
        .iter()
        .filter(|entry| entry.timeline.borrow().finished())
        .map(|entry| entry.id)
        .collect();
    if !finished.is_empty() {
        for entry in entries
            .entries()
            .iter()
            .filter(|entry| finished.contains(&entry.id))
        {
            cancel_hosted_entry(entry);
        }
        let retired = {
            let entries = entries.entries_mut();
            let mut current = std::mem::take(entries);
            let mut keep = Vec::with_capacity(current.len());
            let mut retired = Vec::new();
            for entry in current.drain(..) {
                if finished.contains(&entry.id) {
                    retired.push(entry);
                } else {
                    keep.push(entry);
                }
            }
            *entries = keep;
            retired
        };
        drop(retired);
        aimer_widget::notify_hosted_element_tree_changed();
    }
}

impl Drawable for RawModalOverlay {
    fn update(&self, ctx: &BuildContext) {
        self.draw_entries(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        // The hosted entries are retained children. The overlay's own list
        // holds the layers, which paint above every one of them.
        let canvas = aimer_canvas::Canvas::of(ctx);
        self.recorded_layers.set(OverlayLayer::is_installed());
        record_layers(ctx, &canvas);
        canvas.finish();
    }

    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        // A layer may paint something else every frame, and the list recorded
        // while one was installed has to be replaced once the last one goes.
        OverlayLayer::is_installed() || self.recorded_layers.get()
    }
}

impl EventElement for RawModalOverlay {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        let entries = self.entries.entries();
        if matches!(event, ElementEvent::Scroll { .. }) {
            // Scroll events carry no pointer position. Route them to the
            // topmost modal directly so an anchored wheel can consume a
            // trackpad or mouse-wheel frame instead of the page beneath
            // it. Pointer events still use normal hit testing/capture.
            return self.forward_child_redraw(
                entries.last().map_or_else(EventResult::ignored, |entry| {
                    dispatch_focused_event(entry.element.as_ref(), event)
                }),
            );
        }
        if matches!(
            event,
            ElementEvent::KeyInput {
                key: aimer_events::element::NamedKey::Escape,
                action: aimer_events::element::KeyAction::Pressed,
                ..
            }
        ) {
            return self.forward_child_redraw(
                entries
                    .last()
                    .map_or_else(EventResult::ignored, |entry| entry.element.on_event(event)),
            );
        }
        let pointer = event_pointer_key(event);
        if let Some(pointer) = pointer
            && let Some(entry) = entries
                .iter()
                .rev()
                .find(|entry| entry.dispatcher.borrow().is_captured(pointer))
        {
            let pos = event.get_pointer_pos().unwrap_or_default();
            let result = dispatch_hosted_event(entry, pos, event);
            self.track_promoted_capture(result, pointer);
            return self.forward_child_redraw(result);
        }
        if let Some(pointer) = pointer
            && self.promoted_captures.borrow_mut().remove(&pointer)
        {
            return EventResult::ignored().with_pointer_release(pointer);
        }
        let mut result = EventResult::ignored();
        for entry in entries.iter().rev() {
            let pos = event.get_pointer_pos().unwrap_or_default();
            let entry_result = dispatch_hosted_event(entry, pos, event);
            if let Some(pointer) = pointer {
                self.track_promoted_capture(entry_result, pointer);
            }
            result = result.merge(entry_result);
            if entry_result.is_consumed() {
                return self.forward_child_redraw(result);
            }
        }
        self.forward_child_redraw(result.merge(EventResult::from(!entries.is_empty())))
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn focus_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn hit_test_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}
}

impl RawModalOverlay {
    fn forward_child_redraw(&self, result: EventResult) -> EventResult {
        if result.needs_redraw() {
            // The hosted element already marked its own retained node stale.
            // Schedule the frame without invalidating this viewport-sized
            // routing node as well.
            request_animation_frame();
            result.without_redraw()
        } else {
            result
        }
    }

    fn track_promoted_capture(&self, result: EventResult, pointer: PointerKey) {
        match result.capture_request() {
            aimer_widget::CaptureRequest::Capture(captured) if captured == pointer => {
                self.promoted_captures.borrow_mut().insert(pointer);
            }
            aimer_widget::CaptureRequest::Release(released) if released == pointer => {
                self.promoted_captures.borrow_mut().remove(&pointer);
            }
            _ => {}
        }
    }
}

impl LayoutElement for RawModalOverlay {
    fn size(&self) -> Option<Size> {
        None
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }
}

impl VisitorElement for RawModalOverlay {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for entry in self.entries.entries() {
            visitor(entry.element.as_ref());
        }
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        let entries = self.entries.entries();
        for (index, entry) in entries.iter().enumerate() {
            visitor(index, entry.element.as_ref());
        }
        for (offset, content) in self.contents.items().iter().enumerate() {
            visitor(entries.len() + offset, content.element.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "ModalOverlay"
    }
}

enum ModalCommand {
    Show {
        id: ModalId,
        animation: Option<ModalAnimation>,
        build: EntryBuilder,
    },
    Dismiss(ModalId),
    DismissTop,
}

struct HostedModal {
    id: ModalId,
    element: AnyElement,
    animation: Option<ModalAnimation>,
    timeline: Rc<RefCell<ModalTimeline>>,
    dispatcher: RefCell<EventDispatcher>,
    focus_trap: RefCell<Option<FocusTrap>>,
}

/// UI-thread-owned modal entries shared with the retained overlay's visitors.
///
/// Entry references must remain stable while the retained tree is collected or
/// hosted entries are drawn. Mutations happen at frame preparation or after the
/// hosted draw walk finishes, never from a visitor or entry callback.
struct HostedModalEntries {
    entries: UnsafeCell<Vec<HostedModal>>,
}

impl Default for HostedModalEntries {
    fn default() -> Self {
        Self {
            entries: UnsafeCell::new(Vec::new()),
        }
    }
}

impl HostedModalEntries {
    fn entries(&self) -> &[HostedModal] {
        // SAFETY: the UI thread only mutates entries before retained traversal
        // or after hosted draw callbacks finish; visitors may keep these
        // references until the current traversal finishes.
        unsafe { &*self.entries.get() }
    }

    fn entries_mut(&self) -> &mut Vec<HostedModal> {
        // SAFETY: callers mutate only at the frame safe points documented on
        // this type, never while a visitor or hosted draw callback is active.
        unsafe { &mut *self.entries.get() }
    }
}

impl HostedModal {
    /// Stops confining keyboard focus to this entry.
    ///
    /// Called when dismissal begins rather than when the entry is finally
    /// dropped: an exit animation is the modal leaving, so the application
    /// underneath gets its focus owner back while the content fades out instead
    /// of a few hundred milliseconds later. Releasing twice is harmless, and the
    /// guard is dropped with the entry in any case, so an entry can never leave
    /// focus confined to itself after it is gone.
    fn release_focus_trap(&self) {
        drop(self.focus_trap.borrow_mut().take());
    }
}

fn event_pointer_key(event: &ElementEvent) -> Option<PointerKey> {
    match event {
        ElementEvent::PointerDown(pointer)
        | ElementEvent::PointerUp(pointer)
        | ElementEvent::PointerMove(pointer) => Some(PointerKey::new(pointer.source, pointer.id)),
        ElementEvent::PointerExited(source, id) => Some(PointerKey::new(*source, *id)),
        _ => None,
    }
}

fn dispatch_hosted_event(
    entry: &HostedModal,
    pos: aimer_attribute::Vec2d,
    event: &ElementEvent,
) -> EventResult {
    let pointer = event_pointer_key(event);
    let was_captured =
        pointer.is_some_and(|pointer| entry.dispatcher.borrow().is_captured(pointer));
    let result = entry
        .dispatcher
        .borrow_mut()
        .dispatch(entry.element.as_ref(), pos, event);
    let is_captured = pointer.is_some_and(|pointer| entry.dispatcher.borrow().is_captured(pointer));
    match (pointer, was_captured, is_captured) {
        (Some(pointer), false, true) => result.with_pointer_capture(pointer),
        (Some(pointer), true, false) => result.with_pointer_release(pointer),
        _ => result,
    }
}

fn cancel_hosted_entry(entry: &HostedModal) {
    let _ = broadcast_event(entry.element.as_ref(), &ElementEvent::Cancel);
    entry.dispatcher.borrow_mut().clear_captures();
}

pub(crate) struct ModalTimeline {
    progress: f32,
    phase: TimelinePhase,
}

enum TimelinePhase {
    Entering {
        started: Option<AnimInstant>,
    },
    Shown,
    Exiting {
        started: Option<AnimInstant>,
        from: f32,
    },
    Finished,
}

impl ModalTimeline {
    fn new(animated: bool) -> Self {
        Self {
            progress: if animated { 0.0 } else { 1.0 },
            phase: if animated {
                TimelinePhase::Entering { started: None }
            } else {
                TimelinePhase::Shown
            },
        }
    }

    pub(crate) fn new_static() -> Self {
        Self::new(false)
    }

    pub(crate) fn progress(&self) -> f32 {
        self.progress
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(
            self.phase,
            TimelinePhase::Entering { .. } | TimelinePhase::Exiting { .. }
        )
    }

    fn begin_exit(&mut self, animated: bool) {
        if matches!(
            self.phase,
            TimelinePhase::Exiting { .. } | TimelinePhase::Finished
        ) {
            return;
        }
        if animated {
            self.phase = TimelinePhase::Exiting {
                started: None,
                from: self.progress,
            };
        } else {
            self.progress = 0.0;
            self.phase = TimelinePhase::Finished;
        }
        request_animation_frame();
    }

    fn tick(&mut self, now: AnimInstant, animation: Option<&ModalAnimation>) {
        let Some(animation) = animation else {
            return;
        };
        match &mut self.phase {
            TimelinePhase::Entering { started } => {
                let start = *started.get_or_insert(now);
                let t = duration_progress(now, start, animation.enter_duration);
                self.progress = animation.enter_curve.transform(t);
                if t >= 1.0 {
                    self.progress = 1.0;
                    self.phase = TimelinePhase::Shown;
                } else {
                    request_animation_frame();
                }
            }
            TimelinePhase::Exiting { started, from } => {
                let start = *started.get_or_insert(now);
                let t = duration_progress(now, start, animation.exit_duration);
                self.progress = *from * (1.0 - animation.exit_curve.transform(t));
                if t >= 1.0 {
                    self.progress = 0.0;
                    self.phase = TimelinePhase::Finished;
                } else {
                    request_animation_frame();
                }
            }
            TimelinePhase::Shown | TimelinePhase::Finished => {}
        }
    }

    fn finished(&self) -> bool {
        matches!(self.phase, TimelinePhase::Finished)
    }

    /// Whether the modal is on its way out, animating or already gone.
    fn is_closing(&self) -> bool {
        matches!(
            self.phase,
            TimelinePhase::Exiting { .. } | TimelinePhase::Finished
        )
    }
}

fn duration_progress(now: AnimInstant, start: AnimInstant, duration: std::time::Duration) -> f32 {
    if duration.is_zero() {
        1.0
    } else {
        (now.duration_since(start).as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
    }
}

fn process_commands(ctx: &BuildContext) -> bool {
    let commands = COMMANDS.with(|commands| std::mem::take(&mut *commands.borrow_mut()));
    let mut opened = false;
    let mut added_entry = false;
    for command in commands {
        match command {
            ModalCommand::Show {
                id,
                animation,
                build,
            } => {
                with_entries(|entries| {
                    for entry in entries {
                        cancel_hosted_entry(entry);
                    }
                });
                let timeline = Rc::new(RefCell::new(ModalTimeline::new(animation.is_some())));
                let element = build(ctx, id, timeline.clone());
                element.rebuild_if_dirty(ctx);
                // A modal is a mode: while it is presented it owns the
                // keyboard, and the tree it covers — which dispatches
                // separately, and cannot see this content at all — owns
                // nothing until the trap is released.
                let focus_trap = FocusTrap::acquire();
                let dispatcher = EventDispatcher::new().with_focus_trap(focus_trap.id());
                with_entries_mut(|entries| {
                    entries.push(HostedModal {
                        id,
                        element,
                        animation,
                        timeline,
                        dispatcher: RefCell::new(dispatcher),
                        focus_trap: RefCell::new(Some(focus_trap)),
                    });
                });
                opened = true;
                added_entry = true;
            }
            ModalCommand::Dismiss(id) => {
                with_entries(|entries| {
                    if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
                        cancel_hosted_entry(entry);
                        entry.release_focus_trap();
                        entry
                            .timeline
                            .borrow_mut()
                            .begin_exit(entry.animation.is_some());
                    }
                });
            }
            ModalCommand::DismissTop => {
                with_entries(|entries| {
                    if let Some(entry) = entries.last() {
                        cancel_hosted_entry(entry);
                        entry.release_focus_trap();
                        entry
                            .timeline
                            .borrow_mut()
                            .begin_exit(entry.animation.is_some());
                    }
                });
            }
        }
    }
    if added_entry {
        aimer_widget::notify_hosted_element_tree_changed();
    }
    opened
}

fn enqueue(command: ModalCommand) {
    COMMANDS.with(|commands| commands.borrow_mut().push_back(command));
    request_animation_frame();
}

fn clear_registry() {
    COMMANDS.with(|commands| commands.borrow_mut().clear());
    let retired = with_entries_mut(std::mem::take);
    if !retired.is_empty() {
        drop(retired);
        aimer_widget::notify_hosted_element_tree_changed();
    }
    LAYERS.with(|layers| layers.borrow_mut().clear());
}

pub(crate) fn show(animation: Option<ModalAnimation>, build: EntryBuilder) -> ModalHandle {
    let id = NEXT_ID.with(|next_id| {
        let id = ModalId(next_id.get());
        next_id.set(next_id.get().wrapping_add(1).max(1));
        id
    });
    let dismissed = Rc::new(Cell::new(false));
    enqueue(ModalCommand::Show {
        id,
        animation,
        build,
    });
    ModalHandle { id, dismissed }
}

pub(crate) fn dismiss(id: ModalId) {
    enqueue(ModalCommand::Dismiss(id));
}

#[cfg(test)]
pub(crate) fn reset_registry_for_test() {
    clear_registry();
    NEXT_ID.with(|next_id| next_id.set(1));
}

#[cfg(test)]
pub(crate) fn pending_command_count_for_test() -> usize {
    COMMANDS.with(|commands| commands.borrow().len())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use aimer_animation::{AnimInstant, curves};
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::base::BuildContext;
    use aimer_widget::focus::{FocusNode, FocusTrap, active_focus_trap};
    use aimer_widget::{
        CaptureRequest, Drawable, Element, EventDispatcher, EventElement, EventResult,
        EventTreeRole, LayoutElement, PointerKey, Rebuildable, VisitorElement,
    };

    use super::{HostedModal, ModalId, ModalTimeline, dispatch_hosted_event};
    use crate::ModalAnimation;

    fn headless_context() -> BuildContext<'static> {
        let canvas = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        BuildContext::new(
            aimer_canvas::FrameCanvas::new(canvas),
            aimer_attribute::ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Default::default(),
            Default::default(),
            aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            tokio::runtime::Handle::current(),
        )
    }

    /// What `overlay` records in its own retained list for one frame.
    fn record_overlay(
        overlay: &super::RawModalOverlay,
        ctx: &BuildContext<'_>,
    ) -> std::sync::Arc<[aimer_cupid::draw_cmd_v2::DrawCommand]> {
        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 200.0, 100.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            overlay.paint_local_v2(ctx);
        });
        tree.draw_list_snapshot(root).unwrap().commands
    }

    /// A layer is painted into the overlay's own retained list, above every
    /// hosted modal, and the overlay asks to be recorded again for as long as
    /// a layer is installed (a layer may paint something different each frame)
    /// and once more after the last one goes, so its pixels are cleared.
    #[tokio::test]
    async fn an_installed_layer_is_recorded_into_the_overlay_list() {
        super::clear_registry();
        let ctx = headless_context();
        let overlay = super::RawModalOverlay::default();
        assert!(!overlay.local_v2_paint_needs_recording(&ctx));
        assert!(record_overlay(&overlay, &ctx).is_empty());

        let handle = super::OverlayLayer::install(Rc::new(|_ctx, canvas| {
            canvas.fill_rect(
                aimer_cupid::draw_cmd_v2::Rect::new(10.0, 10.0, 20.0, 20.0),
                [255, 0, 0, 255],
            );
            true
        }));
        assert!(overlay.local_v2_paint_needs_recording(&ctx));
        let commands = record_overlay(&overlay, &ctx);
        assert!(commands.iter().any(|command| matches!(
            command,
            aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { .. }
        )));
        assert!(
            overlay.local_v2_paint_needs_recording(&ctx),
            "a layer may paint something else next frame"
        );

        handle.remove();
        assert!(
            overlay.local_v2_paint_needs_recording(&ctx),
            "the list recorded with the layer has to be replaced"
        );
        assert!(record_overlay(&overlay, &ctx).is_empty());
        assert!(!overlay.local_v2_paint_needs_recording(&ctx));
    }

    /// Content hosted by an overlay layer is a retained child of the overlay:
    /// it gets its own render node, placed where the layer says, and it takes
    /// no events.
    #[tokio::test]
    async fn hosted_content_is_a_retained_child_of_the_overlay_at_its_position() {
        super::clear_registry();
        let ctx = headless_context();
        let overlay = super::RawModalOverlay::default();
        let position = Rc::new(Cell::new(Vec2d { x: 30.0, y: 40.0 }));
        let handle = super::OverlayLayer::host_element(
            CapturingModalElement {
                events: Rc::new(Cell::new(0)),
            }
            .boxed(),
            {
                let position = Rc::clone(&position);
                Rc::new(move || position.get())
            },
        );

        let mut retained = Vec::new();
        overlay.visit_retained_v2_children(&mut |index, child| {
            retained.push((index, child.pos(), child.debug_name()));
        });
        assert_eq!(retained.len(), 1, "{retained:?}");
        // The element reports its place in device pixels, relative to the overlay.
        assert_eq!(retained[0].1, Some(Vec2d { x: 30.0, y: 40.0 }));

        position.set(Vec2d { x: 50.0, y: 60.0 });
        let mut moved = None;
        overlay.visit_retained_v2_children(&mut |_, child| moved = child.pos());
        assert_eq!(
            moved,
            Some(Vec2d { x: 50.0, y: 60.0 }),
            "the place is read each time"
        );

        let mut routed = 0;
        overlay.visit_children(&mut |_| routed += 1);
        assert_eq!(routed, 0, "hosted content takes no events");

        handle.remove();
        let mut left = 0;
        overlay.visit_retained_v2_children(&mut |_, _| left += 1);
        assert_eq!(left, 0);
        let _ = ctx;
    }

    #[test]
    fn modal_host_and_overlay_opt_into_indexed_event_routing() {
        let host = super::RawModalHost {
            child: CapturingModalElement {
                events: Rc::new(Cell::new(0)),
            }
            .boxed(),
            overlay: super::RawModalOverlay::default().boxed(),
        };

        assert_eq!(host.event_tree_role(), EventTreeRole::IndexedTarget);
        assert_eq!(host.overlay.event_tree_role(), EventTreeRole::IndexedTarget);
        assert!(
            host.overlay.element_id().is_some(),
            "the indexed overlay needs a stable element identity"
        );
    }

    /// Builds an entry that confines focus, as `process_commands` does.
    fn trapping_entry(element: aimer_widget::AnyElement) -> HostedModal {
        let focus_trap = FocusTrap::acquire();
        let dispatcher = EventDispatcher::new().with_focus_trap(focus_trap.id());
        HostedModal {
            id: ModalId(1),
            element,
            animation: None,
            timeline: Rc::new(RefCell::new(ModalTimeline::new(false))),
            dispatcher: RefCell::new(dispatcher),
            focus_trap: RefCell::new(Some(focus_trap)),
        }
    }

    struct FocusableModalContent {
        node: FocusNode,
    }

    impl VisitorElement for FocusableModalContent {
        fn debug_name(&self) -> &'static str {
            "FocusableModalContent"
        }
    }

    impl EventElement for FocusableModalContent {
        fn focus_node(&self) -> Option<&FocusNode> {
            Some(&self.node)
        }
    }

    impl LayoutElement for FocusableModalContent {}
    impl Drawable for FocusableModalContent {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for FocusableModalContent {}

    #[test]
    fn a_hosted_entry_confines_focus_until_it_is_dismissed() {
        let entry = trapping_entry(
            CapturingModalElement {
                events: Rc::new(Cell::new(0)),
            }
            .boxed(),
        );

        assert_eq!(
            active_focus_trap(),
            entry.dispatcher.borrow().focus_trap(),
            "the entry dispatches inside the region it traps"
        );

        entry.release_focus_trap();
        assert_eq!(active_focus_trap(), None);

        entry.release_focus_trap();
        assert_eq!(active_focus_trap(), None, "releasing twice is harmless");
    }

    #[test]
    fn a_dropped_entry_stops_confining_focus() {
        let entry = trapping_entry(
            CapturingModalElement {
                events: Rc::new(Cell::new(0)),
            }
            .boxed(),
        );

        drop(entry);

        assert_eq!(active_focus_trap(), None);
    }

    #[test]
    fn content_of_the_trapping_entry_still_takes_focus() {
        let node = FocusNode::new();
        let entry = trapping_entry(FocusableModalContent { node: node.clone() }.boxed());

        node.request_focus();
        let _ = dispatch_hosted_event(&entry, Vec2d::default(), &ElementEvent::Cancel);

        assert!(
            node.has_focus(),
            "a modal confines focus to itself, not away from itself"
        );
        assert!(entry.dispatcher.borrow().focused().is_some());
    }

    struct CapturingModalElement {
        events: Rc<Cell<usize>>,
    }

    impl VisitorElement for CapturingModalElement {
        fn debug_name(&self) -> &'static str {
            "CapturingModalElement"
        }
    }

    impl EventElement for CapturingModalElement {
        fn event_tree_role(&self) -> EventTreeRole {
            EventTreeRole::IndexedTarget
        }

        fn on_event(&self, event: &ElementEvent) -> EventResult {
            self.events.set(self.events.get() + 1);
            match event {
                ElementEvent::PointerDown(pointer) => EventResult::consumed()
                    .with_pointer_capture(PointerKey::new(pointer.source, pointer.id)),
                ElementEvent::PointerUp(pointer) => EventResult::consumed()
                    .with_pointer_release(PointerKey::new(pointer.source, pointer.id)),
                _ => EventResult::consumed(),
            }
        }
    }

    impl LayoutElement for CapturingModalElement {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::default(), Vec2d { x: 10.0, y: 10.0 }))
        }
    }

    impl Drawable for CapturingModalElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for CapturingModalElement {}

    #[test]
    fn hosted_modal_routes_capture_outside_until_up() {
        let events = Rc::new(Cell::new(0));
        let entry = trapping_entry(
            CapturingModalElement {
                events: events.clone(),
            }
            .boxed(),
        );
        let pointer = PointerKey::new(PointerSource::Touch, 4);
        let down = dispatch_hosted_event(
            &entry,
            Vec2d { x: 5.0, y: 5.0 },
            &ElementEvent::PointerDown(PointerInfo::new(
                Vec2d { x: 5.0, y: 5.0 },
                pointer.source,
                pointer.id,
                PointerButton::Primary,
            )),
        );
        assert_eq!(down.capture_request(), CaptureRequest::Capture(pointer));

        let _ = dispatch_hosted_event(
            &entry,
            Vec2d { x: 50.0, y: 50.0 },
            &ElementEvent::PointerMove(PointerInfo::new(
                Vec2d { x: 50.0, y: 50.0 },
                pointer.source,
                pointer.id,
                PointerButton::Primary,
            )),
        );
        let up = dispatch_hosted_event(
            &entry,
            Vec2d { x: 50.0, y: 50.0 },
            &ElementEvent::PointerUp(PointerInfo::new(
                Vec2d { x: 50.0, y: 50.0 },
                pointer.source,
                pointer.id,
                PointerButton::Primary,
            )),
        );

        assert_eq!(events.get(), 3);
        assert_eq!(up.capture_request(), CaptureRequest::Release(pointer));
        assert_eq!(entry.dispatcher.borrow().capture_count(), 0);
    }

    #[test]
    fn modal_host_routes_capture_through_its_indexed_overlay() {
        super::reset_registry_for_test();
        let modal_events = Rc::new(Cell::new(0));
        let app_events = Rc::new(Cell::new(0));
        let entry = trapping_entry(
            CapturingModalElement {
                events: modal_events.clone(),
            }
            .boxed(),
        );
        super::with_entries_mut(|entries| entries.push(entry));

        let host = super::RawModalHost {
            child: CapturingModalElement {
                events: app_events.clone(),
            }
            .boxed(),
            overlay: super::RawModalOverlay::default().boxed(),
        }
        .boxed();
        let mut dispatcher = EventDispatcher::new();
        let pointer = PointerKey::new(PointerSource::Touch, 9);
        let inside = Vec2d { x: 5.0, y: 5.0 };
        let outside = Vec2d { x: 50.0, y: 50.0 };

        let down = ElementEvent::PointerDown(PointerInfo::new(
            inside,
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        ));
        let _ = dispatcher.dispatch(host.as_ref(), inside, &down);
        assert!(dispatcher.is_captured(pointer));
        assert_eq!(modal_events.get(), 1);
        assert_eq!(
            app_events.get(),
            0,
            "the modal overlay stays above app content"
        );

        let move_event = ElementEvent::PointerMove(PointerInfo::new(
            outside,
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        ));
        let _ = dispatcher.dispatch(host.as_ref(), outside, &move_event);
        let up = ElementEvent::PointerUp(PointerInfo::new(
            outside,
            pointer.source,
            pointer.id,
            PointerButton::Primary,
        ));
        let _ = dispatcher.dispatch(host.as_ref(), outside, &up);

        assert_eq!(modal_events.get(), 3);
        assert_eq!(app_events.get(), 0);
        assert!(!dispatcher.is_captured(pointer));
        super::reset_registry_for_test();
    }

    #[test]
    fn a_handle_reports_its_modal_showing_until_it_is_dismissed() {
        super::reset_registry_for_test();
        let handle = super::show(None, Box::new(|_ctx, _id, _timeline| unreachable!()));

        assert!(
            handle.is_showing(),
            "a modal is showing from the moment it is asked for"
        );

        assert!(handle.dismiss());
        assert!(!handle.is_showing());
    }

    #[test]
    fn a_handle_reports_a_modal_dismissed_by_someone_else() {
        super::reset_registry_for_test();
        let handle = super::show(None, Box::new(|_ctx, _id, _timeline| unreachable!()));

        // What the barrier and `Escape` do: dismiss by id, without the handle.
        super::dismiss(handle.id());

        assert!(
            !handle.is_showing(),
            "the owner must learn its modal was closed for it"
        );
    }

    #[test]
    fn timeline_reverses_from_visible_progress_without_a_jump() {
        let animation = ModalAnimation::new()
            .enter_duration(Duration::from_millis(100))
            .exit_duration(Duration::from_millis(100))
            .enter_curve(curves::LINEAR)
            .exit_curve(curves::LINEAR);
        let start = AnimInstant::now();
        let mut timeline = ModalTimeline::new(true);
        assert!(timeline.is_active());

        timeline.tick(start, Some(&animation));
        timeline.tick(start + Duration::from_millis(50), Some(&animation));
        assert!((timeline.progress() - 0.5).abs() < 0.01);
        assert!(timeline.is_active());

        timeline.begin_exit(true);
        assert!(timeline.is_active());
        timeline.tick(start + Duration::from_millis(50), Some(&animation));
        assert!((timeline.progress() - 0.5).abs() < 0.01);

        timeline.tick(start + Duration::from_millis(100), Some(&animation));
        assert!((timeline.progress() - 0.25).abs() < 0.01);
        assert!(timeline.is_active());
    }

    #[test]
    fn zero_duration_timeline_reaches_both_endpoints() {
        let animation = ModalAnimation::new()
            .enter_duration(Duration::ZERO)
            .exit_duration(Duration::ZERO);
        let now = AnimInstant::now();
        let mut timeline = ModalTimeline::new(true);

        timeline.tick(now, Some(&animation));
        assert_eq!(timeline.progress(), 1.0);
        assert!(!timeline.is_active());

        timeline.begin_exit(true);
        timeline.tick(now, Some(&animation));
        assert_eq!(timeline.progress(), 0.0);
        assert!(timeline.finished());
        assert!(!timeline.is_active());
    }

    #[test]
    fn a_layer_installed_in_one_window_is_not_in_another() {
        super::clear_registry();
        let (mut first, mut second) = (
            super::WindowModalState::default(),
            super::WindowModalState::default(),
        );

        first.swap_with_thread();
        super::OverlayLayer::install(Rc::new(|_ctx, _canvas| true));
        assert!(super::OverlayLayer::is_installed());
        first.swap_with_thread();

        second.swap_with_thread();
        assert!(
            !super::OverlayLayer::is_installed(),
            "a layer of another window was installed here"
        );
        second.swap_with_thread();

        first.swap_with_thread();
        assert!(super::OverlayLayer::is_installed());
        super::clear_registry();
        first.swap_with_thread();
    }

    #[test]
    fn a_pending_command_waits_in_the_window_that_issued_it() {
        super::clear_registry();
        let (mut first, mut second) = (
            super::WindowModalState::default(),
            super::WindowModalState::default(),
        );

        first.swap_with_thread();
        super::dismiss(ModalId(1));
        assert_eq!(super::pending_command_count_for_test(), 1);
        first.swap_with_thread();

        second.swap_with_thread();
        assert_eq!(super::pending_command_count_for_test(), 0);
        second.swap_with_thread();

        first.swap_with_thread();
        assert_eq!(super::pending_command_count_for_test(), 1);
        super::clear_registry();
        first.swap_with_thread();
    }

    #[test]
    fn an_overlay_keeps_the_registry_it_was_built_with() {
        super::clear_registry();
        let (mut first, mut second) = (
            super::WindowModalState::default(),
            super::WindowModalState::default(),
        );

        first.swap_with_thread();
        let overlay = super::RawModalOverlay::default();
        let entries = overlay.entries.clone();
        first.swap_with_thread();
        second.swap_with_thread();
        let other = super::RawModalOverlay::default();
        second.swap_with_thread();

        assert!(
            !Rc::ptr_eq(&entries, &other.entries),
            "two windows' overlays share one registry"
        );
    }
}
